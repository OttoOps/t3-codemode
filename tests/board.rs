mod common;

use std::time::Duration;

use common::*;
use serde_json::{Value, json};
use t3_codemode::board::{BOARD, card_text, parse_issue, parse_remote, reply_options};
use t3_codemode::config::Config;
use t3_codemode::fake::{project, thread};
use t3_codemode::server;

const GATE: &str = "PR #81 is green and reviewed.\n\
**ACTION NEEDED:** reply \"merge\" to merge #81, or reply \"merge and deploy\" to ship it too, here: https://github.com/o/widgets/pull/81";

async fn join_board(h: &H, who: &str, label: &str) {
    h.wf(who, "join", json!({"parent": BOARD, "label": label, "role": "ticket"})).await;
}

fn cards(h: &H) -> Vec<Value> {
    h.app.wf.cards().unwrap().into_iter().map(|c| serde_json::to_value(c).unwrap()).collect()
}

fn sent_to(h: &H, id: &str) -> Vec<Value> {
    h.sent().into_iter().filter(|s| s["threadId"] == id).collect()
}

#[test]
fn parsing_issues_remotes_and_options() {
    let i = parse_issue("https://github.com/Acme/widgets/issues/41#issuecomment-1").unwrap();
    assert_eq!((i.owner.as_str(), i.repo.as_str(), i.number), ("Acme", "widgets", 41));
    assert_eq!(i.label(), "widgets#41");
    assert_eq!(parse_issue("Acme/widgets#7").unwrap().number, 7);
    assert_eq!(parse_issue("github.com/o/r/issues/3").unwrap().repo, "r");
    assert!(parse_issue("https://gitlab.com/o/r/issues/3").is_none());
    assert!(parse_issue("o/r#0").is_none());
    for url in [
        "git@github.com:Acme/WIDGETS.git",
        "https://github.com/acme/widgets",
        "https://x-access-token:abc@github.com/Acme/widgets.git",
        "ssh://git@github.com:22/Acme/widgets.git",
    ] {
        let (o, r) = parse_remote(url).unwrap_or_else(|| panic!("{url}"));
        assert!(o.eq_ignore_ascii_case("acme") && r.eq_ignore_ascii_case("widgets"), "{url}");
    }
    assert!(parse_remote("git@gitlab.com:o/r.git").is_none());
    assert_eq!(reply_options(GATE), ["merge", "merge and deploy"]);
    assert_eq!(reply_options("Reply 'yes' or reply \u{201c}no\u{201d}; prereply \"x\""), ["yes", "no"]);
    assert!(card_text(GATE).starts_with("reply \"merge\" to merge #81"));
    assert_eq!(card_text("\n  first line\nsecond"), "first line");
}

#[tokio::test]
async fn a_gate_to_the_board_becomes_a_card_with_its_reply_buttons() {
    let h = H::new();
    join_board(&h, TICKET, "widgets#41").await;
    let r = h.wf(TICKET, "send", json!({"kind": "gate", "gate": "merge", "body": GATE})).await;
    assert_eq!(r["delivery"], "board");
    assert_eq!(r["card"], "a1");
    let c = cards(&h);
    assert_eq!(c.len(), 1);
    assert_eq!(c[0]["id"], "a1");
    assert_eq!(c[0]["kind"], "gate");
    assert_eq!(c[0]["label"], "widgets#41");
    assert_eq!(c[0]["ticket"], TICKET);
    assert_eq!(c[0]["link"], "https://t3/mcp:ticket");
    assert_eq!(c[0]["handle"], r["handle"]);
    assert_eq!(c[0]["options"], json!(["merge", "merge and deploy"]));
    assert!(c[0]["text"].as_str().unwrap().starts_with("reply \"merge\""));
    assert!(h.sent().is_empty(), "nothing goes to T3: {:?}", h.sent());
}

#[tokio::test]
async fn blockers_and_reports_become_cards_and_events_only_feed() {
    let h = H::new();
    join_board(&h, TICKET, "widgets#41").await;
    h.wf(TICKET, "send", json!({"kind": "blocker", "body": "Need an AWS SSO login"})).await;
    h.wf(TICKET, "send", json!({"kind": "report", "body": "Worker 2 died twice"})).await;
    let r = h.wf(TICKET, "send", json!({"kind": "event", "body": "PR 1/2 merged", "wake": true})).await;
    assert_eq!(r["delivery"], "noted");
    h.wf(TICKET, "send", json!({"kind": "note", "body": "starting PR 2/2"})).await;
    let kinds: Vec<Value> = cards(&h).iter().map(|c| c["kind"].clone()).collect();
    assert_eq!(kinds, [json!("blocker"), json!("report")]);
    let feed = h.wf(TICKET, "feed", json!({})).await;
    let heads: Vec<&str> = feed.as_array().unwrap().iter().map(|f| f["head"].as_str().unwrap()).collect();
    assert_eq!(heads, ["starting PR 2/2", "PR 1/2 merged"]);
    assert_eq!(feed[0]["label"], "widgets#41");
    assert!(h.sent().is_empty());
    assert!(h.app.store.outbox_all().unwrap().is_empty());
}

#[tokio::test]
async fn a_handoff_resolves_the_tickets_cards_and_marks_it_done() {
    let h = H::new();
    join_board(&h, TICKET, "widgets#41").await;
    join_board(&h, WORKER, "widgets#42").await;
    h.wf(TICKET, "send", json!({"kind": "gate", "body": GATE})).await;
    h.wf(TICKET, "send", json!({"kind": "blocker", "body": "stuck"})).await;
    h.wf(WORKER, "send", json!({"kind": "blocker", "body": "other ticket"})).await;
    let r = h.wf(TICKET, "send", json!({"kind": "handoff", "body": "merged and deployed"})).await;
    assert_eq!(r["closed"], true);
    let left = cards(&h);
    assert_eq!(left.len(), 1);
    assert_eq!(left[0]["label"], "widgets#42");
    let tree = h.app.wf.tree().unwrap();
    let t = tree.iter().find(|t| t["label"] == "widgets#41").unwrap();
    assert_eq!((t["done"].clone(), t["status"].clone()), (json!(true), json!("done")));
    assert!(h.sent().is_empty());
}

#[tokio::test]
async fn an_ask_can_see_finished_tickets_and_resolved_cards() {
    let h = H::new();
    join_board(&h, TICKET, "widgets#41").await;
    h.wf(TICKET, "send", json!({"kind": "gate", "body": GATE})).await;
    h.wf(TICKET, "send", json!({"kind": "handoff", "body": "merged and deployed"})).await;
    let open = result(
        &h.exec(
            "async () => ({ b: (await wf.board({ parent: 'board' })).children.length, c: (await wf.cards()).length })",
        )
        .await,
    );
    assert_eq!(open, json!({"b": 0, "c": 0}));
    let all = result(
        &h.exec(
            "async () => ({ b: (await wf.board({ parent: 'board', all: true })).children.map(c => c.label), \
             c: (await wf.cards({ all: true })).map(c => [c.id, c.kind, typeof c.resolved, c.resolution]) })",
        )
        .await,
    );
    assert_eq!(all["b"], json!(["widgets#41"]));
    assert_eq!(all["c"][0][0], "a1");
    assert_eq!(all["c"][0][1], "gate");
    assert_eq!(all["c"][0][2], "string");
    assert!(all["c"][0][3].is_string());
    assert!(t3_codemode::board::ask_brief("q").contains("all: true"));
}

#[tokio::test]
async fn clearing_the_gate_in_the_ticket_resolves_its_gate_card() {
    let h = H::new();
    join_board(&h, TICKET, "widgets#41").await;
    h.wf(TICKET, "send", json!({"kind": "gate", "body": GATE})).await;
    h.wf(TICKET, "send", json!({"kind": "blocker", "body": "also this"})).await;
    h.wf(TICKET, "gate", json!({"clear": true})).await;
    let left = cards(&h);
    assert_eq!(left.len(), 1);
    assert_eq!(left[0]["kind"], "blocker");
}

#[tokio::test]
async fn a_newer_gate_supersedes_the_old_gate_card() {
    let h = H::new();
    join_board(&h, TICKET, "widgets#41").await;
    h.wf(TICKET, "send", json!({"kind": "gate", "body": "reply \"go\" to split"})).await;
    let r = h.wf(TICKET, "send", json!({"kind": "gate", "body": GATE})).await;
    assert_eq!(r["card"], "a2");
    let left = cards(&h);
    assert_eq!(left.len(), 1);
    assert_eq!(left[0]["id"], "a2");
}

#[tokio::test]
async fn execute_can_list_answer_and_see_the_whole_board() {
    let h = H::new();
    join_board(&h, TICKET, "widgets#41").await;
    h.wf(TICKET, "openChild", json!({"role": "worker", "label": "#41 PR 1/2"})).await;
    h.wf(WORKER, "join", json!({"parent": TICKET, "label": "#41 PR 1/2", "role": "worker"})).await;
    h.wf(TICKET, "send", json!({"kind": "gate", "body": GATE})).await;
    let out = h.exec("async () => (await wf.cards()).map(c => c.id + ':' + c.options.join('|'))").await;
    assert_eq!(result(&out), json!(["a1:merge|merge and deploy"]));
    let b = result(&h.exec("async () => wf.board({ parent: 'board' })").await);
    assert_eq!(b["children"][0]["label"], "widgets#41");
    assert_eq!(b["children"][0]["workers"][0]["label"], "#41 PR 1/2");
    assert_eq!(b["cards"], 1);
    let r = result(&h.exec("async () => wf.answer('a1', 'merge')").await);
    assert_eq!(r["delivery"], "sent");
    assert_eq!(sent_to(&h, TICKET).len(), 1);
    assert!(cards(&h).is_empty());
}

#[tokio::test]
async fn launch_ticket_maps_the_repo_to_a_project_and_preregisters_the_link() {
    let h = H::new();
    h.fake.set_projects(vec![project("p-docs", "/src/docs"), project("p-widgets", "/src/widgets")]);
    h.remotes.set("/src/docs", "git@github.com:Acme/docs.git");
    h.remotes.set("/src/widgets", "https://github.com/acme/WIDGETS.git");
    let out = h
        .exec("async () => wf.launchTicket({ issue: 'https://github.com/Acme/widgets/issues/41', title: 'Fix the grader' })")
        .await;
    let r = result(&out);
    assert_eq!(r["ok"], true);
    assert_eq!(r["label"], "widgets#41");
    assert_eq!(r["projectId"], "p-widgets");
    let l = &h.fake.launched()[0];
    assert_eq!(l["projectId"], "p-widgets");
    assert_eq!(l["title"], "widgets#41: Fix the grader");
    assert!(l.get("workspaceStrategy").is_none());
    assert_eq!(l["modelSelection"]["model"], "claude-opus-5-5");
    assert_eq!(l["modelSelection"]["options"], json!([{"id": "effort", "value": "high"}]));
    let msg = l["message"].as_str().unwrap();
    assert!(msg.starts_with("Work https://github.com/Acme/widgets/issues/41 with the t3-ticket skill."), "{msg}");
    assert!(msg.contains(r#"wf.join({ parent: "board", label: "widgets#41", role: "ticket" })"#), "{msg}");
    let pending = h.app.store.pending_all().unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].parent, BOARD);
    assert_eq!(pending[0].thread.as_deref(), Some("launch:1"));
    let tree = h.app.wf.tree().unwrap();
    assert_eq!(tree[0]["joined"], false);
    assert_eq!(tree[0]["status"], "running");
    let again = h.exec("async () => wf.launchTicket({ issue: 'Acme/widgets#41' })").await;
    assert!(err_text(&again).contains("has not joined yet"), "{}", again.text);
    h.wf("launch:1", "join", json!({"parent": BOARD, "label": "widgets#41", "role": "ticket"})).await;
    assert!(h.app.store.pending_all().unwrap().is_empty());
    assert_eq!(h.app.store.link_get("launch:1").unwrap().unwrap().params.down, 75);
}

#[tokio::test]
async fn an_unmapped_repo_leaves_an_error_card() {
    let h = H::new();
    h.fake.set_projects(vec![project("p-docs", "/src/docs")]);
    h.remotes.set("/src/docs", "git@github.com:Acme/docs.git");
    let out = h.exec("async () => wf.launchTicket({ issue: 'Acme/widgets#41' })").await;
    assert!(err_text(&out).contains("no T3 project has github.com/Acme/widgets"), "{}", out.text);
    let c = cards(&h);
    assert_eq!(c.len(), 1);
    assert_eq!(c[0]["kind"], "error");
    assert_eq!(c[0]["ticket"], Value::Null);
    assert!(h.fake.launched().is_empty());
    assert!(h.app.store.pending_all().unwrap().is_empty());
}

#[tokio::test]
async fn a_never_joined_board_ticket_becomes_a_card() {
    let h = H::new();
    h.fake.set_projects(vec![project("p-widgets", "/src/widgets")]);
    h.remotes.set("/src/widgets", "git@github.com:o/widgets.git");
    h.app.wf.launch_ticket(serde_json::from_value(json!({"issue": "o/widgets#9"})).unwrap()).await.unwrap();
    assert!(h.tick(70).await.is_empty());
    assert!(h.tick(76).await.contains(&"report never-joined widgets#9".to_string()));
    let c = cards(&h);
    assert_eq!(c.len(), 1);
    assert_eq!(c[0]["kind"], "report");
    assert_eq!(c[0]["ticket"], "launch:1");
    assert_eq!(c[0]["link"], "https://t3/launch:1");
    assert!(c[0]["text"].as_str().unwrap().contains("never joined"));
    assert!(sent_to(&h, BOARD).is_empty());
}

#[tokio::test]
async fn watcher_escalations_about_board_children_are_cards_never_sends() {
    let h = H::new();
    join_board(&h, TICKET, "widgets#41").await;
    assert_eq!(h.tick(75).await, vec!["nudge widgets#41 (1/2)"]);
    assert_eq!(h.tick(150).await, vec!["nudge widgets#41 (2/2)"]);
    let out = h.tick(225).await;
    assert!(out.iter().any(|a| a.starts_with("escalate widgets#41")), "{out:?}");
    let c = cards(&h);
    assert_eq!(c.len(), 1);
    assert_eq!(c[0]["kind"], "escalate");
    assert_eq!(c[0]["ticket"], TICKET);
    h.fake.update(TICKET, json!({"status": "waiting", "pendingRequestCount": 1, "latestRunId": "run:2"}));
    h.tick(400).await;
    assert_eq!(cards(&h).len(), 2);
    assert!(sent_to(&h, BOARD).is_empty());
    assert!(h.sent().iter().all(|s| s["threadId"] == TICKET));
}

#[tokio::test]
async fn an_ask_launches_a_scratch_thread_and_captures_its_answer() {
    let h = H::new();
    let a = h.app.wf.ask("Which tickets wait on me?").await.unwrap();
    assert_eq!(a["id"], "q1");
    let l = &h.fake.launched()[0];
    assert_eq!(l["scratch"], true);
    assert!(l.get("projectId").is_none());
    assert_eq!(l["modelSelection"]["model"], "claude-haiku-5-5");
    let msg = l["message"].as_str().unwrap();
    assert!(msg.contains("Which tickets wait on me?") && msg.contains("mcp__t3cm__execute"), "{msg}");
    assert!(h.app.wf.check_asks(None).await.unwrap(), "still running");
    h.fake.update("launch:1", json!({"status": "completed"}));
    h.fake.set_items(
        "launch:1",
        vec![
            json!({"type": "user_message", "position": 0, "text": msg}),
            json!({"type": "assistant_message", "position": 3, "text": "widgets#41 waits on a merge: a1."}),
        ],
    );
    assert!(!h.app.wf.check_asks(None).await.unwrap());
    let asks = h.app.wf.asks().unwrap();
    assert_eq!(asks[0]["status"], "answered");
    assert_eq!(asks[0]["answer"], "widgets#41 waits on a merge: a1.");
    assert_eq!(asks[0]["archived"], true);
    assert_eq!(h.fake.organized(), vec![json!({"threadId": "launch:1", "action": "archive"})]);
    assert!(h.sent().is_empty());
}

struct Srv {
    base: String,
    http: reqwest::Client,
    h: H,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
}

impl Drop for Srv {
    fn drop(&mut self) {
        if let Some(s) = self.stop.take() {
            let _ = s.send(());
        }
    }
}

async fn start(h: H) -> Srv {
    let (listener, addr) = server::bind(0).await.unwrap();
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let app = h.app.clone();
    tokio::spawn(async move {
        server::serve(app, listener, async move {
            let _ = rx.await;
        })
        .await
        .unwrap();
    });
    Srv {
        base: format!("http://{addr}"),
        http: reqwest::Client::builder().no_proxy().build().unwrap(),
        h,
        stop: Some(tx),
    }
}

impl Srv {
    async fn post(&self, path: &str, body: Value, csrf: Option<&str>, origin: Option<&str>) -> reqwest::Response {
        let mut req = self.http.post(format!("{}{path}", self.base)).json(&body);
        if let Some(c) = csrf {
            req = req.header("X-T3CM-CSRF", c);
        }
        if let Some(o) = origin {
            req = req.header("Origin", o);
        }
        req.send().await.unwrap()
    }

    fn csrf(&self) -> String {
        self.h.app.csrf.clone()
    }
}

#[tokio::test]
async fn the_answer_endpoint_sends_one_reply_and_resolves_the_card() {
    let s = start(H::new()).await;
    join_board(&s.h, TICKET, "widgets#41").await;
    s.h.wf(TICKET, "send", json!({"kind": "gate", "body": GATE})).await;
    let r = s.post("/api/cards/a1/answer", json!({"text": "merge and deploy"}), Some(&s.csrf()), Some(&s.base)).await;
    assert_eq!(r.status(), 200);
    let sent = sent_to(&s.h, TICKET);
    assert_eq!(sent.len(), 1);
    let msg = sent[0]["message"].as_str().unwrap();
    assert!(msg.starts_with("[t3cm reply from=board msg="), "{msg}");
    assert!(msg.contains("The user says \"merge and deploy\" about a1: reply \"merge\" to merge #81"), "{msg}");
    assert!(cards(&s.h).is_empty());
    let r = s.post("/api/cards/a1/answer", json!({"text": "again"}), Some(&s.csrf()), None).await;
    assert_eq!(r.status(), 404);
    assert_eq!(sent_to(&s.h, TICKET).len(), 1);
}

#[tokio::test]
async fn posts_need_the_csrf_token_and_the_boards_own_origin() {
    let s = start(H::new()).await;
    join_board(&s.h, TICKET, "widgets#41").await;
    s.h.wf(TICKET, "send", json!({"kind": "gate", "body": GATE})).await;
    let body = json!({"text": "merge"});
    assert_eq!(s.post("/api/cards/a1/answer", body.clone(), None, None).await.status(), 403);
    assert_eq!(s.post("/api/cards/a1/answer", body.clone(), Some("nope"), None).await.status(), 403);
    let foreign = s.post("/api/cards/a1/answer", body.clone(), Some(&s.csrf()), Some("https://evil.example")).await;
    assert_eq!(foreign.status(), 403);
    let other_port = s.post("/api/cards/a1/answer", body.clone(), Some(&s.csrf()), Some("http://127.0.0.1:1")).await;
    assert_eq!(other_port.status(), 403);
    assert_eq!(s.post("/api/launch", json!({"issues": ["o/r#1"]}), None, None).await.status(), 403);
    assert_eq!(s.post("/api/ask", json!({"question": "x"}), Some("nope"), None).await.status(), 403);
    let r = s.http.get(format!("{}/api/state", s.base)).header("Origin", "https://evil.example").send().await.unwrap();
    assert_eq!(r.status(), 403);
    let r = s.http.get(format!("{}/api/state", s.base)).header("Host", "evil.example:80").send().await.unwrap();
    assert_eq!(r.status(), 403);
    assert!(s.h.sent().is_empty());
    assert_eq!(cards(&s.h).len(), 1);
}

#[tokio::test]
async fn dismiss_resolves_without_messaging() {
    let s = start(H::new()).await;
    join_board(&s.h, TICKET, "widgets#41").await;
    s.h.wf(TICKET, "send", json!({"kind": "blocker", "body": "stuck"})).await;
    let r = s.post("/api/cards/a1/dismiss", json!({}), Some(&s.csrf()), Some(&s.base)).await;
    assert_eq!(r.status(), 200);
    assert!(cards(&s.h).is_empty());
    assert!(s.h.sent().is_empty());
}

#[tokio::test]
async fn state_launch_and_message_endpoints() {
    let s = start(H::new()).await;
    s.h.fake.set_projects(vec![project("p-widgets", "/src/widgets")]);
    s.h.remotes.set("/src/widgets", "git@github.com:o/widgets.git");
    let r =
        s.post("/api/launch", json!({"issues": ["o/widgets#1", "o/other#2", "nonsense"]}), Some(&s.csrf()), None).await;
    assert_eq!(r.status(), 200);
    let res: Value = r.json().await.unwrap();
    let ok: Vec<Value> = res["results"].as_array().unwrap().iter().map(|x| x["ok"].clone()).collect();
    assert_eq!(ok, [json!(true), json!(false), json!(false)]);
    assert_eq!(res["results"][1]["card"], "a1");
    join_board(&s.h, TICKET, "widgets#41").await;
    let sent = s.h.wf(TICKET, "send", json!({"kind": "gate", "body": GATE})).await;
    s.h.fake.put(thread("launch:1", "running", T0));
    let st: Value = s.http.get(format!("{}/api/state", s.base)).send().await.unwrap().json().await.unwrap();
    assert_eq!(st["counts"]["cards"], 2);
    assert_eq!(st["cards"][1]["options"], json!(["merge", "merge and deploy"]));
    let labels: Vec<Value> = st["tree"].as_array().unwrap().iter().map(|t| t["label"].clone()).collect();
    assert_eq!(labels, [json!("widgets#41"), json!("widgets#1")]);
    assert_eq!(st["service"]["t3"]["connected"], true);
    let handle = sent["handle"].as_str().unwrap();
    let m: Value = s.http.get(format!("{}/api/msg/{handle}", s.base)).send().await.unwrap().json().await.unwrap();
    assert_eq!(m["body"], GATE);
}

#[tokio::test]
async fn the_event_stream_pushes_state_when_a_card_appears() {
    let s = start(H::new()).await;
    join_board(&s.h, TICKET, "widgets#41").await;
    let mut r = s.http.get(format!("{}/api/events", s.base)).send().await.unwrap();
    assert_eq!(r.status(), 200);
    assert!(r.headers()["content-type"].to_str().unwrap().starts_with("text/event-stream"));
    let first = tokio::time::timeout(Duration::from_secs(5), r.chunk()).await.unwrap().unwrap().unwrap();
    let first = String::from_utf8_lossy(&first).to_string();
    assert!(first.starts_with("event: state\ndata: {"), "{first}");
    assert!(first.contains("\"cards\":[]"), "{first}");
    s.h.wf(TICKET, "send", json!({"kind": "gate", "body": GATE})).await;
    let mut seen = String::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while !seen.contains("\"id\":\"a1\"") {
        let chunk =
            tokio::time::timeout_at(deadline, r.chunk()).await.expect("a state event in time").unwrap().unwrap();
        seen.push_str(&String::from_utf8_lossy(&chunk));
    }
    assert!(seen.contains("event: state"), "{seen}");
    assert!(seen.contains("merge and deploy"), "{seen}");
}

#[tokio::test]
async fn the_page_embeds_the_token_and_loads_nothing_external() {
    let s = start(H::new()).await;
    let r = s.http.get(format!("{}/", s.base)).send().await.unwrap();
    assert_eq!(r.status(), 200);
    assert!(r.headers()["content-type"].to_str().unwrap().starts_with("text/html"));
    assert!(r.headers()["content-security-policy"].to_str().unwrap().contains("default-src 'none'"));
    let html = r.text().await.unwrap();
    assert!(html.contains(&format!("const CSRF = \"{}\";", s.csrf())));
    assert!(!html.contains("{{CSRF}}"));
    let lower = html.to_lowercase();
    for tag in ["<script", "<link", "<img", "<iframe", "@import", "url("] {
        for (i, _) in lower.match_indices(tag) {
            let end = lower[i..].find('>').map(|e| i + e).unwrap_or(lower.len());
            let t = &lower[i..end];
            assert!(!t.contains("http://") && !t.contains("https://") && !t.contains("src="), "external load: {t}");
        }
    }
    assert!(!html.contains('\u{2014}'));
}

fn settle_calls(h: &H, action: &str) -> Vec<Value> {
    h.fake.organized().into_iter().filter(|o| o["action"] == action).collect()
}

async fn done_ticket(h: &H, who: &str, label: &str) {
    join_board(h, who, label).await;
    h.wf(who, "send", json!({"kind": "handoff", "body": "merged and deployed"})).await;
}

async fn closed_ask(h: &H, question: &str) -> String {
    let a = h.app.wf.ask(question).await.unwrap();
    let thread = a["thread"].as_str().unwrap().to_string();
    h.fake.update(&thread, json!({"status": "completed"}));
    h.fake.set_items(&thread, vec![json!({"type": "assistant_message", "position": 1, "text": "all quiet"})]);
    h.app.wf.check_asks(None).await.unwrap();
    a["id"].as_str().unwrap().to_string()
}

fn snapshot(h: &H) -> Value {
    h.app.wf.snapshot().unwrap()
}

fn labels(v: &Value) -> Vec<Value> {
    v.as_array().unwrap().iter().map(|t| t["label"].clone()).collect()
}

#[tokio::test]
async fn done_tickets_and_closed_asks_settle_once_the_threshold_passes() {
    let h = H::new();
    done_ticket(&h, TICKET, "widgets#41").await;
    join_board(&h, WORKER, "widgets#42").await;
    let ask = closed_ask(&h, "What is waiting?").await;
    h.clock.advance_minutes(24 * 60 - 1);
    h.app.wf.board_tick().await.unwrap();
    let st = snapshot(&h);
    assert_eq!(labels(&st["tree"]), [json!("widgets#42"), json!("widgets#41")]);
    assert_eq!(st["asks"][0]["id"], ask.as_str());
    assert_eq!(st["counts"]["settled"], 0);
    assert!(settle_calls(&h, "settle").is_empty());
    h.clock.advance_minutes(1);
    h.app.wf.board_tick().await.unwrap();
    let st = snapshot(&h);
    assert_eq!(labels(&st["tree"]), [json!("widgets#42")]);
    assert_eq!(st["asks"], json!([]));
    assert_eq!(labels(&st["settled"]["tickets"]), [json!("widgets#41")]);
    assert_eq!(st["settled"]["tickets"][0]["done"], true);
    assert!(st["settled"]["tickets"][0]["settledAt"].as_str().unwrap().contains(':'));
    assert_eq!(st["settled"]["asks"][0]["id"], ask.as_str());
    assert!(st["settled"]["asks"][0]["settledAt"].is_string());
    assert_eq!(st["counts"]["settled"], 2);
    assert_eq!(settle_calls(&h, "settle"), vec![json!({"threadId": TICKET, "action": "settle"})]);
    h.app.wf.board_tick().await.unwrap();
    assert_eq!(settle_calls(&h, "settle").len(), 1);
    let board = h.wf(BOSS, "board", json!({"parent": BOARD, "all": true})).await;
    assert_eq!(board["children"][0]["settled"], true);
    assert_eq!(board["children"][1]["settled"], false);
}

#[tokio::test]
async fn a_failed_ask_settles_from_when_it_was_asked() {
    let h = H::new();
    let a = h.app.wf.ask("Will this fail?").await.unwrap();
    h.fake.update(a["thread"].as_str().unwrap(), json!({"status": "failed"}));
    h.app.wf.check_asks(None).await.unwrap();
    assert!(h.app.wf.asks().unwrap()[0]["answeredAt"].is_null());
    h.clock.advance_minutes(24 * 60 + 1);
    h.app.wf.board_tick().await.unwrap();
    assert_eq!(snapshot(&h)["settled"]["asks"][0]["status"], "failed");
}

#[tokio::test]
async fn zero_settle_after_hours_turns_auto_settle_off() {
    let config = Config::from_value(json!({"board": {"settleAfterHours": 0}})).unwrap();
    let h = H::build(Vec::new(), config);
    done_ticket(&h, TICKET, "widgets#41").await;
    closed_ask(&h, "What is waiting?").await;
    h.clock.advance_minutes(30 * 24 * 60);
    h.app.wf.board_tick().await.unwrap();
    let st = snapshot(&h);
    assert_eq!(labels(&st["tree"]), [json!("widgets#41")]);
    assert_eq!(st["asks"].as_array().unwrap().len(), 1);
    assert_eq!(st["counts"]["settled"], 0);
    assert!(settle_calls(&h, "settle").is_empty());
}

#[tokio::test]
async fn a_restored_ticket_is_not_settled_again_at_once() {
    let h = H::new();
    done_ticket(&h, TICKET, "widgets#41").await;
    let ask = closed_ask(&h, "What is waiting?").await;
    h.clock.advance_minutes(25 * 60);
    h.app.wf.board_tick().await.unwrap();
    assert_eq!(snapshot(&h)["counts"]["settled"], 2);
    h.app.wf.restore_ticket(TICKET).await.unwrap();
    h.app.wf.restore_ask(&ask).unwrap();
    h.clock.advance_minutes(60);
    h.app.wf.board_tick().await.unwrap();
    let st = snapshot(&h);
    assert_eq!(labels(&st["tree"]), [json!("widgets#41")]);
    assert_eq!(st["asks"][0]["id"], ask.as_str());
    assert_eq!(st["counts"]["settled"], 0);
    h.clock.advance_minutes(24 * 60);
    h.app.wf.board_tick().await.unwrap();
    assert_eq!(snapshot(&h)["counts"]["settled"], 2);
}

#[tokio::test]
async fn settle_and_restore_work_through_the_routes_and_call_t3() {
    let s = start(H::new()).await;
    done_ticket(&s.h, TICKET, "widgets#41").await;
    let ask = closed_ask(&s.h, "What is waiting?").await;
    let ticket_settle = format!("/api/tickets/{TICKET}/settle");
    assert_eq!(s.post(&ticket_settle, json!({}), None, None).await.status(), 403);
    assert_eq!(s.post(&ticket_settle, json!({}), Some("nope"), None).await.status(), 403);
    let ask_settle = format!("/api/asks/{ask}/settle");
    assert_eq!(s.post(&ask_settle, json!({}), None, None).await.status(), 403);
    assert_eq!(s.post("/api/settle-done", json!({}), None, None).await.status(), 403);
    assert_eq!(snapshot(&s.h)["counts"]["settled"], 0);
    assert!(settle_calls(&s.h, "settle").is_empty());

    let r = s.post(&ticket_settle, json!({}), Some(&s.csrf()), Some(&s.base)).await;
    assert_eq!(r.status(), 200);
    let body: Value = r.json().await.unwrap();
    assert_eq!((body["settled"].clone(), body["t3"]["ok"].clone()), (json!(true), json!(true)));
    assert_eq!(s.post(&ask_settle, json!({}), Some(&s.csrf()), None).await.status(), 200);
    let st = snapshot(&s.h);
    assert_eq!(st["tree"], json!([]));
    assert_eq!(st["asks"], json!([]));
    assert_eq!(st["settled"]["tickets"][0]["thread"], TICKET);
    assert_eq!(st["settled"]["asks"][0]["id"], ask.as_str());
    assert_eq!(st["counts"]["settled"], 2);
    assert_eq!(settle_calls(&s.h, "settle"), vec![json!({"threadId": TICKET, "action": "settle"})]);
    assert_eq!(s.post(&ticket_settle, json!({}), Some(&s.csrf()), None).await.status(), 400);

    let ticket_restore = format!("/api/tickets/{TICKET}/restore");
    let ask_restore = format!("/api/asks/{ask}/restore");
    assert_eq!(s.post(&ticket_restore, json!({}), None, None).await.status(), 403);
    assert_eq!(s.post(&ticket_restore, json!({}), Some(&s.csrf()), None).await.status(), 200);
    assert_eq!(s.post(&ask_restore, json!({}), Some(&s.csrf()), None).await.status(), 200);
    let st = snapshot(&s.h);
    assert_eq!(labels(&st["tree"]), [json!("widgets#41")]);
    assert_eq!(st["asks"][0]["id"], ask.as_str());
    assert_eq!(st["counts"]["settled"], 0);
    assert_eq!(settle_calls(&s.h, "unsettle"), vec![json!({"threadId": TICKET, "action": "unsettle"})]);
    assert_eq!(s.post(&ticket_restore, json!({}), Some(&s.csrf()), None).await.status(), 400);
    assert_eq!(s.post(&ask_restore, json!({}), Some(&s.csrf()), None).await.status(), 400);
    let missing = s.post("/api/tickets/nope/settle", json!({}), Some(&s.csrf()), None).await;
    assert_eq!(missing.status(), 404);
    assert_eq!(s.post("/api/asks/q99/settle", json!({}), Some(&s.csrf()), None).await.status(), 404);
    assert_eq!(s.post("/api/asks/bogus/settle", json!({}), Some(&s.csrf()), None).await.status(), 404);
}

#[tokio::test]
async fn open_tickets_and_open_asks_cannot_be_settled() {
    let s = start(H::new()).await;
    join_board(&s.h, TICKET, "widgets#41").await;
    let a = s.h.app.wf.ask("Still working?").await.unwrap();
    let r = s.post(&format!("/api/tickets/{TICKET}/settle"), json!({}), Some(&s.csrf()), None).await;
    assert_eq!(r.status(), 400);
    let err: Value = r.json().await.unwrap();
    assert!(err["error"].as_str().unwrap().contains("only done tickets"), "{err}");
    let r = s.post(&format!("/api/asks/{}/settle", a["id"].as_str().unwrap()), json!({}), Some(&s.csrf()), None).await;
    assert_eq!(r.status(), 400);
    let err: Value = r.json().await.unwrap();
    assert!(err["error"].as_str().unwrap().contains("only closed asks"), "{err}");
    let st = snapshot(&s.h);
    assert_eq!(labels(&st["tree"]), [json!("widgets#41")]);
    assert_eq!(st["counts"]["settled"], 0);
    assert!(s.h.fake.organized().is_empty());
}

#[tokio::test]
async fn a_t3_error_does_not_fail_settle_or_restore() {
    let s = start(H::new()).await;
    done_ticket(&s.h, TICKET, "widgets#41").await;
    s.h.fake.set_organize_fails(true);
    let r = s.post(&format!("/api/tickets/{TICKET}/settle"), json!({}), Some(&s.csrf()), None).await;
    assert_eq!(r.status(), 200);
    let body: Value = r.json().await.unwrap();
    assert_eq!(body["settled"], true);
    assert_eq!(body["t3"]["ok"], false);
    assert!(body["t3"]["error"].as_str().unwrap().contains("unreachable"), "{body}");
    assert_eq!(snapshot(&s.h)["counts"]["settled"], 1);
    let r = s.post(&format!("/api/tickets/{TICKET}/restore"), json!({}), Some(&s.csrf()), None).await;
    assert_eq!(r.status(), 200);
    assert_eq!(r.json::<Value>().await.unwrap()["t3"]["ok"], false);
    assert_eq!(snapshot(&s.h)["counts"]["settled"], 0);
}

#[tokio::test]
async fn settle_done_settles_every_finished_item_and_returns_counts() {
    let s = start(H::new()).await;
    done_ticket(&s.h, TICKET, "widgets#41").await;
    join_board(&s.h, WORKER, "widgets#42").await;
    s.h.fake.put(thread("mcp:third", "idle", T0));
    done_ticket(&s.h, "mcp:third", "widgets#43").await;
    closed_ask(&s.h, "First?").await;
    s.h.app.wf.ask("Second?").await.unwrap();
    assert_eq!(s.post("/api/settle-done", json!({}), None, None).await.status(), 403);
    let r = s.post("/api/settle-done", json!({}), Some(&s.csrf()), None).await;
    assert_eq!(r.status(), 200);
    assert_eq!(r.json::<Value>().await.unwrap(), json!({"tickets": 2, "asks": 1}));
    let st = snapshot(&s.h);
    assert_eq!(labels(&st["tree"]), [json!("widgets#42")]);
    assert_eq!(st["asks"].as_array().unwrap().len(), 1);
    assert_eq!(st["asks"][0]["status"], "running");
    assert_eq!(st["counts"]["settled"], 3);
    assert_eq!(settle_calls(&s.h, "settle").len(), 2);
    let again = s.post("/api/settle-done", json!({}), Some(&s.csrf()), None).await;
    assert_eq!(again.json::<Value>().await.unwrap(), json!({"tickets": 0, "asks": 0}));
}

#[tokio::test]
async fn settled_items_are_newest_first_and_capped() {
    let h = H::new();
    for n in 0..205 {
        let id = format!("mcp:t{n}");
        h.fake.put(thread(&id, "idle", T0));
        done_ticket(&h, &id, &format!("widgets#{n}")).await;
        h.clock.advance_minutes(1);
        h.app.wf.settle_ticket(&id).await.unwrap();
    }
    for n in 0..205 {
        closed_ask(&h, &format!("Question {n}?")).await;
        h.clock.advance_minutes(1);
        h.app.wf.settle_ask(&format!("q{}", n + 1)).unwrap();
    }
    let st = snapshot(&h);
    assert_eq!(st["settled"]["tickets"].as_array().unwrap().len(), 200);
    assert_eq!(st["settled"]["asks"].as_array().unwrap().len(), 200);
    assert_eq!(st["settled"]["tickets"][0]["label"], "widgets#204");
    assert_eq!(st["settled"]["asks"][0]["id"], "q205");
    assert_eq!(st["counts"]["settled"], 410);
    assert_eq!(st["asks"], json!([]));
}

#[tokio::test]
async fn links_stored_before_settling_still_load() {
    let h = H::new();
    join_board(&h, TICKET, "widgets#1").await;
    let ln = h.app.store.link_get(TICKET).unwrap().unwrap();
    let mut raw = serde_json::to_value(&ln).unwrap();
    let keys = raw.as_object_mut().unwrap();
    assert!(keys.remove("settledAt").is_some() && keys.remove("restoredAt").is_some());
    let old: t3_codemode::store::Link = serde_json::from_value(raw).unwrap();
    assert_eq!(old, ln);
    assert_eq!((old.settled_at, old.restored_at), (None, None));
}

fn run_page(html: &str, scenario: &str) -> Value {
    let open = html.find("<script>").unwrap() + "<script>".len();
    let close = html.rfind("</script>").unwrap();
    let src =
        format!("{}\n{}\nJSON.stringify(({scenario})())", include_str!("support/board_dom.js"), &html[open..close]);
    let rt = rquickjs::Runtime::new().unwrap();
    let ctx = rquickjs::Context::full(&rt).unwrap();
    ctx.with(|ctx| {
        use rquickjs::CatchResultExt;
        let out: String = ctx.eval(src).catch(&ctx).unwrap_or_else(|e| panic!("{e}"));
        serde_json::from_str(&out).unwrap()
    })
}

#[tokio::test]
async fn typing_in_a_card_reply_survives_state_events() {
    let s = start(H::new()).await;
    join_board(&s.h, TICKET, "widgets#41").await;
    join_board(&s.h, WORKER, "widgets#42").await;
    s.h.wf(TICKET, "send", json!({"kind": "gate", "gate": "merge", "body": GATE})).await;
    s.h.wf(WORKER, "send", json!({"kind": "gate", "gate": "merge", "body": GATE})).await;
    let html = s.http.get(format!("{}/", s.base)).send().await.unwrap().text().await.unwrap();
    let st: Value = s.http.get(format!("{}/api/state", s.base)).send().await.unwrap().json().await.unwrap();
    let ids: Vec<Value> = st["cards"].as_array().unwrap().iter().map(|c| c["id"].clone()).collect();
    assert_eq!(ids, [json!("a1"), json!("a2")]);
    let scenario = format!(
        r#"() => {{
  const base = {st};
  const input = (id) => document.querySelector('#cards [data-id="' + id + '"] input');
  const changed = (id, text) => ({{ ...base, cards: base.cards.map((c) => c.id === id ? {{ ...c, text }} : c) }});
  emitState(base);
  const first = input("a1");
  first.focus();
  typeKeys("hello");
  first.setSelectionRange(2, 2);
  emitState(base);
  typeKeys("X");
  emitState(changed("a2", "the other card changed"));
  typeKeys("Y");
  emitState({{ ...base, cards: [{{ ...base.cards[1], id: "a9" }}, ...base.cards] }});
  typeKeys("Z");
  const kept = input("a1") === first;
  emitState(changed("a1", "this card changed"));
  typeKeys("W");
  const rebuilt = input("a1");
  rebuilt.setSelectionRange(1, 4, "backward");
  emitState(changed("a1", "this card changed again"));
  const last = input("a1");
  return {{
    kept,
    rebuilt: rebuilt !== first && last !== rebuilt,
    value: last.value,
    focused: document.activeElement === last,
    selection: [last.selectionStart, last.selectionEnd, last.selectionDirection],
    order: [...document.getElementById("cards").children].map((c) => c.dataset.id),
  }};
}}"#
    );
    let r = run_page(&html, &scenario);
    assert_eq!(r["kept"], true, "{r}");
    assert_eq!(r["rebuilt"], true, "{r}");
    assert_eq!(r["value"], "heXYZWllo", "{r}");
    assert_eq!(r["focused"], true, "{r}");
    assert_eq!(r["selection"], json!([1, 4, "backward"]), "{r}");
    assert_eq!(r["order"], json!(["a1", "a2"]), "{r}");
}

const THREAD_LINK: &str = "https://app.t3.codes/threads/env-1/mcp%3Alaunch%3Aabc123";
const APP_LINK: &str = "t3code://threads/env-1/mcp%3Alaunch%3Aabc123";

#[test]
fn only_hosted_thread_links_map_to_the_desktop_scheme() {
    use t3_codemode::desktop::app_link;
    assert_eq!(app_link(THREAD_LINK).as_deref(), Some(APP_LINK));
    for bad in [
        "https://github.com/o/widgets/pull/81",
        "http://app.t3.codes/threads/env-1/t1",
        "https://app.t3.codes/threads/env-1",
        "https://app.t3.codes/threads/env-1/t1/",
        "https://app.t3.codes/threads/env-1/t1/extra",
        "https://app.t3.codes/threads//t1",
        "https://app.t3.codes/threads/env-1/t1?x=1",
        "https://app.t3.codes/threads/env-1/t\"1",
        "https://app.t3.codes.evil/threads/env-1/t1",
    ] {
        assert_eq!(app_link(bad), None, "{bad}");
    }
}

#[tokio::test]
async fn a_thread_link_opens_in_the_running_desktop_app() {
    let s = start(H::new()).await;
    s.h.desktop.running.store(true, std::sync::atomic::Ordering::SeqCst);
    let r = s.post("/api/open", json!({"link": THREAD_LINK}), Some(&s.csrf()), Some(&s.base)).await;
    assert_eq!(r.status(), 200);
    assert_eq!(r.json::<Value>().await.unwrap(), json!({"opened": true}));
    assert_eq!(*s.h.desktop.opened.lock().unwrap(), [APP_LINK]);
}

#[tokio::test]
async fn with_the_app_closed_the_page_is_told_to_open_a_tab() {
    let s = start(H::new()).await;
    let r = s.post("/api/open", json!({"link": THREAD_LINK}), Some(&s.csrf()), None).await;
    assert_eq!(r.json::<Value>().await.unwrap(), json!({"opened": false}));
    assert!(s.h.desktop.opened.lock().unwrap().is_empty());
}

#[tokio::test]
async fn the_tab_setting_never_asks_the_desktop_app() {
    let config = Config::from_value(json!({"board": {"openLinks": "tab"}})).unwrap();
    let s = start(H::build(Vec::new(), config)).await;
    s.h.desktop.running.store(true, std::sync::atomic::Ordering::SeqCst);
    let st: Value = s.http.get(format!("{}/api/state", s.base)).send().await.unwrap().json().await.unwrap();
    assert_eq!(st["service"]["openLinks"], "tab");
    let r = s.post("/api/open", json!({"link": THREAD_LINK}), Some(&s.csrf()), None).await;
    assert_eq!(r.json::<Value>().await.unwrap(), json!({"opened": false}));
    assert!(s.h.desktop.opened.lock().unwrap().is_empty());
}

#[tokio::test]
async fn the_open_route_refuses_other_links_and_missing_csrf() {
    let s = start(H::new()).await;
    s.h.desktop.running.store(true, std::sync::atomic::Ordering::SeqCst);
    let st: Value = s.http.get(format!("{}/api/state", s.base)).send().await.unwrap().json().await.unwrap();
    assert_eq!(st["service"]["openLinks"], "desktop");
    let pr = s.post("/api/open", json!({"link": "https://github.com/o/widgets/pull/81"}), Some(&s.csrf()), None).await;
    assert_eq!(pr.status(), 400);
    assert_eq!(s.post("/api/open", json!({"link": THREAD_LINK}), None, None).await.status(), 403);
    let foreign =
        s.post("/api/open", json!({"link": THREAD_LINK}), Some(&s.csrf()), Some("https://evil.example")).await;
    assert_eq!(foreign.status(), 403);
    assert!(s.h.desktop.opened.lock().unwrap().is_empty());
}

#[test]
fn open_links_accepts_only_desktop_or_tab() {
    assert!(Config::from_value(json!({"board": {"openLinks": "window"}})).is_err());
    assert_eq!(Config::default().board.open_links, t3_codemode::config::OpenLinks::Desktop);
}
