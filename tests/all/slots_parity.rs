#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::common::{self, H};
use serde_json::{Value, json};
use t3_codemode::app::ExecuteRequest;
use t3_codemode::config::Config;
use t3_codemode::fake::{project, thread};
use t3_codemode::slots::Folders;
use t3_codemode::wf_slots::Reported;

const CALLER: &str = "mcp:slots-caller";
const T3WF: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/skills/t3-workflow/scripts/t3wf.py");

#[derive(Clone, Copy)]
enum Caps {
    Claude,
    NoClaude,
    Down,
}

fn claude_caps(no_claude: bool) -> Vec<Value> {
    let claude = if no_claude {
        json!({"providerInstanceId": "claudeAgent", "driverKind": "claudeAgent",
               "constraints": ["Provider instance is disabled."], "models": []})
    } else {
        json!({"providerInstanceId": "claudeAgent", "driverKind": "claudeAgent", "constraints": [], "models": [
            {"id": "claude-opus-5-5", "options": [{"id": "effort", "values": ["low", "medium", "high", "max"]}]}]})
    };
    vec![
        claude,
        json!({"providerInstanceId": "claude-2", "driverKind": "claudeAgent", "constraints": [], "models": [
            {"id": "claude-opus-5-5", "options": [{"id": "effort", "values": ["low", "medium", "high", "max"]}]}]}),
        json!({"providerInstanceId": "codex", "driverKind": "codex", "constraints": [], "models": [
            {"id": "gpt-6.2-sol", "options": [{"id": "effort", "values": ["low", "medium", "high"]}]}]}),
    ]
}

struct Fx {
    tmp: PathBuf,
    home: PathBuf,
    claude: PathBuf,
    repo: PathBuf,
    config: PathBuf,
}

impl Fx {
    fn agents(&self) -> PathBuf {
        self.home.join(".agents").join("skills")
    }

    fn codex(&self) -> PathBuf {
        self.home.join(".codex")
    }

    fn skill_as(&self, base: &Path, name: &str, inline: bool, fm_name: Option<&str>) -> PathBuf {
        let d = base.join(name);
        std::fs::create_dir_all(&d).unwrap();
        let extra = if inline { "disable-model-invocation: true\n" } else { "" };
        let p = d.join("SKILL.md");
        std::fs::write(&p, format!("---\nname: {}\ndescription: test\n{extra}---\n\nBody.\n", fm_name.unwrap_or(name)))
            .unwrap();
        p
    }

    fn skill(&self, base: &Path, name: &str) -> PathBuf {
        self.skill_as(base, name, false, None)
    }

    fn inline(&self, base: &Path, name: &str) -> PathBuf {
        self.skill_as(base, name, true, None)
    }

    fn plugin_skill(&self, plugin: &str, name: &str, version: &str) -> (PathBuf, PathBuf) {
        let root = self.claude.join("plugins").join("cache").join("acme").join(plugin).join(version);
        let p = self.skill(&root.join("skills"), name);
        (root, p)
    }

    fn install_plugins(&self, entries: &[(&str, &Path)]) {
        let plugins: serde_json::Map<String, Value> = entries
            .iter()
            .map(|(p, root)| (format!("{p}@acme"), json!([{"scope": "user", "installPath": root.to_string_lossy()}])))
            .collect();
        let path = self.claude.join("plugins").join("installed_plugins.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, json!({"version": 2, "plugins": plugins}).to_string()).unwrap();
    }

    fn write_config(&self, cfg: Value) {
        self.write_raw_config(&cfg.to_string());
    }

    fn write_raw_config(&self, text: &str) {
        std::fs::create_dir_all(self.config.parent().unwrap()).unwrap();
        std::fs::write(&self.config, text).unwrap();
    }

    fn agents_md(&self, block: &str) {
        std::fs::write(
            self.repo.join("AGENTS.md"),
            format!("# Repo\n\nIntro.\n\n## Agent workflow\n\n{block}\n## Other\n\n- Review: ignored-skill\n"),
        )
        .unwrap();
    }

    fn enable_all(&self, keys: &[&str]) {
        let on: serde_json::Map<String, Value> = keys.iter().map(|k| (k.to_string(), json!(true))).collect();
        std::fs::create_dir_all(&self.claude).unwrap();
        std::fs::write(self.claude.join("settings.json"), json!({"enabledPlugins": on}).to_string()).unwrap();
    }
}

struct Case {
    name: &'static str,
    caps: Caps,
    build: fn(&Fx),
}

const LMS_SHAPED: &str = "- Tracker: github-issues (see Backlog)\n\
- Delivery: tag-deploy (see Deploying)\n\
- Deploy skill: `deploy` (`content/v*` when only `training/` changed, else `deploy/v*`)\n\
- Merge method: rebase\n\
- PR format: the `pr` skill\n\
- Checks: `npm test` (runs `tsc` first) and `npm run build`\n\
- In-progress label: none\n\
- Identity: the user's own; no bot\n";

fn real_shaped(f: &Fx) {
    let (cbrip, _) = f.plugin_skill("cbrip", "sol-loop", "1.40.0");
    f.skill(&cbrip.join("skills"), "aws-sso-login");
    f.skill(&cbrip.join("skills"), "deploy");
    let (voice, _) = f.plugin_skill("paul-otto", "paul-voice", "1.2.0");
    f.install_plugins(&[("cbrip", &cbrip), ("paul-otto", &voice)]);
    f.enable_all(&["cbrip@acme", "paul-otto@acme"]);
    f.skill(&f.agents(), "pr");
    f.inline(&f.agents(), "unslop");
    f.skill(&f.agents(), "t3-worker");
    f.write_config(json!({
        "version": 1,
        "writtenBy": "0.6.0",
        "steps": ["slot:review", "slot:passes", "slot:credentials", "slot:delivery", "slot:models", "rule:filed-issues"],
        "review": "cbrip:sol-loop",
        "passes": ["pr", "paul-otto:paul-voice", "unslop"],
        "credentials": [{"match": "ExpiredToken|Token has expired|SSO session|aws sso login", "skill": "cbrip:aws-sso-login"}],
        "delivery": {"mode": "merge"},
        "models": {"ticket": {"provider": "claude-2", "model": "claude-opus-5-5", "effort": "high"},
                   "worker": {"provider": "claude-2", "model": "claude-opus-5-5", "effort": "high"}}
    }));
}

fn cases() -> Vec<Case> {
    vec![
        Case {
            name: "namespaced and bare sol-loop",
            caps: Caps::Claude,
            build: |f| {
                let (root, _) = f.plugin_skill("cbrip", "sol-loop", "1.0.0");
                f.install_plugins(&[("cbrip", &root)]);
                f.skill(&f.agents(), "sol-loop");
                f.write_config(json!({"passes": ["sol-loop", "cbrip:sol-loop"]}));
            },
        },
        Case {
            name: "bare name matching only namespaces is ambiguous",
            caps: Caps::Claude,
            build: |f| {
                let (cbrip, _) = f.plugin_skill("cbrip", "sol-loop", "1.0.0");
                let (tools, _) = f.plugin_skill("tools", "sol-loop", "1.0.0");
                f.install_plugins(&[("cbrip", &cbrip), ("tools", &tools)]);
                f.write_config(json!({"review": "sol-loop"}));
            },
        },
        Case {
            name: "a recorded full name wins",
            caps: Caps::Claude,
            build: |f| {
                let (root, _) = f.plugin_skill("cbrip", "sol-loop", "1.0.0");
                f.install_plugins(&[("cbrip", &root)]);
                f.skill(&f.agents(), "sol-loop");
                f.write_config(json!({"review": "cbrip:sol-loop"}));
            },
        },
        Case {
            name: "inline skill",
            caps: Caps::Claude,
            build: |f| {
                f.inline(&f.claude.join("skills"), "unslop");
                f.write_config(json!({"passes": ["unslop"]}));
            },
        },
        Case {
            name: "every install location",
            caps: Caps::Claude,
            build: |f| {
                f.skill(&f.claude.join("skills"), "a");
                f.skill(&f.agents(), "b");
                f.skill(&f.repo.join(".claude").join("skills"), "c");
                f.skill(&f.codex().join("skills"), "d");
                let (root, _) = f.plugin_skill("tools", "e", "1.0.0");
                f.install_plugins(&[("tools", &root)]);
                f.write_config(json!({"passes": ["a", "b", "c", "d", "e"]}));
            },
        },
        Case {
            name: "a symlinked install counts once",
            caps: Caps::Claude,
            build: |f| {
                f.skill(&f.agents(), "sol-loop");
                std::fs::create_dir_all(f.claude.join("skills")).unwrap();
                std::os::unix::fs::symlink(f.agents().join("sol-loop"), f.claude.join("skills").join("sol-loop"))
                    .unwrap();
                f.write_config(json!({"review": "sol-loop"}));
            },
        },
        Case {
            name: "newest cached version without installed plugins",
            caps: Caps::Claude,
            build: |f| {
                f.plugin_skill("cbrip", "sol-loop", "1.9.0");
                f.plugin_skill("cbrip", "sol-loop", "1.10.0");
                f.write_config(json!({"passes": ["sol-loop", "cbrip:sol-loop"]}));
            },
        },
        Case {
            name: "same-named installs",
            caps: Caps::Claude,
            build: |f| {
                f.skill(&f.claude.join("skills"), "sol-loop");
                f.skill(&f.codex().join("skills"), "sol-loop");
                f.write_config(json!({"review": "sol-loop"}));
            },
        },
        Case {
            name: "a bare symlink to a plugin skill keeps the plugin name",
            caps: Caps::Claude,
            build: |f| {
                let (root, p) = f.plugin_skill("cbrip", "sol-loop", "1.0.0");
                f.install_plugins(&[("cbrip", &root)]);
                std::fs::create_dir_all(f.claude.join("skills")).unwrap();
                std::os::unix::fs::symlink(p.parent().unwrap(), f.claude.join("skills").join("sol-loop")).unwrap();
                f.write_config(json!({"review": "cbrip:sol-loop", "passes": ["sol-loop"]}));
            },
        },
        Case {
            name: "codex plugin cache",
            caps: Caps::Claude,
            build: |f| {
                let root = f.codex().join("plugins").join("cache").join("acme").join("cbrip");
                f.skill(&root.join("1.0.0").join("skills"), "sol-loop");
                f.skill(&root.join("1.2.0").join("skills"), "sol-loop");
                f.write_config(json!({"review": "cbrip:sol-loop"}));
            },
        },
        Case {
            name: "a missing full name lists other matches",
            caps: Caps::Claude,
            build: |f| {
                f.skill(&f.agents(), "sol-loop");
                f.write_config(json!({"review": "cbrip:sol-loop", "passes": ["nothing-here"]}));
            },
        },
        Case {
            name: "frontmatter name differs from folder",
            caps: Caps::Claude,
            build: |f| {
                f.skill_as(&f.agents(), "folder-x", false, Some("named-y"));
                f.write_config(json!({"passes": ["folder-x", "named-y"]}));
            },
        },
        Case { name: "no config yields the defaults", caps: Caps::Claude, build: |_| {} },
        Case {
            name: "default review uses a found sol-loop",
            caps: Caps::Claude,
            build: |f| {
                f.skill(&f.agents(), "sol-loop");
            },
        },
        Case { name: "default model without claude", caps: Caps::NoClaude, build: |_| {} },
        Case {
            name: "default model when T3 is unreachable",
            caps: Caps::Down,
            build: |f| {
                f.write_config(json!({"models": {"worker": {"model": "claude-opus-5-5"}}}));
            },
        },
        Case {
            name: "user config sets every slot",
            caps: Caps::Claude,
            build: |f| {
                f.inline(&f.claude.join("skills"), "unslop");
                f.skill(&f.claude.join("skills"), "pr");
                f.skill(&f.claude.join("skills"), "aws-sso-login");
                f.skill(&f.claude.join("skills"), "deploy");
                f.write_config(json!({"version": 1, "review": null, "passes": ["unslop", "pr"],
                "credentials": [{"match": "ExpiredToken", "skill": "aws-sso-login"}],
                "delivery": {"mode": "tag-deploy", "skill": "deploy"},
                "models": {"ticket": {"model": "claude-opus-5-5", "effort": "max"},
                           "worker": {"provider": "codex", "model": "gpt-6.2-sol", "effort": "high"}}}));
            },
        },
        Case {
            name: "a credentials pattern with Python's ASCII flag",
            caps: Caps::Claude,
            build: |f| {
                f.skill(&f.claude.join("skills"), "aws-sso-login");
                f.write_config(json!({"credentials": [{"match": "(?a)^token expired$", "skill": "aws-sso-login"}]}));
            },
        },
        Case {
            name: "the repo block overrides the user config",
            caps: Caps::Claude,
            build: |f| {
                f.inline(&f.claude.join("skills"), "unslop");
                f.skill(&f.claude.join("skills"), "deploy");
                f.write_config(json!({"passes": ["unslop"], "review": "sol-loop", "delivery": {"mode": "merge"},
                "models": {"worker": {"model": "claude-opus-5-5", "effort": "high"}}}));
                f.agents_md(
                    "- Tracker: github-issues\n- Review: none\n- Passes: none\n\
                         - Delivery: tag-deploy (see Deploying)\n\
                         - Deploy skill: `deploy` (`content/v*` when only `training/` changed)\n\
                         - Worker model: claude-opus-5-5, max effort\n",
                );
            },
        },
        Case {
            name: "the repo block reads passes and credential renewal",
            caps: Caps::Claude,
            build: |f| {
                f.agents_md(
                    "- Passes: `paul-voice`, `unslop`, then `pr`\n\
                         - Credential renewal: `ExpiredToken|sso` -> `aws-sso-login`; `401` -> gh-login\n",
                );
            },
        },
        Case {
            name: "a repo tag-deploy keeps the user deploy skill",
            caps: Caps::Claude,
            build: |f| {
                f.skill(&f.claude.join("skills"), "ship");
                f.write_config(json!({"delivery": {"mode": "merge", "skill": "ship"}}));
                f.agents_md("- Delivery: tag-deploy\n");
            },
        },
        Case {
            name: "a missing skill is skipped",
            caps: Caps::Claude,
            build: |f| {
                f.write_config(json!({"passes": ["gone"]}));
            },
        },
        Case {
            name: "a pool slot",
            caps: Caps::Claude,
            build: |f| {
                f.write_config(json!({"models": {"worker": {"pool": "opus"}}}));
            },
        },
        Case {
            name: "a repo pool replaces a user model",
            caps: Caps::Claude,
            build: |f| {
                f.write_config(json!({"models": {"ticket": {"pool": "opus"},
                "worker": {"provider": "codex", "model": "gpt-6.2-sol", "effort": "high"}}}));
                f.agents_md("- Ticket model: claude-opus-5-5, max effort\n- Worker model: pool `opus`\n");
            },
        },
        Case {
            name: "a model named like a pool",
            caps: Caps::Claude,
            build: |f| {
                f.agents_md("- Worker model: pool-custom, high effort\n");
            },
        },
        Case {
            name: "a model with a colon",
            caps: Caps::Claude,
            build: |f| {
                f.agents_md("- Worker model: pool:custom, high effort\n");
            },
        },
        Case {
            name: "a pool with a colon",
            caps: Caps::Claude,
            build: |f| {
                f.agents_md("- Worker model: pool: `opus`\n");
            },
        },
        Case {
            name: "a removed full name names what is installed",
            caps: Caps::Claude,
            build: |f| {
                f.skill(&f.agents(), "sol-loop");
                f.write_config(json!({"review": "cbrip:sol-loop"}));
            },
        },
        Case {
            name: "models are checked through T3",
            caps: Caps::Claude,
            build: |f| {
                f.write_config(json!({"models": {"ticket": {"model": "no-such-model"},
                "worker": {"model": "claude-opus-5-5", "effort": "turbo"}}}));
            },
        },
        Case {
            name: "a disabled provider does not count",
            caps: Caps::NoClaude,
            build: |f| {
                f.write_config(json!({"models": {"worker": {"model": "claude-opus-5-5"}}}));
            },
        },
        Case {
            name: "a removed deploy skill in merge mode",
            caps: Caps::Claude,
            build: |f| {
                f.write_config(json!({"delivery": {"mode": "merge", "skill": "ship"}}));
            },
        },
        Case {
            name: "tag-deploy needs a deploy skill",
            caps: Caps::Claude,
            build: |f| {
                f.agents_md("- Delivery: tag-deploy\n");
            },
        },
        Case {
            name: "a bad shape is named",
            caps: Caps::Claude,
            build: |f| {
                f.write_config(json!({"version": 1, "reviewer": "x", "passes": "unslop",
                "credentials": [{"match": "(", "skill": "a"}],
                "delivery": {"mode": "ship"}, "models": {"boss": {"model": "m"}}}));
            },
        },
        Case {
            name: "a pool stands alone and needs a name",
            caps: Caps::Claude,
            build: |f| {
                f.write_config(
                    json!({"models": {"ticket": {"pool": "opus", "effort": "max"}, "worker": {"pool": ""}}}),
                );
            },
        },
        Case { name: "invalid json is named", caps: Caps::Claude, build: |f| f.write_raw_config("{not json") },
        Case {
            name: "a bad repo block is named",
            caps: Caps::Claude,
            build: |f| {
                f.agents_md("- Delivery: carrier-pigeon\n- Credential renewal: no arrow here\n");
            },
        },
        Case { name: "the real config with this repo (no block)", caps: Caps::Claude, build: real_shaped },
        Case {
            name: "the real config with a deploying repo's block",
            caps: Caps::Claude,
            build: |f| {
                real_shaped(f);
                f.agents_md(LMS_SHAPED);
            },
        },
    ]
}

#[derive(Clone, Copy, Debug)]
enum Instance {
    Default,
    OwnFolder,
}

fn normalize(v: &mut Value, config: &str) {
    let Some(problems) = v["problems"].as_array_mut() else { return };
    for p in problems {
        let Some(s) = p.as_str() else { continue };
        let mut s = s.to_string();
        if let Some(i) = s.find("not a regular expression:") {
            s.truncate(i + "not a regular expression:".len());
        } else if s.starts_with(&format!("{config}: ")) && !s.contains("top level") {
            s = format!("{config}: <parse error>");
        }
        *p = json!(s);
    }
}

fn python(fx: &Fx, inst: Instance, caps: Caps) -> Value {
    let fake = fx.tmp.join("fake-t3.json");
    let mut data = json!({"threads": {}});
    match caps {
        Caps::Claude => data["capabilities"] = json!({"providers": claude_caps(false)}),
        Caps::NoClaude => data["capabilities"] = json!({"providers": claude_caps(true)}),
        Caps::Down => {}
    }
    std::fs::write(&fake, data.to_string()).unwrap();
    let mut cmd = Command::new("python3");
    cmd.arg(T3WF).args(["slots", "--json", "--repo"]).arg(&fx.repo).current_dir(&fx.tmp).env_clear();
    for k in ["PATH", "LANG", "LC_ALL", "SYSTEMROOT", "TMPDIR"] {
        if let Ok(v) = std::env::var(k) {
            cmd.env(k, v);
        }
    }
    cmd.env("HOME", &fx.home)
        .env("XDG_CONFIG_HOME", fx.home.join(".config"))
        .env("XDG_STATE_HOME", fx.tmp.join("state"))
        .env("T3WF_FAKE_T3", &fake)
        .env("T3WF_NO_LAZY", "1")
        .env("T3WF_SYSTEMCTL", "/bin/true")
        .env("T3WF_TZ", "UTC");
    if let Instance::OwnFolder = inst {
        cmd.env("CLAUDE_CONFIG_DIR", &fx.claude);
    }
    let out = cmd.output().expect("python3 runs t3wf.py");
    assert!(out.status.success(), "t3wf.py slots failed: {}", String::from_utf8_lossy(&out.stderr));
    serde_json::from_slice(&out.stdout).unwrap()
}

async fn service(fx: &Fx, inst: Instance, caps: Caps) -> Value {
    let h = H::build(Vec::new(), Config::default());
    h.app.wf.set_slot_folders(Folders::under(&fx.home, None));
    let instance = match inst {
        Instance::Default => "claudeAgent",
        Instance::OwnFolder => "claude-2",
    };
    let mut th = thread(CALLER, "running", common::T0);
    th["providerInstanceId"] = json!(instance);
    th["projectId"] = json!("proj:widgets");
    h.fake.put(th);
    h.fake.set_projects(vec![project("proj:widgets", &fx.repo.to_string_lossy())]);
    h.fake.set_providers(claude_caps(matches!(caps, Caps::NoClaude)));
    h.fake.set_caps_fail(matches!(caps, Caps::Down));
    let own = match inst {
        Instance::Default => String::new(),
        Instance::OwnFolder => fx.claude.to_string_lossy().into_owned(),
    };
    let out = h
        .app
        .execute(ExecuteRequest {
            code: "async () => wf.slots()".into(),
            session: None,
            timeout_ms: None,
            caller: Some(CALLER.into()),
            reported: Reported { own: Some(own), claude: None, codex: None },
        })
        .await;
    assert!(!out.is_error, "{}", out.text);
    let v: Value = serde_json::from_str(&out.text).unwrap();
    v["result"].clone()
}

async fn run_all(inst: Instance) {
    let mut failures = Vec::new();
    for case in cases() {
        let dir = tempfile::tempdir().unwrap();
        let tmp = dir.path().to_path_buf();
        let home = tmp.join("home");
        let claude = match inst {
            Instance::Default => home.join(".claude"),
            Instance::OwnFolder => home.join(".claude-2"),
        };
        let fx = Fx {
            tmp: tmp.clone(),
            config: home.join(".config/t3-workflow/config.json"),
            home,
            claude,
            repo: tmp.join("repo"),
        };
        std::fs::create_dir_all(&fx.repo).unwrap();
        std::fs::create_dir_all(&fx.home).unwrap();
        (case.build)(&fx);
        let config = fx.config.to_string_lossy().into_owned();
        let mut want = python(&fx, inst, case.caps);
        let got = service(&fx, inst, case.caps).await;
        let mut got_same: serde_json::Map<String, Value> = serde_json::Map::new();
        for k in want.as_object().unwrap().keys() {
            got_same.insert(k.clone(), got[k].clone());
        }
        let mut got_same = Value::Object(got_same);
        normalize(&mut want, &config);
        normalize(&mut got_same, &config);
        if want != got_same {
            failures.push(format!(
                "{} ({inst:?}):\n  t3wf.py: {}\n  wf.slots: {}",
                case.name,
                serde_json::to_string(&want).unwrap(),
                serde_json::to_string(&got_same).unwrap()
            ));
        }
        assert!(got["warnings"].as_array().unwrap().is_empty(), "{}: {}", case.name, got["warnings"]);
    }
    assert!(failures.is_empty(), "{} case(s) differ:\n{}", failures.len(), failures.join("\n"));
}

#[tokio::test(flavor = "multi_thread")]
async fn wf_slots_matches_t3wf_for_a_default_instance() {
    run_all(Instance::Default).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn wf_slots_matches_t3wf_for_an_instance_with_its_own_config_folder() {
    run_all(Instance::OwnFolder).await;
}
