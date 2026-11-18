use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::extract::{Form, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use t3_codemode::clock::SystemClock;
use t3_codemode::store::Store;
use t3_codemode::t3::{T3Client, T3Error};
use t3_codemode::t3_http::HttpT3;

#[derive(Default)]
struct Fake {
    challenge: String,
    issued: Vec<String>,
    rejected: Vec<String>,
    mcp_headers: Vec<(String, Option<String>, Option<String>)>,
}

type S = Arc<Mutex<Fake>>;

async fn register(Json(body): Json<Value>) -> Json<Value> {
    assert!(body["client_name"].as_str().unwrap().starts_with("t3-codemode@"));
    Json(json!({"client_id": "client-1"}))
}

async fn decision(State(s): State<S>, Json(body): Json<Value>) -> Json<Value> {
    assert_eq!(body["decision"]["code"], "PAIR-CODE");
    assert_eq!(body["decision"]["access"], "full-access");
    let a = &body["authorization"];
    assert_eq!(a["code_challenge_method"], "S256");
    s.lock().unwrap().challenge = a["code_challenge"].as_str().unwrap().into();
    Json(json!({"redirectTo": format!("http://127.0.0.1/callback?code=AUTH&state={}", a["state"].as_str().unwrap())}))
}

async fn token(State(s): State<S>, Form(f): Form<HashMap<String, String>>) -> Response {
    let mut st = s.lock().unwrap();
    let want = URL_SAFE_NO_PAD.encode(Sha256::digest(f["code_verifier"].as_bytes()));
    if want != st.challenge || f["code"] != "AUTH" || f["grant_type"] != "authorization_code" {
        return (StatusCode::BAD_REQUEST, Json(json!({"error": "invalid_grant"}))).into_response();
    }
    let tok = format!("tok-{}", st.issued.len() + 1);
    st.issued.push(tok.clone());
    Json(json!({"access_token": tok, "token_type": "Bearer", "expires_in": 2592000})).into_response()
}

async fn mcp(State(s): State<S>, headers: HeaderMap, Json(body): Json<Value>) -> Response {
    let auth = headers.get("authorization").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let tok = auth.strip_prefix("Bearer ").unwrap_or("").to_string();
    {
        let mut st = s.lock().unwrap();
        if !st.issued.contains(&tok) || st.rejected.contains(&tok) {
            return StatusCode::UNAUTHORIZED.into_response();
        }
        let sid = headers.get("mcp-session-id").and_then(|v| v.to_str().ok()).map(String::from);
        let ver = headers.get("mcp-protocol-version").and_then(|v| v.to_str().ok()).map(String::from);
        st.mcp_headers.push((body["method"].as_str().unwrap_or("").into(), sid, ver));
    }
    let Some(id) = body.get("id").cloned() else { return StatusCode::ACCEPTED.into_response() };
    match body["method"].as_str().unwrap() {
        "initialize" => (
            [("mcp-session-id", "sess-1"), ("content-type", "application/json")],
            json!({"jsonrpc": "2.0", "id": id, "result": {"protocolVersion": "2025-06-18"}}).to_string(),
        )
            .into_response(),
        "tools/list" => {
            Json(json!({"jsonrpc": "2.0", "id": id, "result": {"tools": [{"name": "t3_thread_list"}]}})).into_response()
        }
        "tools/call" => {
            let name = body["params"]["name"].as_str().unwrap();
            let result = match name {
                "boom" => json!({"isError": true, "content": [{"type": "text", "text": format!("bad header {auth}")}]}),
                "text" => json!({"content": [{"type": "text", "text": "{\"n\": 2}"}]}),
                _ => json!({"structuredContent": {"echo": body["params"]}, "content": []}),
            };
            let payload = json!({"jsonrpc": "2.0", "id": id, "result": result});
            ([("content-type", "text/event-stream")], format!("event: message\ndata: {payload}\n\n")).into_response()
        }
        _ => Json(json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "nope"}})).into_response(),
    }
}

#[tokio::test]
async fn oauth_session_calls_reauth_and_revoke() {
    let state: S = Arc::new(Mutex::new(Fake::default()));
    let app = Router::new()
        .route("/oauth/mcp/register", post(register))
        .route("/oauth/mcp/decision", post(decision))
        .route("/oauth/mcp/token", post(token))
        .route("/mcp", post(mcp))
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(home.path().join("userdata")).unwrap();
    std::fs::write(
        home.path().join("userdata/server-runtime.json"),
        json!({"origin": format!("http://127.0.0.1:{port}"), "pid": 999999999u64}).to_string(),
    )
    .unwrap();
    let log = home.path().join("t3.log");
    let bin = home.path().join("t3");
    let store = Arc::new(Store::open(&home.path().join("state")).unwrap());
    let label = t3_codemode::t3_http::instance_label(store.path());
    let list = json!([
        {"sessionId": "old", "client": {"label": "t3-codemode"}, "issuedAt": "2026-11-01T00:00:00.000Z"},
        {"sessionId": "mine-old", "client": {"label": label}, "issuedAt": "2026-11-10T00:00:00.000Z"},
        {"sessionId": "sibling", "client": {"label": "t3-codemode@deadbeef"}, "issuedAt": "2026-11-17T00:00:00.000Z"},
        {"sessionId": "new", "client": {"label": label}, "issuedAt": "2026-11-17T00:00:00.000Z"},
        {"sessionId": "other", "client": {"label": "T3 Code Desktop"}, "issuedAt": "2026-11-16T00:00:00.000Z"}
    ]);
    std::fs::write(
        &bin,
        format!(
            "#!/bin/sh\necho \"$@\" >> '{}'\ncase \"$1 $2 $3\" in\n  'auth pairing create') echo 'Pairing created'; echo '{{\"credential\":\"PAIR-CODE\"}}';;\n  'auth session list') echo '{}';;\nesac\n",
            log.display(),
            list
        ),
    )
    .unwrap();
    std::fs::set_permissions(&bin, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    unsafe {
        std::env::set_var("T3CODE_HOME", home.path());
        std::env::set_var("T3CM_T3_BIN", &bin);
        std::env::remove_var("T3CM_T3_ORIGIN");
    }

    let t3 = HttpT3::new(store.clone(), Arc::new(SystemClock));
    let r = t3.call("t3_thread_read", json!({"threadId": "x"})).await.unwrap();
    assert_eq!(r["echo"]["name"], "t3_thread_read");
    assert_eq!(r["echo"]["arguments"], json!({"threadId": "x"}));
    assert_eq!(t3.call("text", json!({})).await.unwrap(), json!({"n": 2}));
    assert_eq!(t3.list_tools().await.unwrap().len(), 1);

    let calls = std::fs::read_to_string(&log).unwrap();
    assert!(
        calls.contains(&format!("auth pairing create --ttl 5m --label {label} --json --scope orchestration:read")),
        "{calls}"
    );
    assert!(calls.contains("auth session revoke mine-old"), "{calls}");
    assert!(
        !calls.contains("revoke new")
            && !calls.contains("revoke other")
            && !calls.contains("revoke sibling")
            && !calls.lines().any(|l| l.contains("revoke old") && !l.contains("mine-old")),
        "{calls}"
    );

    {
        let st = state.lock().unwrap();
        assert_eq!(st.mcp_headers[0], ("initialize".into(), None, Some("2025-06-18".into())));
        assert!(st.mcp_headers[1..].iter().all(|(_, sid, _)| sid.as_deref() == Some("sess-1")), "{:?}", st.mcp_headers);
    }
    assert_eq!(store.token_get().unwrap().unwrap().access_token, "tok-1");
    assert!(t3.status().connected);
    assert!(t3.status().token_expires_at.is_some());

    let again = HttpT3::new(store.clone(), Arc::new(SystemClock));
    again.call("echo", json!({})).await.unwrap();
    assert_eq!(state.lock().unwrap().issued.len(), 1, "a fresh stored token is reused");

    state.lock().unwrap().rejected.push("tok-1".into());
    again.call("echo", json!({})).await.unwrap();
    assert_eq!(state.lock().unwrap().issued, ["tok-1", "tok-2"]);
    assert_eq!(store.token_get().unwrap().unwrap().access_token, "tok-2");

    match again.call("boom", json!({})).await {
        Err(T3Error::Tool(m)) => {
            assert!(m.starts_with("boom: bad header Bearer [redacted]"), "{m}");
            assert!(!m.contains("tok-2"));
        }
        other => panic!("{other:?}"),
    }
}
