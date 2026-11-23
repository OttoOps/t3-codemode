use std::path::Path;
use std::process::{Command, Output};

use serde_json::{Value, json};
use t3_codemode::config::Config;
use t3_codemode::config_edit::{Document, Edit, Rejected, Stale, effective, lookup};
use t3_codemode::fake::{FakeTailscale, provider};
use t3_codemode::tailscale::{ExposeOptions, expose_tailscale, unexpose_board};
use t3_codemode::{field, server};

use crate::common::H;

// Port 0 cannot be connected to, so a test never reaches a service installed on the default port.
const NO_SERVICE: u16 = 0;

pub(crate) struct Cfg {
    pub(crate) dir: tempfile::TempDir,
    pub(crate) file: std::path::PathBuf,
    port: u16,
}

impl Cfg {
    pub(crate) fn absent() -> Cfg {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("config.json");
        Cfg { dir, file, port: NO_SERVICE }
    }

    pub(crate) fn at(file: std::path::PathBuf) -> Cfg {
        Cfg { file, ..Cfg::absent() }
    }

    pub(crate) fn on_port(self, port: u16) -> Cfg {
        Cfg { port, ..self }
    }

    fn with(text: &str) -> Cfg {
        let c = Cfg::absent();
        std::fs::write(&c.file, text).unwrap();
        c
    }

    fn text(&self) -> String {
        std::fs::read_to_string(&self.file).unwrap()
    }

    pub(crate) fn command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_t3cm"));
        cmd.arg("config")
            .args(args)
            .env("T3CODE_HOME", self.dir.path())
            .env("HOME", self.dir.path())
            .env("XDG_CONFIG_HOME", self.dir.path().join("xdg"))
            .env("T3CM_CONFIG", &self.file)
            .env("T3CM_STATE_DIR", self.dir.path().join("state"))
            .env("T3CM_NO_UPDATE_CHECK", "1")
            .env("T3CM_PORT", self.port.to_string())
            .env_remove("VISUAL")
            .env_remove("EDITOR")
            .env_remove("T3CODE_THREAD_ID");
        cmd
    }

    pub(crate) fn run(&self, args: &[&str]) -> Output {
        self.command(args).output().unwrap()
    }

    pub(crate) fn ok(&self, args: &[&str]) -> String {
        let out = self.run(args);
        assert!(out.status.success(), "{args:?}: {}", stderr(&out));
        String::from_utf8(out.stdout).unwrap()
    }

    fn get(&self, path: &str) -> Value {
        serde_json::from_str(&self.ok(&["get", path])).unwrap()
    }
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

pub(crate) fn set(path: &str, value: Value) -> Edit {
    Edit::Set { path: field::parse(path).unwrap(), value }
}

fn unset(path: &str) -> Edit {
    Edit::Unset { path: field::parse(path).unwrap() }
}

pub(crate) fn apply(file: &Path, edit: Edit) {
    Document::read(file).unwrap().edit(&[edit]).unwrap().save().unwrap();
}

pub(crate) fn every_kind_of_key() -> Vec<(&'static str, Value)> {
    vec![
        ("deny", json!(["t3_project_delete", "t3_thread_interrupt"])),
        ("watcherIntervalSecs", json!(90)),
        ("maxHoldMinutes", json!(12.5)),
        ("cpuBudgetMs", json!(3000)),
        ("defaultTimeoutMs", json!(70000)),
        ("maxTimeoutMs", json!(700000)),
        ("maxConcurrentRuns", json!(4)),
        ("runQueueMs", json!(9000)),
        ("link.stall", json!(300)),
        ("link.down", json!(50)),
        ("link.miss", json!(3)),
        ("wake", json!(["event", "gate", "handoff"])),
        ("bodyCap", json!(1600)),
        ("replyCap", json!(1700)),
        ("digestLines", json!(7)),
        ("limitHoldMinutes", json!(90)),
        ("updateCheck", json!(false)),
        ("roles.overseer.children.ticket.stall", json!(500)),
        ("roles.overseer.children.ticket.down", json!(80)),
        ("roles.overseer.children.ticket.miss", json!(4)),
        ("roles.overseer.children.ticket.wake", json!(["gate", "handoff"])),
        ("roles.overseer.children.ticket.bodyCap", json!(700)),
        (
            "pools.\"gpt-6.1\"",
            json!({"driver": "codex", "model": "gpt-6.1", "members": [{"instanceId": "codex-a"}, {"instanceId": "codex-b"}]}),
        ),
        ("pools.\"gpt-6.1\".effort", json!("high")),
        ("pools.\"gpt-6.1\".strategy", json!("priority")),
        ("pools.\"gpt-6.1\".moveOnLimit", json!("auto")),
        ("pools.\"gpt-6.1\".members.0.weight", json!(3)),
        ("pools.\"gpt-6.1\".members.1.reserve", json!(25.0)),
        ("pools.\"gpt-6.1\".members.1.cost", json!(2.0)),
        ("pools.tier", json!({"mixed": true, "members": [{"instanceId": "claude-a", "model": "claude-opus-5-5"}]})),
        ("pools.tier.members.0.effort", json!("high")),
        ("board.instanceId", json!("claude-a")),
        ("board.pool", json!("gpt-6.1")),
        ("board.ticketModel", json!("claude-sonnet-5-5")),
        ("board.ticketEffort", json!("medium")),
        ("board.askInstanceId", json!("claude-b")),
        ("board.askPool", json!("tier")),
        ("board.askModel", json!("claude-sonnet-5-5")),
        ("board.askEffort", json!("low")),
        ("board.runtimeMode", json!("approval-required")),
        ("board.askPollSecs", json!(5)),
        ("board.askTimeoutMinutes", json!(20)),
        ("board.feed", json!(80)),
        ("board.settleAfterHours", json!(48)),
        ("board.reconcileActiveMinutes", json!(3)),
        ("board.reconcileIdleMinutes", json!(12)),
        ("board.openLinks", json!("tab")),
        ("board.remote.origin", json!("https://board.example.com")),
        ("board.remote.mode", json!("pairing")),
        ("board.remote.allowUsers.0", json!("alice@example.com")),
        ("board.remote.requirePairing", json!(true)),
        ("upstreams.0", json!({"name": "docs", "transport": "stdio", "stdio": {"command": "docs-mcp"}})),
        ("upstreams.0.stdio.args.0", json!("--quiet")),
        ("upstreams.0.stdio.env", json!({"LOG.LEVEL": "warn"})),
        ("upstreams.0.stdio.cwd", json!("/srv/docs")),
        ("upstreams.0.enabled", json!(false)),
        ("upstreams.0.timeout_ms", json!(30000)),
        ("upstreams.0.allow.0", json!("search*")),
        ("upstreams.0.deny.0", json!("delete*")),
        ("upstreams.1", json!({"name": "web", "transport": "http", "http": {"url": "https://mcp.example.com/mcp"}})),
        ("upstreams.1.http.headers", json!({"X-Team": "docs"})),
        ("upstreams.1.http.allow_insecure", json!(true)),
        ("flow", json!({"repos": ["acme/widgets"], "optIn": {"label": "flow"}, "maxTickets": 2})),
        ("flow.repos.1", json!("acme/tools")),
        ("flow.optIn.label", json!("ready")),
        ("flow.maxTickets", json!(3)),
        ("flow.launch", json!("auto")),
        ("trackers.jira.PROJ", json!("widgets")),
        ("trackers.hosts.\"jira.acme.example\"", json!("widgets")),
    ]
}

fn paths_of(value: &Value, at: &mut Vec<String>, out: &mut Vec<Vec<String>>) {
    if !at.is_empty() {
        out.push(at.clone());
    }
    let children: Vec<(String, &Value)> = match value {
        Value::Object(map) => map.iter().map(|(k, v)| (k.clone(), v)).collect(),
        Value::Array(items) => items.iter().enumerate().map(|(i, v)| (i.to_string(), v)).collect(),
        _ => Vec::new(),
    };
    for (key, child) in children {
        at.push(key);
        paths_of(child, at, out);
        at.pop();
    }
}

#[test]
fn every_key_can_be_read_and_changed_by_path() {
    let c = Cfg::absent();
    let table = every_kind_of_key();
    for (path, value) in &table {
        apply(&c.file, set(path, value.clone()));
        let all = effective(&Config::load(&c.file).unwrap());
        let got = lookup(&all, &field::parse(path).unwrap()).unwrap();
        match value {
            Value::Object(want) => {
                for (key, v) in want.iter().filter(|(_, v)| !v.is_object() && !v.is_array()) {
                    assert_eq!(&got[key], v, "{path}.{key}");
                }
            }
            _ => assert_eq!(&got, value, "{path}"),
        }
    }

    let all = effective(&Config::load(&c.file).unwrap());
    for key in all.as_object().unwrap().keys() {
        assert!(table.iter().any(|(p, _)| field::parse(p).unwrap()[0] == *key), "no case changes {key}");
    }
    for key in all["board"].as_object().unwrap().keys() {
        let path = format!("board.{key}");
        assert!(table.iter().any(|(p, _)| p.starts_with(&path)), "no case changes {path}");
    }

    let mut paths = Vec::new();
    paths_of(&all, &mut Vec::new(), &mut paths);
    let doc = Document::read(&c.file).unwrap();
    for path in paths {
        let value = lookup(&all, &path).unwrap();
        let shown = field::display(&path);
        assert_eq!(field::parse(&shown).unwrap(), path);
        let draft = doc.edit(&[Edit::Set { path, value }]).unwrap_or_else(|e| panic!("{shown}: {e}"));
        assert_eq!(effective(draft.config()), all, "{shown}");
    }
}

#[test]
fn optional_keys_read_as_their_defaults() {
    let c = Cfg::with(r#"{"pools": {"p": {"driver": "codex", "model": "m", "members": [{"instanceId": "a"}]}}}"#);
    assert_eq!(c.get("board.feed"), json!(50));
    assert_eq!(c.get("board.pool"), Value::Null);
    assert_eq!(c.get("board.remote"), Value::Null);
    assert_eq!(c.get("board.remote.mode"), json!("off"));
    assert_eq!(c.get("board.remote.allowUsers"), json!([]));
    assert_eq!(c.get("board.remote.requirePairing"), json!(false));
    assert_eq!(c.get("pools.p.mixed"), json!(false));
    assert_eq!(c.get("pools.p.effort"), Value::Null);
    assert_eq!(c.get("pools.p.moveOnLimit"), json!("ask"));
    assert_eq!(c.get("pools.p.members.0.weight"), json!(1));
    assert_eq!(c.get("pools.p.members.0.model"), Value::Null);
}

#[test]
fn an_invalid_edit_names_its_field() {
    let base = r#"{
  "pools": {
    "gpt-6.1": {"driver": "codex", "model": "m", "members": [{"instanceId": "a"}, {"instanceId": "b"}]}
  },
  "board": {"pool": "gpt-6.1"},
  "upstreams": [{"name": "docs", "transport": "stdio", "stdio": {"command": "docs-mcp"}}]
}
"#;
    let cases = [
        (set("board.feed", json!("lots")), "board.feed"),
        (set("board.nope", json!(1)), "board.nope"),
        (set("nope", json!(1)), "nope"),
        (set("board.feed.deeper", json!(1)), "board.feed"),
        (set("board.openLinks", json!("window")), "board.openLinks"),
        (set("board.pool", json!("ghost")), "board.pool"),
        (set("board.askPool", json!("ghost")), "board.askPool"),
        (set("board.askModel", json!(" ")), "board.askModel"),
        (set("board.askEffort", json!("")), "board.askEffort"),
        (set("board.askInstanceId", json!("")), "board.askInstanceId"),
        (set("board.reconcileActiveMinutes", json!(99)), "board.reconcileActiveMinutes"),
        (set("board.remote.mode", json!("pairing")), "board.remote.origin"),
        (set("board.remote.origin", json!("http://plain.example.com")), "board.remote.origin"),
        (set("board.remote.allowUsers", json!(["not a login"])), "board.remote.allowUsers.0"),
        (set("maxConcurrentRuns", json!(0)), "maxConcurrentRuns"),
        (set("maxTimeoutMs", json!(99)), "maxTimeoutMs"),
        (set("limitHoldMinutes", json!(0)), "limitHoldMinutes"),
        (set("link.down", json!(0)), "link.down"),
        (set("link.stall", json!(10)), "link.stall"),
        (set("wake", json!(["event", "note"])), "wake.1"),
        (set("roles.overseer.children.ticket.down", json!(900)), "roles.overseer.children.ticket.stall"),
        (set("roles.ticket.children.worker.down", json!(900)), "roles.ticket.children.worker.down"),
        (set("roles.overseer.children.ticket.bodyCap", json!(5)), "roles.overseer.children.ticket.bodyCap"),
        (set("roles.overseer.children.ticket.wake.0", json!("bogus")), "roles.overseer.children.ticket.wake.0"),
        (set("roles.ticket.children.ghost", json!({})), "roles.ticket.children.ghost"),
        (set("pools", json!([])), "pools"),
        (set("pools.empty", json!({"mixed": true, "members": []})), "pools.empty.members"),
        (set("pools.bare", json!({"mixed": true})), "pools.bare.members"),
        (set("pools.\"gpt-6.1\".strategy", json!("random")), "pools.\"gpt-6.1\".strategy"),
        (set("pools.\"gpt-6.1\".typo", json!(1)), "pools.\"gpt-6.1\".typo"),
        (set("pools.\"gpt-6.1\".members.1.weight", json!(0)), "pools.\"gpt-6.1\".members.1.weight"),
        (set("pools.\"gpt-6.1\".members.1.weight", json!("x")), "pools.\"gpt-6.1\".members.1.weight"),
        (set("pools.\"gpt-6.1\".members.1.instanceId", json!("a")), "pools.\"gpt-6.1\".members.1.instanceId"),
        (set("pools.\"gpt-6.1\".members.1.model", json!("n")), "pools.\"gpt-6.1\".members.1.model"),
        (set("pools.\"gpt-6.1\".members.7", json!({})), "pools.\"gpt-6.1\".members.7"),
        (set("upstreams.0.name", json!("wf")), "upstreams.0.name"),
        (set("upstreams.0.allow", json!(["*", ""])), "upstreams.0.allow.1"),
        (set("upstreams.0.stdio.env", json!({"API.TOKEN": "literal"})), "upstreams.0.stdio.env.\"API.TOKEN\""),
        (set("trackers.jira.\"bad-key\"", json!("widgets")), "trackers.jira.bad-key"),
        (set("trackers.jira.PROJ", json!(" ")), "trackers.jira.PROJ"),
        (
            set("trackers.hosts.\"https://jira.example.com\"", json!("widgets")),
            "trackers.hosts.\"https://jira.example.com\"",
        ),
        (set("trackers.hosts.\"jira.example.com\"", json!("")), "trackers.hosts.\"jira.example.com\""),
        (unset("pools.\"gpt-6.1\""), "board.pool"),
        (unset("pools.\"gpt-6.1\".model"), "pools.\"gpt-6.1\".model"),
        (unset("board.nope"), "board.nope"),
        (unset("deny.9"), "deny.9"),
    ];
    let c = Cfg::with(base);
    for (edit, want) in cases {
        let doc = Document::read(&c.file).unwrap();
        let what = format!("{edit:?}");
        let Rejected { errors } = doc.edit(&[edit]).expect_err(&what);
        assert_eq!(errors[0].field, want, "{what}: {errors:?}");
        let shown = Rejected { errors }.to_string();
        let last = field::parse(want).unwrap().pop().unwrap();
        assert!(shown.contains(want) || shown.contains(&last), "{what}: {shown}");
    }
    assert_eq!(c.text(), base);
}

#[test]
fn several_edits_apply_together_or_not_at_all() {
    let c = Cfg::with("{}\n");
    let doc = Document::read(&c.file).unwrap();
    let pool = json!({"driver": "codex", "model": "m", "members": [{"instanceId": "a"}]});
    let draft = doc.edit(&[set("board.pool", json!("p")), set("pools.p", pool)]).unwrap();
    assert_eq!(draft.changed_pools(), ["p"]);
    draft.save().unwrap();

    let doc = Document::read(&c.file).unwrap();
    assert!(doc.edit(&[set("board.feed", json!(9)), unset("pools.p")]).is_err());
    assert_eq!(Config::load(&c.file).unwrap().board.feed, 50);
}

#[test]
fn a_repair_changes_only_the_pools_it_edits() {
    let pool = |id: &str| json!({"mixed": true, "members": [{"instanceId": id, "model": "m"}]});
    let repaired = json!({"board": {"feed": 9}, "pools": {"kept": pool("a"), "moved": pool("b")}}).to_string();
    let invalid_elsewhere = json!({"board": {"feed": "lots"}, "pools": {"kept": pool("a"), "moved": pool("c")}});
    let invalid_pool = json!({"pools": {"kept": {"members": "none"}, "moved": pool("b")}});
    let cases = [
        (invalid_elsewhere.to_string(), vec!["moved"]),
        (invalid_pool.to_string(), vec!["kept"]),
        ("{\"pools\": ".to_string(), vec!["kept", "moved"]),
    ];
    for (before, want) in cases {
        let c = Cfg::with(&before);
        let doc = Document::read(&c.file).unwrap();
        assert_eq!(doc.replace(repaired.clone()).unwrap().changed_pools(), want, "{before}");
    }
}

#[test]
fn a_child_roles_inherited_wake_list_is_read_and_edited_by_index() {
    let c = Cfg::with(r#"{"wake": ["event", "gate"]}"#);
    let path = "roles.ticket.children.worker.wake";
    assert_eq!(c.get(path), Value::Null);
    assert_eq!(c.get(&format!("{path}.0")), json!("event"));
    c.ok(&["set", &format!("{path}.0"), "blocker"]);
    assert_eq!(c.get(path), json!(["blocker", "gate"]));
    assert_eq!(c.get("wake"), json!(["event", "gate"]));
}

#[test]
fn each_edit_in_a_batch_inherits_from_the_edits_before_it() {
    let c = Cfg::with(r#"{"deny": ["custom_a", "custom_b"]}"#);
    let doc = Document::read(&c.file).unwrap();
    let draft = doc.edit(&[unset("deny"), set("deny.0", json!("mine"))]).unwrap();
    assert_eq!(draft.config().deny, ["mine", "t3_environment_unlink"]);

    let edits = [
        set("board.pool", json!("later")),
        set("wake", json!(["handoff"])),
        set("roles.ticket.children.worker.wake.1", json!("gate")),
        set("pools.later", json!({"driver": "codex", "model": "m", "members": [{"instanceId": "a"}]})),
    ];
    let draft = doc.edit(&edits).unwrap();
    let wake = draft.config().roles["ticket"].children["worker"].wake.clone().unwrap();
    assert_eq!(wake, ["handoff", "gate"]);

    let c = Cfg::absent();
    let edits = [
        set("board.pool", json!("p")),
        set("upstreams.0", json!({"name": "docs", "transport": "stdio", "stdio": {"command": "docs-mcp"}})),
        set("upstreams.0.allow.1", json!("search*")),
        set("pools.p", json!({"driver": "codex", "model": "m", "members": [{"instanceId": "a"}]})),
    ];
    let doc = Document::read(&c.file).unwrap();
    let draft = doc.edit(&edits).unwrap();
    assert_eq!(draft.config().upstreams[0].allow, ["*", "search*"]);
}

#[test]
fn a_file_of_the_wrong_shape_is_rejected_without_a_panic() {
    for text in [
        r#"{"board": 5}"#,
        r#"{"roles": {"ticket": 5}}"#,
        r#"{"roles": {"ticket": {"children": {"worker": 5}}}}"#,
        "[]",
    ] {
        let c = Cfg::with(text);
        let doc = Document::read(&c.file).unwrap();
        assert!(doc.edit(&[set("bodyCap", json!(900))]).is_err(), "{text}");
    }
}

#[test]
fn a_write_after_the_file_changed_is_refused() {
    let c = Cfg::with("{}\n");
    let doc = Document::read(&c.file).unwrap();
    let seen = doc.revision().clone();
    let draft = doc.edit(&[set("board.feed", json!(9))]).unwrap();
    let theirs = "{\"bodyCap\": 900}\n";
    std::fs::write(&c.file, theirs).unwrap();
    assert!(draft.save().unwrap_err().is::<Stale>());
    assert_eq!(c.text(), theirs);
    assert_ne!(*Document::read(&c.file).unwrap().revision(), seen);

    let same = doc.edit(&[unset("board.feed")]).unwrap();
    assert!(!same.is_changed());
    assert!(same.save().unwrap_err().is::<Stale>(), "an edit that changes nothing still reports a stale read");

    let gone = Document::read(&c.file).unwrap();
    std::fs::remove_file(&c.file).unwrap();
    assert!(gone.edit(&[set("board.feed", json!(9))]).unwrap().save().unwrap_err().is::<Stale>());
    assert!(!c.file.exists());

    let absent = Document::read(&c.file).unwrap();
    std::fs::write(&c.file, theirs).unwrap();
    assert!(absent.edit(&[set("board.feed", json!(9))]).unwrap().save().unwrap_err().is::<Stale>());
    assert_eq!(c.text(), theirs);
}

#[test]
fn concurrent_writers_lose_no_saved_edit() {
    let c = Cfg::with("{}\n");
    let start = std::sync::Barrier::new(8);
    let saved: Vec<String> = std::thread::scope(|s| {
        let writers: Vec<_> = (0..8)
            .map(|i| {
                let (file, start) = (&c.file, &start);
                s.spawn(move || {
                    let key = format!("p{i}");
                    let pool = json!({"mixed": true, "members": [{"instanceId": "a", "model": "m"}]});
                    let doc = Document::read(file).unwrap();
                    let draft = doc.edit(&[set(&format!("pools.{key}"), pool)]).unwrap();
                    start.wait();
                    match draft.save() {
                        Ok(_) => Some(key),
                        Err(e) if e.is::<Stale>() => None,
                        Err(e) => panic!("{e:#}"),
                    }
                })
            })
            .collect();
        writers.into_iter().filter_map(|w| w.join().unwrap()).collect()
    });
    assert_eq!(saved.len(), 1);
    assert_eq!(Config::load(&c.file).unwrap().pools.keys().cloned().collect::<Vec<_>>(), saved);
}

#[test]
fn saving_returns_the_next_revision() {
    let c = Cfg::absent();
    let saved = Document::read(&c.file).unwrap().edit(&[set("board.feed", json!(9))]).unwrap().save().unwrap();
    assert_eq!(*Document::read(&c.file).unwrap().revision(), saved);
}

fn hold_lock(file: &Path) -> std::fs::File {
    let name = format!(".{}.t3cm-lock", file.file_name().unwrap().to_string_lossy());
    let path = file.with_file_name(name);
    let lock = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(path).unwrap();
    lock.lock().unwrap();
    lock
}

fn loses_nothing_saved_under_the_lock(file: &Path, write: impl FnOnce() + Send) {
    let before = std::fs::read_to_string(file).unwrap();
    let lock = hold_lock(file);
    let during = std::thread::scope(|s| {
        s.spawn(write);
        std::thread::sleep(std::time::Duration::from_millis(500));
        let during = std::fs::read_to_string(file).unwrap();
        std::fs::write(file, before.replace("900", "901")).unwrap();
        drop(lock);
        during
    });
    assert_eq!(during, before, "wrote while another writer held the lock");
    assert_eq!(Config::load(file).unwrap().body_cap, 901, "lost the other writer's save");
}

#[test]
fn saving_the_ask_model_takes_the_config_lock() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let h = rt.block_on(async { H::new() });
    let file = h.dir.path().join("t3cm-config.json");
    std::fs::write(&file, r#"{"bodyCap": 900}"#).unwrap();
    let body = json!({"mode": "model", "provider": "claudeAgent", "model": "claude-opus-5-5"});
    loses_nothing_saved_under_the_lock(&file, || {
        let _ = rt.block_on(h.app.wf.set_ask_model(serde_json::from_value(body).unwrap()));
    });
}

#[test]
fn board_expose_takes_the_config_lock() {
    let c = Cfg::with(r#"{"bodyCap": 900}"#);
    let ts = FakeTailscale::default();
    ts.set_host_login("alice@acme.example");
    let opts = ExposeOptions { https_port: 47732, local_port: 47731, allow_users: vec![], require_pairing: false };
    loses_nothing_saved_under_the_lock(&c.file, || {
        let _ = expose_tailscale(&ts, &c.file, &opts);
    });
}

#[test]
fn board_unexpose_takes_the_config_lock() {
    let remote = r#"{"bodyCap": 900, "board": {"remote": {"mode": "pairing", "origin": "https://board.example.com"}}}"#;
    let c = Cfg::with(remote);
    let ts = FakeTailscale::default();
    loses_nothing_saved_under_the_lock(&c.file, || {
        let _ = unexpose_board(&ts, &c.file, None, 47731);
    });
}

#[test]
fn the_ask_and_remote_writers_keep_the_files_layout() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let h = rt.block_on(async { H::new() });
    let file = h.dir.path().join("t3cm-config.json");
    let deny = "\"deny\":  [\"t3_project_delete\"]";
    std::fs::write(&file, format!("{{\n    {deny},\n    \"board\": null\n}}\n")).unwrap();
    let ts = FakeTailscale::default();
    ts.set_host_login("alice@acme.example");
    let opts = ExposeOptions { https_port: 47732, local_port: 47731, allow_users: vec![], require_pairing: false };
    expose_tailscale(&ts, &file, &opts).unwrap();
    let body = json!({"mode": "model", "provider": "claudeAgent", "model": "claude-opus-5-5"});
    rt.block_on(h.app.wf.set_ask_model(serde_json::from_value(body).unwrap())).unwrap();

    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.contains(deny), "{text}");
    let saved = Config::load(&file).unwrap().board;
    assert_eq!(saved.ask_model, "claude-opus-5-5");
    assert!(saved.active_remote().is_some(), "{text}");
}

#[test]
fn a_rejected_ask_save_does_not_repeat_the_files_values() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let h = rt.block_on(async { H::new() });
    let file = h.dir.path().join("t3cm-config.json");
    let body = json!({"mode": "model", "provider": "claudeAgent", "model": "claude-opus-5-5"});
    for text in [r#"{"board": "private-value"}"#, r#""private-value""#, r#"{"board": ["private-value"]}"#] {
        std::fs::write(&file, text).unwrap();
        let body = serde_json::from_value(body.clone()).unwrap();
        let err = format!("{:#}", rt.block_on(h.app.wf.set_ask_model(body)).unwrap_err());
        assert!(!err.contains("private-value"), "a remote board sees this error: {err}");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), text);
    }
}

#[cfg(unix)]
#[test]
fn a_save_keeps_a_symlink_and_the_files_mode() {
    use std::os::unix::fs::PermissionsExt;
    let c = Cfg::absent();
    let target = c.dir.path().join("dotfiles.json");
    std::fs::write(&target, "{}\n").unwrap();
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o640)).unwrap();
    std::os::unix::fs::symlink(&target, &c.file).unwrap();
    apply(&c.file, set("board.feed", json!(9)));
    assert!(c.file.is_symlink());
    assert_eq!(std::fs::metadata(&target).unwrap().permissions().mode() & 0o777, 0o640);
    assert_eq!(Config::load(&target).unwrap().board.feed, 9);

    std::fs::remove_file(&target).unwrap();
    assert!(Document::read(&c.file).unwrap_err().to_string().contains("symlink"));
}

#[test]
fn cli_gets_sets_and_unsets() {
    let c = Cfg::absent();
    assert_eq!(c.get("board.feed"), json!(50));
    let out = c.ok(&["set", "board.feed", "80"]);
    assert!(out.contains("Updated config") && out.contains("Restart the service"), "{out}");
    assert_eq!(c.text(), "{\n  \"board\": {\n    \"feed\": 80\n  }\n}\n");
    assert_eq!(c.get("board.feed"), json!(80));

    c.ok(&["set", "board.askModel", "claude-sonnet-5-5"]);
    assert_eq!(c.get("board.askModel"), json!("claude-sonnet-5-5"));
    c.ok(&["set", "board.askModel", "\"5\""]);
    assert_eq!(c.get("board.askModel"), json!("5"));
    c.ok(&["unset", "board.askModel"]);
    assert_eq!(c.get("board.askModel"), json!("claude-haiku-5-5"));
    assert!(c.ok(&["unset", "board.askModel"]).contains("No change"));

    let all: Value = serde_json::from_str(&c.ok(&["get"])).unwrap();
    assert_eq!(all["board"]["feed"], json!(80));
    let out = c.run(&["get", "board.nope"]);
    assert!(!out.status.success() && stderr(&out).contains("board.nope"), "{}", stderr(&out));
}

#[test]
fn cli_edits_inside_a_default_list() {
    let c = Cfg::absent();
    c.ok(&["set", "deny.2", "t3_thread_interrupt"]);
    assert_eq!(c.get("deny"), json!(["t3_project_delete", "t3_environment_unlink", "t3_thread_interrupt"]));
    c.ok(&["unset", "deny.0"]);
    assert_eq!(c.get("deny"), json!(["t3_environment_unlink", "t3_thread_interrupt"]));

    let c = Cfg::absent();
    c.ok(&["set", "board.remote.allowUsers.0", "alice@example.com"]);
    assert_eq!(c.get("board.remote.allowUsers"), json!(["alice@example.com"]));

    let c = Cfg::absent();
    c.ok(&["unset", "roles.overseer.children.ticket.wake.0"]);
    assert_eq!(c.get("roles.overseer.children.ticket.wake.0"), json!("blocker"));
    assert_eq!(c.get("roles.overseer.children.ticket.down"), json!(75));
}

#[test]
fn cli_keeps_the_files_layout() {
    let text =
        "{\n    \"deny\":  [\"t3_project_delete\"],\n\n    \"board\": { \"settleAfterHours\": 48, \"feed\": 50 }\n}\n";
    let c = Cfg::with(text);
    c.ok(&["set", "board.feed", "80"]);
    assert_eq!(c.text(), text.replace("50", "80"));
}

#[test]
fn cli_rejects_an_invalid_edit_and_leaves_the_file() {
    let text = "{\"board\":   {\"feed\": 10}}";
    let c = Cfg::with(text);
    let cases: [(&[&str], &str); 6] = [
        (&["set", "board.feed", "lots"], "board.feed"),
        (&["set", "board.feed", "-5"], "board.feed"),
        (&["set", "board.pool", "ghost"], "board.pool"),
        (&["set", "link.stall", "1"], "link.stall"),
        (&["set", "board.typo", "1"], "board.typo"),
        (&["unset", "board.typo"], "board.typo"),
    ];
    for (args, name) in cases {
        let out = c.run(args);
        let err = stderr(&out);
        assert!(!out.status.success(), "{args:?}");
        assert!(err.contains(name) && err.contains("is unchanged"), "{args:?}: {err}");
        assert_eq!(c.text(), text, "{args:?}");
    }
}

#[test]
fn cli_flags_work_on_either_side_of_the_value() {
    let c = Cfg::absent();
    let pool = r#"{"mixed": true, "members": [{"instanceId": "a", "model": "m"}]}"#;
    c.ok(&["set", "pools.p", pool, "--no-t3"]);
    c.ok(&["set", "--no-t3", "pools.q", pool]);
    c.ok(&["set", "pools.r", "--no-t3", pool]);
    for name in ["p", "q", "r"] {
        assert_eq!(c.get(&format!("pools.{name}.members.0.instanceId")), json!("a"));
    }
}

#[test]
fn cli_validates_the_file() {
    let c = Cfg::absent();
    assert!(c.ok(&["validate", "--no-t3"]).contains("built-in defaults"));
    std::fs::write(&c.file, "{\"board\": {\"feed\": 9}}").unwrap();
    assert!(c.ok(&["validate", "--no-t3"]).contains("is valid"));
    for (text, name) in [("{\"board\": {\"feed\": \"x\"}}", "board.feed"), ("{\"board\": ", "not valid JSON")] {
        std::fs::write(&c.file, text).unwrap();
        let out = c.run(&["validate", "--no-t3"]);
        assert!(!out.status.success() && stderr(&out).contains(name), "{}", stderr(&out));
    }
}

#[cfg(unix)]
#[test]
fn cli_edit_keeps_only_a_valid_result() {
    let c = Cfg::with("{\"board\": {\"feed\": 10}}\n");
    let next = c.dir.path().join("next file.json");
    let edit = |editor: &str| {
        let mut cmd = c.command(&["edit", "--no-t3"]);
        cmd.env("EDITOR", editor).stdin(std::process::Stdio::null()).output().unwrap()
    };
    let drafts = || {
        std::fs::read_dir(c.dir.path())
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.file_name().unwrap().to_string_lossy().starts_with(".config-edit."))
            .collect::<Vec<_>>()
    };

    let out = edit("true");
    assert!(out.status.success() && String::from_utf8_lossy(&out.stdout).contains("No change"));
    assert!(!edit("false").status.success());
    assert!(drafts().is_empty(), "an untouched copy is removed");

    let valid = "{ \"board\": { \"feed\": 77 } }\n";
    std::fs::write(&next, valid).unwrap();
    let out = edit(&format!("cp '{}'", next.display()));
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(c.text(), valid);
    assert!(drafts().is_empty());

    let invalid = "{ \"board\": { \"feed\": \"x\" } }\n";
    std::fs::write(&next, invalid).unwrap();
    let out = edit(&format!("cp '{}'", next.display()));
    let err = stderr(&out);
    assert!(!out.status.success() && err.contains("board.feed"), "{err}");
    assert_eq!(c.text(), valid);
    let kept = drafts();
    assert_eq!(kept.len(), 1);
    assert!(err.contains(&*kept[0].to_string_lossy()), "{err}");
    assert_eq!(std::fs::read_to_string(&kept[0]).unwrap(), invalid);
}

#[cfg(unix)]
#[test]
fn cli_edit_repairs_a_broken_file() {
    let c = Cfg::with("{\"board\": ");
    let next = c.dir.path().join("next.json");
    std::fs::write(&next, "{}\n").unwrap();
    let out = c.command(&["edit", "--no-t3"]).env("EDITOR", format!("cp '{}'", next.display())).output().unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(c.text(), "{}\n");
}

#[test]
fn cli_edit_needs_an_editor() {
    let out = Cfg::absent().run(&["edit", "--no-t3"]);
    assert!(!out.status.success() && stderr(&out).contains("EDITOR"), "{}", stderr(&out));
}

#[tokio::test(flavor = "multi_thread")]
async fn cli_checks_pool_members_against_t3() {
    let h = H::with_tools();
    h.fake.set_providers(vec![
        provider("claude-a", "claudeAgent", &["claude-opus-5-5"]),
        provider("codex-a", "codex", &["gpt-6-astra"]),
    ]);
    h.app.refresh_catalog().await.unwrap();
    let (listener, addr) = server::bind(0).await.unwrap();
    let app = h.app.clone();
    tokio::spawn(async move {
        let _ = server::serve(app, listener, std::future::pending()).await;
    });
    let port = addr.port().to_string();
    tokio::task::spawn_blocking(move || {
        let c = Cfg::absent();
        let pool = |id: &str| {
            format!(r#"{{"driver":"claudeAgent","model":"claude-opus-5-5","members":[{{"instanceId":"{id}"}}]}}"#)
        };
        for (id, why) in [("ghost", "not a T3 provider instance"), ("codex-a", "is a codex instance")] {
            let out = c.run(&["set", "pools.opus", &pool(id), "--port", &port]);
            let err = stderr(&out);
            assert!(!out.status.success() && err.contains(why), "{err}");
            assert!(err.contains("pools.opus.members.0.instanceId") && !c.file.exists(), "{err}");
        }
        let out = c.run(&["set", "pools.opus", &pool("claude-a"), "--port", &port]);
        assert!(out.status.success(), "{}", stderr(&out));
        assert!(!stderr(&out).contains("not checked"), "{}", stderr(&out));

        c.ok(&["set", "pools.stale", &pool("ghost"), "--no-t3"]);
        let out = c.run(&["validate", "--port", &port]);
        assert!(!out.status.success() && stderr(&out).contains("ghost"), "{}", stderr(&out));
        c.ok(&["set", "board.feed", "90", "--port", &port]);
    })
    .await
    .unwrap();
}

#[test]
fn cli_writes_with_a_warning_when_t3_cannot_be_asked() {
    let c = Cfg::absent();
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port().to_string();
    let pool = r#"{"mixed":true,"members":[{"instanceId":"a","model":"m"}]}"#;
    let out = c.run(&["set", "pools.p", pool, "--port", &port]);
    assert!(out.status.success() && stderr(&out).contains("not checked against T3"), "{}", stderr(&out));
}
