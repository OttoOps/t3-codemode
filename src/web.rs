use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, Method, StatusCode, header};
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
use crate::config::{RemoteMode, RemoteOrigin};
use crate::contexts::{ContextBody, ContextError};
use crate::server::{RequestKind, SingleHeader, classify_request, is_own_loopback_host_str, single_header};

pub const CSRF_HEADER: &str = "x-t3cm-csrf";
const PAGE: &str = include_str!("board.html");
const MAX_LAUNCH: usize = 20;
const KEEPALIVE: Duration = Duration::from_secs(25);
const SETTLE: Duration = Duration::from_millis(150);
const CSP: &str = "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; connect-src 'self'; \
img-src 'self' data:; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

#[derive(Clone)]
struct Web {
    app: Arc<App>,
    port: u16,
    remote: Option<(RemoteMode, RemoteOrigin)>,
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

    fn check_remote_host_and_origin(&self, headers: &HeaderMap, origin: &RemoteOrigin) -> Option<Response> {
        match single_header(headers, "x-forwarded-proto") {
            SingleHeader::Missing => {}
            SingleHeader::Present(proto) if proto.eq_ignore_ascii_case("https") => {}
            SingleHeader::Present(_) | SingleHeader::Invalid => {
                return Some(error(StatusCode::FORBIDDEN, "remote board requests must arrive over https"));
            }
        }
        let SingleHeader::Present(host) = single_header(headers, header::HOST.as_str()) else {
            return Some(error(StatusCode::FORBIDDEN, "remote request Host does not match board.remote.origin"));
        };
        let xfh = match single_header(headers, "x-forwarded-host") {
            SingleHeader::Missing => None,
            SingleHeader::Present(v) => Some(v),
            SingleHeader::Invalid => {
                return Some(error(StatusCode::FORBIDDEN, "remote request Host does not match board.remote.origin"));
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
            return Some(error(StatusCode::FORBIDDEN, "remote request Host does not match board.remote.origin"));
        }
        match single_header(headers, header::ORIGIN.as_str()) {
            SingleHeader::Missing => {}
            SingleHeader::Present(req_origin) if origin.matches_origin(req_origin) => {}
            SingleHeader::Present(_) | SingleHeader::Invalid => {
                return Some(error(StatusCode::FORBIDDEN, "cross-origin requests are refused"));
            }
        }
        None
    }

    /// Authenticate a remote board request. Pairing-cookie and Tailscale Serve
    /// auth modes slot in here; until an auth mode is implemented, every
    /// remote request is refused.
    fn authenticate_remote(&self, mode: RemoteMode, _headers: &HeaderMap) -> Option<Response> {
        let _ = mode;
        Some(error(StatusCode::UNAUTHORIZED, "remote board authentication is required"))
    }

    fn check_remote(&self, headers: &HeaderMap) -> Option<Response> {
        let Some((mode, origin)) = &self.remote else {
            return Some(error(StatusCode::FORBIDDEN, "remote board access is disabled"));
        };
        if let Some(r) = self.check_remote_host_and_origin(headers, origin) {
            return Some(r);
        }
        self.authenticate_remote(*mode, headers)
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

async fn board_guard(State(web): State<Web>, req: axum::extract::Request, next: axum::middleware::Next) -> Response {
    let headers = req.headers();
    let mutating = is_mutating(req.method());
    match classify_request(headers, web.port) {
        RequestKind::Local => {
            if let Some(r) = web.check_local_origin(headers) {
                return r;
            }
        }
        RequestKind::Remote => {
            if let Some(r) = web.check_remote(headers) {
                return r;
            }
        }
    }
    if mutating && let Some(r) = web.check_csrf(headers) {
        return r;
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

async fn events(State(web): State<Web>) -> Response {
    let rx = web.app.wf.subscribe();
    let stop = web.app.stopping.subscribe();
    let s = stream::unfold((web.app.clone(), rx, stop, None::<String>), |(app, mut rx, mut stop, last)| async move {
        let mut wait = last.is_some();
        loop {
            if *stop.borrow() {
                return None;
            }
            if wait {
                tokio::select! {
                    r = rx.recv() => match r {
                        Ok(()) | Err(RecvError::Lagged(_)) => {}
                        Err(RecvError::Closed) => return None,
                    },
                    _ = stop.wait_for(|s| *s) => return None,
                }
                tokio::time::sleep(SETTLE).await;
                while matches!(rx.try_recv(), Ok(()) | Err(TryRecvError::Lagged(_))) {}
            }
            wait = true;
            let snap = match app.wf.snapshot() {
                Ok(v) => v.to_string(),
                Err(e) => {
                    tracing::warn!("board snapshot failed: {e:#}");
                    continue;
                }
            };
            if last.as_deref() != Some(snap.as_str()) {
                let ev = Event::default().event("state").data(snap.clone());
                return Some((Ok::<_, Infallible>(ev), (app, rx, stop, Some(snap))));
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
        Ok((created, c)) => json_ok(if created { StatusCode::CREATED } else { StatusCode::OK }, json!({"context": c})),
        Err(e) => context_failed(&web.app, e),
    }
}

async fn delete_context(State(web): State<Web>, Path(id): Path<String>) -> Response {
    match web.app.wf.delete_context(&id) {
        Ok(()) => json_ok(StatusCode::OK, json!({"deleted": id})),
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
    let remote = app.config.board.active_remote();
    let web = Web { app, port, remote };
    Router::new()
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
        .with_state(web)
}
