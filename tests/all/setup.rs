use std::collections::BTreeMap;
use std::io::Write;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use serde_json::{Value, json};
use t3_codemode::fake::provider;
use t3_codemode::server;
use t3_codemode::setup::{self, WriteOpts};
use t3_codemode::skills::Bundle;
use t3_codemode::slots::{Folders, realpath};
use t3_codemode::steps::{Env, RULE_END, RULE_START};

use crate::common::H;

fn serve_health(body: String) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        while let Ok((mut stream, _)) = listener.accept() {
            let mut buf = [0u8; 1024];
            let _ = std::io::Read::read(&mut stream, &mut buf);
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(resp.as_bytes());
        }
    });
    port
}

const SLOT_STEPS: &[&str] = &["slot:review", "slot:passes", "slot:credentials", "slot:delivery", "slot:models"];

fn rule_text() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("skills/t3-workflow/rules/report-filed-issues.md");
    std::fs::read_to_string(path).unwrap().replace("\r\n", "\n")
}

fn codex_block() -> String {
    format!("{RULE_START}\n{}\n{RULE_END}\n", rule_text().trim_end())
}

struct SetupEnv {
    dir: tempfile::TempDir,
    home: PathBuf,
    repo: PathBuf,
    bin: PathBuf,
    claude: PathBuf,
    codex: PathBuf,
    config: PathBuf,
    closed: TcpListener,
    port: u16,
}

impl SetupEnv {
    fn new() -> SetupEnv {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let repo = dir.path().join("repo");
        let bin = dir.path().join("bin");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&repo).unwrap();
        std::fs::create_dir_all(&bin).unwrap();
        // Put git in bin if found on PATH so git_root works even with a minimal PATH.
        if let Some(path_var) = std::env::var_os("PATH") {
            for d in std::env::split_paths(&path_var) {
                let g = d.join(t3_codemode::platform::exe_name("git"));
                if g.is_file() {
                    #[cfg(unix)]
                    let _ = std::os::unix::fs::symlink(&g, bin.join("git"));
                    #[cfg(windows)]
                    let _ = std::fs::copy(&g, bin.join("git.exe"));
                    break;
                }
            }
        }
        let closed = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = closed.local_addr().unwrap().port();
        let claude = home.join(".claude");
        let codex = home.join(".codex");
        let config = home.join(".config").join("t3-workflow").join("config.json");
        SetupEnv { dir, home, repo, bin, claude, codex, config, closed, port }
    }

    fn skill(&self, base: &Path, name: &str, inline: bool) -> PathBuf {
        let d = base.join(name);
        std::fs::create_dir_all(&d).unwrap();
        let extra = if inline { "disable-model-invocation: true\n" } else { "" };
        let p = d.join("SKILL.md");
        std::fs::write(&p, format!("---\nname: {name}\ndescription: test\n{extra}---\n\nBody.\n")).unwrap();
        p
    }

    fn plugin_skill(&self, plugin: &str, name: &str) -> (PathBuf, PathBuf) {
        let root = self.claude.join("plugins/cache/acme").join(plugin).join("1.0.0");
        let p = self.skill(&root.join("skills"), name, false);
        (root, p)
    }

    fn install_plugins(&self, items: &[(&str, &Path)]) {
        let mut plugins = serde_json::Map::new();
        for (name, root) in items {
            plugins.insert(format!("{name}@acme"), json!([{"scope": "user", "installPath": root.to_string_lossy()}]));
        }
        let path = self.claude.join("plugins/installed_plugins.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, json!({"version": 2, "plugins": plugins}).to_string()).unwrap();
    }

    fn both_sol_loops(&self) -> (PathBuf, PathBuf) {
        let (root, plugin_path) = self.plugin_skill("cbrip", "sol-loop");
        self.install_plugins(&[("cbrip", &root)]);
        let bare = self.skill(&self.home.join(".agents/skills"), "sol-loop", false);
        (plugin_path, bare)
    }

    fn write_config(&self, val: &Value) {
        std::fs::create_dir_all(self.config.parent().unwrap()).unwrap();
        std::fs::write(&self.config, serde_json::to_string(val).unwrap()).unwrap();
    }

    fn write_raw_config(&self, text: &str) {
        std::fs::create_dir_all(self.config.parent().unwrap()).unwrap();
        std::fs::write(&self.config, text).unwrap();
    }

    fn agents_md(&self, body: &str) {
        std::fs::write(self.repo.join("AGENTS.md"), format!("# Repo\n\n## Agent workflow\n\n{body}")).unwrap();
    }

    fn cmd(&self, args: &[&str], stdin: Option<&str>, extra_env: &[(&str, &str)]) -> Output {
        let mut c = Command::new(env!("CARGO_BIN_EXE_t3cm"));
        c.args(args)
            .current_dir(self.dir.path())
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("PATH", &self.bin)
            .env("T3CM_PORT", self.port.to_string())
            .env("T3CM_STATE_DIR", self.dir.path().join("state"))
            .env("T3CM_NO_UPDATE_CHECK", "1")
            .env_remove("T3CODE_HOME")
            .env_remove("T3CM_T3_ORIGIN")
            .env_remove("T3CM_T3_BIN")
            .env_remove("XDG_CONFIG_HOME")
            .env_remove("CLAUDE_CONFIG_DIR")
            .env_remove("CODEX_HOME")
            .env_remove("T3WF_CONFIG");
        for (k, v) in extra_env {
            c.env(k, v);
        }
        c.stdout(Stdio::piped()).stderr(Stdio::piped());
        if let Some(input) = stdin {
            c.stdin(Stdio::piped());
            let mut child = c.spawn().unwrap();
            child.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
            child.wait_with_output().unwrap()
        } else {
            c.stdin(Stdio::null()).output().unwrap()
        }
    }

    fn setup_raw(&self, sub: &str, args: &[&str], stdin: Option<&str>, extra_env: &[(&str, &str)]) -> Output {
        let repo_str = self.repo.to_string_lossy().into_owned();
        let mut full = vec!["setup", sub];
        full.extend_from_slice(args);
        if sub != "rule" {
            full.push("--repo");
            full.push(&repo_str);
        }
        self.cmd(&full, stdin, extra_env)
    }

    fn detect(&self, args: &[&str]) -> Value {
        let out = self.setup_raw("detect", args, None, &[]);
        assert!(out.status.success(), "detect failed: {}", String::from_utf8_lossy(&out.stderr));
        serde_json::from_slice(&out.stdout).unwrap()
    }

    fn write_ok(&self, cfg: &Value, args: &[&str]) -> Output {
        let out = self.setup_raw("write", args, Some(&cfg.to_string()), &[]);
        assert!(out.status.success(), "write failed: {}", String::from_utf8_lossy(&out.stderr));
        out
    }

    fn write_err(&self, cfg: &Value, args: &[&str]) -> Output {
        let out = self.setup_raw("write", args, Some(&cfg.to_string()), &[]);
        assert!(!out.status.success(), "write unexpectedly succeeded: {}", String::from_utf8_lossy(&out.stdout));
        out
    }

    fn rule_ok(&self, extra_env: &[(&str, &str)]) -> Output {
        let out = self.setup_raw("rule", &[], None, extra_env);
        assert!(out.status.success(), "rule failed: {}", String::from_utf8_lossy(&out.stderr));
        out
    }

    fn rule_err(&self, extra_env: &[(&str, &str)]) -> Output {
        let out = self.setup_raw("rule", &[], None, extra_env);
        assert!(!out.status.success(), "rule unexpectedly succeeded");
        out
    }

    fn raw_saved(&self) -> Value {
        serde_json::from_str(&std::fs::read_to_string(&self.config).unwrap()).unwrap()
    }

    fn saved(&self) -> Value {
        let mut obj = self.raw_saved().as_object().unwrap().clone();
        obj.remove("steps");
        obj.remove("writtenBy");
        Value::Object(obj)
    }

    fn missing_steps(&self) -> Vec<String> {
        self.detect(&[])["steps"]["missing"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["id"].as_str().unwrap().to_string())
            .collect()
    }

    fn claude_rule(&self) -> PathBuf {
        self.claude.join("rules/t3-workflow-filed-issues.md")
    }
}

// ---------------------------------------------------------------- Detect

#[test]
fn detect_offers_a_namespaced_and_a_bare_sol_loop() {
    let s = SetupEnv::new();
    let (plugin_path, bare_path) = s.both_sol_loops();
    let review = s.detect(&[])["slots"]["review"].clone();
    let found: BTreeMap<String, String> = review["found"]["sol-loop"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| (c["name"].as_str().unwrap().into(), c["path"].as_str().unwrap().into()))
        .collect();
    assert_eq!(
        found,
        BTreeMap::from([
            ("cbrip:sol-loop".into(), plugin_path.to_string_lossy().into_owned()),
            ("sol-loop".into(), bare_path.to_string_lossy().into_owned()),
        ])
    );
    assert!(review.get("suggest").is_none());
}

#[test]
fn detect_suggests_tstacks_sol_loop_when_none_is_installed() {
    let s = SetupEnv::new();
    let review = s.detect(&[])["slots"]["review"].clone();
    assert_eq!(review["found"], json!({}));
    assert_eq!(review["missing"], json!(["sol-loop"]));
    assert!(review["suggest"].as_str().unwrap().contains("https://github.com/t3dotgg/tstack"));
}

#[test]
fn detect_marks_a_slash_only_pass_inline() {
    let s = SetupEnv::new();
    s.skill(&s.claude.join("skills"), "unslop", true);
    s.skill(&s.claude.join("skills"), "pr", false);
    let passes = s.detect(&[])["slots"]["passes"].clone();
    assert_eq!(passes["found"]["unslop"][0]["invoke"], "inline");
    assert_eq!(passes["found"]["pr"][0]["invoke"], "skill");
}

#[test]
fn detect_includes_voice_skills_listed_in_the_session_as_pass_candidates() {
    let s = SetupEnv::new();
    let (root, _) = s.plugin_skill("writers", "ann-voice");
    s.install_plugins(&[("writers", &root)]);
    let passes = s.detect(&["--skills", "writers:ann-voice, pr"])["slots"]["passes"].clone();
    let names: Vec<&str> =
        passes["found"]["writers:ann-voice"].as_array().unwrap().iter().map(|c| c["name"].as_str().unwrap()).collect();
    assert_eq!(names, vec!["writers:ann-voice"]);
}

#[test]
fn detect_reports_a_missing_full_name_with_what_is_installed_instead() {
    let s = SetupEnv::new();
    let bare = s.skill(&s.home.join(".agents/skills"), "sol-loop", false);
    s.write_config(&json!({"review": "cbrip:sol-loop"}));
    let review = s.detect(&[])["slots"]["review"].clone();
    assert!(review["found"].get("cbrip:sol-loop").is_none());
    assert!(review["missing"].as_array().unwrap().contains(&json!("cbrip:sol-loop")));
    assert_eq!(review["installedInstead"]["cbrip:sol-loop"][0]["path"], bare.to_string_lossy().as_ref());
    assert_eq!(review["found"]["sol-loop"][0]["path"], bare.to_string_lossy().as_ref());
}

#[test]
fn detect_resolves_a_name_that_looks_like_an_option() {
    let s = SetupEnv::new();
    s.write_config(&json!({"review": "-review"}));
    let review = s.detect(&[])["slots"]["review"].clone();
    assert!(review["missing"].as_array().unwrap().contains(&json!("-review")));
}

#[test]
fn detect_keeps_repo_problems_apart_from_the_user_config() {
    let s = SetupEnv::new();
    s.write_config(&json!({"passes": []}));
    s.agents_md("- Delivery: carrier-pigeon\n");
    let config = s.detect(&[])["config"].clone();
    assert_eq!(config["problems"], json!([]));
    assert!(
        config["repoProblems"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p.as_str().is_some_and(|t| t.contains("carrier-pigeon")))
    );
}

#[test]
fn detect_searches_configured_skills_too() {
    let s = SetupEnv::new();
    s.skill(&s.claude.join("skills"), "ship", false);
    s.write_config(&json!({"delivery": {"mode": "tag-deploy", "skill": "ship"}}));
    let delivery = s.detect(&[])["slots"]["delivery"].clone();
    assert_eq!(delivery["configured"], json!(["ship"]));
    assert!(delivery["found"].get("ship").is_some());
    assert_eq!(delivery["missing"], json!(["deploy"]));
}

#[tokio::test(flavor = "multi_thread")]
async fn detect_lists_enabled_providers_and_their_models_and_summary() {
    let h = H::with_tools();
    let mut claude_disabled = provider("claudeAgent", "claudeAgent", &["claude-opus-5-5"]);
    claude_disabled["constraints"] = json!(["Provider is disabled"]);
    let mut codex = provider("codex", "codex", &["gpt-6.2-sol"]);
    codex["displayName"] = json!("Codex");
    codex["models"] =
        json!([{"id": "gpt-6.2-sol", "options": [{"id": "reasoningEffort", "values": ["low", "medium", "high"]}]}]);
    let gemini = json!({
        "providerInstanceId": "gemini",
        "driverKind": "gemini",
        "displayName": "Gemini",
        "constraints": [],
        "models": [
            {"id": "g-1", "options": [{"id": "thinking_level", "values": ["low", "high"], "default": "low"}]},
            {"id": "g-2"}
        ]
    });
    h.fake.set_providers(vec![claude_disabled, codex, gemini]);
    h.app.refresh_catalog().await.unwrap();
    let (listener, addr) = server::bind(0).await.unwrap();
    let app = h.app.clone();
    tokio::spawn(async move {
        let _ = server::serve(app, listener, std::future::pending()).await;
    });

    let mut s = SetupEnv::new();
    s.port = addr.port();
    let want_config = s.config.to_string_lossy().into_owned();
    let d = tokio::task::spawn_blocking(move || s.detect(&["--json"])).await.unwrap();
    let p = &d["providers"];
    assert_eq!(p["disabled"], json!(["claudeAgent"]));
    let enabled: Vec<&str> = p["enabled"].as_array().unwrap().iter().map(|e| e["provider"].as_str().unwrap()).collect();
    assert_eq!(enabled, vec!["codex", "gemini"]);
    assert_eq!(p["enabled"][0]["models"], json!(["gpt-6.2-sol (effort low|medium|high)"]));
    assert_eq!(p["enabled"][1]["models"], json!(["g-1 (effort low|high; default low)", "g-2"]));
    assert_eq!(d["config"]["exists"], false);
    assert_eq!(d["config"]["path"], want_config);
    assert!(d["summary"].as_array().unwrap().contains(&json!(
        "review: none (no reviewer installed; suggest sol-loop from https://github.com/t3dotgg/tstack) [default]"
    )));
}

#[tokio::test(flavor = "multi_thread")]
async fn detect_queries_t3_directly_when_t3cm_service_is_stopped() {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use axum::extract::{Form, State};
    use axum::http::{HeaderMap, StatusCode};
    use axum::response::{IntoResponse, Response};
    use axum::routing::post;
    use axum::{Json, Router};
    use base64::Engine;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use sha2::{Digest, Sha256};

    #[derive(Default)]
    struct DirectT3 {
        challenge: String,
        issued: Vec<String>,
    }
    type St = Arc<Mutex<DirectT3>>;

    async fn reg() -> Json<Value> {
        Json(json!({"client_id": "client-1"}))
    }
    async fn dec(State(st): State<St>, Json(body): Json<Value>) -> Json<Value> {
        let a = &body["authorization"];
        st.lock().unwrap().challenge = a["code_challenge"].as_str().unwrap().into();
        Json(
            json!({"redirectTo": format!("http://127.0.0.1/callback?code=AUTH&state={}", a["state"].as_str().unwrap())}),
        )
    }
    async fn tok(State(st): State<St>, Form(f): Form<HashMap<String, String>>) -> Response {
        let mut g = st.lock().unwrap();
        let want = URL_SAFE_NO_PAD.encode(Sha256::digest(f["code_verifier"].as_bytes()));
        if want != g.challenge || f["code"] != "AUTH" {
            return (StatusCode::BAD_REQUEST, Json(json!({"error": "invalid_grant"}))).into_response();
        }
        g.issued.push("tok-1".into());
        Json(json!({"access_token": "tok-1", "token_type": "Bearer", "expires_in": 3600})).into_response()
    }
    async fn mcp(State(st): State<St>, headers: HeaderMap, Json(body): Json<Value>) -> Response {
        let auth = headers.get("authorization").and_then(|v| v.to_str().ok()).unwrap_or("");
        if !st.lock().unwrap().issued.iter().any(|t| auth == format!("Bearer {t}")) {
            return StatusCode::UNAUTHORIZED.into_response();
        }
        let Some(id) = body.get("id").cloned() else { return StatusCode::ACCEPTED.into_response() };
        match body["method"].as_str().unwrap_or_default() {
            "initialize" => (
                [("mcp-session-id", "sess-1"), ("content-type", "application/json")],
                json!({"jsonrpc": "2.0", "id": id, "result": {"protocolVersion": "2025-06-18"}}).to_string(),
            )
                .into_response(),
            "tools/call" => {
                let caps = json!({
                    "providers": [
                        {"providerInstanceId": "codex", "displayName": "Codex", "constraints": [],
                         "models": [{"id": "gpt-6.2-sol", "options": [{"id": "reasoningEffort", "values": ["low", "high"]}]}]}
                    ]
                });
                let payload = json!({"jsonrpc": "2.0", "id": id, "result": {"structuredContent": caps, "content": []}});
                ([("content-type", "text/event-stream")], format!("event: message\ndata: {payload}\n\n"))
                    .into_response()
            }
            _ => StatusCode::BAD_REQUEST.into_response(),
        }
    }

    let st = St::default();
    let app = Router::new()
        .route("/oauth/mcp/register", post(reg))
        .route("/oauth/mcp/decision", post(dec))
        .route("/oauth/mcp/token", post(tok))
        .route("/mcp", post(mcp))
        .with_state(st);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let t3_addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let s = SetupEnv::new();
    let t3_home = s.dir.path().join("t3-home");
    std::fs::create_dir_all(t3_home.join("userdata")).unwrap();
    std::fs::write(
        t3_home.join("userdata/server-runtime.json"),
        json!({"origin": format!("http://{t3_addr}"), "port": t3_addr.port()}).to_string(),
    )
    .unwrap();
    let fake_t3_bin = env!("CARGO_BIN_EXE_t3cm-fake-mcp").to_string();
    let t3_home_str = t3_home.to_string_lossy().into_owned();
    let t3_log = s.dir.path().join("t3.log").to_string_lossy().into_owned();
    let (out_runtime, out_origin) = tokio::task::spawn_blocking(move || {
        let r1 = s.setup_raw(
            "detect",
            &[],
            None,
            &[("T3CODE_HOME", &t3_home_str), ("T3CM_T3_BIN", &fake_t3_bin), ("FAKE_T3_LOG", &t3_log)],
        );
        // Also verify explicit T3CM_T3_ORIGIN without any server-runtime.json.
        let empty_home = s.dir.path().join("no-runtime").to_string_lossy().into_owned();
        let origin = format!("http://{t3_addr}");
        let r2 = s.setup_raw(
            "detect",
            &[],
            None,
            &[
                ("T3CODE_HOME", &empty_home),
                ("T3CM_T3_ORIGIN", &origin),
                ("T3CM_T3_BIN", &fake_t3_bin),
                ("FAKE_T3_LOG", &t3_log),
            ],
        );
        (r1, r2)
    })
    .await
    .unwrap();
    for out in [out_runtime, out_origin] {
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        let d: Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(d["t3cm"]["running"], false, "t3cm service is stopped");
        assert_eq!(d["providers"]["enabled"][0]["provider"], "codex", "{d}");
        assert_eq!(d["providers"]["enabled"][0]["models"], json!(["gpt-6.2-sol (effort low|high)"]));
    }
}

#[test]
fn detect_names_the_error_when_t3_does_not_answer() {
    let s = SetupEnv::new();
    let err = s.detect(&[])["providers"]["error"].as_str().unwrap().to_string();
    assert!(err.contains("T3 unreachable"), "{err}");
}

#[test]
fn detect_platform_reports_systemd_user_units() {
    let s = SetupEnv::new();
    let want_sys = if cfg!(target_os = "macos") { "darwin" } else { std::env::consts::OS };
    let plat = s.detect(&[])["platform"].clone();
    assert_eq!(plat["system"], want_sys);
    assert_eq!(plat["systemdUser"], false, "no systemctl on minimal PATH");
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::PermissionsExt;
        let fake_systemctl = s.bin.join("systemctl");
        std::fs::write(&fake_systemctl, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&fake_systemctl, std::fs::Permissions::from_mode(0o755)).unwrap();
        let plat = s.detect(&[])["platform"].clone();
        assert_eq!(plat["systemdUser"], true);
        assert_eq!(plat["t3cmSupported"], true);
    }
}

// ---------------------------------------------------------------- T3cm

#[test]
fn t3cm_missing_and_stopped() {
    let s = SetupEnv::new();
    let t = s.detect(&[])["t3cm"].clone();
    assert_eq!(t["binary"], Value::Null);
    assert_eq!(t["running"], false);
    assert_eq!(t["connected"], false);
    assert_eq!(t["url"], format!("http://127.0.0.1:{}/", s.port));
    drop(s.closed);
}

#[test]
fn t3cm_running_and_connected() {
    let mut s = SetupEnv::new();
    let fake_bin = s.bin.join(t3_codemode::platform::exe_name("t3cm"));
    std::fs::write(&fake_bin, "#!/bin/sh\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&fake_bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    s.port = serve_health(json!({"version": "9.9.9", "t3": {"connected": true}}).to_string());
    let t = s.detect(&["--no-t3"])["t3cm"].clone();
    assert_eq!(t["binary"], fake_bin.to_string_lossy().as_ref());
    assert_eq!(t["running"], true);
    assert_eq!(t["connected"], true);
    assert_eq!(t["version"], "9.9.9");
}

#[test]
fn t3cm_another_service_on_the_port_is_not_t3cm() {
    for body in [json!([]), json!({}), json!({"t3": {"connected": "false"}})] {
        let mut s = SetupEnv::new();
        s.port = serve_health(body.to_string());
        let t = s.detect(&["--no-t3"])["t3cm"].clone();
        assert_eq!(t["running"], false, "{body}");
        assert_eq!(t["error"], "/healthz did not answer like t3cm", "{body}");
    }
}

#[test]
fn t3cm_probe_skips_the_proxy() {
    let mut s = SetupEnv::new();
    s.port = serve_health(json!({"t3": {"connected": true}}).to_string());
    let out = s.setup_raw(
        "detect",
        &["--no-t3"],
        None,
        &[
            ("http_proxy", "http://127.0.0.1:9"),
            ("HTTP_PROXY", "http://127.0.0.1:9"),
            ("no_proxy", ""),
            ("NO_PROXY", ""),
        ],
    );
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["t3cm"]["running"], true);
}

#[test]
fn t3cm_bad_port_setting_is_reported() {
    let s = SetupEnv::new();
    let out = s.setup_raw("detect", &["--no-t3"], None, &[("T3CM_PORT", "abc")]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["t3cm"]["running"], false);
    assert_eq!(v["t3cm"]["error"], "T3CM_PORT='abc' is not a port number");
}

// ---------------------------------------------------------------- Write

#[test]
fn write_records_the_namespaced_choice() {
    let s = SetupEnv::new();
    let _ = s.both_sol_loops();
    let out = s.write_ok(&json!({"review": "cbrip:sol-loop"}), &["--no-t3"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(s.saved(), json!({"review": "cbrip:sol-loop", "version": 1}));
    assert!(stdout.contains(&format!("wrote {}", realpath(&s.config).display())), "{stdout}");
    assert!(stdout.contains("review: cbrip:sol-loop (Skill tool) [user]"), "{stdout}");
}

#[test]
fn write_records_the_bare_choice() {
    let s = SetupEnv::new();
    let (_, bare_path) = s.both_sol_loops();
    let out = s.write_ok(&json!({"review": "sol-loop"}), &["--no-t3", "--skills", "cbrip:sol-loop"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("review: sol-loop (Skill tool) [user]"), "{stdout}");
    let folders = Folders::under(&s.home, None);
    let enabled = t3_codemode::slots::Enabled::load(&folders.claude, &s.repo);
    let resolved = t3_codemode::slots::Resolved::build(&t3_codemode::slots::Inputs {
        folders: &folders,
        root: &s.repo,
        listed: &["cbrip:sol-loop".into()],
        enabled: &enabled,
    });
    assert_eq!(resolved.review["skill"]["status"], "ok");
    assert_eq!(resolved.review["skill"]["path"], bare_path.to_string_lossy().as_ref());
}

#[test]
fn write_refuses_a_bare_name_that_matches_several_skills() {
    let s = SetupEnv::new();
    let (cbrip, _) = s.plugin_skill("cbrip", "sol-loop");
    let (tools, _) = s.plugin_skill("tools", "sol-loop");
    s.install_plugins(&[("cbrip", &cbrip), ("tools", &tools)]);
    s.write_config(&json!({"review": null}));
    let out = s.write_err(&json!({"review": "sol-loop"}), &["--no-t3"]);
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("review: sol-loop matches"), "{stderr}");
    assert!(stderr.contains("cbrip:sol-loop") && stderr.contains("tools:sol-loop"), "{stderr}");
    assert!(stderr.contains("record one full name"), "{stderr}");
    assert_eq!(s.saved(), json!({"review": null}));
}

#[test]
fn write_warns_on_a_kept_ambiguous_name_without_blocking_another_slot() {
    let s = SetupEnv::new();
    let (cbrip, _) = s.plugin_skill("cbrip", "sol-loop");
    let (tools, _) = s.plugin_skill("tools", "sol-loop");
    s.install_plugins(&[("cbrip", &cbrip), ("tools", &tools)]);
    s.write_config(&json!({"review": "sol-loop"}));
    let out = s.write_ok(&json!({"models": {"worker": {"model": "claude-opus-5-5"}}}), &["--no-t3"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("warn: review: sol-loop matches"), "{stdout}");
    assert_eq!(s.saved()["models"], json!({"worker": {"model": "claude-opus-5-5"}}));
}

#[test]
fn write_records_a_slash_only_skill_inline() {
    let s = SetupEnv::new();
    let path = s.skill(&s.claude.join("skills"), "unslop", true);
    let out = s.write_ok(&json!({"passes": ["unslop"]}), &["--no-t3"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(s.saved()["passes"], json!(["unslop"]));
    assert!(
        stdout.contains(&format!("passes: unslop (inline: read {} and apply it) [user]", path.display())),
        "{stdout}"
    );
}

#[test]
fn write_merges_into_the_current_config_and_unsets() {
    let s = SetupEnv::new();
    s.write_config(&json!({
        "version": 1,
        "passes": ["pr"],
        "delivery": {"mode": "merge"},
        "models": {"ticket": {"model": "claude-opus-5-5", "effort": "max"}}
    }));
    s.write_ok(
        &json!({"models": {"worker": {"provider": "codex", "model": "gpt-6.2-sol", "effort": "high"}}}),
        &["--no-t3", "--unset", "passes"],
    );
    assert_eq!(
        s.saved(),
        json!({
            "version": 1,
            "delivery": {"mode": "merge"},
            "models": {
                "ticket": {"model": "claude-opus-5-5", "effort": "max"},
                "worker": {"provider": "codex", "model": "gpt-6.2-sol", "effort": "high"}
            }
        })
    );
    s.write_ok(&json!({}), &["--no-t3", "--unset", "models.ticket"]);
    let saved = s.saved();
    let keys: Vec<&String> = saved["models"].as_object().unwrap().keys().collect();
    assert_eq!(keys, vec!["worker"]);
}

#[test]
fn write_unset_rejects_a_target_that_is_not_a_slot_or_model_role() {
    let s = SetupEnv::new();
    s.write_config(&json!({"version": 1, "passes": ["pr"]}));
    let out = s.write_err(&json!({}), &["--no-t3", "--unset", "passes.worker"]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("--unset passes.worker: name a slot"), "{stderr}");
    assert_eq!(s.saved()["passes"], json!(["pr"]));
}

#[test]
fn write_does_not_blame_a_kept_pass_for_a_review_that_is_unset() {
    let s = SetupEnv::new();
    let (cbrip, _) = s.plugin_skill("cbrip", "sol-loop");
    let (tools, _) = s.plugin_skill("tools", "sol-loop");
    s.install_plugins(&[("cbrip", &cbrip), ("tools", &tools)]);
    s.write_config(&json!({"passes": ["sol-loop"]}));
    let out = s.write_ok(&json!({"review": "sol-loop"}), &["--no-t3", "--unset", "review"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("warn: passes[0]: sol-loop matches"), "{stdout}");
}

#[test]
fn write_drops_a_slot_set_and_unset_in_one_write() {
    let s = SetupEnv::new();
    s.write_ok(&json!({"passes": true}), &["--no-t3", "--unset", "passes"]);
    assert!(s.saved().get("passes").is_none());
}

#[test]
fn write_requires_a_deploy_skill_for_tag_deploy() {
    let s = SetupEnv::new();
    let out = s.write_err(&json!({"delivery": {"mode": "tag-deploy"}}), &["--no-t3"]);
    assert!(String::from_utf8_lossy(&out.stderr).contains("delivery: tag-deploy needs a deploy skill"));
    s.write_ok(&json!({"delivery": {"mode": "tag-deploy", "skill": "ship"}}), &["--no-t3"]);
    assert_eq!(s.saved()["delivery"], json!({"mode": "tag-deploy", "skill": "ship"}));
}

#[test]
fn write_drops_tag_deploy_when_also_unset() {
    let s = SetupEnv::new();
    s.write_ok(&json!({"delivery": {"mode": "tag-deploy"}}), &["--no-t3", "--unset", "delivery"]);
    assert!(s.saved().get("delivery").is_none());
}

#[test]
fn write_requires_a_model_for_a_model_role() {
    let s = SetupEnv::new();
    s.write_config(&json!({"models": {"worker": {"model": "claude-opus-5-5"}}}));
    let out = s.write_err(&json!({"models": {"worker": {"effort": "high"}}}), &["--no-t3"]);
    assert!(String::from_utf8_lossy(&out.stderr).contains("models.worker: names no model"));
}

#[test]
fn write_replaces_a_model_role_with_a_pool() {
    let s = SetupEnv::new();
    s.write_config(&json!({"models": {
        "worker": {"model": "claude-opus-5-5", "effort": "high"},
        "ticket": {"model": "claude-opus-5-5"}
    }}));
    let out = s.write_ok(&json!({"models": {"worker": {"pool": "opus"}}}), &["--no-t3"]);
    assert_eq!(s.saved()["models"], json!({"worker": {"pool": "opus"}, "ticket": {"model": "claude-opus-5-5"}}));
    assert!(String::from_utf8_lossy(&out.stdout).contains("model worker: pool opus"));
    let out = s.write_err(&json!({"models": {"ticket": {"pool": "opus", "model": "claude-opus-5-5"}}}), &["--no-t3"]);
    assert!(String::from_utf8_lossy(&out.stderr).contains("models.ticket: a pool sets the model and effort"));
}

#[test]
fn write_leaves_another_config_version_alone() {
    let s = SetupEnv::new();
    s.write_config(&json!({"version": 2, "review": "sol-loop"}));
    let out = s.write_err(&json!({"passes": []}), &["--no-t3"]);
    assert!(String::from_utf8_lossy(&out.stderr).contains("is config version 2; pass --replace"));
    assert_eq!(s.saved()["version"], 2);
}

#[cfg(unix)]
#[test]
fn write_follows_a_symlinked_config() {
    let s = SetupEnv::new();
    let target = s.dir.path().join("dotfiles/t3wf.json");
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::fs::write(&target, json!({"passes": ["pr"]}).to_string()).unwrap();
    std::fs::create_dir_all(s.config.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&target, &s.config).unwrap();
    s.write_ok(&json!({"review": null}), &["--no-t3"]);
    assert!(std::fs::symlink_metadata(&s.config).unwrap().file_type().is_symlink());
    assert_eq!(s.saved(), json!({"version": 1, "passes": ["pr"], "review": null}));
    let target_json: Value = serde_json::from_str(&std::fs::read_to_string(&target).unwrap()).unwrap();
    assert!(target_json.get("steps").is_some());
}

#[test]
fn write_fails_cleanly_when_config_dir_cannot_be_made() {
    let s = SetupEnv::new();
    std::fs::create_dir_all(s.config.parent().unwrap().parent().unwrap()).unwrap();
    std::fs::write(s.config.parent().unwrap(), "").unwrap();
    let out = s.write_err(&json!({"passes": []}), &["--no-t3", "--replace"]);
    assert!(String::from_utf8_lossy(&out.stderr).contains("not written"));
}

#[test]
fn write_leaves_no_staged_file_on_failed_save() {
    let s = SetupEnv::new();
    std::fs::create_dir_all(&s.config).unwrap();
    std::fs::write(s.config.join("occupied"), "").unwrap();
    let out = s.write_err(&json!({"passes": []}), &["--no-t3", "--replace"]);
    assert!(String::from_utf8_lossy(&out.stderr).contains("not written"));
    let mut entries: Vec<String> = std::fs::read_dir(s.config.parent().unwrap())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    entries.sort();
    assert_eq!(entries, vec!["config.json"]);
}

#[test]
fn write_review_none_turns_review_off() {
    let s = SetupEnv::new();
    let _ = s.both_sol_loops();
    let out = s.write_ok(&json!({"review": null}), &["--no-t3"]);
    assert_eq!(s.saved()["review"], Value::Null);
    assert!(String::from_utf8_lossy(&out.stdout).contains("review: none (none configured) [user]"));
}

#[test]
fn write_does_not_write_a_bad_setting() {
    let s = SetupEnv::new();
    let out = s.write_err(&json!({"delivery": {"mode": "ship"}}), &["--no-t3"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("'ship' is not one of merge, tag-deploy"));
    assert!(!s.config.exists());
}

#[test]
fn write_warns_for_a_skill_not_yet_installed() {
    let s = SetupEnv::new();
    let out = s.write_ok(&json!({"review": "sol-loop"}), &["--no-t3"]);
    assert_eq!(s.saved()["review"], "sol-loop");
    assert!(String::from_utf8_lossy(&out.stdout).contains("warn: review: sol-loop is not installed yet"));
}

#[test]
fn write_requires_replace_for_a_broken_config() {
    let s = SetupEnv::new();
    s.write_raw_config("{not json");
    let out = s.write_err(&json!({"passes": []}), &["--no-t3"]);
    assert!(String::from_utf8_lossy(&out.stderr).contains("--replace"));
    s.write_ok(&json!({"passes": []}), &["--no-t3", "--replace"]);
    assert_eq!(s.saved(), json!({"passes": [], "version": 1}));
}

// ---------------------------------------------------------------- Rule

#[test]
fn rule_text_tells_every_thread_to_report_with_wf_issue() {
    let rule = rule_text();
    for text in ["wf.issue(", "`gh issue create`", "GitHub MCP", "every thread", "`t3cm exec -`"] {
        assert!(rule.contains(text), "missing {text}");
    }
}

#[test]
fn rule_installs_for_claude_code() {
    let s = SetupEnv::new();
    std::fs::create_dir_all(&s.claude).unwrap();
    let out = s.rule_ok(&[]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(std::fs::read_to_string(s.claude_rule()).unwrap(), rule_text());
    assert!(stdout.contains(&format!("claude: wrote {}", realpath(&s.claude_rule()).display())), "{stdout}");
    assert!(stdout.contains("codex: skipped"), "{stdout}");
}

#[test]
fn rule_follows_claude_config_dir() {
    let s = SetupEnv::new();
    let other = s.dir.path().join("claude-2");
    std::fs::create_dir_all(&other).unwrap();
    s.rule_ok(&[("CLAUDE_CONFIG_DIR", &other.to_string_lossy())]);
    assert_eq!(std::fs::read_to_string(other.join("rules/t3-workflow-filed-issues.md")).unwrap(), rule_text());
    assert!(!s.claude_rule().exists());
}

#[cfg(unix)]
#[test]
fn rule_writes_through_a_symlinked_rules_folder() {
    let s = SetupEnv::new();
    let shared = s.dir.path().join("shared-rules");
    std::fs::create_dir_all(&shared).unwrap();
    std::fs::create_dir_all(&s.claude).unwrap();
    std::os::unix::fs::symlink(&shared, s.claude.join("rules")).unwrap();
    s.rule_ok(&[]);
    assert!(std::fs::symlink_metadata(s.claude.join("rules")).unwrap().file_type().is_symlink());
    assert_eq!(std::fs::read_to_string(shared.join("t3-workflow-filed-issues.md")).unwrap(), rule_text());
}

#[test]
fn rule_second_run_changes_nothing() {
    let s = SetupEnv::new();
    std::fs::create_dir_all(&s.claude).unwrap();
    s.rule_ok(&[]);
    let out = s.rule_ok(&[]);
    assert!(String::from_utf8_lossy(&out.stdout).contains("is current"));
    assert_eq!(std::fs::read_to_string(s.claude_rule()).unwrap(), rule_text());
}

#[test]
fn rule_rewrites_a_stale_rule() {
    let s = SetupEnv::new();
    std::fs::create_dir_all(s.claude_rule().parent().unwrap()).unwrap();
    std::fs::write(s.claude_rule(), "# Old rule\n").unwrap();
    assert_eq!(s.detect(&[])["rule"]["claude"]["status"], "stale");
    s.rule_ok(&[]);
    assert_eq!(std::fs::read_to_string(s.claude_rule()).unwrap(), rule_text());
    assert_eq!(s.detect(&[])["rule"]["claude"]["status"], "current");
}

#[test]
fn rule_appends_a_block_to_codex_agents_md_and_keeps_the_rest() {
    let s = SetupEnv::new();
    std::fs::create_dir_all(&s.codex).unwrap();
    let agents = s.codex.join("AGENTS.md");
    std::fs::write(&agents, "# Mine\n\nKeep this.\n").unwrap();
    s.rule_ok(&[]);
    assert_eq!(std::fs::read_to_string(&agents).unwrap(), format!("# Mine\n\nKeep this.\n\n{}", codex_block()));
    s.rule_ok(&[]);
    assert_eq!(std::fs::read_to_string(&agents).unwrap().matches(RULE_START).count(), 1);
}

#[test]
fn rule_replaces_a_stale_codex_block_in_place() {
    let s = SetupEnv::new();
    std::fs::create_dir_all(&s.codex).unwrap();
    let agents = s.codex.join("AGENTS.md");
    std::fs::write(&agents, format!("# Mine\n\n{RULE_START}\nold\n{RULE_END}\n\n## After\n\nStays.\n")).unwrap();
    assert_eq!(s.detect(&[])["rule"]["codex"]["status"], "stale");
    s.rule_ok(&[]);
    assert_eq!(std::fs::read_to_string(&agents).unwrap(), format!("# Mine\n\n{}\n## After\n\nStays.\n", codex_block()));
}

#[test]
fn rule_leaves_an_unpaired_codex_marker_for_the_user() {
    let s = SetupEnv::new();
    std::fs::create_dir_all(&s.codex).unwrap();
    let agents = s.codex.join("AGENTS.md");
    for bad in [
        format!("# Mine\n\n{RULE_START}\nold\n\n## After\n"),
        format!("old\n{RULE_END}\n"),
        format!("{RULE_END}\n{RULE_START}\n"),
    ] {
        std::fs::write(&agents, &bad).unwrap();
        let out = s.rule_err(&[]);
        assert!(String::from_utf8_lossy(&out.stderr).contains("without its pair"));
        assert_eq!(std::fs::read_to_string(&agents).unwrap(), bad);
    }
}

#[test]
fn rule_writes_codex_override_file_when_it_exists() {
    let s = SetupEnv::new();
    std::fs::create_dir_all(&s.codex).unwrap();
    let over = s.codex.join("AGENTS.override.md");
    std::fs::write(&over, "# Override\n").unwrap();
    s.rule_ok(&[]);
    assert!(std::fs::read_to_string(&over).unwrap().contains(RULE_START));
    assert!(!s.codex.join("AGENTS.md").exists());
}

#[test]
fn rule_detect_reports_each_agent() {
    let s = SetupEnv::new();
    std::fs::create_dir_all(&s.claude).unwrap();
    let rule = s.detect(&[])["rule"].clone();
    assert_eq!(rule["claude"], json!({"status": "missing", "path": realpath(&s.claude_rule()).to_string_lossy()}));
    assert_eq!(rule["codex"]["status"], "no agent");
}

#[test]
fn rule_unreadable_target_leaves_the_rest_of_detect() {
    let s = SetupEnv::new();
    std::fs::create_dir_all(s.claude_rule()).unwrap();
    let out = s.detect(&[]);
    assert!(out["rule"]["error"].as_str().unwrap().contains("cannot read"));
    assert!(out.get("summary").is_some());
}

#[test]
fn rule_errors_when_no_agent_folder_exists() {
    let s = SetupEnv::new();
    let out = s.rule_err(&[]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("neither a Claude Code nor a Codex config folder"));
}

// ---------------------------------------------------------------- Steps

#[test]
fn steps_first_write_records_only_the_slots_it_covers() {
    let s = SetupEnv::new();
    s.write_ok(&json!({"review": null}), &["--no-t3", "--asked", "passes,delivery"]);
    assert_eq!(s.raw_saved()["steps"], json!(["slot:review", "slot:passes", "slot:delivery"]));
    assert_eq!(s.raw_saved()["writtenBy"], env!("CARGO_PKG_VERSION"));
    assert_eq!(s.missing_steps(), vec!["slot:credentials", "slot:models"]);
}

#[test]
fn steps_config_from_before_steps_counts_every_slot_as_asked() {
    let s = SetupEnv::new();
    s.write_config(&json!({"version": 1, "passes": ["pr"]}));
    assert_eq!(s.missing_steps(), Vec::<String>::new());
    s.write_ok(&json!({"review": null}), &["--no-t3"]);
    assert_eq!(s.raw_saved()["steps"], json!(SLOT_STEPS));
}

#[test]
fn steps_reports_a_slot_recorded_by_no_run_as_missing() {
    let s = SetupEnv::new();
    s.write_config(&json!({"version": 1, "steps": &SLOT_STEPS[..4]}));
    assert_eq!(
        s.detect(&[])["steps"]["missing"],
        json!([{"id": "slot:models", "kind": "interview", "slot": "models"}])
    );
    s.write_ok(&json!({}), &["--no-t3", "--asked", "models"]);
    assert_eq!(s.raw_saved()["steps"], json!(SLOT_STEPS));
}

#[test]
fn steps_no_config_misses_every_slot_and_a_missing_rule() {
    let s = SetupEnv::new();
    std::fs::create_dir_all(&s.claude).unwrap();
    let mut want: Vec<&str> = SLOT_STEPS.to_vec();
    want.push("rule:filed-issues");
    assert_eq!(s.missing_steps(), want);
}

#[test]
fn steps_rule_step_is_recorded_in_an_existing_config() {
    let s = SetupEnv::new();
    s.write_config(&json!({"version": 1, "passes": ["pr"]}));
    std::fs::create_dir_all(&s.claude).unwrap();
    assert_eq!(s.missing_steps(), vec!["rule:filed-issues"]);
    let out = s.rule_ok(&[]);
    assert!(String::from_utf8_lossy(&out.stdout).contains("recorded rule:filed-issues"));
    let mut want: Vec<&str> = SLOT_STEPS.to_vec();
    want.push("rule:filed-issues");
    assert_eq!(s.raw_saved()["steps"], json!(want));
    assert_eq!(s.saved(), json!({"version": 1, "passes": ["pr"]}));
    assert_eq!(s.missing_steps(), Vec::<String>::new());
}

#[test]
fn steps_rule_does_not_create_a_config() {
    let s = SetupEnv::new();
    std::fs::create_dir_all(&s.claude).unwrap();
    s.rule_ok(&[]);
    assert!(!s.config.exists());
}

#[test]
fn steps_write_records_a_rule_that_is_already_current() {
    let s = SetupEnv::new();
    std::fs::create_dir_all(&s.claude).unwrap();
    s.rule_ok(&[]);
    s.write_ok(&json!({"review": null}), &["--no-t3"]);
    assert_eq!(s.raw_saved()["steps"], json!(["slot:review", "rule:filed-issues"]));
}

#[test]
fn steps_stdin_cannot_set_the_record() {
    let s = SetupEnv::new();
    let out = s.write_err(&json!({"steps": []}), &["--no-t3"]);
    assert!(String::from_utf8_lossy(&out.stderr).contains("setup records those itself"));
}

#[test]
fn steps_asked_must_name_a_slot() {
    let s = SetupEnv::new();
    let out = s.write_err(&json!({}), &["--no-t3", "--asked", "reviewer"]);
    assert!(String::from_utf8_lossy(&out.stderr).contains("--asked reviewer"));
}

#[test]
fn steps_stamped_bundle_records_its_release() {
    let s = SetupEnv::new();
    let embedded = Bundle::embedded();
    let files = [
        (t3_codemode::steps::CATALOG.to_string(), embedded.raw(t3_codemode::steps::CATALOG).unwrap().to_vec()),
        (t3_codemode::steps::RULE.to_string(), embedded.raw(t3_codemode::steps::RULE).unwrap().to_vec()),
    ];
    let custom = Bundle::new("9.8.7", files);
    let folders = Folders::under(&s.home, None);
    let env = Env::under(&s.home, None);
    setup::write_config(
        &folders,
        &env,
        &custom,
        "{}",
        &WriteOpts {
            repo: s.repo.clone(),
            listed: vec![],
            unset: vec![],
            replace: false,
            asked: vec![],
            repo_config: false,
        },
    )
    .unwrap();
    assert_eq!(s.raw_saved()["writtenBy"], "9.8.7");
}

#[test]
fn steps_doctor_accepts_and_validates_the_record() {
    let s = SetupEnv::new();
    s.write_config(&json!({"version": 1, "writtenBy": "0.6.0", "steps": SLOT_STEPS}));
    let out = s.cmd(&["doctor", "--no-t3", "--json", "--repo", &s.repo.to_string_lossy()], None, &[]);
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["settings"]["problems"], json!([]));

    s.write_config(&json!({"version": 1, "steps": "slot:review"}));
    let out = s.cmd(&["doctor", "--no-t3", "--json", "--repo", &s.repo.to_string_lossy()], None, &[]);
    assert_eq!(out.status.code(), Some(1));
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(
        v["problems"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p.as_str().is_some_and(|t| t.contains("steps: must be a list")))
    );
}

#[test]
fn steps_malformed_claude_headers_does_not_abort_steps_status() {
    let s = SetupEnv::new();
    s.write_config(&json!({"version": 1, "steps": SLOT_STEPS}));
    let claude_json = s.home.join(".claude.json");
    std::fs::write(&claude_json, json!({"mcpServers": {"t3cm": {"type": "http", "headers": 1}}}).to_string()).unwrap();
    assert!(s.missing_steps().contains(&"mcp:config-dir-header".to_string()));
    std::fs::write(
        &claude_json,
        json!({"mcpServers": {"t3cm": {"type": "http", "headers": {"X-T3-Config-Dir": null}}}}).to_string(),
    )
    .unwrap();
    assert!(s.missing_steps().contains(&"mcp:config-dir-header".to_string()));
    std::fs::write(
        &claude_json,
        json!({"mcpServers": {"t3cm": {"type": "http", "headers": {"X-T3-Config-Dir": "${CLAUDE_CONFIG_DIR:-}"}}}})
            .to_string(),
    )
    .unwrap();
    assert!(!s.missing_steps().contains(&"mcp:config-dir-header".to_string()));
}

// ---------------------------------------------------------------- Composable delivery & .t3-workflow.json

#[test]
fn setup_detect_and_write_handle_delivery_steps_and_repo_config() {
    let s = SetupEnv::new();
    s.skill(&s.claude.join("skills"), "deploy", false);
    s.skill(&s.claude.join("skills"), "lms-publish", false);

    let lms_delivery = json!({
        "delivery": {
            "steps": [
                {"id": "merge",   "kind": "builtin", "gate": "always", "scope": "pr"},
                {"id": "deploy",  "skill": "deploy", "gate": "chain",  "scope": "ticket", "watch": true},
                {"id": "publish", "skill": "lms-publish", "gate": "always", "scope": "post-handoff", "paths": ["training/**"]}
            ],
            "closeAfter": "ticket"
        }
    });

    // Reject --asked with --repo-config
    let out = s.write_err(&lms_delivery, &["--no-t3", "--repo-config", "--asked", "delivery"]);
    assert!(String::from_utf8_lossy(&out.stderr).contains("--asked only applies to the user config"));

    // Write lms delivery steps to repo .t3-workflow.json
    let out = s.write_ok(&lms_delivery, &["--no-t3", "--repo-config"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let repo_cfg = s.repo.join(".t3-workflow.json");
    assert!(repo_cfg.exists());
    assert!(!s.config.exists(), "writing --repo-config must not touch user config");
    assert!(stdout.contains(&format!("wrote {}", realpath(&repo_cfg).display())), "{stdout}");
    assert!(stdout.contains("publish via lms-publish"), "{stdout}");
    assert!(stdout.contains("[.t3-workflow.json]"), "{stdout}");

    let saved_repo: Value = serde_json::from_str(&std::fs::read_to_string(&repo_cfg).unwrap()).unwrap();
    assert_eq!(saved_repo["version"], 1);
    assert_eq!(saved_repo["delivery"], lms_delivery["delivery"]);
    assert!(saved_repo.get("steps").is_none(), "repo config must not record per-machine setup steps");
    assert!(saved_repo.get("writtenBy").is_none());

    // detect reads .t3-workflow.json and searches every configured delivery step skill
    let detected = s.detect(&["--no-t3"]);
    assert_eq!(detected["config"]["repoExists"], true);
    assert_eq!(detected["config"]["repoProblems"], json!([]));
    assert_eq!(detected["slots"]["delivery"]["configured"], json!(["deploy", "lms-publish"]));
    assert!(detected["slots"]["delivery"]["found"].get("deploy").is_some());
    assert!(detected["slots"]["delivery"]["found"].get("lms-publish").is_some());

    // Write composable delivery steps to user config as well
    let user_steps = json!({
        "delivery": {
            "steps": [
                {"id": "merge", "kind": "builtin", "gate": "always", "scope": "pr"},
                {"id": "verify", "skill": "not-installed-yet", "gate": "never", "scope": "pr"}
            ],
            "closeAfter": "pr"
        }
    });
    let out = s.write_ok(&user_steps, &["--no-t3"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("warn: delivery.verify: not-installed-yet is not installed yet"), "{stdout}");
    assert_eq!(s.saved()["delivery"], user_steps["delivery"]);

    // Reject ambiguous bare skill in a newly written delivery step
    let (p1, _) = s.plugin_skill("one", "ship-it");
    let (p2, _) = s.plugin_skill("two", "ship-it");
    s.install_plugins(&[("one", &p1), ("two", &p2)]);
    let bad_ambig = json!({
        "delivery": {
            "steps": [
                {"id": "merge", "kind": "builtin", "gate": "always"},
                {"id": "deploy", "skill": "ship-it", "gate": "chain"}
            ]
        }
    });
    let out = s.write_err(&bad_ambig, &["--no-t3"]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("delivery.deploy: ship-it matches"), "{stderr}");
    assert!(stderr.contains("record one full name"), "{stderr}");

    // Reject invalid delivery.steps shape on write
    let bad_shape = json!({
        "delivery": {
            "steps": [
                {"id": "deploy", "skill": "deploy", "gate": "always"},
                {"id": "merge", "kind": "builtin", "gate": "always"}
            ]
        }
    });
    let out = s.write_err(&bad_shape, &["--no-t3", "--repo-config"]);
    assert!(String::from_utf8_lossy(&out.stderr).contains("builtin merge step must be first"));
}
