use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::{Value, json};
use t3_codemode::skills::Bundle;
use t3_codemode::steps::{self, Env, Kind, RULE_END, RULE_START};

const SLOT_STEPS: [&str; 5] = ["slot:review", "slot:passes", "slot:credentials", "slot:delivery", "slot:models"];

fn t3cm(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_t3cm"))
        .args(args)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("CLAUDE_CONFIG_DIR")
        .env_remove("CODEX_HOME")
        .env_remove("T3WF_CONFIG")
        .env_remove("T3CODE_HOME")
        .output()
        .unwrap()
}

fn text(out: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr))
}

fn config_path(home: &Path) -> PathBuf {
    home.join(".config").join("t3-workflow").join("config.json")
}

fn write_config(home: &Path, cfg: &Value) {
    let path = config_path(home);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, serde_json::to_string(cfg).unwrap()).unwrap();
}

fn read_config(home: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(config_path(home)).unwrap()).unwrap()
}

fn claude_rule(home: &Path) -> PathBuf {
    home.join(".claude").join("rules").join("t3-workflow-filed-issues.md")
}

fn rule_text() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("skills/t3-workflow/rules/report-filed-issues.md");
    std::fs::read_to_string(path).unwrap().replace("\r\n", "\n")
}

#[test]
fn update_installs_the_rule_and_records_it_in_a_config_that_predates_it() {
    let home = tempfile::tempdir().unwrap();
    let out = t3cm(home.path(), &["skills", "install"]);
    assert!(out.status.success(), "{}", text(&out));
    std::fs::create_dir(home.path().join(".claude")).unwrap();
    write_config(home.path(), &json!({"version": 1, "passes": ["pr"]}));

    let out = t3cm(home.path(), &["skills", "status"]);
    assert!(!out.status.success(), "a missing rule fails status: {}", text(&out));
    assert!(text(&out).contains("fix: t3cm skills update"), "{}", text(&out));
    let out = t3cm(home.path(), &["skills", "status", "--json"]);
    assert!(!out.status.success());
    assert_eq!(serde_json::from_slice::<Value>(&out.stdout).unwrap()["current"], false);

    let out = t3cm(home.path(), &["skills", "update"]);
    assert!(out.status.success(), "{}", text(&out));
    let shown = text(&out);
    assert!(shown.contains("wrote the filed-issue rule to"), "{shown}");
    assert!(shown.contains("recorded setup steps in"), "{shown}");
    assert!(!shown.contains("run setup-t3-workflow"), "{shown}");
    assert_eq!(std::fs::read_to_string(claude_rule(home.path())).unwrap(), rule_text());
    let mut want: Vec<&str> = SLOT_STEPS.to_vec();
    want.push("rule:filed-issues");
    assert_eq!(
        read_config(home.path()),
        json!({"version": 1, "passes": ["pr"], "writtenBy": env!("CARGO_PKG_VERSION"), "steps": want})
    );

    let out = t3cm(home.path(), &["skills", "status", "--json"]);
    assert!(out.status.success(), "{}", text(&out));
    let st: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(st["current"], true);
    assert_eq!(st["steps"]["missing"], json!([]));
    assert_eq!(st["steps"]["configState"], "recorded");
    assert_eq!(st["steps"]["rule"][0]["status"], "current");
    assert_eq!(st["steps"]["rule"][1]["status"], "no agent");
}

#[test]
fn install_writes_a_codex_block_and_keeps_the_rest_of_agents_md() {
    let home = tempfile::tempdir().unwrap();
    let codex = home.path().join("codex-home");
    std::fs::create_dir(&codex).unwrap();
    std::fs::write(codex.join("AGENTS.md"), "# Mine\r\n\r\nKeep this.\r\n").unwrap();
    let env = Env { codex: codex.clone(), ..Env::under(home.path(), None) };
    let applied = steps::apply(&env, &Bundle::embedded()).unwrap();
    assert_eq!(applied.wrote.len(), 1);
    let block = format!("{RULE_START}\n{}\n{RULE_END}\n", rule_text().trim_end());
    assert_eq!(std::fs::read_to_string(codex.join("AGENTS.md")).unwrap(), format!("# Mine\n\nKeep this.\n\n{block}"));
    let again = steps::apply(&env, &Bundle::embedded()).unwrap();
    assert!(again.wrote.is_empty(), "a second run changes nothing");
    assert!(!env.config.exists(), "t3cm does not create a config");
}

#[test]
fn a_symlinked_rules_folder_is_written_through() {
    let home = tempfile::tempdir().unwrap();
    let shared = home.path().join("shared-rules");
    std::fs::create_dir_all(&shared).unwrap();
    std::fs::create_dir(home.path().join(".claude")).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&shared, home.path().join(".claude").join("rules")).unwrap();
    #[cfg(windows)]
    if std::os::windows::fs::symlink_dir(&shared, home.path().join(".claude").join("rules")).is_err() {
        return;
    }
    steps::apply(&Env::under(home.path(), None), &Bundle::embedded()).unwrap();
    assert!(std::fs::symlink_metadata(home.path().join(".claude").join("rules")).unwrap().file_type().is_symlink());
    assert_eq!(std::fs::read_to_string(shared.join("t3-workflow-filed-issues.md")).unwrap(), rule_text());
}

fn release_with(steps_json: Value) -> Bundle {
    let rule = rule_text();
    let catalog = serde_json::to_string(&steps_json).unwrap();
    Bundle::new(
        "9.9.9",
        [
            ("setup-t3-workflow/steps.json".to_string(), catalog.into_bytes()),
            ("t3-workflow/rules/report-filed-issues.md".to_string(), rule.into_bytes()),
        ],
    )
}

#[test]
fn a_new_interview_step_is_named_as_a_setup_run() {
    let home = tempfile::tempdir().unwrap();
    let env = Env::under(home.path(), None);
    let bundle = release_with(json!({"version": 1, "steps": [
        {"id": "slot:review", "since": "0.3.0", "kind": "interview", "slot": "review", "legacy": true},
        {"id": "slot:widgets", "since": "9.9.9", "kind": "interview", "slot": "widgets"},
        {"id": "rule:filed-issues", "since": "0.5.0", "kind": "auto"}
    ]}));
    write_config(home.path(), &json!({"version": 1}));
    let applied = steps::apply(&env, &bundle).unwrap();
    let missing = &applied.steps.missing;
    assert_eq!(missing.len(), 1, "{missing:?}");
    assert_eq!(missing[0].id, "slot:widgets");
    assert_eq!(missing[0].kind, Kind::Interview);
    assert_eq!(missing[0].fix, "run setup-t3-workflow: new slot widgets");
    assert!(steps::describe_applied(&applied).contains("run setup-t3-workflow: new slot widgets\n"));
    assert!(applied.wrote.is_empty(), "no agent folder, so no rule to write");
    assert_eq!(read_config(home.path()), json!({"version": 1}), "nothing new to record");

    write_config(home.path(), &json!({"version": 1, "steps": ["slot:review", "slot:widgets", "slot:later"]}));
    std::fs::create_dir(home.path().join(".claude")).unwrap();
    steps::apply(&env, &bundle).unwrap();
    assert_eq!(
        read_config(home.path())["steps"],
        json!(["slot:review", "slot:widgets", "rule:filed-issues", "slot:later"]),
        "a step from a newer release is kept"
    );
    assert!(steps::missing_steps(&env, &bundle).unwrap().is_empty());
}

#[test]
fn without_a_config_update_points_at_the_setup_skill() {
    let home = tempfile::tempdir().unwrap();
    assert!(t3cm(home.path(), &["skills", "install"]).status.success());
    let out = t3cm(home.path(), &["skills", "update"]);
    assert!(out.status.success(), "{}", text(&out));
    assert!(text(&out).contains("no T3 workflow config at"), "{}", text(&out));
    assert!(text(&out).contains("run setup-t3-workflow"), "{}", text(&out));
    assert!(!config_path(home.path()).exists());
    let out = t3cm(home.path(), &["skills", "status"]);
    assert!(out.status.success(), "interview steps alone do not fail status: {}", text(&out));
}

#[test]
fn a_broken_config_is_left_alone() {
    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir(home.path().join(".claude")).unwrap();
    for bad in ["{not json", r#"{"version": 2}"#, r#"{"version": 1, "steps": "slot:review"}"#] {
        let path = config_path(home.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, bad).unwrap();
        let env = Env::under(home.path(), None);
        let applied = steps::apply(&env, &Bundle::embedded()).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), bad);
        assert!(applied.steps.problem.is_some(), "{bad}");
        assert!(steps::describe_applied(&applied).contains("is ignored"), "{bad}");
        assert!(applied.steps.missing.is_empty(), "{bad}: {:?}", applied.steps.missing);
    }
    assert_eq!(std::fs::read_to_string(claude_rule(home.path())).unwrap(), rule_text());
}

#[test]
fn a_stale_claude_rule_is_rewritten() {
    let home = tempfile::tempdir().unwrap();
    let path = claude_rule(home.path());
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "# Old rule\n").unwrap();
    let env = Env::under(home.path(), None);
    let st = steps::check(&env, &Bundle::embedded()).unwrap();
    assert!(st.needs_update());
    assert!(steps::describe_steps(&st).contains("is out of date"));
    steps::apply(&env, &Bundle::embedded()).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), rule_text());
}

#[cfg(unix)]
#[test]
fn recording_keeps_the_config_file_mode() {
    use std::os::unix::fs::PermissionsExt;
    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir(home.path().join(".claude")).unwrap();
    write_config(home.path(), &json!({"version": 1}));
    let path = config_path(home.path());
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let applied = steps::apply(&Env::under(home.path(), None), &Bundle::embedded()).unwrap();
    assert_eq!(applied.newly_recorded, ["rule:filed-issues"]);
    assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
}

#[test]
fn an_unpaired_codex_marker_stops_before_any_write() {
    let home = tempfile::tempdir().unwrap();
    let codex = home.path().join(".codex");
    std::fs::create_dir(&codex).unwrap();
    for bad in [
        format!("# Mine\n\n{RULE_START}\nold\n\n## After\n\nStays.\n"),
        format!("# Mine\n\nold\n{RULE_END}\n"),
        format!("{RULE_END}\n{RULE_START}\n"),
    ] {
        std::fs::write(codex.join("AGENTS.md"), &bad).unwrap();
        let err = steps::apply(&Env::under(home.path(), None), &Bundle::embedded()).unwrap_err();
        assert!(format!("{err:#}").contains("without its pair"), "{err:#}");
        assert_eq!(std::fs::read_to_string(codex.join("AGENTS.md")).unwrap(), bad);
    }
}

#[cfg(unix)]
fn fake_claude(dir: &Path, claude_json: &Path, fail_add: bool) -> PathBuf {
    let log = dir.join("claude.log");
    let script = dir.join("claude");
    let add = if fail_add {
        "exit 1".to_string()
    } else {
        format!(r#"printf '{{"mcpServers":{{"t3cm":%s}}}}' "$6" > '{}'"#, claude_json.display())
    };
    t3_codemode::fake::write_executable(
        &script,
        &format!(
            "#!/bin/sh\necho \"$2 ${{CLAUDE_CONFIG_DIR:-unset}}\" >> '{}'\ncase \"$2\" in\n  add-json) {add} ;;\nesac\n",
            log.display()
        ),
    );
    script
}

#[cfg(unix)]
fn old_entry(home: &Path) {
    let entry =
        json!({"type": "http", "url": "http://127.0.0.1:47731/mcp", "headers": {"X-T3-Thread": "${T3CODE_THREAD_ID}"}});
    std::fs::write(home.join(".claude.json"), json!({"mcpServers": {"t3cm": entry}, "projects": {}}).to_string())
        .unwrap();
}

#[cfg(unix)]
#[test]
fn update_reregisters_a_claude_entry_that_sends_no_config_folder() {
    let home = tempfile::tempdir().unwrap();
    let bin = tempfile::tempdir().unwrap();
    old_entry(home.path());
    write_config(home.path(), &json!({"version": 1, "passes": ["pr"]}));
    let env = Env {
        claude_cmd: fake_claude(bin.path(), &home.path().join(".claude.json"), false),
        ..Env::under(home.path(), None)
    };
    let before = steps::check(&env, &Bundle::embedded()).unwrap();
    assert!(before.missing.iter().any(|m| m.id == steps::HEADER_STEP && m.fix == "t3cm skills update"));
    assert!(steps::describe_steps(&before).contains("does not send X-T3-Config-Dir"));

    let applied = steps::apply(&env, &Bundle::embedded()).unwrap();
    assert_eq!(applied.registered, vec![home.path().join(".claude.json")], "{:?}", applied.notes);
    assert!(steps::describe_applied(&applied).contains("re-registered Claude Code's t3cm server"));
    let log = std::fs::read_to_string(bin.path().join("claude.log")).unwrap();
    assert_eq!(log, "remove unset\nadd-json unset\n");
    let saved: Value =
        serde_json::from_str(&std::fs::read_to_string(home.path().join(".claude.json")).unwrap()).unwrap();
    assert_eq!(
        saved["mcpServers"]["t3cm"]["headers"],
        json!({"X-T3-Thread": "${T3CODE_THREAD_ID}", "X-T3-Config-Dir": "${CLAUDE_CONFIG_DIR:-}"})
    );
    assert!(read_config(home.path())["steps"].as_array().unwrap().contains(&json!(steps::HEADER_STEP)));
    assert!(!steps::check(&env, &Bundle::embedded()).unwrap().missing.iter().any(|m| m.id == steps::HEADER_STEP));
}

#[cfg(unix)]
#[test]
fn a_registration_that_cannot_be_restored_fails_and_names_the_add_command() {
    let home = tempfile::tempdir().unwrap();
    let bin = tempfile::tempdir().unwrap();
    old_entry(home.path());
    let own = home.path().join(".claude-2");
    let env = Env {
        claude_cmd: fake_claude(bin.path(), &own.join(".claude.json"), true),
        claude_json: own.join(".claude.json"),
        claude_config_dir: Some(own.clone()),
        claude: own.clone(),
        ..Env::under(home.path(), None)
    };
    std::fs::create_dir_all(&own).unwrap();
    std::fs::copy(home.path().join(".claude.json"), own.join(".claude.json")).unwrap();
    let err = format!("{:#}", steps::apply(&env, &Bundle::embedded()).unwrap_err());
    assert!(err.contains("restoring the old entry failed too"), "{err}");
    assert!(
        err.contains(&format!("run: CLAUDE_CONFIG_DIR='{}' claude mcp add-json --scope user t3cm", own.display())),
        "{err}"
    );
    assert!(!err.contains("mcp remove"), "{err}");
    let log = std::fs::read_to_string(bin.path().join("claude.log")).unwrap();
    let dir = own.display().to_string();
    assert_eq!(log, format!("remove {dir}\nadd-json {dir}\nadd-json {dir}\n"));
}

#[test]
fn null_header_value_is_stale_and_recovery_commands_quote_for_unix_and_powershell() {
    let home = tempfile::tempdir().unwrap();
    let entry = json!({
        "type": "http",
        "url": "http://127.0.0.1:47731/mcp",
        "headers": {
            "Authorization": "Bearer foo bar",
            "X-Path": r"C:\dir\",
            "X-User": "O'Neil",
            "X-T3-Thread": "${T3CODE_THREAD_ID}",
            "x-t3-config-dir": null
        }
    });
    std::fs::write(home.path().join(".claude.json"), json!({"mcpServers": {"t3cm": entry}}).to_string()).unwrap();
    let own = home.path().join("o'neil/.claude-2");
    let env = Env { claude_config_dir: Some(own.clone()), ..Env::under(home.path(), None) };
    let reg = steps::registration(&env);
    assert_eq!(reg.status, steps::RegState::Stale, "null X-T3-Config-Dir must not count as current");

    let updated = steps::registered_entry(&reg).unwrap();
    let headers = updated["headers"].as_object().unwrap();
    assert_eq!(headers.get("X-T3-Config-Dir"), Some(&json!("${CLAUDE_CONFIG_DIR:-}")));
    assert!(!headers.contains_key("x-t3-config-dir"), "lowercase duplicate key should be normalized");

    let unix_cmd = steps::reregister_command_for(&env, &reg, false, false);
    assert!(unix_cmd.contains(r"CLAUDE_CONFIG_DIR='") && unix_cmd.contains(r"o'\''neil/.claude-2'"), "{unix_cmd}");
    assert!(unix_cmd.contains(" && "), "{unix_cmd}");

    let ps_cmd = steps::reregister_command_for(&env, &reg, false, true);
    assert!(ps_cmd.contains("$env:CLAUDE_CONFIG_DIR = '") && ps_cmd.contains("o''neil/.claude-2'"), "{ps_cmd}");
    assert!(!ps_cmd.contains(" && "), "PowerShell 5.1 does not support &&: {ps_cmd}");
    assert!(!ps_cmd.contains("Start-Process"), "Start-Process fails on npm .ps1 shims: {ps_cmd}");
    assert!(
        ps_cmd.contains("claude mcp remove --scope user t3cm; if ($? -and $LASTEXITCODE -eq 0) { claude mcp add-json --scope user t3cm $j }"),
        "Windows recovery command must invoke claude directly and gate add on remove: {ps_cmd}"
    );
    assert!(
        ps_cmd.contains(r#"\"Authorization\":\"Bearer\u0020foo\u0020bar\""#)
            && ps_cmd.contains(r#"\"X-Path\":\"C:\\dir\\\\\""#)
            && ps_cmd.contains(r#"\"X-User\":\"O''Neil\""#)
            && ps_cmd.contains(r#"\"X-T3-Config-Dir\":\"${CLAUDE_CONFIG_DIR:-}\""#)
            && ps_cmd.contains(r#""Authorization":"Bearer\u0020foo\u0020bar""#),
        "Windows command must select Legacy vs modern quoting with space-escaped JSON: {ps_cmd}"
    );

    let ps_add_only = steps::reregister_command_for(&env, &reg, true, true);
    assert!(!ps_add_only.contains("mcp remove"), "{ps_add_only}");
    assert!(ps_add_only.contains("$env:CLAUDE_CONFIG_DIR = '"), "{ps_add_only}");
    assert!(
        ps_add_only.ends_with("claude mcp add-json --scope user t3cm $j")
            && ps_add_only.contains(r#"\"X-T3-Config-Dir\":\"${CLAUDE_CONFIG_DIR:-}\""#),
        "{ps_add_only}"
    );
}

#[cfg(unix)]
#[test]
fn retire_t3wf_removes_timers_shim_and_pycache_and_keeps_registry_data() {
    let home = tempfile::tempdir().unwrap();
    let bin = tempfile::tempdir().unwrap();
    let log = bin.path().join("systemctl.log");
    let fake_sysctl = bin.path().join("systemctl");
    t3_codemode::fake::write_executable(&fake_sysctl, &format!("#!/bin/sh\necho \"$*\" >> '{}'\n", log.display()));

    let mut env = Env::under(home.path(), None);
    env.systemctl_cmd = Some(fake_sysctl);
    write_config(home.path(), &json!({"version": 1, "passes": ["pr"]}));

    std::fs::create_dir_all(&env.systemd_user).unwrap();
    for name in ["t3wf-tick.timer", "t3wf-tick.service", "cbrip-t3wf-tick.timer", "cbrip-t3wf-tick.service"] {
        std::fs::write(env.systemd_user.join(name), "[Unit]\n").unwrap();
    }
    let shim = env.t3wf_state.join("v2").join("bin").join("t3wf-tick");
    std::fs::create_dir_all(shim.parent().unwrap()).unwrap();
    std::fs::write(&shim, "#!/bin/sh\n").unwrap();
    let links = env.t3wf_state.join("v2").join("links.json");
    std::fs::write(&links, r#"{"links":{},"outbox":[]}"#).unwrap();
    let pycache = env.skills_dir.join("t3-workflow").join("scripts").join("__pycache__");
    std::fs::create_dir_all(&pycache).unwrap();
    std::fs::write(pycache.join("t3wf.cpython-312.pyc"), b"pyc").unwrap();

    let before = steps::check(&env, &Bundle::embedded()).unwrap();
    assert!(before.needs_update());
    assert!(before.missing.iter().any(|m| m.id == steps::RETIRE_T3WF_STEP));
    assert!(steps::describe_steps(&before).contains("t3wf leftovers remain"));

    let applied = steps::apply(&env, &Bundle::embedded()).unwrap();
    assert_eq!(applied.removed.len(), 6, "{:?}", applied.removed);
    assert!(applied.newly_recorded.contains(&steps::RETIRE_T3WF_STEP.to_string()));
    assert!(!env.systemd_user.join("t3wf-tick.timer").exists());
    assert!(!env.systemd_user.join("cbrip-t3wf-tick.service").exists());
    assert!(!shim.exists());
    assert!(!shim.parent().unwrap().exists(), "empty bin/ folder should be removed");
    assert!(!pycache.exists());
    assert!(!pycache.parent().unwrap().exists(), "empty scripts/ folder should be removed");
    assert!(links.is_file(), "registry data must be left alone");
    let sysctl_log = std::fs::read_to_string(&log).unwrap();
    assert!(sysctl_log.contains("--user disable --now t3wf-tick.timer"), "{sysctl_log}");
    assert!(sysctl_log.contains("--user disable --now cbrip-t3wf-tick.timer"), "{sysctl_log}");
    assert!(sysctl_log.contains("--user daemon-reload"), "{sysctl_log}");
    assert!(!steps::check(&env, &Bundle::embedded()).unwrap().missing.iter().any(|m| m.id == steps::RETIRE_T3WF_STEP));
}

#[cfg(unix)]
#[test]
fn retire_t3wf_warns_and_nudges_open_links_without_blocking() {
    use t3_codemode::config::LinkParams;
    use t3_codemode::store::{Link, Store};

    let home = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let bin = tempfile::tempdir().unwrap();
    let env = Env::under(home.path(), None);

    let links = env.t3wf_state.join("v2").join("links.json");
    std::fs::create_dir_all(links.parent().unwrap()).unwrap();
    std::fs::write(
        &links,
        json!({
            "links": {
                "t-worker-1": {"child": "t-worker-1", "parent": "t-ticket-1", "label": "#42 PR 1/1", "closed": false},
                "t-closed": {"child": "t-closed", "parent": "t-ticket-1", "label": "#41", "closed": true}
            },
            "outbox": [{"to": "t-ticket-1", "body": "pending"}]
        })
        .to_string(),
    )
    .unwrap();

    let store = Store::open(state.path()).unwrap();
    store
        .link_save(&Link {
            child: "mcp:worker-2".into(),
            parent: "mcp:ticket-2".into(),
            label: "#43 PR 1/1".into(),
            role: "worker".into(),
            parent_role: Some("ticket".into()),
            params: LinkParams { down: 45, miss: 2, stall: 240 },
            link: None,
            opened: "2026-11-20T00:00:00Z".into(),
            last_up: "2026-11-20T00:00:00Z".into(),
            last_seen: None,
            last_nudge: None,
            nudges: 0,
            escalated: false,
            reported: Vec::new(),
            gate: None,
            gate_since: None,
            closed: false,
            closed_at: None,
            closed_how: None,
            unread: Vec::new(),
            bad_run: None,
            settled_at: None,
            restored_at: None,
            project: None,
        })
        .unwrap();

    let report = steps::check_open_links(&env, state.path());
    assert_eq!(report.warnings.len(), 2, "{:?}", report.warnings);
    assert!(report.warnings[0].contains("1 open link(s) and 1 undelivered message(s)"), "{:?}", report.warnings);
    assert!(report.warnings[1].contains("1 open workflow link(s)"), "{:?}", report.warnings);
    assert_eq!(report.t3wf_threads, vec!["t-ticket-1", "t-worker-1"]);
    assert_eq!(report.t3cm_threads, vec!["mcp:ticket-2", "mcp:worker-2"]);

    let log = bin.path().join("t3.log");
    let fake_t3 = bin.path().join("t3");
    t3_codemode::fake::write_executable(
        &fake_t3,
        &format!("#!/bin/sh\nmsg=$(cat)\necho \"$1 $2 $3|$msg\" >> '{}'\n", log.display()),
    );

    let (nudged, notes) = steps::nudge_threads(Some(&fake_t3), &report.t3wf_threads);
    assert_eq!(nudged, vec!["t-ticket-1", "t-worker-1"]);
    assert!(notes.is_empty(), "{notes:?}");
    let sent = std::fs::read_to_string(&log).unwrap();
    assert!(sent.contains(&format!("thread send t-ticket-1|{}", steps::SKILLS_CHANGED_NOTE)), "{sent}");
    assert!(sent.contains(&format!("thread send t-worker-1|{}", steps::SKILLS_CHANGED_NOTE)), "{sent}");

    let (nudged_none, notes_none) = steps::nudge_threads(None, &report.t3cm_threads);
    assert!(nudged_none.is_empty());
    assert!(notes_none[0].contains("`t3` CLI was not found"), "{notes_none:?}");
}

#[cfg(unix)]
#[test]
fn retire_t3wf_surfaces_systemctl_failure_and_leaves_unit_detectable_for_retry() {
    let home = tempfile::tempdir().unwrap();
    let bin = tempfile::tempdir().unwrap();
    let fake_sysctl = bin.path().join("systemctl");
    t3_codemode::fake::write_executable(&fake_sysctl, "#!/bin/sh\necho 'Failed to connect to bus' >&2\nexit 1\n");

    let mut env = Env::under(home.path(), None);
    env.systemctl_cmd = Some(fake_sysctl);
    write_config(home.path(), &json!({"version": 1, "passes": ["pr"]}));

    let wants = env.systemd_user.join("timers.target.wants");
    std::fs::create_dir_all(&wants).unwrap();
    let timer = env.systemd_user.join("t3wf-tick.timer");
    std::fs::write(&timer, "[Unit]\n").unwrap();
    std::os::unix::fs::symlink(&timer, wants.join("t3wf-tick.timer")).unwrap();

    let applied = steps::apply(&env, &Bundle::embedded()).unwrap();
    assert!(applied.removed.is_empty(), "failed disable must leave unit files in place: {:?}", applied.removed);
    assert!(
        applied
            .notes
            .iter()
            .any(|n| n.contains("could not disable t3wf-tick.timer") && n.contains("Failed to connect to bus")),
        "{:?}",
        applied.notes
    );
    assert!(!applied.newly_recorded.contains(&steps::RETIRE_T3WF_STEP.to_string()));
    assert!(timer.exists(), "unit file must stay on disk so retry runs");
    assert!(wants.join("t3wf-tick.timer").exists(), "enabled wants symlink must stay detectable");
    assert!(
        steps::check(&env, &Bundle::embedded()).unwrap().missing.iter().any(|m| m.id == steps::RETIRE_T3WF_STEP),
        "retire:t3wf must remain missing after systemctl failure"
    );
}

fn agy_profile(home: &Path, hash: &str) -> PathBuf {
    let dir = home.join(".t3").join("userdata").join("providers").join("antigravity").join(hash).join("config");
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("mcp_config.json")
}

#[test]
fn update_registers_t3cm_stdio_for_every_antigravity_profile_and_merges_json() {
    let home = tempfile::tempdir().unwrap();
    let p1 = agy_profile(home.path(), "aaaa1111");
    let p2 = agy_profile(home.path(), "bbbb2222");
    // Fresh Antigravity profiles carry a 0-byte mcp_config.json.
    std::fs::write(&p1, "").unwrap();
    std::fs::write(
        &p2,
        json!({
            "extra": 1,
            "mcpServers": {
                "other": {"command": "other-mcp", "args": ["--flag"]}
            }
        })
        .to_string(),
    )
    .unwrap();
    // Stray file next to profiles must be ignored; ~/.gemini must stay untouched.
    let providers_dir = home.path().join(".t3/userdata/providers/antigravity");
    std::fs::write(providers_dir.join("stray.txt"), "ignore\n").unwrap();
    let gemini_cfg = home.path().join(".gemini/config/mcp_config.json");
    std::fs::create_dir_all(gemini_cfg.parent().unwrap()).unwrap();
    std::fs::write(&gemini_cfg, r#"{"mcpServers":{"mine":{"command":"x"}}}"#).unwrap();

    write_config(home.path(), &json!({"version": 1, "passes": ["pr"]}));
    let env = Env::under(home.path(), None);

    let before = steps::check(&env, &Bundle::embedded()).unwrap();
    assert!(before.missing.iter().any(|m| m.id == steps::ANTIGRAVITY_STEP && m.fix == "t3cm skills update"));
    let shown_before = steps::describe_steps(&before);
    assert!(shown_before.contains(&p1.display().to_string()), "{shown_before}");
    assert!(shown_before.contains(&p2.display().to_string()), "{shown_before}");

    let applied = steps::apply(&env, &Bundle::embedded()).unwrap();
    assert_eq!(applied.antigravity, vec![p1.clone(), p2.clone()], "{:?}", applied.notes);
    assert!(applied.notes.is_empty(), "{:?}", applied.notes);
    let shown = steps::describe_applied(&applied);
    assert_eq!(shown.matches("system text-generation model").count(), 1, "caveat must print once: {shown}");

    let want = steps::antigravity_entry(&env.t3cm_bin);
    let v1: Value = serde_json::from_str(&std::fs::read_to_string(&p1).unwrap()).unwrap();
    assert_eq!(v1, json!({"mcpServers": {"t3cm": want}}));
    let v2: Value = serde_json::from_str(&std::fs::read_to_string(&p2).unwrap()).unwrap();
    assert_eq!(v2["extra"], 1);
    assert_eq!(v2["mcpServers"]["other"], json!({"command": "other-mcp", "args": ["--flag"]}));
    assert_eq!(v2["mcpServers"]["t3cm"], want);
    assert_eq!(
        std::fs::read_to_string(&gemini_cfg).unwrap(),
        r#"{"mcpServers":{"mine":{"command":"x"}}}"#,
        "~/.gemini/config/mcp_config.json must never be modified"
    );
    assert!(read_config(home.path())["steps"].as_array().unwrap().contains(&json!(steps::ANTIGRAVITY_STEP)));

    // Rerun is idempotent and does not repeat the caveat.
    let raw2_before = std::fs::read_to_string(&p2).unwrap();
    let again = steps::apply(&env, &Bundle::embedded()).unwrap();
    assert!(again.antigravity.is_empty());
    assert!(!steps::describe_applied(&again).contains("system text-generation model"));
    assert_eq!(std::fs::read_to_string(&p2).unwrap(), raw2_before);
    assert!(!steps::check(&env, &Bundle::embedded()).unwrap().missing.iter().any(|m| m.id == steps::ANTIGRAVITY_STEP));
}

#[test]
fn antigravity_merges_eager_flags_into_existing_stdio_and_reports_conflicts_and_invalid_json() {
    let home = tempfile::tempdir().unwrap();
    let stale = agy_profile(home.path(), "stale000");
    let conflict = agy_profile(home.path(), "conflict1");
    let broken = agy_profile(home.path(), "broken22");
    std::fs::write(
        &stale,
        json!({
            "mcpServers": {
                "t3cm": {
                    "command": "/opt/custom/t3cm",
                    "args": ["stdio", "--state-dir", "/tmp/x"],
                    "env": {"FOO": "bar"},
                    "tools": {"search": {"eager": false, "note": "keep"}}
                }
            }
        })
        .to_string(),
    )
    .unwrap();
    let conflict_raw = json!({"mcpServers": {"t3cm": {"url": "http://127.0.0.1:47731/mcp"}}}).to_string();
    std::fs::write(&conflict, &conflict_raw).unwrap();
    std::fs::write(&broken, "{not valid json").unwrap();

    let env = Env::under(home.path(), None);
    let st = steps::check(&env, &Bundle::embedded()).unwrap();
    assert!(st.needs_update(), "stale stdio entry needs eager flags: {:?}", st.missing);
    let shown_before = steps::describe_steps(&st);
    assert!(shown_before.contains("missing eager tool flags"), "{shown_before}");
    assert!(shown_before.contains("incompatible t3cm entry"), "{shown_before}");
    assert!(shown_before.contains("could not be read"), "{shown_before}");

    let applied = steps::apply(&env, &Bundle::embedded()).unwrap();
    assert_eq!(applied.antigravity, vec![stale.clone()]);
    assert_eq!(applied.notes.len(), 2, "{:?}", applied.notes);
    assert!(applied.notes.iter().any(|n| n.contains(&conflict.display().to_string()) && n.contains("incompatible")));
    assert!(applied.notes.iter().any(|n| n.contains(&broken.display().to_string())));

    let updated: Value = serde_json::from_str(&std::fs::read_to_string(&stale).unwrap()).unwrap();
    assert_eq!(updated["mcpServers"]["t3cm"]["command"], "/opt/custom/t3cm");
    assert_eq!(updated["mcpServers"]["t3cm"]["args"], json!(["stdio", "--state-dir", "/tmp/x"]));
    assert_eq!(updated["mcpServers"]["t3cm"]["env"], json!({"FOO": "bar"}));
    assert_eq!(updated["mcpServers"]["t3cm"]["tools"]["search"], json!({"eager": true, "note": "keep"}));
    assert_eq!(updated["mcpServers"]["t3cm"]["tools"]["execute"], json!({"eager": true}));
    assert_eq!(std::fs::read_to_string(&conflict).unwrap(), conflict_raw);
    assert_eq!(std::fs::read_to_string(&broken).unwrap(), "{not valid json");
    assert!(!steps::check(&env, &Bundle::embedded()).unwrap().needs_update());
}

#[test]
fn t3code_home_env_selects_antigravity_profile_root() {
    let home = tempfile::tempdir().unwrap();
    let custom_t3 = home.path().join("custom-t3");
    let cfg = custom_t3.join("userdata/providers/antigravity/deadbeef/config/mcp_config.json");
    std::fs::create_dir_all(cfg.parent().unwrap()).unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_t3cm"))
        .args(["skills", "install"])
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .env("T3CODE_HOME", &custom_t3)
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("CLAUDE_CONFIG_DIR")
        .env_remove("CODEX_HOME")
        .env_remove("T3WF_CONFIG")
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", text(&out));
    let saved: Value = serde_json::from_str(&std::fs::read_to_string(&cfg).unwrap()).unwrap();
    assert_eq!(saved["mcpServers"]["t3cm"]["args"], json!(["stdio"]));
    assert_eq!(saved["mcpServers"]["t3cm"]["tools"]["search"]["eager"], true);
    assert_eq!(saved["mcpServers"]["t3cm"]["tools"]["execute"]["eager"], true);
}

#[cfg(unix)]
#[test]
fn antigravity_registration_keeps_file_mode() {
    use std::os::unix::fs::PermissionsExt;
    let home = tempfile::tempdir().unwrap();
    let cfg = agy_profile(home.path(), "mode0600");
    std::fs::write(&cfg, "").unwrap();
    std::fs::set_permissions(&cfg, std::fs::Permissions::from_mode(0o600)).unwrap();
    let applied = steps::apply(&Env::under(home.path(), None), &Bundle::embedded()).unwrap();
    assert_eq!(applied.antigravity, vec![cfg.clone()]);
    assert_eq!(std::fs::metadata(&cfg).unwrap().permissions().mode() & 0o777, 0o600);
}

#[cfg(unix)]
#[test]
fn install_sh_registers_and_uninstalls_antigravity_in_scratch_home() {
    use sha2::{Digest, Sha256};

    let scratch = tempfile::tempdir().unwrap();
    let pkg = tempfile::tempdir().unwrap();
    let crate_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let version = env!("CARGO_PKG_VERSION");
    let stem = format!("t3cm-{version}-test-target");
    let stage = pkg.path().join(&stem);
    std::fs::create_dir_all(&stage).unwrap();
    std::fs::copy(env!("CARGO_BIN_EXE_t3cm"), stage.join("t3cm")).unwrap();
    std::fs::copy(crate_root.join("deploy/t3-codemode.service"), stage.join("t3-codemode.service")).unwrap();
    std::fs::copy(crate_root.join("deploy/com.ottoops.t3-codemode.plist"), stage.join("com.ottoops.t3-codemode.plist"))
        .unwrap();

    let archive_name = format!("{stem}.tar.gz");
    let archive = pkg.path().join(&archive_name);
    let tar = Command::new("tar")
        .args(["-czf", archive.to_str().unwrap(), "-C", pkg.path().to_str().unwrap(), &stem])
        .output()
        .unwrap();
    assert!(tar.status.success(), "{}", text(&tar));
    let digest: String = Sha256::digest(std::fs::read(&archive).unwrap()).iter().map(|b| format!("{b:02x}")).collect();
    let sums = pkg.path().join("SHA256SUMS");
    std::fs::write(&sums, format!("{digest}  {archive_name}\n")).unwrap();

    let p1 = agy_profile(scratch.path(), "prof1111");
    let p2 = agy_profile(scratch.path(), "prof2222");
    std::fs::write(&p1, "").unwrap();
    std::fs::write(&p2, json!({"keep": true, "mcpServers": {"other": {"command": "other"}}}).to_string()).unwrap();
    let gemini = scratch.path().join(".gemini/config/mcp_config.json");
    std::fs::create_dir_all(gemini.parent().unwrap()).unwrap();
    std::fs::write(&gemini, r#"{"mcpServers":{"stay":{"command":"y"}}}"#).unwrap();

    let run_install = |args: &[&str]| {
        Command::new("bash")
            .arg(crate_root.join("deploy/install.sh"))
            .args(args)
            .env_clear()
            .env("HOME", scratch.path())
            .env("PATH", "/usr/bin:/bin")
            .output()
            .unwrap()
    };

    // 1. Install with --no-workflow exercises install.sh's own JSON registration path.
    let out = run_install(&[
        "--files-only",
        "--no-workflow",
        "--archive",
        archive.to_str().unwrap(),
        "--sums",
        sums.to_str().unwrap(),
    ]);
    assert!(out.status.success(), "{}", text(&out));
    let shown = text(&out);
    assert_eq!(shown.matches("system text-generation model").count(), 1, "{shown}");

    let expected_bin = scratch.path().join(".local/bin/t3cm").to_string_lossy().into_owned();
    let expected_entry = json!({
        "command": expected_bin,
        "args": ["stdio"],
        "tools": {
            "search": {"eager": true},
            "execute": {"eager": true}
        }
    });
    let v1: Value = serde_json::from_str(&std::fs::read_to_string(&p1).unwrap()).unwrap();
    assert_eq!(v1, json!({"mcpServers": {"t3cm": expected_entry}}));
    let v2: Value = serde_json::from_str(&std::fs::read_to_string(&p2).unwrap()).unwrap();
    assert_eq!(v2["keep"], true);
    assert_eq!(v2["mcpServers"]["other"], json!({"command": "other"}));
    assert_eq!(v2["mcpServers"]["t3cm"], expected_entry);
    assert_eq!(std::fs::read_to_string(&gemini).unwrap(), r#"{"mcpServers":{"stay":{"command":"y"}}}"#);

    // 2. Rerun is idempotent and does not repeat the caveat.
    let p2_before = std::fs::read_to_string(&p2).unwrap();
    let rerun = run_install(&[
        "--files-only",
        "--no-workflow",
        "--archive",
        archive.to_str().unwrap(),
        "--sums",
        sums.to_str().unwrap(),
    ]);
    assert!(rerun.status.success(), "{}", text(&rerun));
    assert!(!text(&rerun).contains("system text-generation model"), "{}", text(&rerun));
    assert_eq!(std::fs::read_to_string(&p2).unwrap(), p2_before);

    // 3. Uninstall removes only t3cm, keeping other servers and top-level keys.
    let un = run_install(&["--uninstall", "--files-only"]);
    assert!(un.status.success(), "{}", text(&un));
    let v1_after: Value = serde_json::from_str(&std::fs::read_to_string(&p1).unwrap()).unwrap();
    assert_eq!(v1_after, json!({"mcpServers": {}}));
    let v2_after: Value = serde_json::from_str(&std::fs::read_to_string(&p2).unwrap()).unwrap();
    assert_eq!(v2_after, json!({"keep": true, "mcpServers": {"other": {"command": "other"}}}));
    assert_eq!(std::fs::read_to_string(&gemini).unwrap(), r#"{"mcpServers":{"stay":{"command":"y"}}}"#);

    // 4. Install with --workflow (default) also prints the caveat only once (steps::apply writes, register_mcp skips).
    let wf = run_install(&[
        "--files-only",
        "--workflow",
        "--archive",
        archive.to_str().unwrap(),
        "--sums",
        sums.to_str().unwrap(),
    ]);
    assert!(wf.status.success(), "{}", text(&wf));
    let wf_shown = text(&wf);
    assert_eq!(wf_shown.matches("system text-generation model").count(), 1, "{wf_shown}");
    let v1_wf: Value = serde_json::from_str(&std::fs::read_to_string(&p1).unwrap()).unwrap();
    assert_eq!(v1_wf["mcpServers"]["t3cm"], expected_entry);

    // 5. Minimal PATH without python3 or jq still registers (--no-workflow) and uninstalls via installed t3cm.
    let min_bin = tempfile::tempdir().unwrap();
    for cmd in [
        "awk",
        "basename",
        "bash",
        "cat",
        "chmod",
        "dirname",
        "gzip",
        "install",
        "mkdir",
        "mktemp",
        "mv",
        "plutil",
        "rm",
        "sed",
        "sha256sum",
        "shasum",
        "tar",
        "uname",
    ] {
        for dir in ["/usr/bin", "/bin"] {
            let p = Path::new(dir).join(cmd);
            if p.is_file() {
                let _ = std::os::unix::fs::symlink(&p, min_bin.path().join(cmd));
                break;
            }
        }
    }
    let un_min = Command::new("bash")
        .arg(crate_root.join("deploy/install.sh"))
        .args(["--uninstall", "--files-only"])
        .env_clear()
        .env("HOME", scratch.path())
        .env("PATH", min_bin.path())
        .output()
        .unwrap();
    assert!(un_min.status.success(), "{}", text(&un_min));
    let v1_min_un: Value = serde_json::from_str(&std::fs::read_to_string(&p1).unwrap()).unwrap();
    assert_eq!(v1_min_un, json!({"mcpServers": {}}));

    let in_min = Command::new("bash")
        .arg(crate_root.join("deploy/install.sh"))
        .args([
            "--files-only",
            "--no-workflow",
            "--archive",
            archive.to_str().unwrap(),
            "--sums",
            sums.to_str().unwrap(),
        ])
        .env_clear()
        .env("HOME", scratch.path())
        .env("PATH", min_bin.path())
        .output()
        .unwrap();
    assert!(in_min.status.success(), "{}", text(&in_min));
    let v1_min_in: Value = serde_json::from_str(&std::fs::read_to_string(&p1).unwrap()).unwrap();
    assert_eq!(v1_min_in["mcpServers"]["t3cm"], expected_entry);

    // 6. Uninstall retains t3cm when a known registration cannot be removed, and succeeds once writable.
    if unsafe { libc::geteuid() } != 0 {
        use std::os::unix::fs::PermissionsExt;
        let p1_dir = p1.parent().unwrap();
        let bin_path = scratch.path().join(".local/bin/t3cm");
        std::fs::set_permissions(p1_dir, std::fs::Permissions::from_mode(0o555)).unwrap();
        let blocked = Command::new("bash")
            .arg(crate_root.join("deploy/install.sh"))
            .args(["--uninstall", "--files-only"])
            .env_clear()
            .env("HOME", scratch.path())
            .env("PATH", min_bin.path())
            .output()
            .unwrap();
        std::fs::set_permissions(p1_dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(!blocked.status.success(), "uninstall must fail when a known registration cannot be removed");
        assert!(bin_path.is_file(), "t3cm binary must be retained for retry: {}", text(&blocked));
        // Writable sibling profile was still cleaned up on the same pass.
        let v2_partial: Value = serde_json::from_str(&std::fs::read_to_string(&p2).unwrap()).unwrap();
        assert_eq!(v2_partial, json!({"keep": true, "mcpServers": {"other": {"command": "other"}}}));

        let retry = Command::new("bash")
            .arg(crate_root.join("deploy/install.sh"))
            .args(["--uninstall", "--files-only"])
            .env_clear()
            .env("HOME", scratch.path())
            .env("PATH", min_bin.path())
            .output()
            .unwrap();
        assert!(retry.status.success(), "{}", text(&retry));
        assert!(!bin_path.exists(), "t3cm binary should be removed once cleanup succeeds");
        let v1_cleaned: Value = serde_json::from_str(&std::fs::read_to_string(&p1).unwrap()).unwrap();
        assert_eq!(v1_cleaned, json!({"mcpServers": {}}));
    }
}

#[test]
fn remove_antigravity_leaves_incompatible_t3cm_entry_alone() {
    let home = tempfile::tempdir().unwrap();
    let owned = agy_profile(home.path(), "owned111");
    let foreign = agy_profile(home.path(), "foreign2");
    let env = Env::under(home.path(), None);
    let owned_raw =
        json!({"mcpServers": {"t3cm": steps::antigravity_entry(&env.t3cm_bin), "keep": {"command": "k"}}}).to_string();
    let foreign_raw = json!({"mcpServers": {"t3cm": {"url": "http://127.0.0.1:47731/mcp"}}}).to_string();
    std::fs::write(&owned, &owned_raw).unwrap();
    std::fs::write(&foreign, &foreign_raw).unwrap();

    assert!(steps::remove_antigravity(&owned).unwrap());
    assert!(!steps::remove_antigravity(&foreign).unwrap());
    let owned_after: Value = serde_json::from_str(&std::fs::read_to_string(&owned).unwrap()).unwrap();
    assert_eq!(owned_after, json!({"mcpServers": {"keep": {"command": "k"}}}));
    assert_eq!(std::fs::read_to_string(&foreign).unwrap(), foreign_raw);
}

#[cfg(unix)]
#[test]
fn antigravity_targets_reports_unreadable_provider_and_profile_dirs() {
    use std::os::unix::fs::PermissionsExt;
    if unsafe { libc::geteuid() } == 0 {
        return;
    }
    let home = tempfile::tempdir().unwrap();
    let good = agy_profile(home.path(), "good1111");
    let bad = agy_profile(home.path(), "bad22222");
    std::fs::write(&good, "").unwrap();
    std::fs::write(&bad, "").unwrap();
    let bad_profile = bad.parent().unwrap().parent().unwrap();
    let root = bad_profile.parent().unwrap();

    // Unreadable individual profile dir reports a Problem alongside the readable sibling.
    std::fs::set_permissions(bad_profile, std::fs::Permissions::from_mode(0o000)).unwrap();
    let targets = steps::antigravity_targets(&Env::under(home.path(), None));
    std::fs::set_permissions(bad_profile, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(targets.iter().any(|t| t.path == good && t.status == steps::AntigravityState::Missing), "{targets:?}");
    assert!(targets.iter().any(|t| t.status == steps::AntigravityState::Problem), "{targets:?}");

    // Unreadable provider root reports a Problem instead of an empty list.
    std::fs::set_permissions(root, std::fs::Permissions::from_mode(0o000)).unwrap();
    let root_targets = steps::antigravity_targets(&Env::under(home.path(), None));
    std::fs::set_permissions(root, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(root_targets.len(), 1, "{root_targets:?}");
    assert_eq!(root_targets[0].status, steps::AntigravityState::Problem);
}

#[test]
fn write_antigravity_serializes_concurrent_writers() {
    let home = tempfile::tempdir().unwrap();
    let cfg = agy_profile(home.path(), "race1111");
    std::fs::write(&cfg, json!({"mcpServers": {"other": {"command": "other"}}}).to_string()).unwrap();
    let bin = home.path().join(".local/bin/t3cm");

    std::thread::scope(|s| {
        for _ in 0..8 {
            s.spawn(|| {
                steps::write_antigravity(&cfg, &bin).unwrap();
            });
        }
    });
    let saved: Value = serde_json::from_str(&std::fs::read_to_string(&cfg).unwrap()).unwrap();
    assert_eq!(saved["mcpServers"]["other"], json!({"command": "other"}));
    assert_eq!(saved["mcpServers"]["t3cm"], steps::antigravity_entry(&bin));
}
