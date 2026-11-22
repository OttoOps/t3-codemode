use std::path::PathBuf;
use std::time::Duration;

use reqwest::Method;
use serde_json::{Value, json};
use t3_codemode::clock::Clock;
use t3_codemode::config::{Config, RemoteOrigin};
use t3_codemode::config_edit::{Document, Stale, effective};
use t3_codemode::fake::provider;
use t3_codemode::store::Limit;
use t3_codemode::{pairing, server};

use crate::common::*;
use crate::config_edit::{apply, every_kind_of_key, set};

const ROUTES: &[(&str, &str)] = &[
    ("GET", "/api/settings"),
    ("HEAD", "/api/settings"),
    ("PUT", "/api/settings"),
    ("GET", "/api/settings/pools"),
    ("HEAD", "/api/settings/pools"),
];

struct Board {
    h: H,
    base: String,
    http: reqwest::Client,
}

impl Board {
    async fn start(h: H) -> Board {
        let (listener, addr) = server::bind(0).await.unwrap();
        let app = h.app.clone();
        tokio::spawn(async move {
            let _ = server::serve(app, listener, std::future::pending()).await;
        });
        Board { h, base: format!("http://{addr}"), http: reqwest::Client::builder().no_proxy().build().unwrap() }
    }

    fn file(&self) -> PathBuf {
        self.h.dir.path().join("t3cm-config.json")
    }

    fn write(&self, text: &str) {
        std::fs::write(self.file(), text).unwrap();
    }

    fn text(&self) -> Option<String> {
        std::fs::read_to_string(self.file()).ok()
    }

    fn request(&self, method: &str, path: &str) -> reqwest::RequestBuilder {
        self.http.request(Method::from_bytes(method.as_bytes()).unwrap(), format!("{}{path}", self.base))
    }

    async fn get(&self, path: &str) -> Value {
        let r = self.request("GET", path).send().await.unwrap();
        assert_eq!(r.status(), 200, "{path}");
        r.json().await.unwrap()
    }

    async fn settings(&self) -> Value {
        self.get("/api/settings").await
    }

    async fn put(&self, body: &Value) -> (u16, Value) {
        let req = self.request("PUT", "/api/settings").header("X-T3CM-CSRF", &self.h.app.csrf).json(body);
        let r = req.send().await.unwrap();
        (r.status().as_u16(), r.json().await.unwrap())
    }

    async fn save(&self, file: Value) -> (u16, Value) {
        let revision = self.settings().await["revision"].clone();
        self.put(&json!({"revision": revision, "file": file})).await
    }

    async fn saved(&self, file: Value) -> Value {
        let (status, body) = self.save(file).await;
        assert_eq!(status, 200, "{body}");
        body
    }
}

fn remote_config(remote: Value) -> Config {
    Config::from_value(json!({"board": {"remote": remote}})).unwrap()
}

async fn refuses_signed_in_remote_requests(remote: Value, sign_in: &[&str]) {
    let host = "board.acme.example:47732";
    let origin = format!("https://{host}");
    let h = H::build(Vec::new(), remote_config(remote.clone()));
    h.tailscale.set_host_login("alice@acme.example");
    let mut signed_in: Vec<(&str, String)> = Vec::new();
    if sign_in.contains(&"tailscale") {
        signed_in.push(("tailscale-user-login", "alice@acme.example".into()));
    }
    if sign_in.contains(&"pairing") {
        let (store, now) = (&h.app.store, h.clock.now());
        let minted = pairing::mint_pairing_token(store, &RemoteOrigin::parse(&origin).unwrap(), None, now).unwrap();
        let (session, _) = pairing::redeem_pairing_token(store, &minted.token, None, now).unwrap().unwrap();
        signed_in.push(("cookie", format!("{}={session}", pairing::SESSION_COOKIE)));
    }
    let b = Board::start(h).await;
    let revision = b.settings().await["revision"].clone();

    let by_host = vec![("host", host.to_string()), ("x-forwarded-proto", "https".to_string())];
    let by_proxy = vec![("x-forwarded-host", host.to_string()), ("x-forwarded-proto", "https".to_string())];
    for route in [by_host, by_proxy] {
        let send = |method: &str, path: &str| {
            let mut req = b.request(method, path).header("origin", &origin).header("X-T3CM-CSRF", &b.h.app.csrf);
            for (name, value) in route.iter().chain(&signed_in) {
                req = req.header(*name, value);
            }
            req
        };
        let allowed = send("GET", "/api/models").send().await.unwrap();
        assert_eq!(allowed.status(), 200, "the request must be authenticated: {remote}");
        for (method, path) in ROUTES {
            let body = json!({"revision": revision, "file": {"deny": []}});
            let r = send(method, path).json(&body).send().await.unwrap();
            assert_eq!(r.status(), 403, "{method} {path} {remote}");
            if *method != "HEAD" {
                let said: Value = r.json().await.unwrap();
                assert!(said["error"].as_str().unwrap().contains("only allowed from loopback"), "{said}");
            }
        }
    }
    assert_eq!(b.text(), None, "a refused save writes nothing");
    assert_eq!(b.h.app.config().deny, Config::default().deny);
}

#[tokio::test]
async fn every_settings_route_refuses_a_paired_remote_browser() {
    let remote = json!({"mode": "pairing", "origin": "https://board.acme.example:47732"});
    refuses_signed_in_remote_requests(remote, &["pairing"]).await;
}

#[tokio::test]
async fn every_settings_route_refuses_a_tailscale_user() {
    let remote = json!({"mode": "tailscale", "origin": "https://board.acme.example:47732"});
    refuses_signed_in_remote_requests(remote, &["tailscale"]).await;
}

#[tokio::test]
async fn every_settings_route_refuses_a_paired_tailscale_user() {
    let remote = json!({"mode": "tailscale", "origin": "https://board.acme.example:47732", "requirePairing": true});
    refuses_signed_in_remote_requests(remote, &["tailscale", "pairing"]).await;
}

#[tokio::test]
async fn every_settings_route_keeps_the_board_guard() {
    let b = Board::start(H::new()).await;
    let body = json!({"revision": "absent", "file": {"deny": []}});
    let refused = [
        ("host", "board.acme.example"),
        ("host", "127.0.0.1:1"),
        ("origin", "https://evil.example"),
        ("origin", "http://127.0.0.1:1"),
        ("x-forwarded-for", "100.64.0.9"),
        ("tailscale-user-login", "alice@acme.example"),
    ];
    for (method, path) in ROUTES {
        for (name, value) in refused {
            let req = b.request(method, path).header("X-T3CM-CSRF", &b.h.app.csrf).header(name, value).json(&body);
            assert_eq!(req.send().await.unwrap().status(), 403, "{method} {path} {name}: {value}");
        }
    }
    let put = || b.request("PUT", "/api/settings").json(&body);
    assert_eq!(put().send().await.unwrap().status(), 403, "no CSRF token");
    assert_eq!(put().header("X-T3CM-CSRF", "guess").send().await.unwrap().status(), 403, "wrong CSRF token");
    assert_eq!(b.text(), None);
}

#[tokio::test]
async fn execute_code_has_no_config_global() {
    let h = H::new();
    assert_eq!(result(&h.exec("async () => typeof wf.config").await), "undefined");
    for method in ["config.get", "config.set", "settings"] {
        assert!(h.app.wf.dispatch(method, Some(BOSS), json!({})).await.is_err(), "{method}");
    }
}

#[tokio::test]
async fn settings_show_each_value_and_whether_the_file_sets_it() {
    let b = Board::start(H::new()).await;
    let fresh = b.settings().await;
    assert_eq!(fresh["revision"], "absent");
    assert_eq!(fresh["file"], json!({}));
    assert_eq!(fresh["values"], effective(&Config::default()));
    assert_eq!(fresh["running"], fresh["values"]);
    assert_eq!((&fresh["errors"], &fresh["restart"]), (&json!([]), &json!([])));
    assert_eq!(fresh["path"], json!(b.file().to_string_lossy()));

    let file = json!({"maxConcurrentRuns": 4, "board": {"feed": 7, "settleAfterHours": 24}});
    b.write(&file.to_string());
    let shown = b.settings().await;
    assert_eq!(shown["file"], file);
    assert_eq!(shown["file"]["board"]["settleAfterHours"], 24, "a key set to its default still counts as set");
    assert_eq!(shown["file"]["board"].get("askPollSecs"), None);
    assert_eq!(shown["values"]["board"]["askPollSecs"], 4);
    assert_eq!(shown["values"]["board"]["feed"], 7);
    assert_eq!(shown["running"]["board"]["feed"], 50, "a hand edit waits for a reload");
    assert_eq!(shown["restart"], json!(["maxConcurrentRuns"]));
    assert_ne!(shown["revision"], fresh["revision"]);
}

#[tokio::test]
async fn a_save_applies_at_once_and_returns_what_waits_for_a_restart() {
    let b = Board::start(H::new()).await;
    let first = b.saved(json!({"maxConcurrentRuns": 4, "board": {"feed": 7}})).await;
    assert_eq!(first["restart"], json!(["maxConcurrentRuns"]));
    assert_eq!(first["warnings"], json!([]));
    assert_eq!((b.h.app.config().board.feed, b.h.app.config().max_concurrent_runs), (7, 16));

    let body = json!({"revision": first["revision"], "file": {"maxConcurrentRuns": 4, "deny": ["t3_thread_launch"]}});
    let (status, next) = b.put(&body).await;
    assert_eq!(status, 200, "the returned revision is good for the next save: {next}");
    assert_eq!(next["restart"], json!(["maxConcurrentRuns"]), "an earlier change still waits");
    assert_eq!(b.h.app.config().deny, ["t3_thread_launch"]);
    assert_eq!(b.h.app.config().board.feed, 50, "a key left out takes its default");

    assert_eq!(b.saved(json!({})).await["restart"], json!([]));
    assert_eq!(b.h.app.config().deny, Config::default().deny);
}

#[tokio::test]
async fn a_save_that_changes_nothing_still_reloads() {
    let b = Board::start(H::new()).await;
    let text = "{ \"board\": {\"feed\":  9} }";
    b.write(text);
    b.saved(json!({"board": {"feed": 9}})).await;
    assert_eq!(b.text().unwrap(), text);
    assert_eq!(b.h.app.config().board.feed, 9);
}

#[tokio::test]
async fn a_save_keeps_the_layout_of_what_it_does_not_change() {
    let b = Board::start(H::new()).await;
    let text = "{\n    \"deny\":  [\"a\", \"b\"],\n\n    \"board\": { \"settleAfterHours\": 48, \"feed\": 50 }\n}\n";
    b.write(text);
    b.saved(json!({"board": {"feed": 80, "settleAfterHours": 48}, "deny": ["a", "b"]})).await;
    assert_eq!(b.text().unwrap(), text.replace("50", "80"));
}

#[tokio::test]
async fn every_key_saves_through_the_board() {
    let b = Board::start(H::new()).await;
    b.h.fake.set_providers(vec![
        provider("codex-a", "codex", &["gpt-6.1"]),
        provider("codex-b", "codex", &["gpt-6.1"]),
        provider("claude-a", "claudeAgent", &["claude-opus-5-5"]),
    ]);
    let mirror = b.h.dir.path().join("mirror.json");
    for (path, value) in every_kind_of_key() {
        apply(&mirror, set(path, value));
        let want = Document::read(&mirror).unwrap().value().unwrap();
        b.saved(want.clone()).await;
        assert_eq!(Document::read(&b.file()).unwrap().value().unwrap(), want, "{path}");
    }
    assert_eq!(b.settings().await["values"], effective(&Config::load(&mirror).unwrap()));

    b.saved(json!({"bodyCap": 900})).await;
    assert_eq!(Document::read(&b.file()).unwrap().value().unwrap(), json!({"bodyCap": 900}));
}

#[tokio::test]
async fn a_list_grows_and_shrinks_as_a_whole_value() {
    let b = Board::start(H::new()).await;
    let docs = |allow: Value| json!({"name": "docs", "transport": "stdio", "stdio": {"command": "d"}, "allow": allow});
    for allow in [json!(["*", "search*"]), json!(["*", "search*", "read*"]), json!(["read*"])] {
        b.saved(json!({"upstreams": [docs(allow.clone())]})).await;
        assert_eq!(json!(Config::load(&b.file()).unwrap().upstreams[0].allow), allow);
    }
}

#[tokio::test]
async fn an_invalid_save_names_its_field_and_leaves_the_file() {
    let b = Board::start(H::new()).await;
    let text = "{\"board\":   {\"feed\": 10}}";
    b.write(text);
    b.h.app.wf.reload_config().await.unwrap();
    let cases = [
        (json!({"board": {"feed": "lots"}}), "board.feed"),
        (json!({"board": {"typo": 1}}), "board.typo"),
        (json!({"board": {"askPool": "ghost"}}), "board.askPool"),
        (json!({"link": {"stall": 1}}), "link.stall"),
        (json!({"wake": ["event", "note"]}), "wake.1"),
        (json!({"deny": "t3_thread_launch"}), "deny"),
        (json!({"board": {"remote": {"mode": "pairing"}}}), "board.remote.origin"),
        (json!({"pools": {"gpt-6.1": {"driver": "codex", "model": "m", "members": []}}}), "pools.\"gpt-6.1\".members"),
        (json!({"upstreams": [{"name": "wf", "transport": "stdio", "stdio": {"command": "x"}}]}), "upstreams.0.name"),
    ];
    for (file, want) in cases {
        let (status, body) = b.save(file.clone()).await;
        assert_eq!(status, 422, "{file}: {body}");
        assert_eq!(body["errors"][0]["field"], want, "{file}: {body}");
        assert!(!body["errors"][0]["message"].as_str().unwrap().is_empty());
        assert!(!body["error"].as_str().unwrap().is_empty());
        assert_eq!(b.text().unwrap(), text, "{file}");
        assert_eq!(b.h.app.config().board.feed, 10, "{file}");
    }
}

#[tokio::test]
async fn a_save_against_a_stale_revision_is_refused() {
    let b = Board::start(H::new()).await;
    let opened = b.settings().await["revision"].clone();
    let theirs = "{\"bodyCap\": 900}\n";
    b.write(theirs);
    for file in [json!({"board": {"feed": 7}}), json!({"bodyCap": 900}), json!({"board": {"feed": "invalid"}})] {
        let (status, body) = b.put(&json!({"revision": opened, "file": file})).await;
        assert_eq!(status, 409, "{body}");
        assert!(body["error"].as_str().unwrap().contains("changed on disk"), "{body}");
        assert_eq!(b.text().unwrap(), theirs);
    }
    assert_eq!(b.h.app.config().board.feed, 50);

    let seen = b.settings().await["revision"].clone();
    std::fs::remove_file(b.file()).unwrap();
    assert_eq!(b.put(&json!({"revision": seen, "file": {}})).await.0, 409);
    assert_eq!(b.text(), None, "a deleted file is not written back");
}

#[tokio::test(start_paused = true)]
async fn a_change_during_the_member_check_is_not_overwritten() {
    let h = H::new();
    let path = h.dir.path().join("t3cm-config.json");
    let pool = json!({"mixed": true, "members": [{"instanceId": "claudeAgent", "model": "claude-opus-5-5"}]});
    let body = json!({"revision": Document::read(&path).unwrap().revision(), "file": {"pools": {"p": pool}}});
    h.fake.set_delay(Duration::from_secs(1));
    let wf = h.app.wf.clone();
    let saving = tokio::spawn(async move { wf.save_settings(serde_json::from_value(body).unwrap()).await });
    tokio::time::sleep(Duration::from_millis(100)).await;
    std::fs::write(&path, "{\"bodyCap\": 900}\n").unwrap();
    assert!(saving.await.unwrap().unwrap_err().is::<Stale>());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "{\"bodyCap\": 900}\n");
    assert!(h.app.config().pools.is_empty());
}

#[tokio::test]
async fn a_save_takes_only_a_whole_object_and_a_revision() {
    let b = Board::start(H::new()).await;
    let text = "{\"bodyCap\": 900}";
    b.write(text);
    let revision = b.settings().await["revision"].clone();
    for body in [
        json!({"revision": revision, "file": null}),
        json!({"revision": revision, "file": []}),
        json!({"revision": revision}),
        json!({"file": {}}),
        json!({"revision": revision, "file": {}, "edits": []}),
    ] {
        assert_eq!(b.put(&body).await.0, 400, "{body}");
        assert_eq!(b.text().unwrap(), text);
    }
}

#[tokio::test]
async fn an_invalid_file_still_opens() {
    let b = Board::start(H::new()).await;
    b.write("{\"board\": ");
    let broken = b.settings().await;
    assert_eq!((&broken["file"], &broken["values"], &broken["restart"]), (&Value::Null, &Value::Null, &Value::Null));
    assert!(broken["errors"][0]["message"].as_str().unwrap().contains("not valid JSON"), "{broken}");
    assert_eq!(broken["running"]["board"]["feed"], 50);
    assert_eq!(b.save(json!({})).await.0, 422);
    assert_eq!(b.text().unwrap(), "{\"board\": ", "broken JSON is never replaced blind");

    b.write(r#"{"board": {"feed": "lots"}}"#);
    let invalid = b.settings().await;
    assert_eq!(invalid["file"]["board"]["feed"], "lots");
    assert_eq!(invalid["errors"][0]["field"], "board.feed");
    b.saved(json!({"board": {"feed": 9}})).await;
    assert_eq!(b.h.app.config().board.feed, 9);
}

#[cfg(unix)]
#[tokio::test]
async fn a_dangling_symlink_is_an_error_and_not_an_empty_config() {
    let b = Board::start(H::new()).await;
    std::os::unix::fs::symlink(b.h.dir.path().join("gone.json"), b.file()).unwrap();
    let r = b.request("GET", "/api/settings").send().await.unwrap();
    assert_eq!(r.status(), 500);
    assert!(r.json::<Value>().await.unwrap()["error"].as_str().unwrap().contains("symlink"));
}

#[tokio::test]
async fn a_changed_pool_has_its_members_checked() {
    let b = Board::start(H::new()).await;
    b.h.fake.set_providers(vec![
        provider("claude-a", "claudeAgent", &["claude-opus-5-5"]),
        provider("codex-a", "codex", &["gpt-6-astra"]),
    ]);
    let pools = |id: &str| {
        let pool = json!({"driver": "claudeAgent", "model": "claude-opus-5-5", "members": [{"instanceId": id}]});
        json!({"pools": {"opus": pool}})
    };
    for (id, why) in [("ghost", "not a T3 provider instance"), ("codex-a", "is a codex instance")] {
        let (status, body) = b.save(pools(id)).await;
        assert_eq!(status, 422, "{body}");
        assert_eq!(body["errors"][0]["field"], "pools.opus.members.0.instanceId");
        assert!(body["errors"][0]["message"].as_str().unwrap().contains(why), "{body}");
        assert_eq!(b.text(), None);
    }

    b.h.fake.set_caps_fail(true);
    let unchecked = b.saved(pools("ghost")).await;
    assert!(unchecked["warnings"][0].as_str().unwrap().contains("not checked against T3"), "{unchecked}");

    b.h.fake.set_caps_fail(false);
    let mut unrelated = pools("ghost");
    unrelated["bodyCap"] = json!(900);
    assert_eq!(b.saved(unrelated).await["warnings"], json!([]), "a pool left alone is not checked again");
}

#[tokio::test]
async fn pool_member_state_is_live_and_separate_from_the_file() {
    let b = Board::start(H::new()).await;
    b.h.fake.set_providers(vec![
        provider("claude-a", "claudeAgent", &["claude-opus-5-5"]),
        provider("claude-b", "claudeAgent", &["claude-opus-5-5"]),
    ]);
    let members = json!([{"instanceId": "claude-a"}, {"instanceId": "claude-b"}]);
    let pool = json!({"driver": "claudeAgent", "model": "claude-opus-5-5", "members": members});
    let saved = b.saved(json!({"pools": {"opus": pool}})).await;
    let limit = Limit { until: at(90), reason: None, at: at(0), thread: None };
    b.h.app.store.limit_set("claude-b", &limit).unwrap();

    let shown = b.get("/api/settings/pools").await;
    let states: Vec<&Value> = shown["pools"][0]["members"].as_array().unwrap().iter().map(|m| &m["state"]).collect();
    assert_eq!(states, ["ready", "limited"]);
    assert_eq!(shown["pools"], b.h.wf(BOSS, "pools", json!({})).await);
    assert_eq!(b.settings().await["revision"], saved["revision"], "live state never makes a form stale");

    b.h.fake.set_caps_fail(true);
    let down = b.get("/api/settings/pools").await;
    assert_eq!(down["pools"][0]["members"][0]["state"], "unknown");
    assert!(down["pools"][0]["error"].as_str().unwrap().contains("T3 did not answer"), "{down}");
    assert_eq!(b.settings().await["values"]["pools"]["opus"]["model"], "claude-opus-5-5", "the file needs no T3");
}
