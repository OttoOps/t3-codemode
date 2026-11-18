mod common;

use std::sync::Arc;

use common::*;
use serde_json::{Value, json};
use t3_codemode::app::{App, ExecuteRequest, ToolOutput};
use t3_codemode::catalog::{Catalog, aliases};
use t3_codemode::clock::FakeClock;
use t3_codemode::config::Config;
use t3_codemode::fake::{FakeT3, thread};
use t3_codemode::server;
use t3_codemode::store::Store;
use t3_codemode::upstream::fake::{FakeConnector, FakeUpstream, tool};
use t3_codemode::upstream::{ToolDef, UpstreamError, UpstreamManager};

const SECRET: &str = "sk-live-7f3a9c2e51";
const MULTI: &str = "line-one-secret\nline-two-secret";
const QUOTED: &str = "quo\"te\\secret-99";
const QUOTE_VAR: &str = "UPCAT_QUOTE_SECRET";

fn search_graph() -> ToolDef {
    tool(
        "search_graph",
        "Search the code graph.",
        json!({"type": "object", "properties": {"query": {"type": "string"}}, "required": ["query"]}),
    )
}

fn get_thing() -> ToolDef {
    tool("get-thing", "Get one thing.", json!({"type": "object", "properties": {"id": {"type": "string"}}}))
}

fn cbm_tools() -> Vec<ToolDef> {
    let open = json!({"type": "object"});
    vec![
        search_graph(),
        get_thing(),
        tool("fail", "Always fails.", open.clone()),
        tool("leak", &format!("Returns a secret like {SECRET}."), open.clone()),
        tool("leak_error", "Fails with a secret.", open.clone()),
        tool("call", "A tool named call.", open.clone()),
        tool("a.b", "A dotted name.", open.clone()),
        tool("drop_all", "Denied by config.", open),
    ]
}

fn kinds(cat: &Catalog) -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    for e in &cat.entries {
        if out.last() != Some(&e.kind) {
            out.push(e.kind);
        }
    }
    out
}

fn as_json(cat: &Catalog, kind: &str) -> Vec<Value> {
    cat.entries.iter().filter(|e| e.kind == kind).map(|e| serde_json::to_value(e).unwrap()).collect()
}

#[test]
fn build_with_adds_namespaced_upstream_entries() {
    let fixture = fixture_tools();
    let cat = Catalog::build_with(&fixture, &[("cbm".into(), vec![search_graph(), get_thing(), search_graph()])]);
    assert_eq!(kinds(&cat), ["t3", "mcp", "mem", "wf"]);

    let sg = cat.entries.iter().find(|e| e.name == "cbm.search_graph").unwrap();
    assert_eq!((sg.kind, sg.server.as_str()), ("mcp", "cbm"));
    assert_eq!(sg.description, "Search the code graph.");
    assert!(sg.signature.contains("cbm.search_graph(input: {"), "{}", sg.signature);
    assert!(sg.signature.contains("query: string;"), "{}", sg.signature);
    let gt = cat.entries.iter().find(|e| e.name == "cbm.get-thing").unwrap();
    assert!(gt.signature.contains(r#"cbm["get-thing"](input?: {"#), "{}", gt.signature);
    assert_eq!(cat.entries.iter().filter(|e| e.kind == "mcp").count(), 2, "duplicate names: first wins");
    assert_eq!(cat.upstream_names["cbm"].iter().map(String::as_str).collect::<Vec<_>>(), ["get-thing", "search_graph"]);

    for e in &cat.entries {
        match e.kind {
            "t3" => assert_eq!(e.server, "t3"),
            "mcp" => assert_eq!(e.server, "cbm"),
            other => assert_eq!(e.server, other),
        }
    }
    let plain = Catalog::build(&fixture);
    assert_eq!(as_json(&cat, "t3"), as_json(&plain, "t3"));
    assert_eq!(plain.t3_names, cat.t3_names);
    assert_eq!(plain.entries.len() + 2, cat.entries.len());
    assert_eq!(plain.json, Catalog::build_with(&fixture, &[]).json);
    assert!(plain.upstream_names.is_empty());
    let entry: Value = serde_json::from_str(&cat.json).unwrap();
    assert_eq!(entry[74]["server"], "cbm");
    assert_eq!(entry[0]["server"], "t3");
}

#[test]
fn upstreams_keep_config_order_and_skip_reserved_t3() {
    let cat = Catalog::build_with(
        &[],
        &[
            ("zeta".into(), vec![tool("one", "", json!({}))]),
            ("t3".into(), vec![tool("sneaky", "", json!({}))]),
            ("alpha".into(), vec![tool("two", "", json!({}))]),
            ("empty".into(), vec![]),
        ],
    );
    let names: Vec<&str> = cat.entries.iter().filter(|e| e.kind == "mcp").map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["zeta.one", "alpha.two"]);
    assert!(cat.upstream_names["empty"].is_empty());
    let d = cat.declarations();
    let pos = |s: &str| d.find(s).unwrap_or_else(|| panic!("{s} missing in:\n{d}"));
    assert!(pos("declare const zeta: {") < pos("declare const alpha: {"));
    assert!(pos("declare const alpha: {") < pos("declare const empty: {"));
    assert!(!d.contains("sneaky"));
}

#[test]
fn declarations_cover_upstreams_and_tools() {
    let fixture = fixture_tools();
    let plain = Catalog::build(&fixture).declarations();
    let cat = Catalog::build_with(&fixture, &[("cbm".into(), vec![search_graph(), get_thing()])]);
    let d = cat.declarations();
    let t3_block = &plain[..plain.find("};\n").unwrap() + 3];
    assert!(d.starts_with(t3_block));
    let pos = |s: &str| d.find(s).unwrap_or_else(|| panic!("{s} missing in:\n{d}"));
    assert!(pos("declare const cbm: {") < pos("declare const tools: {"));
    assert!(pos("declare const tools: {") < pos("mem.get(key: string)"));
    let cbm = &d[pos("declare const cbm: {")..pos("declare const tools: {")];
    assert!(cbm.contains("\n  search_graph(input: {"), "{cbm}");
    assert!(cbm.contains("\n  \"get-thing\"(input?: {"), "{cbm}");
    assert!(cbm.contains("  /** Search the code graph. */\n"), "{cbm}");
    assert!(cbm.ends_with("  call(name: string, input?: object): Promise<unknown>;\n};\n"), "{cbm}");
    assert!(!cbm.contains("cbm."), "{cbm}");
    assert!(d.contains("declare const tools: {\n  call(name: string, input?: object): Promise<unknown>;\n};\n"));
}

#[test]
fn aliases_follow_the_collision_rules() {
    let got = aliases(&["get-thing", "search_graph", "a-b", "a.b", "x-y", "x_y", "delete", "call", "c-all"]);
    let alias = |name: &str| got.iter().find(|(n, _)| n == name).unwrap().1.clone();
    assert_eq!(alias("get-thing").as_deref(), Some("get_thing"));
    assert_eq!(alias("search_graph"), None);
    assert_eq!(alias("a-b"), None, "two tools sanitize to a_b");
    assert_eq!(alias("a.b"), None);
    assert_eq!(alias("x-y"), None, "x_y is another tool's exact name");
    assert_eq!(alias("delete").as_deref(), Some("delete_"));
    assert_eq!(alias("call"), None);
    assert_eq!(alias("c-all").as_deref(), Some("c_all"));
    assert_eq!(got.len(), 9);
}

#[tokio::test]
async fn tools_list_without_upstreams_still_reads_correctly() {
    let h = H::new();
    let first = list_tools(&h.app, None).await;
    assert_eq!(first, list_tools(&h.app, Some(WORKER)).await);
    let v: Value = serde_json::from_str(&first).unwrap();
    let tools = v["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 2);
    for t in tools {
        let d = t["description"].as_str().unwrap();
        assert!(d.contains("No MCP servers are configured."), "{d}");
        assert!(d.len() < 1600, "{} chars", d.len());
    }
}

struct U {
    _dir: tempfile::TempDir,
    t3: Arc<FakeT3>,
    cbm: Arc<FakeUpstream>,
    conn: Arc<FakeConnector>,
    app: Arc<App>,
}

const FAKE_BIN: &str = env!("CARGO_BIN_EXE_t3cm-fake-mcp");

fn config(secret_path: &std::path::Path, multi_path: &std::path::Path) -> Config {
    Config::from_value(json!({"upstreams": [
        {
            "name": "cbm",
            "transport": "stdio",
            "stdio": {"command": FAKE_BIN, "env": {
                "CBM_TOKEN": format!("${{file:{}}}", secret_path.display()),
                "CBM_MULTI": format!("${{file:{}}}", multi_path.display()),
                "CBM_QUOTE": format!("${{env:{QUOTE_VAR}}}"),
            }},
            "deny": ["drop_*"]
        },
        {"name": "gh", "transport": "stdio", "enabled": false, "stdio": {"command": FAKE_BIN}}
    ]}))
    .unwrap()
}

async fn setup() -> U {
    let dir = tempfile::tempdir().unwrap();
    let secret_path = dir.path().join("cbm.token");
    std::fs::write(&secret_path, format!("{SECRET}\n")).unwrap();
    let multi_path = dir.path().join("cbm.multi");
    std::fs::write(&multi_path, format!("{MULTI}\n")).unwrap();
    unsafe { std::env::set_var(QUOTE_VAR, QUOTED) };
    let config = config(&secret_path, &multi_path);
    let conn = FakeConnector::new();
    let cbm = FakeUpstream::new("cbm", cbm_tools());
    cbm.reply("fail", Err(UpstreamError::Tool("cbm.fail: the graph is on fire".into())));
    cbm.reply(
        "leak",
        Ok(json!({"token": SECRET, "note": format!("Bearer {SECRET}"), "multi": MULTI, "quote": QUOTED, SECRET: 1})),
    );
    cbm.reply("leak_error", Err(UpstreamError::Tool(format!("cbm.leak_error: token {SECRET} was rejected"))));
    conn.add(cbm.clone());
    let manager = Arc::new(UpstreamManager::with_connector(config.upstreams.clone(), conn.clone()));
    assert_eq!(manager.namespaces(), ["cbm"]);
    let t3 = Arc::new(FakeT3::with_tools(fixture_tools()));
    t3.put(thread(WORKER, "idle", T0));
    let store = Arc::new(Store::open(dir.path()).unwrap());
    let app = App::with_upstreams(config, store, t3.clone(), Arc::new(FakeClock::at(T0)), manager);
    app.refresh_catalog().await.unwrap();
    U { _dir: dir, t3, cbm, conn, app }
}

impl U {
    async fn exec(&self, code: &str) -> ToolOutput {
        self.app.execute(ExecuteRequest { code: code.into(), session: None, timeout_ms: None, caller: None }).await
    }

    async fn ok(&self, code: &str) -> Value {
        result(&self.exec(code).await)
    }

    async fn err(&self, code: &str) -> String {
        err_text(&self.exec(code).await).to_string()
    }
}

async fn list_tools(app: &Arc<App>, thread: Option<&str>) -> String {
    let (listener, addr) = server::bind(0).await.unwrap();
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let app = app.clone();
    let task = tokio::spawn(async move {
        server::serve(app, listener, async move {
            let _ = rx.await;
        })
        .await
        .unwrap();
    });
    let http = reqwest::Client::builder().no_proxy().build().unwrap();
    let mut bodies = Vec::new();
    for _ in 0..2 {
        let mut req =
            http.post(format!("http://{addr}/mcp")).json(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}));
        if let Some(t) = thread {
            req = req.header("X-T3-Thread", t);
        }
        bodies.push(req.send().await.unwrap().text().await.unwrap());
    }
    let _ = tx.send(());
    let _ = task.await;
    assert_eq!(bodies[0], bodies[1]);
    bodies.remove(0)
}

async fn healthz(app: &Arc<App>) -> Value {
    let (listener, addr) = server::bind(0).await.unwrap();
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let app = app.clone();
    let task = tokio::spawn(async move {
        server::serve(app, listener, async move {
            let _ = rx.await;
        })
        .await
        .unwrap();
    });
    let http = reqwest::Client::builder().no_proxy().build().unwrap();
    let v = http.get(format!("http://{addr}/healthz")).send().await.unwrap().json::<Value>().await.unwrap();
    let _ = tx.send(());
    let _ = task.await;
    v
}

#[tokio::test]
async fn search_sees_upstream_tools_by_server() {
    let u = setup().await;
    let out = u.app.search(r#"async () => catalog.filter(t => t.server === "cbm").map(t => t.name)"#).await;
    assert_eq!(
        result(&out),
        json!(["cbm.search_graph", "cbm.get-thing", "cbm.fail", "cbm.leak", "cbm.leak_error", "cbm.call", "cbm.a.b"])
    );
    let out = u.app.search(r#"async () => [...new Set(catalog.map(t => t.kind + ":" + t.server))]"#).await;
    assert_eq!(result(&out), json!(["t3:t3", "mcp:cbm", "mem:mem", "wf:wf"]));
    let out = u.app.search(r#"async () => catalog.find(t => t.name === "cbm.get-thing").signature"#).await;
    assert!(result(&out).as_str().unwrap().contains(r#"cbm["get-thing"](input?"#));
    assert!(!u.app.catalog().upstream_names.contains_key("gh"));
}

#[tokio::test]
async fn execute_reaches_upstreams_every_way() {
    let u = setup().await;
    let call = |tool: &str, args: Value| json!({"server": "cbm", "tool": tool, "args": args});
    assert_eq!(u.ok("async () => cbm.search_graph({ query: 'x' })").await, call("search_graph", json!({"query": "x"})));
    assert_eq!(u.ok(r#"async () => cbm["get-thing"]({ id: "7" })"#).await, call("get-thing", json!({"id": "7"})));
    assert_eq!(u.ok("async () => cbm.get_thing({})").await, call("get-thing", json!({})));
    assert_eq!(u.ok("async () => cbm.call('search_graph', {})").await, call("search_graph", json!({})));
    assert_eq!(u.ok("async () => cbm.call('call', { n: 1 })").await, call("call", json!({"n": 1})));
    assert_eq!(u.ok(r#"async () => cbm["a.b"]()"#).await, call("a.b", json!({})));
    assert_eq!(u.ok("async () => tools.call('cbm.a.b', null)").await, call("a.b", json!({})));
    assert_eq!(u.ok("async () => tools.call('cbm.search_graph', {})").await, call("search_graph", json!({})));
    assert_eq!(u.ok("async () => cbm.search_graph()").await, call("search_graph", json!({})));
    assert_eq!(u.ok("async () => typeof cbm.call").await, json!("function"));

    let threads = u.ok("async () => (await tools.call('t3.thread_list')).threads.map(t => t.threadId)").await;
    assert_eq!(threads, json!([WORKER]));
    let threads = u.ok("async () => (await tools.call('t3.t3_thread_list', {})).threads.length").await;
    assert_eq!(threads, json!(1));
    assert_eq!(u.t3.calls(), ["t3_thread_list", "t3_thread_list"]);
    for code in [
        "async () => tools.call('t3.t3_project_delete', { projectId: 'p' })",
        "async () => tools.call('t3.project_delete', {})",
        "async () => t3.project_delete({})",
    ] {
        assert!(u.err(code).await.contains("denied by the t3-codemode deny list; do not retry it"), "{code}");
    }
    assert!(!u.t3.calls().iter().any(|c| c.contains("delete")));
    let tools: Vec<String> = u.cbm.calls().into_iter().map(|(t, _)| t).collect();
    assert_eq!(tools.len(), 9);
}

#[tokio::test]
async fn upstream_errors_throw_with_their_messages() {
    let u = setup().await;
    assert_eq!(u.exec("async () => cbm.fail({})").await.text, "Error: cbm.fail: the graph is on fire");
    let caught =
        u.ok("async () => { try { await cbm.fail({}) } catch (e) { return e instanceof Error && e.message } }").await;
    assert_eq!(caught, json!("cbm.fail: the graph is on fire"));

    for code in ["async () => cbm.call('drop_all', {})", "async () => tools.call('cbm.drop_all', {})"] {
        assert!(u.err(code).await.contains("do not retry"), "{code}");
    }
    assert_eq!(u.ok("async () => 'drop_all' in cbm").await, json!(false));
    assert!(!u.cbm.calls().iter().any(|(t, _)| t == "drop_all"));

    let e = u.err("async () => tools.call('nope.thing', {})").await;
    assert!(e.contains(r#"unknown namespace "nope"; namespaces are t3, cbm"#), "{e}");
    let e = u.err("async () => tools.call('gh.list', {})").await;
    assert!(e.contains(r#"unknown namespace "gh""#), "{e}");
    let e = u.err("async () => tools.call('nodot', {})").await;
    assert!(e.contains("namespaces are t3, cbm"), "{e}");

    for code in [
        "async () => cbm.search_graph('x')",
        "async () => cbm.search_graph([1])",
        "async () => tools.call('cbm.search_graph', 5)",
    ] {
        let e = u.err(code).await;
        assert!(e.contains("cbm.search_graph(input): input must be an object"), "{code}: {e}");
    }
    assert!(
        u.err("async () => t3.t3_thread_list(3)").await.contains("t3.t3_thread_list(input): input must be an object")
    );
    let e = u.err("async () => cbm.no_such_tool({})").await;
    assert!(e.starts_with("Error: TypeError:") && e.contains("not a function"), "{e}");
}

#[tokio::test]
async fn upstream_globals_are_frozen_and_listed() {
    let u = setup().await;
    let r = u
        .ok(r#"async () => {
            const out = { type: typeof cbm, gh: typeof gh, tools: typeof tools, frozen: Object.isFrozen(cbm) && Object.isFrozen(tools) };
            try { cbm = 1; } catch (e) { out.assign = e.name; }
            try { Object.defineProperty(globalThis, "cbm", { value: 2 }); out.redefine = "redefined"; } catch (e) { out.redefine = e.name; }
            try { out.deleted = delete globalThis.cbm; } catch (e) { out.deleted = e.name; }
            try { cbm.search_graph = null; } catch (e) { out.member = e.name; }
            try { tools.call = null; } catch (e) { out.toolsMember = e.name; }
            const d = Object.getOwnPropertyDescriptor(globalThis, "cbm");
            out.descriptor = [d.writable, d.configurable];
            out.after = typeof cbm + ":" + typeof cbm.search_graph + ":" + typeof tools.call;
            return out;
        }"#)
        .await;
    assert_eq!(r["type"], "object");
    assert_eq!(r["gh"], "undefined");
    assert_eq!(r["tools"], "object");
    assert_eq!(r["frozen"], true);
    assert_eq!(r["redefine"], "TypeError");
    assert!(r["deleted"] == false || r["deleted"] == "TypeError", "{r}");
    assert_eq!(r["descriptor"], json!([false, false]));
    assert_eq!(r["after"], "object:function:function");

    let e = u.err("async () => nope").await;
    assert!(e.contains("Available globals: t3, cbm, tools, mem, vars, wf, console."), "{e}");
    let r = u.ok("async () => typeof __upstreams + typeof __names + typeof __host").await;
    assert_eq!(r, json!("undefinedundefinedundefined"));
}

#[tokio::test]
async fn resolved_secrets_never_come_back() {
    let u = setup().await;
    let spec = u.conn.last_spec("cbm");
    let spec = match spec {
        Some(s) => s,
        None => {
            u.ok("async () => cbm.search_graph({ query: 'warm' })").await;
            u.conn.last_spec("cbm").expect("connected")
        }
    };
    assert!(spec.secrets.iter().any(|s| s.expose() == SECRET));

    let out = u.exec("async () => cbm.leak({})").await;
    assert!(!out.text.contains(SECRET), "{}", out.text);
    assert_eq!(
        result(&out),
        json!({"token": "[redacted]", "note": "Bearer [redacted]", "multi": "[redacted]", "quote": "[redacted]", "[redacted]": 1})
    );
    for code in [
        "async () => { const r = await cbm.leak({}); return [r.multi.split('').reverse().join(''), r.quote] }",
        "async () => { const r = await cbm.leak({}); console.log(JSON.stringify(r)); return JSON.stringify(r) }",
    ] {
        let out = u.exec(code).await;
        for leaked in ["line-one-secret", "line-two-secret", "terces-owt-enil", "te\\\\secret", "secret-99"] {
            assert!(!out.text.contains(leaked), "{leaked} in {}", out.text);
        }
    }
    let r = u.ok("async () => { const r = await cbm.leak({}); return r.token.split('').reverse().join('') }").await;
    assert_eq!(r, json!("]detcader["));
    let out = u.exec("async () => { console.log(JSON.stringify(await cbm.leak({}))); return 1 }").await;
    assert!(!out.text.contains(SECRET), "{}", out.text);

    let out = u.exec("async () => cbm.leak_error({})").await;
    assert_eq!(out.text, "Error: cbm.leak_error: token [redacted] was rejected");
    let r = u
        .ok("async () => { try { await cbm.leak_error({}) } catch (e) { return e.message.split('').reverse().join('') } }")
        .await;
    assert!(!r.as_str().unwrap().contains(&SECRET.chars().rev().collect::<String>()));
}

#[tokio::test]
async fn tools_list_is_identical_for_every_caller() {
    let u = setup().await;
    let anon = list_tools(&u.app, None).await;
    assert_eq!(anon, list_tools(&u.app, Some(WORKER)).await);
    assert_eq!(anon, list_tools(&u.app, Some(BOSS)).await);
    u.ok("async () => cbm.search_graph({ query: 'x' })").await;
    u.cbm.set_down(true);
    let _ = u.exec("async () => cbm.search_graph({ query: 'x' })").await;
    assert_eq!(anon, list_tools(&u.app, None).await, "connection state must not leak into descriptions");
    let v: Value = serde_json::from_str(&anon).unwrap();
    let tools = v["result"]["tools"].as_array().unwrap();
    for t in tools {
        let d = t["description"].as_str().unwrap();
        assert!(d.contains("MCP server namespaces: cbm."), "{d}");
        assert!(!d.contains("gh"), "{d}");
        assert!(d.len() < 1600, "{} chars", d.len());
    }
    assert!(tools[0]["description"].as_str().unwrap().contains(r#"kind "mcp""#));
    assert!(tools[1]["description"].as_str().unwrap().contains(r#"tools.call("<namespace>.<tool>", input)"#));
}

#[tokio::test]
async fn healthz_reports_upstreams_and_keeps_old_keys() {
    let u = setup().await;
    u.ok("async () => cbm.search_graph({ query: 'x' })").await;
    let h = healthz(&u.app).await;
    for key in ["version", "uptimeSecs", "t3", "tools", "watcher", "store", "upstreams"] {
        assert!(h.get(key).is_some(), "missing {key}: {h}");
    }
    assert_eq!(h["tools"], 74);
    assert_eq!(h["watcher"]["intervalSecs"], 60);
    let ups = h["upstreams"].as_array().unwrap();
    assert_eq!(ups.len(), 2);
    assert_eq!(ups[0]["name"], "cbm");
    assert_eq!(ups[0]["state"], "ready");
    assert!(ups[0]["tools"].as_u64().unwrap() > 0, "{}", ups[0]);
    assert_eq!(ups[1]["name"], "gh");
    assert_eq!(ups[1]["state"], "disabled");
    assert!(!h.to_string().contains(SECRET));
}

#[tokio::test]
async fn the_catalog_follows_set_tools() {
    let u = setup().await;
    let names = "async () => catalog.filter(t => t.server === 'cbm').map(t => t.name)";
    let before = result(&u.app.search(names).await);
    assert!(!before.as_array().unwrap().contains(&json!("cbm.fresh_tool")));
    let mut tools = cbm_tools();
    tools.push(tool("fresh_tool", "Added later.", json!({"type": "object"})));
    u.cbm.set_tools(tools);
    let after = result(&u.app.search(names).await);
    assert!(after.as_array().unwrap().contains(&json!("cbm.fresh_tool")), "{after}");
    assert_eq!(u.ok("async () => cbm.fresh_tool({ a: 1 })").await["tool"], "fresh_tool");
    assert!(u.app.catalog().declarations().contains("  fresh_tool(input?: Record<string, unknown>)"));

    u.cbm.set_tools(cbm_tools());
    u.app.refresh_catalog().await.unwrap();
    assert!(!u.app.catalog().upstream_names["cbm"].contains("fresh_tool"));
}

#[tokio::test]
async fn search_output_never_carries_a_secret() {
    let u = setup().await;
    let out = u.app.search(r#"async () => catalog.find(t => t.name === "cbm.leak").description"#).await;
    assert_eq!(result(&out), json!("Returns a secret like [redacted]."));
    let out = u.app.search(r#"async () => JSON.stringify(catalog)"#).await;
    assert!(!out.text.contains(SECRET));
}

#[tokio::test]
async fn a_tool_named_call_is_signed_as_reached() {
    let u = setup().await;
    let cat = u.app.catalog();
    let entry = cat.entries.iter().find(|e| e.name == "cbm.call").unwrap();
    assert!(entry.signature.contains(r#"cbm.call("call", input?: "#), "{}", entry.signature);
    let d = cat.declarations();
    let block = &d[d.find("declare const cbm: {").unwrap()..d.find("declare const tools: {").unwrap()];
    assert!(!block.contains("\n  call(input"), "{block}");
    assert_eq!(u.ok(r#"async () => cbm.call("call", { n: 1 })"#).await["tool"], "call");
}

struct Hang;

impl t3_codemode::upstream::Connector for Hang {
    fn connect<'a>(
        &'a self,
        _spec: &'a t3_codemode::upstream::ResolvedUpstream,
    ) -> t3_codemode::upstream::BoxFut<'a, Result<Arc<dyn t3_codemode::upstream::Upstream>, UpstreamError>> {
        Box::pin(std::future::pending())
    }
}

#[tokio::test]
async fn a_hung_upstream_does_not_stall_search_or_execute() {
    let dir = tempfile::tempdir().unwrap();
    let config = Config::from_value(json!({"upstreams": [
        {"name": "slow", "transport": "stdio", "stdio": {"command": FAKE_BIN}}
    ]}))
    .unwrap();
    let manager = Arc::new(UpstreamManager::with_connector(config.upstreams.clone(), Arc::new(Hang)));
    let t3 = Arc::new(FakeT3::with_tools(fixture_tools()));
    let store = Arc::new(Store::open(dir.path()).unwrap());
    let app = App::with_upstreams(config, store, t3, Arc::new(FakeClock::at(T0)), manager);
    for _ in 0..2 {
        let started = std::time::Instant::now();
        let out = app
            .execute(ExecuteRequest { code: "async () => 1".into(), session: None, timeout_ms: None, caller: None })
            .await;
        assert_eq!(result(&out), json!(1));
        assert!(started.elapsed() < std::time::Duration::from_secs(5), "{:?}", started.elapsed());
        let started = std::time::Instant::now();
        assert_eq!(result(&app.search("async () => catalog.length > 0").await), json!(true));
        assert!(started.elapsed() < std::time::Duration::from_secs(5), "{:?}", started.elapsed());
    }
    let started = std::time::Instant::now();
    assert!(tokio::time::timeout(std::time::Duration::from_secs(5), app.refresh_t3_catalog()).await.is_ok());
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
}

#[tokio::test]
async fn the_server_refuses_a_foreign_host() {
    let u = setup().await;
    let (listener, addr) = server::bind(0).await.unwrap();
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let app = u.app.clone();
    let task = tokio::spawn(async move {
        server::serve(app, listener, async move {
            let _ = rx.await;
        })
        .await
        .unwrap();
    });
    let http = reqwest::Client::builder().no_proxy().build().unwrap();
    let get = |host: String| http.get(format!("http://{addr}/healthz")).header("host", host).send();
    assert_eq!(get(format!("rebound.example:{}", addr.port())).await.unwrap().status(), 403);
    for host in [format!("127.0.0.1:{}", addr.port()), format!("localhost:{}", addr.port()), "[::1]".into()] {
        assert_eq!(get(host.clone()).await.unwrap().status(), 200, "{host}");
    }
    let post = http
        .post(format!("http://{addr}/mcp"))
        .header("host", "rebound.example")
        .json(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}))
        .send()
        .await
        .unwrap();
    assert_eq!(post.status(), 403);
    let _ = tx.send(());
    let _ = task.await;
}
