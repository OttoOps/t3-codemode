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

use crate::board::run_page_async_with;
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

    async fn page(&self) -> String {
        self.request("GET", "/").send().await.unwrap().text().await.unwrap()
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

const REMOTE_HOST: &str = "board.acme.example:47732";
const OWNER: &str = "alice@acme.example";
const OTHER: &str = "bob@acme.example";

struct Remote {
    b: Board,
    origin: String,
    signed_in: Vec<(&'static str, String)>,
}

impl Remote {
    async fn start(remote: Value, login: Option<&str>, paired: bool) -> Remote {
        let origin = format!("https://{REMOTE_HOST}");
        let h = H::build(Vec::new(), remote_config(remote));
        h.tailscale.set_host_login(OWNER);
        let mut signed_in = Vec::new();
        if let Some(login) = login {
            signed_in.push(("tailscale-user-login", login.to_string()));
        }
        if paired {
            let (store, now) = (&h.app.store, h.clock.now());
            let minted = pairing::mint_pairing_token(store, &RemoteOrigin::parse(&origin).unwrap(), None, now).unwrap();
            let (session, _) = pairing::redeem_pairing_token(store, &minted.token, None, now).unwrap().unwrap();
            signed_in.push(("cookie", format!("{}={session}", pairing::SESSION_COOKIE)));
        }
        Remote { b: Board::start(h).await, origin, signed_in }
    }

    fn routes() -> [Vec<(&'static str, String)>; 2] {
        let by_host = vec![("host", REMOTE_HOST.to_string()), ("x-forwarded-proto", "https".to_string())];
        let by_proxy = vec![("x-forwarded-host", REMOTE_HOST.to_string()), ("x-forwarded-proto", "https".to_string())];
        [by_host, by_proxy]
    }

    fn send(&self, route: &[(&'static str, String)], method: &str, path: &str) -> reqwest::RequestBuilder {
        let mut req = self.b.request(method, path).header("origin", &self.origin);
        for (name, value) in route.iter().chain(&self.signed_in) {
            req = req.header(*name, value);
        }
        req
    }

    async fn page(&self, route: &[(&'static str, String)], settings: u16) -> Value {
        let allowed = self.send(route, "GET", "/api/models").send().await.unwrap();
        assert_eq!(allowed.status(), 200, "the request must be authenticated");
        let page = self.send(route, "GET", "/").send().await.unwrap().text().await.unwrap();
        assert!(!page.contains(r#"id="models-btn""#), "one Settings button, no Models button");
        assert!(!page.contains(r#"id="models-dlg""#), "one Settings dialog, no Models dialog");
        let dialog = &page[page.find(r#"<dialog id="settings-dlg""#).unwrap()..];
        let dialog = &dialog[..dialog.find("</dialog>").unwrap()];
        for picker in [r#"id="tm-save""#, r#"id="am-save""#] {
            assert!(dialog.contains(picker), "the Settings dialog holds the model pickers");
        }
        let prelude = format!(
            r#"globalThis.__fetchReply = (url) => {{
  if (url !== "/api/settings") return new Promise(() => {{}});
  return Promise.resolve({{ ok: {settings} < 300, status: {settings}, json: () => Promise.resolve({{}}) }});
}};"#
        );
        run_page_async_with(
            &page,
            &prelude,
            "async () => { for (let i = 0; i < 5; i++) await null; \
             const dlg = $(\"settings-dlg\"); \
             $(\"settings-btn\").dispatch(\"click\"); \
             return { local: LOCAL, settings: SETTINGS, sections: conf.panels.size, gear: $(\"settings-btn\").hidden === false, \
               open: !!dlg.open, modelsOnly: dlg.classList.contains(\"models-only\") }; }",
        )
    }

    async fn refused(&self, remote: &Value, why: &str) {
        let revision = self.b.settings().await["revision"].clone();
        self.b.h.fake.set_providers(vec![provider("claudeAgent", "claudeAgent", &["claude-opus-5-5"])]);
        for route in Remote::routes() {
            let opened = self.page(&route, 403).await;
            assert_eq!(
                opened,
                json!({"local": false, "settings": false, "sections": 0, "gear": true, "open": true, "modelsOnly": true}),
                "a refused board opens Settings with only the model pickers: {remote}"
            );
            let pick =
                json!({"mode": "model", "provider": "claudeAgent", "model": "claude-opus-5-5", "effort": "high"});
            let req = self.send(&route, "PUT", "/api/models/ticket").header("X-T3CM-CSRF", &self.b.h.app.csrf);
            let saved = req.json(&pick).send().await.unwrap();
            assert_eq!(saved.status(), 200, "a refused board still saves its ticket model: {remote}");
            for (method, path) in ROUTES {
                let body = json!({"revision": revision, "file": {"deny": []}});
                let req = self.send(&route, method, path).header("X-T3CM-CSRF", &self.b.h.app.csrf);
                let r = req.json(&body).send().await.unwrap();
                assert_eq!(r.status(), 403, "{method} {path} {remote}");
                if *method != "HEAD" {
                    let said: Value = r.json().await.unwrap();
                    assert!(said["error"].as_str().unwrap().contains(why), "{said}");
                }
            }
        }
        assert_eq!(self.b.text(), None, "a refused save writes nothing");
        assert_eq!(self.b.h.app.config().deny, Config::default().deny);
    }

    async fn allowed(&self) {
        for route in Remote::routes() {
            let opened = self.page(&route, 200).await;
            assert_eq!(opened["settings"], true, "{opened}");
            assert_eq!(opened["gear"], true, "the owner's board shows the gear: {opened}");
            assert_eq!(opened["modelsOnly"], false, "the owner's Settings shows every section: {opened}");
            assert!(opened["sections"].as_u64().unwrap() > 0, "{opened}");
            for (method, path) in ROUTES.iter().filter(|(m, _)| *m != "PUT") {
                let r = self.send(&route, method, path).send().await.unwrap();
                assert_eq!(r.status(), 200, "{method} {path}");
            }
            let revision = self.b.settings().await["revision"].clone();
            let body = json!({"revision": revision, "file": {"board": {"feed": 33}}});
            let put = || self.send(&route, "PUT", "/api/settings").json(&body);
            assert_eq!(put().send().await.unwrap().status(), 403, "no CSRF token");
            assert_eq!(put().header("X-T3CM-CSRF", "guess").send().await.unwrap().status(), 403, "wrong CSRF token");
            assert_eq!(self.b.text(), None, "a save without the token writes nothing");
            let saved = put().header("X-T3CM-CSRF", &self.b.h.app.csrf).send().await.unwrap();
            assert_eq!(saved.status(), 200);
            assert_eq!(self.b.h.app.config().board.feed, 33);
            std::fs::remove_file(self.b.file()).unwrap();
        }
    }
}

const TAILSCALE: &str = r#"{"mode": "tailscale", "origin": "https://board.acme.example:47732"}"#;

fn tailscale(extra: Value) -> Value {
    let mut remote: Value = serde_json::from_str(TAILSCALE).unwrap();
    remote.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    remote
}

#[tokio::test]
async fn every_settings_route_refuses_a_paired_remote_browser() {
    let remote = json!({"mode": "pairing", "origin": "https://board.acme.example:47732"});
    Remote::start(remote.clone(), None, true).await.refused(&remote, "in Tailscale mode, the board's owner").await;
}

#[tokio::test]
async fn a_pairing_session_with_the_owner_login_header_is_still_refused() {
    let remote = json!({"mode": "pairing", "origin": "https://board.acme.example:47732"});
    Remote::start(remote.clone(), Some(OWNER), true).await.refused(&remote, "in Tailscale mode").await;
}

#[tokio::test]
async fn the_tailscale_owner_reaches_every_settings_route() {
    Remote::start(tailscale(json!({})), Some(OWNER), false).await.allowed().await;
}

#[tokio::test]
async fn a_paired_tailscale_owner_reaches_every_settings_route() {
    Remote::start(tailscale(json!({"requirePairing": true})), Some(OWNER), true).await.allowed().await;
}

#[tokio::test]
async fn every_settings_route_refuses_another_allowed_tailscale_user() {
    let remote = tailscale(json!({"allowUsers": [OWNER, OTHER]}));
    Remote::start(remote.clone(), Some(OTHER), false).await.refused(&remote, "board.remote.settingsUsers").await;
    let paired = tailscale(json!({"allowUsers": [OWNER, OTHER], "requirePairing": true}));
    Remote::start(paired.clone(), Some(OTHER), true).await.refused(&paired, "board.remote.settingsUsers").await;
}

#[tokio::test]
async fn settings_users_replace_the_host_login() {
    let remote = tailscale(json!({"allowUsers": [OWNER, OTHER], "settingsUsers": [OTHER]}));
    Remote::start(remote.clone(), Some(OTHER), false).await.allowed().await;
    Remote::start(remote.clone(), Some(OWNER), false).await.refused(&remote, "board.remote.settingsUsers").await;
}

#[tokio::test]
async fn remote_settings_need_https() {
    let r = Remote::start(tailscale(json!({})), Some(OWNER), false).await;
    let plain = [vec![("host", REMOTE_HOST.to_string())], vec![("x-forwarded-host", REMOTE_HOST.to_string())]];
    for route in plain {
        assert_eq!(r.send(&route, "GET", "/api/models").send().await.unwrap().status(), 200, "the board still answers");
        for (method, path) in ROUTES {
            let req = r.send(&route, method, path).header("X-T3CM-CSRF", &r.b.h.app.csrf);
            let res = req.json(&json!({"revision": "absent", "file": {"deny": []}})).send().await.unwrap();
            assert_eq!(res.status(), 403, "{method} {path}");
            if *method != "HEAD" {
                let said: Value = res.json().await.unwrap();
                assert!(said["error"].as_str().unwrap().contains("X-Forwarded-Proto: https"), "{said}");
            }
        }
    }
    assert_eq!(r.b.text(), None);
}

#[tokio::test]
async fn a_loopback_board_builds_settings_without_asking() {
    let b = Board::start(H::new()).await;
    let opened = run_page_async_with(
        &b.page().await,
        "",
        "async () => ({ local: LOCAL, settings: SETTINGS, built: conf.panels.size > 0, gear: $(\"settings-btn\").hidden === false })",
    );
    assert_eq!(opened, json!({"local": true, "settings": true, "built": true, "gear": true}));
}

#[tokio::test]
async fn a_change_link_opens_settings_at_its_picker() {
    let b = Board::start(H::new()).await;
    let page = b.page().await;
    assert_eq!(page.matches(r#"id="tm-save""#).count(), 1, "one ticket picker");
    assert!(!page.contains(r#"id="models-dlg""#));
    let r = run_page_async_with(
        &page,
        "globalThis.__fetchReply = () => new Promise(() => {});",
        r#"async () => {
  const dlg = $("settings-dlg");
  showSection("board");
  closeDialog(dlg);
  $("ask-model-edit").dispatch("click");
  return {
    open: !!dlg.open, modelsOnly: dlg.classList.contains("models-only"), tab: conf.tab,
    focus: document.activeElement.id, home: $("md-sec-ask").parentNode.id,
  };
}"#,
    );
    assert_eq!(
        r,
        json!({"open": true, "modelsOnly": false, "tab": "launches", "focus": "am-mode-model", "home": "settings-pickers"})
    );
}

#[test]
fn changed_keys_name_each_leaf_that_differs() {
    let old = json!({"board": {"feed": 10, "remote": {"mode": "off"}}, "deny": ["a"], "pools": {"a.b": {"x": 1}}});
    let new = json!({"board": {"feed": 10, "remote": {"mode": "tailscale", "origin": "o"}}, "deny": ["a", "b"], "maxConcurrentRuns": 2});
    assert_eq!(
        t3_codemode::config_edit::changed_keys(&old, &new),
        ["board.remote.mode", "board.remote.origin", "deny", "maxConcurrentRuns", "pools"]
    );
    assert!(t3_codemode::config_edit::changed_keys(&old, &old).is_empty());
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
    assert_eq!(fresh["defaults"], effective(&Config::default()));
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
    assert_eq!(shown["defaults"]["board"]["feed"], 50, "the file does not move the defaults");
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
        (
            json!({"board": {"remote": {"mode": "tailscale", "origin": "https://board.acme.example", "allowUsers": ["alice@acme.example"], "settingsUsers": ["bob@acme.example"]}}}),
            "board.remote.settingsUsers.0",
        ),
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

    b.h.fake.fail_tool("orchestrator_capabilities");
    let unchecked = b.saved(pools("ghost")).await;
    assert!(unchecked["warnings"][0].as_str().unwrap().contains("not checked against T3"), "{unchecked}");

    b.h.fake.clear_faults_on("orchestrator_capabilities");
    let mut unrelated = pools("ghost");
    unrelated["bodyCap"] = json!(900);
    assert_eq!(b.saved(unrelated).await["warnings"], json!([]), "a pool left alone is not checked again");
}

#[tokio::test]
async fn repairing_another_field_leaves_an_unchanged_pool_unchecked() {
    let b = Board::start(H::new()).await;
    let pools = |id: &str| json!({"p": {"mixed": true, "members": [{"instanceId": id, "model": "m"}]}});
    let broken = json!({"board": {"feed": "lots"}, "pools": pools("retired")}).to_string();
    b.write(&broken);
    let (status, body) = b.save(json!({"board": {"feed": 9}, "pools": pools("ghost")})).await;
    assert_eq!(status, 422, "a changed pool is still checked: {body}");
    assert_eq!(body["errors"][0]["field"], "pools.p.members.0.instanceId");
    assert_eq!(b.text().unwrap(), broken);
    assert_eq!(b.saved(json!({"board": {"feed": 9}, "pools": pools("retired")})).await["warnings"], json!([]));
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

    b.h.fake.fail_tool("orchestrator_capabilities");
    let down = b.get("/api/settings/pools").await;
    assert_eq!(down["pools"][0]["members"][0]["state"], "unknown");
    assert!(down["pools"][0]["error"].as_str().unwrap().contains("T3 did not answer"), "{down}");
    assert_eq!(b.settings().await["values"]["pools"]["opus"]["model"], "claude-opus-5-5", "the file needs no T3");
}

const DIALOG_HELPERS: &str = r##"
const settle = async () => { for (let i = 0; i < 50; i++) await null; };
const field = (key) => [...document.querySelectorAll("[data-conf]")].find((el) => el.dataset.conf === key);
const type = (el, text) => { el.value = text; el.dispatch("input"); };
const locked = (el) => {
  if (el.hasAttribute("readonly")) return true;
  for (let at = el; at && at.hasAttribute; at = at.parentNode) if (at.hasAttribute("inert")) return true;
  return false;
};
const enter = (el, text) => { if (!locked(el)) type(el, text); };
const byKey = (el, text) => {
  enter(el, "");
  for (let n = 1; n <= text.length; n++) enter(el, text.slice(0, n));
};
const insert = (el, data, gives, inputType = "insertText") => {
  if (locked(el) || el.dispatch("beforeinput", { data, inputType }).defaultPrevented || el.value === gives) return;
  el.value = gives;
  el.dispatch("input", { data, inputType });
};
const undo = (el, text) => {
  if (!el.isConnected) return;
  el.value = text;
  el.dispatch("input", { inputType: "historyUndo" });
};
const pick = (el, value) => { el.value = value; el.dispatch("change"); };
const typedBox = (key) => field(key).parentNode.querySelector("input");
const puts = () => fetches.filter((f) => f.method === "PUT").map((f) => JSON.parse(f.body));
const open = async () => { openSettings(); await settle(); };
"##;

async fn drive(before: &Value, scenario: &str) -> (Board, Value) {
    let b = Board::start(H::new()).await;
    b.write(&before.to_string());
    let opened = b.settings().await;
    let saved = json!({"revision": "next", "restart": [], "warnings": []});
    let r = run_dialog(&b.page().await, &[&opened], (200, &saved), scenario);
    (b, r)
}

fn run_dialog(html: &str, gets: &[&Value], put: (u16, &Value), scenario: &str) -> Value {
    let gets = json!(gets);
    let (status, body) = put;
    let prelude = format!(
        r#"const settingsReplies = {gets};
globalThis.__fetchReply = (url, opts) => {{
  if (url !== "/api/settings") return new Promise(() => {{}});
  const get = !opts || opts.method !== "PUT";
  const code = get ? 200 : {status};
  const data = !get ? {body} : settingsReplies.length > 1 ? settingsReplies.shift() : settingsReplies[0];
  return Promise.resolve({{ ok: code < 300, status: code, json: () => Promise.resolve(JSON.parse(JSON.stringify(data))) }});
}};"#
    );
    let r = run_page_async_with(html, &prelude, &format!("async () => {{\n{DIALOG_HELPERS}\n{scenario}\n}}"));
    assert_eq!(r.get("error"), None, "{r}");
    r
}

#[tokio::test]
async fn the_dialog_saves_a_copy_of_the_file_with_only_the_edit() {
    let b = Board::start(H::new()).await;
    let name = "<img src=x onerror=alert(1)>";
    let pool = json!({"mixed": true, "members": [{"instanceId": "claudeAgent", "model": "claude-opus-5-5"}]});
    let before = json!({"board": {"settleAfterHours": 48}, "pools": {name: pool}});
    b.write(&before.to_string());
    let opened = b.settings().await;
    let saved = json!({"revision": "next", "restart": [], "warnings": []});
    let r = run_dialog(
        &b.page().await,
        &[&opened],
        (200, &saved),
        r#"await open();
  showSection("board");
  type(field("board.feed"), "80");
  await saveSettings();
  const pools = conf.panels.get("pools").body;
  return { puts: puts(), heading: pools.querySelector("h4").textContent, markup: pools.querySelectorAll("img").length };"#,
    );
    assert_eq!(r["heading"], name, "a config string renders as text: {r}");
    assert_eq!(r["markup"], 0, "{r}");
    let puts = r["puts"].as_array().unwrap();
    assert_eq!(puts.len(), 1, "{r}");
    let mut want = before;
    want["board"]["feed"] = json!(80);
    assert_eq!(puts[0], json!({"revision": opened["revision"], "file": want}), "only the edit joins the file");
    assert_eq!(b.put(&puts[0]).await.0, 200);
    assert_eq!(Document::read(&b.file()).unwrap().value().unwrap(), want);
}

#[tokio::test]
async fn the_dialog_shows_a_rejected_edit_beside_its_field() {
    let b = Board::start(H::new()).await;
    let opened = b.settings().await;
    let (status, refused) = b.save(json!({"link": {"stall": 1}})).await;
    assert_eq!(status, 422);
    let r = run_dialog(
        &b.page().await,
        &[&opened],
        (status, &refused),
        r#"await open();
  showSection("links");
  const input = field("link.stall");
  type(input, "1");
  showSection("board");
  await saveSettings();
  const shown = field("link.stall");
  const error = shown.parentNode.parentNode.querySelector(".conf-err");
  return {
    tab: conf.tab,
    focused: document.activeElement === shown,
    invalid: shown.getAttribute("aria-invalid"),
    shown: error.hidden ? "" : error.textContent,
    note: $("settings-notes").textContent,
  };"#,
    );
    assert_eq!((&r["tab"], &r["focused"], &r["invalid"]), (&json!("links"), &json!(true), &json!("true")), "{r}");
    assert!(r["shown"].as_str().unwrap().contains("must exceed down"), "{r}");
    assert!(r["note"].as_str().unwrap().contains("The file is unchanged"), "{r}");
    assert_eq!(b.text(), None);
}

#[tokio::test]
async fn the_dialog_never_saves_over_a_stale_revision() {
    let b = Board::start(H::new()).await;
    let opened = b.settings().await;
    b.write(r#"{"bodyCap": 900}"#);
    let (status, refused) = b.put(&json!({"revision": opened["revision"], "file": {"board": {"feed": 80}}})).await;
    assert_eq!(status, 409);
    let theirs = b.settings().await;
    let r = run_dialog(
        &b.page().await,
        &[&opened, &theirs],
        (status, &refused),
        r#"await open();
  showSection("board");
  type(field("board.feed"), "80");
  await saveSettings();
  await settle();
  await saveSettings();
  return { puts: puts().length, note: $("settings-notes").textContent, draft: conf.draft };"#,
    );
    assert_eq!(r["puts"], 1, "a second Save sends nothing: {r}");
    assert!(r["note"].as_str().unwrap().contains("Changed there: bodyCap"), "{r}");
    assert_eq!(r["draft"], json!({"board": {"feed": 80}}), "the edit stays to copy: {r}");
}

#[tokio::test]
async fn the_dialog_has_a_control_or_an_editor_for_every_key() {
    let b = Board::start(H::new()).await;
    let every = json!({
        "pools": {
            "one": {"driver": "claudeAgent", "model": "m", "effort": "high", "moveOnLimit": "auto", "members": [{"instanceId": "claudeAgent"}]},
            "tier": {"mixed": true, "members": [{"instanceId": "claudeAgent", "model": "m", "effort": "low"}]},
        },
        "board": {"remote": {"mode": "tailscale", "origin": "https://board.acme.example", "allowUsers": ["alice@acme.example"], "requirePairing": true, "settingsUsers": ["alice@acme.example"]}},
        "flow": {
            "repos": ["acme/widgets"],
            "includeRepos": ["acme/extra"],
            "excludeRepos": ["acme/archive"],
            "optIn": {"label": "flow", "milestone": "v1.0"},
            "maxTickets": 2,
            "launch": "ask",
            "serializeFileConflicts": true,
        },
        "maxConcurrentRuns": 4,
        "upstreams": [{"name": "docs", "transport": "stdio", "stdio": {"command": "docs-mcp"}}],
    });
    b.write(&every.to_string());
    let opened = b.settings().await;
    assert_eq!(opened["errors"], json!([]));
    let r = run_dialog(
        &b.page().await,
        &[&opened],
        (200, &json!(null)),
        r#"await open();
  const controls = [...conf.fields.values()].flat().filter((f) => f.key && f.ctl !== f.err);
  const leaves = (v, at) => Array.isArray(v) && v.some(isObj) ? v.flatMap((x, i) => leaves(x, [...at, String(i)]))
    : isObj(v) && Object.keys(v).length ? Object.entries(v).flatMap(([k, x]) => leaves(x, [...at, k])) : [at];
  const jsonOnly = leaves(conf.data.values, [])
    .filter((path) => path[0] !== "pools" || getAt(conf.data.values, path) !== null)
    .map(pathKey)
    .filter((key) => !controls.some((f) => within(key, f.key)));
  const editor = JSON.parse(field("").value);
  return { jsonOnly, editor, effective: overDefaults(conf.data.defaults, conf.draft), restart: CONF_RESTART.map(pathKey) };"#,
    );
    let mut json_only: Vec<&str> = r["jsonOnly"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
    json_only.sort_unstable();
    assert_eq!(
        json_only,
        [
            "board.askEffort",
            "board.askInstanceId",
            "board.askModel",
            "board.askPool",
            "board.instanceId",
            "board.pool",
            "board.ticketEffort",
            "board.ticketModel",
            "forges",
            "maxTicketsTotal",
            "trackers.hosts",
            "trackers.jira",
        ],
        "a new key needs a control, or a place on this list of keys only the JSON editor holds"
    );
    assert_eq!(r["editor"], every, "the whole-file editor holds what the file sets");
    for (key, value) in opened["defaults"]["board"].as_object().unwrap() {
        if key != "remote" {
            assert_eq!(&r["effective"]["board"][key], value, "the editor shows an unset key: {key}");
        }
    }
    assert_eq!(r["restart"], opened["restart"], "the restart badges follow the service's list");
}

#[tokio::test]
async fn the_dialog_refuses_an_integer_it_would_round() {
    let b = Board::start(H::new()).await;
    let text = r#"{"maxTimeoutMs": 9007199254740993}"#;
    b.write(text);
    let huge = b.settings().await;
    assert_eq!(huge["errors"], json!([]), "the file is valid");
    let fine = {
        b.write("{}");
        b.settings().await
    };
    let page = b.page().await;
    let saved = json!({"revision": "next", "restart": [], "warnings": []});
    let r = run_dialog(
        &page,
        &[&huge],
        (200, &saved),
        r#"await open();
  await saveSettings();
  return { puts: puts().length, note: $("settings-notes").textContent, editable: !!conf.draft };"#,
    );
    assert_eq!((&r["puts"], &r["editable"]), (&json!(0), &json!(false)), "{r}");
    assert!(r["note"].as_str().unwrap().contains("too large"), "{r}");

    for (key, value) in [("maxTimeoutMs", "9007199254740993"), ("", r#"{"link": {"stall": 9007199254740993}}"#)] {
        let scenario = format!(
            r#"await open();
  type(field({key:?}), {value:?});
  await saveSettings();
  return {{ puts: puts().length, bad: Object.fromEntries([...conf.bad].map(([k, v]) => [k, v.message || v])) }};"#
        );
        let r = run_dialog(&page, &[&fine], (200, &saved), &scenario);
        assert_eq!(r["puts"], 0, "{key:?}: {r}");
        let bad = r["bad"].as_object().unwrap();
        assert_eq!(bad.keys().collect::<Vec<_>>(), vec![key], "{r}");
        assert!(bad[key].as_str().unwrap().contains("too large"), "{key:?}: {r}");
    }
}

#[tokio::test]
async fn the_whole_file_editor_waits_for_a_broken_section_draft() {
    let before = json!({"roles": {"ticket": {"children": {"worker": {"miss": 4}}}}});
    let (_, r) = drive(
        &before,
        r#"await open();
  showSection("links");
  enter(field("roles"), "{");
  showSection("json");
  const text = field("").value;
  enter(field(""), text.replace('"miss": 4', '"miss": 5'));
  enter(field(""), text);
  const held = { bad: [...conf.bad.keys()], readOnly: locked(field("")) };
  showSection("links");
  enter(field("roles"), "{}");
  showSection("json");
  return { held, editable: !locked(field("")) };"#,
    )
    .await;
    assert_eq!(r, json!({"held": {"bad": ["roles"], "readOnly": true}, "editable": true}), "replacing it is gone");
}

#[tokio::test]
async fn an_unrelated_whole_file_edit_keeps_an_invalid_number() {
    for before in [json!({"board": {"feed": 60}}), json!({"board": {"feed": 60}, "maxTimeoutMs": 300000})] {
        for input in ["byKey", "type"] {
            let b = Board::start(H::new()).await;
            b.write(&before.to_string());
            let opened = b.settings().await;
            let saved = json!({"revision": "next", "restart": [], "warnings": []});
            let scenario = format!(
                r#"await open();
  {input}(field("maxTimeoutMs"), "9007199254740993");
  showSection("json");
  const file = copyJson(conf.draft);
  file.board.feed = 80;
  enter(field(""), JSON.stringify(file));
  await saveSettings();
  return {{ puts: puts(), problems: confProblems().map((p) => p.field) }};"#
            );
            let r = run_dialog(&b.page().await, &[&opened], (200, &saved), &scenario);
            for put in r["puts"].as_array().unwrap() {
                b.put(put).await;
            }
            assert_eq!(r, json!({"puts": [], "problems": ["maxTimeoutMs"]}), "{input} into {before}");
            assert_eq!(b.text(), Some(before.to_string()));
        }
    }
}

#[tokio::test]
async fn an_invalid_number_is_fixed_in_its_section_and_not_in_the_file_editor() {
    let before = json!({"board": {"feed": 60}, "maxTimeoutMs": 300000});
    let (b, r) = drive(
        &before,
        r#"await open();
  showSection("runtime");
  byKey(field("maxTimeoutMs"), "9007199254740993");
  showSection("json");
  const readOnly = locked(field(""));
  const note = $("settings-notes").textContent;
  const text = field("").value;
  const file = JSON.parse(text);
  delete file.maxTimeoutMs;
  enter(field(""), pretty(file));
  enter(field(""), text);
  await saveSettings();
  const blocked = { bad: [...conf.bad.keys()], puts: puts() };
  showSection("runtime");
  byKey(field("maxTimeoutMs"), "400000");
  showSection("json");
  return { readOnly, note, blocked, editable: !locked(field("")), value: conf.draft.maxTimeoutMs };"#,
    )
    .await;
    for put in r["blocked"]["puts"].as_array().unwrap() {
        b.put(put).await;
    }
    assert_eq!(r["blocked"], json!({"bad": ["maxTimeoutMs"], "puts": []}), "cut and pasted back: {r}");
    assert_eq!(b.text(), Some(before.to_string()));
    assert_eq!(r["readOnly"], true, "{r}");
    assert!(r["note"].as_str().unwrap().contains("maxTimeoutMs: fix it under Runtime"), "{r}");
    assert_eq!((&r["editable"], &r["value"]), (&json!(true), &json!(400000)), "{r}");
}

#[tokio::test]
async fn broken_file_text_holds_the_other_sections() {
    let b = Board::start(H::new()).await;
    let before = json!({"board": {"feed": 60}, "link": {"down": 45, "stall": 240}});
    b.write(&before.to_string());
    let opened = b.settings().await;
    let saved = json!({"revision": "next", "restart": [], "warnings": []});
    let r = run_dialog(
        &b.page().await,
        &[&opened],
        (200, &saved),
        r#"await open();
  showSection("json");
  const text = field("").value;
  enter(field(""), text + ",");
  showSection("links");
  byKey(field("link.stall"), "1");
  const held = locked(field("link.stall"));
  const said = $("settings-status").textContent;
  await saveSettings();
  const back = conf.tab;
  enter(field(""), text);
  showSection("links");
  byKey(field("link.stall"), "1");
  await saveSettings();
  return { held, said, back, puts: puts() };"#,
    );
    assert_eq!((&r["held"], &r["back"]), (&json!(true), &json!("json")), "{r}");
    assert!(r["said"].as_str().unwrap().contains("Fix All keys (JSON) before you edit another section"), "{r}");
    let puts = r["puts"].as_array().unwrap();
    assert_eq!(puts.len(), 1, "{r}");
    assert_eq!(puts[0]["file"]["link"]["stall"], 1, "the edit made after the repair is the one sent: {r}");
    assert_eq!(b.put(&puts[0]).await.0, 422);
    assert_eq!(b.text(), Some(before.to_string()));
}

#[tokio::test]
async fn an_invalid_member_number_makes_the_file_editor_read_only() {
    let b = Board::start(H::new()).await;
    let pool = json!({"mixed": true, "members": [{"instanceId": "codex", "model": "gpt-6.1", "weight": 3}]});
    let before = json!({"pools": {"gpt-6.1": pool}});
    b.write(&before.to_string());
    let opened = b.settings().await;
    let saved = json!({"revision": "next", "restart": [], "warnings": []});
    let page = b.page().await;
    for typed in ["25e", "9007199254740993"] {
        for (fix, weight) in [(r#"byKey(weight, "5")"#, json!(5)), (r#"enter(weight, "")"#, json!(null))] {
            let scenario = format!(
                r#"await open();
  const key = 'pools."gpt-6.1".members.0.weight';
  showSection("pools");
  byKey(field(key), "{typed}");
  showSection("json");
  const readOnly = locked(field(""));
  const note = $("settings-notes").textContent;
  const file = copyJson(conf.draft);
  file.pools.renamed = file.pools["gpt-6.1"];
  delete file.pools["gpt-6.1"];
  enter(field(""), JSON.stringify(file));
  await saveSettings();
  const blocked = {{ puts: puts().length, problems: confProblems().map((p) => p.field) }};
  showSection("pools");
  const weight = field(key);
  if (!weight) return {{ readOnly, note, blocked }};
  {fix};
  showSection("json");
  const now = JSON.parse(field("").value).pools["gpt-6.1"];
  return {{
    readOnly, note, blocked,
    editable: !locked(field("")),
    cleared: !$("settings-notes").textContent.includes("read-only"),
    weight: now ? now.members[0].weight ?? null : "gone",
  }};"#
            );
            let r = run_dialog(&page, &[&opened], (200, &saved), &scenario);
            let key = r#"pools."gpt-6.1".members.0.weight"#;
            assert_eq!(r["readOnly"], true, "{typed}: {r}");
            assert_eq!(r["blocked"], json!({"puts": 0, "problems": [key]}), "{typed}: {r}");
            let note = r["note"].as_str().unwrap();
            assert!(note.contains("All keys (JSON) is read-only") && note.contains(key), "{typed}: {r}");
            assert_eq!((&r["editable"], &r["cleared"]), (&json!(true), &json!(true)), "{typed}, {fix}: {r}");
            assert_eq!(r["weight"], weight, "{typed}, {fix}: {r}");
        }
    }
}

#[tokio::test]
async fn an_edit_under_an_empty_list_reaches_the_check() {
    let b = Board::start(H::new()).await;
    let pool = json!({"mixed": true, "members": [{"instanceId": "codex", "model": "gpt-6.1"}]});
    let before = json!({"board": {"remote": []}, "pools": {"2": pool}});
    b.write(&before.to_string());
    let opened = b.settings().await;
    assert_eq!(opened["errors"], json!([]), "t3cm accepts the list");
    let saved = json!({"revision": "next", "restart": [], "warnings": []});
    let r = run_dialog(
        &b.page().await,
        &[&opened],
        (200, &saved),
        r#"await open();
  showSection("pools");
  byKey(field("pools.2.members.0.weight"), "4");
  showSection("board");
  byKey(field("board.remote.origin"), "nonsense");
  await saveSettings();
  return { puts: puts() };"#,
    );
    let put = &r["puts"][0];
    let member = json!({"instanceId": "codex", "model": "gpt-6.1", "weight": 4});
    assert_eq!(put["file"]["pools"]["2"]["members"], json!([member]), "members stay a list: {r}");
    assert_eq!(put["file"]["board"]["remote"], json!({"origin": "nonsense"}), "the edit is sent: {r}");
    assert_eq!(b.put(put).await.0, 422, "the check sees it");
    assert_eq!(b.text(), Some(before.to_string()));
}

#[tokio::test]
async fn undo_cannot_reach_a_section_that_was_left() {
    let (_, r) = drive(
        &json!({"board": {"feed": 60}, "link": {"down": 30, "stall": 45}}),
        r#"await open();
  showSection("json");
  const whole = field("");
  const text = whole.value;
  enter(whole, text.replace("60", "61"));
  showSection("links");
  byKey(field("link.stall"), "43");
  undo(whole, text);
  const feed = conf.draft.board.feed;
  const stall = conf.draft.link.stall;
  showSection("board");
  const left = field("board.feed");
  byKey(left, "70");
  $("settings-dlg").dispatch("close");
  return { feed, stall, reachable: left.isConnected };"#,
    )
    .await;
    assert_eq!(r, json!({"feed": 61, "stall": 43, "reachable": false}));
}

#[tokio::test]
async fn picking_the_original_value_back_restores_it() {
    for (before, section, key, other) in [
        (json!({"board": {"runtimeMode": "plan"}}), "launches", "board.runtimeMode", "auto"),
        (json!({"board": {"remote": {"mode": "bogus"}}}), "board", "board.remote.mode", "pairing"),
    ] {
        let scenario = format!(
            r#"await open();
  showSection("{section}");
  const el = field("{key}");
  const was = el.value;
  pick(el, "{other}");
  pick(el, was);
  return {{ draft: conf.draft, dirty: confDirty() }};"#
        );
        let (_, r) = drive(&before, &scenario).await;
        assert_eq!(r, json!({"draft": before, "dirty": false}), "{key}");
    }
}

#[tokio::test]
async fn a_list_where_an_object_belongs_is_left_to_the_cli() {
    for (before, section, key, edit) in [
        (json!({"link": [45, 3, 200]}), "links", "link.stall", "120"),
        (
            json!({"board": {"remote": ["pairing", "https://board.acme.example"]}}),
            "board",
            "board.remote.origin",
            "nonsense",
        ),
    ] {
        let scenario = format!(
            r#"await open();
  showSection("{section}");
  const el = field("{key}");
  if (el) byKey(el, "{edit}");
  await saveSettings();
  return {{ puts: puts(), editable: !!conf.draft, note: $("settings-notes").textContent }};"#
        );
        let (b, r) = drive(&before, &scenario).await;
        assert_eq!(b.settings().await["errors"], json!([]), "t3cm accepts the list");
        assert_eq!((&r["puts"], &r["editable"]), (&json!([]), &json!(false)), "{key}: {r}");
        assert!(r["note"].as_str().unwrap().contains("t3cm config edit"), "{key}: {r}");
    }
    let (_, r) = drive(
        &json!({}),
        r#"await open();
  showSection("json");
  enter(field(""), '{"link": [45, 3, 200]}');
  await saveSettings();
  return { puts: puts(), bad: [...conf.bad.keys()] };"#,
    )
    .await;
    assert_eq!(r, json!({"puts": [], "bad": [""]}), "the file editor cannot make one");
}

#[tokio::test]
async fn a_typed_instance_sets_the_driver_when_it_is_finished() {
    let (_, r) = drive(
        &json!({"pools": {"x": {"members": [{"instanceId": ""}]}}}),
        r#"await open();
  modelsState.data = { providers: [
    { id: "codex-a", driver: "codex", models: [] },
    { id: "codex-a-claude", driver: "claudeAgent", models: [] },
  ] };
  showSection("pools");
  const key = "pools.x.members.0.instanceId";
  pick(field(key), OPT_TYPED);
  const box = typedBox(key);
  byKey(box, "codex-a-claude");
  const while_typing = conf.draft.pools.x.driver ?? null;
  box.dispatch("change");
  return { while_typing, after: conf.draft.pools.x.driver ?? null };"#,
    )
    .await;
    assert_eq!(r, json!({"while_typing": null, "after": "claudeAgent"}));
}

#[tokio::test]
async fn a_number_field_refuses_text_it_would_change() {
    let before = json!({"maxHoldMinutes": 700});
    for (from, data, gives) in [
        ("700", "abc", ""),
        ("700", "9,999", "9999"),
        ("700", "12abc", "12"),
        ("700", "1.2.3", "1.23"),
        ("2", "5-", "25"),
        ("2.5", ".", "2.5"),
        ("2.5", ".7", "2.57"),
        ("700", "\u{2212}5", "5"),
        ("1", ",", "1"),
    ] {
        let scenario = format!(
            r#"await open();
  showSection("runtime");
  const el = field("maxHoldMinutes");
  enter(el, "{from}");
  insert(el, "{data}", "{gives}", "insertFromPaste");
  advance(0);
  if ("{data}" === ",") insert(el, "5", "15");
  await saveSettings();
  return {{ puts: puts(), bad: [...conf.bad.keys()] }};"#
        );
        let (_, r) = drive(&before, &scenario).await;
        assert_eq!(r, json!({"puts": [], "bad": ["maxHoldMinutes"]}), "{data} into {from}");
    }
    for (then, why) in [
        (r#"insert(el, "3", "2.53");"#, "the next key comes before any timer"),
        (r#"el.dispatch("blur");"#, "Save comes before any timer"),
    ] {
        let scenario = format!(
            r#"await open();
  showSection("runtime");
  const el = field("maxHoldMinutes");
  enter(el, "2.5");
  insert(el, ".", "2.5");
  {then}
  await saveSettings();
  return {{ puts: puts(), bad: [...conf.bad.keys()] }};"#
        );
        let (_, r) = drive(&before, &scenario).await;
        assert_eq!(r, json!({"puts": [], "bad": ["maxHoldMinutes"]}), "{why}");
    }
    let (_, r) = drive(
        &before,
        r#"await open();
  showSection("runtime");
  const el = field("maxHoldMinutes");
  insert(el, "x", "700");
  insert(el, "9", "9");
  return { bad: conf.bad.size, value: conf.draft.maxHoldMinutes };"#,
    )
    .await;
    assert_eq!(r, json!({"bad": 0, "value": 9}), "replacing the whole field clears it");
}

#[tokio::test]
async fn undo_back_to_a_blank_number_keeps_its_error() {
    let (_, r) = drive(
        &json!({"maxTimeoutMs": 700000}),
        r#"await open();
  showSection("runtime");
  const el = field("maxTimeoutMs");
  el.validity = { badInput: true };
  enter(el, "");
  showSection("board");
  showSection("runtime");
  const blank = field("maxTimeoutMs");
  enter(blank, "7");
  undo(blank, "");
  await saveSettings();
  return { puts: puts(), bad: [...conf.bad.keys()] };"#,
    )
    .await;
    assert_eq!(r, json!({"puts": [], "bad": ["maxTimeoutMs"]}));
}

#[tokio::test]
async fn editing_one_line_keeps_the_other_entries_as_they_were() {
    let append = r#"enter(el, el.value + "\nt3_example")"#;
    let replace_first = r#"enter(el, ["t3_example", ...el.value.split("\n").slice(1)].join("\n"))"#;
    let cut = |from: usize, to: usize| {
        format!(
            r#"el.setSelectionRange({from}, {to}); insert(el, null, el.value.slice(0, {from}) + el.value.slice({to}), "deleteContentForward")"#
        )
    };
    for (before, edit, want) in [
        (json!({"deny": [{"tool": "x"}]}), append.to_string(), json!([{"tool": "x"}, "t3_example"])),
        (json!({"deny": [" t3_a", "t3_b"]}), r#"enter(el, el.value + "c")"#.to_string(), json!([" t3_a", "t3_bc"])),
        (json!({"deny": [5, "5"]}), append.to_string(), json!([5, "5", "t3_example"])),
        (json!({"deny": ["", "t3_a"]}), append.to_string(), json!(["", "t3_a", "t3_example"])),
        (json!({"deny": ["5", 5]}), append.to_string(), json!(["5", 5, "t3_example"])),
        (json!({"deny": [5, "5"]}), replace_first.to_string(), json!(["t3_example", "5"])),
        (json!({"deny": ["5", 5]}), replace_first.to_string(), json!(["t3_example", 5])),
        (json!({"deny": ["t3_a", "t3_b\nt3_c"]}), replace_first.to_string(), json!(["t3_example", "t3_b\nt3_c"])),
        (json!({"deny": [5, "5"]}), cut(0, 2), json!(["5"])),
        (json!({"deny": ["5", 5]}), cut(0, 2), json!([5])),
        (json!({"deny": [5, "5"]}), cut(1, 3), json!([5])),
        (json!({"deny": ["5", 5]}), cut(1, 3), json!(["5"])),
    ] {
        let scenario = format!(
            r#"await open();
  showSection("upstreams");
  const el = field("deny");
  {edit};
  return conf.draft.deny;"#
        );
        let (_, r) = drive(&before, &scenario).await;
        assert_eq!(r, want, "{before} {edit}");
    }
    let (_, r) = drive(
        &json!({}),
        r#"await open();
  showSection("upstreams");
  const el = field("deny");
  enter(el, "  t3_example  ");
  el.dispatch("change");
  return { shown: field("deny").value, saved: conf.draft.deny };"#,
    )
    .await;
    assert_eq!(r, json!({"shown": "t3_example", "saved": ["t3_example"]}), "the field shows what is saved");
}

#[tokio::test]
async fn a_checkbox_edit_keeps_an_invalid_entry_for_the_check() {
    let before = json!({"wake": ["event", 5]});
    let (b, r) = drive(
        &before,
        r#"await open();
  showSection("links");
  const box = field("wake").querySelectorAll("input").find((el) => el.value === "gate");
  box.checked = true;
  box.dispatch("change");
  await saveSettings();
  return { puts: puts() };"#,
    )
    .await;
    assert_eq!(r["puts"][0]["file"]["wake"], json!(["event", 5, "gate"]), "{r}");
    assert_eq!(b.put(&r["puts"][0]).await.0, 422);
    assert_eq!(b.text(), Some(before.to_string()));
}

#[tokio::test]
async fn an_open_typed_box_shows_what_will_be_saved() {
    let (_, r) = drive(
        &json!({"pools": {"x": {"members": [{"instanceId": ""}]}, "y": {"driver": 5, "members": []}}}),
        r#"await open();
  modelsState.data = { providers: [{ id: "claudeAgent", driver: "claudeAgent", models: [] }] };
  showSection("pools");
  pick(field("pools.y.driver"), OPT_TYPED);
  const prefilled = typedBox("pools.y.driver").value;
  pick(field("pools.x.driver"), OPT_TYPED);
  const box = typedBox("pools.x.driver");
  const member = field("pools.x.members.0.instanceId");
  member.focus();
  pick(member, "claudeAgent");
  return { prefilled, box: box.value, draft: conf.draft.pools.x.driver };"#,
    )
    .await;
    assert_eq!(r, json!({"prefilled": "5", "box": "claudeAgent", "draft": "claudeAgent"}));
}

#[tokio::test]
async fn undo_in_a_closed_typed_box_keeps_the_choice() {
    let (_, r) = drive(
        &json!({"pools": {"x": {"driver": "claudeAgent", "model": "claude-opus-5-5", "members": []}}}),
        r#"await open();
  modelsState.data = { providers: [{ id: "a", driver: "claudeAgent", models: [{ id: "claude-haiku-5-5" }] }] };
  showSection("pools");
  const key = "pools.x.model";
  pick(field(key), OPT_TYPED);
  const box = typedBox(key);
  byKey(box, "half-typed");
  pick(field(key), "claude-haiku-5-5");
  undo(box, "half-type");
  return conf.draft.pools.x.model;"#,
    )
    .await;
    assert_eq!(r, "claude-haiku-5-5");
}

#[tokio::test]
async fn turning_mixed_off_and_on_again_keeps_each_members_model() {
    let before = json!({"pools": {"t": {"mixed": true, "members": [
        {"instanceId": "a", "model": "claude-opus-5-5", "effort": "high"},
        {"instanceId": "b", "model": "claude-haiku-5-5"},
    ]}}});
    let (_, r) = drive(
        &before,
        r#"await open();
  showSection("pools");
  confSetMixed("t", false);
  confSetMixed("t", true);
  const back = copyJson(conf.draft.pools.t.members);
  const models = () => conf.draft.pools.t.members.map((m) => m.model);
  const whole = (change) => {
    const file = copyJson(conf.draft);
    change(file);
    showSection("json");
    enter(field(""), pretty(file));
    showSection("pools");
  };
  confSetMixed("t", false);
  whole((file) => { file.board = { feed: 70 }; });
  confSetMixed("t", true);
  const elsewhere = models();
  confSetMixed("t", false);
  whole((file) => { file.pools.t.members[1].weight = 2; });
  confSetMixed("t", true);
  const replaced = models();
  confSetMixed("t", false);
  confSet(["pools", "t", "model"], "claude-sonnet-5-5");
  confSetMixed("t", true);
  return { back, elsewhere, replaced, edited: models() };"#,
    )
    .await;
    assert_eq!(r["back"], before["pools"]["t"]["members"], "{r}");
    assert_eq!(
        r["elsewhere"],
        json!(["claude-opus-5-5", "claude-haiku-5-5"]),
        "an edit elsewhere keeps the cache: {r}"
    );
    assert_eq!(r["replaced"], json!(["claude-opus-5-5", "claude-opus-5-5"]), "a file edit to the pool drops it: {r}");
    assert_eq!(r["edited"], json!(["claude-sonnet-5-5", "claude-sonnet-5-5"]), "a later pick wins: {r}");
}

#[tokio::test]
async fn a_pool_name_not_yet_added_holds_save() {
    let (_, r) = drive(
        &json!({}),
        r#"await open();
  showSection("pools");
  enter(field("op:pools:name"), "spare");
  const asks = confDirty();
  showSection("board");
  byKey(field("board.feed"), "80");
  await saveSettings();
  const held = { puts: puts().length, tab: conf.tab };
  const said = $("conf-pool-name-err").textContent;
  enter(field("op:pools:name"), "");
  await saveSettings();
  return { asks, held, said, puts: puts().length };"#,
    )
    .await;
    assert_eq!(r["held"], json!({"puts": 0, "tab": "pools"}), "{r}");
    assert_eq!((&r["asks"], &r["puts"]), (&json!(true), &json!(1)), "{r}");
    assert!(r["said"].as_str().unwrap().contains("Add the pool"), "{r}");
}

#[tokio::test]
async fn escape_asks_when_focus_has_left_the_dialog() {
    let (_, r) = drive(
        &json!({}),
        r#"await open();
  $("settings-dlg").open = true;
  showSection("board");
  byKey(field("board.feed"), "80");
  document.activeElement.blur();
  document.activeElement = document.body;
  const esc = { key: "Escape", target: document.body, defaultPrevented: false, preventDefault() { this.defaultPrevented = true; } };
  for (const f of document.listeners.keydown || []) f(esc);
  const asked = { handled: esc.defaultPrevented, asks: !$("settings-confirm").hidden };
  $("settings-keep").dispatch("click");
  return { asked, focusInside: document.activeElement !== document.body };"#,
    )
    .await;
    assert_eq!(r, json!({"asked": {"handled": true, "asks": true}, "focusInside": true}));
}

#[tokio::test]
async fn nothing_is_editable_while_a_save_is_checked() {
    let b = Board::start(H::new()).await;
    let opened = b.settings().await;
    let saved = json!({"revision": "next", "restart": [], "warnings": []});
    let r = run_dialog(
        &b.page().await,
        &[&opened],
        (200, &saved),
        r#"await open();
  type(field("board.feed"), "80");
  const saving = saveSettings();
  const during = conf.panels.get("board").body.hasAttribute("inert");
  await saving;
  return { during, after: conf.panels.get("board").body.hasAttribute("inert") };"#,
    );
    assert_eq!(r, json!({"during": true, "after": false}));
}

#[tokio::test]
async fn saving_one_picker_keeps_the_other_pickers_choice() {
    let b = Board::start(H::new()).await;
    let models = b.get("/api/models").await;
    let prelude = format!(
        r#"globalThis.__fetchReply = (url) => url.startsWith("/api/models")
  ? Promise.resolve({{ ok: true, status: 200, json: () => Promise.resolve({models}) }})
  : new Promise(() => {{}});"#
    );
    let scenario = format!(
        r#"async () => {{
{DIALOG_HELPERS}
  await settle();
  openSettings("ticket");
  setPickerMode("ask", "pool", true);
  await settle();
  const opened = modelsState.ask.mode;
  await saveModelSection("ticket");
  return {{ opened, saved: modelsState.ask.mode }};
}}"#
    );
    let r = run_page_async_with(&b.page().await, &prelude, &scenario);
    assert_eq!(r, json!({"opened": "pool", "saved": "pool"}));
}

#[tokio::test]
async fn a_picker_chosen_before_the_first_snapshot_still_fills() {
    let b = Board::start(H::new()).await;
    let pool = json!({"mixed": true, "members": [{"instanceId": "claudeAgent", "model": "claude-opus-5-5"}]});
    b.saved(json!({"pools": {"opus": pool}})).await;
    let models = b.get("/api/models").await;
    for answered in ["startup", "open"] {
        let read = if answered == "startup" { 1 } else { 2 };
        let prelude = format!(
            r#"let modelReads = 0;
globalThis.__fetchReply = (url) => url === "/api/models" && ++modelReads === {read}
  ? Promise.resolve({{ ok: true, status: 200, json: () => Promise.resolve({models}) }})
  : new Promise(() => {{}});"#
        );
        let scenario = format!(
            r#"async () => {{
{DIALOG_HELPERS}
  openSettings("ticket");
  setPickerMode("ticket", "pool", true);
  await settle();
  return {{ mode: modelsState.ticket.mode, pool: $("tm-pool").value, off: $("tm-save").disabled }};
}}"#
        );
        let r = run_page_async_with(&b.page().await, &prelude, &scenario);
        assert_eq!(r, json!({"mode": "pool", "pool": "opus", "off": false}), "the {answered} read answers first");
    }
}

#[tokio::test]
async fn the_flow_section_edits_and_saves_every_flow_key() {
    let b = Board::start(H::new()).await;
    let opened = b.settings().await;
    let saved = json!({"revision": "next", "restart": [], "warnings": []});
    let r = run_dialog(
        &b.page().await,
        &[&opened],
        (200, &saved),
        r#"await open();
  modelsState.data = {
    projects: [
      { id: "p1", title: "Widgets", remote: { host: "github.com", path: "acme/widgets" } },
      { id: "p2", title: "GitLab", remote: { host: "gitlab.acme.example", path: "acme/internal" } },
    ],
  };
  showSection("flow");
  const initialLaunch = field("flow.launch").value;
  const discoveredNote = conf.flowNote.textContent;
  byKey(field("flow.maxTickets"), "3");
  const launchAfterMax = field("flow.launch").value;
  pick(field("flow.launch"), "auto");
  const ser = field("flow.serializeFileConflicts");
  ser.checked = true;
  ser.dispatch("change");
  enter(field("flow.repos"), "acme/widgets\nacme/tools");
  const explicitNote = conf.flowNote.textContent;
  enter(field("flow.includeRepos"), "acme/extra");
  enter(field("flow.excludeRepos"), "acme/archive");
  enter(field("flow.optIn.label"), "flow");
  enter(field("flow.optIn.milestone"), "v1.0");
  await saveSettings();
  return { initialLaunch, discoveredNote, launchAfterMax, explicitNote, puts: puts() };"#,
    );
    assert_eq!(r["initialLaunch"], "off", "launch defaults to off while maxTickets is unset: {r}");
    assert!(r["discoveredNote"].as_str().unwrap().contains("acme/widgets"), "{r}");
    assert!(!r["discoveredNote"].as_str().unwrap().contains("acme/internal"), "non-GitHub remotes ignored: {r}");
    assert_eq!(r["launchAfterMax"], "ask", "setting maxTickets switches default launch display to ask: {r}");
    assert!(r["explicitNote"].as_str().unwrap().contains("explicit repository list"), "{r}");

    let puts = r["puts"].as_array().unwrap();
    assert_eq!(puts.len(), 1, "{r}");
    let want = json!({
        "flow": {
            "maxTickets": 3,
            "launch": "auto",
            "serializeFileConflicts": true,
            "repos": ["acme/widgets", "acme/tools"],
            "includeRepos": ["acme/extra"],
            "excludeRepos": ["acme/archive"],
            "optIn": {"label": "flow", "milestone": "v1.0"},
        }
    });
    assert_eq!(puts[0], json!({"revision": opened["revision"], "file": want}));
    let (status, reply) = b.put(&puts[0]).await;
    assert_eq!(status, 200, "{reply}");
    assert_eq!(reply["restart"], json!([]), "flow settings reload with no restart");
    assert_eq!(Document::read(&b.file()).unwrap().value().unwrap(), want);
    let live = b.h.app.config().flow.clone().expect("flow config reloaded live");
    assert_eq!(live.max_tickets, Some(3));
    assert_eq!(live.launch.as_str(), "auto");
    assert!(live.serialize_file_conflicts);
    assert_eq!(live.repos, Some(vec!["acme/widgets".into(), "acme/tools".into()]));
    assert_eq!(live.include_repos, vec!["acme/extra"]);
    assert_eq!(live.exclude_repos, vec!["acme/archive"]);
    let opt = live.opt_in.expect("optIn loaded");
    assert_eq!(opt.label.as_deref(), Some("flow"));
    assert_eq!(opt.milestone.as_deref(), Some("v1.0"));
}

#[tokio::test]
async fn the_flow_section_clears_optional_keys_and_surfaces_validation_errors() {
    // 1. Clearing flow.repos and flow.optIn.label on an existing config produces a valid save.
    let before = json!({
        "flow": {
            "repos": ["acme/widgets"],
            "maxTickets": 2,
            "optIn": {"label": "flow", "milestone": null},
        }
    });
    let (b, r) = drive(
        &before,
        r#"await open();
  showSection("flow");
  enter(field("flow.repos"), "");
  enter(field("flow.optIn.label"), "");
  await saveSettings();
  return { puts: puts() };"#,
    )
    .await;
    let put = &r["puts"][0];
    assert_eq!(put["file"], json!({"flow": {"maxTickets": 2}}), "clearing repos and optIn prunes invalid empties");
    assert_eq!(b.put(put).await.0, 200);
    let live = b.h.app.config().flow.clone().unwrap();
    assert_eq!(live.repos, None);
    assert!(live.opt_in.is_none());

    // 1b. Clearing the last flow.optIn field when flow.optIn existed in conf.base prunes empty draft.flow.
    let before_opt_only = json!({
        "flow": {
            "optIn": {"label": "flow"},
        }
    });
    let (b2, r2) = drive(
        &before_opt_only,
        r#"await open();
  showSection("flow");
  enter(field("flow.optIn.label"), "");
  await saveSettings();
  return { puts: puts(), draft: conf.draft };"#,
    )
    .await;
    let put2 = &r2["puts"][0];
    assert_eq!(put2["file"], json!({}), "clearing last flow.optIn field must prune empty flow object: {r2}");
    assert_eq!(b2.put(put2).await.0, 200);
    assert!(b2.h.app.config().flow.is_none());

    // 1c. Typing and clearing optIn.label when conf.base already had flow: {} keeps draft clean.
    let (_, clean_empty_flow) = drive(
        &json!({"flow": {}}),
        r#"await open();
  showSection("flow");
  enter(field("flow.optIn.label"), "ready");
  enter(field("flow.optIn.label"), "");
  return { draft: conf.draft, dirty: confDirty() };"#,
    )
    .await;
    assert_eq!(clean_empty_flow, json!({"draft": {"flow": {}}, "dirty": false}));

    // 2. Typing and clearing optIn.label from an empty config leaves draft clean (not dirty).
    let (_, clean) = drive(
        &json!({}),
        r#"await open();
  showSection("flow");
  enter(field("flow.optIn.label"), "ready");
  enter(field("flow.optIn.label"), "");
  return { draft: conf.draft, dirty: confDirty() };"#,
    )
    .await;
    assert_eq!(clean, json!({"draft": {}, "dirty": false}));

    // 3. Server validation errors route back to the Flow tab and the exact field.
    let b = Board::start(H::new()).await;
    let opened = b.settings().await;
    let (status, refused) = b.save(json!({"flow": {"launch": "ask"}})).await;
    assert_eq!(status, 422);
    let r = run_dialog(
        &b.page().await,
        &[&opened],
        (status, &refused),
        r#"await open();
  showSection("flow");
  pick(field("flow.launch"), "ask");
  showSection("board");
  await saveSettings();
  const max = field("flow.maxTickets");
  const err = max.parentNode.parentNode.querySelector(".conf-err");
  return { tab: conf.tab, focused: document.activeElement === max, invalid: max.getAttribute("aria-invalid"), err: err.hidden ? "" : err.textContent };"#,
    );
    assert_eq!((&r["tab"], &r["focused"], &r["invalid"]), (&json!("flow"), &json!(true), &json!("true")), "{r}");
    assert!(r["err"].as_str().unwrap().contains("maxTickets must be at least 1"), "{r}");

    let (status, refused) = b.save(json!({"flow": {"repos": ["not-a-repo"]}})).await;
    assert_eq!(status, 422);
    let r = run_dialog(
        &b.page().await,
        &[&opened],
        (status, &refused),
        r#"await open();
  showSection("flow");
  enter(field("flow.repos"), "not-a-repo");
  await saveSettings();
  const repos = field("flow.repos");
  const err = repos.parentNode.parentNode.querySelector(".conf-err");
  return { focused: document.activeElement === repos, invalid: repos.getAttribute("aria-invalid"), err: err.hidden ? "" : err.textContent };"#,
    );
    assert_eq!((&r["focused"], &r["invalid"]), (&json!(true), &json!("true")), "{r}");
    assert!(r["err"].as_str().unwrap().contains("owner/name"), "{r}");

    // 4. Non-empty list where flow or flow.optIn object belongs is left to the CLI.
    for before in [json!({"flow": ["acme/widgets"]}), json!({"flow": {"optIn": ["flow"]}})] {
        let (_, r) = drive(
            &before,
            r#"await open();
  await saveSettings();
  return { puts: puts(), editable: !!conf.draft, note: $("settings-notes").textContent };"#,
        )
        .await;
        assert_eq!((&r["puts"], &r["editable"]), (&json!([]), &json!(false)), "{before}: {r}");
        assert!(r["note"].as_str().unwrap().contains("t3cm config edit"), "{before}: {r}");
    }
}
