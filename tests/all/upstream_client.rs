use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use serde_json::json;
use t3_codemode::upstream::client_http::HttpClient;
use t3_codemode::upstream::client_stdio::StdioClient;
use t3_codemode::upstream::{
    Connector, McpConnector, REDACTED, ResolvedTransport, ResolvedUpstream, Secret, Transport, Upstream, UpstreamError,
    UpstreamState,
};

use crate::http_server::{self, FakeHttp};

const NS: &str = "fk";
const TOKEN: &str = "tok-abcdef-123";

fn fake_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_t3cm-fake-mcp"))
}

fn stdio_spec_with(command: PathBuf, env: &[(&str, &str)], timeout_ms: u64) -> ResolvedUpstream {
    let env: Vec<(String, Secret)> = env.iter().map(|(k, v)| (k.to_string(), Secret::new(v.to_string()))).collect();
    ResolvedUpstream {
        name: NS.into(),
        transport: ResolvedTransport::Stdio { command, args: vec![], env, cwd: None },
        timeout: Duration::from_millis(timeout_ms),
        secrets: vec![Secret::new(TOKEN.into())],
    }
}

fn stdio_spec(env: &[(&str, &str)]) -> ResolvedUpstream {
    stdio_spec_with(fake_bin(), env, 10_000)
}

fn http_spec(url: &str, headers: &[(&str, &str)], secrets: &[&str]) -> ResolvedUpstream {
    ResolvedUpstream {
        name: "web".into(),
        transport: ResolvedTransport::Http {
            url: Secret::new(url.into()),
            headers: headers.iter().map(|(k, v)| (k.to_string(), Secret::new(v.to_string()))).collect(),
        },
        timeout: Duration::from_secs(10),
        secrets: secrets.iter().map(|s| Secret::new(s.to_string())).collect(),
    }
}

async fn stdio(env: &[(&str, &str)]) -> StdioClient {
    StdioClient::connect(&stdio_spec(env)).await.unwrap()
}

fn unavailable<T>(r: Result<T, UpstreamError>) -> String {
    match r {
        Err(UpstreamError::Unavailable(m)) => m,
        Err(other) => panic!("expected Unavailable, got {other:?}"),
        Ok(_) => panic!("expected Unavailable, got Ok"),
    }
}

#[cfg(unix)]
fn alive(pid: u32) -> bool {
    let pid = i32::try_from(pid).unwrap();
    let found = unsafe { libc::kill(pid, 0) } == 0;
    found || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(windows)]
fn alive(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
    use windows_sys::Win32::System::Threading::{GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return false;
        }
        let mut code = 0u32;
        let ok = GetExitCodeProcess(h, &mut code);
        CloseHandle(h);
        ok != 0 && code == STILL_ACTIVE as u32
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn stdio_handshake_server_info_and_paginated_tools() {
    let c = stdio(&[]).await;
    assert!(c.pid().is_some());
    let st = c.status();
    assert_eq!(st.name, NS);
    assert_eq!(st.transport, Transport::Stdio);
    assert_eq!(st.state, UpstreamState::Ready);
    assert_eq!(st.server, Some(json!({"name": "t3cm-fake-mcp", "version": "1.2.3"})));
    let tools = c.list_tools().await.unwrap();
    let names: Vec<_> = tools.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names.len(), 13, "{names:?}");
    assert_eq!(names[0], "client_info");
    assert!(names.contains(&"echo") && names.contains(&"ping_first"));
    assert_eq!(c.status().tools, 13);
    let info = c.call("client_info", json!({})).await.unwrap();
    assert_eq!(info["protocolVersion"], "2025-06-18");
    assert_eq!(info["clientInfo"]["name"], "t3-codemode");
    assert_eq!(info["clientInfo"]["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(info["capabilities"], json!({}));
}

#[tokio::test(flavor = "multi_thread")]
async fn stdio_rejects_an_unsupported_protocol_version() {
    let msg = unavailable(StdioClient::connect(&stdio_spec(&[("FAKE_MCP_VERSION", "1999-01-01")])).await);
    assert!(msg.starts_with("fk: unsupported MCP protocol version"), "{msg}");
    assert!(msg.contains("1999-01-01"), "{msg}");
}

#[tokio::test(flavor = "multi_thread")]
async fn stdio_call_result_shapes() {
    let c = stdio(&[]).await;
    assert_eq!(c.call("echo", json!({"x": 1})).await.unwrap(), json!({"echo": {"x": 1}}));
    assert_eq!(c.call("text_json", json!({})).await.unwrap(), json!({"a": 1, "b": [2]}));
    assert_eq!(c.call("plain", json!({})).await.unwrap(), json!("hello world"));
    let image = c.call("image", json!({})).await.unwrap();
    assert_eq!(
        image,
        json!([{"type": "image", "data": "aGk=", "mimeType": "image/png"}, {"type": "text", "text": "caption"}])
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn stdio_is_error_and_rpc_error_are_tool_errors() {
    let c = stdio(&[]).await;
    assert_eq!(c.call("fail", json!({})).await, Err(UpstreamError::Tool("fk.fail: it broke".into())));
    assert_eq!(c.call("rpc_error", json!({})).await, Err(UpstreamError::Tool("fk.rpc_error: rpc went wrong".into())));
    assert_eq!(c.call("echo", json!({})).await.unwrap(), json!({"echo": {}}));
    assert_eq!(c.status().state, UpstreamState::Ready);
}

#[tokio::test(flavor = "multi_thread")]
async fn stdio_concurrent_calls_resolve_to_their_own_results() {
    let c = stdio(&[]).await;
    let started = Instant::now();
    let calls = (0..10u64).map(|i| {
        let ms = 300 - i * 30;
        let c = &c;
        async move { (ms, c.call("sleep", json!({"ms": ms})).await.unwrap()) }
    });
    let results = futures_util::future::join_all(calls).await;
    for (ms, v) in results {
        assert_eq!(v, json!({"slept": ms}));
    }
    assert!(started.elapsed() < Duration::from_millis(1500), "calls ran serially: {:?}", started.elapsed());
}

#[tokio::test(flavor = "multi_thread")]
async fn stdio_answers_server_requests_mid_call() {
    let c = stdio(&[]).await;
    let v = c.call("ping_first", json!({})).await.unwrap();
    assert_eq!(v["ping"]["result"], json!({}));
    assert_eq!(v["roots"]["error"]["code"], -32601);
}

#[tokio::test(flavor = "multi_thread")]
async fn stdio_list_changed_raises_tools_changed_once() {
    let c = stdio(&[]).await;
    assert!(!c.tools_changed());
    c.call("notify_changed", json!({})).await.unwrap();
    assert!(c.tools_changed());
    assert!(!c.tools_changed());
}

#[tokio::test(flavor = "multi_thread")]
async fn stdio_timeout_keeps_the_connection() {
    let c = StdioClient::connect(&stdio_spec_with(fake_bin(), &[], 300)).await.unwrap();
    let started = Instant::now();
    assert_eq!(
        c.call("sleep", json!({"ms": 3000})).await,
        Err(UpstreamError::Timeout("fk.sleep: timed out after 300 ms".into()))
    );
    assert!(started.elapsed() < Duration::from_millis(2000));
    assert_eq!(c.call("echo", json!({"again": true})).await.unwrap(), json!({"echo": {"again": true}}));
    assert_eq!(c.status().state, UpstreamState::Ready);
}

#[tokio::test(flavor = "multi_thread")]
async fn stdio_crash_fails_now_and_later_calls_fast() {
    let c = stdio(&[]).await;
    let pending = c.call("sleep", json!({"ms": 5000}));
    let crash = c.call("crash", json!({"code": 7}));
    let (slow, crashed) = tokio::join!(pending, crash);
    let msg = unavailable(crashed);
    assert!(msg.starts_with("fk: process exited ("), "{msg}");
    assert!(msg.contains('7'), "{msg}");
    assert!(unavailable(slow).starts_with("fk: process exited"));
    let started = Instant::now();
    assert!(unavailable(c.call("echo", json!({})).await).starts_with("fk: process exited"));
    assert!(unavailable(c.list_tools().await).starts_with("fk: process exited"));
    assert!(started.elapsed() < Duration::from_millis(100), "{:?}", started.elapsed());
    assert_eq!(c.status().state, UpstreamState::Backoff);
    assert!(!c.is_alive());
}

#[tokio::test(flavor = "multi_thread")]
async fn stdio_env_passthrough_and_redaction() {
    unsafe { std::env::set_var("T3CM_CLIENT_TEST_LEAK", "should-not-pass") };
    let c = stdio(&[("FAKE_TOKEN", TOKEN)]).await;
    assert_eq!(c.call("env", json!({"name": "FAKE_TOKEN"})).await.unwrap(), json!({"value": TOKEN}));
    let path = std::env::var("PATH").unwrap();
    assert_eq!(c.call("env", json!({"name": "PATH"})).await.unwrap(), json!({"value": path}));
    assert_eq!(c.call("env", json!({"name": "T3CM_CLIENT_TEST_LEAK"})).await.unwrap(), json!({"value": null}));

    let err = c.call("fail_with_env", json!({"name": "FAKE_TOKEN"})).await.unwrap_err();
    assert_eq!(err, UpstreamError::Tool(format!("fk.fail_with_env: token rejected: {REDACTED}")));
    assert_eq!(c.status().last_error.unwrap(), err.message());
    assert_eq!(c.redact(format!("x {TOKEN} y")), format!("x {REDACTED} y"));

    let msg = unavailable(c.call("crash", json!({"stderr_env": "FAKE_TOKEN"})).await);
    assert!(msg.contains(&format!("crashing with {REDACTED}")), "{msg}");
    assert!(!msg.contains(TOKEN));
    let last = c.status().last_error.unwrap();
    assert!(last.contains(REDACTED) && !last.contains(TOKEN), "{last}");
}

#[tokio::test(flavor = "multi_thread")]
async fn stdio_connect_failure_carries_redacted_stderr() {
    let spec = stdio_spec(&[("FAKE_MCP_DIE_AT_START", &format!("fatal: bad key {TOKEN}"))]);
    let msg = unavailable(StdioClient::connect(&spec).await);
    assert!(msg.starts_with("fk: "), "{msg}");
    assert!(msg.contains(&format!("fatal: bad key {REDACTED}")), "{msg}");
    assert!(!msg.contains(TOKEN));
}

#[tokio::test(flavor = "multi_thread")]
async fn stdio_missing_command_names_it() {
    let spec = stdio_spec_with(PathBuf::from("/nonexistent/t3cm-no-such-server"), &[], 1000);
    let msg = unavailable(StdioClient::connect(&spec).await);
    assert!(msg.contains("/nonexistent/t3cm-no-such-server"), "{msg}");
}

#[tokio::test(flavor = "multi_thread")]
async fn stdio_shutdown_ends_the_child() {
    let c = stdio(&[]).await;
    let pid = c.pid().unwrap();
    assert!(alive(pid));
    let started = Instant::now();
    c.shutdown().await;
    assert!(started.elapsed() < Duration::from_secs(3));
    assert!(!alive(pid));
}

#[tokio::test(flavor = "multi_thread")]
async fn stdio_shutdown_kills_a_child_that_ignores_eof() {
    let c = stdio(&[("FAKE_MCP_IGNORE_EOF", "1")]).await;
    let pid = c.pid().unwrap();
    let started = Instant::now();
    c.shutdown().await;
    let took = started.elapsed();
    assert!(took >= Duration::from_millis(1900) && took < Duration::from_secs(3), "{took:?}");
    assert!(!alive(pid));
}

#[tokio::test(flavor = "multi_thread")]
async fn stdio_drop_kills_the_child() {
    let c = stdio(&[("FAKE_MCP_IGNORE_EOF", "1")]).await;
    let pid = c.pid().unwrap();
    drop(c);
    let deadline = Instant::now() + Duration::from_secs(3);
    while alive(pid) && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(!alive(pid));
}

#[tokio::test(flavor = "multi_thread")]
async fn http_json_session_headers_and_configured_headers() {
    let (fake, url) = http_server::start(FakeHttp::with_token("Bearer s3cret-value")).await;
    let spec = http_spec(&url, &[("Authorization", "Bearer s3cret-value"), ("X-Team", "blue")], &["s3cret-value"]);
    let c = HttpClient::connect(&spec).await.unwrap();
    assert_eq!(c.session_id().as_deref(), Some("sess-1"));
    assert_eq!(c.status().server, Some(json!({"name": "fake-http", "version": "0.9"})));
    assert_eq!(c.status().transport, Transport::Http);

    let init = fake.last("initialize");
    assert!(init.headers.get("mcp-session-id").is_none());
    assert!(init.headers.get("mcp-protocol-version").is_none());
    assert_eq!(init.headers["x-team"], "blue");
    assert_eq!(init.headers["content-type"], "application/json");
    assert_eq!(init.headers["accept"], "application/json, text/event-stream");
    let note = fake.last("notifications/initialized");
    assert_eq!(note.headers["mcp-session-id"], "sess-1");

    let names: Vec<_> = c.list_tools().await.unwrap().into_iter().map(|t| t.name).collect();
    assert_eq!(names, ["echo", "fail", "ping_first", "notify"]);
    assert_eq!(c.call("echo", json!({"q": 1})).await.unwrap(), json!({"echo": {"q": 1}}));
    let call = fake.last("tools/call");
    assert_eq!(call.headers["mcp-session-id"], "sess-1");
    assert_eq!(call.headers["mcp-protocol-version"], "2025-06-18");
    assert_eq!(call.headers["authorization"], "Bearer s3cret-value");
    assert_eq!(call.headers["x-team"], "blue");

    assert_eq!(c.call("fail", json!({})).await, Err(UpstreamError::Tool("web.fail: http tool broke".into())));
    assert_eq!(c.status().last_error.as_deref(), Some("web.fail: http tool broke"));

    c.shutdown().await;
    assert_eq!(*fake.deleted.lock().unwrap(), ["sess-1"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn http_sse_responses_server_ping_and_list_changed() {
    let (fake, url) = http_server::start(FakeHttp::default()).await;
    fake.sse.store(true, Ordering::SeqCst);
    let c = HttpClient::connect(&http_spec(&url, &[], &[])).await.unwrap();
    assert_eq!(c.list_tools().await.unwrap().len(), 4);
    assert_eq!(c.call("echo", json!({"s": "x"})).await.unwrap(), json!({"echo": {"s": "x"}}));
    let v = c.call("ping_first", json!({})).await.unwrap();
    assert_eq!(v["ping"]["result"], json!({}));
    assert_eq!(v["ping"]["id"], "srv-ping");
    assert!(!c.tools_changed());
    assert_eq!(c.call("notify", json!({})).await.unwrap(), json!("ok"));
    assert!(c.tools_changed());
    assert!(!c.tools_changed());

    fake.sse.store(false, Ordering::SeqCst);
    assert_eq!(c.call("notify", json!({})).await.unwrap(), json!("ok"));
    assert!(c.tools_changed());
}

#[tokio::test(flavor = "multi_thread")]
async fn http_404_reinitializes_once_and_resends() {
    let (fake, url) = http_server::start(FakeHttp::default()).await;
    let c = HttpClient::connect(&http_spec(&url, &[], &[])).await.unwrap();
    assert_eq!(c.call("echo", json!({"n": 1})).await.unwrap(), json!({"echo": {"n": 1}}));
    fake.expire_session();
    assert_eq!(c.call("echo", json!({"n": 2})).await.unwrap(), json!({"echo": {"n": 2}}));
    assert_eq!(fake.initializes(), 2);
    assert_eq!(c.session_id().as_deref(), Some("sess-2"));
    assert_eq!(fake.last("tools/call").headers["mcp-session-id"], "sess-2");
}

#[tokio::test(flavor = "multi_thread")]
async fn http_401_is_unavailable_without_the_header_value() {
    let (_fake, url) = http_server::start(FakeHttp::with_token("Bearer right")).await;
    let spec = http_spec(&url, &[("Authorization", "Bearer wrong-secret-xyz")], &["wrong-secret-xyz"]);
    let msg = unavailable(HttpClient::connect(&spec).await);
    assert_eq!(msg, "web: HTTP 401 unauthorized");
}

#[tokio::test(flavor = "multi_thread")]
async fn http_timeout_and_server_errors_keep_the_client() {
    let (_fake, url) = http_server::start(FakeHttp::with_token("Bearer hidden-1234")).await;
    let mut spec = http_spec(&url, &[("Authorization", "Bearer hidden-1234")], &["hidden-1234"]);
    spec.timeout = Duration::from_millis(300);
    let c = HttpClient::connect(&spec).await.unwrap();
    assert_eq!(c.call("slow", json!({})).await, Err(UpstreamError::Timeout("web.slow: timed out after 300 ms".into())));
    let msg = unavailable(c.call("boom", json!({})).await);
    assert_eq!(msg, format!("web: HTTP 500: server fell over holding Bearer {REDACTED}"));
    assert_eq!(c.status().last_error.as_deref(), Some(msg.as_str()));
    assert_eq!(c.call("echo", json!({})).await.unwrap(), json!({"echo": {}}));
}

#[tokio::test(flavor = "multi_thread")]
async fn http_unreachable_is_unavailable() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/mcp", listener.local_addr().unwrap());
    drop(listener);
    let msg = unavailable(HttpClient::connect(&http_spec(&format!("{url}?api_key=k-77a1"), &[], &[])).await);
    assert!(msg.starts_with("web: request failed"), "{msg}");
    assert!(!msg.contains("k-77a1") && !msg.contains("/mcp"), "{msg}");
}

#[tokio::test(flavor = "multi_thread")]
async fn mcp_connector_dispatches_by_transport() {
    let stdio = McpConnector.connect(&stdio_spec(&[])).await.unwrap();
    assert_eq!(stdio.status().transport, Transport::Stdio);
    assert_eq!(stdio.status().server.unwrap()["name"], "t3cm-fake-mcp");
    stdio.shutdown().await;

    let (_fake, url) = http_server::start(FakeHttp::default()).await;
    let http = McpConnector.connect(&http_spec(&url, &[], &[])).await.unwrap();
    assert_eq!(http.status().transport, Transport::Http);
    assert_eq!(http.status().server.unwrap()["name"], "fake-http");
    assert_eq!(http.call("echo", json!({})).await.unwrap(), json!({"echo": {}}));
}

async fn wait_for<F: Fn() -> bool>(f: F) -> bool {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !f() && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    f()
}

#[tokio::test(flavor = "multi_thread")]
async fn stdio_child_gets_only_the_allowlisted_service_environment() {
    unsafe { std::env::set_var("UPCLIENT_OTHER_UPSTREAM_TOKEN", "other-secret-1") };
    let c = stdio(&[("OWN", "mine")]).await;
    let env = |name: &str| c.call("env", json!({"name": name}));
    assert_eq!(env("UPCLIENT_OTHER_UPSTREAM_TOKEN").await.unwrap(), json!({"value": null}));
    assert_eq!(env("OWN").await.unwrap(), json!({"value": "mine"}));
    for name in ["PATH", "HOME", "SYSTEMROOT"].into_iter().filter(|n| std::env::var_os(n).is_some()) {
        assert_eq!(env(name).await.unwrap(), json!({"value": std::env::var(name).unwrap()}), "{name}");
    }
    assert!(t3_codemode::upstream::client_stdio::inherited_env("LC_ALL"));
    assert!(!t3_codemode::upstream::client_stdio::inherited_env("GITHUB_TOKEN"));
}

#[tokio::test(flavor = "multi_thread")]
async fn stdio_timeouts_and_dropped_calls_send_cancellations() {
    let c = StdioClient::connect(&stdio_spec_with(fake_bin(), &[], 300)).await.unwrap();
    assert!(matches!(c.call("sleep", json!({"ms": 3000})).await, Err(UpstreamError::Timeout(_))));
    let dropped = tokio::time::timeout(Duration::from_millis(50), c.call("sleep", json!({"ms": 3000}))).await;
    assert!(dropped.is_err());
    let ids = c.call("cancelled", json!({})).await.unwrap()["ids"].clone();
    assert_eq!(ids.as_array().map(Vec::len), Some(2), "{ids}");
    assert_eq!(c.call("echo", json!({})).await.unwrap(), json!({"echo": {}}));
}

#[tokio::test(flavor = "multi_thread")]
async fn stdio_call_results_keep_text_boundaries_and_types() {
    let c = stdio(&[]).await;
    assert_eq!(c.call("parts", json!({})).await.unwrap(), json!(["first item", "second item"]));
    assert_eq!(c.call("number_text", json!({})).await.unwrap(), json!("123"));
    assert_eq!(c.call("text_json", json!({})).await.unwrap(), json!({"a": 1, "b": [2]}));
}

#[tokio::test(flavor = "multi_thread")]
async fn stdio_a_repeated_cursor_is_an_error() {
    let c = stdio(&[("FAKE_MCP_REPEAT_CURSOR", "1")]).await;
    let msg = unavailable(c.list_tools().await);
    assert!(msg.contains("repeated the cursor"), "{msg}");
}

#[tokio::test(flavor = "multi_thread")]
async fn stdio_an_oversized_message_fails_the_connection() {
    let c = stdio(&[]).await;
    let msg = unavailable(c.call("flood", json!({"bytes": 17 * 1024 * 1024})).await);
    assert!(msg.contains("over 16777216 bytes"), "{msg}");
    assert!(!c.is_alive());
}

#[tokio::test(flavor = "multi_thread")]
async fn stdio_a_flood_of_server_requests_is_survived() {
    let c = stdio(&[]).await;
    assert_eq!(c.call("ping_flood", json!({"n": 5000})).await.unwrap(), json!("flooded"));
    assert_eq!(c.call("echo", json!({"after": 1})).await.unwrap(), json!({"echo": {"after": 1}}));
}

#[cfg(unix)]
fn wrapper() -> (PathBuf, Vec<String>) {
    (PathBuf::from("/bin/sh"), vec!["-c".into(), "\"$0\"; :".into(), fake_bin().display().to_string()])
}

#[cfg(windows)]
fn wrapper() -> (PathBuf, Vec<String>) {
    (PathBuf::from("cmd.exe"), vec!["/c".into(), fake_bin().display().to_string()])
}

#[tokio::test(flavor = "multi_thread")]
async fn stdio_shutdown_ends_a_wrapped_server_too() {
    let (command, args) = wrapper();
    let spec = ResolvedUpstream {
        name: NS.into(),
        transport: ResolvedTransport::Stdio {
            command,
            args,
            env: vec![("FAKE_MCP_IGNORE_EOF".into(), Secret::new("1".into()))],
            cwd: None,
        },
        timeout: Duration::from_secs(10),
        secrets: vec![],
    };
    let c = StdioClient::connect(&spec).await.unwrap();
    let shell = c.pid().unwrap();
    let server = c.call("pid", json!({})).await.unwrap()["pid"].as_u64().unwrap() as u32;
    assert_ne!(server, shell);
    assert!(alive(server));
    c.shutdown().await;
    assert!(wait_for(|| !alive(server)).await, "the wrapped server outlived shutdown");
    assert!(!alive(shell));
}

#[tokio::test(flavor = "multi_thread")]
async fn http_cancels_a_timed_out_request() {
    let (fake, url) = http_server::start(FakeHttp::default()).await;
    let mut spec = http_spec(&url, &[], &[]);
    spec.timeout = Duration::from_millis(300);
    let c = HttpClient::connect(&spec).await.unwrap();
    assert!(matches!(c.call("slow", json!({})).await, Err(UpstreamError::Timeout(_))));
    let seen = || fake.seen.lock().unwrap().iter().any(|s| s.rpc.as_deref() == Some("notifications/cancelled"));
    assert!(wait_for(seen).await);
}

#[tokio::test(flavor = "multi_thread")]
async fn http_redirects_are_not_followed() {
    let (fake, url) = http_server::start(FakeHttp::with_token("Bearer redirect-secret")).await;
    let moved = url.replace("/mcp", "/moved");
    let spec = http_spec(&moved, &[("X-Api-Key", "redirect-secret")], &["redirect-secret"]);
    let msg = unavailable(HttpClient::connect(&spec).await);
    assert!(msg.starts_with("web: HTTP 307"), "{msg}");
    assert!(fake.seen.lock().unwrap().iter().all(|s| s.rpc.as_deref() == Some("moved")));
}

#[tokio::test(flavor = "multi_thread")]
async fn http_an_oversized_response_is_refused() {
    let (fake, url) = http_server::start(FakeHttp::default()).await;
    let c = HttpClient::connect(&http_spec(&url, &[], &[])).await.unwrap();
    let msg = unavailable(c.call("huge", json!({})).await);
    assert!(msg.contains("over 16777216 bytes"), "{msg}");
    fake.sse.store(true, Ordering::SeqCst);
    let msg = unavailable(c.call("huge", json!({})).await);
    assert!(msg.contains("over 16777216 bytes"), "{msg}");
}
