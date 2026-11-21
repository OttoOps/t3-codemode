use std::sync::Arc;
use std::time::Duration;

use futures_util::future::join_all;
use jiff::Timestamp;
use serde_json::{Value, json};
use t3_codemode::config::Config;
use t3_codemode::upstream::config::{glob_match, has_reference, parse_references, resolve, resolve_value};
use t3_codemode::upstream::fake::{FakeConnector, FakeUpstream, tool};
use t3_codemode::upstream::{
    BoxFut, Connector, REDACTED, RESERVED_JS, RESERVED_NAMES, Reference, ResolvedTransport, ResolvedUpstream, ToolDef,
    ToolFilter, Transport, Upstream, UpstreamConfig, UpstreamError, UpstreamManager, UpstreamState,
};

fn example() -> Value {
    json!({
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
    })
}

fn http_up(name: &str) -> Value {
    json!({"name": name, "transport": "http", "http": {"url": "http://127.0.0.1:9/mcp"}})
}

const SHELL: &str = if cfg!(windows) { "cmd" } else { "sh" };

fn cfg(v: Value) -> UpstreamConfig {
    serde_json::from_value(v).unwrap()
}

fn with(mut base: Value, patch: Value) -> Value {
    for (k, v) in patch.as_object().unwrap() {
        base[k] = v.clone();
    }
    base
}

fn set_env(name: &str, value: &str) {
    unsafe { std::env::set_var(name, value) };
}

fn echo_tools() -> Vec<ToolDef> {
    vec![tool("echo", "echo the input", json!({"type": "object"}))]
}

fn names(tools: &[ToolDef]) -> Vec<&str> {
    tools.iter().map(|t| t.name.as_str()).collect()
}

fn manager(configs: Vec<Value>, fakes: &[&Arc<FakeUpstream>]) -> (UpstreamManager, Arc<FakeConnector>) {
    let connector = FakeConnector::new();
    for f in fakes {
        connector.add(Arc::clone(f));
    }
    let configs = configs.into_iter().map(cfg).collect();
    (UpstreamManager::with_connector(configs, connector.clone()), connector)
}

fn state(m: &UpstreamManager, name: &str) -> t3_codemode::upstream::UpstreamStatus {
    m.status().into_iter().find(|s| s.name == name).unwrap()
}

#[test]
fn example_config_parses_and_defaults_apply() {
    let c = Config::from_value(example()).unwrap();
    assert_eq!(c.upstreams.len(), 3);
    let [cbm, linear, gh] = [&c.upstreams[0], &c.upstreams[1], &c.upstreams[2]];
    assert_eq!(cbm.transport, Transport::Stdio);
    assert!(cbm.enabled);
    assert_eq!(cbm.timeout_ms, 120_000);
    assert_eq!(cbm.allow, vec!["*"]);
    assert_eq!(cbm.deny, vec!["delete_project", "index_repository"]);
    assert!(cbm.stdio.as_ref().unwrap().args.is_empty());
    assert!(linear.enabled);
    assert_eq!(linear.timeout_ms, 60_000);
    assert!(linear.deny.is_empty());
    assert_eq!(linear.allow, vec!["list_*", "get_*", "search_*"]);
    assert!(!gh.enabled);
    assert_eq!(gh.stdio.as_ref().unwrap().cwd.as_deref(), Some("~"));
    assert!(Config::default().upstreams.is_empty());
}

fn reject(upstreams: Vec<Value>, index: usize, name: &str, field: &str) {
    let err = Config::from_value(json!({ "upstreams": upstreams })).err().unwrap_or_else(|| {
        panic!("expected a rejection naming {field}");
    });
    let msg = format!("{err:#}");
    let who = format!("upstreams[{index}] \"{name}\"");
    assert!(msg.contains(&who), "{msg:?} should name {who}");
    assert!(msg.contains(field), "{msg:?} should name {field}");
}

fn reject_one(up: Value, field: &str) {
    let name = up["name"].as_str().unwrap().to_string();
    reject(vec![up], 0, &name, field);
}

#[test]
fn rejects_bad_names() {
    for name in RESERVED_NAMES.iter().chain(RESERVED_JS) {
        reject_one(http_up(name), "name");
    }
    for name in ["class", "function", "await", "Foo", "a-b", "9lives", "", &"a".repeat(33)] {
        reject_one(http_up(name), "name");
    }
    assert!(Config::from_value(json!({"upstreams": [http_up(&"a".repeat(32))]})).is_ok());
    reject(vec![http_up("dup"), http_up("other"), http_up("dup")], 2, "dup", "duplicate");
    let disabled = with(http_up("dup"), json!({"enabled": false}));
    reject(vec![http_up("dup"), disabled], 1, "dup", "name");
}

#[test]
fn rejects_too_many_upstreams() {
    let ups: Vec<Value> = (0..33).map(|i| http_up(&format!("u{i}"))).collect();
    let err = format!("{:#}", Config::from_value(json!({ "upstreams": ups })).unwrap_err());
    assert!(err.contains("at most 32"), "{err}");
    let ups: Vec<Value> = (0..32).map(|i| http_up(&format!("u{i}"))).collect();
    assert!(Config::from_value(json!({ "upstreams": ups })).is_ok());
}

#[test]
fn rejects_bad_transport_sections() {
    reject_one(json!({"name": "a", "transport": "stdio", "http": {"url": "http://x"}}), "stdio");
    reject_one(json!({"name": "a", "transport": "http", "stdio": {"command": "x"}}), "http");
    reject_one(json!({"name": "a", "transport": "http"}), "http");
    reject_one(
        json!({"name": "a", "transport": "http", "http": {"url": "http://x"}, "stdio": {"command": "x"}}),
        "transport",
    );
    for url in ["ftp://x", "localhost:8080", "https://", "file:///etc/passwd"] {
        reject_one(json!({"name": "a", "transport": "http", "http": {"url": url}}), "http.url");
    }
}

#[test]
fn rejects_references_where_not_allowed() {
    let stdio = |patch: Value| {
        let mut s = json!({"command": "x"});
        for (k, v) in patch.as_object().unwrap() {
            s[k] = v.clone();
        }
        json!({"name": "a", "transport": "stdio", "stdio": s})
    };
    reject_one(stdio(json!({"command": "${env:CMD}"})), "stdio.command");
    reject_one(stdio(json!({"args": ["ok", "--token=${env:T}"]})), "stdio.args[1]");
    reject_one(stdio(json!({"cwd": "${env:HOME}"})), "stdio.cwd");
    reject_one(json!({"name": "a", "transport": "http", "http": {"url": "https://${env:H}/mcp"}}), "http.url");
    reject_one(json!({"name": "a", "transport": "http", "http": {"url": "https://x/${env:}"}}), "http.url");
    let path_ref = json!({"name": "a", "transport": "http", "http": {"url": "https://x/s/${env:P}/mcp"}});
    assert!(Config::from_value(json!({"upstreams": [path_ref]})).is_ok());
}

#[test]
fn rejects_credentials_over_cleartext_to_remote_hosts() {
    let up = |url: &str, headers: Value, insecure: bool| {
        json!({"name": "a", "transport": "http",
               "http": {"url": url, "headers": headers, "allow_insecure": insecure}})
    };
    let auth = json!({"Authorization": "Bearer ${env:X}"});
    let ok = |v: Value| assert!(Config::from_value(json!({"upstreams": [v.clone()]})).is_ok(), "{v}");
    reject_one(up("http://mcp.example.com/mcp", auth.clone(), false), "http.url");
    reject_one(up("http://mcp.example.com/s/${env:P}/mcp", json!({}), false), "http.url");
    reject_one(up("http://user:pw@mcp.example.com:8080/mcp", auth.clone(), false), "http.url");
    ok(up("http://mcp.example.com/mcp", auth.clone(), true));
    ok(up("https://mcp.example.com/mcp", auth.clone(), false));
    ok(up("http://mcp.example.com/mcp", json!({"X-Team": "blue"}), false));
    for host in ["127.0.0.1:9", "localhost:9", "[::1]:9", "api.localhost"] {
        ok(up(&format!("http://{host}/mcp"), auth.clone(), false));
    }
}

#[test]
fn url_references_and_query_credentials_become_secrets() {
    set_env("UPMGR_URL_TOKEN", "url-secret-31");
    let up = cfg(json!({
        "name": "u",
        "transport": "http",
        "http": {"url": "https://example.test/s/${env:UPMGR_URL_TOKEN}/mcp?api_key=query-secret-8&format=json"}
    }));
    let r = resolve(&up).unwrap();
    let ResolvedTransport::Http { url, .. } = &r.transport else { panic!("http") };
    assert_eq!(url.expose(), "https://example.test/s/url-secret-31/mcp?api_key=query-secret-8&format=json");
    let secrets: Vec<&str> = r.secrets.iter().map(|s| s.expose()).collect();
    assert!(secrets.contains(&"url-secret-31") && secrets.contains(&"query-secret-8"));
    assert!(!secrets.contains(&"json"));
    assert!(!format!("{r:?}").contains("url-secret-31"));
}

#[test]
fn a_relative_command_resolves_against_the_configured_cwd() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("bin")).unwrap();
    let srv = dir.path().join("bin").join(format!("srv{}", std::env::consts::EXE_SUFFIX));
    std::fs::copy(env!("CARGO_BIN_EXE_t3cm-fake-mcp"), &srv).unwrap();
    let up = cfg(json!({"name": "rel", "transport": "stdio",
                        "stdio": {"command": "./bin/srv", "cwd": dir.path().display().to_string()}}));
    let r = resolve(&up).unwrap();
    let ResolvedTransport::Stdio { command, .. } = &r.transport else { panic!("stdio") };
    assert!(command.is_absolute(), "{command:?}");
    assert_eq!(command.canonicalize().unwrap(), srv.canonicalize().unwrap());
    let missing = cfg(json!({"name": "rel", "transport": "stdio",
                             "stdio": {"command": "./bin/nope", "cwd": dir.path().display().to_string()}}));
    assert!(resolve(&missing).unwrap_err().message().contains("./bin/nope"));
}

#[test]
fn rejects_malformed_references_and_literal_secrets() {
    let header =
        |k: &str, v: &str| json!({"name": "a", "transport": "http", "http": {"url": "https://x", "headers": {k: v}}});
    let env = |k: &str, v: &str| json!({"name": "a", "transport": "stdio", "stdio": {"command": "x", "env": {k: v}}});
    for bad in ["${env:}", "${foo:x}", "Bearer ${env:X", "${file:relative/path}", "${env:1X}", "${}"] {
        reject_one(header("X-Thing", bad), "http.headers.X-Thing");
        reject_one(env("THING", bad), "stdio.env.THING");
    }
    reject_one(header("Authorization", "Bearer abc123"), "http.headers.Authorization");
    reject_one(header("proxy-authorization", "Basic abc"), "http.headers.proxy-authorization");
    reject_one(header("Cookie", "s=1"), "http.headers.Cookie");
    reject_one(header("X-Api-Key", "abc"), "http.headers.X-Api-Key");
    reject_one(header("Authorization", "Bearer $${env:X}"), "http.headers.Authorization");
    reject_one(env("FOO_TOKEN", "abc"), "stdio.env.FOO_TOKEN");
    reject_one(env("db_password", "abc"), "stdio.env.db_password");
    assert!(Config::from_value(json!({"upstreams": [header("Authorization", "Bearer ${env:X}")]})).is_ok());
    assert!(Config::from_value(json!({"upstreams": [env("FOO_TOKEN", "${file:~/t}")]})).is_ok());
    assert!(Config::from_value(json!({"upstreams": [env("PLAIN", "a $${b} $c")]})).is_ok());
}

#[test]
fn rejects_timeout_and_empty_globs() {
    for t in [0, 99, 600_001] {
        reject_one(with(http_up("a"), json!({"timeout_ms": t})), "timeout_ms");
    }
    for t in [100, 600_000] {
        assert!(Config::from_value(json!({"upstreams": [with(http_up("a"), json!({"timeout_ms": t}))]})).is_ok());
    }
    reject_one(with(http_up("a"), json!({"allow": ["get_*", ""]})), "allow[1]");
    reject_one(with(http_up("a"), json!({"deny": [""]})), "deny[0]");
}

#[test]
fn parses_references() {
    assert_eq!(parse_references("plain").unwrap(), vec![Reference::Literal("plain".into())]);
    assert_eq!(
        parse_references("Bearer ${env:TOK}!").unwrap(),
        vec![Reference::Literal("Bearer ".into()), Reference::Env("TOK".into()), Reference::Literal("!".into())]
    );
    assert_eq!(parse_references("${file:~/a/b}").unwrap(), vec![Reference::File("~/a/b".into())]);
    assert_eq!(parse_references("a $${x} $ $$").unwrap(), vec![Reference::Literal("a ${x} $ $$".into())]);
    assert!(parse_references("${env:}").is_err());
    assert!(parse_references("${foo:x}").is_err());
    assert!(parse_references("${env:X").is_err());
    assert!(has_reference("x ${env:A}"));
    assert!(has_reference("${file:/x}"));
    assert!(!has_reference("$${env:A}"));
    assert!(!has_reference("plain"));
    assert!(!has_reference("${broken"));
}

#[test]
fn resolves_references_into_secrets() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("token");
    std::fs::write(&file, "file-secret-77\r\n\n").unwrap();
    set_env("UPMGR_RESOLVE_TOKEN", "env-secret-42");
    let up = cfg(json!({
        "name": "svc",
        "transport": "http",
        "timeout_ms": 1500,
        "http": {
            "url": "https://example.test/mcp",
            "headers": {
                "Authorization": "Bearer ${env:UPMGR_RESOLVE_TOKEN}",
                "X-File": format!("${{file:{}}}", file.display()),
                "X-Lit": "a $${b}",
            }
        }
    }));
    let r = resolve(&up).unwrap();
    assert_eq!(r.name, "svc");
    assert_eq!(r.timeout, Duration::from_millis(1500));
    let ResolvedTransport::Http { url, headers } = &r.transport else { panic!("http") };
    assert_eq!(url.expose(), "https://example.test/mcp");
    let h: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (k.as_str(), v.expose())).collect();
    assert_eq!(h, vec![("Authorization", "Bearer env-secret-42"), ("X-File", "file-secret-77"), ("X-Lit", "a ${b}")]);
    let secrets: Vec<&str> = r.secrets.iter().map(|s| s.expose()).collect();
    for s in ["env-secret-42", "Bearer env-secret-42", "file-secret-77"] {
        assert!(secrets.contains(&s), "{s} in secrets");
    }
    assert!(!secrets.contains(&"a ${b}"));
    let dbg = format!("{r:?}");
    assert!(!dbg.contains("env-secret-42") && !dbg.contains("file-secret-77"), "{dbg}");
    assert!(dbg.contains(REDACTED));
    assert_eq!(r.redact("x env-secret-42 y".into()), format!("x {REDACTED} y"));

    assert_eq!(resolve_value("pre-${env:UPMGR_RESOLVE_TOKEN}").unwrap().expose(), "pre-env-secret-42");
    assert_eq!(resolve_value("$${env:UPMGR_RESOLVE_TOKEN}").unwrap().expose(), "${env:UPMGR_RESOLVE_TOKEN}");
    assert_eq!(format!("{:?}", resolve_value("${env:UPMGR_RESOLVE_TOKEN}").unwrap()), REDACTED);
}

#[test]
fn resolve_failures_name_the_reference_not_the_value() {
    set_env("UPMGR_RESOLVE_EMPTY", "");
    let up =
        |v: &str| cfg(json!({"name": "svc", "transport": "http", "http": {"url": "http://x", "headers": {"X-A": v}}}));
    let e = resolve(&up("${env:UPMGR_NEVER_SET_1}")).unwrap_err();
    assert!(matches!(e, UpstreamError::Config(_)));
    assert_eq!(e.message(), "svc: env var UPMGR_NEVER_SET_1 is not set");
    let e = resolve(&up("${env:UPMGR_RESOLVE_EMPTY}")).unwrap_err();
    assert!(e.message().contains("UPMGR_RESOLVE_EMPTY"));
    let e = resolve(&up("${file:/nonexistent/upmgr/secret}")).unwrap_err();
    assert!(matches!(e, UpstreamError::Config(_)));
    assert!(e.message().starts_with("svc: ") && e.message().contains("/nonexistent/upmgr/secret"), "{e}");
    let e = resolve_value("${env:UPMGR_NEVER_SET_2}").unwrap_err();
    assert_eq!(e, UpstreamError::Config("env var UPMGR_NEVER_SET_2 is not set".into()));
}

#[test]
fn resolves_stdio_command_cwd_and_env() {
    set_env("UPMGR_STDIO_SECRET", "stdio-secret-9");
    let up = cfg(json!({
        "name": "s",
        "transport": "stdio",
        "stdio": {"command": SHELL, "args": ["-c", "true"], "cwd": "~", "env": {"API_TOKEN": "${env:UPMGR_STDIO_SECRET}", "PLAIN": "1"}}
    }));
    let r = resolve(&up).unwrap();
    let ResolvedTransport::Stdio { command, args, env, cwd } = &r.transport else { panic!("stdio") };
    assert!(command.is_absolute(), "{command:?}");
    assert!(command.file_stem().is_some_and(|s| s.eq_ignore_ascii_case(SHELL)), "{command:?}");
    assert_eq!(args, &vec!["-c".to_string(), "true".to_string()]);
    assert_eq!(cwd.clone(), t3_codemode::platform::home_dir());
    let env: Vec<(&str, &str)> = env.iter().map(|(k, v)| (k.as_str(), v.expose())).collect();
    assert_eq!(env, vec![("API_TOKEN", "stdio-secret-9"), ("PLAIN", "1")]);
    assert_eq!(r.secrets.len(), 1);

    let abs =
        cfg(json!({"name": "s", "transport": "stdio", "stdio": {"command": env!("CARGO_BIN_EXE_t3cm-fake-mcp")}}));
    assert!(resolve(&abs).is_ok());
    let missing = cfg(json!({"name": "s", "transport": "stdio", "stdio": {"command": "upmgr-no-such-command"}}));
    let e = resolve(&missing).unwrap_err();
    assert!(matches!(e, UpstreamError::Config(_)));
    assert!(e.message().contains("upmgr-no-such-command"), "{e}");
    let bad_cwd =
        cfg(json!({"name": "s", "transport": "stdio", "stdio": {"command": SHELL, "cwd": "/nonexistent/upmgr"}}));
    let e = resolve(&bad_cwd).unwrap_err();
    assert!(e.message().contains("/nonexistent/upmgr"), "{e}");
}

#[test]
fn globs_and_filter() {
    assert!(glob_match("*", "anything"));
    assert!(glob_match("*", ""));
    assert!(glob_match("list_*", "list_issues"));
    assert!(!glob_match("list_*", "get_list_x"));
    assert!(glob_match("*_project", "delete_project"));
    assert!(glob_match("get_?", "get_a"));
    assert!(!glob_match("get_?", "get_ab"));
    assert!(glob_match("a*b*c", "axxbyyc"));
    assert!(!glob_match("a*b*c", "axxbyy"));
    assert!(glob_match("exact", "exact"));
    assert!(!glob_match("exact", "exactly"));
    assert!(glob_match("*.*", "a.b"));

    let f = ToolFilter::new(&["list_*".into(), "get_*".into()], &["get_secret*".into()]);
    assert!(f.allows("list_issues"));
    assert!(f.allows("get_issue"));
    assert!(!f.allows("get_secrets"));
    assert!(!f.allows("delete_issue"));
    let deny_all = ToolFilter::new(&["*".into()], &["*".into()]);
    assert!(!deny_all.allows("x"));
    assert!(!ToolFilter::new(&[], &[]).allows("x"));
}

#[tokio::test]
async fn namespaces_are_enabled_only_in_config_order() {
    let (m, conn) = manager(
        vec![http_up("zeta"), with(http_up("off"), json!({"enabled": false})), http_up("alpha")],
        &[&FakeUpstream::new("zeta", echo_tools()), &FakeUpstream::new("off", echo_tools())],
    );
    assert_eq!(m.namespaces(), vec!["zeta", "alpha"]);
    assert!(m.is_namespace("zeta") && !m.is_namespace("off") && !m.is_namespace("t3"));
    let e = m.call("off", "echo", json!({})).await.unwrap_err();
    assert_eq!(e, UpstreamError::UnknownNamespace("unknown namespace \"off\"; namespaces are t3, zeta, alpha".into()));
    assert!(matches!(m.call("nope", "echo", json!({})).await, Err(UpstreamError::UnknownNamespace(_))));
    let st = m.status();
    assert_eq!(st.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), vec!["zeta", "off", "alpha"]);
    assert_eq!(st[1].state, UpstreamState::Disabled);
    assert!(!st[1].enabled);
    m.tools().await;
    assert_eq!(conn.connects("off"), 0);
    assert_eq!(state(&m, "off").state, UpstreamState::Disabled);
    assert_eq!(UpstreamManager::empty().namespaces(), Vec::<String>::new());
}

#[tokio::test]
async fn connects_lazily() {
    let fake = FakeUpstream::new("a", echo_tools());
    let (m, conn) = manager(vec![http_up("a")], &[&fake]);
    assert_eq!(conn.connects("a"), 0);
    assert_eq!(state(&m, "a").state, UpstreamState::Idle);
    assert_eq!(m.cached_tools(), vec![("a".to_string(), vec![])]);
    assert_eq!(m.generation(), 0);
    let tools = m.tools().await;
    assert_eq!(conn.connects("a"), 1);
    assert_eq!(tools[0].0, "a");
    assert_eq!(names(&tools[0].1), vec!["echo"]);
    let s = state(&m, "a");
    assert_eq!(s.state, UpstreamState::Ready);
    assert_eq!(s.tools, 1);
    assert_eq!(s.server, Some(json!({"name": "fake-a", "version": "0"})));
    m.tools().await;
    m.call("a", "echo", json!({"x": 1})).await.unwrap();
    assert_eq!(conn.connects("a"), 1);
    assert_eq!(m.generation(), 1);
    let _ = UpstreamManager::new(vec![cfg(http_up("never"))]);
}

#[tokio::test]
async fn filters_tools_and_denies_without_contact() {
    let fake = FakeUpstream::new(
        "a",
        vec![
            tool("list_x", "", json!({})),
            tool("get_y", "", json!({})),
            tool("delete_z", "", json!({})),
            tool("list_x", "duplicate", json!({})),
        ],
    );
    let (m, conn) = manager(vec![with(http_up("a"), json!({"deny": ["delete_*"]}))], &[&fake]);
    let e = m.call("a", "delete_z", json!({})).await.unwrap_err();
    assert_eq!(
        e,
        UpstreamError::Denied(
            "a.delete_z is not allowed by the t3-codemode config for upstream a; do not retry it".into()
        )
    );
    assert_eq!(conn.connects("a"), 0);
    let tools = m.tools().await;
    assert_eq!(names(&tools[0].1), vec!["list_x", "get_y"]);
    assert_eq!(tools[0].1[0].description, "");
    assert!(matches!(m.call("a", "delete_z", json!({})).await, Err(UpstreamError::Denied(_))));
    assert!(fake.calls().is_empty());
    let out = m.call("a", "get_y", json!({"q": 1})).await.unwrap();
    assert_eq!(out, json!({"server": "a", "tool": "get_y", "args": {"q": 1}}));
    assert_eq!(fake.calls(), vec![("get_y".to_string(), json!({"q": 1}))]);
}

#[tokio::test]
async fn unknown_tool_is_not_called() {
    let fake = FakeUpstream::new("a", echo_tools());
    let (m, _) = manager(vec![http_up("a")], &[&fake]);
    let e = m.call("a", "nope", json!({})).await.unwrap_err();
    assert_eq!(e, UpstreamError::UnknownTool("unknown tool a.nope; use the search tool to find the right name".into()));
    assert!(fake.calls().is_empty());
    assert_eq!(state(&m, "a").state, UpstreamState::Ready);
}

#[tokio::test]
async fn tool_errors_keep_the_connection() {
    let fake = FakeUpstream::new("a", echo_tools());
    fake.reply("echo", Err(UpstreamError::Tool("a.echo: boom".into())));
    let (m, conn) = manager(vec![http_up("a")], &[&fake]);
    assert_eq!(m.call("a", "echo", json!({})).await, Err(UpstreamError::Tool("a.echo: boom".into())));
    let s = state(&m, "a");
    assert_eq!((s.state, s.restarts), (UpstreamState::Ready, 0));
    assert_eq!(fake.calls().len(), 1);
    assert_eq!(fake.shutdowns(), 0);
    assert_eq!(conn.connects("a"), 1);
}

#[tokio::test]
async fn backoff_and_restart() {
    let fake = FakeUpstream::new("a", echo_tools());
    let (m, conn) = manager(vec![http_up("a")], &[&fake]);
    m.call("a", "echo", json!({})).await.unwrap();

    fake.set_down(true);
    let e = m.call("a", "echo", json!({})).await.unwrap_err();
    assert!(matches!(e, UpstreamError::Unavailable(_)), "{e:?}");
    assert_eq!(fake.calls().len(), 1);
    let s = state(&m, "a");
    assert_eq!((s.state, s.restarts), (UpstreamState::Backoff, 1));
    assert!(s.last_error.as_deref().unwrap().contains("connection lost"));
    assert!(s.retry_at.is_some());
    assert_eq!(fake.shutdowns(), 1);
    assert_eq!(m.cached_tools()[0].1.len(), 1);

    m.call("a", "echo", json!({})).await.unwrap();
    assert_eq!(conn.connects("a"), 2);
    let s = state(&m, "a");
    assert_eq!((s.state, s.restarts, s.retry_at.clone()), (UpstreamState::Ready, 1, None));

    fake.set_down(true);
    conn.refuse("a", Some("refused"));
    assert!(m.call("a", "echo", json!({})).await.is_err());
    assert_eq!(state(&m, "a").restarts, 2);
    let before = Timestamp::now();
    let e = m.call("a", "echo", json!({})).await.unwrap_err();
    assert_eq!(conn.connects("a"), 3);
    assert_eq!(e, UpstreamError::Unavailable("a: refused".into()));
    let s = state(&m, "a");
    assert_eq!((s.state, s.restarts), (UpstreamState::Backoff, 3));
    let retry: Timestamp = s.retry_at.clone().unwrap().parse().unwrap();
    let ahead = retry.duration_since(before).as_secs_f64();
    assert!((0.99..2.0).contains(&ahead), "retry_at {ahead} s ahead");

    let e = m.call("a", "echo", json!({})).await.unwrap_err();
    let UpstreamError::Unavailable(msg) = &e else { panic!("{e:?}") };
    assert_eq!(msg, &format!("a is unavailable (retry at {}): a: refused", s.retry_at.unwrap()));
    assert_eq!(conn.connects("a"), 3);
    m.tools().await;
    assert_eq!(conn.connects("a"), 3);
    assert_eq!(state(&m, "a").restarts, 3);

    conn.refuse("a", None);
    tokio::time::sleep(Duration::from_millis(1100)).await;
    m.call("a", "echo", json!({})).await.unwrap();
    assert_eq!(conn.connects("a"), 4);
    let s = state(&m, "a");
    assert_eq!((s.state, s.restarts, s.retry_at, s.last_error), (UpstreamState::Ready, 3, None, None));

    conn.refuse("a", Some("refused again"));
    fake.set_down(true);
    assert!(m.call("a", "echo", json!({})).await.is_err());
    let s = state(&m, "a");
    let retry: Timestamp = s.retry_at.unwrap().parse().unwrap();
    assert!(retry.duration_since(Timestamp::now()).as_secs_f64() < 0.5, "success reset the backoff");
    assert!(m.call("a", "echo", json!({})).await.unwrap_err().message().contains("refused again"));
    assert_eq!(conn.connects("a"), 5);
}

#[tokio::test]
async fn backoff_grows_after_the_first_failure() {
    let (m, conn) = manager(vec![http_up("a")], &[]);
    conn.refuse("a", Some("down"));
    let mut aheads = Vec::new();
    for _ in 0..2 {
        let now = Timestamp::now();
        assert!(m.call("a", "echo", json!({})).await.is_err());
        let retry: Timestamp = state(&m, "a").retry_at.unwrap().parse().unwrap();
        aheads.push(retry.duration_since(now).as_secs_f64().round() as i64);
    }
    assert_eq!(aheads, vec![0, 1]);
    assert_eq!(conn.connects("a"), 2);
}

struct SlowConnector {
    inner: Arc<FakeConnector>,
    delay: Duration,
}

impl Connector for SlowConnector {
    fn connect<'a>(&'a self, spec: &'a ResolvedUpstream) -> BoxFut<'a, Result<Arc<dyn Upstream>, UpstreamError>> {
        Box::pin(async move {
            tokio::time::sleep(self.delay).await;
            self.inner.connect(spec).await
        })
    }
}

fn slow(configs: Vec<Value>, fake: &Arc<FakeUpstream>, delay: Duration) -> (Arc<UpstreamManager>, Arc<FakeConnector>) {
    let inner = FakeConnector::new();
    inner.add(fake.clone());
    let connector = Arc::new(SlowConnector { inner: inner.clone(), delay });
    let m = UpstreamManager::with_connector(configs.into_iter().map(cfg).collect(), connector);
    (Arc::new(m), inner)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_first_calls_connect_once() {
    let fake = FakeUpstream::new("a", echo_tools());
    let (m, conn) = slow(vec![http_up("a")], &fake, Duration::from_millis(100));
    let calls = (0..10).map(|i| {
        let m = m.clone();
        tokio::spawn(async move { m.call("a", "echo", json!({"i": i})).await })
    });
    let results = join_all(calls).await;
    for (i, r) in results.into_iter().enumerate() {
        assert_eq!(r.unwrap().unwrap()["args"]["i"], i);
    }
    assert_eq!(conn.connects("a"), 1);
    assert_eq!(fake.calls().len(), 10);

    let fake_b = FakeUpstream::new("b", echo_tools());
    let (m, conn) = slow(vec![http_up("b")], &fake_b, Duration::from_millis(100));
    conn.refuse("b", Some("nope"));
    let results = join_all((0..10).map(|_| m.call("b", "echo", json!({})))).await;
    assert!(results.iter().all(|r| matches!(r, Err(UpstreamError::Unavailable(_)))));
    assert_eq!(conn.connects("b"), 1);
    assert_eq!(state(&m, "b").restarts, 1);
}

#[tokio::test]
async fn relists_on_change_and_cache_survives_backoff() {
    let fake = FakeUpstream::new("a", echo_tools());
    let (m, _) = manager(vec![with(http_up("a"), json!({"deny": ["hidden"]}))], &[&fake]);
    assert_eq!(m.generation(), 0);
    m.tools().await;
    let g1 = m.generation();
    assert_eq!(g1, 1);
    m.tools().await;
    m.call("a", "echo", json!({})).await.unwrap();
    assert_eq!(m.generation(), g1);

    fake.set_tools(vec![tool("echo", "", json!({})), tool("hidden", "", json!({}))]);
    m.call("a", "echo", json!({})).await.unwrap();
    let g2 = m.generation();
    assert!(g2 > g1);
    assert_eq!(names(&m.cached_tools()[0].1), vec!["echo"]);

    fake.set_tools(vec![tool("echo", "", json!({})), tool("hidden", "", json!({}))]);
    m.tools().await;
    assert_eq!(m.generation(), g2, "an unchanged filtered list keeps the generation");

    fake.set_tools(vec![tool("echo", "", json!({})), tool("fresh", "", json!({}))]);
    let out = m.call("a", "fresh", json!({})).await.unwrap();
    assert_eq!(out["tool"], "fresh");
    let g3 = m.generation();
    assert!(g3 > g2);

    fake.set_tools(vec![tool("echo", "", json!({})), tool("fresh", "", json!({})), tool("third", "", json!({}))]);
    let listed = m.tools().await;
    assert_eq!(names(&listed[0].1), vec!["echo", "fresh", "third"]);
    assert!(m.generation() > g3);

    fake.set_down(true);
    assert!(m.call("a", "echo", json!({})).await.is_err());
    assert_eq!(state(&m, "a").state, UpstreamState::Backoff);
    assert_eq!(names(&m.cached_tools()[0].1), vec!["echo", "fresh", "third"]);
    assert_eq!(state(&m, "a").tools, 3);
}

#[tokio::test]
async fn refresh_relists_and_reports() {
    let a = FakeUpstream::new("a", echo_tools());
    let (m, conn) = manager(vec![http_up("a"), http_up("b")], &[&a]);
    let r = m.refresh().await;
    assert_eq!(r[0], ("a".to_string(), Ok(1)));
    assert_eq!(r[1].0, "b");
    assert!(r[1].1.as_ref().unwrap_err().contains("no fake registered"));
    a.set_tools(vec![tool("echo", "", json!({})), tool("two", "", json!({}))]);
    let g = m.generation();
    let r = m.refresh().await;
    assert_eq!(r[0], ("a".to_string(), Ok(2)));
    assert!(m.generation() > g);
    assert_eq!(conn.connects("a"), 1);
}

#[tokio::test]
async fn redacts_secrets_everywhere() {
    set_env("UPMGR_REDACT_TOKEN", "sekrit-token-123");
    let up = json!({
        "name": "a",
        "transport": "http",
        "http": {"url": "http://127.0.0.1:9/mcp", "headers": {"Authorization": "Bearer ${env:UPMGR_REDACT_TOKEN}"}}
    });
    let fake = FakeUpstream::new("a", echo_tools());
    let (m, conn) = manager(vec![up], &[&fake]);
    conn.refuse("a", Some("rejected sekrit-token-123 and Bearer sekrit-token-123"));
    let e = m.call("a", "echo", json!({})).await.unwrap_err();
    assert!(!e.message().contains("sekrit"), "{e}");
    assert!(e.message().contains(REDACTED));
    let last = state(&m, "a").last_error.unwrap();
    assert!(!last.contains("sekrit") && last.contains(REDACTED), "{last}");
    assert_eq!(m.redact("token=sekrit-token-123;".into()), format!("token={REDACTED};"));
    let spec = conn.last_spec("a").unwrap();
    assert!(!format!("{spec:?}").contains("sekrit"));

    conn.refuse("a", None);
    fake.reply("echo", Err(UpstreamError::Tool("a.echo: saw sekrit-token-123".into())));
    let e = m.call("a", "echo", json!({})).await.unwrap_err();
    assert_eq!(e, UpstreamError::Tool(format!("a.echo: saw {REDACTED}")));
}

#[tokio::test]
async fn call_timeout_is_a_backstop() {
    let fake = FakeUpstream::new("a", vec![tool("slow", "", json!({}))]);
    let (m, conn) = manager(vec![with(http_up("a"), json!({"timeout_ms": 100}))], &[&fake]);
    m.tools().await;
    fake.set_delay(Duration::from_millis(400));
    let started = std::time::Instant::now();
    let e = m.call("a", "slow", json!({})).await.unwrap_err();
    assert_eq!(e, UpstreamError::Timeout("a.slow: timed out after 100 ms".into()));
    assert!(started.elapsed() < Duration::from_millis(390), "{:?}", started.elapsed());
    let s = state(&m, "a");
    assert_eq!((s.state, s.restarts), (UpstreamState::Ready, 0));
    fake.set_delay(Duration::ZERO);
    m.call("a", "slow", json!({})).await.unwrap();
    assert_eq!(conn.connects("a"), 1);
}

#[tokio::test]
async fn shutdown_reaches_each_live_connection() {
    let a = FakeUpstream::new("a", echo_tools());
    let b = FakeUpstream::new("b", echo_tools());
    let idle = FakeUpstream::new("c", echo_tools());
    let (m, _) = manager(vec![http_up("a"), http_up("b"), http_up("c")], &[&a, &b, &idle]);
    m.call("a", "echo", json!({})).await.unwrap();
    m.call("b", "echo", json!({})).await.unwrap();
    m.shutdown().await;
    assert_eq!((a.shutdowns(), b.shutdowns(), idle.shutdowns()), (1, 1, 0));
    assert_eq!(state(&m, "a").state, UpstreamState::Idle);
    m.shutdown().await;
    assert_eq!(a.shutdowns(), 1);
}

#[tokio::test]
async fn unset_env_fails_only_that_upstream() {
    let bad = json!({
        "name": "bad",
        "transport": "http",
        "http": {"url": "http://127.0.0.1:9/mcp", "headers": {"Authorization": "Bearer ${env:UPMGR_NEVER_SET_3}"}}
    });
    let good = FakeUpstream::new("good", echo_tools());
    let (m, conn) = manager(vec![bad, http_up("good")], &[&FakeUpstream::new("bad", echo_tools()), &good]);
    let tools = m.tools().await;
    assert_eq!(tools.iter().map(|(n, t)| (n.as_str(), t.len())).collect::<Vec<_>>(), vec![("bad", 0), ("good", 1)]);
    assert_eq!(conn.connects("bad"), 0);
    let s = state(&m, "bad");
    assert_eq!((s.state, s.restarts), (UpstreamState::Backoff, 1));
    assert_eq!(s.last_error.as_deref(), Some("bad: env var UPMGR_NEVER_SET_3 is not set"));
    let e = m.call("bad", "echo", json!({})).await.unwrap_err();
    assert_eq!(e, UpstreamError::Config("bad: env var UPMGR_NEVER_SET_3 is not set".into()));
    assert_eq!(state(&m, "good").state, UpstreamState::Ready);
    m.call("good", "echo", json!({})).await.unwrap();
}

#[tokio::test]
async fn an_exited_connection_reconnects_on_the_next_call_without_failing_it() {
    let fake = FakeUpstream::new("a", echo_tools());
    let (m, conn) = manager(vec![http_up("a")], &[&fake]);
    m.call("a", "echo", json!({})).await.unwrap();
    fake.set_exited(true);
    let s = state(&m, "a");
    assert_ne!(s.state, UpstreamState::Ready);
    assert_eq!(s.last_error.as_deref(), Some("a: process exited"));
    m.call("a", "echo", json!({"again": true})).await.unwrap();
    assert_eq!(conn.connects("a"), 2);
    let s = state(&m, "a");
    assert_eq!((s.state, s.restarts, s.retry_at, s.last_error), (UpstreamState::Ready, 1, None, None));
    tokio::task::yield_now().await;
    assert_eq!(fake.shutdowns(), 1);
}

#[tokio::test]
async fn repeated_timeouts_recycle_the_connection() {
    let fake = FakeUpstream::new("a", vec![tool("slow", "", json!({}))]);
    let (m, conn) = manager(vec![with(http_up("a"), json!({"timeout_ms": 100}))], &[&fake]);
    m.tools().await;
    fake.set_delay(Duration::from_millis(500));
    for i in 1..=t3_codemode::upstream::manager::MAX_TIMEOUTS {
        assert!(matches!(m.call("a", "slow", json!({})).await, Err(UpstreamError::Timeout(_))));
        let s = state(&m, "a");
        if i < t3_codemode::upstream::manager::MAX_TIMEOUTS {
            assert_eq!((s.state, s.restarts), (UpstreamState::Ready, 0), "after {i}");
        } else {
            assert_eq!((s.state, s.restarts), (UpstreamState::Backoff, 1), "after {i}");
            assert!(s.last_error.unwrap().contains("timed out"));
        }
    }
    assert_eq!(fake.shutdowns(), 1);
    fake.set_delay(Duration::ZERO);
    m.call("a", "slow", json!({})).await.unwrap();
    assert_eq!(conn.connects("a"), 2);
}

#[tokio::test(start_paused = true)]
async fn relisting_a_hung_connection_is_bounded() {
    let fake = FakeUpstream::new("a", echo_tools());
    let (m, _) = manager(vec![with(http_up("a"), json!({"timeout_ms": 600000}))], &[&fake]);
    m.tools().await;
    fake.set_list_delay(Duration::from_secs(3600));
    let started = tokio::time::Instant::now();
    let r = m.refresh().await;
    assert!(r[0].1.as_ref().unwrap_err().contains("tools/list timed out after 15 s"), "{r:?}");
    assert!(started.elapsed() <= Duration::from_secs(16), "{:?}", started.elapsed());
}

#[tokio::test(start_paused = true)]
async fn the_connect_budget_follows_timeout_ms() {
    let fake = FakeUpstream::new("a", echo_tools());
    let (m, _) = slow(vec![with(http_up("a"), json!({"timeout_ms": 120000}))], &fake, Duration::from_secs(17));
    m.call("a", "echo", json!({})).await.unwrap();
    assert_eq!(state(&m, "a").state, UpstreamState::Ready);

    let fake = FakeUpstream::new("a", echo_tools());
    let (m, _) = slow(vec![with(http_up("a"), json!({"timeout_ms": 1000}))], &fake, Duration::from_secs(17));
    let e = m.call("a", "echo", json!({})).await.unwrap_err();
    assert_eq!(e, UpstreamError::Unavailable("a: connect timed out after 15 s".into()));
}

#[tokio::test]
async fn a_connect_keeps_going_when_its_caller_gives_up() {
    let fake = FakeUpstream::new("a", echo_tools());
    let (m, conn) = slow(vec![http_up("a")], &fake, Duration::from_millis(200));
    assert!(tokio::time::timeout(Duration::from_millis(20), m.call("a", "echo", json!({}))).await.is_err());
    tokio::time::sleep(Duration::from_millis(400)).await;
    let s = state(&m, "a");
    assert_eq!((s.state, s.tools), (UpstreamState::Ready, 1));
    m.call("a", "echo", json!({})).await.unwrap();
    assert_eq!(conn.connects("a"), 1);
}

#[tokio::test]
async fn a_connect_that_finishes_after_shutdown_is_closed() {
    let fake = FakeUpstream::new("a", echo_tools());
    let (m, _) = slow(vec![http_up("a")], &fake, Duration::from_millis(200));
    let pending = {
        let m = m.clone();
        tokio::spawn(async move { m.call("a", "echo", json!({})).await })
    };
    tokio::time::sleep(Duration::from_millis(20)).await;
    m.shutdown().await;
    let e = pending.await.unwrap().unwrap_err();
    assert!(e.message().contains("shutting down"), "{e}");
    assert_eq!(fake.shutdowns(), 1);
    assert_ne!(state(&m, "a").state, UpstreamState::Ready);
    assert!(m.call("a", "echo", json!({})).await.unwrap_err().message().contains("shutting down"));
}

#[tokio::test]
async fn tools_for_run_waits_only_briefly_for_a_hung_connect() {
    let fake = FakeUpstream::new("a", echo_tools());
    let (m, _) = slow(vec![http_up("a")], &fake, Duration::from_secs(3600));
    for _ in 0..2 {
        let started = std::time::Instant::now();
        let tools = m.tools_for_run(Duration::from_millis(200)).await;
        assert_eq!(tools, vec![("a".to_string(), vec![])]);
        assert!(started.elapsed() < Duration::from_millis(600), "{:?}", started.elapsed());
    }
    let fast = FakeUpstream::new("b", echo_tools());
    let (m, _) = slow(vec![http_up("b")], &fast, Duration::from_millis(50));
    let tools = m.tools_for_run(Duration::from_secs(2)).await;
    assert_eq!(names(&tools[0].1), vec!["echo"], "a quick first connect is waited for");
}

#[tokio::test]
async fn tool_definitions_and_server_info_are_redacted_and_trimmed() {
    set_env("UPMGR_DEF_TOKEN", "def-secret-5521");
    let up = json!({
        "name": "a",
        "transport": "http",
        "http": {"url": "http://127.0.0.1:9/mcp", "headers": {"Authorization": "Bearer ${env:UPMGR_DEF_TOKEN}"}}
    });
    let fake = FakeUpstream::new(
        "a",
        vec![tool(
            "echo",
            "uses def-secret-5521",
            json!({"type": "object", "properties": {"k": {"description": "def-secret-5521"}}}),
        )],
    );
    fake.set_server(json!({"name": "srv def-secret-5521", "version": "1", "extra": {"big": "x".repeat(5000)}}));
    let (m, _) = manager(vec![up], &[&fake]);
    let tools = m.tools().await;
    let dump = serde_json::to_string(&tools[0].1).unwrap();
    assert!(!dump.contains("def-secret-5521") && dump.contains(REDACTED), "{dump}");
    let server = state(&m, "a").server.unwrap();
    assert_eq!(server, json!({"name": format!("srv {REDACTED}"), "version": "1"}));
}
