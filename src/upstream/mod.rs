pub mod client_http;
pub mod client_stdio;
pub mod config;
pub mod fake;
pub mod manager;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub use crate::t3::BoxFut;
use crate::t3::{T3Client, T3Error, T3Status, truncate_chars};
pub use manager::UpstreamManager;

pub const T3_NAMESPACE: &str = "t3";
pub const RESERVED_NAMES: &[&str] = &["t3", "mem", "vars", "wf", "console", "tools", "catalog"];
pub const RESERVED_JS: &[&str] = &["eval", "undefined", "arguments", "escape", "unescape", "async", "globalThis"];
pub const MAX_NAME_LEN: usize = 32;
pub const DEFAULT_TIMEOUT_MS: u64 = 60_000;
pub const MIN_TIMEOUT_MS: u64 = 100;
pub const MAX_TIMEOUT_MS: u64 = 600_000;
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
pub const BACKOFF_MIN: Duration = Duration::from_secs(1);
pub const BACKOFF_MAX: Duration = Duration::from_secs(60);
pub const MCP_PROTOCOL_VERSION: &str = "2025-06-18";
pub const CLIENT_NAME: &str = "t3-codemode";
pub const STDERR_LINES: usize = 50;
pub const ERROR_CHARS: usize = 1000;
pub const MIN_SECRET_LEN: usize = 4;
pub const REDACTED: &str = "[redacted]";
pub const SECRET_NAME_HINTS: &[&str] =
    &["TOKEN", "SECRET", "KEY", "PASSWORD", "PASSWD", "AUTH", "COOKIE", "CREDENTIAL"];
pub const MAX_MESSAGE_BYTES: usize = 16 * 1024 * 1024;
pub const SERVER_INFO_CHARS: usize = 100;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Transport {
    Stdio,
    Http,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StdioConfig {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub cwd: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HttpConfig {
    pub url: String,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub allow_insecure: bool,
}

fn yes() -> bool {
    true
}

fn default_timeout_ms() -> u64 {
    DEFAULT_TIMEOUT_MS
}

fn allow_all() -> Vec<String> {
    vec!["*".into()]
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpstreamConfig {
    pub name: String,
    pub transport: Transport,
    #[serde(default)]
    pub stdio: Option<StdioConfig>,
    #[serde(default)]
    pub http: Option<HttpConfig>,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default = "allow_all")]
    pub allow: Vec<String>,
    #[serde(default)]
    pub deny: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reference {
    Literal(String),
    Env(String),
    File(PathBuf),
}

#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: String) -> Secret {
        Secret(value)
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(REDACTED)
    }
}

impl std::fmt::Display for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(REDACTED)
    }
}

#[derive(Clone, Debug)]
pub enum ResolvedTransport {
    Stdio { command: PathBuf, args: Vec<String>, env: Vec<(String, Secret)>, cwd: Option<PathBuf> },
    Http { url: Secret, headers: Vec<(String, Secret)> },
}

#[derive(Clone, Debug)]
pub struct ResolvedUpstream {
    pub name: String,
    pub transport: ResolvedTransport,
    pub timeout: Duration,
    pub secrets: Vec<Secret>,
}

impl ResolvedUpstream {
    pub fn redact(&self, text: String) -> String {
        redact_secrets(text, &self.secrets)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ToolFilter {
    pub allow: Vec<String>,
    pub deny: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotations: Option<Value>,
}

impl ToolDef {
    pub fn from_mcp(tool: &Value) -> Option<ToolDef> {
        let name = tool["name"].as_str().filter(|n| !n.is_empty())?.to_string();
        Some(ToolDef {
            name,
            description: tool["description"].as_str().unwrap_or_default().to_string(),
            input_schema: tool
                .get("inputSchema")
                .filter(|v| v.is_object())
                .cloned()
                .unwrap_or_else(|| json!({"type": "object"})),
            output_schema: tool.get("outputSchema").filter(|v| v.is_object()).cloned(),
            annotations: tool.get("annotations").filter(|v| v.is_object()).cloned(),
        })
    }

    pub fn to_mcp(&self) -> Value {
        let mut v = json!({"name": self.name, "description": self.description, "inputSchema": self.input_schema});
        if let Some(o) = &self.output_schema {
            v["outputSchema"] = o.clone();
        }
        if let Some(a) = &self.annotations {
            v["annotations"] = a.clone();
        }
        v
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum UpstreamError {
    Tool(String),
    Unavailable(String),
    Timeout(String),
    Denied(String),
    UnknownTool(String),
    UnknownNamespace(String),
    Config(String),
}

impl UpstreamError {
    pub fn message(&self) -> &str {
        match self {
            UpstreamError::Tool(m)
            | UpstreamError::Unavailable(m)
            | UpstreamError::Timeout(m)
            | UpstreamError::Denied(m)
            | UpstreamError::UnknownTool(m)
            | UpstreamError::UnknownNamespace(m)
            | UpstreamError::Config(m) => m,
        }
    }

    pub fn restarts_connection(&self) -> bool {
        matches!(self, UpstreamError::Unavailable(_))
    }

    pub fn not_implemented(what: &str) -> UpstreamError {
        UpstreamError::Unavailable(format!("{what}: not implemented"))
    }
}

impl std::fmt::Display for UpstreamError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}

impl std::error::Error for UpstreamError {}

impl From<T3Error> for UpstreamError {
    fn from(e: T3Error) -> UpstreamError {
        match e {
            T3Error::Tool(m) => UpstreamError::Tool(m),
            T3Error::Unavailable(m) => UpstreamError::Unavailable(m),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum UpstreamState {
    Disabled,
    #[default]
    Idle,
    Connecting,
    Ready,
    Backoff,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpstreamStatus {
    pub name: String,
    pub transport: Transport,
    pub enabled: bool,
    pub state: UpstreamState,
    pub tools: usize,
    pub restarts: u32,
    pub last_error: Option<String>,
    pub retry_at: Option<String>,
    pub server: Option<Value>,
}

impl UpstreamStatus {
    pub fn new(name: &str, transport: Transport) -> UpstreamStatus {
        UpstreamStatus {
            name: name.into(),
            transport,
            enabled: true,
            state: UpstreamState::Idle,
            tools: 0,
            restarts: 0,
            last_error: None,
            retry_at: None,
            server: None,
        }
    }
}

pub trait Upstream: Send + Sync {
    fn name(&self) -> &str;

    fn list_tools(&self) -> BoxFut<'_, Result<Vec<ToolDef>, UpstreamError>>;

    fn call<'a>(&'a self, tool: &'a str, args: Value) -> BoxFut<'a, Result<Value, UpstreamError>>;

    fn status(&self) -> UpstreamStatus;

    fn tools_changed(&self) -> bool {
        false
    }

    fn is_alive(&self) -> bool {
        true
    }

    fn redact(&self, text: String) -> String {
        text
    }

    fn shutdown(&self) -> BoxFut<'_, ()> {
        Box::pin(async {})
    }
}

pub trait Connector: Send + Sync {
    fn connect<'a>(&'a self, spec: &'a ResolvedUpstream) -> BoxFut<'a, Result<Arc<dyn Upstream>, UpstreamError>>;
}

pub struct McpConnector;

impl Connector for McpConnector {
    fn connect<'a>(&'a self, spec: &'a ResolvedUpstream) -> BoxFut<'a, Result<Arc<dyn Upstream>, UpstreamError>> {
        Box::pin(async move {
            let up: Arc<dyn Upstream> = match &spec.transport {
                ResolvedTransport::Stdio { .. } => Arc::new(client_stdio::StdioClient::connect(spec).await?),
                ResolvedTransport::Http { .. } => Arc::new(client_http::HttpClient::connect(spec).await?),
            };
            Ok(up)
        })
    }
}

pub struct T3Upstream {
    client: Arc<dyn T3Client>,
}

impl T3Upstream {
    pub fn new(client: Arc<dyn T3Client>) -> T3Upstream {
        T3Upstream { client }
    }
}

impl Upstream for T3Upstream {
    fn name(&self) -> &str {
        T3_NAMESPACE
    }

    fn list_tools(&self) -> BoxFut<'_, Result<Vec<ToolDef>, UpstreamError>> {
        Box::pin(async move {
            let tools = self.client.list_tools().await?;
            Ok(tools.iter().filter_map(ToolDef::from_mcp).collect())
        })
    }

    fn call<'a>(&'a self, tool: &'a str, args: Value) -> BoxFut<'a, Result<Value, UpstreamError>> {
        Box::pin(async move { Ok(self.client.call(tool, args).await?) })
    }

    fn status(&self) -> UpstreamStatus {
        let T3Status { connected, origin, last_error, .. } = self.client.status();
        UpstreamStatus {
            state: if connected { UpstreamState::Ready } else { UpstreamState::Backoff },
            last_error,
            server: origin.map(|o| json!({"origin": o})),
            ..UpstreamStatus::new(T3_NAMESPACE, Transport::Http)
        }
    }

    fn redact(&self, text: String) -> String {
        self.client.redact(text)
    }
}

pub fn qualified(ns: &str, tool: &str) -> String {
    format!("{ns}.{tool}")
}

pub fn split_qualified(name: &str) -> Option<(&str, &str)> {
    let (ns, tool) = name.split_once('.')?;
    if ns.is_empty() || tool.is_empty() { None } else { Some((ns, tool)) }
}

pub fn is_loopback_host(host: &str) -> bool {
    let host = host.trim_start_matches('[').trim_end_matches(']');
    host.eq_ignore_ascii_case("localhost")
        || host.to_ascii_lowercase().ends_with(".localhost")
        || host.parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

fn json_escaped(s: &str) -> String {
    let quoted = Value::String(s.to_string()).to_string();
    quoted[1..quoted.len() - 1].to_string()
}

pub fn redact_secrets(text: String, secrets: &[Secret]) -> String {
    let mut out = text;
    for s in secrets {
        let raw = s.expose();
        if raw.len() < MIN_SECRET_LEN {
            continue;
        }
        let once = json_escaped(raw);
        let twice = json_escaped(&once);
        for form in [raw, once.as_str(), twice.as_str()] {
            if out.contains(form) {
                out = out.replace(form, REDACTED);
            }
        }
    }
    out
}

pub fn redact_json(value: Value, redact: &dyn Fn(String) -> String) -> Value {
    match value {
        Value::String(s) => Value::String(redact(s)),
        Value::Array(a) => Value::Array(a.into_iter().map(|v| redact_json(v, redact)).collect()),
        Value::Object(o) => Value::Object(o.into_iter().map(|(k, v)| (redact(k), redact_json(v, redact))).collect()),
        Value::Number(n) => {
            let text = n.to_string();
            let clean = redact(text.clone());
            if clean == text { Value::Number(n) } else { Value::String(clean) }
        }
        v => v,
    }
}

pub fn server_info(info: Option<Value>, redact: &dyn Fn(String) -> String) -> Option<Value> {
    let info = info?;
    let mut out = serde_json::Map::new();
    for key in ["name", "title", "version"] {
        if let Some(v) = info[key].as_str() {
            out.insert(key.into(), Value::String(redact(truncate_chars(v, SERVER_INFO_CHARS))));
        }
    }
    if out.is_empty() { None } else { Some(Value::Object(out)) }
}

pub fn unwrap_call_result(qualified_name: &str, result: &Value) -> Result<Value, UpstreamError> {
    let parts = result["content"].as_array().cloned().unwrap_or_default();
    let all_text = parts.iter().all(|p| p["type"] == "text");
    let texts: Vec<&str> = parts.iter().filter_map(|p| p["text"].as_str()).collect();
    if result["isError"] == true {
        let msg = texts.join("\n");
        let msg = if msg.is_empty() { "the tool reported an error".into() } else { msg };
        return Err(UpstreamError::Tool(format!("{qualified_name}: {}", truncate_chars(&msg, ERROR_CHARS))));
    }
    if let Some(sc) = result.get("structuredContent")
        && !sc.is_null()
    {
        return Ok(sc.clone());
    }
    if !all_text {
        return Ok(Value::Array(parts));
    }
    let joined = texts.concat();
    if let Ok(v @ (Value::Object(_) | Value::Array(_) | Value::String(_))) = serde_json::from_str::<Value>(&joined) {
        return Ok(v);
    }
    Ok(match texts.len() {
        0 | 1 => Value::String(joined),
        _ => Value::Array(texts.into_iter().map(|t| Value::String(t.to_string())).collect()),
    })
}
