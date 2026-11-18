# Upstreams contract

t3-codemode becomes a Code Mode for any MCP servers. An **upstream** is another
MCP server (stdio or Streamable HTTP) listed in `config.json`. Its tools join
the catalog under the upstream's **namespace**: `search` finds them, `execute`
calls them. The service holds the upstream's credentials; they never reach the
sandbox, tool output, `/healthz` or logs. T3 stays built in as namespace `t3`
with its OAuth client and every existing behavior.

This file is the interface three builders work against in parallel. The
shared types, the trait and every public signature already exist and compile
(`src/upstream/mod.rs`, stubs in the owned files). Builders change bodies and
private items in their own files only.

## 1. How T3 fits

T3 implements the `Upstream` trait through `T3Upstream` (an adapter over the
existing `T3Client`, done in `mod.rs`). It is **not** owned by
`UpstreamManager`: its lifecycle stays with `HttpT3` (OAuth, token refresh,
re-auth on 401), and `App` keeps the T3 path it has today (the `t3_` prefix
alias, `config.deny`, the cached tool list in the store, `kind: "t3"`). The
manager owns only configured upstreams. Routing in `App`:

- namespace `t3` goes to the existing T3 path;
- any other namespace goes to `UpstreamManager::call(ns, tool, args)`.

`wf` keeps using `T3Client` directly. `T3Upstream` exists so code that wants
"every namespace" (status, a future uniform catalog) has one shape.

## 2. Config schema

`~/.config/t3-codemode/config.json` (or `$T3CM_CONFIG`) gains a top-level
`upstreams` array. Arrays replace the built-in default (`[]`) in the deep
merge. Keys inside an upstream are snake_case, as below.

| Key | Type | Default | Rule |
|---|---|---|---|
| `name` | string | required | the JS namespace; identifier rules in section 4 |
| `transport` | `"stdio"` \| `"http"` | required | selects the section below |
| `stdio.command` | string | required for stdio | bare name (searched on `PATH`, then `$HOME/.local/bin`; on Windows each `PATHEXT` extension too) or a path (`~` expanded); no references |
| `stdio.args` | string[] | `[]` | no references (argv is visible in `ps`) |
| `stdio.env` | {string: string} | `{}` | values may hold references |
| `stdio.cwd` | string | service cwd | `~` expanded; must exist at connect; no references |
| `http.url` | string | required for http | `http://` or `https://`; references allowed outside the host |
| `http.headers` | {string: string} | `{}` | values may hold references |
| `enabled` | bool | `true` | `false`: no global, no catalog entries, never connects |
| `timeout_ms` | integer | `60000` | per request, `100..=600000` |
| `allow` | string[] | `["*"]` | tool-name globs (`*`, `?`), matched on the upstream's raw tool name |
| `deny` | string[] | `[]` | globs; deny wins over allow |

Exactly one of `stdio` / `http` is present and it matches `transport`.
Unknown keys are errors (`deny_unknown_fields`). At most 32 upstreams.

### References (secrets)

A value in `stdio.env` or `http.headers` may contain any mix of literal text
and references:

- `${env:NAME}`: the service process's environment variable `NAME`
  (`[A-Za-z_][A-Za-z0-9_]*`). Note: under systemd this is the unit's
  environment, not the caller's shell.
- `${file:/abs/path}` or `${file:~/path}` (on Windows also `C:\path` or
  `~\path`): the file's UTF-8 content with trailing `\r`/`\n` trimmed.
- `$${` is a literal `${`. Any other `${` is a config error.

Examples: `"Bearer ${env:LINEAR_TOKEN}"`, `"${file:~/.config/t3-codemode/gh.pat}"`.

Secrets only by reference: a header named `Authorization`,
`Proxy-Authorization` or `Cookie`, or any header or env name containing one of
`SECRET_NAME_HINTS` (`TOKEN`, `SECRET`, `KEY`, `PASSWORD`, `PASSWD`, `AUTH`,
`COOKIE`, `CREDENTIAL`, case-insensitive), whose value holds no reference is a
config error. `${` in `command`, `args`, `cwd` or `url` is a config error.

References are parsed at load (syntax errors fail the load) and **resolved at
connect time**, every connect, so a rotated file or env value is picked up on
the next reconnect. An unset or empty env var, or an unreadable file, fails
that connect with `UpstreamError::Config` naming the variable or path, never
the value; the upstream goes to `backoff`, others are unaffected. Resolved
values are held in `Secret` (its `Debug`/`Display` print `[redacted]`) and
listed in `ResolvedUpstream::secrets`: every reference's resolved value and
every composed value that contains one.

### Example

```json
{
  "upstreams": [
    {
      "name": "cbm",
      "transport": "stdio",
      "stdio": { "command": "codebase-memory-mcp" },
      "timeout_ms": 120000,
      "deny": ["delete_project", "index_repository"]
    },
    {
      "name": "linear",
      "transport": "http",
      "http": {
        "url": "https://mcp.linear.app/mcp",
        "headers": { "Authorization": "Bearer ${file:~/.config/t3-codemode/linear.token}" }
      },
      "allow": ["list_*", "get_*", "search_*"]
    },
    {
      "name": "gh",
      "transport": "stdio",
      "enabled": false,
      "stdio": {
        "command": "github-mcp-server",
        "args": ["stdio"],
        "env": { "GITHUB_PERSONAL_ACCESS_TOKEN": "${env:GH_PAT}" },
        "cwd": "~"
      }
    }
  ]
}
```

`codebase-memory-mcp` (installed at `~/.local/bin/codebase-memory-mcp`)
runs an MCP server on stdio with no arguments; the bare command resolves
through the `$HOME/.local/bin` fallback even when the unit's `PATH` lacks it.

## 3. Rust layout and signatures

```
src/upstream/mod.rs          shared types, Upstream + Connector traits, McpConnector, T3Upstream, helpers  (architect; frozen)
src/upstream/fake.rs         FakeUpstream + FakeConnector for tests                                       (architect; frozen)
src/upstream/config.rs       validate, references, resolve, globs, ToolFilter impl                         (manager)
src/upstream/manager.rs      UpstreamManager                                                              (manager)
src/upstream/client_stdio.rs StdioClient                                                                  (client)
src/upstream/client_http.rs  HttpClient                                                                   (client)
tests/support/fake_mcp_server.rs  [[bin]] t3cm-fake-mcp, a scriptable stdio MCP server                    (client)
```

`src/config.rs` already has `#[serde(default)] pub upstreams: Vec<UpstreamConfig>`
and calls `crate::upstream::config::validate(&self.upstreams)` from
`Config::validate`. `App` already has `pub upstreams: Arc<UpstreamManager>`;
`App::new` builds it from `config.upstreams` with `McpConnector`, and
`App::with_upstreams(config, store, t3, clock, upstreams)` injects one (tests).
`main.rs` warms it (`upstreams.tools()` in the background at start) and calls
`upstreams.shutdown()` after the server stops.

### Shared (mod.rs, implemented)

```rust
pub const T3_NAMESPACE: &str = "t3";
pub const RESERVED_NAMES: &[&str] = &["t3", "mem", "vars", "wf", "console", "tools", "catalog"];
pub const RESERVED_JS: &[&str] = &["eval", "undefined", "arguments", "escape", "unescape", "async", "globalThis"];
pub const MAX_NAME_LEN: usize = 32;
pub const DEFAULT_TIMEOUT_MS: u64 = 60_000;  MIN_TIMEOUT_MS = 100;  MAX_TIMEOUT_MS = 600_000;
pub const CONNECT_TIMEOUT: Duration = 15 s;   // spawn/handshake + initialize + first tools/list
pub const BACKOFF_MIN: Duration = 1 s;  BACKOFF_MAX: Duration = 60 s;
pub const MCP_PROTOCOL_VERSION: &str = "2025-06-18";  CLIENT_NAME = "t3-codemode";
pub const STDERR_LINES: usize = 50;  ERROR_CHARS = 1000;  MIN_SECRET_LEN = 4;  REDACTED = "[redacted]";
pub const SECRET_NAME_HINTS: &[&str];

pub enum Transport { Stdio, Http }                         // serde "stdio" | "http"
pub struct StdioConfig { command, args: Vec<String>, env: BTreeMap<String, String>, cwd: Option<String> }
pub struct HttpConfig { url, headers: BTreeMap<String, String> }
pub struct UpstreamConfig { name, transport, stdio: Option<StdioConfig>, http: Option<HttpConfig>,
                            enabled: bool, timeout_ms: u64, allow: Vec<String>, deny: Vec<String> }
pub enum Reference { Literal(String), Env(String), File(PathBuf) }
pub struct Secret(..);  fn new(String) -> Secret;  fn expose(&self) -> &str;   // Debug/Display: [redacted]
pub enum ResolvedTransport {
    Stdio { command: PathBuf, args: Vec<String>, env: Vec<(String, Secret)>, cwd: Option<PathBuf> },
    Http { url: String, headers: Vec<(String, Secret)> },
}
pub struct ResolvedUpstream { name, transport: ResolvedTransport, timeout: Duration, secrets: Vec<Secret> }
    fn redact(&self, String) -> String;
pub struct ToolFilter { pub allow: Vec<String>, pub deny: Vec<String> }      // impl in config.rs
pub struct ToolDef { name, description, input_schema: Value, output_schema: Option<Value>, annotations: Option<Value> }
    fn from_mcp(&Value) -> Option<ToolDef>;  fn to_mcp(&self) -> Value;
pub enum UpstreamError { Tool, Unavailable, Timeout, Denied, UnknownTool, UnknownNamespace, Config }  // each (String)
    fn message(&self) -> &str;  fn restarts_connection(&self) -> bool /* Unavailable only */;  fn not_implemented(&str) -> Self;
    impl Display, Error, From<T3Error>
pub enum UpstreamState { Disabled, Idle, Connecting, Ready, Backoff }        // serde lowercase
pub struct UpstreamStatus { name, transport, enabled, state, tools: usize, restarts: u32,
                            last_error: Option<String>, retry_at: Option<String>, server: Option<Value> }  // serde camelCase

pub trait Upstream: Send + Sync {
    fn name(&self) -> &str;
    fn list_tools(&self) -> BoxFut<'_, Result<Vec<ToolDef>, UpstreamError>>;      // unfiltered, all pages
    fn call<'a>(&'a self, tool: &'a str, args: Value) -> BoxFut<'a, Result<Value, UpstreamError>>;  // unwrapped result
    fn status(&self) -> UpstreamStatus;                                            // connection view; manager overlays
    fn tools_changed(&self) -> bool { false }                                      // true once after list_changed
    fn redact(&self, text: String) -> String { text }
    fn shutdown(&self) -> BoxFut<'_, ()> { no-op }
}
pub trait Connector: Send + Sync {
    fn connect<'a>(&'a self, spec: &'a ResolvedUpstream) -> BoxFut<'a, Result<Arc<dyn Upstream>, UpstreamError>>;
}
pub struct McpConnector;   // Stdio -> StdioClient::connect, Http -> HttpClient::connect
pub struct T3Upstream;     // T3Upstream::new(Arc<dyn T3Client>)

pub fn qualified(ns, tool) -> String;                          // "ns.tool"
pub fn split_qualified(&str) -> Option<(&str, &str)>;          // splits at the FIRST '.'
pub fn redact_secrets(String, &[Secret]) -> String;            // secrets shorter than MIN_SECRET_LEN are skipped
pub fn unwrap_call_result(qualified, &Value) -> Result<Value, UpstreamError>;
```

`unwrap_call_result`: `isError` gives `Tool("ns.tool: <text, cut at 1000>")`;
else `structuredContent` when present; else, when every content part is text,
the joined text parsed as JSON (or the string); else the `content` array as is.

`BoxFut` is `crate::t3::BoxFut` (`Pin<Box<dyn Future + Send>>`). Every future a
builder returns, including those of `async fn`s on `UpstreamManager`, must be
`Send`: `App` awaits them inside `'static + Send` host futures.

### client (client_stdio.rs, client_http.rs)

```rust
impl StdioClient { pub async fn connect(spec: &ResolvedUpstream) -> Result<StdioClient, UpstreamError>; pub fn pid(&self) -> Option<u32>; }
impl HttpClient  { pub async fn connect(spec: &ResolvedUpstream) -> Result<HttpClient, UpstreamError>;  pub fn session_id(&self) -> Option<String>; }
impl Upstream for StdioClient / HttpClient
```

`connect` finishes the handshake before returning: `initialize` with
`{protocolVersion: MCP_PROTOCOL_VERSION, capabilities: {}, clientInfo: {name: CLIENT_NAME, version: CARGO_PKG_VERSION}}`,
a server `protocolVersion` in `crate::mcp::SUPPORTED_VERSIONS` (else
`Unavailable`), then `notifications/initialized`. `status().server` is the
server's `serverInfo`. `list_tools` follows `nextCursor` (at most 50 pages).
`call` sends `tools/call {name, arguments}` and returns
`unwrap_call_result(qualified(ns, tool), result)`. A JSON-RPC error answer to
`tools/call` is `Tool("ns.tool: <message>")`; to `initialize` or `tools/list`,
`Unavailable`. Requests are multiplexed by id; calls run concurrently.
Server-to-client requests: `ping` answers `{}`, anything else answers
`-32601`. Notifications: `notifications/tools/list_changed` sets the
`tools_changed` flag; others are ignored. Each request is bounded by
`spec.timeout` (`Timeout("ns.tool: timed out after N ms")`); the connection is
kept (stdio sends `notifications/cancelled`). Every error string passes
through `spec.redact` before it leaves the client.

Stdio specifics: spawn with `kill_on_drop`, stdin/stdout piped, stderr drained
continuously into a ring of the last `STDERR_LINES` lines (redacted; the last
lines are appended to a connect failure and logged at `debug`, never at a
higher level). The child gets only an allowlist of the service environment
(`client_stdio::INHERITED_ENV` and `LC_*`; on Windows `INHERITED_ENV` and
`INHERITED_ENV_WINDOWS`, matched in any case), plus the resolved `env`, and
runs in its own process group, or its own Job Object on Windows. Non-JSON stdout lines are ignored.
A dead child (EOF, write error, exit) fails every pending and later request
with `Unavailable("ns: process exited (<status>)")` at once, no hang.
`shutdown`: close stdin, wait up to 2 s, then kill.

HTTP specifics (Streamable HTTP only; the legacy HTTP+SSE transport is not
supported): POST to `url` with the resolved headers, `Content-Type:
application/json`, `Accept: application/json, text/event-stream`,
`MCP-Protocol-Version` after initialize, and `Mcp-Session-Id` once issued.
Responses are JSON or SSE (read events until the response with the request's
id). `202` for notifications. `404` while holding a session id: re-initialize
once and resend once. `401`/`403`: `Unavailable("ns: HTTP 401 unauthorized")`
with no header values. Other non-2xx: `Unavailable` with the status and the
first 300 chars of the body, redacted. Connect timeout 10 s. No proxy for
loopback hosts. `shutdown` sends `DELETE` with the session id, best effort.

### manager (manager.rs, config.rs)

```rust
// config.rs
pub fn validate(upstreams: &[UpstreamConfig]) -> anyhow::Result<()>;
pub fn validate_name(name: &str) -> anyhow::Result<()>;
pub fn parse_references(raw: &str) -> anyhow::Result<Vec<Reference>>;
pub fn has_reference(raw: &str) -> bool;
pub fn resolve_value(raw: &str) -> Result<Secret, UpstreamError>;                 // Config on failure
pub fn resolve(cfg: &UpstreamConfig) -> Result<ResolvedUpstream, UpstreamError>;  // Config on failure
pub fn glob_match(pattern: &str, name: &str) -> bool;
impl ToolFilter { pub fn new(allow: &[String], deny: &[String]) -> ToolFilter; pub fn allows(&self, tool: &str) -> bool; }

// manager.rs
impl UpstreamManager {
    pub fn new(configs: Vec<UpstreamConfig>) -> UpstreamManager;                   // McpConnector; no I/O, no spawn
    pub fn with_connector(configs: Vec<UpstreamConfig>, connector: Arc<dyn Connector>) -> UpstreamManager;
    pub fn empty() -> UpstreamManager;
    pub fn namespaces(&self) -> Vec<String>;                                       // enabled, config order
    pub fn is_namespace(&self, ns: &str) -> bool;
    pub async fn tools(&self) -> Vec<(String, Vec<ToolDef>)>;                      // lazy connect, filtered
    pub fn cached_tools(&self) -> Vec<(String, Vec<ToolDef>)>;                     // no I/O, filtered
    pub fn generation(&self) -> u64;
    pub async fn refresh(&self) -> Vec<(String, Result<usize, String>)>;
    pub async fn call(&self, ns: &str, tool: &str, args: Value) -> Result<Value, UpstreamError>;
    pub fn status(&self) -> Vec<UpstreamStatus>;                                   // every configured upstream, config order
    pub fn redact(&self, text: String) -> String;
    pub async fn shutdown(&self);
}
```

`validate` errors name the upstream (`upstreams[2] "linear": ...`) and the
field. `resolve` expands `~`, resolves the command (`/`, or `\` on Windows, in
it: a path; else `PATH`, then `$HOME/.local/bin`; on Windows each `PATHEXT`
extension is tried too; not found is `Config` naming the command),
resolves references, checks `cwd` exists, and fills `secrets`.

Lifecycle per upstream (`UpstreamState`):

- `Disabled` (`enabled: false`): not a namespace, never connects; `call`
  gives `UnknownNamespace`. It still appears in `status()`.
- `Idle` until first use. Nothing connects in `new`. `tools()`, `call` and
  `refresh` connect lazily: `resolve` then `Connector::connect` then
  `list_tools`, all under `CONNECT_TIMEOUT`. Connects are single-flight:
  concurrent callers share one attempt. `tools()` connects all idle or
  due upstreams concurrently.
- `Ready` after a successful connect and list.
- `Backoff` after any `Unavailable` (from connect, list or call), a
  `Config` from `resolve`, or a connect timeout: the connection is dropped
  (`shutdown()`), `restarts += 1`, `last_error` set (redacted), and
  `retry_at` = now + delay, where the delay is `0` for the first consecutive
  failure, then `BACKOFF_MIN` doubling up to `BACKOFF_MAX`. Before
  `retry_at`, `call` fails fast with
  `Unavailable("ns is unavailable (retry at <ISO time>): <last error>")` and
  `tools()` does not reconnect it. After `retry_at` the next use reconnects.
  A successful connect resets the failure count.
- A failed call is never retried automatically (tools need not be
  idempotent). `Tool`, `Timeout`, `Denied` and `UnknownTool` do not touch
  the connection.
- `call` wraps the upstream call in `tokio::time::timeout(timeout_ms)` as a
  backstop (`Timeout("ns.tool: timed out after N ms")`).
- After each call and in `tools()`/`refresh()`, `tools_changed()` true means
  re-list. `generation()` increases whenever any upstream's filtered list
  changes, including its first listing. `cached_tools()` keeps the last list
  through `Backoff`, so `search` still shows the tools.
- Duplicate tool names in one upstream's list: the first wins.
- `call` order of checks: unknown namespace (`UnknownNamespace("unknown
  namespace \"x\"; namespaces are t3, cbm")`); filter (`Denied("ns.tool is
  not allowed by the t3-codemode config for upstream ns; do not retry
  it")`, without contacting the upstream); backoff; connect if needed;
  unknown tool against the known list (`UnknownTool("unknown tool ns.tool;
  use the search tool to find the right name")`, without calling); then the
  call.
- `redact` applies every secret resolved so far, for every upstream, plus
  each live connection's `redact`. `status().last_error` is always redacted.
- No lock is held across an await on an upstream call.
- `shutdown` calls `shutdown()` on every live connection, concurrently.

### catalog (catalog.rs, app.rs, mcp.rs, server.rs /healthz, sandbox.rs and types.rs only if needed, README.md)

```rust
pub struct Entry { name, kind: &'static str, server: String, description, signature, input_schema }   // server added
pub struct Catalog { entries, t3_names, upstream_names: BTreeMap<String, BTreeSet<String>>, json }       // upstream_names added
impl Catalog {
    pub fn build(t3_tools: &[Value]) -> Catalog;                                     // unchanged behavior = build_with(t3, &[])
    pub fn build_with(t3_tools: &[Value], upstreams: &[(String, Vec<ToolDef>)]) -> Catalog;
    pub fn declarations(&self) -> String;
}
impl App {
    pub fn new(config, store, t3, clock) -> Arc<App>;                                // unchanged signature
    pub fn with_upstreams(config, store, t3, clock, upstreams: Arc<UpstreamManager>) -> Arc<App>;
}
```

- Entries: T3 tools first (name unchanged, `kind: "t3"`, `server: "t3"`), then
  upstream tools grouped by namespace in config order and tool order as
  listed (`name: "ns.tool"`, `kind: "mcp"`, `server: ns`), then the built-ins
  (`server` = their kind: `mem` or `wf`). `signature` uses `ns.<tool>` when the
  raw tool name is an identifier, else `ns["raw-name"]`.
- `declarations()`: the existing `t3` block unchanged, then one
  `declare const <ns>: { ...signatures...; call(name: string, input?: object): Promise<unknown>; };`
  per upstream, then `declare const tools: { call(name: string, input?: object): Promise<unknown>; };`,
  then the built-ins.
- `execute` globals: `t3`, one frozen, non-writable, non-configurable global
  per enabled upstream namespace (from `upstreams.namespaces()`, present even
  before its tool list is known), `tools`, `mem`, `vars`, `wf`, `console`.
  An upstream global has each tool by exact name, a sanitized alias
  (`types::sanitize_identifier`) when it differs and collides with nothing,
  and `call(name, input)`. `tools.call("ns.tool", input)` reaches any
  namespace including `t3` (which keeps the `t3_` alias and `config.deny`).
  The ReferenceError hint lists the real globals in that order.
- Host side: one op for upstream calls; `t3` goes to the existing path, any
  other namespace to `upstreams.call`. Input `undefined`/`null` becomes `{}`;
  a non-object is `ns.tool(input): input must be an object`. Every error and
  the final output pass through `t3.redact` and `upstreams.redact`.
- Catalog freshness: `catalog_for_run` calls `upstreams.tools()` (lazy
  connect; returns at once for upstreams in backoff) and rebuilds when
  `upstreams.generation()` differs from the generation the current catalog was
  built from. `refresh_catalog` keeps its T3 return value and also calls
  `upstreams.refresh()` (the watcher already calls it every 10 minutes).
- `mcp.rs` descriptions: identical bytes for every caller and every request
  for a given config. They describe upstreams generically (`kind: "mcp"`,
  `server`, `<namespace>.<tool>(input)`, `tools.call("ns.tool", input)`) and
  may list the enabled namespace names from config; never connection state,
  tool counts or anything per caller. With no upstreams they still read
  correctly.
- `/healthz` adds `"upstreams": upstreams.status()`; every existing key stays.
- README: document the `upstreams` config, references, globals and
  `/healthz`.

## 4. Namespacing and identifier rules

- `name` matches `^[a-z][a-z0-9_]{0,31}$`.
- Not in `RESERVED_NAMES` (`t3`, `mem`, `vars`, `wf`, `console`, `tools`,
  `catalog`), not in `RESERVED_JS`, and not a JS reserved word
  (`types::sanitize_identifier(name) == name` must hold).
- Unique across `upstreams` (disabled ones included).
- Catalog names: T3 tools keep their bare names (`t3_thread_send`); upstream
  tools are `ns.tool`; built-ins are `mem.*`, `wf.*`, `vars`. Reserved
  namespaces make these disjoint.

## 5. Collision rules

| Case | Result |
|---|---|
| upstream named like a reserved name, keyword or another upstream | config error, service does not start |
| two tools in one upstream with the same name | first wins |
| raw tool name not an identifier (`get-thing`) | `ns["get-thing"]`, alias `ns.get_thing` if free |
| two tools sanitize to the same alias | no alias for either; both by exact name |
| alias equals another tool's exact name | the exact name wins; no alias |
| a tool named `call` | `ns.call` is the method; reach the tool with `ns.call("call", input)` or `tools.call("ns.call", input)` |
| a tool name with dots (`a.b`) | `ns["a.b"]`; `tools.call("ns.a.b")` splits at the first dot |
| denied tool (allow/deny) | absent from catalog and global; calls give `Denied` |

## 6. Error semantics

| Variant | Raised by | Message shape | Connection | In the sandbox |
|---|---|---|---|---|
| `Tool` | client (`isError`, JSON-RPC error on call) | `ns.tool: <text>` | kept | throws `Error(message)` |
| `Timeout` | client, manager backstop | `ns.tool: timed out after N ms` | kept | throws |
| `Unavailable` | client (spawn, I/O, exit, HTTP status), manager (backoff) | `ns: <reason>` / `ns is unavailable (retry at T): <last>` | dropped, backoff | throws |
| `Config` | `resolve` (unset env, unreadable file, missing command or cwd) | `ns: env var NAME is not set` | backoff | throws |
| `Denied` | manager filter | `... do not retry it` | untouched | throws |
| `UnknownTool` | manager | `unknown tool ns.tool; use the search tool ...` | untouched | throws |
| `UnknownNamespace` | manager, `tools.call` | `unknown namespace "x"; namespaces are ...` | n/a | throws |

Static config errors (`validate`) fail `Config::load`, as today. No message
ever contains a resolved secret or the code being run.

## 7. Ownership

| Builder | Owns (may edit) |
|---|---|
| client | `src/upstream/client_stdio.rs`, `src/upstream/client_http.rs`, `tests/upstream_client.rs`, `tests/support/fake_mcp_server.rs` (the `t3cm-fake-mcp` bin, declared), any new file under `tests/support/` |
| manager | `src/upstream/manager.rs`, `src/upstream/config.rs`, `tests/upstream_manager.rs` |
| catalog | `src/catalog.rs`, `src/app.rs`, `src/mcp.rs`, `src/server.rs` (`/healthz` only), `src/sandbox.rs` and `src/types.rs` only if needed, `README.md`, `tests/upstream_catalog.rs` |

Frozen for all three: `Cargo.toml`, `Cargo.lock`, `src/lib.rs`,
`src/config.rs`, `src/main.rs`, `src/upstream/mod.rs`,
`src/upstream/fake.rs`, `tests/common/`, every existing test file, and this
contract. A builder that needs a change there stops and asks. Public
signatures listed above do not change; private items are free. Every builder
keeps `CARGO_BUILD_JOBS=8 cargo test`, `cargo clippy --all-targets` (no
warnings) and `cargo fmt --check` clean; no code comments; no em dashes.

Test fixtures available to everyone: `upstream::fake::{FakeUpstream,
FakeConnector, tool}`. `FakeConnector::connect` records the
`ResolvedUpstream` (`last_spec`), counts connects, can `refuse(name,
Some(msg))`, and brings a `set_down(true)` fake back up on reconnect.
`FakeUpstream` answers its listed tools with `{server, tool, args}`, takes
canned `reply(tool, result)`, `set_delay`, `set_tools` (raises
`tools_changed`), and records `calls()` and `shutdowns()`.

Dependencies available: `reqwest` (`json`, `stream`, `rustls-tls`),
`futures-util`, `tokio` (`process`, `io-util`, `time`, `sync`, `net`, ...),
`serde_json` (`preserve_order`), `axum` (dev: `form`), `tempfile` (dev).

## 8. Acceptance tests

### client: `tests/upstream_client.rs`

The fake server is `env!("CARGO_BIN_EXE_t3cm-fake-mcp")`; its tools and
behaviors are the client builder's to design (suggested: `echo`, `structured`,
`image`, `fail`, `rpc_error`, `sleep{ms}`, `crash`, `env{name}`,
`fail_with_env{name}`, `notify_changed`, `ping_first`, two pages of
`tools/list`). HTTP tests use an in-process axum server.

1. Stdio connect completes the handshake; `status().server` is the fake's
   `serverInfo`; `list_tools` returns both pages.
2. `call` returns `structuredContent`, parsed text JSON, plain text, and the
   raw content array for non-text content.
3. `isError` and a JSON-RPC error both give `Tool` starting `ns.tool:`.
4. Ten concurrent calls with different `sleep` values all resolve to their
   own results.
5. A server ping sent mid-call is answered and the call completes.
6. `list_changed` makes `tools_changed()` true exactly once.
7. A call past `timeout` gives `Timeout`; the next call on the same client
   succeeds.
8. `crash` gives `Unavailable`; later calls fail at once with `Unavailable`.
9. Configured env reaches the child; a secret echoed into an error or into
   stderr shows as `[redacted]` in the error and in `status().last_error`.
10. A missing command gives `Unavailable` naming it.
11. `shutdown` ends the child process within 3 s.
12. HTTP: JSON and SSE responses; `Mcp-Session-Id` and
    `MCP-Protocol-Version` sent after initialize; configured headers sent;
    `404` re-initializes once and succeeds; `401` gives `Unavailable` naming
    401 and not the header value; `isError` gives `Tool`.
13. `McpConnector` dispatches by transport.

### manager: `tests/upstream_manager.rs`

1. The section 2 example parses through `Config::from_value` and validates;
   defaults apply (`enabled`, `timeout_ms`, `allow`).
2. Each rejection: every reserved name, a keyword (`class`), uppercase, a
   bad char, 33 chars, a duplicate, transport/section mismatch, both
   sections, a bad URL scheme, `${` in command/args/url/cwd, malformed
   references (`${env:}`, `${foo:x}`, unterminated), a literal
   `Authorization` header, a literal `FOO_TOKEN` env, `timeout_ms` out of
   range, an empty glob. Each error names the upstream and field.
3. `resolve`: `${env:..}` (test-unique variable), `${file:..}` trimmed,
   `"Bearer ${env:..}"`, `$${` escape; unset variable gives `Config` naming
   the variable, not a value; `secrets` holds the resolved values; the
   `Debug` of a `ResolvedUpstream` contains no secret.
4. `glob_match` and `ToolFilter` (deny wins).
5. `namespaces()` is enabled-only in config order; a disabled upstream's
   `call` gives `UnknownNamespace`.
6. `new`/`with_connector` connect nothing (`connects == 0`) until first use.
7. `tools()` is filtered; a denied call gives `Denied` and the fake records
   no call.
8. An unknown tool gives `UnknownTool` without a call.
9. `set_down(true)`: the call gives `Unavailable`, status `backoff`,
   `restarts == 1`; the next call reconnects and succeeds. With `refuse`,
   the second consecutive failure sets `retry_at` at least 1 s ahead and calls
   before it fail fast without connecting; success resets the backoff.
10. Ten concurrent first calls connect once.
11. `set_tools` leads to a re-list and a higher `generation()`;
    `cached_tools()` keeps the last list during backoff.
12. `redact` hides a resolved secret in any text; a refused connect whose
    message carries the secret is redacted in the error and in
    `status().last_error`.
13. A fake delay past `timeout_ms` gives `Timeout`.
14. `shutdown()` reaches each live connection.
15. An unset env reference puts only that upstream in backoff with
    `last_error` naming the variable; the others work.

### catalog: `tests/upstream_catalog.rs`

Pure catalog tests run on their own; App-level tests use
`App::with_upstreams` with `UpstreamManager::with_connector` and a
`FakeConnector`, and pass once the manager lands.

1. `build_with(fixture, [("cbm", [search_graph, get-thing])])`: entries
   `cbm.search_graph` with `kind: "mcp"`, `server: "cbm"`; T3 entries have
   `server: "t3"`; built-ins have `server` = kind; `build(fixture)` gives
   the same T3 entries in the same order as before; the `get-thing`
   signature uses `cbm["get-thing"]`.
2. `declarations()` holds `declare const cbm: {` and `declare const tools: {`.
3. `search`: `catalog.filter(t => t.server === "cbm")` returns the fake's
   tools.
4. `execute`: `cbm.search_graph(...)`, `cbm["get-thing"]({})`,
   `cbm.get_thing({})`, `cbm.call("search_graph", {})`,
   `tools.call("cbm.search_graph", {})`, and `tools.call("t3.thread_list")`
   (reaches FakeT3) all work; `tools.call("t3.t3_project_delete")` is still
   denied.
5. Errors: a `Tool` error throws its message; a denied tool throws with "do
   not retry"; an unknown namespace via `tools.call` lists namespaces; a
   non-object input is rejected.
6. `typeof cbm === "object"`, and it cannot be reassigned or redefined; a
   disabled upstream is not a global; a ReferenceError lists the upstream
   namespaces and `tools`.
7. A result or error containing a resolved secret comes back `[redacted]`.
8. `tools/list` is byte-identical with and without `X-T3-Thread` and across
   two requests; it mentions upstreams; with no upstreams it is still valid.
9. `/healthz` has `upstreams` with name, state and tools, and every old key.
10. After the fake's `set_tools`, the next `search` sees the new tool.
11. Every pre-existing test still passes unchanged.
