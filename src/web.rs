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
use crate::board::{CardRef, LaunchArgs, ModelSettingBody, T3Unreachable};
use crate::clock::iso;
use crate::config::{BoardRemoteConfig, RemoteMode, RemoteOrigin};
use crate::config_edit::{Rejected, Stale};
use crate::contexts::{ContextBody, ContextError};
use crate::flow_triage::{TriageApplyArgs, TriageDismissArgs, TriageRunning};
use crate::issues::{FlowEdgeArgs, FlowIssueUpdateArgs};
use crate::pairing::{self, CookieExtract};
use crate::server::{RequestKind, SingleHeader, classify_request, is_own_loopback_host_str, single_header};
use crate::settings::SettingsBody;

pub const CSRF_HEADER: &str = "x-t3cm-csrf";
const PAGE: &str = include_str!("board.html");
const PAIR_PAGE: &str = include_str!("pair.html");
const ICON_SVG: &str = include_str!("icon.svg");
const ICON_192_PNG: &[u8] = include_bytes!("icon-192.png");
const ICON_512_PNG: &[u8] = include_bytes!("icon-512.png");
const MANIFEST_JSON: &str = concat!(
    r##"{"name":"t3 board","short_name":"t3 board","description":"T3 Code workflow board","##,
    r##""start_url":"/","scope":"/","display":"standalone","##,
    r##""background_color":"#0f1113","theme_color":"#181b1f","##,
    r##""icons":["##,
    r##"{"src":"/icon.svg","sizes":"any","type":"image/svg+xml","purpose":"any maskable"},"##,
    r##"{"src":"/icon-192.png","sizes":"192x192","type":"image/png","purpose":"any"},"##,
    r##"{"src":"/icon-512.png","sizes":"512x512","type":"image/png","purpose":"any"}"##,
    r##"]}"##,
);
const MAX_LAUNCH: usize = 20;
const KEEPALIVE: Duration = Duration::from_secs(25);
const SETTLE: Duration = Duration::from_millis(150);
const SESSION_CHECK: Duration = Duration::from_secs(1);
const CSP: &str = "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; connect-src 'self'; \
img-src 'self' data:; manifest-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";
const STATIC_CSP: &str = "default-src 'none'; frame-ancestors 'none'";
const AUTH_PAGE_CSP: &str =
    "default-src 'none'; style-src 'unsafe-inline'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

#[derive(Clone, Debug)]
struct AuthSession {
    seq: Option<i64>,
    tailscale_login: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AuthErrorKind {
    RemoteDisabled,
    OriginMismatch,
    PairingRequired,
    SessionInvalid,
    TailscaleMissing,
    TailscaleDenied,
    TailscaleUnavailable,
    Internal,
}

#[derive(Clone, Debug)]
struct AuthError {
    status: StatusCode,
    message: &'static str,
    kind: AuthErrorKind,
    login: Option<String>,
}

impl AuthError {
    fn plain(status: StatusCode, message: &'static str, kind: AuthErrorKind) -> Self {
        Self { status, message, kind, login: None }
    }

    fn tailscale_denied(login: &str) -> Self {
        Self {
            status: StatusCode::FORBIDDEN,
            message: "Tailscale user is not allowed to access this board",
            kind: AuthErrorKind::TailscaleDenied,
            login: Some(login.to_string()),
        }
    }

    fn supports_html_screen(&self) -> bool {
        matches!(
            self.kind,
            AuthErrorKind::PairingRequired
                | AuthErrorKind::SessionInvalid
                | AuthErrorKind::TailscaleMissing
                | AuthErrorKind::TailscaleDenied
                | AuthErrorKind::TailscaleUnavailable
        )
    }

    fn into_json_response(self) -> Response {
        let auth = match self.kind {
            AuthErrorKind::PairingRequired => Some(json!({
                "kind": "pairing",
                "reason": "missing",
                "pairingUrl": "/pair",
            })),
            AuthErrorKind::SessionInvalid => Some(json!({
                "kind": "pairing",
                "reason": "expired",
                "pairingUrl": "/pair",
            })),
            AuthErrorKind::TailscaleMissing => Some(json!({"kind": "tailscale_missing"})),
            AuthErrorKind::TailscaleDenied => Some(json!({
                "kind": "tailscale_denied",
                "login": self.login,
            })),
            AuthErrorKind::TailscaleUnavailable => Some(json!({"kind": "tailscale_unavailable"})),
            AuthErrorKind::RemoteDisabled => Some(json!({"kind": "remote_disabled"})),
            AuthErrorKind::OriginMismatch | AuthErrorKind::Internal => None,
        };
        let body = match auth {
            Some(a) => json!({"error": self.message, "auth": a}),
            None => json!({"error": self.message}),
        };
        json_ok(self.status, body)
    }

    fn into_html_response(self) -> Response {
        let (title, lead, hint, cmd, show_pair) = match self.kind {
            AuthErrorKind::PairingRequired => (
                "Sign in to t3 board",
                "This browser needs a remote pairing session to open the board.".to_string(),
                "Run on the host, then scan the QR code or open the pairing link:",
                "t3cm board pair",
                true,
            ),
            AuthErrorKind::SessionInvalid => (
                "Session expired or revoked",
                "This browser's remote session is no longer valid. Pair this device again to continue.".to_string(),
                "Run on the host, then scan the QR code or open the pairing link:",
                "t3cm board pair",
                true,
            ),
            AuthErrorKind::TailscaleDenied => {
                let lead = match self.login.as_deref() {
                    Some(login) => format!(
                        "Signed in via Tailscale as <code>{}</code>, which is not allowed to access this board.",
                        escape_html(login)
                    ),
                    None => "Your Tailscale account is not allowed to access this board.".to_string(),
                };
                (
                    "Access denied",
                    lead,
                    "Switch Tailscale accounts on this device, or set board.remote.allowUsers in t3cm's config.json on the host (include the host's own Tailscale login too, since a non-empty list replaces the default) and restart t3cm.",
                    "",
                    false,
                )
            }
            AuthErrorKind::TailscaleUnavailable => (
                "Tailscale sign-in required",
                "The host could not verify its Tailscale login to authorize this request.".to_string(),
                "Check Tailscale status and remote board health on the host:",
                "t3cm doctor",
                false,
            ),
            AuthErrorKind::TailscaleMissing
            | AuthErrorKind::RemoteDisabled
            | AuthErrorKind::OriginMismatch
            | AuthErrorKind::Internal => (
                "Tailscale sign-in required",
                "This request did not carry a Tailscale user identity header.".to_string(),
                "Open the board through its Tailscale Serve HTTPS URL, or check the host:",
                "t3cm doctor",
                false,
            ),
        };
        let pair_btn = if show_pair { r#"<a class="btn go" href="/pair">Open pairing page</a>"# } else { "" };
        let retry_cls = if show_pair { "btn" } else { "btn go" };
        let cmd_block =
            if cmd.is_empty() { String::new() } else { format!(r#"<code class="cmd">{}</code>"#, escape_html(cmd)) };
        let html = format!(
            r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="color-scheme" content="light dark">
<title>{title} - t3 board</title>
<style>
  :root {{
    --canvas: #f3f3ef;
    --panel: #ffffff;
    --panel-2: #f7f7f4;
    --line: #83878d;
    --field: #7d8187;
    --ink: #1b1d20;
    --ink-2: #565b62;
    --go: #2f5bd3;
    --go-ink: #ffffff;
    --go-hover: #2549b0;
    --mono: ui-monospace, "SF Mono", "Cascadia Mono", Menlo, Consolas, monospace;
  }}
  @media (prefers-color-scheme: dark) {{
    :root {{
      --canvas: #0f1113;
      --panel: #181b1f;
      --panel-2: #1d2125;
      --line: #6e747c;
      --field: #7d848c;
      --ink: #eceef0;
      --ink-2: #a9aeb5;
      --go: #4268dc;
      --go-ink: #ffffff;
      --go-hover: #3a5fd0;
    }}
  }}
  * {{ box-sizing: border-box; }}
  body {{
    margin: 0;
    min-height: 100vh;
    min-height: 100dvh;
    display: grid;
    place-items: center;
    padding: 24px 16px max(24px, env(safe-area-inset-bottom, 0px));
    background: var(--canvas);
    color: var(--ink);
    font: 400 15px/1.5 system-ui, -apple-system, "Segoe UI", Roboto, sans-serif;
  }}
  main {{
    width: 100%;
    max-width: 460px;
    background: var(--panel);
    border: 1px solid var(--line);
    border-radius: 8px;
    padding: 24px;
    display: grid;
    gap: 12px;
  }}
  h1 {{ margin: 0; font-size: 19px; line-height: 24px; font-weight: 600; }}
  p {{ margin: 0; color: var(--ink-2); font-size: 14px; line-height: 20px; }}
  code {{
    font-family: var(--mono);
    font-size: 13px;
    color: var(--ink);
  }}
  .cmd {{
    display: block;
    padding: 8px 10px;
    border: 1px solid var(--line);
    border-radius: 6px;
    background: var(--panel-2);
    overflow-wrap: anywhere;
    user-select: all;
  }}
  .acts {{ display: flex; flex-wrap: wrap; gap: 8px; margin-top: 4px; }}
  .btn {{
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-height: 36px;
    padding: 0 14px;
    border-radius: 6px;
    border: 1px solid var(--field);
    background: transparent;
    color: var(--ink);
    text-decoration: none;
    font-size: 14px;
    font-weight: 500;
  }}
  .btn:hover {{ background: var(--panel-2); }}
  .btn.go {{ background: var(--go); border-color: var(--go); color: var(--go-ink); font-weight: 600; }}
  .btn.go:hover {{ background: var(--go-hover); border-color: var(--go-hover); }}
  @media (max-width: 600px) {{
    .acts {{ flex-direction: column; }}
    .btn {{ min-height: 44px; width: 100%; }}
  }}
</style>
</head>
<body>
<main>
  <h1>{title}</h1>
  <p>{lead}</p>
  <p>{hint}</p>
  {cmd_block}
  <div class="acts">{pair_btn}<a class="{retry_cls}" href="/">Try again</a></div>
</main>
</body>
</html>"#,
            title = escape_html(title),
            lead = lead,
            hint = escape_html(hint),
            cmd_block = cmd_block,
            pair_btn = pair_btn,
            retry_cls = retry_cls,
        );
        (
            self.status,
            [
                (header::CONTENT_TYPE, "text/html; charset=utf-8"),
                (header::CACHE_CONTROL, "no-store"),
                (header::CONTENT_SECURITY_POLICY, AUTH_PAGE_CSP),
                (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
                (header::REFERRER_POLICY, "no-referrer"),
            ],
            html,
        )
            .into_response()
    }
}

fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

fn accepts_html(headers: &HeaderMap) -> bool {
    headers.get_all(header::ACCEPT).iter().filter_map(|v| v.to_str().ok()).any(|v| {
        v.split(',').any(|part| {
            let mut segs = part.split(';');
            let media = segs.next().map(str::trim).unwrap_or("");
            if !media.eq_ignore_ascii_case("text/html") {
                return false;
            }
            !segs.any(|p| {
                let p = p.trim();
                p.strip_prefix("q=")
                    .or_else(|| p.strip_prefix("Q="))
                    .is_some_and(|q| q.trim().parse::<f32>().ok().is_some_and(|val| val == 0.0))
            })
        })
    })
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

    fn check_remote_host_and_origin(&self, headers: &HeaderMap, origin: &RemoteOrigin) -> Result<(), AuthError> {
        match single_header(headers, "x-forwarded-proto") {
            SingleHeader::Missing => {}
            SingleHeader::Present(proto) if proto.eq_ignore_ascii_case("https") => {}
            SingleHeader::Present(_) | SingleHeader::Invalid => {
                return Err(AuthError::plain(
                    StatusCode::FORBIDDEN,
                    "remote board requests must arrive over https",
                    AuthErrorKind::OriginMismatch,
                ));
            }
        }
        let SingleHeader::Present(host) = single_header(headers, header::HOST.as_str()) else {
            return Err(AuthError::plain(
                StatusCode::FORBIDDEN,
                "remote request Host does not match board.remote.origin",
                AuthErrorKind::OriginMismatch,
            ));
        };
        let xfh = match single_header(headers, "x-forwarded-host") {
            SingleHeader::Missing => None,
            SingleHeader::Present(v) => Some(v),
            SingleHeader::Invalid => {
                return Err(AuthError::plain(
                    StatusCode::FORBIDDEN,
                    "remote request Host does not match board.remote.origin",
                    AuthErrorKind::OriginMismatch,
                ));
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
            return Err(AuthError::plain(
                StatusCode::FORBIDDEN,
                "remote request Host does not match board.remote.origin",
                AuthErrorKind::OriginMismatch,
            ));
        }
        match single_header(headers, header::ORIGIN.as_str()) {
            SingleHeader::Missing => {}
            SingleHeader::Present(req_origin) if origin.matches_origin(req_origin) => {}
            SingleHeader::Present(_) | SingleHeader::Invalid => {
                return Err(AuthError::plain(
                    StatusCode::FORBIDDEN,
                    "cross-origin requests are refused",
                    AuthErrorKind::OriginMismatch,
                ));
            }
        }
        Ok(())
    }

    fn pairing_enabled(&self) -> bool {
        self.remote.as_ref().is_some_and(|(cfg, _)| cfg.pairing_enabled())
    }

    fn authenticate_pairing_cookie(&self, headers: &HeaderMap) -> Result<AuthSession, AuthError> {
        let raw = match pairing::extract_session_cookie(headers) {
            CookieExtract::Present(v) => v,
            CookieExtract::Missing => {
                return Err(AuthError::plain(
                    StatusCode::UNAUTHORIZED,
                    "remote board authentication is required",
                    AuthErrorKind::PairingRequired,
                ));
            }
            CookieExtract::Invalid => {
                return Err(AuthError::plain(
                    StatusCode::UNAUTHORIZED,
                    "remote board session is invalid or expired",
                    AuthErrorKind::SessionInvalid,
                ));
            }
        };
        let hash = pairing::hash_token(raw);
        let now = iso(self.app.clock.now());
        match self.app.store.board_session_verify(&hash, &now) {
            Ok(Some(session)) => Ok(AuthSession { seq: Some(session.seq), tailscale_login: None }),
            Ok(None) => Err(AuthError::plain(
                StatusCode::UNAUTHORIZED,
                "remote board session is invalid or expired",
                AuthErrorKind::SessionInvalid,
            )),
            Err(e) => {
                tracing::warn!("board session lookup failed: {e:#}");
                Err(AuthError::plain(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "session lookup failed",
                    AuthErrorKind::Internal,
                ))
            }
        }
    }

    async fn authenticate_tailscale(&self, cfg: &BoardRemoteConfig, headers: &HeaderMap) -> Result<String, AuthError> {
        let login = match single_header(headers, "tailscale-user-login") {
            SingleHeader::Present(v)
                if !v.is_empty()
                    && v == v.trim()
                    && !v.bytes().any(|b| b.is_ascii_whitespace() || b.is_ascii_control()) =>
            {
                v
            }
            SingleHeader::Missing | SingleHeader::Present(_) | SingleHeader::Invalid => {
                return Err(AuthError::plain(
                    StatusCode::UNAUTHORIZED,
                    "remote board authentication is required",
                    AuthErrorKind::TailscaleMissing,
                ));
            }
        };
        if !cfg.allow_users.is_empty() {
            if cfg.allow_users.iter().any(|u| u.eq_ignore_ascii_case(login)) {
                return Ok(login.to_string());
            }
            return Err(AuthError::tailscale_denied(login));
        }
        match self.app.tailscale_host_login_async().await {
            Ok(Some(host_login)) => {
                if host_login.eq_ignore_ascii_case(login) {
                    Ok(login.to_string())
                } else {
                    Err(AuthError::tailscale_denied(login))
                }
            }
            Ok(None) => Err(AuthError::plain(
                StatusCode::UNAUTHORIZED,
                "Tailscale host login is unavailable",
                AuthErrorKind::TailscaleUnavailable,
            )),
            Err(e) => {
                tracing::warn!("tailscale status lookup failed: {e}");
                Err(AuthError::plain(
                    StatusCode::UNAUTHORIZED,
                    "Tailscale host login is unavailable",
                    AuthErrorKind::TailscaleUnavailable,
                ))
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
    ) -> Result<Option<AuthSession>, AuthError> {
        match cfg.mode {
            RemoteMode::Pairing => self.authenticate_pairing_cookie(headers).map(Some),
            RemoteMode::Tailscale => {
                let login = self.authenticate_tailscale(cfg, headers).await?;
                let seq = if cfg.require_pairing { self.authenticate_pairing_cookie(headers)?.seq } else { None };
                Ok(Some(AuthSession { seq, tailscale_login: Some(login) }))
            }
            RemoteMode::Off => Err(AuthError::plain(
                StatusCode::FORBIDDEN,
                "remote board access is disabled",
                AuthErrorKind::RemoteDisabled,
            )),
        }
    }

    async fn check_remote(&self, headers: &HeaderMap) -> Result<Option<AuthSession>, AuthError> {
        let Some((cfg, origin)) = &self.remote else {
            return Err(AuthError::plain(
                StatusCode::FORBIDDEN,
                "remote board access is disabled",
                AuthErrorKind::RemoteDisabled,
            ));
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
    let is_root_html = req.method() == Method::GET && req.uri().path() == "/" && accepts_html(req.headers());
    let headers = req.headers();
    let mutating = is_mutating(req.method());
    let kind = classify_request(headers, web.port);
    let session = match kind {
        RequestKind::Local => {
            if let Some(r) = web.check_local_origin(headers) {
                return r;
            }
            None
        }
        RequestKind::Remote => match web.check_remote(headers).await {
            Ok(session) => session,
            Err(err) => {
                if is_root_html && err.supports_html_screen() {
                    return err.into_html_response();
                }
                return err.into_json_response();
            }
        },
    };
    if mutating && let Some(r) = web.check_csrf(headers) {
        return r;
    }
    req.extensions_mut().insert(kind);
    if let Some(session) = session {
        req.extensions_mut().insert(session);
    }
    next.run(req).await
}

#[derive(Clone, Debug)]
struct SettingsLogin(String);

async fn settings_guard(
    State(web): State<Web>,
    Extension(kind): Extension<RequestKind>,
    session: Option<Extension<AuthSession>>,
    mut req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    if kind == RequestKind::Local {
        return next.run(req).await;
    }
    let login = session.and_then(|Extension(s)| s.tailscale_login);
    match web.settings_owner(req.headers(), login).await {
        Ok(login) => {
            req.extensions_mut().insert(SettingsLogin(login));
            next.run(req).await
        }
        Err(why) => error(StatusCode::FORBIDDEN, why),
    }
}

impl Web {
    async fn settings_owner(&self, headers: &HeaderMap, login: Option<String>) -> Result<String, &'static str> {
        let Some((cfg, _)) = self.remote.as_ref().filter(|(cfg, _)| cfg.mode == RemoteMode::Tailscale) else {
            return Err("board settings are only allowed from loopback or, in Tailscale mode, the board's owner");
        };
        if !matches!(single_header(headers, "x-forwarded-proto"), SingleHeader::Present(p) if p.eq_ignore_ascii_case("https"))
        {
            return Err("remote board settings need X-Forwarded-Proto: https");
        }
        let Some(login) = login else {
            return Err("remote board settings need a Tailscale login");
        };
        let owner = if cfg.settings_users.is_empty() {
            matches!(
                self.app.tailscale_host_login_async().await,
                Ok(Some(ref host)) if host.eq_ignore_ascii_case(&login)
            )
        } else {
            cfg.settings_users.iter().any(|u| u.eq_ignore_ascii_case(&login))
        };
        if owner {
            Ok(login)
        } else {
            Err("this Tailscale login may not open board settings; see board.remote.settingsUsers")
        }
    }
}

async fn pair_guard(State(web): State<Web>, req: axum::extract::Request, next: axum::middleware::Next) -> Response {
    let is_pair_html = req.method() == Method::GET && req.uri().path() == "/pair" && accepts_html(req.headers());
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
                return AuthError::plain(
                    StatusCode::FORBIDDEN,
                    "remote board access is disabled",
                    AuthErrorKind::RemoteDisabled,
                )
                .into_json_response();
            };
            if let Err(err) = web.check_remote_host_and_origin(headers, origin) {
                return err.into_json_response();
            }
            if !web.pairing_enabled() {
                return error(StatusCode::FORBIDDEN, "remote pairing is not enabled");
            }
            if cfg.mode == RemoteMode::Tailscale
                && let Err(err) = web.authenticate_tailscale(cfg, headers).await
            {
                if is_pair_html && err.supports_html_screen() {
                    return err.into_html_response();
                }
                return err.into_json_response();
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
    let missing = ["no open board card", "no board ticket", "no ask", "no new issue", "no triage"]
        .iter()
        .any(|p| msg.starts_with(p));
    let status = if missing { StatusCode::NOT_FOUND } else { StatusCode::BAD_REQUEST };
    error(status, msg)
}

async fn page(State(web): State<Web>, Extension(kind): Extension<RequestKind>) -> Response {
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
        PAGE.replace("{{CSRF}}", &web.app.csrf)
            .replace("{{VERSION}}", env!("CARGO_PKG_VERSION"))
            .replace("{{LOCAL}}", &(kind == RequestKind::Local).to_string()),
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
#[serde(untagged)]
enum LaunchItem {
    Ref(String),
    WithProject(LaunchItemObj),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LaunchItemObj {
    issue: String,
    #[serde(default)]
    project: Option<String>,
}

#[derive(Deserialize)]
struct LaunchBody {
    issues: Vec<LaunchItem>,
}

async fn launch(State(web): State<Web>, raw: Bytes) -> Response {
    let b: LaunchBody = match body(&raw) {
        Ok(b) => b,
        Err(e) => return error(StatusCode::BAD_REQUEST, e),
    };
    let issues: Vec<(String, Option<String>)> = b
        .issues
        .into_iter()
        .filter_map(|item| {
            let (issue, project) = match item {
                LaunchItem::Ref(issue) => (issue, None),
                LaunchItem::WithProject(LaunchItemObj { issue, project }) => (issue, project),
            };
            let issue = issue.trim().to_string();
            if issue.is_empty() {
                return None;
            }
            let project = project.map(|p| p.trim().to_string()).filter(|p| !p.is_empty());
            Some((issue, project))
        })
        .collect();
    if issues.is_empty() {
        return error(StatusCode::BAD_REQUEST, "no issues given");
    }
    if issues.len() > MAX_LAUNCH {
        return error(StatusCode::BAD_REQUEST, format!("at most {MAX_LAUNCH} issues at a time"));
    }
    let mut results = Vec::new();
    for (issue, project) in issues {
        let args = LaunchArgs {
            issue: issue.clone(),
            parent_issue: None,
            part: None,
            project,
            title: None,
            model: None,
            effort: None,
        };
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

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct CloseTicketBody {
    #[serde(default)]
    notify: bool,
}

async fn close_ticket(State(web): State<Web>, Path(thread): Path<String>, raw: Bytes) -> Response {
    let b: CloseTicketBody = if raw.is_empty() {
        CloseTicketBody::default()
    } else {
        match body(&raw) {
            Ok(b) => b,
            Err(e) => return error(StatusCode::BAD_REQUEST, e),
        }
    };
    match web.app.wf.close_ticket(&thread, b.notify).await {
        Ok(v) => json_ok(StatusCode::OK, v),
        Err(e) if e.is::<T3Unreachable>() => {
            error(StatusCode::SERVICE_UNAVAILABLE, web.app.t3.redact(format!("{e:#}")))
        }
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

async fn pause_link(State(web): State<Web>, Path(thread): Path<String>) -> Response {
    match web.app.wf.pause_link(&thread, true).await {
        Ok(v) => json_ok(StatusCode::OK, v),
        Err(e) => failed(&web.app, e),
    }
}

async fn resume_link(State(web): State<Web>, Path(thread): Path<String>) -> Response {
    match web.app.wf.pause_link(&thread, false).await {
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

async fn manifest() -> Response {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/manifest+json; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
            (header::CONTENT_SECURITY_POLICY, STATIC_CSP),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            (header::REFERRER_POLICY, "no-referrer"),
        ],
        MANIFEST_JSON,
    )
        .into_response()
}

async fn icon_svg() -> Response {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "image/svg+xml; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
            (header::CONTENT_SECURITY_POLICY, STATIC_CSP),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            (header::REFERRER_POLICY, "no-referrer"),
        ],
        ICON_SVG,
    )
        .into_response()
}

async fn icon_192() -> Response {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "image/png"),
            (header::CACHE_CONTROL, "no-store"),
            (header::CONTENT_SECURITY_POLICY, STATIC_CSP),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            (header::REFERRER_POLICY, "no-referrer"),
        ],
        ICON_192_PNG,
    )
        .into_response()
}

async fn icon_512() -> Response {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "image/png"),
            (header::CACHE_CONTROL, "no-store"),
            (header::CONTENT_SECURITY_POLICY, STATIC_CSP),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            (header::REFERRER_POLICY, "no-referrer"),
        ],
        ICON_512_PNG,
    )
        .into_response()
}

async fn open_link(State(web): State<Web>, Extension(kind): Extension<RequestKind>, raw: Bytes) -> Response {
    if kind == RequestKind::Remote {
        return error(StatusCode::FORBIDDEN, "opening desktop links is only allowed from loopback");
    }
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

async fn get_flow(State(web): State<Web>) -> Response {
    match web.app.wf.flow(true).await {
        Ok(v) => json_ok(StatusCode::OK, v),
        Err(e) => error(StatusCode::BAD_GATEWAY, web.app.t3.redact(format!("{e:#}"))),
    }
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct SyncFlowLinksBody {}

async fn post_flow_sync_links(State(web): State<Web>, raw: Bytes) -> Response {
    if !raw.is_empty()
        && let Err(e) = body::<SyncFlowLinksBody>(&raw)
    {
        return error(StatusCode::BAD_REQUEST, e);
    }
    match web.app.wf.sync_flow_links().await {
        Ok(res) => match serde_json::to_value(res) {
            Ok(v) => json_ok(StatusCode::OK, web.app.redact_value(v)),
            Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, web.app.t3.redact(format!("{e:#}"))),
        },
        Err(e) => error(StatusCode::BAD_GATEWAY, web.app.t3.redact(format!("{e:#}"))),
    }
}

async fn post_flow_edge(State(web): State<Web>, raw: Bytes) -> Response {
    let b: FlowEdgeArgs = match body(&raw) {
        Ok(b) => b,
        Err(e) => return error(StatusCode::BAD_REQUEST, e),
    };
    match web.app.wf.edit_flow_edge(b).await {
        Ok(res) => match serde_json::to_value(res) {
            Ok(v) => json_ok(StatusCode::OK, web.app.redact_value(v)),
            Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, web.app.t3.redact(format!("{e:#}"))),
        },
        Err(e) => failed(&web.app, e),
    }
}

async fn post_flow_issue(State(web): State<Web>, raw: Bytes) -> Response {
    let b: FlowIssueUpdateArgs = match body(&raw) {
        Ok(b) => b,
        Err(e) => return error(StatusCode::BAD_REQUEST, e),
    };
    match web.app.wf.update_flow_issue(b).await {
        Ok(res) => match serde_json::to_value(res) {
            Ok(v) => json_ok(StatusCode::OK, web.app.redact_value(v)),
            Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, web.app.t3.redact(format!("{e:#}"))),
        },
        Err(e) => failed(&web.app, e),
    }
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct TriageStartBody {}

async fn post_flow_triage(State(web): State<Web>, raw: Bytes) -> Response {
    if !raw.is_empty()
        && let Err(e) = body::<TriageStartBody>(&raw)
    {
        return error(StatusCode::BAD_REQUEST, e);
    }
    match web.app.start_flow_triage().await {
        Ok(v) => json_ok(StatusCode::OK, web.app.redact_value(v)),
        Err(e) if e.is::<TriageRunning>() => error(StatusCode::CONFLICT, web.app.t3.redact(format!("{e:#}"))),
        Err(e) => failed(&web.app, e),
    }
}

async fn post_flow_triage_apply(State(web): State<Web>, Path(id): Path<String>, raw: Bytes) -> Response {
    let b: TriageApplyArgs = match body(&raw) {
        Ok(b) => b,
        Err(e) => return error(StatusCode::BAD_REQUEST, e),
    };
    match web.app.wf.apply_flow_triage(&id, b).await {
        Ok(res) => match serde_json::to_value(res) {
            Ok(v) => json_ok(StatusCode::OK, web.app.redact_value(v)),
            Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, web.app.t3.redact(format!("{e:#}"))),
        },
        Err(e) => failed(&web.app, e),
    }
}

async fn post_flow_triage_dismiss(State(web): State<Web>, Path(id): Path<String>, raw: Bytes) -> Response {
    let b: TriageDismissArgs = if raw.is_empty() {
        TriageDismissArgs { change_ids: Vec::new(), all: true }
    } else {
        match body(&raw) {
            Ok(b) => b,
            Err(e) => return error(StatusCode::BAD_REQUEST, e),
        }
    };
    match web.app.wf.dismiss_flow_triage(&id, b).await {
        Ok(v) => json_ok(StatusCode::OK, web.app.redact_value(v)),
        Err(e) => failed(&web.app, e),
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

fn percent_decoded(v: &str) -> Option<String> {
    let mut out = String::new();
    let mut bytes = v.bytes();
    while let Some(b) = bytes.next() {
        let b = match b {
            b'%' => {
                let hex = [bytes.next()?, bytes.next()?];
                u8::from_str_radix(std::str::from_utf8(&hex).ok()?, 16).ok()?
            }
            b => b,
        };
        let c = char::from(b);
        if !c.is_ascii_alphanumeric() {
            return None;
        }
        out.push(c);
    }
    Some(out)
}

fn window_query(query: Option<&str>) -> Result<Option<String>, String> {
    let mut window = None;
    for pair in query.unwrap_or_default().split('&').filter(|p| !p.is_empty()) {
        match pair.split_once('=').map(|(k, v)| (k, percent_decoded(v))) {
            Some(("window", Some(v))) if window.is_none() => window = Some(v),
            _ => return Err("the only query parameter is window, once, such as ?window=24h".into()),
        }
    }
    Ok(window)
}

async fn get_usage_stats(State(web): State<Web>, uri: axum::http::Uri) -> Response {
    let window = match window_query(uri.query()) {
        Ok(w) => w,
        Err(e) => return error(StatusCode::BAD_REQUEST, e),
    };
    match web.app.wf.usage_stats(window.as_deref()).await {
        Ok(v) => json_ok(StatusCode::OK, web.app.redact_value(v)),
        Err(e) if e.to_string().starts_with("wf.usageStats:") => error(StatusCode::BAD_REQUEST, format!("{e:#}")),
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

fn settings_failed(app: &App, e: anyhow::Error) -> Response {
    if let Some(rejected) = e.downcast_ref::<Rejected>() {
        let body = json!({"error": rejected.to_string(), "errors": rejected.errors});
        return json_ok(StatusCode::UNPROCESSABLE_ENTITY, app.redact_value(body));
    }
    let status = if e.is::<Stale>() { StatusCode::CONFLICT } else { StatusCode::INTERNAL_SERVER_ERROR };
    error(status, app.redact(format!("{e:#}")))
}

async fn get_settings(State(web): State<Web>) -> Response {
    match web.app.wf.settings().await {
        Ok(v) => json_ok(StatusCode::OK, v),
        Err(e) => settings_failed(&web.app, e),
    }
}

async fn put_settings(State(web): State<Web>, remote: Option<Extension<SettingsLogin>>, raw: Bytes) -> Response {
    let b: SettingsBody = match body(&raw) {
        Ok(b) => b,
        Err(e) => return error(StatusCode::BAD_REQUEST, e),
    };
    match web.app.wf.save_settings(b).await {
        Ok(saved) => {
            if let Some(Extension(SettingsLogin(login))) = remote {
                tracing::info!(login = %login, keys = ?saved.changed, "remote board settings saved");
            }
            json_ok(StatusCode::OK, json!(saved))
        }
        Err(e) => settings_failed(&web.app, e),
    }
}

async fn get_settings_pools(State(web): State<Web>) -> Response {
    match web.app.wf.pools().await {
        Ok(v) => json_ok(StatusCode::OK, web.app.redact_value(json!({"pools": v}))),
        Err(e) => settings_failed(&web.app, e),
    }
}

pub fn routes(app: Arc<App>, port: u16) -> Router {
    let remote = app.config().board.active_remote_config().map(|(cfg, origin)| (cfg.clone(), origin));
    let web = Web { app, port, remote };
    let settings = Router::new()
        .route("/api/settings", get(get_settings).put(put_settings))
        .route("/api/settings/pools", get(get_settings_pools))
        .route_layer(axum::middleware::from_fn_with_state(web.clone(), settings_guard));
    let board = Router::new()
        .route("/", get(page))
        .route("/manifest.webmanifest", get(manifest))
        .route("/icon.svg", get(icon_svg))
        .route("/icon-192.png", get(icon_192))
        .route("/icon-512.png", get(icon_512))
        .route("/api/state", get(state))
        .route("/api/events", get(events))
        .route("/api/msg/{handle}", get(message))
        .route("/api/cards/{id}/answer", post(answer))
        .route("/api/cards/{id}/dismiss", post(dismiss))
        .route("/api/launch", post(launch))
        .route("/api/ask", post(ask))
        .route("/api/models", get(get_models))
        .route("/api/usage-stats", get(get_usage_stats))
        .route("/api/models/ticket", put(put_ticket_model))
        .route("/api/models/ask", put(put_ask_model))
        .route("/api/tickets/{thread}/close", post(close_ticket))
        .route("/api/tickets/{thread}/settle", post(settle_ticket))
        .route("/api/tickets/{thread}/restore", post(restore_ticket))
        .route("/api/links/{thread}/pause", post(pause_link))
        .route("/api/links/{thread}/resume", post(resume_link))
        .route("/api/asks/{id}/settle", post(settle_ask))
        .route("/api/asks/{id}/restore", post(restore_ask))
        .route("/api/settle-done", post(settle_done))
        .route("/api/new-issues", get(list_new_issues))
        .route("/api/new-issues/{id}/launch", post(launch_new_issue))
        .route("/api/new-issues/{id}/dismiss", post(dismiss_new_issue))
        .route("/api/new-issues/{id}/restore", post(restore_new_issue))
        .route("/api/flow", get(get_flow))
        .route("/api/flow/sync-links", post(post_flow_sync_links))
        .route("/api/flow/edge", post(post_flow_edge))
        .route("/api/flow/issue", post(post_flow_issue))
        .route("/api/flow/triage", post(post_flow_triage))
        .route("/api/flow/triage/{id}/apply", post(post_flow_triage_apply))
        .route("/api/flow/triage/{id}/dismiss", post(post_flow_triage_dismiss))
        .route("/api/open", post(open_link))
        .route("/api/contexts", get(list_contexts))
        .route("/api/contexts/{id}", put(put_context).delete(delete_context))
        .merge(settings)
        .route_layer(axum::middleware::from_fn_with_state(web.clone(), board_guard))
        .with_state(web.clone());
    let pair = Router::new()
        .route("/pair", get(pair_page))
        .route("/api/pair", post(post_pair))
        .route_layer(axum::middleware::from_fn_with_state(web.clone(), pair_guard))
        .with_state(web);
    board.merge(pair)
}
