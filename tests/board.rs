mod common;

use std::time::Duration;

use common::*;
use serde_json::{Value, json};
use t3_codemode::board::{BOARD, card_text, parse_issue, parse_remote, reply_options};
use t3_codemode::config::Config;
use t3_codemode::fake::{project, pull_request, thread};
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

#[test]
fn every_quoted_answer_on_an_action_line_is_an_option() {
    let corpus: &[(&str, &[&str])] = &[
        (
            "**ACTION NEEDED:** reply how to track the finding (...): \"advisory\" for a private \
             security advisory, \"issue\" for a public issue, or \"nothing\", here: https://github.com/acme/widgets/pull/37",
            &["advisory", "issue", "nothing"],
        ),
        (
            "**ACTION NEEDED:** reply \"file follow-ups\" to open issues for the 3 items deferred from #39 (...), or \"no follow-ups\", here: acme/widgets#39",
            &["file follow-ups", "no follow-ups"],
        ),
        (
            "**ACTION NEEDED:** reply \"merge\" to merge #81, or review it first, here: https://github.com/acme/widgets/pull/81",
            &["merge"],
        ),
        (
            "**ACTION NEEDED:** reply \"merge and deploy\" to merge #81 and push content/v1.4.2, or \"merge\" alone, \
             or review it first, here: https://github.com/acme/widgets/pull/81",
            &["merge and deploy", "merge"],
        ),
        (
            "**ACTION NEEDED:** reply \"deploy\" to push content/v1.4.2, or review it first, here: https://github.com/acme/widgets/pull/81",
            &["deploy"],
        ),
        (
            "**ACTION NEEDED:** reopen #81, then reply \"merge\" to merge it, here: https://github.com/acme/widgets/pull/81",
            &["merge"],
        ),
        (
            "**ACTION NEEDED:** reply \"advisory\" to open a private security advisory, reply \"issue\" to file a \
             public issue, or reply \"nothing\" to leave it, here: https://github.com/acme/widgets/pull/37",
            &["advisory", "issue", "nothing"],
        ),
        ("**ACTION NEEDED:** answer \"A\", \"B\", or \"C\"", &["A", "B", "C"]),
        ("**ACTION NEEDED:** Say \u{201c}ship it\u{201d} or \u{201c}hold\u{201d}", &["ship it", "hold"]),
        (
            "**ACTION NEEDED:** reply 'keep' or \u{2018}drop\u{2019} for the user's draft, here: acme/widgets#5",
            &["keep", "drop"],
        ),
        ("**ACTION NEEDED:** reply 'the user's call' or 'skip'", &["the user's call", "skip"]),
        ("**ACTION NEEDED:** don't reply 'yet', it's \u{2018}pending\u{2019}", &["yet", "pending"]),
        ("**ACTION NEEDED:** in \"draft\" mode, reply \"go\"", &["go"]),
        ("**ACTION NEEDED:** replying \"x\" or essay \"y\" does nothing", &[]),
        ("**ACTION NEEDED:** change \"no reply\" to \"pending\", then reply \"done\" or \"skip\"", &["done", "skip"]),
        ("**ACTION NEEDED:** set reply_mode \"on\" or auto_reply \"x\", then answer \"ok\"", &["ok"]),
        ("**ACTION NEEDED:** start the code-mode service, here: `t3cm serve`", &[]),
        ("**ACTION NEEDED:** reply \"\", \"   \", or \"Merge\" or \"merge\"", &["Merge"]),
    ];
    for (line, want) in corpus {
        assert_eq!(reply_options(line), *want, "{line}");
    }
}

#[test]
fn quoted_text_off_the_action_line_makes_no_button() {
    let body = "Earlier the user said \"ship it\", and the reviewer asked us to reply \"later\".\n\
        **ACTION NEEDED:** reply \"merge\" to merge #81, here: https://github.com/acme/widgets/pull/81\n\
        Notes: \"draft\" mode was off.";
    assert_eq!(reply_options(body), ["merge"]);
    let none = "We reply \"merge\" on green.\n**ACTION NEEDED:** review #81 first, here: acme/widgets#81";
    assert!(reply_options(none).is_empty());
    let two = "**ACTION NEEDED:** reply \"merge\" to merge #81\n**ACTION NEEDED:** answer \"retry\" or \"skip\" for the flaky check";
    assert_eq!(reply_options(two), ["merge", "retry", "skip"]);
    let many: String = (1..=12).map(|n| format!(" \"o{n}\"")).collect();
    assert_eq!(reply_options(&format!("**ACTION NEEDED:** reply{many}")).len(), 8);
    let long = format!("**ACTION NEEDED:** reply \"{}\" or \"ok\"", "x".repeat(61));
    assert_eq!(reply_options(&long), ["ok"]);
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
    assert_eq!(tree[0]["repo"], "Acme/widgets");
    assert_eq!(tree[0]["project"], json!({"id": "p-widgets", "title": "p-widgets"}));
    assert_eq!(tree[0]["title"], "widgets#41: Fix the grader");
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
async fn card_actions_for_an_older_decision_with_the_same_id_do_nothing() {
    let s = start(H::new()).await;
    join_board(&s.h, TICKET, "widgets#41").await;
    s.h.wf(TICKET, "send", json!({"kind": "gate", "body": GATE})).await;
    let old = cards(&s.h)[0].clone();
    assert_eq!(old["id"], "a1");
    let want = |c: &Value| json!({"at": c["at"], "token": c["token"]});
    let r = s.post("/api/cards/a1/dismiss", want(&old), Some(&s.csrf()), None).await;
    assert_eq!(r.status(), 200);
    for batch in [2..=19, 20..=36] {
        let issues: Vec<String> = batch.map(|n| format!("o/nowhere#{n}")).collect();
        assert_eq!(s.post("/api/launch", json!({"issues": issues}), Some(&s.csrf()), None).await.status(), 200);
    }
    assert_eq!(cards(&s.h).len(), 35);
    s.h.wf(TICKET, "send", json!({"kind": "gate", "body": GATE})).await;
    let new = cards(&s.h).into_iter().find(|c| c["id"] == "a1").unwrap();
    assert_eq!(new["at"], old["at"]);
    assert_ne!(new["token"], old["token"]);
    let mut stale = want(&old);
    stale["text"] = json!("merge");
    let r = s.post("/api/cards/a1/answer", stale, Some(&s.csrf()), None).await;
    assert_eq!(r.status(), 404);
    let r = s.post("/api/cards/a1/dismiss", want(&old), Some(&s.csrf()), None).await;
    assert_eq!(r.status(), 404);
    assert!(sent_to(&s.h, TICKET).is_empty());
    assert!(cards(&s.h).iter().any(|c| c["id"] == "a1"));
    let mut fresh = want(&new);
    fresh["text"] = json!("merge");
    let r = s.post("/api/cards/a1/answer", fresh, Some(&s.csrf()), None).await;
    assert_eq!(r.status(), 200);
    assert_eq!(sent_to(&s.h, TICKET).len(), 1);
}

#[tokio::test]
async fn a_reply_in_flight_resolves_only_the_card_it_answered() {
    let s = start(H::new()).await;
    join_board(&s.h, TICKET, "widgets#41").await;
    s.h.wf(TICKET, "send", json!({"kind": "gate", "body": GATE})).await;
    let old = cards(&s.h)[0].clone();
    s.h.fake.set_delay(Duration::from_millis(400));
    let body = json!({"text": "merge", "at": old["at"], "token": old["token"]});
    let csrf = s.csrf();
    let reply = s.post("/api/cards/a1/answer", body, Some(&csrf), None);
    let reuse = async {
        while s.h.app.store.outbox_all().unwrap().is_empty() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        s.h.app.store.waiting_resolve(BOARD, "a1", Some("resolved elsewhere"), "2026-11-17T12:00:00Z").unwrap();
        let item = t3_codemode::store::WaitingItem {
            id: "a1".into(),
            text: "a newer gate".into(),
            from: None,
            link: None,
            at: old["at"].as_str().unwrap().into(),
        };
        s.h.app.store.waiting_insert(BOARD, &item, Some(&json!({"kind": "gate", "ticket": TICKET}))).unwrap();
    };
    let (r, ()) = tokio::join!(reply, reuse);
    s.h.fake.set_delay(Duration::ZERO);
    assert_eq!(r.status(), 200);
    let open = cards(&s.h);
    assert_eq!(open.len(), 1);
    assert_eq!(open[0]["id"], "a1");
    assert_eq!(open[0]["text"], "a newer gate");
    assert_ne!(open[0]["token"], old["token"]);
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
async fn a_ticket_settled_or_archived_in_t3_settles_on_the_board() {
    let h = H::new();
    done_ticket(&h, TICKET, "widgets#41").await;
    done_ticket(&h, WORKER, "widgets#42").await;
    done_ticket(&h, BOSS, "widgets#43").await;
    h.app.wf.board_tick().await.unwrap();
    assert_eq!(snapshot(&h)["counts"]["settled"], 0);
    h.fake.update(TICKET, json!({"settled": true, "settledAt": "2026-11-17T12:05:00.123Z"}));
    h.fake.update(WORKER, json!({"archived": true, "updatedAt": "2026-11-17T12:05:00.000Z"}));
    h.clock.advance_minutes(10);
    h.app.wf.board_tick().await.unwrap();
    let st = snapshot(&h);
    assert_eq!(labels(&st["tree"]), [json!("widgets#43")]);
    let mut settled = labels(&st["settled"]["tickets"]);
    settled.sort_by_key(|v| v.to_string());
    assert_eq!(settled, [json!("widgets#41"), json!("widgets#42")]);
    assert!(settle_calls(&h, "settle").is_empty());
}

#[tokio::test]
async fn a_ticket_whose_thread_is_gone_settles_on_the_board() {
    let h = H::new();
    done_ticket(&h, TICKET, "widgets#41").await;
    h.fake.remove(TICKET);
    h.app.wf.board_tick().await.unwrap();
    assert_eq!(labels(&snapshot(&h)["settled"]["tickets"]), [json!("widgets#41")]);
    assert!(settle_calls(&h, "settle").is_empty());
}

#[tokio::test]
async fn a_restored_ticket_ignores_a_t3_settle_older_than_the_restore() {
    let h = H::new();
    done_ticket(&h, TICKET, "widgets#41").await;
    h.app.wf.settle_ticket(TICKET).await.unwrap();
    h.clock.advance_minutes(10);
    h.fake.set_organize_fails(true);
    h.app.wf.restore_ticket(TICKET).await.unwrap();
    h.fake.update(TICKET, json!({"settled": true, "settledAt": "2026-11-17T12:00:00.000Z"}));
    h.app.wf.board_tick().await.unwrap();
    assert_eq!(labels(&snapshot(&h)["tree"]), [json!("widgets#41")]);
    h.clock.advance_minutes(10);
    h.fake.update(TICKET, json!({"settledAt": "2026-11-17T12:15:00.000Z"}));
    h.app.wf.board_tick().await.unwrap();
    assert_eq!(labels(&snapshot(&h)["settled"]["tickets"]), [json!("widgets#41")]);
}

#[tokio::test]
async fn a_restored_ticket_stays_restored_while_its_thread_is_archived() {
    let h = H::new();
    done_ticket(&h, TICKET, "widgets#41").await;
    h.fake.update(TICKET, json!({"archived": true}));
    h.app.wf.board_tick().await.unwrap();
    h.clock.advance_minutes(10);
    h.app.wf.restore_ticket(TICKET).await.unwrap();
    h.fake.update(TICKET, json!({"updatedAt": "2026-11-17T12:30:00.000Z"}));
    h.clock.advance_minutes(30);
    h.app.wf.board_tick().await.unwrap();
    assert_eq!(labels(&snapshot(&h)["tree"]), [json!("widgets#41")]);
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
    same: rebuilt === first && last === first,
    value: last.value,
    focused: document.activeElement === last,
    selection: [last.selectionStart, last.selectionEnd, last.selectionDirection],
    order: [...document.getElementById("cards").children].map((c) => c.dataset.id),
  }};
}}"#
    );
    let r = run_page(&html, &scenario);
    assert_eq!(r["kept"], true, "{r}");
    assert_eq!(r["same"], true, "{r}");
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

async fn two_gate_page() -> (String, Value) {
    let s = start(H::new()).await;
    join_board(&s.h, TICKET, "widgets#41").await;
    join_board(&s.h, WORKER, "widgets#42").await;
    s.h.wf(TICKET, "send", json!({"kind": "gate", "gate": "merge", "body": GATE})).await;
    s.h.wf(WORKER, "send", json!({"kind": "gate", "gate": "merge", "body": GATE})).await;
    let html = s.http.get(format!("{}/", s.base)).send().await.unwrap().text().await.unwrap();
    let st: Value = s.http.get(format!("{}/api/state", s.base)).send().await.unwrap().json().await.unwrap();
    (html, st)
}

const PAGE_HELPERS: &str = r##"
const q = (sel) => document.querySelector(sel);
const card = (id) => q('#cards [data-key="' + id + '"]');
const ids = () => [...document.getElementById("cards").children].map((c) => c.dataset.key);
const answers = () => fetches.filter((f) => /\/(answer|dismiss)$/.test(f.url));
"##;

fn run_page_async(html: &str, scenario: &str) -> Value {
    let open = html.find("<script>").unwrap() + "<script>".len();
    let close = html.rfind("</script>").unwrap();
    let src = format!(
        "{}\n{}\nPromise.resolve(({scenario})()).then((v) => {{ globalThis.__out = JSON.stringify(v); }}, (e) => {{ globalThis.__out = JSON.stringify({{ error: String(e) }}); }});",
        include_str!("support/board_dom.js"),
        &html[open..close]
    );
    let rt = rquickjs::Runtime::new().unwrap();
    let ctx = rquickjs::Context::full(&rt).unwrap();
    ctx.with(|ctx| {
        use rquickjs::CatchResultExt;
        ctx.eval::<(), _>(src).catch(&ctx).unwrap_or_else(|e| panic!("{e}"));
    });
    while rt.is_job_pending() {
        rt.execute_pending_job().unwrap();
    }
    ctx.with(|ctx| {
        let out: String = ctx.eval("globalThis.__out").unwrap();
        serde_json::from_str(&out).unwrap()
    })
}

fn page_scenario(html: &str, st: &Value, body: &str) -> Value {
    run_page(html, &format!("() => {{\n{PAGE_HELPERS}\nconst base = {st};\n{body}\n}}"))
}

#[tokio::test]
async fn the_page_is_a_fixed_shell_with_panels_tabs_and_skip_links() {
    let (html, _) = two_gate_page().await;
    for needle in [
        "height: 100dvh",
        "html, body { height: 100%; margin: 0; overflow: hidden; }",
        r#"role="tablist" aria-label="Ticket lists""#,
        r#"id="tab-open""#,
        r#"id="tab-done""#,
        r#"id="tab-settled""#,
        r#"id="new-cards""#,
        r#"id="new-tix""#,
        r#"id="new-feed""#,
        r#"aria-live="polite""#,
        r#"role="alert""#,
        "Skip to Decisions",
        "Skip to Launch",
        "Skip to Ask",
        "Skip to Tickets",
        "Skip to Activity",
        r#"aria-label="Launch and Ask""#,
    ] {
        assert!(html.contains(needle), "missing {needle}");
    }
    assert!(html.find("Skip to Ask").unwrap() < html.find("Skip to Tickets").unwrap());
    for banned in ["gradient", "btn primary", "backdrop-filter", "requestPermission();\n}, { once: true }"] {
        assert!(!html.contains(banned), "found {banned}");
    }
    for line in html.lines().filter(|l| l.contains("box-shadow")) {
        assert!(line.contains("box-shadow: none"), "{line}");
    }
}

#[tokio::test]
async fn new_cards_wait_behind_the_count_while_the_reader_is_in_the_panel() {
    let (html, st) = two_gate_page().await;
    let r = page_scenario(
        &html,
        &st,
        r##"
emitState(base);
const input = card("a1").querySelector("input");
input.focus();
const blocker = { ...base.cards[1], id: "a9", kind: "blocker" };
emitState({ ...base, cards: [...base.cards, blocker] });
const held = { order: ids(), slot: q("#new-cards").textContent, shown: !q("#new-cards").classList.contains("off"), focused: document.activeElement === input };
q("#new-cards").dispatch("click");
return { held, after: ids(), focus: document.activeElement.dataset.key, slotAfter: q("#new-cards").classList.contains("off") };
"##,
    );
    assert_eq!(
        r["held"],
        json!({"order": ["a1", "a2"], "slot": "1 new - 1 blocker", "shown": true, "focused": true}),
        "{r}"
    );
    assert_eq!(r["after"], json!(["a9", "a1", "a2"]), "{r}");
    assert_eq!(r["focus"], "a9", "{r}");
    assert_eq!(r["slotAfter"], true, "{r}");
}

#[tokio::test]
async fn a_removed_card_keeps_a_placeholder_until_the_panel_is_free() {
    let (html, st) = two_gate_page().await;
    let r = page_scenario(
        &html,
        &st,
        r##"
emitState(base);
card("a2").querySelector("input").focus();
emitState({ ...base, cards: [base.cards[1]] });
const a1 = card("a1");
const held = { gone: a1.classList.contains("gone"), buttons: a1.querySelectorAll("button").length, text: a1.textContent, order: ids() };
document.activeElement = document.body;
emitState({ ...base, cards: [base.cards[1]] });
return { held, after: ids() };
"##,
    );
    assert_eq!(r["held"], json!({"gone": true, "buttons": 0, "text": "a1 resolved", "order": ["a1", "a2"]}), "{r}");
    assert_eq!(r["after"], json!(["a2"]), "{r}");
}

#[tokio::test]
async fn a_reply_leaves_after_five_seconds_and_undo_sends_nothing() {
    let (html, st) = two_gate_page().await;
    let r = page_scenario(
        &html,
        &st,
        r##"
emitState(base);
card("a1").querySelector('[data-opt="1"]').dispatch("click");
const what = card("a1").querySelector('[data-part="what"]').textContent;
const at0 = answers().length;
advance(4999);
const at4999 = answers().length;
advance(1);
const sent = answers().map((f) => [f.url, JSON.parse(f.body)]);
card("a2").querySelector('[data-opt="2"]').dispatch("click");
advance(1000);
card("a2").querySelector('[data-part="undo"]').dispatch("click");
advance(10000);
return { what, at0, at4999, sent, after: answers().length, back: !card("a2").querySelector('[data-part="act"]').hidden };
"##,
    );
    assert_eq!(r["what"], "Sending \u{201c}merge\u{201d} to widgets#41", "{r}");
    assert_eq!((r["at0"].as_i64(), r["at4999"].as_i64()), (Some(0), Some(0)), "{r}");
    assert_eq!(
        r["sent"],
        json!([["/api/cards/a1/answer", {"text": "merge", "at": st["cards"][0]["at"], "token": st["cards"][0]["token"]}]]),
        "{r}"
    );
    assert_eq!(r["after"], 1, "{r}");
    assert_eq!(r["back"], true, "{r}");
}

#[tokio::test]
async fn a_reply_to_a_card_resolved_in_the_undo_window_sends_nothing() {
    let (html, st) = two_gate_page().await;
    let r = page_scenario(
        &html,
        &st,
        r##"
emitState(base);
card("a1").querySelector('[data-opt="1"]').dispatch("click");
emitState({ ...base, cards: base.cards.filter((c) => c.id !== "a1") });
advance(6000);
const c = card("a1");
return {
  sent: answers().length,
  kept: !!c,
  sending: c ? !c.querySelector('[data-part="sending"]').hidden : null,
  focused: document.activeElement === c,
  disabled: c ? [...c.querySelectorAll('[data-part="act"] button[data-opt]')].map((b) => b.getAttribute("aria-disabled")) : null,
  live: document.getElementById("live").textContent,
};
"##,
    );
    assert_eq!(r["sent"], 0, "{r}");
    assert_eq!(r["kept"], true, "{r}");
    assert_eq!(r["sending"], false, "{r}");
    assert_eq!(r["focused"], true, "{r}");
    assert_eq!(r["disabled"], json!(["true", "true"]), "{r}");
    assert_eq!(r["live"], "a1 was already resolved. Nothing was sent.", "{r}");
}

#[tokio::test]
async fn a_reply_waiting_on_undo_is_not_sent_after_the_stream_drops() {
    let (html, st) = two_gate_page().await;
    let r = page_scenario(
        &html,
        &st,
        r##"
emitState(base);
card("a1").querySelector('[data-opt="1"]').dispatch("click");
advance(1000);
eventSource.onerror();
advance(5000);
const sent = answers().length;
const alert = document.getElementById("alert").textContent;
emitState(base);
return { sent, alert, again: !card("a1").querySelector('[data-part="act"]').hidden };
"##,
    );
    assert_eq!(r["sent"], 0, "{r}");
    assert_eq!(r["alert"], "a1 was not sent because the board is offline. Send it again after it reconnects.", "{r}");
    assert_eq!(r["again"], true, "{r}");
}

#[tokio::test]
async fn a_reply_is_not_sent_to_a_new_card_that_reuses_the_id() {
    let (html, st) = two_gate_page().await;
    let r = page_scenario(
        &html,
        &st,
        r##"
emitState(base);
card("a1").querySelector('[data-opt="1"]').dispatch("click");
emitState({ ...base, cards: base.cards.map((c) => c.id === "a1" ? { ...c, at: "2099-01-01T00:00:00Z" } : c) });
advance(6000);
return { sent: answers().length, live: document.getElementById("live").textContent };
"##,
    );
    assert_eq!(r["sent"], 0, "{r}");
    assert_eq!(r["live"], "a1 was already resolved. Nothing was sent.", "{r}");
}

#[tokio::test]
async fn a_reply_is_not_sent_when_the_card_gets_a_new_handle() {
    let (html, st) = two_gate_page().await;
    let r = page_scenario(
        &html,
        &st,
        r##"
emitState(base);
card("a1").querySelector('[data-opt="1"]').dispatch("click");
emitState({ ...base, cards: base.cards.map((c) => c.id === "a1" ? { ...c, handle: "m99" } : c) });
const act = !card("a1").querySelector('[data-part="act"]').hidden;
const focused = document.activeElement === card("a1");
advance(6000);
return { act, focused, sent: answers().length, live: document.getElementById("live").textContent };
"##,
    );
    assert_eq!(r["act"], true, "{r}");
    assert_eq!(r["focused"], true, "{r}");
    assert_eq!(r["sent"], 0, "{r}");
    assert_eq!(r["live"], "a1 was already resolved. Nothing was sent.", "{r}");
}

#[tokio::test]
async fn closing_an_action_error_returns_focus_to_the_action_or_the_composer() {
    let (html, st) = two_gate_page().await;
    let r = run_page_async(
        &html,
        &format!(
            "async () => {{\n{PAGE_HELPERS}\nconst base = {st};\n{}\n}}",
            r##"
emitState(base);
fetchReply = () => Promise.reject(new Error("settle failed"));
const composer = document.getElementById("composer");
const b = document.createElement("button");
composer.append(b);
const err = document.getElementById("composer-err");
const close = document.getElementById("composer-err-close");
await act(b, "/api/asks/q1/settle");
const shown = !err.hidden;
close.focus();
close.dispatch("click");
const back = document.activeElement === b;
await act(b, "/api/asks/q1/settle");
b.disabled = true;
close.focus();
close.dispatch("click");
const unfocusable = document.activeElement === document.getElementById("issues");
b.disabled = false;
await act(b, "/api/asks/q1/settle");
b.remove();
close.focus();
close.dispatch("click");
return { shown, back, unfocusable, hidden: err.hidden, fallback: document.activeElement === document.getElementById("issues") };
"##
        ),
    );
    assert_eq!(r["shown"], true, "{r}");
    assert_eq!(r["back"], true, "{r}");
    assert_eq!(r["unfocusable"], true, "{r}");
    assert_eq!(r["hidden"], true, "{r}");
    assert_eq!(r["fallback"], true, "{r}");
}

#[tokio::test]
async fn a_late_message_for_an_old_handle_never_replaces_the_new_one() {
    let (html, st) = two_gate_page().await;
    let r = run_page_async(
        &html,
        &format!(
            "async () => {{\n{PAGE_HELPERS}\nconst base = {st};\n{}\n}}",
            r##"
const waiting = [];
fetchReply = () => new Promise((resolve) => waiting.push(resolve));
const reply = (body) => ({ ok: true, json: async () => ({ body }) });
const tick = async () => { for (let i = 0; i < 5; i++) await Promise.resolve(); };
emitState(base);
const c = card("a1");
const btn = c.querySelector('[data-part="fullbtn"]');
const pre = c.querySelector('[data-part="full"]').firstChild;
btn.dispatch("click");
emitState({ ...base, cards: base.cards.map((x) => x.id === "a1" ? { ...x, handle: "m99" } : x) });
btn.dispatch("click");
waiting[1](reply("new body"));
await tick();
waiting[0](reply("old body"));
await tick();
const shown = pre.textContent;
btn.dispatch("click");
btn.dispatch("click");
return { shown, again: pre.textContent, msgs: fetches.filter((f) => f.url.startsWith("/api/msg/")).map((f) => f.url) };
"##
        ),
    );
    assert_eq!(r["shown"], "new body", "{r}");
    assert_eq!(r["again"], "new body", "{r}");
    assert_eq!(r["msgs"].as_array().unwrap().len(), 2, "{r}");
    assert_eq!(r["msgs"][1], "/api/msg/m99", "{r}");
}

#[tokio::test]
async fn a_card_id_reused_by_a_new_decision_drops_the_old_draft_and_message() {
    let (html, st) = two_gate_page().await;
    let r = page_scenario(
        &html,
        &st,
        r##"
emitState(base);
const c = card("a1");
c.querySelector('[data-part="replybtn"]').dispatch("click");
typeKeys("for the old gate");
c.querySelector('[data-part="fullbtn"]').dispatch("click");
const reused = { ...base, cards: base.cards.map((x) => x.id === "a1" ? { ...x, at: "2099-01-01T00:00:00Z", handle: "m99" } : x) };
emitState(reused);
const after = {
  same: card("a1") === c,
  value: c.querySelector("input").value,
  reply: c.querySelector('[data-part="reply"]').hidden,
  full: c.querySelector('[data-part="full"]').hidden,
  expanded: c.querySelector('[data-part="fullbtn"]').getAttribute("aria-expanded"),
};
emitState(reused);
c.querySelector('[data-part="fullbtn"]').dispatch("click");
return { ...after, kept: c.querySelector("input").value, msgs: fetches.filter((f) => f.url.startsWith("/api/msg/")).map((f) => f.url) };
"##,
    );
    assert_eq!(r["same"], true, "{r}");
    assert_eq!(r["value"], "", "{r}");
    assert_eq!(r["reply"], true, "{r}");
    assert_eq!(r["full"], true, "{r}");
    assert_eq!(r["expanded"], "false", "{r}");
    assert_eq!(r["kept"], "", "{r}");
    assert_eq!(r["msgs"].as_array().unwrap().len(), 2, "{r}");
    assert_eq!(r["msgs"][1], "/api/msg/m99", "{r}");
}

#[tokio::test]
async fn a_sent_reply_does_not_hold_a_new_card_that_reuses_the_id() {
    let (html, st) = two_gate_page().await;
    let r = page_scenario(
        &html,
        &st,
        r##"
emitState(base);
card("a1").querySelector('[data-opt="1"]').dispatch("click");
advance(5000);
const sent = answers().length;
emitState({ ...base, cards: base.cards.map((c) => c.id === "a1" ? { ...c, at: "2099-01-01T00:00:00Z" } : c) });
return { sent, act: !card("a1").querySelector('[data-part="act"]').hidden };
"##,
    );
    assert_eq!(r["sent"], 1, "{r}");
    assert_eq!(r["act"], true, "{r}");
}

#[tokio::test]
async fn a_restore_that_finishes_while_its_row_is_stale_stays_disabled() {
    let (html, st) = two_gate_page().await;
    let r = run_page_async(
        &html,
        &format!(
            "async () => {{\n{PAGE_HELPERS}\nconst base = {st};\n{}\n}}",
            r##"
const waiting = [];
fetchReply = () => new Promise((resolve) => waiting.push(resolve));
const T = { thread: "th-7", label: "acme/widgets#7", settledAt: "2026-01-01T00:00:00Z" };
const withT = { ...base, settled: { tickets: [T], asks: [] } };
emitState(withT);
const b = q('#settled [data-key="t:th-7"] [data-part="restore"]');
b.focus();
b.dispatch("click");
emitState({ ...base, settled: { tickets: [], asks: [] } });
waiting[0]({ ok: true, json: async () => ({}) });
for (let i = 0; i < 5; i++) await Promise.resolve();
const stale = b.getAttribute("aria-disabled");
emitState(withT);
return { stale, back: b.getAttribute("aria-disabled") };
"##
        ),
    );
    assert_eq!(r["stale"], "true", "{r}");
    assert_eq!(r["back"], serde_json::Value::Null, "{r}");
}

#[tokio::test]
async fn a_card_replaced_while_typing_a_reply_keeps_keyboard_focus() {
    let (html, st) = two_gate_page().await;
    let r = page_scenario(
        &html,
        &st,
        r##"
emitState(base);
const c = card("a1");
c.querySelector('[data-part="replybtn"]').dispatch("click");
typeKeys("for the old gate");
emitState({ ...base, cards: base.cards.map((x) => x.id === "a1" ? { ...x, at: "2099-01-01T00:00:00Z" } : x) });
return { focused: document.activeElement === c, value: c.querySelector("input").value };
"##,
    );
    assert_eq!(r["focused"], true, "{r}");
    assert_eq!(r["value"], "", "{r}");
}

#[tokio::test]
async fn a_resolution_note_stays_with_the_decision_that_produced_it() {
    let (html, st) = two_gate_page().await;
    let r = run_page_async(
        &html,
        &format!(
            "async () => {{\n{PAGE_HELPERS}\nconst base = {st};\n{}\n}}",
            r##"
fetchReply = async () => ({ ok: true, json: async () => ({}) });
emitState(base);
card("a1").querySelector('[data-opt="1"]').dispatch("click");
advance(5000);
for (let i = 0; i < 5; i++) await Promise.resolve();
emitState({ ...base, cards: base.cards.map((x) => x.id === "a1" ? { ...x, at: "2099-01-01T00:00:00Z" } : x) });
card("a2").focus();
emitState({ ...base, cards: base.cards.filter((x) => x.id !== "a1") });
return { note: card("a1").textContent };
"##
        ),
    );
    assert_eq!(r["note"], "a1 resolved", "{r}");
}

#[tokio::test]
async fn a_pending_launch_row_is_not_given_to_one_of_two_threads_with_its_label() {
    let (html, st) = two_gate_page().await;
    let r = page_scenario(
        &html,
        &st,
        r##"
const P = { ...base.tree[0], thread: null, label: "acme/widgets#9", status: "running", done: false, gate: null, last: null };
emitState({ ...base, tree: [...base.tree, P] });
const row = q('#open [data-key="pending:acme/widgets#9"]');
q("#open").children[0].focus();
emitState({ ...base, tree: [...base.tree, { ...P, thread: "th-9" }, { ...P, thread: "th-10" }] });
return { key: row.getAttribute("data-key"), gone: row.classList.contains("gone") };
"##,
    );
    assert_eq!(r["key"], "pending:acme/widgets#9", "{r}");
    assert_eq!(r["gone"], true, "{r}");
}

#[tokio::test]
async fn the_composer_error_strip_moves_with_the_dock() {
    let (html, _) = two_gate_page().await;
    let dock = html.find(r#"id="dock""#).unwrap();
    let err = html.find(r#"id="composer-err""#).unwrap();
    let forms = html.find(r#"class="forms""#).unwrap();
    assert!(dock < err && err < forms);
}

#[tokio::test]
async fn a_settled_row_that_comes_back_can_be_restored_again() {
    let (html, st) = two_gate_page().await;
    let r = page_scenario(
        &html,
        &st,
        r##"
const T = { thread: "th-7", label: "acme/widgets#7", settledAt: "2026-01-01T00:00:00Z" };
const withT = { ...base, settled: { tickets: [T], asks: [] } };
emitState(withT);
const b = q('#settled [data-key="t:th-7"] [data-part="restore"]');
b.focus();
emitState({ ...base, settled: { tickets: [], asks: [] } });
const stale = b.getAttribute("aria-disabled");
emitState(withT);
b.dispatch("click");
return { stale, after: b.getAttribute("aria-disabled"), posts: fetches.filter((f) => /\/restore$/.test(f.url)).length };
"##,
    );
    assert_eq!(r["stale"], "true", "{r}");
    assert_eq!(r["posts"], 1, "{r}");
}

#[tokio::test]
async fn a_launch_keeps_its_row_when_it_gets_a_thread() {
    let (html, st) = two_gate_page().await;
    let r = page_scenario(
        &html,
        &st,
        r##"
const P = { ...base.tree[0], thread: null, label: "acme/widgets#9", status: "running", done: false, gate: null, last: null };
emitState({ ...base, tree: [...base.tree, P] });
const row = q('#open [data-key="pending:acme/widgets#9"]');
q("#open").children[0].focus();
emitState({ ...base, tree: [...base.tree, { ...P, thread: "th-9" }] });
return {
  same: q('#open [data-key="th-9"]') === row,
  gone: row.classList.contains("gone"),
  held: !q("#new-tix").classList.contains("off"),
};
"##,
    );
    assert_eq!(r["same"], true, "{r}");
    assert_eq!(r["gone"], false, "{r}");
    assert_eq!(r["held"], false, "{r}");
}

#[tokio::test]
async fn board_shortcuts_wait_while_the_shortcuts_dialog_is_open() {
    let (html, st) = two_gate_page().await;
    let r = page_scenario(
        &html,
        &st,
        r##"
emitState(base);
const before = document.getElementById("app").dataset.panel;
document.getElementById("keys").open = true;
const close = document.getElementById("keys-close");
for (const key of ["g", "t"]) for (const f of document.listeners.keydown) f({ key, target: close, preventDefault() {} });
return { before, after: document.getElementById("app").dataset.panel };
"##,
    );
    assert_eq!(r["after"], r["before"], "{r}");
}

#[tokio::test]
async fn options_have_equal_weight_and_digits_only_focus_them() {
    let (html, st) = two_gate_page().await;
    let r = page_scenario(
        &html,
        &st,
        r##"
emitState(base);
const opts = [...document.querySelectorAll("#cards button[data-opt]")];
card("a1").focus();
for (const f of document.listeners.keydown) f({ key: "2", target: card("a1"), preventDefault() {} });
advance(10000);
return {
  classes: [...new Set(opts.map((b) => b.className))],
  names: opts.map((b) => b.getAttribute("aria-label")),
  focused: document.activeElement.textContent,
  sent: answers().length,
};
"##,
    );
    assert_eq!(r["classes"], json!(["btn"]), "{r}");
    assert_eq!(r["names"][0], "merge, reply to a1 widgets#41", "{r}");
    assert_eq!(r["names"][3], "merge and deploy, reply to a2 widgets#42", "{r}");
    assert_eq!(r["focused"], "merge and deploy", "{r}");
    assert_eq!(r["sent"], 0, "{r}");
}

#[tokio::test]
async fn ticket_rows_update_in_place_and_finished_tickets_move_to_done() {
    let (html, st) = two_gate_page().await;
    let r = page_scenario(
        &html,
        &st,
        &format!(
            r##"
const T = "{TICKET}";
const row = (list) => q("#" + list + ' [data-thread="' + T + '"]');
emitState(base);
const label = row("open").querySelector('[data-part="label"]');
const noted = base.tree.map((t) => t.thread === T ? {{ ...t, last: {{ handle: "m9", kind: "note", head: "**ACTION NEEDED:** build green", at: "10:15 MST" }} }} : t);
emitState({{ ...base, cards: [], tree: noted }});
const kept = row("open").querySelector('[data-part="label"]') === label;
const note = row("open").querySelector('[data-part="note"]').textContent;
q("#open").children.find((el) => el.dataset.thread !== T).focus();
const done = noted.map((t) => t.thread === T ? {{ ...t, done: true, status: "done" }} : t);
emitState({{ ...base, cards: [], tree: done }});
return {{
  kept, note,
  placeholder: row("open").textContent,
  inDone: !!row("done"),
  settle: row("done").querySelector('[data-part="settle"]').getAttribute("aria-label"),
  doneCount: q("#n-done").textContent,
}};
"##
        ),
    );
    assert_eq!(r["kept"], true, "{r}");
    assert_eq!(r["note"], "10:15  note: build green", "{r}");
    assert_eq!(r["placeholder"], "widgets#41 finished and moved to Done", "{r}");
    assert_eq!(r["inDone"], true, "{r}");
    assert_eq!(r["settle"], "Settle widgets#41", "{r}");
    assert_eq!(r["doneCount"], "1", "{r}");
}

#[tokio::test]
async fn a_needs_you_row_links_to_its_card() {
    let (html, st) = two_gate_page().await;
    let r = page_scenario(
        &html,
        &st,
        &format!(
            r##"
emitState(base);
const needs = q('#open [data-thread="{TICKET}"] [data-part="needs"]');
needs.dispatch("click");
const toCard = document.activeElement.dataset.key;
card("a1").querySelector('[data-part="id"]').dispatch("click");
return {{ text: needs.textContent, hidden: needs.hidden, toCard, back: document.activeElement.dataset.thread }};
"##
        ),
    );
    assert_eq!(r["text"], "needs you - a1", "{r}");
    assert_eq!(r["hidden"], false, "{r}");
    assert_eq!(r["toCard"], "a1", "{r}");
    assert_eq!(r["back"], TICKET, "{r}");
}

#[tokio::test]
async fn a_dropped_stream_marks_the_board_stale_and_holds_replies() {
    let (html, st) = two_gate_page().await;
    let r = page_scenario(
        &html,
        &st,
        r##"
emitState(base);
eventSource.onerror();
const opts = [...document.querySelectorAll("#cards [data-opt]")];
opts[0].dispatch("click");
advance(6000);
const during = {
  stale: document.body.classList.contains("stale"),
  header: q("#svc-text").textContent,
  disabled: opts.every((b) => b.getAttribute("aria-disabled") === "true"),
  sent: answers().length,
};
emitState(base);
return { during, after: { stale: document.body.classList.contains("stale"), header: q("#svc-text").textContent } };
"##,
    );
    assert_eq!(r["during"]["stale"], true, "{r}");
    assert!(r["during"]["header"].as_str().unwrap().starts_with("Disconnected"), "{r}");
    assert_eq!(r["during"]["disabled"], true, "{r}");
    assert_eq!(r["during"]["sent"], 0, "{r}");
    assert_eq!(r["after"]["stale"], false, "{r}");
    assert!(r["after"]["header"].as_str().unwrap().starts_with("T3 connected"), "{r}");
}

fn worker_pr(h: &H) -> Value {
    let tree = h.app.wf.tree().unwrap();
    let t = tree.iter().find(|t| t["label"] == "widgets#41").unwrap();
    t["workers"][0]["pr"].clone()
}

fn pr_lists(h: &H) -> usize {
    h.fake.calls().iter().filter(|c| *c == "list_thread_pull_requests").count()
}

async fn ticket_with_worker(h: &H) {
    join_board(h, TICKET, "widgets#41").await;
    h.wf(WORKER, "join", json!({"parent": TICKET, "label": "#41 PR 1/2", "role": "worker"})).await;
}

#[tokio::test]
async fn a_finished_workers_merged_pr_shows_on_the_board() {
    let h = H::new();
    ticket_with_worker(&h).await;
    h.fake.set_prs(WORKER, vec![pull_request("acme/widgets", 81, "open")]);
    h.tick(1).await;
    assert_eq!(worker_pr(&h)["state"], "open");
    assert_eq!(pr_lists(&h), 0);
    h.fake.set_prs(WORKER, vec![pull_request("acme/widgets", 81, "merged")]);
    h.wf(WORKER, "send", json!({"kind": "handoff", "body": "merged at abc123"})).await;
    let pr = worker_pr(&h);
    assert_eq!(
        (pr["number"].clone(), pr["state"].clone(), pr["url"].clone()),
        (json!(81), json!("merged"), json!("https://github.com/acme/widgets/pull/81"))
    );
    let tree = h.app.wf.tree().unwrap();
    assert_eq!(tree[0]["workers"][0]["done"], true);
    h.tick(2).await;
    h.app.wf.board_tick().await.unwrap();
    assert_eq!(pr_lists(&h), 1);
}

#[tokio::test]
async fn a_finished_workers_closed_pr_shows_after_a_restart() {
    let mut h = H::new();
    ticket_with_worker(&h).await;
    h.fake.set_prs(WORKER, vec![pull_request("acme/widgets", 82, "closed")]);
    h.wf(WORKER, "send", json!({"kind": "closed", "body": "abandoned"})).await;
    assert_eq!(worker_pr(&h)["state"], "closed");
    h.reopen();
    assert_eq!(worker_pr(&h), Value::Null);
    h.app.wf.board_tick().await.unwrap();
    assert_eq!((worker_pr(&h)["number"].clone(), worker_pr(&h)["state"].clone()), (json!(82), json!("closed")));
}

#[tokio::test]
async fn a_reopened_pr_is_listed_again_when_it_closes() {
    let h = H::new();
    ticket_with_worker(&h).await;
    h.fake.set_prs(WORKER, vec![pull_request("acme/widgets", 83, "closed")]);
    h.tick(3).await;
    assert_eq!(worker_pr(&h)["state"], "closed");
    h.fake.set_prs(WORKER, vec![pull_request("acme/widgets", 83, "open")]);
    h.tick(4).await;
    assert_eq!(worker_pr(&h)["state"], "open");
    h.fake.set_prs(WORKER, vec![pull_request("acme/widgets", 83, "merged")]);
    h.tick(5).await;
    assert_eq!(worker_pr(&h)["state"], "merged");
    assert_eq!(pr_lists(&h), 2);
}

#[tokio::test]
async fn a_failed_pr_lookup_is_retried_on_the_next_board_tick() {
    let h = H::new();
    ticket_with_worker(&h).await;
    h.fake.set_prs(WORKER, vec![pull_request("acme/widgets", 84, "merged")]);
    h.fake.set_prs_fail(true);
    h.wf(WORKER, "send", json!({"kind": "handoff", "body": "merged at abc123"})).await;
    assert_eq!(worker_pr(&h), Value::Null);
    h.app.wf.board_tick().await.unwrap();
    assert_eq!(worker_pr(&h), Value::Null);
    h.fake.set_prs_fail(false);
    h.app.wf.board_tick().await.unwrap();
    assert_eq!(worker_pr(&h)["state"], "merged");
    h.app.wf.board_tick().await.unwrap();
    assert_eq!(pr_lists(&h), 3);
}

#[tokio::test]
async fn a_failed_read_at_handoff_is_retried_on_the_next_board_tick() {
    let h = H::new();
    ticket_with_worker(&h).await;
    h.fake.set_prs(WORKER, vec![pull_request("acme/widgets", 85, "open")]);
    h.tick(1).await;
    assert_eq!(worker_pr(&h)["state"], "open");
    h.fake.set_prs(WORKER, vec![pull_request("acme/widgets", 85, "merged")]);
    h.fake.set_read_fails(true);
    h.wf(WORKER, "send", json!({"kind": "handoff", "body": "merged at abc123"})).await;
    h.fake.set_read_fails(false);
    h.app.wf.board_tick().await.unwrap();
    assert_eq!(worker_pr(&h)["state"], "merged");
}

#[tokio::test]
async fn a_closed_pr_that_reopens_and_merges_between_reads_shows_merged() {
    let h = H::new();
    ticket_with_worker(&h).await;
    h.fake.set_prs(WORKER, vec![pull_request("acme/widgets", 86, "closed")]);
    h.tick(1).await;
    assert_eq!(worker_pr(&h)["state"], "closed");
    h.fake.set_prs(WORKER, vec![pull_request("acme/widgets", 86, "merged")]);
    h.tick(2).await;
    assert_eq!(worker_pr(&h)["state"], "merged");
    h.tick(3).await;
    assert_eq!(pr_lists(&h), 2);
}

#[tokio::test]
async fn worker_prs_are_read_across_every_page() {
    let h = H::new();
    ticket_with_worker(&h).await;
    h.fake.set_pr_page(2);
    h.fake.set_prs(
        WORKER,
        vec![
            pull_request("acme/widgets", 87, "merged"),
            pull_request("acme/widgets", 88, "merged"),
            pull_request("acme/widgets", 89, "merged"),
        ],
    );
    h.tick(1).await;
    assert_eq!((worker_pr(&h)["number"].clone(), worker_pr(&h)["state"].clone()), (json!(89), json!("merged")));
    assert_eq!(pr_lists(&h), 2);
}

#[tokio::test(start_paused = true)]
async fn a_stalled_pr_lookup_gives_up_and_is_retried() {
    let h = H::new();
    ticket_with_worker(&h).await;
    h.fake.set_prs(WORKER, vec![pull_request("acme/widgets", 90, "merged")]);
    h.fake.set_delay(Duration::from_secs(3600));
    h.tick(1).await;
    assert_eq!(worker_pr(&h), Value::Null);
    h.fake.set_delay(Duration::ZERO);
    h.tick(2).await;
    assert_eq!(worker_pr(&h)["state"], "merged");
}

#[tokio::test]
async fn a_pr_that_reopens_between_the_read_and_the_list_is_listed_again() {
    let h = H::new();
    ticket_with_worker(&h).await;
    h.fake.set_prs(WORKER, vec![pull_request("acme/widgets", 91, "closed")]);
    h.fake.set_prs_before_list(WORKER, vec![pull_request("acme/widgets", 91, "open")]);
    h.tick(1).await;
    assert_eq!(worker_pr(&h)["state"], "open");
    h.fake.set_prs(WORKER, vec![pull_request("acme/widgets", 91, "merged")]);
    h.tick(2).await;
    assert_eq!(worker_pr(&h)["state"], "merged");
}

#[tokio::test]
async fn a_worker_that_finishes_with_an_open_pr_shows_its_merge() {
    let h = H::new();
    ticket_with_worker(&h).await;
    h.fake.set_prs(WORKER, vec![pull_request("acme/widgets", 92, "closed")]);
    h.fake.set_prs_before_list(WORKER, vec![pull_request("acme/widgets", 92, "open")]);
    h.wf(WORKER, "send", json!({"kind": "handoff", "body": "ready to merge"})).await;
    assert_eq!(worker_pr(&h)["state"], "open");
    h.fake.set_prs(WORKER, vec![pull_request("acme/widgets", 92, "merged")]);
    h.app.wf.board_tick().await.unwrap();
    assert_eq!(worker_pr(&h)["state"], "merged");
    let reads = |h: &H| h.fake.calls().iter().filter(|c| *c == "t3_thread_read").count();
    let before = reads(&h);
    h.app.wf.board_tick().await.unwrap();
    assert_eq!(reads(&h), before);
}

fn by_label<'a>(items: &'a Value, label: &str) -> &'a Value {
    items.as_array().unwrap().iter().find(|t| t["label"] == label).unwrap_or_else(|| panic!("no {label} in {items}"))
}

#[tokio::test]
async fn the_state_carries_each_tickets_repo_project_and_title() {
    let mut h = H::new();
    let mut gadgets = project("p-gadgets", "/src/gadgets");
    gadgets["title"] = json!("Gadgets");
    gadgets["repositoryIdentity"] = json!({
        "canonicalKey": "github.com/acme/gadgets", "owner": "acme", "name": "gadgets",
        "locator": {"source": "git-remote", "remoteName": "origin", "remoteUrl": "https://github.com/acme/gadgets.git"},
    });
    h.fake.set_projects(vec![project("p-widgets", "/src/widgets"), gadgets]);
    h.remotes.set("/src/widgets", "git@github.com:acme/widgets.git");
    h.fake.update(TICKET, json!({"projectId": "p-widgets", "title": "widgets#41: Fix the grader"}));
    h.fake.update(WORKER, json!({"projectId": "p-gadgets"}));
    join_board(&h, TICKET, "widgets#41").await;
    join_board(&h, WORKER, "gadgets#7").await;
    h.wf(TICKET, "send", json!({"kind": "gate", "gate": "merge", "body": GATE})).await;
    h.wf(WORKER, "send", json!({"kind": "event", "body": "PR opened"})).await;
    h.app.wf.board_tick().await.unwrap();
    let st = snapshot(&h);
    let w = by_label(&st["tree"], "widgets#41");
    assert_eq!(w["repo"], "acme/widgets", "{w}");
    assert_eq!(w["project"], json!({"id": "p-widgets", "title": "p-widgets"}));
    assert_eq!(w["title"], "widgets#41: Fix the grader");
    let g = by_label(&st["tree"], "gadgets#7");
    assert_eq!(g["repo"], "acme/gadgets", "{g}");
    assert_eq!(g["project"], json!({"id": "p-gadgets", "title": "Gadgets"}));
    assert_eq!(st["cards"][0]["ticket"], TICKET);
    assert_eq!(st["cards"][0]["repo"], "acme/widgets");
    assert_eq!(st["cards"][0]["project"]["id"], "p-widgets");
    let f = by_label(&st["feed"], "gadgets#7");
    assert_eq!((&f["ticket"], &f["repo"]), (&json!(WORKER), &json!("acme/gadgets")), "{f}");
    assert_eq!((&f["ticketLabel"], &f["ticketTitle"]), (&json!("gadgets#7"), &json!(WORKER)), "{f}");
    h.reopen();
    assert_eq!(by_label(&snapshot(&h)["tree"], "gadgets#7")["project"]["id"], "p-gadgets");
    h.app.wf.board_tick().await.unwrap();
    let again = snapshot(&h);
    assert_eq!(by_label(&again["tree"], "gadgets#7")["repo"], "acme/gadgets");
    assert_eq!(by_label(&again["tree"], "widgets#41")["repo"], "acme/widgets");
}

const FILTER_HELPERS: &str = r##"
const $$ = (id) => document.getElementById(id);
const keysOf = (id) => [...$$(id).children].filter((e) => !e.classList.contains("gone")).map((e) => e.dataset.key);
const facet = (key) => q('[data-facet="' + key + '"]');
const pick = (key, val) => {
  if (facet(key).querySelector('[data-part="fpop"]').hidden) facet(key).querySelector('[data-part="fbtn"]').dispatch("click");
  const box = facet(key).querySelector('[data-val="' + val + '"] input');
  box.checked = !box.checked;
  box.dispatch("change");
};
const view = () => ({
  cards: ids(), open: keysOf("open"), done: keysOf("done"), feed: keysOf("feed"), settled: keysOf("settled"),
  n: ["n-cards", "n-open", "n-done", "n-feed", "n-settled"].map((id) => $$(id).textContent),
});
"##;

fn filter_scenario(html: &str, st: &Value, body: &str) -> Value {
    let wrap = format!(
        r##"{FILTER_HELPERS}
const T = "{TICKET}", W = "{WORKER}";
const repoOf = (t) => t === T ? "acme/widgets" : "acme/gadgets";
const place = (t) => ({{ repo: repoOf(t), project: {{ id: "p-" + repoOf(t).slice(5), title: repoOf(t).slice(5) }} }});
const s0 = {{
  ...base,
  tree: base.tree.map((t) => ({{ ...t, ...place(t.thread), title: t.thread === T ? "widgets#41: Fix the grader" : "widgets#42: Ship the gadget" }})),
  cards: base.cards.map((c) => ({{ ...c, ...place(c.ticket) }})),
  feed: [
    {{ handle: "m91", at: "10:00 MST", iso: "2026-11-17T17:00:00Z", label: "widgets#41", kind: "note", head: "tests green", ticket: T, ...place(T) }},
    {{ handle: "m92", at: "10:01 MST", iso: "2026-11-17T17:01:00Z", label: "widgets#42", kind: "event", head: "PR opened", ticket: W, ...place(W) }},
  ],
  settled: {{
    tickets: [{{ thread: "th-s", label: "gadgets#3", status: "done", done: true, settledAt: "09:00 MST", pr: null, link: null, ...place(W) }}],
    asks: [{{ id: "q1", seq: 1, question: "Which tickets wait?", answer: "None", status: "answered" }}],
  }},
  counts: {{ ...base.counts, settled: 2 }},
}};
{body}"##
    );
    page_scenario(html, st, &wrap)
}

#[tokio::test]
async fn a_project_filter_narrows_every_list_and_shows_visible_counts() {
    let (html, st) = two_gate_page().await;
    let r = filter_scenario(
        &html,
        &st,
        r##"
emitState(s0);
const all = view();
facet("projects").querySelector('[data-part="fbtn"]').dispatch("click");
const opts = [...facet("projects").querySelectorAll(".fopt")].map((o) => o.dataset.val);
const find = facet("projects").querySelector('[data-part="ffind"]');
find.value = "gadg";
find.dispatch("input");
const found = [...facet("projects").querySelectorAll(".fopt")].filter((o) => !o.hidden).map((o) => o.dataset.val);
find.value = "";
find.dispatch("input");
pick("projects", "acme/widgets");
const one = { ...view(), btn: facet("projects").querySelector('[data-part="fbtn"]').textContent, clear: !$$("filter-clear").hidden };
pick("projects", "acme/gadgets");
const two = view();
$$("filter-clear").dispatch("click");
return { all, opts, found, one, two, after: view(), btn: facet("projects").querySelector('[data-part="fbtn"]').textContent };
"##,
    );
    let t = json!(TICKET);
    let w = json!(WORKER);
    assert_eq!(r["all"]["cards"], json!(["a1", "a2"]), "{r}");
    assert_eq!(r["all"]["n"], json!(["2", "2", "", "2", "2"]), "{r}");
    assert_eq!(r["opts"], json!(["acme/gadgets", "acme/widgets"]), "{r}");
    assert_eq!(r["found"], json!(["acme/gadgets"]), "{r}");
    assert_eq!(r["one"]["cards"], json!(["a1"]), "{r}");
    assert_eq!(r["one"]["open"], json!([t]), "{r}");
    assert_eq!(r["one"]["feed"], json!(["m91"]), "{r}");
    assert_eq!(r["one"]["settled"], json!([]), "{r}");
    assert_eq!(r["one"]["n"], json!(["1/2", "1/2", "0/0", "1/2", "0/2"]), "{r}");
    assert_eq!(r["one"]["btn"], "Project: acme/widgets", "{r}");
    assert_eq!(r["one"]["clear"], true, "{r}");
    assert_eq!(r["two"]["cards"], json!(["a1", "a2"]), "{r}");
    assert_eq!(r["two"]["settled"], json!(["t:th-s"]), "{r}");
    assert_eq!(r["after"]["open"].as_array().unwrap().len(), 2, "{r}");
    assert!(r["after"]["open"].as_array().unwrap().contains(&w), "{r}");
    assert_eq!(r["after"]["n"], r["all"]["n"], "{r}");
    assert_eq!(r["btn"], "Project", "{r}");
}

#[tokio::test]
async fn the_search_box_narrows_after_a_pause_and_esc_clears_it() {
    let (html, st) = two_gate_page().await;
    let r = filter_scenario(
        &html,
        &st,
        r##"
emitState(s0);
const box = $$("filter-q");
box.focus();
typeKeys("42");
const typing = ids();
advance(200);
const number = view();
box.value = "";
typeKeys("GRADER fix");
advance(200);
const words = view();
box.value = "";
typeKeys("gadgets");
advance(200);
const repo = view();
const old = { handle: "m93", at: "09:00 MST", iso: "2026-11-17T16:00:00Z", label: "PR 1/2", kind: "note", head: "rebased", ticket: "th-old", ticketLabel: "gizmos#5", ticketTitle: "gizmos#5: Tune the gizmo", repo: null, project: null };
emitState({ ...s0, feed: [...s0.feed, old] });
box.value = "";
typeKeys("tune");
advance(200);
const parent = view().feed;
box.dispatch("keydown", { key: "Escape", stopPropagation() {} });
return { typing, number, words, repo, parent, cleared: { value: box.value, ...view() }, focused: document.activeElement === box };
"##,
    );
    let t = json!(TICKET);
    let w = json!(WORKER);
    assert_eq!(r["typing"], json!(["a1", "a2"]), "{r}");
    assert_eq!(r["number"]["cards"], json!(["a2"]), "{r}");
    assert_eq!(r["number"]["open"], json!([w]), "{r}");
    assert_eq!(r["number"]["feed"], json!(["m92"]), "{r}");
    assert_eq!(r["words"]["cards"], json!(["a1"]), "{r}");
    assert_eq!(r["words"]["open"], json!([t]), "{r}");
    assert_eq!(r["repo"]["cards"], json!(["a2"]), "{r}");
    assert_eq!(r["repo"]["settled"], json!(["t:th-s"]), "{r}");
    assert_eq!(r["parent"], json!(["m93"]), "{r}");
    assert_eq!(r["cleared"]["value"], "", "{r}");
    assert_eq!(r["cleared"]["cards"], json!(["a1", "a2"]), "{r}");
    assert_eq!(r["cleared"]["settled"], json!(["t:th-s", "a:q1"]), "{r}");
    assert_eq!(r["focused"], true, "{r}");
}

#[tokio::test]
async fn a_new_card_outside_the_filter_shows_as_hidden() {
    let (html, st) = two_gate_page().await;
    let r = filter_scenario(
        &html,
        &st,
        r##"
emitState(s0);
pick("projects", "acme/widgets");
const before = { hidden: $$("hidden-new").hidden };
const a3 = { ...s0.cards[1], id: "a3", at: "2026-11-17T12:30:00Z", kind: "blocker", token: "t-a3" };
emitState({ ...s0, cards: [...s0.cards, a3] });
const after = { hidden: $$("hidden-new").hidden, text: $$("hidden-new").textContent, urgent: $$("hidden-new").getAttribute("data-urgent"), cards: ids(), n: $$("n-cards").textContent };
$$("hidden-new").dispatch("click");
return { before, after, shown: ids(), still: $$("hidden-new").hidden };
"##,
    );
    assert_eq!(r["before"]["hidden"], true, "{r}");
    assert_eq!(
        r["after"],
        json!({"hidden": false, "text": "1 hidden", "urgent": "1", "cards": ["a1"], "n": "1/3"}),
        "{r}"
    );
    assert_eq!(r["shown"], json!(["a3", "a1", "a2"]), "{r}");
    assert_eq!(r["still"], true, "{r}");
}

#[tokio::test]
async fn status_and_kind_filters_pick_tickets_and_cards() {
    let (html, st) = two_gate_page().await;
    let r = filter_scenario(
        &html,
        &st,
        r##"
const a3 = { ...s0.cards[0], id: "a3", kind: "blocker", token: "t-a3" };
const s1 = { ...s0, cards: [...s0.cards, a3], tree: s0.tree.map((t) => t.thread === W ? { ...t, done: true, status: "done", gate: null } : t) };
emitState(s1);
pick("statuses", "done");
const done = view();
pick("statuses", "done");
pick("statuses", "blocked");
const blocked = view();
pick("statuses", "blocked");
pick("kinds", "gate");
const gates = view();
return { done, blocked, gates };
"##,
    );
    let t = json!(TICKET);
    let w = json!(WORKER);
    assert_eq!(r["done"]["cards"], json!(["a2"]), "{r}");
    assert_eq!(r["done"]["open"], json!([]), "{r}");
    assert_eq!(r["done"]["done"], json!([w]), "{r}");
    assert_eq!(r["done"]["settled"], json!(["t:th-s"]), "{r}");
    assert_eq!(r["blocked"]["cards"], json!(["a3", "a1"]), "{r}");
    assert_eq!(r["blocked"]["open"], json!([t]), "{r}");
    assert_eq!(r["blocked"]["feed"], json!(["m91"]), "{r}");
    assert_eq!(r["gates"]["cards"], json!(["a1", "a2"]), "{r}");
    assert_eq!(r["gates"]["open"], json!([t]), "{r}");
}

#[tokio::test]
async fn the_f_key_focuses_search_only_with_single_key_shortcuts_on() {
    let (html, st) = two_gate_page().await;
    let r = page_scenario(
        &html,
        &st,
        r##"
emitState(base);
const press = (key) => { for (const f of document.listeners.keydown) f({ key, target: document.body, preventDefault() {} }); };
press("f");
const on = document.activeElement === document.getElementById("filter-q");
document.activeElement = document.body;
const box = document.getElementById("keys-on");
box.checked = false;
box.dispatch("change");
press("f");
return { on, off: document.activeElement === document.getElementById("filter-q") };
"##,
    );
    assert_eq!(r, json!({"on": true, "off": false}));
}

#[tokio::test]
async fn a_ticket_joined_before_projects_were_stored_gets_its_project_back() {
    let mut h = H::new();
    h.fake.set_projects(vec![project("p-widgets", "/src/widgets")]);
    h.remotes.set("/src/widgets", "git@github.com:acme/widgets.git");
    join_board(&h, TICKET, "widgets#41").await;
    h.wf(TICKET, "send", json!({"kind": "handoff", "body": "done"})).await;
    assert_eq!(h.app.store.link_get(TICKET).unwrap().unwrap().project, None);
    h.fake.update(TICKET, json!({"projectId": "p-widgets"}));
    h.reopen();
    h.app.wf.board_tick().await.unwrap();
    assert_eq!(h.app.store.link_get(TICKET).unwrap().unwrap().project.as_deref(), Some("p-widgets"));
    let st = snapshot(&h);
    let t = by_label(&st["tree"], "widgets#41");
    assert_eq!((&t["repo"], &t["project"]["id"]), (&json!("acme/widgets"), &json!("p-widgets")), "{t}");
}

#[tokio::test]
async fn a_reply_waiting_on_undo_stays_visible_when_the_filter_changes() {
    let (html, st) = two_gate_page().await;
    let r = filter_scenario(
        &html,
        &st,
        r##"
emitState(s0);
card("a1").querySelector("[data-opt]").dispatch("click");
pick("projects", "acme/gadgets");
const during = { cards: ids(), undo: !card("a1").querySelector('[data-part="undo"]').hidden };
advance(6000);
const sent = answers().map((f) => f.url);
card("a2").querySelector("[data-opt]").dispatch("click");
pick("projects", "acme/gadgets");
pick("projects", "acme/widgets");
const held = ids();
card("a2").querySelector('[data-part="undo"]').dispatch("click");
document.activeElement = document.body;
advance(10);
return { during, sent, held, undone: ids(), after: answers().length };
"##,
    );
    assert_eq!(r["during"], json!({"cards": ["a1", "a2"], "undo": true}), "{r}");
    assert_eq!(r["sent"], json!(["/api/cards/a1/answer"]), "{r}");
    assert_eq!(r["held"], json!(["a1", "a2"]), "{r}");
    assert_eq!(r["undone"], json!(["a1"]), "{r}");
    assert_eq!(r["after"], 1, "{r}");
}
