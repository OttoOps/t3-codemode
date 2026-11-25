use std::io::Write;
use std::sync::{Arc, Mutex, OnceLock};

use crate::common::*;
use crate::http_server::{self, FakeHttp};
use serde_json::{Value, json};
use t3_codemode::config::Config;
use t3_codemode::server;

const STDIO_SECRET: &str = "stdio-secret-9c41e2b7";
const HTTP_SECRET: &str = "http-secret-5d8a3f10";
const STDIO_VAR: &str = "UPE2E_STDIO_TOKEN";
const HTTP_VAR: &str = "UPE2E_HTTP_TOKEN";

#[derive(Clone)]
struct Sink(Arc<Mutex<Vec<u8>>>);

impl Write for Sink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn log_sink() -> Sink {
    static SINK: OnceLock<Sink> = OnceLock::new();
    SINK.get_or_init(|| {
        let sink = Sink(Arc::new(Mutex::new(Vec::new())));
        let writer = sink.clone();
        let _ = tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::new("t3_codemode=debug"))
            .with_writer(move || writer.clone())
            .with_ansi(false)
            .try_init();
        sink
    })
    .clone()
}

fn logged() -> String {
    String::from_utf8_lossy(&log_sink().0.lock().unwrap()).into_owned()
}

struct Srv {
    base: String,
    http: reqwest::Client,
    h: H,
    web: Arc<FakeHttp>,
    seen: Vec<String>,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
}

impl Drop for Srv {
    fn drop(&mut self) {
        if let Some(s) = self.stop.take() {
            let _ = s.send(());
        }
    }
}

async fn start() -> Srv {
    log_sink();
    unsafe {
        std::env::set_var(STDIO_VAR, STDIO_SECRET);
        std::env::set_var(HTTP_VAR, HTTP_SECRET);
    }
    let (web, url) = http_server::start(FakeHttp::with_token(&format!("Bearer {HTTP_SECRET}"))).await;
    let config = Config::from_value(json!({
        "upstreams": [
            {
                "name": "fake",
                "transport": "stdio",
                "stdio": {
                    "command": env!("CARGO_BIN_EXE_t3cm-fake-mcp"),
                    "env": {"FAKE_API_TOKEN": format!("${{env:{STDIO_VAR}}}")}
                },
                "timeout_ms": 5000,
                "deny": ["image", "rpc_*"]
            },
            {
                "name": "web",
                "transport": "http",
                "http": {"url": url, "headers": {"Authorization": format!("Bearer ${{env:{HTTP_VAR}}}")}},
                "about": "Example web API"
            }
        ]
    }))
    .unwrap();
    let h = H::build(fixture_tools(), config);
    h.load_catalog().await;
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
        h,
        web,
        seen: Vec::new(),
        stop: Some(tx),
    }
}

impl Srv {
    async fn rpc(&mut self, body: Value, thread: Option<&str>) -> Value {
        let mut req = self
            .http
            .post(format!("{}/mcp", self.base))
            .header("MCP-Protocol-Version", "2025-06-18")
            .header("Accept", "application/json, text/event-stream")
            .json(&body);
        if let Some(t) = thread {
            req = req.header("X-T3-Thread", t);
        }
        let r = req.send().await.unwrap();
        assert_eq!(r.status(), 200);
        let text = r.text().await.unwrap();
        self.seen.push(text.clone());
        serde_json::from_str(&text).unwrap()
    }

    async fn tool(&mut self, tool: &str, code: &str) -> (bool, String) {
        let body = json!({"jsonrpc": "2.0", "id": 9, "method": "tools/call",
                          "params": {"name": tool, "arguments": {"code": code}}});
        let r = self.rpc(body, None).await;
        let result = &r["result"];
        (result["isError"] == true, result["content"][0]["text"].as_str().unwrap().to_string())
    }

    async fn ok(&mut self, tool: &str, code: &str) -> Value {
        let (is_error, text) = self.tool(tool, code).await;
        assert!(!is_error, "{code}: {text}");
        serde_json::from_str::<Value>(&text).unwrap_or_else(|e| panic!("{e}: {text}"))["result"].clone()
    }

    async fn err(&mut self, code: &str) -> String {
        let (is_error, text) = self.tool("execute", code).await;
        assert!(is_error, "{code} should fail: {text}");
        text
    }

    async fn healthz(&mut self) -> Value {
        let text = self.http.get(format!("{}/healthz", self.base)).send().await.unwrap().text().await.unwrap();
        self.seen.push(text.clone());
        serde_json::from_str(&text).unwrap()
    }

    fn assert_no_secret(&self) {
        let logs = logged();
        for secret in [STDIO_SECRET, HTTP_SECRET] {
            for text in &self.seen {
                assert!(!text.contains(secret), "secret in output: {text}");
            }
            assert!(!logs.contains(secret), "secret in logs: {logs}");
        }
    }
}

fn upstream<'a>(health: &'a Value, name: &str) -> &'a Value {
    health["upstreams"].as_array().unwrap().iter().find(|u| u["name"] == name).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn upstreams_end_to_end_over_http() {
    let mut s = start().await;

    let list = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"});
    let anon = s.rpc(list.clone(), None).await;
    let names: Vec<&str> =
        anon["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["search", "execute"]);
    let boss = s.rpc(list.clone(), Some(BOSS)).await;
    let worker = s.rpc(list.clone(), Some(WORKER)).await;
    assert_eq!(boss.to_string(), worker.to_string());
    assert_eq!(anon.to_string(), worker.to_string());
    assert!(anon.to_string().contains("fake (Fake MCP server for t3cm tests.), web (Example web API)."), "{anon}");
    let init = json!({"jsonrpc": "2.0", "id": 2, "method": "initialize", "params": {"protocolVersion": "2025-06-18"}});
    let init = s.rpc(init, None).await;
    assert_eq!(
        init["result"]["instructions"],
        "T3 Code and these MCP servers, reached through `search` then `execute`: \
         `fake` (Fake MCP server for t3cm tests.), `web` (Example web API). \
         Prefer these over the web or a browser for the same data."
    );

    let found = s
        .ok(
            "search",
            "async () => catalog.filter(t => t.kind === 'mcp')\
             .map(t => ({ name: t.name, server: t.server, signature: t.signature }))",
        )
        .await;
    let found = found.as_array().unwrap();
    let echo = found.iter().find(|t| t["name"] == "fake.echo").expect("fake.echo in catalog");
    assert_eq!(echo["server"], "fake");
    assert!(echo["signature"].as_str().unwrap().contains("fake.echo(input"), "{echo}");
    assert!(echo["signature"].as_str().unwrap().contains("Promise<"), "{echo}");
    assert!(found.iter().any(|t| t["name"] == "fake.ping_first"), "second page listed");
    assert!(found.iter().any(|t| t["name"] == "web.echo" && t["server"] == "web"));
    for denied in ["fake.image", "fake.rpc_error"] {
        assert!(!found.iter().any(|t| t["name"] == denied), "{denied} should be hidden");
    }
    let total = s.ok("search", "async () => catalog.length").await;
    assert!(total.as_u64().unwrap() > 74);

    assert_eq!(s.ok("execute", "async () => fake.echo({ a: 1 })").await, json!({"echo": {"a": 1}}));
    assert_eq!(s.ok("execute", "async () => tools.call('fake.echo', { b: 2 })").await, json!({"echo": {"b": 2}}));
    assert_eq!(s.ok("execute", "async () => fake.call('plain')").await, json!("hello world"));
    assert_eq!(s.ok("execute", "async () => web.echo({ x: 1 })").await, json!({"echo": {"x": 1}}));
    assert_eq!(s.ok("execute", "async () => tools.call('web.echo', {})").await, json!({"echo": {}}));
    assert_eq!(
        s.ok("execute", "async () => (await t3.t3_thread_list({ allProjects: true })).threads.length").await,
        json!(3)
    );
    assert_eq!(s.web.last("tools/call").headers["authorization"], format!("Bearer {HTTP_SECRET}").as_str());

    assert_eq!(s.ok("execute", "async () => typeof fake.image").await, json!("undefined"));
    for code in ["async () => tools.call('fake.image', {})", "async () => fake.call('rpc_error', {})"] {
        let e = s.err(code).await;
        assert!(e.contains("not allowed by the t3-codemode config") && e.contains("do not retry"), "{e}");
    }

    let leaked = s.ok("execute", "async () => fake.env({ name: 'FAKE_API_TOKEN' })").await;
    assert_eq!(leaked, json!({"value": "[redacted]"}));
    let rebuilt = s
        .ok(
            "execute",
            "async () => { const { value } = await fake.env({ name: 'FAKE_API_TOKEN' }); \
             console.log(value); return value.split('').reverse().join('') }",
        )
        .await;
    assert_eq!(rebuilt, json!("]detcader["));
    let e = s.err("async () => fake.fail_with_env({ name: 'FAKE_API_TOKEN' })").await;
    assert!(e.contains("fake.fail_with_env: token rejected: [redacted]"), "{e}");

    let before = s.healthz().await;
    assert_eq!(upstream(&before, "fake")["state"], "ready");
    assert_eq!(upstream(&before, "fake")["restarts"], 0);
    assert_eq!(upstream(&before, "web")["state"], "ready");
    for key in ["version", "uptimeSecs", "t3", "tools", "watcher", "store"] {
        assert!(before.get(key).is_some(), "missing {key}");
    }

    let e = s.err("async () => fake.crash({ stderr_env: 'FAKE_API_TOKEN' })").await;
    assert!(e.contains("fake: process exited"), "{e}");
    let down = s.healthz().await;
    let fake = upstream(&down, "fake");
    assert_eq!(fake["state"], "backoff");
    assert_eq!(fake["restarts"], 1);
    assert!(fake["lastError"].as_str().unwrap().contains("process exited"), "{fake}");
    assert!(fake["lastError"].as_str().unwrap().contains("[redacted]"), "{fake}");
    assert_eq!(upstream(&down, "web")["state"], "ready");

    assert_eq!(s.ok("execute", "async () => fake.echo({ back: true })").await, json!({"echo": {"back": true}}));
    let up = s.healthz().await;
    let fake = upstream(&up, "fake");
    assert_eq!(fake["state"], "ready");
    assert_eq!(fake["restarts"], 1);
    assert!(fake["lastError"].is_null(), "{fake}");
    assert_eq!(fake["tools"], 11);

    s.h.app.upstreams.shutdown().await;
    let logs = logged();
    assert!(logs.contains("upstream connected"), "{logs}");
    assert!(logs.contains("upstream unavailable"), "{logs}");
    assert!(logs.contains("[redacted]"), "{logs}");
    s.assert_no_secret();
}
