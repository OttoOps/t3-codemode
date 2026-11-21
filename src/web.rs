use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::extract::{Extension, Path, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use futures_util::stream;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio::sync::broadcast::error::{RecvError, TryRecvError};

use crate::app::App;
use crate::board::{CardRef, LaunchArgs, ModelSettingBody};
use crate::clock::iso;
use crate::config::{BoardRemoteConfig, RemoteMode, RemoteOrigin};
use crate::contexts::{ContextBody, ContextError};
use crate::pairing::{self, CookieExtract};
use crate::server::{RequestKind, SingleHeader, classify_request, is_own_loopback_host_str, single_header};

pub const CSRF_HEADER: &str = "x-t3cm-csrf";
const PAGE: &str = include_str!("board.html");
const PAIR_PAGE: &str = include_str!("pair.html");
const MAX_LAUNCH: usize = 20;
const KEEPALIVE: Duration = Duration::from_secs(25);
const SETTLE: Duration = Duration::from_millis(150);
const SESSION_CHECK: Duration = Duration::from_secs(1);
const CSP: &str = "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; connect-src 'self'; \
img-src 'self' data:; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

#[derive(Clone, Debug)]
struct AuthSession {
    seq: Option<i64>,
    tailscale_login: Option<String>,
}

#[derive(Clone)]
struct Web {
    app: Arc<App>,
    port: u16,
    remote: Option<(BoardRemoteConfig, RemoteOrigin)>,
}

fn error(status: StatusCode, msg: impl Into<String>) -> Response {
    json_ok(status, json!({"error": msg.into()}))
}

fn json_ok(status: StatusCode, body: Value) -> Response {
    (status, [(header::CONTENT_TYPE, "application/json"), (header::CACHE_CONTROL, "no-store")], body.to_string())
        .into_response()
}

fn same_bytes(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn is_mutating(method: &Method) -> bool {
    !matches!(*method, Method::GET | Method::HEAD)
}

impl Web {
    fn check_local_origin(&self, headers: &HeaderMap) -> Option<Response> {
        let origin = match single_header(headers, header::ORIGIN.as_str()) {
            SingleHeader::Present(o) => o,
            SingleHeader::Missing => return None,
            SingleHeader::Invalid => return Some(error(StatusCode::FORBIDDEN, "cross-origin requests are refused")),
        };
        let Some(rest) = origin.strip_prefix("http://") else {
            return Some(error(StatusCode::FORBIDDEN, "cross-origin requests are refused"));
        };
        if !is_own_loopback_host_str(rest, self.port) {
            return Some(error(StatusCode::FORBIDDEN, "cross-origin requests are refused"));
        }
        None
    }

    fn check_remote_host_and_origin(
        &self,
        headers: &HeaderMap,
        origin: &RemoteOrigin,
    ) -> Result<(), (StatusCode, &'static str)> {
        match single_header(headers, "x-forwarded-proto") {
            SingleHeader::Missing => {}
            SingleHeader::Present(proto) if proto.eq_ignore_ascii_case("https") => {}
            SingleHeader::Present(_) | SingleHeader::Invalid => {
                return Err((StatusCode::FORBIDDEN, "remote board requests must arrive over https"));
            }
        }
        let SingleHeader::Present(host) = single_header(headers, header::HOST.as_str()) else {
            return Err((StatusCode::FORBIDDEN, "remote request Host does not match board.remote.origin"));
        };
        let xfh = match single_header(headers, "x-forwarded-host") {
            SingleHeader::Missing => None,
            SingleHeader::Present(v) => Some(v),
            SingleHeader::Invalid => {
                return Err((StatusCode::FORBIDDEN, "remote request Host does not match board.remote.origin"));
            }
        };
        let host_ok = match xfh {
            Some(forwarded) => {
                origin.matches_host(forwarded)
                    && (origin.matches_host(host) || is_own_loopback_host_str(host, self.port))
            }
            None => origin.matches_host(host),
        };
        if !host_ok {
            return Err((StatusCode::FORBIDDEN, "remote request Host does not match board.remote.origin"));
        }
        match single_header(headers, header::ORIGIN.as_str()) {
            SingleHeader::Missing => {}
            SingleHeader::Present(req_origin) if origin.matches_origin(req_origin) => {}
            SingleHeader::Present(_) | SingleHeader::Invalid => {
                return Err((StatusCode::FORBIDDEN, "cross-origin requests are refused"));
            }
        }
        Ok(())
    }

    fn pairing_enabled(&self) -> bool {
        self.remote.as_ref().is_some_and(|(cfg, _)| cfg.pairing_enabled())
    }

    fn authenticate_pairing_cookie(&self, headers: &HeaderMap) -> Result<AuthSession, (StatusCode, &'static str)> {
        let raw = match pairing::extract_session_cookie(headers) {
            CookieExtract::Present(v) => v,
            CookieExtract::Missing => {
                return Err((StatusCode::UNAUTHORIZED, "remote board authentication is required"));
            }
            CookieExtract::Invalid => {
                return Err((StatusCode::UNAUTHORIZED, "remote board session is invalid or expired"));
            }
        };
        let hash = pairing::hash_token(raw);
        let now = iso(self.app.clock.now());
        match self.app.store.board_session_verify(&hash, &now) {
            Ok(Some(session)) => Ok(AuthSession { seq: Some(session.seq), tailscale_login: None }),
            Ok(None) => Err((StatusCode::UNAUTHORIZED, "remote board session is invalid or expired")),
            Err(e) => {
                tracing::warn!("board session lookup failed: {e:#}");
                Err((StatusCode::INTERNAL_SERVER_ERROR, "session lookup failed"))
            }
        }
    }

    async fn authenticate_tailscale(
        &self,
        cfg: &BoardRemoteConfig,
        headers: &HeaderMap,
    ) -> Result<String, (StatusCode, &'static str)> {
        let login = match single_header(headers, "tailscale-user-login") {
            SingleHeader::Present(v)
                if !v.is_empty()
                    && v == v.trim()
                    && !v.bytes().any(|b| b.is_ascii_whitespace() || b.is_ascii_control()) =>
            {
                v
            }
            SingleHeader::Missing | SingleHeader::Present(_) | SingleHeader::Invalid => {
                return Err((StatusCode::UNAUTHORIZED, "remote board authentication is required"));
            }
        };
        if !cfg.allow_users.is_empty() {
            if cfg.allow_users.iter().any(|u| u.eq_ignore_ascii_case(login)) {
                return Ok(login.to_string());
            }
            return Err((StatusCode::FORBIDDEN, "Tailscale user is not allowed to access this board"));
        }
        match self.app.tailscale_host_login_async().await {
            Ok(Some(host_login)) => {
                if host_login.eq_ignore_ascii_case(login) {
                    Ok(login.to_string())
                } else {
                    Err((StatusCode::FORBIDDEN, "Tailscale user is not allowed to access this board"))
                }
            }
            Ok(None) => Err((StatusCode::UNAUTHORIZED, "Tailscale host login is unavailable")),
            Err(e) => {
                tracing::warn!("tailscale status lookup failed: {e}");
                Err((StatusCode::UNAUTHORIZED, "Tailscale host login is unavailable"))
            }
        }
    }

    async fn tailscale_login_still_allowed(&self, login: &str) -> bool {
        let Some((cfg, _)) = &self.remote else {
            return false;
        };
        if cfg.mode != RemoteMode::Tailscale {
            return false;
        }
        if !cfg.allow_users.is_empty() {
            return cfg.allow_users.iter().any(|u| u.eq_ignore_ascii_case(login));
        }
        matches!(
            self.app.tailscale_host_login_async().await,
            Ok(Some(ref host_login)) if host_login.eq_ignore_ascii_case(login)
        )
    }

    /// Authenticate a remote board request. Pairing-cookie auth verifies a
    /// hashed session cookie from `store.db`; Tailscale Serve auth verifies
    /// `Tailscale-User-Login` against `allowUsers` (defaulting to the host's
    /// own login) and layers `authenticate_pairing_cookie` when
    /// `requirePairing` is enabled.
    async fn authenticate_remote(
        &self,
        cfg: &BoardRemoteConfig,
        headers: &HeaderMap,
    ) -> Result<Option<AuthSession>, (StatusCode, &'static str)> {
        match cfg.mode {
            RemoteMode::Pairing => self.authenticate_pairing_cookie(headers).map(Some),
            RemoteMode::Tailscale => {
                let login = self.authenticate_tailscale(cfg, headers).await?;
                let seq = if cfg.require_pairing { self.authenticate_pairing_cookie(headers)?.seq } else { None };
                Ok(Some(AuthSession { seq, tailscale_login: Some(login) }))
            }
            RemoteMode::Off => Err((StatusCode::FORBIDDEN, "remote board access is disabled")),
        }
    }

    async fn check_remote(&self, headers: &HeaderMap) -> Result<Option<AuthSession>, (StatusCode, &'static str)> {
        let Some((cfg, origin)) = &self.remote else {
            return Err((StatusCode::FORBIDDEN, "remote board access is disabled"));
        };
        self.check_remote_host_and_origin(headers, origin)?;
        self.authenticate_remote(cfg, headers).await
    }

    fn check_csrf(&self, headers: &HeaderMap) -> Option<Response> {
        let ok = matches!(
            single_header(headers, CSRF_HEADER),
            SingleHeader::Present(t) if same_bytes(t.as_bytes(), self.app.csrf.as_bytes())
        );
        if !ok {
            return Some(error(StatusCode::FORBIDDEN, "missing or wrong X-T3CM-CSRF token; reload the board"));
        }
        None
    }
}

async fn board_guard(
    State(web): State<Web>,
    mut req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let headers = req.headers();
    let mutating = is_mutating(req.method());
    let session = match classify_request(headers, web.port) {
        RequestKind::Local => {
            if let Some(r) = web.check_local_origin(headers) {
                return r;
            }
            None
        }
        RequestKind::Remote => match web.check_remote(headers).await {
            Ok(session) => session,
            Err((status, msg)) => return error(status, msg),
        },
    };
    if mutating && let Some(r) = web.check_csrf(headers) {
        return r;
    }
    if let Some(session) = session {
        req.extensions_mut().insert(session);
    }
    next.run(req).await
}

async fn pair_guard(State(web): State<Web>, req: axum::extract::Request, next: axum::middleware::Next) -> Response {
    let headers = req.headers();
    match classify_request(headers, web.port) {
        RequestKind::Local => {
            if let Some(r) = web.check_local_origin(headers) {
                return r;
            }
            if !web.pairing_enabled() {
                return error(StatusCode::FORBIDDEN, "remote pairing is not enabled");
            }
            return error(
                StatusCode::FORBIDDEN,
                "remote pairing must be completed over the configured https:// origin",
            );
        }
        RequestKind::Remote => {
            let Some((cfg, origin)) = &web.remote else {
                return error(StatusCode::FORBIDDEN, "remote board access is disabled");
            };
            if let Err((status, msg)) = web.check_remote_host_and_origin(headers, origin) {
                return error(status, msg);
            }
            if !web.pairing_enabled() {
                return error(StatusCode::FORBIDDEN, "remote pairing is not enabled");
            }
            if cfg.mode == RemoteMode::Tailscale
                && let Err((status, msg)) = web.authenticate_tailscale(cfg, headers).await
            {
                return error(status, msg);
            }
        }
    }
    next.run(req).await
}

fn body<T: DeserializeOwned>(raw: &Bytes) -> Result<T, String> {
    serde_json::from_slice(raw).map_err(|e| format!("bad request body: {e}"))
}

fn failed(app: &App, e: anyhow::Error) -> Response {
    let msg = app.t3.redact(format!("{e:#}"));
    let missing =
        ["no open board card", "no board ticket", "no ask", "no new issue"].iter().any(|p| msg.starts_with(p));
    let status = if missing { StatusCode::NOT_FOUND } else { StatusCode::BAD_REQUEST };
    error(status, msg)
}

async fn page(State(web): State<Web>) -> Response {
    let wf = web.app.wf.clone();
    tokio::spawn(async move {
        if let Err(e) = wf.reconcile_soon().await {
            tracing::warn!("github reconcile on page load failed: {e:#}");
        }
    });
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
            (header::CONTENT_SECURITY_POLICY, CSP),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            (header::REFERRER_POLICY, "no-referrer"),
        ],
        PAGE.replace("{{CSRF}}", &web.app.csrf).replace("{{VERSION}}", env!("CARGO_PKG_VERSION")),
    )
        .into_response()
}

async fn state(State(web): State<Web>) -> Response {
    match web.app.wf.snapshot() {
        Ok(v) => json_ok(StatusCode::OK, v),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")),
    }
}

async fn message(State(web): State<Web>, Path(handle): Path<String>) -> Response {
    match web.app.store.msg_get(&handle) {
        Ok(Some(m)) => json_ok(StatusCode::OK, json!(m)),
        Ok(None) => error(StatusCode::NOT_FOUND, format!("no message {handle:?}")),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")),
    }
}

async fn pair_page() -> Response {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
            (header::CONTENT_SECURITY_POLICY, CSP),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            (header::REFERRER_POLICY, "no-referrer"),
        ],
        PAIR_PAGE,
    )
        .into_response()
}

#[derive(Deserialize)]
struct PairBody {
    token: String,
    #[serde(default)]
    label: Option<String>,
}

fn is_json_content_type(headers: &HeaderMap) -> bool {
    let SingleHeader::Present(ct) = single_header(headers, header::CONTENT_TYPE.as_str()) else {
        return false;
    };
    ct.split(';').next().is_some_and(|m| m.trim().eq_ignore_ascii_case("application/json"))
}

async fn post_pair(State(web): State<Web>, headers: HeaderMap, raw: Bytes) -> Response {
    if !is_json_content_type(&headers) {
        return error(StatusCode::UNSUPPORTED_MEDIA_TYPE, "Content-Type must be application/json");
    }
    let b: PairBody = match body(&raw) {
        Ok(b) => b,
        Err(e) => return error(StatusCode::BAD_REQUEST, e),
    };
    let label = match pairing::clean_label(b.label.as_deref()) {
        Ok(l) => l,
        Err(e) => return error(StatusCode::BAD_REQUEST, format!("{e:#}")),
    };
    match pairing::redeem_pairing_token(&web.app.store, &b.token, label.as_deref(), web.app.clock.now()) {
        Ok(Some((raw_session, session))) => {
            let mut resp = json_ok(StatusCode::OK, json!({"ok": true, "session": session}));
            let cookie = pairing::session_cookie_header(&raw_session);
            if let Ok(v) = HeaderValue::from_str(&cookie) {
                resp.headers_mut().insert(header::SET_COOKIE, v);
            }
            resp
        }
        Ok(None) => error(StatusCode::UNAUTHORIZED, "pairing token is invalid, expired, or already used"),
        Err(e) => {
            tracing::warn!("pairing redemption failed: {e:#}");
            error(StatusCode::INTERNAL_SERVER_ERROR, "pairing redemption failed")
        }
    }
}

fn session_valid(app: &App, session_seq: Option<i64>) -> bool {
    let Some(seq) = session_seq else {
        return true;
    };
    let now = iso(app.clock.now());
    match app.store.board_session_is_active(seq, &now) {
        Ok(active) => active,
        Err(e) => {
            tracing::warn!("board session check failed: {e:#}");
            false
        }
    }
}

async fn auth_still_valid(web: &Web, auth: Option<&AuthSession>) -> bool {
    let Some(auth) = auth else {
        return true;
    };
    if let Some(seq) = auth.seq
        && !session_valid(&web.app, Some(seq))
    {
        return false;
    }
    if let Some(login) = auth.tailscale_login.as_deref()
        && !web.tailscale_login_still_allowed(login).await
    {
        return false;
    }
    true
}

async fn events(State(web): State<Web>, session: Option<Extension<AuthSession>>) -> Response {
    let rx = web.app.wf.subscribe();
    let stop = web.app.stopping.subscribe();
    let auth = session.map(|Extension(s)| s);
    let s = stream::unfold((web, rx, stop, auth, None::<String>), |(web, mut rx, mut stop, auth, last)| async move {
        let mut wait = last.is_some();
        loop {
            if *stop.borrow() {
                return None;
            }
            if !auth_still_valid(&web, auth.as_ref()).await {
                return None;
            }
            if wait {
                let mut got_event = false;
                while !got_event {
                    if auth.is_some() {
                        let mut check_auth = false;
                        tokio::select! {
                            r = rx.recv() => match r {
                                Ok(()) | Err(RecvError::Lagged(_)) => got_event = true,
                                Err(RecvError::Closed) => return None,
                            },
                            _ = stop.wait_for(|s| *s) => return None,
                            _ = tokio::time::sleep(SESSION_CHECK) => {
                                check_auth = true;
                            }
                        }
                        if check_auth && !auth_still_valid(&web, auth.as_ref()).await {
                            return None;
                        }
                    } else {
                        tokio::select! {
                            r = rx.recv() => match r {
                                Ok(()) | Err(RecvError::Lagged(_)) => got_event = true,
                                Err(RecvError::Closed) => return None,
                            },
                            _ = stop.wait_for(|s| *s) => return None,
                        }
                    }
                }
                tokio::time::sleep(SETTLE).await;
                while matches!(rx.try_recv(), Ok(()) | Err(TryRecvError::Lagged(_))) {}
                if *stop.borrow() {
                    return None;
                }
                if !auth_still_valid(&web, auth.as_ref()).await {
                    return None;
                }
            }
            wait = true;
            let snap = match web.app.wf.snapshot() {
                Ok(v) => v.to_string(),
                Err(e) => {
                    tracing::warn!("board snapshot failed: {e:#}");
                    continue;
                }
            };
            if last.as_deref() != Some(snap.as_str()) {
                let ev = Event::default().event("state").data(snap.clone());
                return Some((Ok::<_, Infallible>(ev), (web, rx, stop, auth, Some(snap))));
            }
        }
    });
    Sse::new(s).keep_alive(KeepAlive::new().interval(KEEPALIVE).text("keepalive")).into_response()
}

#[derive(Deserialize)]
struct TextBody {
    text: String,
    #[serde(default)]
    at: Option<String>,
    #[serde(default)]
    token: Option<String>,
}

#[derive(Deserialize, Default)]
struct AtBody {
    #[serde(default)]
    at: Option<String>,
    #[serde(default)]
    token: Option<String>,
}

async fn answer(State(web): State<Web>, Path(id): Path<String>, raw: Bytes) -> Response {
    let b: TextBody = match body(&raw) {
        Ok(b) => b,
        Err(e) => return error(StatusCode::BAD_REQUEST, e),
    };
    match web.app.wf.answer_at(&id, &b.text, &CardRef { at: b.at.as_deref(), token: b.token.as_deref() }).await {
        Ok(v) => json_ok(StatusCode::OK, v),
        Err(e) => failed(&web.app, e),
    }
}

async fn dismiss(State(web): State<Web>, Path(id): Path<String>, raw: Bytes) -> Response {
    let b: AtBody = if raw.is_empty() {
        AtBody::default()
    } else {
        match body(&raw) {
            Ok(b) => b,
            Err(e) => return error(StatusCode::BAD_REQUEST, e),
        }
    };
    match web.app.wf.dismiss_at(&id, &CardRef { at: b.at.as_deref(), token: b.token.as_deref() }).await {
        Ok(v) => json_ok(StatusCode::OK, v),
        Err(e) => failed(&web.app, e),
    }
}

#[derive(Deserialize)]
struct LaunchBody {
    issues: Vec<String>,
}

async fn launch(State(web): State<Web>, raw: Bytes) -> Response {
    let b: LaunchBody = match body(&raw) {
        Ok(b) => b,
        Err(e) => return error(StatusCode::BAD_REQUEST, e),
    };
    let issues: Vec<String> = b.issues.into_iter().map(|i| i.trim().to_string()).filter(|i| !i.is_empty()).collect();
    if issues.is_empty() {
        return error(StatusCode::BAD_REQUEST, "no issues given");
    }
    if issues.len() > MAX_LAUNCH {
        return error(StatusCode::BAD_REQUEST, format!("at most {MAX_LAUNCH} issues at a time"));
    }
    let mut results = Vec::new();
    for issue in issues {
        let args = LaunchArgs { issue: issue.clone(), project: None, title: None, model: None, effort: None };
        let mut r = match web.app.wf.launch_ticket(args).await {
            Ok(v) => v,
            Err(e) => json!({"ok": false, "error": web.app.t3.redact(format!("{e:#}"))}),
        };
        r["issue"] = json!(issue);
        results.push(r);
    }
    json_ok(StatusCode::OK, json!({"results": results}))
}

#[derive(Deserialize)]
struct AskBody {
    question: String,
}

async fn ask(State(web): State<Web>, raw: Bytes) -> Response {
    let b: AskBody = match body(&raw) {
        Ok(b) => b,
        Err(e) => return error(StatusCode::BAD_REQUEST, e),
    };
    match web.app.ask(&b.question).await {
        Ok(v) => json_ok(StatusCode::OK, v),
        Err(e) => failed(&web.app, e),
    }
}

async fn settle_ticket(State(web): State<Web>, Path(thread): Path<String>) -> Response {
    match web.app.wf.settle_ticket(&thread).await {
        Ok(v) => json_ok(StatusCode::OK, v),
        Err(e) => failed(&web.app, e),
    }
}

async fn restore_ticket(State(web): State<Web>, Path(thread): Path<String>) -> Response {
    match web.app.wf.restore_ticket(&thread).await {
        Ok(v) => json_ok(StatusCode::OK, v),
        Err(e) => failed(&web.app, e),
    }
}

async fn settle_ask(State(web): State<Web>, Path(id): Path<String>) -> Response {
    match web.app.wf.settle_ask(&id) {
        Ok(v) => json_ok(StatusCode::OK, v),
        Err(e) => failed(&web.app, e),
    }
}

async fn restore_ask(State(web): State<Web>, Path(id): Path<String>) -> Response {
    match web.app.wf.restore_ask(&id) {
        Ok(v) => json_ok(StatusCode::OK, v),
        Err(e) => failed(&web.app, e),
    }
}

#[derive(Deserialize)]
struct OpenBody {
    link: String,
}

async fn open_link(State(web): State<Web>, raw: Bytes) -> Response {
    let b: OpenBody = match body(&raw) {
        Ok(b) => b,
        Err(e) => return error(StatusCode::BAD_REQUEST, e),
    };
    match web.app.open_link(&b.link).await {
        Ok(v) => json_ok(StatusCode::OK, v),
        Err(e) => failed(&web.app, e),
    }
}

async fn settle_done(State(web): State<Web>) -> Response {
    match web.app.wf.settle_closed(None).await {
        Ok(v) => json_ok(StatusCode::OK, v),
        Err(e) => failed(&web.app, e),
    }
}

async fn list_new_issues(State(web): State<Web>) -> Response {
    match web.app.wf.new_issues() {
        Ok((issues, dismissed)) => json_ok(StatusCode::OK, json!({"issues": issues, "dismissed": dismissed})),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")),
    }
}

async fn launch_new_issue(State(web): State<Web>, Path(id): Path<String>) -> Response {
    match web.app.wf.launch_new_issue(&id).await {
        Ok(v) => json_ok(StatusCode::OK, v),
        Err(e) => failed(&web.app, e),
    }
}

async fn dismiss_new_issue(State(web): State<Web>, Path(id): Path<String>) -> Response {
    match web.app.wf.dismiss_new_issue(&id).await {
        Ok(v) => json_ok(StatusCode::OK, v),
        Err(e) => failed(&web.app, e),
    }
}

async fn restore_new_issue(State(web): State<Web>, Path(id): Path<String>) -> Response {
    match web.app.wf.restore_new_issue(&id).await {
        Ok(v) => json_ok(StatusCode::OK, v),
        Err(e) => failed(&web.app, e),
    }
}

fn context_failed(app: &App, e: ContextError) -> Response {
    let status = match e {
        ContextError::Invalid(_) => StatusCode::BAD_REQUEST,
        ContextError::Missing(_) => StatusCode::NOT_FOUND,
        ContextError::Taken(_) => StatusCode::CONFLICT,
        ContextError::Store(_) => StatusCode::INTERNAL_SERVER_ERROR,
    };
    error(status, app.t3.redact(e.to_string()))
}

async fn list_contexts(State(web): State<Web>) -> Response {
    match web.app.wf.contexts() {
        Ok(v) => json_ok(StatusCode::OK, json!({"contexts": v})),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")),
    }
}

async fn put_context(State(web): State<Web>, Path(id): Path<String>, raw: Bytes) -> Response {
    let b: ContextBody = match body(&raw) {
        Ok(b) => b,
        Err(e) => return error(StatusCode::BAD_REQUEST, e),
    };
    match web.app.wf.put_context(&id, b) {
        Ok((created, c, rev)) => {
            json_ok(if created { StatusCode::CREATED } else { StatusCode::OK }, json!({"context": c, "rev": rev}))
        }
        Err(e) => context_failed(&web.app, e),
    }
}

async fn delete_context(State(web): State<Web>, Path(id): Path<String>) -> Response {
    match web.app.wf.delete_context(&id) {
        Ok(rev) => json_ok(StatusCode::OK, json!({"deleted": id, "rev": rev})),
        Err(e) => context_failed(&web.app, e),
    }
}

async fn get_models(State(web): State<Web>) -> Response {
    match web.app.wf.models_view().await {
        Ok(v) => json_ok(StatusCode::OK, v),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, web.app.t3.redact(format!("{e:#}"))),
    }
}

async fn put_ticket_model(State(web): State<Web>, raw: Bytes) -> Response {
    let b: ModelSettingBody = match body(&raw) {
        Ok(b) => b,
        Err(e) => return error(StatusCode::BAD_REQUEST, e),
    };
    match web.app.wf.set_ticket_model(b).await {
        Ok(v) => json_ok(StatusCode::OK, v),
        Err(e) => failed(&web.app, e),
    }
}

async fn put_ask_model(State(web): State<Web>, raw: Bytes) -> Response {
    let b: ModelSettingBody = match body(&raw) {
        Ok(b) => b,
        Err(e) => return error(StatusCode::BAD_REQUEST, e),
    };
    match web.app.wf.set_ask_model(b).await {
        Ok(v) => json_ok(StatusCode::OK, v),
        Err(e) => failed(&web.app, e),
    }
}

pub fn routes(app: Arc<App>, port: u16) -> Router {
    let remote = app.config.board.active_remote_config().map(|(cfg, origin)| (cfg.clone(), origin));
    let web = Web { app, port, remote };
    let board = Router::new()
        .route("/", get(page))
        .route("/api/state", get(state))
        .route("/api/events", get(events))
        .route("/api/msg/{handle}", get(message))
        .route("/api/cards/{id}/answer", post(answer))
        .route("/api/cards/{id}/dismiss", post(dismiss))
        .route("/api/launch", post(launch))
        .route("/api/ask", post(ask))
        .route("/api/models", get(get_models))
        .route("/api/models/ticket", put(put_ticket_model))
        .route("/api/models/ask", put(put_ask_model))
        .route("/api/tickets/{thread}/settle", post(settle_ticket))
        .route("/api/tickets/{thread}/restore", post(restore_ticket))
        .route("/api/asks/{id}/settle", post(settle_ask))
        .route("/api/asks/{id}/restore", post(restore_ask))
        .route("/api/settle-done", post(settle_done))
        .route("/api/new-issues", get(list_new_issues))
        .route("/api/new-issues/{id}/launch", post(launch_new_issue))
        .route("/api/new-issues/{id}/dismiss", post(dismiss_new_issue))
        .route("/api/new-issues/{id}/restore", post(restore_new_issue))
        .route("/api/open", post(open_link))
        .route("/api/contexts", get(list_contexts))
        .route("/api/contexts/{id}", put(put_context).delete(delete_context))
        .route_layer(axum::middleware::from_fn_with_state(web.clone(), board_guard))
        .with_state(web.clone());
    let pair = Router::new()
        .route("/pair", get(pair_page))
        .route("/api/pair", post(post_pair))
        .route_layer(axum::middleware::from_fn_with_state(web.clone(), pair_guard))
        .with_state(web);
    board.merge(pair)
}
