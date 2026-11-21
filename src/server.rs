use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;

use crate::app::App;
use crate::config::parse_host_header;
use crate::wf_slots::Reported;
use crate::{mcp, web};

pub const THREAD_HEADER: &str = "x-t3-thread";

pub const PROXY_HEADERS: &[&str] = &[
    "tailscale-user-login",
    "x-forwarded-host",
    "x-forwarded-for",
    "x-forwarded-proto",
    "x-forwarded-port",
    "forwarded",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestKind {
    Local,
    Remote,
}

fn session_id() -> String {
    let mut b = [0u8; 16];
    let _ = getrandom::fill(&mut b);
    b.iter().map(|x| format!("{x:02x}")).collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SingleHeader<'a> {
    Missing,
    Present(&'a str),
    Invalid,
}

pub fn single_header<'a>(headers: &'a HeaderMap, name: &str) -> SingleHeader<'a> {
    let mut all = headers.get_all(name).iter();
    let Some(first) = all.next() else {
        return SingleHeader::Missing;
    };
    if all.next().is_some() {
        return SingleHeader::Invalid;
    }
    match first.to_str() {
        Ok(s) => SingleHeader::Present(s),
        Err(_) => SingleHeader::Invalid,
    }
}

pub fn has_proxy_headers(headers: &HeaderMap) -> bool {
    PROXY_HEADERS.iter().any(|name| headers.contains_key(*name))
}

fn is_loopback_name(host: &str) -> bool {
    matches!(host, "127.0.0.1" | "localhost" | "[::1]")
}

pub fn is_own_loopback_host_str(raw: &str, port: u16) -> bool {
    let Some((host, parsed_port)) = parse_host_header(raw) else {
        return false;
    };
    is_loopback_name(&host) && parsed_port == Some(port)
}

pub fn is_own_loopback_host(headers: &HeaderMap, port: u16) -> bool {
    matches!(single_header(headers, header::HOST.as_str()), SingleHeader::Present(host) if is_own_loopback_host_str(host, port))
}

pub fn classify_request(headers: &HeaderMap, port: u16) -> RequestKind {
    if !has_proxy_headers(headers) && is_own_loopback_host(headers, port) {
        RequestKind::Local
    } else {
        RequestKind::Remote
    }
}

fn local_origin(headers: &HeaderMap) -> bool {
    let origin = match single_header(headers, header::ORIGIN.as_str()) {
        SingleHeader::Present(o) => o,
        SingleHeader::Missing => return true,
        SingleHeader::Invalid => return false,
    };
    let Some(rest) = origin.strip_prefix("http://").or_else(|| origin.strip_prefix("https://")) else {
        return false;
    };
    parse_host_header(rest).is_some_and(|(host, _)| is_loopback_name(&host))
}

pub fn local_host(headers: &HeaderMap) -> bool {
    let SingleHeader::Present(host) = single_header(headers, header::HOST.as_str()) else {
        return false;
    };
    parse_host_header(host).is_some_and(|(name, _)| is_loopback_name(&name))
}

async fn check_loopback(req: axum::extract::Request, next: axum::middleware::Next) -> Response {
    let headers = req.headers();
    if has_proxy_headers(headers) {
        return json_response(StatusCode::FORBIDDEN, json!({"error": "proxied requests are refused on this endpoint"}));
    }
    if !local_host(headers) {
        return json_response(StatusCode::FORBIDDEN, json!({"error": "requests must address a loopback host"}));
    }
    if !local_origin(headers) {
        return json_response(StatusCode::FORBIDDEN, json!({"error": "cross-origin requests are refused"}));
    }
    next.run(req).await
}

fn json_response(status: StatusCode, body: Value) -> Response {
    (status, [(header::CONTENT_TYPE, "application/json")], body.to_string()).into_response()
}

async fn post_mcp(State(app): State<Arc<App>>, headers: HeaderMap, body: Bytes) -> Response {
    let caller = mcp::caller_from_header(headers.get(THREAD_HEADER).and_then(|v| v.to_str().ok()));
    let reported = Reported::from_header_map(&headers);
    let msg: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            return json_response(
                StatusCode::BAD_REQUEST,
                json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": format!("parse error: {e}")}}),
            );
        }
    };
    let is_init = msg.get("method").and_then(Value::as_str) == Some("initialize");
    let reply = match msg {
        Value::Array(batch) => {
            let mut out = Vec::new();
            for m in batch {
                if let Some(r) = mcp::handle_with(&app, m, caller.as_deref(), &reported).await {
                    out.push(r);
                }
            }
            if out.is_empty() { None } else { Some(Value::Array(out)) }
        }
        m => mcp::handle_with(&app, m, caller.as_deref(), &reported).await,
    };
    let Some(reply) = reply else {
        return StatusCode::ACCEPTED.into_response();
    };
    let mut resp = json_response(StatusCode::OK, reply);
    if is_init && let Ok(v) = HeaderValue::from_str(&session_id()) {
        resp.headers_mut().insert("mcp-session-id", v);
    }
    resp
}

async fn not_allowed() -> Response {
    (StatusCode::METHOD_NOT_ALLOWED, [(header::ALLOW, "POST")], "").into_response()
}

async fn healthz(State(app): State<Arc<App>>) -> Response {
    json_response(StatusCode::OK, app.health_async().await)
}

pub fn router(app: Arc<App>, port: u16) -> Router {
    let loopback = Router::new()
        .route("/mcp", post(post_mcp).get(not_allowed).delete(not_allowed))
        .route("/healthz", get(healthz))
        .route_layer(axum::middleware::from_fn(check_loopback))
        .with_state(app.clone());
    loopback.merge(web::routes(app, port))
}

pub async fn serve(
    app: Arc<App>,
    listener: TcpListener,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    let port = listener.local_addr()?.port();
    let stopping = app.stopping.clone();
    let shutdown = async move {
        shutdown.await;
        stopping.send_replace(true);
    };
    axum::serve(listener, router(app, port)).with_graceful_shutdown(shutdown).await
}

pub async fn bind(port: u16) -> std::io::Result<(TcpListener, SocketAddr)> {
    let listener = TcpListener::bind(("127.0.0.1", port)).await?;
    let addr = listener.local_addr()?;
    Ok((listener, addr))
}

pub async fn stdio(app: Arc<App>, caller: Option<String>) -> std::io::Result<()> {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Value>();
    let writer = tokio::spawn(async move {
        let mut out = tokio::io::stdout();
        while let Some(v) = rx.recv().await {
            let mut line = v.to_string();
            line.push('\n');
            if out.write_all(line.as_bytes()).await.is_err() || out.flush().await.is_err() {
                break;
            }
        }
    });
    let reported = Reported::from_env();
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let msg: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                let _ = tx.send(json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": format!("parse error: {e}")}}));
                continue;
            }
        };
        let (app, tx, caller, reported) = (app.clone(), tx.clone(), caller.clone(), reported.clone());
        tokio::spawn(async move {
            if let Some(r) = mcp::handle_with(&app, msg, caller.as_deref(), &reported).await {
                let _ = tx.send(r);
            }
        });
    }
    drop(tx);
    let _ = writer.await;
    Ok(())
}
