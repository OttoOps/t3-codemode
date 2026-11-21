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
    use std::os::unix::fs::PermissionsExt;
    let log = dir.join("claude.log");
    let script = dir.join("claude");
    let add = if fail_add {
        "exit 1".to_string()
    } else {
        format!(r#"printf '{{"mcpServers":{{"t3cm":%s}}}}' "$6" > '{}'"#, claude_json.display())
    };
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\necho \"$2 ${{CLAUDE_CONFIG_DIR:-unset}}\" >> '{}'\ncase \"$2\" in\n  add-json) {add} ;;\nesac\n",
            log.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
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
