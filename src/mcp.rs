use std::sync::Arc;

use serde_json::{Value, json};

use crate::app::{App, ExecuteRequest, ToolOutput};

pub const SUPPORTED_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

pub const SEARCH_DESCRIPTION: &str = r#"Search the T3 Code API before you call `execute`. Write a JavaScript async arrow function; it runs in a sandbox whose only globals are `catalog` and `console`, and its return value comes back as JSON.

`catalog` is an array of { name, kind, description, signature, inputSchema }. `signature` is a TypeScript declaration with the input and result types. kind "t3": T3 Code tools, called in execute as t3.<name>(input). kind "wf": the thread-workflow layer (wf.*). kind "mem": persistent memory (mem.*, vars).

Examples:
async () => catalog.filter(t => /thread/.test(t.name)).map(t => t.name)
async () => catalog.find(t => t.name === "t3_thread_send").signature
async () => catalog.filter(t => t.kind === "wf").map(t => t.signature).join("\n\n")"#;

pub const EXECUTE_DESCRIPTION: &str = r#"Run JavaScript against T3 Code. Use `search` first to find tool names and their TypeScript signatures; do not guess.

Write an async arrow function that returns a JSON-serializable value. TypeScript annotations are fine (they are stripped). The ONLY globals are:
- t3: every T3 Code tool by exact name, e.g. await t3.t3_thread_list({ allProjects: true }); also without the t3_ prefix, and t3.call(name, input).
- mem: persistent key-value store: get(key), set(key, value), del(key), list(prefix?); mem.ns(name) opens a shared namespace.
- vars: a plain object restored before the run and saved after it, per session.
- wf: the thread-workflow layer (parent/child links, notes and wakes, gates, the waiting list); search kind "wf".
- console: log/warn/error lines come back as `logs`.
There is no fs, require, process, fetch, import or timers.

Returns {"result": ..., "logs": [...]}; output over 24,000 chars is cut structurally. A failed T3 call throws an Error carrying T3's message. A tool denied by policy throws too: do not retry it. timeout_ms defaults to 60000 (max 600000). session names the mem/vars namespace (default: your thread).

Example:
async () => {
  const { threads } = await t3.t3_thread_list({ allProjects: true, statuses: ["idle"], limit: 50 });
  vars.idle = threads.map(t => ({ id: t.threadId, title: t.title }));
  return { idle: vars.idle.length };
}"#;

pub fn tools() -> Value {
    json!([
        {
            "name": "search",
            "title": "Search the T3 Code API",
            "description": SEARCH_DESCRIPTION,
            "inputSchema": {
                "type": "object",
                "properties": {
                    "code": {"type": "string", "description": "JavaScript async arrow function that queries `catalog` and returns the result"}
                },
                "required": ["code"],
                "additionalProperties": false
            },
            "annotations": {"title": "Search the T3 Code API", "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false}
        },
        {
            "name": "execute",
            "title": "Run code against T3 Code",
            "description": EXECUTE_DESCRIPTION,
            "inputSchema": {
                "type": "object",
                "properties": {
                    "code": {"type": "string", "description": "JavaScript async arrow function to run"},
                    "session": {"type": "string", "description": "mem/vars namespace; default thread:<your thread>"},
                    "timeout_ms": {"type": "integer", "minimum": 100, "maximum": 600000, "description": "Wall-clock limit; default 60000"},
                    "thread": {"type": "string", "description": "Your T3 thread id, only when the connection does not send it"}
                },
                "required": ["code"],
                "additionalProperties": false
            },
            "annotations": {"title": "Run code against T3 Code", "readOnlyHint": false, "destructiveHint": true, "idempotentHint": false, "openWorldHint": true}
        }
    ])
}

fn ok(id: Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn err(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

fn tool_result(out: ToolOutput) -> Value {
    let mut r = json!({"content": [{"type": "text", "text": out.text}]});
    if out.is_error {
        r["isError"] = json!(true);
    }
    r
}

pub fn caller_from_header(raw: Option<&str>) -> Option<String> {
    let v = raw?.trim();
    if v.is_empty() || v.contains("${") { None } else { Some(v.to_string()) }
}

pub async fn handle(app: &Arc<App>, msg: Value, caller: Option<&str>) -> Option<Value> {
    let Some(method) = msg.get("method").and_then(Value::as_str) else {
        return msg.get("id").map(|id| err(id.clone(), -32600, "invalid request: no method"));
    };
    let id = msg.get("id").cloned()?;
    let params = msg.get("params").cloned().unwrap_or(Value::Null);
    Some(match method {
        "initialize" => {
            let asked = params["protocolVersion"].as_str().unwrap_or("");
            let version = if SUPPORTED_VERSIONS.contains(&asked) { asked } else { SUPPORTED_VERSIONS[0] };
            ok(
                id,
                json!({
                    "protocolVersion": version,
                    "capabilities": {"tools": {"listChanged": false}},
                    "serverInfo": {"name": "t3-codemode", "version": env!("CARGO_PKG_VERSION")},
                    "instructions": "T3 Code in two tools: `search` the API catalog with code, then `execute` code against it."
                }),
            )
        }
        "ping" => ok(id, json!({})),
        "tools/list" => ok(id, json!({"tools": tools()})),
        "tools/call" => {
            let name = params["name"].as_str().unwrap_or("");
            let args = &params["arguments"];
            let Some(code) = args["code"].as_str().map(String::from) else {
                if name != "search" && name != "execute" {
                    return Some(err(id, -32602, &format!("unknown tool {name:?}")));
                }
                return Some(ok(
                    id,
                    tool_result(ToolOutput { is_error: true, text: "Error: `code` is required".into() }),
                ));
            };
            let out = match name {
                "search" => app.search(&code).await,
                "execute" => {
                    let thread = args["thread"].as_str().map(String::from);
                    app.execute(ExecuteRequest {
                        code,
                        session: args["session"].as_str().map(String::from),
                        timeout_ms: args["timeout_ms"].as_u64(),
                        caller: caller.map(String::from).or(thread),
                    })
                    .await
                }
                other => return Some(err(id, -32602, &format!("unknown tool {other:?}"))),
            };
            ok(id, tool_result(out))
        }
        other => err(id, -32601, &format!("method not found: {other}")),
    })
}
