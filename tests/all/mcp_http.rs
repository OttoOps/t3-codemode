use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

use crate::common::*;
use serde_json::{Value, json};
use t3_codemode::server;

struct Srv {
    base: String,
    http: reqwest::Client,
    _h: H,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
}

impl Drop for Srv {
    fn drop(&mut self) {
        if let Some(s) = self.stop.take() {
            let _ = s.send(());
        }
    }
}

async fn start(h: H) -> Srv {
    let (listener, addr) = server::bind(0).await.unwrap();
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let app = h.app.clone();
    tokio::spawn(async move {
        server::serve(app, listener, async move {
            let _ = rx.await;
        })
        .await
        .unwrap();
    });
    Srv {
        base: format!("http://{addr}"),
        http: reqwest::Client::builder().no_proxy().build().unwrap(),
        _h: h,
        stop: Some(tx),
    }
}

impl Srv {
    async fn rpc(&self, body: Value, thread: Option<&str>) -> reqwest::Response {
        let mut req = self
            .http
            .post(format!("{}/mcp", self.base))
            .header("MCP-Protocol-Version", "2025-06-18")
            .header("Accept", "application/json, text/event-stream")
            .json(&body);
        if let Some(t) = thread {
            req = req.header("X-T3-Thread", t);
        }
        req.send().await.unwrap()
    }

    async fn call(&self, tool: &str, args: Value, thread: Option<&str>) -> Value {
        let r = self
            .rpc(
                json!({"jsonrpc": "2.0", "id": 7, "method": "tools/call", "params": {"name": tool, "arguments": args}}),
                thread,
            )
            .await;
        assert_eq!(r.status(), 200);
        r.json::<Value>().await.unwrap()["result"].clone()
    }
}

#[tokio::test]
async fn initialize_list_and_call_over_http() {
    let h = H::with_tools();
    h.load_catalog().await;
    let s = start(h).await;

    let r = s
        .rpc(json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
                    "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "test", "version": "1"}}}), None)
        .await;
    assert_eq!(r.status(), 200);
    assert!(r.headers().get("mcp-session-id").is_some());
    assert_eq!(r.headers().get("content-type").unwrap(), "application/json");
    let init: Value = r.json().await.unwrap();
    assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(init["result"]["serverInfo"]["name"], "t3-codemode");

    let r = s.rpc(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}), None).await;
    assert_eq!(r.status(), 202);

    let r = s.rpc(json!({"jsonrpc": "2.0", "id": 2, "method": "ping"}), None).await;
    assert_eq!(r.json::<Value>().await.unwrap()["result"], json!({}));

    let list = |thread: Option<&'static str>| {
        let s = &s;
        async move {
            s.rpc(json!({"jsonrpc": "2.0", "id": 3, "method": "tools/list"}), thread)
                .await
                .json::<Value>()
                .await
                .unwrap()
        }
    };
    let anon = list(None).await;
    let tools = anon["result"]["tools"].as_array().unwrap();
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["search", "execute"]);
    assert_eq!(anon, list(Some(WORKER)).await);
    assert_eq!(anon, list(Some(BOSS)).await);
    for t in tools {
        let d = t["description"].as_str().unwrap();
        println!("{} description: {} chars (~{} tokens)", t["name"], d.len(), d.len() / 4);
        assert!(d.len() / 4 < 400, "{} description too long", t["name"]);
    }

    let r = s.call("search", json!({"code": "async () => catalog.filter(t => t.kind === 't3').length"}), None).await;
    assert_eq!(r["content"][0]["text"], r#"{"result":74}"#);
    assert!(r.get("isError").is_none());

    let code = "async () => { const { threads } = await t3.t3_thread_list({ allProjects: true, limit: 20 }); \
                const by = {}; for (const t of threads) by[t.status] = (by[t.status] ?? 0) + 1; return by }";
    let r = s.call("execute", json!({"code": code}), None).await;
    assert_eq!(r["content"][0]["text"], r#"{"result":{"idle":3}}"#);

    let r = s
        .call("execute", json!({"code": "async () => (await wf.init({ role: 'ticket' })).thread"}), Some(TICKET))
        .await;
    assert_eq!(r["content"][0]["text"], r#"{"result":"mcp:ticket"}"#);
    let r = s
        .call(
            "execute",
            json!({"code": "async () => (await wf.init({ role: 'worker' })).thread", "thread": WORKER}),
            None,
        )
        .await;
    assert_eq!(r["content"][0]["text"], r#"{"result":"mcp:worker"}"#);

    let r = s.call("execute", json!({"code": "async () => nope"}), None).await;
    assert_eq!(r["isError"], true);
    assert!(r["content"][0]["text"].as_str().unwrap().contains("Available globals"));

    let r = s.call("execute", json!({}), None).await;
    assert_eq!(r["isError"], true);
}

#[tokio::test]
async fn http_edges() {
    let s = start(H::new()).await;
    let r = s.http.get(format!("{}/mcp", s.base)).send().await.unwrap();
    assert_eq!(r.status(), 405);
    let r = s.http.get(format!("{}/healthz", s.base)).send().await.unwrap();
    let health: Value = r.json().await.unwrap();
    assert_eq!(health["version"], env!("CARGO_PKG_VERSION"));
    assert!(health["store"].as_str().unwrap().ends_with("store.db"));
    assert_eq!(health["t3"]["connected"], true);
    let r = s
        .http
        .post(format!("{}/mcp", s.base))
        .header("Origin", "https://evil.example")
        .json(&json!({"jsonrpc": "2.0", "id": 1, "method": "ping"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 403);
    let r = s
        .http
        .post(format!("{}/mcp", s.base))
        .header("Origin", "http://localhost:3000")
        .json(&json!({"jsonrpc": "2.0", "id": 1, "method": "ping"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let r = s.http.post(format!("{}/mcp", s.base)).body("{not json").send().await.unwrap();
    assert_eq!(r.status(), 400);
    let r = s.rpc(json!({"jsonrpc": "2.0", "id": 9, "method": "resources/list"}), None).await;
    assert_eq!(r.json::<Value>().await.unwrap()["error"]["code"], -32601);
    let r = s.rpc(json!({"jsonrpc": "2.0", "id": 9, "method": "tools/call", "params": {"name": "other", "arguments": {"code": "1"}}}), None).await;
    assert_eq!(r.json::<Value>().await.unwrap()["error"]["code"], -32602);
    let r = s.rpc(json!({"jsonrpc": "2.0", "id": 9, "method": "tools/list"}), Some("${T3CODE_THREAD_ID}")).await;
    assert_eq!(r.status(), 200);
}

#[tokio::test]
async fn mcp_and_healthz_refuse_remote_hosts_origins_and_proxy_headers() {
    let cfg = t3_codemode::config::Config::from_value(json!({
        "board": {
            "remote": {
                "mode": "tailscale",
                "origin": "https://board.acme.example:47732"
            }
        }
    }))
    .unwrap();
    let s = start(H::build(Vec::new(), cfg)).await;
    let port = s.base.rsplit(':').next().unwrap();
    let ping = json!({"jsonrpc": "2.0", "id": 1, "method": "ping"});

    // Direct loopback (127.0.0.1, localhost, [::1]) behaves as before even with board.remote set.
    for host in [format!("127.0.0.1:{port}"), format!("localhost:{port}"), format!("[::1]:{port}")] {
        let r = s.http.post(format!("{}/mcp", s.base)).header("Host", &host).json(&ping).send().await.unwrap();
        assert_eq!(r.status(), 200, "POST /mcp Host={host}");
        let r = s.http.get(format!("{}/healthz", s.base)).header("Host", &host).send().await.unwrap();
        assert_eq!(r.status(), 200, "GET /healthz Host={host}");
    }
    let r =
        s.http.post(format!("{}/mcp", s.base)).header("Origin", "http://[::1]:3000").json(&ping).send().await.unwrap();
    assert_eq!(r.status(), 200);

    // Non-loopback hosts (including the configured remote board host) are refused on /mcp and /healthz.
    for host in ["evil.example:47731", "board.acme.example:47732", "127.0.0.1:0", "127.0.0.1@evil.example"] {
        let r = s.http.post(format!("{}/mcp", s.base)).header("Host", host).json(&ping).send().await.unwrap();
        assert_eq!(r.status(), 403, "POST /mcp Host={host}");
        let r = s.http.get(format!("{}/mcp", s.base)).header("Host", host).send().await.unwrap();
        assert_eq!(r.status(), 403, "GET /mcp Host={host}");
        let r = s.http.put(format!("{}/mcp", s.base)).header("Host", host).send().await.unwrap();
        assert_eq!(r.status(), 403, "PUT /mcp Host={host}");
        let r = s.http.get(format!("{}/healthz", s.base)).header("Host", host).send().await.unwrap();
        assert_eq!(r.status(), 403, "GET /healthz Host={host}");
        let r = s.http.post(format!("{}/healthz", s.base)).header("Host", host).send().await.unwrap();
        assert_eq!(r.status(), 403, "POST /healthz Host={host}");
    }

    // Non-loopback origins (including the configured remote board origin) are refused on /mcp and /healthz.
    for origin in [
        "https://evil.example",
        "https://board.acme.example:47732",
        "http://127.0.0.1@evil.example",
        "http://127.0.0.1:3000/path",
        "null",
    ] {
        let r = s.http.post(format!("{}/mcp", s.base)).header("Origin", origin).json(&ping).send().await.unwrap();
        assert_eq!(r.status(), 403, "POST /mcp Origin={origin}");
        let r = s.http.get(format!("{}/healthz", s.base)).header("Origin", origin).send().await.unwrap();
        assert_eq!(r.status(), 403, "GET /healthz Origin={origin}");
    }

    // Any proxy header is refused on /mcp and /healthz even when Host is loopback.
    for (name, value) in [
        ("Tailscale-User-Login", "alice@acme.example"),
        ("Tailscale-User-Login", ""),
        ("X-Forwarded-Host", "board.acme.example:47732"),
        ("X-Forwarded-For", "100.64.0.1"),
        ("X-Forwarded-Proto", "https"),
        ("X-Forwarded-Port", "47732"),
        ("Forwarded", "for=100.64.0.1;proto=https"),
    ] {
        let r = s.http.post(format!("{}/mcp", s.base)).header(name, value).json(&ping).send().await.unwrap();
        assert_eq!(r.status(), 403, "POST /mcp {name}={value:?}");
        let r = s.http.get(format!("{}/mcp", s.base)).header(name, value).send().await.unwrap();
        assert_eq!(r.status(), 403, "GET /mcp {name}={value:?}");
        let r = s.http.get(format!("{}/healthz", s.base)).header(name, value).send().await.unwrap();
        assert_eq!(r.status(), 403, "GET /healthz {name}={value:?}");
    }
}

#[tokio::test]
async fn the_deny_list_is_enforced_host_side() {
    let h = H::with_tools();
    h.load_catalog().await;
    for code in [
        "async () => t3.t3_project_delete({ projectId: 'p' })",
        "async () => t3.project_delete({ projectId: 'p' })",
        "async () => t3.call('t3_environment_unlink', {})",
        "async () => t3.call('environment_unlink', {})",
    ] {
        let out = h.exec(code).await;
        assert!(
            err_text(&out).contains("denied by the t3-codemode deny list; do not retry it"),
            "{code}: {}",
            out.text
        );
    }
    assert!(!h.fake.calls().iter().any(|c| c.contains("delete") || c.contains("unlink")));
    let out = h.exec("async () => t3.call('t3_no_such_tool', {})").await;
    assert!(err_text(&out).contains("unknown T3 tool"), "{}", out.text);
    let out = h.exec("async () => (await t3.thread_list({ limit: 1 })).threads.length").await;
    assert_eq!(result(&out), json!(1));
}

#[test]
fn stdio_mode_speaks_mcp() {
    let home = tempfile::tempdir().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_t3cm"))
        .args(["stdio", "--state-dir"])
        .arg(home.path().join("state"))
        .env("T3CODE_HOME", home.path())
        .env("T3CM_T3_BIN", "/bin/false")
        .env("T3CM_CONFIG", home.path().join("none.json"))
        .env("T3CODE_THREAD_ID", "mcp:stdio")
        .env("RUST_LOG", "off")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut out = BufReader::new(child.stdout.take().unwrap());
    let mut ask = |v: Value| -> Value {
        writeln!(stdin, "{v}").unwrap();
        let mut line = String::new();
        out.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    };
    let init =
        ask(json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-03-26"}}));
    assert_eq!(init["result"]["protocolVersion"], "2025-03-26");
    let list = ask(json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}));
    assert_eq!(list["result"]["tools"].as_array().unwrap().len(), 2);
    let r = ask(json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "execute",
        "arguments": {"code": "async () => (await wf.init({ role: 'overseer' })).thread"}}}));
    assert_eq!(r["result"]["content"][0]["text"], r#"{"result":"mcp:stdio"}"#);
    drop(stdin);
    assert!(child.wait().unwrap().success());
}
