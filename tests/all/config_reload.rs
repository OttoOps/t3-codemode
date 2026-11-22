use std::path::PathBuf;
use std::time::Duration;

use serde_json::{Value, json};
use t3_codemode::config::Config;
use t3_codemode::config_edit::{Document, effective, lookup};
use t3_codemode::fake::provider;
use t3_codemode::release::Version;
use t3_codemode::{field, server, watcher};

use crate::common::*;
use crate::config_edit::{Cfg, apply, every_kind_of_key, set};

fn file(h: &H) -> PathBuf {
    h.dir.path().join("t3cm-config.json")
}

fn write(h: &H, config: Value) {
    std::fs::write(file(h), config.to_string()).unwrap();
}

async fn reload(h: &H) -> Vec<&'static str> {
    h.app.wf.reload_config().await.unwrap().restart
}

async fn load(h: &H, config: Value) -> Vec<&'static str> {
    write(h, config);
    reload(h).await
}

#[cfg(unix)]
fn edit_to(c: &Cfg, text: &str) -> std::process::Output {
    let next = c.dir.path().join("next.json");
    std::fs::write(&next, text).unwrap();
    c.command(&["edit", "--no-t3"]).env("EDITOR", format!("cp '{}'", next.display())).output().unwrap()
}

async fn serve(h: &H) -> u16 {
    let (listener, addr) = server::bind(0).await.unwrap();
    let app = h.app.clone();
    tokio::spawn(async move {
        let _ = server::serve(app, listener, std::future::pending()).await;
    });
    addr.port()
}

#[tokio::test]
async fn every_key_either_applies_or_reports_a_restart() {
    let h = H::new();
    let started = effective(&h.app.config());
    for (path, value) in every_kind_of_key() {
        apply(&file(&h), set(path, value));
        let restart = reload(&h).await;
        let parsed = field::parse(path).unwrap();
        let running = effective(&h.app.config());
        let saved = effective(&Config::load(&file(&h)).unwrap());
        let held = ["maxConcurrentRuns", "upstreams", "board.remote"]
            .into_iter()
            .find(|key| path == *key || path.starts_with(&format!("{key}.")));
        match held {
            Some(key) => {
                assert!(restart.contains(&key), "{path}: {restart:?}");
                let key = field::parse(key).unwrap();
                assert_eq!(lookup(&running, &key), lookup(&started, &key), "{path}");
            }
            None => assert_eq!(lookup(&running, &parsed), lookup(&saved, &parsed), "{path}"),
        }
    }

    let mut expected = effective(&Config::load(&file(&h)).unwrap());
    expected["maxConcurrentRuns"] = started["maxConcurrentRuns"].clone();
    expected["upstreams"] = started["upstreams"].clone();
    expected["board"]["remote"] = started["board"]["remote"].clone();
    assert_eq!(effective(&h.app.config()), expected);
    assert_eq!(reload(&h).await, ["maxConcurrentRuns", "upstreams", "board.remote"]);
}

#[tokio::test]
async fn board_feed_changes_without_a_restart() {
    let h = H::new();
    h.wf(TICKET, "init", json!({"role": "ticket"})).await;
    for body in ["one", "two", "three"] {
        h.wf(TICKET, "send", json!({"kind": "note", "body": body})).await;
    }
    let shown = async || h.wf(BOSS, "feed", json!({})).await.as_array().unwrap().len();
    assert_eq!(shown().await, 3);
    assert!(load(&h, json!({"board": {"feed": 1}})).await.is_empty());
    assert_eq!(shown().await, 1);
    assert!(load(&h, json!({})).await.is_empty());
    assert_eq!(shown().await, 3);
}

#[tokio::test]
async fn the_deny_list_changes_without_a_restart() {
    let h = H::with_tools();
    h.load_catalog().await;
    let read = "async () => t3.thread_read({ threadId: 'mcp:boss' })";
    ok_json(&h.exec(read).await);
    load(&h, json!({"deny": ["t3_thread_read"]})).await;
    let out = h.exec(read).await;
    assert!(err_text(&out).contains("denied"), "{}", out.text);
    load(&h, json!({})).await;
    ok_json(&h.exec(read).await);
}

#[tokio::test]
async fn pools_and_the_ask_model_change_without_a_restart() {
    let h = H::new();
    h.fake.set_providers(vec![provider("claude-a", "claudeAgent", &["claude-opus-5-5", "claude-haiku-5-5"])]);
    assert!(h.wf_err(BOSS, "pick", json!({"pool": "opus"})).await.contains("no pool"));
    let before = h.app.wf.models_view().await.unwrap();
    let pool = json!({"driver": "claudeAgent", "model": "claude-opus-5-5", "members": [{"instanceId": "claude-a"}]});
    load(&h, json!({"pools": {"opus": pool}, "board": {"askPool": "opus"}})).await;
    assert_eq!(h.wf(BOSS, "pick", json!({"pool": "opus"})).await["instanceId"], "claude-a");
    let after = h.app.wf.models_view().await.unwrap();
    assert_eq!(after["pools"][0]["name"], "opus");
    assert_eq!((&after["ask"]["mode"], &after["ask"]["pool"]), (&json!("pool"), &json!("opus")));
    assert!(after["modelsRev"].as_u64() > before["modelsRev"].as_u64(), "open boards refetch: {after}");

    load(&h, json!({})).await;
    let cleared = h.app.wf.models_view().await.unwrap();
    assert_eq!((&cleared["ask"]["source"], &cleared["pools"]), (&json!("default"), &json!([])));
}

#[tokio::test]
async fn a_board_ask_save_applies_only_the_ask_keys() {
    let h = H::new();
    write(&h, json!({"maxConcurrentRuns": 4, "board": {"feed": 9}}));
    let body = json!({"mode": "model", "provider": "claudeAgent", "model": "claude-opus-5-5"});
    h.app.wf.set_ask_model(serde_json::from_value(body).unwrap()).await.unwrap();
    let running = h.app.config();
    assert_eq!(running.board.ask_model, "claude-opus-5-5");
    assert_eq!((running.max_concurrent_runs, running.board.feed), (16, 50));
    assert_eq!(reload(&h).await, ["maxConcurrentRuns"]);
    let running = h.app.config();
    assert_eq!((running.board.ask_model.as_str(), running.board.feed), ("claude-opus-5-5", 9));
}

#[tokio::test]
async fn restart_only_keys_keep_their_values_until_a_restart() {
    let h = H::new();
    let remote = json!({"mode": "pairing", "origin": "https://board.example.com"});
    let restart = load(&h, json!({"maxConcurrentRuns": 2, "board": {"remote": remote, "feed": 7}})).await;
    assert_eq!(restart, ["maxConcurrentRuns", "board.remote"]);
    let health = h.app.health();
    assert_eq!(health["runs"]["max"], 16);
    assert_eq!(health["runs"]["running"], 0);
    assert_eq!(health["remote"]["mode"], "off");
    assert_eq!(h.app.config().board.feed, 7);
    assert!(load(&h, json!({"board": {"remote": {"mode": "off"}}})).await.is_empty());
}

#[tokio::test]
async fn an_invalid_file_never_replaces_the_running_config() {
    let h = H::new();
    load(&h, json!({"deny": ["t3_thread_interrupt"], "board": {"feed": 7}})).await;
    let running = effective(&h.app.config());
    let cases = [
        ("{\"board\": ", "not valid JSON"),
        ("", "not valid JSON"),
        (r#"{"board": {"feed": "lots"}}"#, "board.feed"),
        (r#"{"board": {"feed": 9, "askPool": "gone"}}"#, "board.askPool"),
        (r#"{"board": {"feed": 9}, "maxConcurrentRuns": 0}"#, "maxConcurrentRuns"),
    ];
    for (text, why) in cases {
        std::fs::write(file(&h), text).unwrap();
        let err = format!("{:#}", h.app.wf.reload_config().await.unwrap_err());
        assert!(err.contains(why), "{err}");
        assert_eq!(effective(&h.app.config()), running, "{text}");
    }
}

#[tokio::test(start_paused = true)]
async fn a_tightened_runtime_mode_reaches_an_ask_still_resolving_its_model() {
    let h = H::new();
    h.fake.set_delay(Duration::from_secs(1));
    let app = h.app.clone();
    let asking = tokio::spawn(async move { app.wf.ask("how is it going?").await.map(|_| ()) });
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(h.fake.launched().is_empty());
    load(&h, json!({"board": {"runtimeMode": "approval-required"}})).await;
    asking.await.unwrap().unwrap();
    assert_eq!(h.fake.launched()[0]["runtimeMode"], "approval-required");
}

#[tokio::test(start_paused = true)]
async fn an_open_ask_follows_a_longer_timeout_without_the_watcher() {
    let h = H::build(Vec::new(), Config::from_value(json!({"board": {"askTimeoutMinutes": 1}})).unwrap());
    h.app.ask("how is it going?").await.unwrap();
    load(&h, json!({"board": {"askTimeoutMinutes": 10}})).await;
    tokio::time::sleep(Duration::from_secs(5 * 60)).await;
    assert_eq!(h.app.wf.asks().unwrap()[0]["status"], "running");
    h.clock.set(&at(11));
    tokio::time::sleep(Duration::from_secs(10)).await;
    assert_eq!(h.app.wf.asks().unwrap()[0]["status"], "timed out");
}

#[cfg(unix)]
#[tokio::test]
async fn a_dangling_symlink_does_not_reset_the_config_to_defaults() {
    let h = H::new();
    load(&h, json!({"deny": ["t3_thread_interrupt"]})).await;
    std::fs::remove_file(file(&h)).unwrap();
    std::os::unix::fs::symlink(h.dir.path().join("gone.json"), file(&h)).unwrap();
    assert!(format!("{:#}", h.app.wf.reload_config().await.unwrap_err()).contains("symlink"));
    assert_eq!(h.app.config().deny, ["t3_thread_interrupt"]);
}

#[tokio::test]
async fn turning_update_checks_off_hides_a_known_release() {
    let h = H::new();
    h.app.wf.set_release(Version::parse("v99.0.0").unwrap());
    assert_eq!(h.app.wf.update_view()["available"], true);
    load(&h, json!({"updateCheck": false})).await;
    assert_eq!(h.app.wf.update_view(), Value::Null);
    load(&h, json!({})).await;
    assert_eq!(h.app.wf.update_view()["available"], true);
}

#[tokio::test]
async fn the_watcher_takes_a_new_interval_without_a_restart() {
    let h = H::build(Vec::new(), Config::from_value(json!({"watcherIntervalSecs": 3600})).unwrap());
    let watching = watcher::spawn(h.app.clone());
    let wait_for_pass = async |at: String| {
        tokio::time::timeout(Duration::from_secs(10), async {
            while h.app.health()["watcher"]["lastPass"] != json!(at) {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("no pass at {at}: {}", h.app.health()["watcher"]));
    };
    wait_for_pass(T0.to_string()).await;
    h.clock.set(&at(5));
    load(&h, json!({"watcherIntervalSecs": 5})).await;
    wait_for_pass(at(5)).await;
    assert_eq!(h.app.health()["watcher"]["intervalSecs"], 5);
    watching.abort();
}

#[tokio::test]
async fn the_reload_route_is_loopback_only_and_checks_the_path() {
    let h = H::new();
    let url = format!("http://127.0.0.1:{}/reload", serve(&h).await);
    write(&h, json!({"board": {"feed": 7}}));
    let http = reqwest::Client::builder().no_proxy().build().unwrap();
    let body = json!({"path": Document::location(&file(&h)).to_string_lossy()});
    let refused = [
        ("host", "board.example.com"),
        ("origin", "https://board.example.com"),
        ("x-forwarded-for", "100.64.0.9"),
        ("tailscale-user-login", "alice@example.com"),
    ];
    for (name, value) in refused {
        assert_eq!(http.post(&url).header(name, value).json(&body).send().await.unwrap().status(), 403, "{name}");
    }
    assert_eq!(http.get(&url).send().await.unwrap().status(), 405);
    assert_eq!(http.post(&url).body("{}").send().await.unwrap().status(), 400);
    let other = http.post(&url).json(&json!({"path": "/elsewhere/config.json"})).send().await.unwrap();
    assert_eq!(other.status(), 409);
    assert_eq!(h.app.config().board.feed, 50, "a refused request reloads nothing");

    let done = http.post(&url).json(&body).send().await.unwrap();
    assert_eq!(done.status(), 200);
    assert_eq!(done.json::<Value>().await.unwrap(), json!({"restart": []}));
    assert_eq!(h.app.config().board.feed, 7);

    std::fs::write(file(&h), "{").unwrap();
    let bad = http.post(&url).json(&body).send().await.unwrap();
    assert_eq!(bad.status(), 422);
    assert!(bad.json::<Value>().await.unwrap()["error"].as_str().unwrap().contains("not valid JSON"));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_cli_reloads_the_running_service() {
    let h = H::new();
    let port = serve(&h).await;
    let app = h.app.clone();
    let path = file(&h);
    tokio::task::spawn_blocking(move || {
        let c = Cfg::at(path).on_port(port);
        let out = c.ok(&["set", "board.feed", "80"]);
        assert!(out.contains("Reloaded") && !out.contains("Restart"), "{out}");
        assert_eq!(app.config().board.feed, 80);

        let out = c.ok(&["set", "maxConcurrentRuns", "4"]);
        assert!(out.contains("need a restart: maxConcurrentRuns") && out.contains("Restart the service"), "{out}");
        assert_eq!(app.config().max_concurrent_runs, 16);

        let out = c.ok(&["set", "board.feed", "90"]);
        assert!(out.contains("need a restart: maxConcurrentRuns"), "an earlier change is still waiting: {out}");
        assert_eq!(app.config().board.feed, 90);

        let out = c.ok(&["unset", "maxConcurrentRuns"]);
        assert!(out.contains("Reloaded") && !out.contains("Restart"), "{out}");
        c.ok(&["unset", "board.feed"]);
        assert_eq!(app.config().board.feed, 50);

        std::fs::write(&c.file, r#"{"board": {"feed": 33}}"#).unwrap();
        assert!(c.ok(&["set", "board.feed", "33"]).contains("No change"));
        assert_eq!(app.config().board.feed, 33, "an unchanged set still reloads");

        #[cfg(unix)]
        {
            let out = edit_to(&c, r#"{"board": {"feed": 12}}"#);
            assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
            assert!(String::from_utf8_lossy(&out.stdout).contains("Reloaded"));
            assert_eq!(app.config().board.feed, 12);
        }

        let other = Cfg::absent().on_port(port);
        let out = other.run(&["set", "board.feed", "5"]);
        let (said, warned) = (String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        assert!(out.status.success() && warned.contains("reads") && !said.contains("Restart"), "{said}{warned}");
        assert_ne!(app.config().board.feed, 5);
    })
    .await
    .unwrap();
}

async fn stub(status: u16, body: &'static str) -> u16 {
    let status = axum::http::StatusCode::from_u16(status).unwrap();
    let router = axum::Router::new().fallback(move || async move { (status, body) });
    let (listener, addr) = server::bind(0).await.unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await });
    addr.port()
}

#[tokio::test(flavor = "multi_thread")]
async fn the_cli_says_when_the_service_did_not_reload() {
    let old = stub(404, "").await;
    let strange = stub(200, "hello").await;
    let refusing = stub(422, r#"{"error": "board.feed: too strange"}"#).await;
    tokio::task::spawn_blocking(move || {
        for (port, why) in [(old, "older"), (strange, "unexpected")] {
            let out = Cfg::absent().on_port(port).run(&["set", "board.feed", "80"]);
            let (said, warned) = (String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
            assert!(out.status.success() && warned.contains(why) && said.contains("Restart"), "{said}{warned}");
            assert!(!said.contains("Reloaded"), "{said}");
        }
        let c = Cfg::absent().on_port(refusing);
        let out = c.run(&["set", "board.feed", "9"]);
        let warned = String::from_utf8_lossy(&out.stderr);
        assert!(!out.status.success() && warned.contains("too strange"), "{warned}");
        assert_eq!(Config::load(&c.file).unwrap().board.feed, 9);

        #[cfg(unix)]
        {
            let out = edit_to(&c, r#"{"board": {"feed": 12}}"#);
            let warned = String::from_utf8_lossy(&out.stderr);
            assert!(!out.status.success() && warned.contains("too strange"), "{warned}");
            assert!(!warned.contains("was not written"), "the edit was saved: {warned}");
            assert_eq!(Config::load(&c.file).unwrap().board.feed, 12);
        }
    })
    .await
    .unwrap();
}
