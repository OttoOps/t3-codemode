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
use crate::{mcp, web};

pub const THREAD_HEADER: &str = "x-t3-thread";

fn session_id() -> String {
    let mut b = [0u8; 16];
    let _ = getrandom::fill(&mut b);
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn local_origin(headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) else { return true };
    let Some(rest) = origin.strip_prefix("http://").or_else(|| origin.strip_prefix("https://")) else {
        return false;
    };
    let host = match rest.strip_prefix('[') {
        Some(v6) => v6.split(']').next().unwrap_or(""),
        None => rest.split([':', '/']).next().unwrap_or(""),
    };
    matches!(host, "127.0.0.1" | "localhost" | "::1")
}

pub fn local_host(headers: &HeaderMap) -> bool {
    let Some(host) = headers.get(header::HOST).and_then(|v| v.to_str().ok()) else { return false };
    let name = match host.strip_prefix('[') {
        Some(v6) => v6.split(']').next().unwrap_or(""),
        None => host.split(':').next().unwrap_or(""),
    };
    matches!(name.to_ascii_lowercase().as_str(), "127.0.0.1" | "localhost" | "::1")
}

async fn check_host(req: axum::extract::Request, next: axum::middleware::Next) -> Response {
    if !local_host(req.headers()) {
        return json_response(StatusCode::FORBIDDEN, json!({"error": "requests must address a loopback host"}));
    }
    next.run(req).await
}

fn json_response(status: StatusCode, body: Value) -> Response {
    (status, [(header::CONTENT_TYPE, "application/json")], body.to_string()).into_response()
}

async fn post_mcp(State(app): State<Arc<App>>, headers: HeaderMap, body: Bytes) -> Response {
    if !local_origin(&headers) {
        return json_response(StatusCode::FORBIDDEN, json!({"error": "cross-origin requests are refused"}));
    }
    let caller = mcp::caller_from_header(headers.get(THREAD_HEADER).and_then(|v| v.to_str().ok()));
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
                if let Some(r) = mcp::handle(&app, m, caller.as_deref()).await {
                    out.push(r);
                }
            }
            if out.is_empty() { None } else { Some(Value::Array(out)) }
        }
        m => mcp::handle(&app, m, caller.as_deref()).await,
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
    json_response(StatusCode::OK, app.health())
}

pub fn router(app: Arc<App>, port: u16) -> Router {
    Router::new()
        .route("/mcp", post(post_mcp).get(not_allowed).delete(not_allowed))
        .route("/healthz", get(healthz))
        .with_state(app.clone())
        .merge(web::routes(app, port))
        .layer(axum::middleware::from_fn(check_host))
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
        let (app, tx, caller) = (app.clone(), tx.clone(), caller.clone());
        tokio::spawn(async move {
            if let Some(r) = mcp::handle(&app, msg, caller.as_deref()).await {
                let _ = tx.send(r);
            }
        });
    }
    drop(tx);
    let _ = writer.await;
    Ok(())
}
