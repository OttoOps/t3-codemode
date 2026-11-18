use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use futures_util::stream;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio::sync::broadcast::error::{RecvError, TryRecvError};

use crate::app::App;
use crate::board::LaunchArgs;

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

impl Web {
    fn own(&self, value: &str, scheme: &str) -> bool {
        [format!("{scheme}127.0.0.1:{}", self.port), format!("{scheme}localhost:{}", self.port)]
            .iter()
            .any(|o| o == value)
    }

    fn reject(&self, headers: &HeaderMap, post: bool) -> Option<Response> {
        let get = |name: &str| headers.get(name).and_then(|v: &HeaderValue| v.to_str().ok());
        if !get(header::HOST.as_str()).is_some_and(|h| self.own(h, "")) {
            return Some(error(StatusCode::FORBIDDEN, "the board answers only on its own 127.0.0.1 or localhost host"));
        }
        if let Some(origin) = get(header::ORIGIN.as_str())
            && !self.own(origin, "http://")
        {
            return Some(error(StatusCode::FORBIDDEN, "cross-origin requests are refused"));
        }
        if post && !get(CSRF_HEADER).is_some_and(|t| same_bytes(t.as_bytes(), self.app.csrf.as_bytes())) {
            return Some(error(StatusCode::FORBIDDEN, "missing or wrong X-T3CM-CSRF token; reload the board"));
        }
        None
    }
}

fn body<T: DeserializeOwned>(raw: &Bytes) -> Result<T, String> {
    serde_json::from_slice(raw).map_err(|e| format!("bad request body: {e}"))
}

fn failed(app: &App, e: anyhow::Error) -> Response {
    let msg = app.t3.redact(format!("{e:#}"));
    let missing = ["no open board card", "no board ticket", "no ask"].iter().any(|p| msg.starts_with(p));
    let status = if missing { StatusCode::NOT_FOUND } else { StatusCode::BAD_REQUEST };
    error(status, msg)
}

async fn page(State(web): State<Web>, headers: HeaderMap) -> Response {
    if let Some(r) = web.reject(&headers, false) {
        return r;
    }
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
            (header::CONTENT_SECURITY_POLICY, CSP),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            (header::REFERRER_POLICY, "no-referrer"),
        ],
        PAGE.replace("{{CSRF}}", &web.app.csrf),
    )
        .into_response()
}

async fn state(State(web): State<Web>, headers: HeaderMap) -> Response {
    if let Some(r) = web.reject(&headers, false) {
        return r;
    }
    match web.app.wf.snapshot() {
        Ok(v) => json_ok(StatusCode::OK, v),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")),
    }
}

async fn message(State(web): State<Web>, headers: HeaderMap, Path(handle): Path<String>) -> Response {
    if let Some(r) = web.reject(&headers, false) {
        return r;
    }
    match web.app.store.msg_get(&handle) {
        Ok(Some(m)) => json_ok(StatusCode::OK, json!(m)),
        Ok(None) => error(StatusCode::NOT_FOUND, format!("no message {handle:?}")),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")),
    }
}

async fn events(State(web): State<Web>, headers: HeaderMap) -> Response {
    if let Some(r) = web.reject(&headers, false) {
        return r;
    }
    let rx = web.app.wf.subscribe();
    let s = stream::unfold((web.app.clone(), rx, None::<String>), |(app, mut rx, last)| async move {
        let mut wait = last.is_some();
        loop {
            if wait {
                match rx.recv().await {
                    Ok(()) | Err(RecvError::Lagged(_)) => {}
                    Err(RecvError::Closed) => return None,
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
                return Some((Ok::<_, Infallible>(ev), (app, rx, Some(snap))));
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
}

#[derive(Deserialize, Default)]
struct AtBody {
    #[serde(default)]
    at: Option<String>,
}

async fn answer(State(web): State<Web>, headers: HeaderMap, Path(id): Path<String>, raw: Bytes) -> Response {
    if let Some(r) = web.reject(&headers, true) {
        return r;
    }
    let b: TextBody = match body(&raw) {
        Ok(b) => b,
        Err(e) => return error(StatusCode::BAD_REQUEST, e),
    };
    match web.app.wf.answer_at(&id, &b.text, b.at.as_deref()).await {
        Ok(v) => json_ok(StatusCode::OK, v),
        Err(e) => failed(&web.app, e),
    }
}

async fn dismiss(State(web): State<Web>, headers: HeaderMap, Path(id): Path<String>, raw: Bytes) -> Response {
    if let Some(r) = web.reject(&headers, true) {
        return r;
    }
    let b: AtBody = if raw.is_empty() {
        AtBody::default()
    } else {
        match body(&raw) {
            Ok(b) => b,
            Err(e) => return error(StatusCode::BAD_REQUEST, e),
        }
    };
    match web.app.wf.dismiss_at(&id, b.at.as_deref()).await {
        Ok(v) => json_ok(StatusCode::OK, v),
        Err(e) => failed(&web.app, e),
    }
}

#[derive(Deserialize)]
struct LaunchBody {
    issues: Vec<String>,
}

async fn launch(State(web): State<Web>, headers: HeaderMap, raw: Bytes) -> Response {
    if let Some(r) = web.reject(&headers, true) {
        return r;
    }
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

async fn ask(State(web): State<Web>, headers: HeaderMap, raw: Bytes) -> Response {
    if let Some(r) = web.reject(&headers, true) {
        return r;
    }
    let b: AskBody = match body(&raw) {
        Ok(b) => b,
        Err(e) => return error(StatusCode::BAD_REQUEST, e),
    };
    match web.app.ask(&b.question).await {
        Ok(v) => json_ok(StatusCode::OK, v),
        Err(e) => failed(&web.app, e),
    }
}

async fn settle_ticket(State(web): State<Web>, headers: HeaderMap, Path(thread): Path<String>) -> Response {
    if let Some(r) = web.reject(&headers, true) {
        return r;
    }
    match web.app.wf.settle_ticket(&thread).await {
        Ok(v) => json_ok(StatusCode::OK, v),
        Err(e) => failed(&web.app, e),
    }
}

async fn restore_ticket(State(web): State<Web>, headers: HeaderMap, Path(thread): Path<String>) -> Response {
    if let Some(r) = web.reject(&headers, true) {
        return r;
    }
    match web.app.wf.restore_ticket(&thread).await {
        Ok(v) => json_ok(StatusCode::OK, v),
        Err(e) => failed(&web.app, e),
    }
}

async fn settle_ask(State(web): State<Web>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    if let Some(r) = web.reject(&headers, true) {
        return r;
    }
    match web.app.wf.settle_ask(&id) {
        Ok(v) => json_ok(StatusCode::OK, v),
        Err(e) => failed(&web.app, e),
    }
}

async fn restore_ask(State(web): State<Web>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    if let Some(r) = web.reject(&headers, true) {
        return r;
    }
    match web.app.wf.restore_ask(&id) {
        Ok(v) => json_ok(StatusCode::OK, v),
        Err(e) => failed(&web.app, e),
    }
}

#[derive(Deserialize)]
struct OpenBody {
    link: String,
}

async fn open_link(State(web): State<Web>, headers: HeaderMap, raw: Bytes) -> Response {
    if let Some(r) = web.reject(&headers, true) {
        return r;
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

async fn settle_done(State(web): State<Web>, headers: HeaderMap) -> Response {
    if let Some(r) = web.reject(&headers, true) {
        return r;
    }
    match web.app.wf.settle_closed(None).await {
        Ok(v) => json_ok(StatusCode::OK, v),
        Err(e) => failed(&web.app, e),
    }
}

pub fn routes(app: Arc<App>, port: u16) -> Router {
    Router::new()
        .route("/", get(page))
        .route("/api/state", get(state))
        .route("/api/events", get(events))
        .route("/api/msg/{handle}", get(message))
        .route("/api/cards/{id}/answer", post(answer))
        .route("/api/cards/{id}/dismiss", post(dismiss))
        .route("/api/launch", post(launch))
        .route("/api/ask", post(ask))
        .route("/api/tickets/{thread}/settle", post(settle_ticket))
        .route("/api/tickets/{thread}/restore", post(restore_ticket))
        .route("/api/asks/{id}/settle", post(settle_ask))
        .route("/api/asks/{id}/restore", post(restore_ask))
        .route("/api/settle-done", post(settle_done))
        .route("/api/open", post(open_link))
        .with_state(Web { app, port })
}
