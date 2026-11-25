use std::collections::HashMap;
use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot};

const PAGE_ONE: &[&str] = &["client_info", "echo", "text_json", "plain", "image", "fail", "rpc_error", "sleep"];
const PAGE_TWO: &[&str] = &["crash", "env", "fail_with_env", "notify_changed", "ping_first"];
const SECOND_CURSOR: &str = "page-2";

struct Fake {
    out: mpsc::UnboundedSender<String>,
    initialized: AtomicBool,
    client: Mutex<Value>,
    waiting: Mutex<HashMap<String, oneshot::Sender<Value>>>,
    next: AtomicU64,
    version: Option<String>,
    cancelled: Mutex<Vec<Value>>,
    repeat_cursor: bool,
}

fn tool_def(name: &str) -> Value {
    json!({"name": name, "description": format!("fake tool {name}"), "inputSchema": {"type": "object"}})
}

fn text(t: impl Into<String>) -> Value {
    json!({"content": [{"type": "text", "text": t.into()}]})
}

fn env_value(args: &Value, key: &str) -> Option<String> {
    args[key].as_str().and_then(|name| std::env::var(name).ok())
}

impl Fake {
    fn send(&self, msg: Value) {
        let _ = self.out.send(msg.to_string());
    }

    fn reply(&self, id: Value, result: Result<Value, (i64, String)>) {
        match result {
            Ok(r) => self.send(json!({"jsonrpc": "2.0", "id": id, "result": r})),
            Err((code, message)) => {
                self.send(json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}))
            }
        }
    }

    async fn ask(&self, method: &str) -> Value {
        let id = format!("srv-{}", self.next.fetch_add(1, Ordering::Relaxed));
        let (tx, rx) = oneshot::channel();
        self.waiting.lock().unwrap().insert(id.clone(), tx);
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method}));
        match tokio::time::timeout(Duration::from_secs(5), rx).await {
            Ok(Ok(v)) => v,
            _ => json!({"missing": method}),
        }
    }

    async fn handle(self: Arc<Self>, msg: Value) {
        let method = msg["method"].as_str().map(str::to_string);
        let id = msg.get("id").cloned();
        let Some(method) = method else {
            if let Some(id) = id.as_ref().and_then(|i| i.as_str()) {
                let tx = self.waiting.lock().unwrap().remove(id);
                if let Some(tx) = tx {
                    let _ = tx.send(msg.clone());
                }
            }
            return;
        };
        let Some(id) = id else {
            if method == "notifications/initialized" {
                self.initialized.store(true, Ordering::SeqCst);
            }
            if method == "notifications/cancelled" {
                self.cancelled.lock().unwrap().push(msg["params"]["requestId"].clone());
            }
            return;
        };
        let result = match method.as_str() {
            "initialize" => {
                *self.client.lock().unwrap() = msg["params"].clone();
                let asked = msg["params"]["protocolVersion"].as_str().unwrap_or("2025-06-18").to_string();
                Ok(json!({
                    "protocolVersion": self.version.clone().unwrap_or(asked),
                    "capabilities": {"tools": {"listChanged": true}},
                    "serverInfo": {"name": "t3cm-fake-mcp", "version": "1.2.3"},
                    "instructions": "Fake MCP server for t3cm tests.\nIt echoes, fails and pages on demand.",
                }))
            }
            "ping" => Ok(json!({})),
            "tools/list" => self.list(&msg["params"]),
            "tools/call" => self.call(&msg["params"]).await,
            other => Err((-32601, format!("method not found: {other}"))),
        };
        self.reply(id, result);
    }

    fn list(&self, params: &Value) -> Result<Value, (i64, String)> {
        if !self.initialized.load(Ordering::SeqCst) {
            return Err((-32002, "tools/list before notifications/initialized".into()));
        }
        match params["cursor"].as_str() {
            None => Ok(
                json!({"tools": PAGE_ONE.iter().map(|n| tool_def(n)).collect::<Vec<_>>(), "nextCursor": SECOND_CURSOR}),
            ),
            Some(SECOND_CURSOR) if self.repeat_cursor => Ok(
                json!({"tools": PAGE_TWO.iter().map(|n| tool_def(n)).collect::<Vec<_>>(), "nextCursor": SECOND_CURSOR}),
            ),
            Some(SECOND_CURSOR) => Ok(json!({"tools": PAGE_TWO.iter().map(|n| tool_def(n)).collect::<Vec<_>>()})),
            Some(other) => Err((-32602, format!("bad cursor {other}"))),
        }
    }

    async fn call(&self, params: &Value) -> Result<Value, (i64, String)> {
        let args = &params["arguments"];
        match params["name"].as_str().unwrap_or_default() {
            "client_info" => Ok(json!({"structuredContent": self.client.lock().unwrap().clone(), "content": []})),
            "echo" => {
                Ok(json!({"structuredContent": {"echo": args}, "content": [{"type": "text", "text": "ignored"}]}))
            }
            "text_json" => Ok(json!({"content": [
                {"type": "text", "text": "{\"a\": 1, "},
                {"type": "text", "text": "\"b\": [2]}"},
            ]})),
            "plain" => Ok(text("hello world")),
            "image" => Ok(json!({"content": [
                {"type": "image", "data": "aGk=", "mimeType": "image/png"},
                {"type": "text", "text": "caption"},
            ]})),
            "fail" => Ok(json!({"isError": true, "content": [{"type": "text", "text": "it broke"}]})),
            "rpc_error" => Err((-32000, "rpc went wrong".into())),
            "sleep" => {
                let ms = args["ms"].as_u64().unwrap_or(0);
                tokio::time::sleep(Duration::from_millis(ms)).await;
                Ok(json!({"structuredContent": {"slept": ms}, "content": []}))
            }
            "crash" => {
                if let Some(v) = env_value(args, "stderr_env") {
                    eprintln!("crashing with {v}");
                }
                let _ = std::io::stderr().flush();
                std::process::exit(args["code"].as_i64().unwrap_or(7) as i32);
            }
            "env" => Ok(json!({"structuredContent": {"value": env_value(args, "name")}, "content": []})),
            "fail_with_env" => Ok(json!({"isError": true, "content": [{"type": "text",
                "text": format!("token rejected: {}", env_value(args, "name").unwrap_or_default())}]})),
            "notify_changed" => {
                self.send(json!({"jsonrpc": "2.0", "method": "notifications/tools/list_changed"}));
                Ok(text("{}"))
            }
            "ping_first" => {
                let ping = self.ask("ping").await;
                let roots = self.ask("roots/list").await;
                Ok(json!({"structuredContent": {"ping": ping, "roots": roots}, "content": []}))
            }
            "cancelled" => {
                Ok(json!({"structuredContent": {"ids": self.cancelled.lock().unwrap().clone()}, "content": []}))
            }
            "parts" => Ok(json!({"content": [
                {"type": "text", "text": "first item"},
                {"type": "text", "text": "second item"},
            ]})),
            "number_text" => Ok(text("123")),
            "pid" => Ok(json!({"structuredContent": {"pid": std::process::id()}, "content": []})),
            "flood" => {
                let bytes = args["bytes"].as_u64().unwrap_or(0) as usize;
                self.send(Value::String("x".repeat(bytes)));
                Ok(text("{}"))
            }
            "ping_flood" => {
                for i in 0..args["n"].as_u64().unwrap_or(0) {
                    self.send(json!({"jsonrpc": "2.0", "id": format!("flood-{i}"), "method": "ping"}));
                }
                Ok(text("\"flooded\""))
            }
            other => Err((-32602, format!("unknown tool {other}"))),
        }
    }
}

fn fake_t3_cli(log: &str) -> ! {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(log).unwrap();
    writeln!(f, "{}", args.join(" ")).unwrap();
    match args.iter().take(3).map(String::as_str).collect::<Vec<_>>()[..] {
        ["auth", "pairing", "create"] => {
            println!("Pairing created");
            println!("{}", json!({"credential": "PAIR-CODE"}));
        }
        ["auth", "session", "list"] => println!("{}", std::env::var("FAKE_T3_SESSIONS").unwrap_or_default()),
        _ => {}
    }
    std::process::exit(0)
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() {
    if let Ok(log) = std::env::var("FAKE_T3_LOG") {
        fake_t3_cli(&log);
    }
    if let Ok(msg) = std::env::var("FAKE_MCP_DIE_AT_START") {
        eprintln!("{msg}");
        std::process::exit(3);
    }
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    let writer = tokio::spawn(async move {
        let mut stdout = tokio::io::stdout();
        while let Some(line) = rx.recv().await {
            if stdout.write_all(format!("{line}\n").as_bytes()).await.is_err() || stdout.flush().await.is_err() {
                break;
            }
        }
    });
    let _ = tx.send("t3cm-fake-mcp starting (not JSON)".into());
    let fake = Arc::new(Fake {
        out: tx,
        initialized: AtomicBool::new(false),
        client: Mutex::new(Value::Null),
        waiting: Mutex::new(HashMap::new()),
        next: AtomicU64::new(1),
        version: std::env::var("FAKE_MCP_VERSION").ok(),
        cancelled: Mutex::new(Vec::new()),
        repeat_cursor: std::env::var("FAKE_MCP_REPEAT_CURSOR").is_ok(),
    });
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
        if msg.get("method").is_some() && msg.get("id").is_none() {
            fake.clone().handle(msg).await;
        } else {
            tokio::spawn(fake.clone().handle(msg));
        }
    }
    if std::env::var("FAKE_MCP_IGNORE_EOF").is_ok() {
        loop {
            tokio::time::sleep(Duration::from_secs(3600)).await;
        }
    }
    drop(fake);
    let _ = tokio::time::timeout(Duration::from_millis(200), writer).await;
}
