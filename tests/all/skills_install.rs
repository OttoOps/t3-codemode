use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::{Value, json};
use t3_codemode::skills::{self, Bundle, FileState, Mode, Options, Paths, SkillState};

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

fn skills_dir(home: &Path) -> PathBuf {
    home.join(".agents").join("skills")
}

fn repo_skills() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("skills")
}

fn status_json(home: &Path) -> (bool, Value) {
    let out = t3cm(home, &["skills", "status", "--json"]);
    (out.status.success(), serde_json::from_slice(&out.stdout).unwrap_or_else(|_| panic!("{}", text(&out))))
}

#[test]
fn install_then_status_reports_every_skill_current_from_an_empty_home() {
    let home = tempfile::tempdir().unwrap();
    let out = t3cm(home.path(), &["skills", "install"]);
    assert!(out.status.success(), "{}", text(&out));
    let release = env!("CARGO_PKG_VERSION");
    let out = t3cm(home.path(), &["skills", "status"]);
    assert!(out.status.success(), "{}", text(&out));
    let shown = text(&out);
    for name in ["setup-t3-workflow", "t3-overseer", "t3-ticket", "t3-worker", "t3-workflow"] {
        let current = format!("current ({release})");
        assert!(shown.lines().any(|l| l.trim_start().starts_with(name) && l.ends_with(&current)), "{shown}");
        let md = std::fs::read_to_string(skills_dir(home.path()).join(name).join("SKILL.md")).unwrap();
        assert!(md.contains(&format!("  t3-codemode-release: \"{release}\"\n")), "{name}: {md}");
    }
    assert_eq!(shown.matches(&format!("current ({release})")).count(), 5, "{shown}");
    let (ok, st) = status_json(home.path());
    assert!(ok);
    assert_eq!(st["current"], true);
    assert_eq!(st["installed"], release);
    let manifest: Value = serde_json::from_str(
        &std::fs::read_to_string(home.path().join(".config").join("t3-workflow").join("installed.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(manifest["release"], release);
    let files = manifest["files"].as_object().unwrap();
    assert!(files.keys().all(|k| !k.ends_with(".py")), "manifest must carry no .py files: {files:?}");
    assert!(files.contains_key("t3-workflow/codemode.md"));
    assert!(files.values().all(|h| h.as_str().is_some_and(|h| h.len() == 64)));
}

#[test]
fn update_keeps_a_locally_edited_file_unless_forced() {
    let home = tempfile::tempdir().unwrap();
    assert!(t3cm(home.path(), &["skills", "install"]).status.success());
    let edited = skills_dir(home.path()).join("t3-worker").join("procedure.md");
    let mut body = std::fs::read_to_string(&edited).unwrap();
    body.push_str("\nA local rule.\n");
    std::fs::write(&edited, &body).unwrap();
    let removed = skills_dir(home.path()).join("t3-ticket").join("SKILL.md");
    std::fs::remove_file(&removed).unwrap();

    let out = t3cm(home.path(), &["skills", "update"]);
    assert!(out.status.success(), "{}", text(&out));
    assert!(text(&out).contains("t3-worker/procedure.md"), "{}", text(&out));
    assert!(text(&out).contains("--force"), "{}", text(&out));
    assert_eq!(std::fs::read_to_string(&edited).unwrap(), body);
    assert!(removed.is_file(), "a missing file is restored");

    let (ok, st) = status_json(home.path());
    assert!(!ok);
    let worker = st["skills"].as_array().unwrap().iter().find(|s| s["name"] == "t3-worker").unwrap();
    assert_eq!(worker["state"], "edited");
    assert_eq!(worker["files"], json!([{"path": "t3-worker/procedure.md", "state": "edited"}]));
    assert_eq!(st["fix"], "t3cm skills update --force");

    let out = t3cm(home.path(), &["skills", "update", "--force"]);
    assert!(out.status.success(), "{}", text(&out));
    assert_eq!(
        std::fs::read(&edited).unwrap(),
        std::fs::read(repo_skills().join("t3-worker").join("procedure.md")).unwrap()
    );
    assert!(status_json(home.path()).0);
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap().flatten() {
        let name = e.file_name();
        if name.to_string_lossy().starts_with('.') || name == "__pycache__" {
            continue;
        }
        if e.path().is_dir() {
            copy_tree(&e.path(), &to.join(&name));
        } else {
            std::fs::copy(e.path(), to.join(&name)).unwrap();
        }
    }
}

fn npx_install(home: &Path, edit: bool) -> PathBuf {
    copy_tree(&repo_skills(), &skills_dir(home));
    if edit {
        let p = skills_dir(home).join("t3-ticket").join("SKILL.md");
        let mut s = std::fs::read_to_string(&p).unwrap();
        s.push_str("\nAn older line.\n");
        std::fs::write(p, s).unwrap();
    }
    let mut entries = serde_json::Map::new();
    entries.insert(
        "other-skill".into(),
        json!({"source": "acme/widgets", "sourceType": "github", "commit": "1111111111111111111111111111111111111111"}),
    );
    for name in ["setup-t3-workflow", "t3-overseer", "t3-ticket", "t3-worker", "t3-workflow"] {
        entries.insert(
            name.into(),
            json!({"source": "OttoOps/t3-codemode", "sourceType": "github", "commit": "8e179c3aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "skillPath": format!("skills/{name}/SKILL.md")}),
        );
    }
    let lock = home.join(".agents").join(".skill-lock.json");
    std::fs::write(&lock, serde_json::to_string_pretty(&json!({"version": 3, "skills": entries})).unwrap()).unwrap();
    lock
}

#[test]
fn status_reports_an_npx_skills_install_and_its_skew() {
    let home = tempfile::tempdir().unwrap();
    npx_install(home.path(), true);
    let out = t3cm(home.path(), &["skills", "status"]);
    assert!(!out.status.success());
    let shown = text(&out);
    assert!(shown.contains("npx skills (commit 8e179c3)"), "{shown}");
    assert!(shown.contains("1 of its files differ from this release"), "{shown}");
    assert!(shown.contains("fix: t3cm skills install --take-over"), "{shown}");
    let (_, st) = status_json(home.path());
    assert_eq!(st["foreign"]["tool"], "npx skills");
    assert_eq!(st["foreign"]["commit"], "8e179c3aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    assert_eq!(st["installed"], Value::Null);
}

#[test]
fn install_takes_over_an_npx_skills_install_only_when_asked() {
    let home = tempfile::tempdir().unwrap();
    let lock = npx_install(home.path(), true);
    let before = std::fs::read(&lock).unwrap();

    for args in [&["skills", "install"][..], &["skills", "update"][..]] {
        let out = t3cm(home.path(), args);
        assert!(!out.status.success(), "{args:?}");
        assert!(text(&out).contains("t3cm skills install --take-over"), "{}", text(&out));
    }
    assert_eq!(std::fs::read(&lock).unwrap(), before, "nothing changes without --take-over");
    assert!(!home.path().join(".config").join("t3-workflow").join("installed.json").exists());

    let out = t3cm(home.path(), &["skills", "install", "--take-over"]);
    assert!(out.status.success(), "{}", text(&out));
    assert!(text(&out).contains("npx skills no longer manages"), "{}", text(&out));
    let left: Value = serde_json::from_slice(&std::fs::read(&lock).unwrap()).unwrap();
    assert_eq!(left["version"], 3);
    assert_eq!(left["skills"].as_object().unwrap().keys().collect::<Vec<_>>(), ["other-skill"]);
    let (ok, st) = status_json(home.path());
    assert!(ok, "{st}");
    assert_eq!(st["foreign"], Value::Null);
}

fn bundle(release: &str, files: &[(&str, &str)]) -> Bundle {
    Bundle::new(release, files.iter().map(|(p, b)| (p.to_string(), b.as_bytes().to_vec())))
}

const MD: &str = "---\nname: a\ndescription: d\n---\nbody\n";

#[test]
fn an_update_to_a_new_release_writes_changed_files_and_drops_removed_ones() {
    let home = tempfile::tempdir().unwrap();
    let paths = Paths::under(home.path(), None);
    let old =
        bundle("0.1.0", &[("a/SKILL.md", MD), ("a/one.md", "one\n"), ("a/gone.md", "gone\n"), ("a/kept.md", "kept\n")]);
    let opts = |mode| Options { mode, force: false, take_over: false };
    skills::sync(&paths, &old, opts(Mode::Install)).unwrap();
    std::fs::write(paths.dir.join("a").join("kept.md"), "mine\n").unwrap();

    let new = bundle("0.2.0", &[("a/SKILL.md", MD), ("a/one.md", "one, revised\n"), ("a/two.md", "two\n")]);
    let st = skills::status(&paths, &new).unwrap();
    assert!(!st.current);
    assert_eq!(st.installed.as_deref(), Some("0.1.0"));
    let a = &st.skills[0];
    assert_eq!(a.state, SkillState::Edited);
    let states: Vec<_> = a.files.iter().map(|f| (f.path.as_str(), f.state)).collect();
    assert_eq!(
        states,
        [
            ("a/SKILL.md", FileState::Stale),
            ("a/one.md", FileState::Stale),
            ("a/two.md", FileState::Missing),
            ("a/gone.md", FileState::Obsolete),
            ("a/kept.md", FileState::ObsoleteEdited),
        ]
    );

    let r = skills::sync(&paths, &new, opts(Mode::Update)).unwrap();
    assert_eq!(r.previous.as_deref(), Some("0.1.0"));
    assert_eq!(r.written, ["a/SKILL.md", "a/one.md", "a/two.md"]);
    assert_eq!(r.removed, ["a/gone.md"]);
    assert_eq!(r.kept, ["a/kept.md"]);
    assert!(!paths.dir.join("a").join("gone.md").exists());
    assert_eq!(std::fs::read_to_string(paths.dir.join("a").join("kept.md")).unwrap(), "mine\n");
    let md = std::fs::read_to_string(paths.dir.join("a").join("SKILL.md")).unwrap();
    assert!(md.contains("t3-codemode-release: \"0.2.0\""), "{md}");

    let st = skills::status(&paths, &new).unwrap();
    assert_eq!(st.skills[0].files.iter().map(|f| f.state).collect::<Vec<_>>(), [FileState::ObsoleteEdited]);
    skills::sync(&paths, &new, Options { mode: Mode::Update, force: true, take_over: false }).unwrap();
    assert!(!paths.dir.join("a").join("kept.md").exists());
    assert!(skills::status(&paths, &new).unwrap().current);
}

#[test]
fn a_skill_dropped_from_the_release_is_removed_on_update() {
    let home = tempfile::tempdir().unwrap();
    let paths = Paths::under(home.path(), None);
    let opts = Options { mode: Mode::Update, force: false, take_over: false };
    skills::sync(
        &paths,
        &bundle("0.1.0", &[("a/SKILL.md", MD), ("b/SKILL.md", MD), ("b/x/y.md", "y\n")]),
        Options { mode: Mode::Install, ..opts },
    )
    .unwrap();
    let new = bundle("0.2.0", &[("a/SKILL.md", MD)]);
    let st = skills::status(&paths, &new).unwrap();
    assert_eq!(
        st.skills.iter().map(|s| (s.name.as_str(), s.state)).collect::<Vec<_>>(),
        [("a", SkillState::Stale), ("b", SkillState::Stale)]
    );
    skills::sync(&paths, &new, opts).unwrap();
    assert!(!paths.dir.join("b").exists());
    let st = skills::status(&paths, &new).unwrap();
    assert!(st.current, "{st:?}");
    assert_eq!(st.skills.len(), 1);
}

#[test]
fn install_honours_xdg_config_home_for_the_manifest() {
    let home = tempfile::tempdir().unwrap();
    let config = home.path().join("cfg");
    let paths = Paths::under(home.path(), Some(&config));
    assert_eq!(paths.manifest, config.join("t3-workflow").join("installed.json"));
    skills::sync(
        &paths,
        &bundle("0.1.0", &[("a/SKILL.md", MD)]),
        Options { mode: Mode::Install, force: false, take_over: false },
    )
    .unwrap();
    assert!(paths.manifest.is_file());
}

#[cfg(unix)]
#[test]
fn a_linked_skill_folder_is_never_written_through() {
    let home = tempfile::tempdir().unwrap();
    let paths = Paths::under(home.path(), None);
    let target = home.path().join("clone");
    std::fs::create_dir_all(&target).unwrap();
    std::fs::create_dir_all(&paths.dir).unwrap();
    std::os::unix::fs::symlink(&target, paths.dir.join("a")).unwrap();
    let err = skills::sync(
        &paths,
        &bundle("0.1.0", &[("a/SKILL.md", MD)]),
        Options { mode: Mode::Install, force: true, take_over: true },
    )
    .unwrap_err();
    assert!(format!("{err:#}").contains("is a link to"), "{err:#}");
    assert_eq!(std::fs::read_dir(&target).unwrap().count(), 0);
}

#[test]
fn a_kept_file_restored_to_its_installed_bytes_counts_as_stale_again() {
    let home = tempfile::tempdir().unwrap();
    let paths = Paths::under(home.path(), None);
    let opts = |mode| Options { mode, force: false, take_over: false };
    skills::sync(&paths, &bundle("0.1.0", &[("a/SKILL.md", MD), ("a/one.md", "one\n")]), opts(Mode::Install)).unwrap();
    let one = paths.dir.join("a").join("one.md");
    std::fs::write(&one, "mine\n").unwrap();
    let new = bundle("0.2.0", &[("a/SKILL.md", MD), ("a/one.md", "one, revised\n")]);
    assert_eq!(skills::sync(&paths, &new, opts(Mode::Update)).unwrap().kept, ["a/one.md"]);
    std::fs::write(&one, "one\n").unwrap();
    let st = skills::status(&paths, &new).unwrap();
    assert_eq!(st.skills[0].files.iter().map(|f| f.state).collect::<Vec<_>>(), [FileState::Stale]);
    assert_eq!(skills::sync(&paths, &new, opts(Mode::Update)).unwrap().written, ["a/one.md"]);
}

#[test]
fn a_fresh_install_that_lost_its_manifest_resumes_without_a_take_over() {
    let home = tempfile::tempdir().unwrap();
    let paths = Paths::under(home.path(), None);
    let b = bundle("0.1.0", &[("a/SKILL.md", MD), ("b/SKILL.md", MD)]);
    let opts = Options { mode: Mode::Install, force: false, take_over: false };
    skills::sync(&paths, &b, opts).unwrap();
    std::fs::remove_dir_all(paths.dir.join("b")).unwrap();
    std::fs::remove_file(&paths.manifest).unwrap();
    let st = skills::status(&paths, &b).unwrap();
    assert!(st.foreign.is_none(), "{st:?}");
    assert_eq!(st.fix.as_deref(), Some("t3cm skills install"));
    assert_eq!(skills::sync(&paths, &b, opts).unwrap().written, ["b/SKILL.md"]);
    assert!(skills::status(&paths, &b).unwrap().current);
}

#[test]
fn npx_skew_counts_only_the_skills_it_installed() {
    let home = tempfile::tempdir().unwrap();
    let paths = Paths::under(home.path(), None);
    let b = bundle("0.1.0", &[("a/SKILL.md", MD), ("b/SKILL.md", MD)]);
    std::fs::create_dir_all(paths.dir.join("a")).unwrap();
    std::fs::write(paths.dir.join("a").join("SKILL.md"), MD).unwrap();
    std::fs::create_dir_all(paths.dir.join("b")).unwrap();
    std::fs::write(paths.dir.join("b").join("SKILL.md"), "a copy by hand\n").unwrap();
    std::fs::write(&paths.lock, r#"{"version":3,"skills":{"a":{"source":"acme/widgets","commit":"abc"}}}"#).unwrap();
    let f = skills::status(&paths, &b).unwrap().foreign.unwrap();
    assert_eq!((f.skills.as_slice(), f.differing_files, f.matches_release), (&["a".to_string()][..], 0, true));
}

#[cfg(unix)]
const SKILLS: [&str; 5] = ["setup-t3-workflow", "t3-overseer", "t3-ticket", "t3-worker", "t3-workflow"];

#[cfg(unix)]
fn claude_skill(home: &Path, name: &str) -> PathBuf {
    home.join(".claude").join("skills").join(name)
}

#[cfg(unix)]
#[test]
fn install_links_every_skill_into_claude_codes_folder() {
    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir(home.path().join(".claude")).unwrap();
    let out = t3cm(home.path(), &["skills", "install"]);
    assert!(out.status.success(), "{}", text(&out));
    assert!(text(&out).contains("for Claude Code"), "{}", text(&out));
    for name in SKILLS {
        let link = claude_skill(home.path(), name);
        assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink(), "{name}");
        assert_eq!(
            std::fs::canonicalize(&link).unwrap(),
            std::fs::canonicalize(skills_dir(home.path()).join(name)).unwrap()
        );
    }
    let (ok, st) = status_json(home.path());
    assert!(ok);
    assert!(st["claudeLinks"].as_array().unwrap().iter().all(|l| l["state"] == "linked"), "{st}");
    let again = t3cm(home.path(), &["skills", "update", "--json"]);
    let v: Value = serde_json::from_slice(&again.stdout).unwrap();
    assert_eq!(v["claudeLinks"]["made"], json!([]));
}

#[test]
fn no_claude_folder_means_no_links_and_no_complaint() {
    let home = tempfile::tempdir().unwrap();
    let out = t3cm(home.path(), &["skills", "install"]);
    assert!(out.status.success(), "{}", text(&out));
    assert!(!home.path().join(".claude").exists());
    let out = t3cm(home.path(), &["skills", "status"]);
    assert!(out.status.success());
    assert!(!text(&out).contains("Claude Code"), "{}", text(&out));
}

#[cfg(unix)]
#[test]
fn a_real_folder_in_claudes_place_is_kept_and_only_reported() {
    let home = tempfile::tempdir().unwrap();
    let mine = claude_skill(home.path(), "t3-ticket");
    std::fs::create_dir_all(&mine).unwrap();
    std::fs::write(mine.join("SKILL.md"), "mine\n").unwrap();
    let out = t3cm(home.path(), &["skills", "install"]);
    assert!(out.status.success(), "{}", text(&out));
    assert_eq!(std::fs::read_to_string(mine.join("SKILL.md")).unwrap(), "mine\n");
    assert!(!std::fs::symlink_metadata(&mine).unwrap().file_type().is_symlink());
    assert!(std::fs::symlink_metadata(claude_skill(home.path(), "t3-worker")).unwrap().file_type().is_symlink());
    let out = t3cm(home.path(), &["skills", "status"]);
    assert!(out.status.success(), "a Claude Code copy never fails the status: {}", text(&out));
    assert!(text(&out).contains("is not a link to"), "{}", text(&out));
}

#[cfg(unix)]
#[test]
fn a_link_elsewhere_is_left_alone_and_a_removed_link_is_reported() {
    let home = tempfile::tempdir().unwrap();
    let other = home.path().join("elsewhere");
    std::fs::create_dir_all(&other).unwrap();
    std::fs::create_dir_all(home.path().join(".claude").join("skills")).unwrap();
    std::os::unix::fs::symlink(&other, claude_skill(home.path(), "t3-overseer")).unwrap();
    assert!(t3cm(home.path(), &["skills", "install"]).status.success());
    assert_eq!(std::fs::read_link(claude_skill(home.path(), "t3-overseer")).unwrap(), other);
    std::fs::remove_file(claude_skill(home.path(), "t3-worker")).unwrap();
    let out = t3cm(home.path(), &["skills", "status"]);
    assert!(out.status.success(), "{}", text(&out));
    assert!(text(&out).contains("Claude Code does not see t3-worker"), "{}", text(&out));
    let (_, st) = status_json(home.path());
    assert_eq!(st["current"], true);
}

#[cfg(unix)]
#[test]
fn claude_config_dir_picks_the_folder_to_link_into() {
    let home = tempfile::tempdir().unwrap();
    let cfg = home.path().join("claude-2");
    std::fs::create_dir(&cfg).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_t3cm"))
        .args(["skills", "install"])
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .env("CLAUDE_CONFIG_DIR", &cfg)
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("CODEX_HOME")
        .env_remove("T3WF_CONFIG")
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", text(&out));
    assert!(std::fs::symlink_metadata(cfg.join("skills").join("t3-workflow")).is_ok());
    assert!(!home.path().join(".claude").exists());
}

#[cfg(unix)]
#[test]
fn npx_relative_links_count_as_linked_after_a_take_over() {
    let home = tempfile::tempdir().unwrap();
    npx_install(home.path(), false);
    std::fs::create_dir_all(home.path().join(".claude").join("skills")).unwrap();
    for name in SKILLS {
        std::os::unix::fs::symlink(format!("../../.agents/skills/{name}"), claude_skill(home.path(), name)).unwrap();
    }
    let out = t3cm(home.path(), &["skills", "install", "--take-over", "--json"]);
    assert!(out.status.success(), "{}", text(&out));
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["claudeLinks"]["made"], json!([]));
    assert!(v["claudeLinks"]["links"].as_array().unwrap().iter().all(|l| l["state"] == "linked"), "{v}");
}
