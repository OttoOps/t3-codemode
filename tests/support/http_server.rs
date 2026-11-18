use std::collections::HashMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};

#[derive(Clone, Debug)]
pub struct Seen {
    pub rpc: Option<String>,
    pub headers: HeaderMap,
}

#[derive(Default)]
pub struct FakeHttp {
    pub token: Option<String>,
    pub sse: AtomicBool,
    pub seen: Mutex<Vec<Seen>>,
    pub deleted: Mutex<Vec<String>>,
    sessions: AtomicU64,
    current: Mutex<Option<String>>,
    waiting: Mutex<HashMap<String, oneshot::Sender<Value>>>,
}

impl FakeHttp {
    pub fn with_token(token: &str) -> FakeHttp {
        FakeHttp { token: Some(token.into()), ..Default::default() }
    }

    pub fn expire_session(&self) {
        *self.current.lock().unwrap() = None;
    }

    pub fn initializes(&self) -> usize {
        self.seen.lock().unwrap().iter().filter(|s| s.rpc.as_deref() == Some("initialize")).count()
    }

    pub fn last(&self, rpc: &str) -> Seen {
        self.seen.lock().unwrap().iter().rev().find(|s| s.rpc.as_deref() == Some(rpc)).cloned().unwrap()
    }
}

pub async fn start(fake: FakeHttp) -> (Arc<FakeHttp>, String) {
    let fake = Arc::new(fake);
    let app =
        Router::new().route("/mcp", post(handle).delete(delete)).route("/moved", post(moved)).with_state(fake.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (fake, format!("http://{addr}/mcp"))
}

fn header(headers: &HeaderMap, name: &str) -> Option<String> {
    headers.get(name).and_then(|v| v.to_str().ok()).map(String::from)
}

fn frame(msg: &Value) -> String {
    format!("event: message\ndata: {msg}\n\n")
}

fn reply(fake: &FakeHttp, msg: Value, session: Option<String>, extra: Vec<Value>) -> Response {
    let mut builder = Response::builder().status(StatusCode::OK);
    if let Some(s) = session {
        builder = builder.header("mcp-session-id", s);
    }
    if fake.sse.load(Ordering::SeqCst) {
        let mut body = String::from(": keepalive\n\n");
        for e in &extra {
            body.push_str(&frame(e));
        }
        body.push_str(&frame(&msg));
        builder.header("content-type", "text/event-stream").body(Body::from(body)).unwrap()
    } else {
        let mut all = extra;
        let body = if all.is_empty() {
            msg
        } else {
            all.push(msg);
            Value::Array(all)
        };
        builder.header("content-type", "application/json").body(Body::from(body.to_string())).unwrap()
    }
}

fn tool(name: &str) -> Value {
    json!({"name": name, "inputSchema": {"type": "object"}})
}

async fn handle(State(fake): State<Arc<FakeHttp>>, headers: HeaderMap, body: Bytes) -> Response {
    if let Some(token) = &fake.token
        && header(&headers, "authorization").as_deref() != Some(token.as_str())
    {
        return (StatusCode::UNAUTHORIZED, "no entry").into_response();
    }
    let Ok(msg) = serde_json::from_slice::<Value>(&body) else {
        return (StatusCode::BAD_REQUEST, "bad json").into_response();
    };
    let rpc = msg["method"].as_str().map(String::from);
    fake.seen.lock().unwrap().push(Seen { rpc: rpc.clone(), headers: headers.clone() });
    let id = msg.get("id").cloned().unwrap_or(Value::Null);
    if rpc.as_deref() == Some("initialize") {
        let sid = format!("sess-{}", fake.sessions.fetch_add(1, Ordering::SeqCst) + 1);
        *fake.current.lock().unwrap() = Some(sid.clone());
        let result = json!({"protocolVersion": "2025-06-18", "capabilities": {"tools": {}},
            "serverInfo": {"name": "fake-http", "version": "0.9"}});
        return reply(&fake, json!({"jsonrpc": "2.0", "id": id, "result": result}), Some(sid), vec![]);
    }
    let current = fake.current.lock().unwrap().clone();
    if current.is_none() || header(&headers, "mcp-session-id") != current {
        return (StatusCode::NOT_FOUND, "unknown session").into_response();
    }
    let Some(method) = rpc else {
        if let Some(key) = id.as_str() {
            let tx = fake.waiting.lock().unwrap().remove(key);
            if let Some(tx) = tx {
                let _ = tx.send(msg.clone());
            }
        }
        return StatusCode::ACCEPTED.into_response();
    };
    if id.is_null() {
        return StatusCode::ACCEPTED.into_response();
    }
    match method.as_str() {
        "tools/list" => {
            let result = match msg["params"]["cursor"].as_str() {
                None => json!({"tools": [tool("echo"), tool("fail")], "nextCursor": "two"}),
                Some(_) => json!({"tools": [tool("ping_first"), tool("notify")]}),
            };
            reply(&fake, json!({"jsonrpc": "2.0", "id": id, "result": result}), None, vec![])
        }
        "tools/call" => {
            let args = msg["params"]["arguments"].clone();
            match msg["params"]["name"].as_str().unwrap_or_default() {
                "echo" => {
                    let result = json!({"structuredContent": {"echo": args}, "content": []});
                    reply(&fake, json!({"jsonrpc": "2.0", "id": id, "result": result}), None, vec![])
                }
                "fail" => {
                    let result = json!({"isError": true, "content": [{"type": "text", "text": "http tool broke"}]});
                    reply(&fake, json!({"jsonrpc": "2.0", "id": id, "result": result}), None, vec![])
                }
                "notify" => {
                    let note = json!({"jsonrpc": "2.0", "method": "notifications/tools/list_changed"});
                    let result = json!({"content": [{"type": "text", "text": "\"ok\""}]});
                    reply(&fake, json!({"jsonrpc": "2.0", "id": id, "result": result}), None, vec![note])
                }
                "ping_first" => ping_first(fake, id).await,
                "slow" => {
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    reply(&fake, json!({"jsonrpc": "2.0", "id": id, "result": {"content": []}}), None, vec![])
                }
                "huge" => {
                    let result = json!({"content": [{"type": "text", "text": "x".repeat(17 * 1024 * 1024)}]});
                    reply(&fake, json!({"jsonrpc": "2.0", "id": id, "result": result}), None, vec![])
                }
                "boom" => {
                    let auth = header(&headers, "authorization").unwrap_or_default();
                    (StatusCode::INTERNAL_SERVER_ERROR, format!("server fell over holding {auth}")).into_response()
                }
                other => {
                    let err = json!({"code": -32602, "message": format!("unknown tool {other}")});
                    reply(&fake, json!({"jsonrpc": "2.0", "id": id, "error": err}), None, vec![])
                }
            }
        }
        _ => {
            let err = json!({"code": -32601, "message": "method not found"});
            reply(&fake, json!({"jsonrpc": "2.0", "id": id, "error": err}), None, vec![])
        }
    }
}

async fn ping_first(fake: Arc<FakeHttp>, id: Value) -> Response {
    let (tx, rx) = oneshot::channel();
    fake.waiting.lock().unwrap().insert("srv-ping".into(), tx);
    let (out, events) = mpsc::unbounded_channel::<String>();
    let _ = out.send(frame(&json!({"jsonrpc": "2.0", "id": "srv-ping", "method": "ping"})));
    tokio::spawn(async move {
        let answer = tokio::time::timeout(Duration::from_secs(5), rx).await.ok().and_then(|r| r.ok());
        let result = json!({"structuredContent": {"ping": answer}, "content": []});
        let msg = frame(&json!({"jsonrpc": "2.0", "id": id, "result": result}));
        let (a, b) = msg.split_at(msg.len() / 2);
        let _ = out.send(a.to_string());
        let _ = out.send(b.to_string());
    });
    let stream = futures_util::stream::unfold(events, |mut rx| async move {
        rx.recv().await.map(|chunk| (Ok::<_, Infallible>(Bytes::from(chunk)), rx))
    });
    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "text/event-stream")
        .body(Body::from_stream(stream))
        .unwrap()
}

async fn delete(State(fake): State<Arc<FakeHttp>>, headers: HeaderMap) -> StatusCode {
    if let Some(s) = header(&headers, "mcp-session-id") {
        fake.deleted.lock().unwrap().push(s);
    }
    StatusCode::OK
}

async fn moved(State(fake): State<Arc<FakeHttp>>, headers: HeaderMap) -> Response {
    fake.seen.lock().unwrap().push(Seen { rpc: Some("moved".into()), headers });
    Response::builder().status(StatusCode::TEMPORARY_REDIRECT).header("location", "/mcp").body(Body::empty()).unwrap()
}
