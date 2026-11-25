use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command};
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use tokio::time::Instant;

use super::{
    BoxFut, CLIENT_NAME, ERROR_CHARS, MAX_MESSAGE_BYTES, MCP_PROTOCOL_VERSION, ResolvedTransport, ResolvedUpstream,
    STDERR_LINES, Secret, ToolDef, Transport, Upstream, UpstreamError, UpstreamState, UpstreamStatus, qualified,
    redact_secrets, unwrap_call_result,
};
use crate::mcp::SUPPORTED_VERSIONS;
use crate::platform::ProcessTree;
use crate::t3::truncate_chars;

const MAX_PAGES: usize = 50;
const SHUTDOWN_GRACE: Duration = Duration::from_secs(2);
const EXIT_GRACE: Duration = Duration::from_secs(1);
const STDERR_DRAIN: Duration = Duration::from_millis(500);
const STDERR_IN_ERRORS: usize = 5;
const STDERR_LINE_BYTES: usize = 4 * ERROR_CHARS;
const OUTBOX: usize = 64;
pub const INHERITED_ENV: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "SHELL",
    "LANG",
    "LANGUAGE",
    "TZ",
    "TMPDIR",
    "TERM",
    "XDG_RUNTIME_DIR",
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
    "XDG_CACHE_HOME",
    "XDG_STATE_HOME",
];

pub const INHERITED_ENV_WINDOWS: &[&str] = &[
    "SYSTEMROOT",
    "SYSTEMDRIVE",
    "WINDIR",
    "COMSPEC",
    "PATHEXT",
    "TEMP",
    "TMP",
    "USERPROFILE",
    "USERNAME",
    "USERDOMAIN",
    "HOMEDRIVE",
    "HOMEPATH",
    "APPDATA",
    "LOCALAPPDATA",
    "PROGRAMDATA",
    "PROGRAMFILES",
    "PROGRAMFILES(X86)",
    "PROGRAMW6432",
    "COMMONPROGRAMFILES",
    "COMMONPROGRAMFILES(X86)",
    "COMMONPROGRAMW6432",
    "COMPUTERNAME",
    "NUMBER_OF_PROCESSORS",
    "PROCESSOR_ARCHITECTURE",
    "OS",
];

pub fn inherited_env(key: &str) -> bool {
    if cfg!(windows) {
        let key = key.to_ascii_uppercase();
        return INHERITED_ENV.contains(&key.as_str())
            || INHERITED_ENV_WINDOWS.contains(&key.as_str())
            || key.starts_with("LC_");
    }
    INHERITED_ENV.contains(&key) || key.starts_with("LC_")
}

pub(super) enum RpcError {
    Remote(Value),
    Failed(UpstreamError),
}

pub(super) fn initialize_params() -> Value {
    json!({
        "protocolVersion": MCP_PROTOCOL_VERSION,
        "capabilities": {},
        "clientInfo": {"name": CLIENT_NAME, "version": env!("CARGO_PKG_VERSION")},
    })
}

pub(super) fn accept_initialize(ns: &str, result: &Value) -> Result<(String, Option<Value>), UpstreamError> {
    let version = result["protocolVersion"].as_str().unwrap_or_default();
    if !SUPPORTED_VERSIONS.contains(&version) {
        return Err(UpstreamError::Unavailable(format!(
            "{ns}: unsupported MCP protocol version {:?} (supported: {})",
            truncate_chars(version, 64),
            SUPPORTED_VERSIONS.join(", ")
        )));
    }
    let mut server = result.get("serverInfo").filter(|v| v.is_object()).cloned();
    if let Some(text) = result["instructions"].as_str().filter(|t| !t.trim().is_empty()) {
        let info = server.get_or_insert_with(|| json!({}));
        info["instructions"] = Value::String(text.to_string());
    }
    Ok((version.to_string(), server))
}

pub(super) fn remote_message(err: &Value) -> String {
    let msg = err["message"].as_str().map(str::to_string).unwrap_or_else(|| err.to_string());
    truncate_chars(&msg, ERROR_CHARS)
}

pub(super) fn handshake_error(ns: &str, method: &str, e: RpcError) -> UpstreamError {
    match e {
        RpcError::Remote(err) => UpstreamError::Unavailable(format!("{ns}: {method} failed: {}", remote_message(&err))),
        RpcError::Failed(e) => e,
    }
}

pub(super) fn call_outcome(ns: &str, tool: &str, r: Result<Value, RpcError>) -> Result<Value, UpstreamError> {
    let q = qualified(ns, tool);
    match r {
        Ok(result) => unwrap_call_result(&q, &result),
        Err(RpcError::Remote(err)) => Err(UpstreamError::Tool(format!("{q}: {}", remote_message(&err)))),
        Err(RpcError::Failed(e)) => Err(e),
    }
}

pub(super) fn server_request_reply(msg: &Value) -> Option<Value> {
    let method = msg["method"].as_str()?;
    let id = msg.get("id").filter(|id| !id.is_null())?.clone();
    Some(if method == "ping" {
        json!({"jsonrpc": "2.0", "id": id, "result": {}})
    } else {
        json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": format!("method not found: {}", truncate_chars(method, 100))}})
    })
}

pub(super) fn is_list_changed(msg: &Value) -> bool {
    msg["method"] == "notifications/tools/list_changed" && msg.get("id").is_none()
}

pub(super) fn response_id(msg: &Value) -> Option<u64> {
    if msg.get("method").is_some() || (msg.get("result").is_none() && msg.get("error").is_none()) {
        return None;
    }
    match &msg["id"] {
        Value::Number(n) => n.as_u64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

pub(super) fn into_reply(msg: Value) -> Result<Value, RpcError> {
    match msg.get("error").filter(|e| !e.is_null()) {
        Some(e) => Err(RpcError::Remote(e.clone())),
        None => Ok(msg["result"].clone()),
    }
}

pub(super) fn messages(v: Value) -> Vec<Value> {
    match v {
        Value::Array(a) => a,
        v => vec![v],
    }
}

pub(super) fn redact_error(e: UpstreamError, secrets: &[Secret]) -> UpstreamError {
    let r = |m: String| redact_secrets(m, secrets);
    match e {
        UpstreamError::Tool(m) => UpstreamError::Tool(r(m)),
        UpstreamError::Unavailable(m) => UpstreamError::Unavailable(r(m)),
        UpstreamError::Timeout(m) => UpstreamError::Timeout(r(m)),
        UpstreamError::Denied(m) => UpstreamError::Denied(r(m)),
        UpstreamError::UnknownTool(m) => UpstreamError::UnknownTool(r(m)),
        UpstreamError::UnknownNamespace(m) => UpstreamError::UnknownNamespace(r(m)),
        UpstreamError::Config(m) => UpstreamError::Config(r(m)),
    }
}

pub(super) async fn list_pages<F, Fut>(ns: &str, mut page: F) -> Result<Vec<ToolDef>, UpstreamError>
where
    F: FnMut(Value) -> Fut,
    Fut: Future<Output = Result<Value, RpcError>>,
{
    let mut tools = Vec::new();
    let mut cursor: Option<String> = None;
    let mut seen = std::collections::HashSet::new();
    for _ in 0..MAX_PAGES {
        let params = match &cursor {
            Some(c) => json!({"cursor": c}),
            None => json!({}),
        };
        let result = page(params).await.map_err(|e| handshake_error(ns, "tools/list", e))?;
        tools.extend(result["tools"].as_array().into_iter().flatten().filter_map(ToolDef::from_mcp));
        match result["nextCursor"].as_str().filter(|c| !c.is_empty()) {
            Some(c) if !seen.insert(c.to_string()) => {
                return Err(UpstreamError::Unavailable(format!(
                    "{ns}: tools/list repeated the cursor {:?}",
                    truncate_chars(c, 64)
                )));
            }
            Some(c) => cursor = Some(c.to_string()),
            None => return Ok(tools),
        }
    }
    Err(UpstreamError::Unavailable(format!("{ns}: tools/list did not finish within {MAX_PAGES} pages")))
}

type Reply = Result<Value, RpcError>;

#[derive(Default)]
struct WireState {
    pending: HashMap<u64, oneshot::Sender<Reply>>,
    dead: Option<String>,
}

struct Wire {
    ns: String,
    secrets: Vec<Secret>,
    state: Mutex<WireState>,
    out: Mutex<Option<mpsc::Sender<String>>>,
    changed: AtomicBool,
    stderr: Mutex<VecDeque<String>>,
    exited: watch::Sender<bool>,
}

impl Wire {
    fn redact(&self, text: String) -> String {
        redact_secrets(text, &self.secrets)
    }

    fn dead(&self) -> Option<String> {
        self.state.lock().unwrap().dead.clone()
    }

    fn fail_all(&self, reason: String) -> String {
        let mut st = self.state.lock().unwrap();
        let reason = st.dead.get_or_insert_with(|| truncate_chars(&redact_secrets(reason, &self.secrets), ERROR_CHARS));
        let reason = reason.clone();
        for (_, tx) in st.pending.drain() {
            let _ = tx.send(Err(RpcError::Failed(UpstreamError::Unavailable(reason.clone()))));
        }
        reason
    }

    fn stderr_tail(&self) -> Option<String> {
        let ring = self.stderr.lock().unwrap();
        let skip = ring.len().saturating_sub(STDERR_IN_ERRORS);
        let tail = ring.iter().skip(skip).cloned().collect::<Vec<_>>().join(" | ");
        if tail.is_empty() { None } else { Some(tail) }
    }

    fn exit_message(&self, status: &str) -> String {
        let mut msg = format!("{}: process exited ({status})", self.ns);
        if let Some(tail) = self.stderr_tail() {
            msg.push_str("; stderr: ");
            msg.push_str(&tail);
        }
        msg
    }

    fn line(msg: &Value) -> String {
        let mut line = msg.to_string();
        line.push('\n');
        line
    }

    fn sender(&self) -> Option<mpsc::Sender<String>> {
        self.out.lock().unwrap().clone()
    }

    fn close_stdin(&self) {
        self.out.lock().unwrap().take();
    }

    async fn write(&self, msg: &Value) -> Result<(), String> {
        let tx = self.sender().ok_or_else(|| "stdin closed".to_string())?;
        tx.send(Wire::line(msg)).await.map_err(|_| "stdin closed".to_string())
    }

    fn try_write(&self, msg: &Value) -> bool {
        self.sender().is_some_and(|tx| tx.try_send(Wire::line(msg)).is_ok())
    }

    async fn wait_exited(&self, limit: Duration) -> bool {
        let mut rx = self.exited.subscribe();
        tokio::time::timeout(limit, rx.wait_for(|e| *e)).await.is_ok_and(|r| r.is_ok())
    }

    async fn died(&self, fallback: String) -> String {
        if let Some(d) = self.dead() {
            return d;
        }
        self.wait_exited(EXIT_GRACE).await;
        self.fail_all(fallback)
    }

    fn dispatch(self: &Arc<Self>, raw: &str) {
        let Ok(v) = serde_json::from_str::<Value>(raw.trim()) else { return };
        for msg in messages(v) {
            if let Some(reply) = server_request_reply(&msg) {
                if !self.try_write(&reply) {
                    tracing::debug!(upstream = %self.ns, "dropped a reply to a server request: the outbox is full");
                }
            } else if is_list_changed(&msg) {
                self.changed.store(true, Ordering::SeqCst);
            } else if let Some(id) = response_id(&msg) {
                let tx = self.state.lock().unwrap().pending.remove(&id);
                if let Some(tx) = tx {
                    let _ = tx.send(into_reply(msg));
                }
            }
        }
    }
}

enum Line {
    Eof,
    Full(String),
    Cut(String),
}

async fn next_line<R: AsyncBufRead + Unpin>(reader: &mut R, buf: &mut Vec<u8>, cap: usize) -> Line {
    buf.clear();
    let mut cut = false;
    loop {
        let (used, done) = match reader.fill_buf().await {
            Err(_) => return Line::Eof,
            Ok([]) if buf.is_empty() && !cut => return Line::Eof,
            Ok([]) => (0, true),
            Ok(chunk) => {
                let (used, done) = match chunk.iter().position(|b| *b == b'\n') {
                    Some(i) => (i + 1, true),
                    None => (chunk.len(), false),
                };
                let room = cap.saturating_sub(buf.len());
                cut |= used > room;
                buf.extend_from_slice(&chunk[..used.min(room)]);
                (used, done)
            }
        };
        reader.consume(used);
        if done {
            let text = String::from_utf8_lossy(buf).trim_end_matches(['\r', '\n']).to_string();
            return if cut { Line::Cut(text) } else { Line::Full(text) };
        }
    }
}

async fn read_stdout(stdout: ChildStdout, wire: Arc<Wire>) {
    let mut reader = BufReader::new(stdout);
    let mut buf = Vec::new();
    loop {
        match next_line(&mut reader, &mut buf, MAX_MESSAGE_BYTES).await {
            Line::Full(line) => wire.dispatch(&line),
            Line::Cut(_) => {
                wire.close_stdin();
                wire.fail_all(format!("{}: a message from the server is over {MAX_MESSAGE_BYTES} bytes", wire.ns));
                return;
            }
            Line::Eof => break,
        }
    }
    if !wire.wait_exited(EXIT_GRACE).await {
        wire.fail_all(format!("{}: process exited (stdout closed)", wire.ns));
    }
}

async fn write_stdin(mut stdin: ChildStdin, mut rx: mpsc::Receiver<String>, wire: Arc<Wire>) {
    while let Some(line) = rx.recv().await {
        let written = match stdin.write_all(line.as_bytes()).await {
            Ok(()) => stdin.flush().await,
            Err(e) => Err(e),
        };
        if let Err(e) = written {
            wire.close_stdin();
            rx.close();
            drop(stdin);
            wire.died(format!("{}: process exited (write failed: {e})", wire.ns)).await;
            return;
        }
    }
}

async fn drain_stderr(stderr: ChildStderr, wire: Arc<Wire>) {
    let mut reader = BufReader::new(stderr);
    let mut buf = Vec::new();
    loop {
        let line = match next_line(&mut reader, &mut buf, STDERR_LINE_BYTES).await {
            Line::Full(l) | Line::Cut(l) => l,
            Line::Eof => break,
        };
        let line = truncate_chars(&wire.redact(line), ERROR_CHARS);
        tracing::debug!(upstream = %wire.ns, "stderr: {line}");
        let mut ring = wire.stderr.lock().unwrap();
        ring.push_back(line);
        while ring.len() > STDERR_LINES {
            ring.pop_front();
        }
    }
}

async fn watch_child(
    mut child: Child,
    tree: ProcessTree,
    wire: Arc<Wire>,
    mut kill: oneshot::Receiver<()>,
    stderr: JoinHandle<()>,
) {
    let status = tokio::select! {
        s = child.wait() => s,
        _ = &mut kill => {
            tree.kill();
            let _ = child.start_kill();
            child.wait().await
        }
    };
    tree.kill();
    wire.close_stdin();
    let _ = tokio::time::timeout(STDERR_DRAIN, stderr).await;
    let status = match status {
        Ok(s) => s.to_string(),
        Err(e) => e.to_string(),
    };
    wire.fail_all(wire.exit_message(&status));
    wire.exited.send_replace(true);
}

struct PendingGuard<'a> {
    wire: &'a Wire,
    id: u64,
    cancel: bool,
}

impl Drop for PendingGuard<'_> {
    fn drop(&mut self) {
        let waiting = self.wire.state.lock().unwrap().pending.remove(&self.id).is_some();
        if waiting && self.cancel {
            let note = json!({"jsonrpc": "2.0", "method": "notifications/cancelled",
                "params": {"requestId": self.id, "reason": "timed out"}});
            self.wire.try_write(&note);
        }
    }
}

pub struct StdioClient {
    name: String,
    wire: Arc<Wire>,
    timeout: Duration,
    pid: Option<u32>,
    next_id: AtomicU64,
    server: Option<Value>,
    last_error: Mutex<Option<String>>,
    tools: AtomicUsize,
    kill: Mutex<Option<oneshot::Sender<()>>>,
}

impl StdioClient {
    pub async fn connect(spec: &ResolvedUpstream) -> Result<StdioClient, UpstreamError> {
        let ResolvedTransport::Stdio { command, args, env, cwd } = &spec.transport else {
            return Err(UpstreamError::Config(format!("{}: not a stdio upstream", spec.name)));
        };
        let mut cmd = Command::new(command);
        cmd.args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .env_clear();
        ProcessTree::prepare(&mut cmd);
        for (key, value) in std::env::vars_os() {
            if key.to_str().is_some_and(inherited_env) {
                cmd.env(&key, &value);
            }
        }
        for (key, value) in env {
            cmd.env(key, value.expose());
        }
        if let Some(dir) = cwd {
            cmd.current_dir(dir);
        }
        let mut child = cmd.spawn().map_err(|e| {
            UpstreamError::Unavailable(spec.redact(format!("{}: cannot start {}: {e}", spec.name, command.display())))
        })?;
        let tree = ProcessTree::start(&child).map_err(|e| {
            UpstreamError::Unavailable(spec.redact(format!("{}: cannot start {}: {e}", spec.name, command.display())))
        })?;
        let pid = child.id();
        let (Some(stdin), Some(stdout), Some(stderr)) = (child.stdin.take(), child.stdout.take(), child.stderr.take())
        else {
            return Err(UpstreamError::Unavailable(format!("{}: could not open the child's stdio pipes", spec.name)));
        };
        let (out_tx, out_rx) = mpsc::channel(OUTBOX);
        let wire = Arc::new(Wire {
            ns: spec.name.clone(),
            secrets: spec.secrets.clone(),
            state: Mutex::new(WireState::default()),
            out: Mutex::new(Some(out_tx)),
            changed: AtomicBool::new(false),
            stderr: Mutex::new(VecDeque::new()),
            exited: watch::Sender::new(false),
        });
        let (kill_tx, kill_rx) = oneshot::channel();
        let stderr_task = tokio::spawn(drain_stderr(stderr, wire.clone()));
        tokio::spawn(read_stdout(stdout, wire.clone()));
        tokio::spawn(write_stdin(stdin, out_rx, wire.clone()));
        tokio::spawn(watch_child(child, tree, wire.clone(), kill_rx, stderr_task));
        let mut client = StdioClient {
            name: spec.name.clone(),
            wire,
            timeout: spec.timeout,
            pid,
            next_id: AtomicU64::new(1),
            server: None,
            last_error: Mutex::new(None),
            tools: AtomicUsize::new(0),
            kill: Mutex::new(Some(kill_tx)),
        };
        match client.handshake().await {
            Ok(server) => {
                client.server = server;
                Ok(client)
            }
            Err(e) => Err(client.connect_failure(e)),
        }
    }

    pub fn pid(&self) -> Option<u32> {
        self.pid
    }

    async fn handshake(&self) -> Result<Option<Value>, UpstreamError> {
        let ns = &self.name;
        let result = self
            .request("initialize", initialize_params(), &format!("{ns}: initialize"))
            .await
            .map_err(|e| handshake_error(ns, "initialize", e))?;
        let (_, server) = accept_initialize(ns, &result)?;
        let note = json!({"jsonrpc": "2.0", "method": "notifications/initialized"});
        if let Err(e) = self.wire.write(&note).await {
            return Err(UpstreamError::Unavailable(
                self.wire.died(format!("{ns}: process exited (write failed: {e})")).await,
            ));
        }
        Ok(server)
    }

    fn connect_failure(&self, e: UpstreamError) -> UpstreamError {
        let mut msg = e.message().to_string();
        if self.wire.dead().is_none()
            && let Some(tail) = self.wire.stderr_tail()
        {
            msg.push_str("; stderr: ");
            msg.push_str(&tail);
        }
        UpstreamError::Unavailable(truncate_chars(&self.wire.redact(msg), ERROR_CHARS))
    }

    async fn request(&self, method: &str, params: Value, label: &str) -> Reply {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        {
            let mut st = self.wire.state.lock().unwrap();
            if let Some(d) = &st.dead {
                return Err(RpcError::Failed(UpstreamError::Unavailable(d.clone())));
            }
            st.pending.insert(id, tx);
        }
        let mut guard = PendingGuard { wire: &self.wire, id, cancel: false };
        let deadline = Instant::now() + self.timeout;
        let msg = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        let timed_out = || {
            RpcError::Failed(UpstreamError::Timeout(format!(
                "{label}: timed out after {} ms",
                self.timeout.as_millis()
            )))
        };
        match tokio::time::timeout_at(deadline, self.wire.write(&msg)).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                drop(guard);
                let reason = self.wire.died(format!("{}: process exited (write failed: {e})", self.name)).await;
                return Err(RpcError::Failed(UpstreamError::Unavailable(reason)));
            }
            Err(_) => return Err(timed_out()),
        }
        guard.cancel = method != "initialize";
        match tokio::time::timeout_at(deadline, rx).await {
            Ok(Ok(reply)) => reply,
            Ok(Err(_)) => {
                drop(guard);
                let reason = self.wire.died(format!("{}: process exited (connection closed)", self.name)).await;
                Err(RpcError::Failed(UpstreamError::Unavailable(reason)))
            }
            Err(_) => Err(timed_out()),
        }
    }

    fn finish<T>(&self, r: Result<T, UpstreamError>) -> Result<T, UpstreamError> {
        r.map_err(|e| {
            let e = redact_error(e, &self.wire.secrets);
            *self.last_error.lock().unwrap() = Some(e.message().to_string());
            e
        })
    }
}

impl Upstream for StdioClient {
    fn name(&self) -> &str {
        &self.name
    }

    fn list_tools(&self) -> BoxFut<'_, Result<Vec<ToolDef>, UpstreamError>> {
        Box::pin(async move {
            let label = format!("{}: tools/list", self.name);
            let r = list_pages(&self.name, |params| self.request("tools/list", params, &label)).await;
            if let Ok(tools) = &r {
                self.tools.store(tools.len(), Ordering::Relaxed);
            }
            self.finish(r)
        })
    }

    fn call<'a>(&'a self, tool: &'a str, args: Value) -> BoxFut<'a, Result<Value, UpstreamError>> {
        Box::pin(async move {
            let label = qualified(&self.name, tool);
            let r = self.request("tools/call", json!({"name": tool, "arguments": args}), &label).await;
            self.finish(call_outcome(&self.name, tool, r))
        })
    }

    fn status(&self) -> UpstreamStatus {
        let dead = self.wire.dead().is_some();
        UpstreamStatus {
            state: if dead { UpstreamState::Backoff } else { UpstreamState::Ready },
            tools: self.tools.load(Ordering::Relaxed),
            last_error: self.last_error.lock().unwrap().clone().or_else(|| self.wire.dead()),
            server: self.server.clone(),
            ..UpstreamStatus::new(&self.name, Transport::Stdio)
        }
    }

    fn tools_changed(&self) -> bool {
        self.wire.changed.swap(false, Ordering::SeqCst)
    }

    fn is_alive(&self) -> bool {
        self.wire.dead().is_none() && !*self.wire.exited.borrow()
    }

    fn redact(&self, text: String) -> String {
        self.wire.redact(text)
    }

    fn shutdown(&self) -> BoxFut<'_, ()> {
        Box::pin(async move {
            self.wire.close_stdin();
            if self.wire.wait_exited(SHUTDOWN_GRACE).await {
                return;
            }
            let kill = self.kill.lock().unwrap().take();
            if let Some(kill) = kill {
                let _ = kill.send(());
            }
            self.wire.wait_exited(SHUTDOWN_GRACE).await;
        })
    }
}
