use std::future::Future;
use std::pin::Pin;

use serde::Serialize;
use serde_json::Value;

pub type BoxFut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Debug, Clone, PartialEq)]
pub enum T3Error {
    Tool(String),
    Unavailable(String),
}

impl T3Error {
    pub fn message(&self) -> &str {
        match self {
            T3Error::Tool(m) | T3Error::Unavailable(m) => m,
        }
    }

    pub fn is_not_found(&self) -> bool {
        matches!(self, T3Error::Tool(m) if m.to_lowercase().contains("not found"))
    }
}

impl std::fmt::Display for T3Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}

impl std::error::Error for T3Error {}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct T3Status {
    pub connected: bool,
    pub origin: Option<String>,
    pub token_expires_at: Option<String>,
    pub last_error: Option<String>,
}

pub trait T3Client: Send + Sync {
    fn call<'a>(&'a self, tool: &'a str, args: Value) -> BoxFut<'a, Result<Value, T3Error>>;

    fn list_tools(&self) -> BoxFut<'_, Result<Vec<Value>, T3Error>>;

    fn status(&self) -> T3Status {
        T3Status { connected: true, ..T3Status::default() }
    }

    fn redact(&self, text: String) -> String {
        text
    }
}

pub fn unwrap_tool_result(tool: &str, result: &Value) -> Result<Value, T3Error> {
    let text = || -> String {
        result["content"]
            .as_array()
            .map(|parts| {
                parts
                    .iter()
                    .filter(|p| p["type"] == "text")
                    .filter_map(|p| p["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("")
            })
            .unwrap_or_default()
    };
    if result["isError"] == true {
        let msg = text();
        let msg = if msg.is_empty() { "the tool reported an error".into() } else { msg };
        return Err(T3Error::Tool(format!("{tool}: {}", truncate_chars(&msg, 1000))));
    }
    if let Some(sc) = result.get("structuredContent")
        && !sc.is_null()
    {
        return Ok(sc.clone());
    }
    let t = text();
    Ok(serde_json::from_str(&t).unwrap_or(Value::String(t)))
}

pub fn truncate_chars(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((i, _)) => format!("{}...", &s[..i]),
        None => s.to_string(),
    }
}
