use std::time::Duration;

use crate::common::*;
use serde_json::{Value, json};
use t3_codemode::board::{BOARD, Tracker, card_text, parse_issue, parse_remote, reply_options};
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
    assert_eq!(i.tracker, Tracker::Github);
    assert_eq!(i.host.as_deref(), Some("github.com"));
    assert_eq!((i.scope.as_str(), i.key.as_str()), ("Acme/widgets", "41"));
    assert_eq!(i.url(), Some("https://github.com/Acme/widgets/issues/41"));
    assert_eq!(i.label(), "widgets#41");
    assert_eq!(i.slug(), "Acme/widgets#41");
    assert_eq!(parse_issue("Acme/widgets#7").unwrap().key, "7");
    assert_eq!(parse_issue("github.com/o/r/issues/3").unwrap().scope, "o/r");
    assert!(parse_issue("https://gitlab.com/o/r/issues/3").is_none());
    assert!(parse_issue("o/r#0").is_none());
    for url in [
        "git@github.com:Acme/WIDGETS.git",
        "https://github.com/acme/widgets",
        "https://x-access-token:abc@github.com/Acme/widgets.git",
        "ssh://git@github.com:22/Acme/widgets.git",
    ] {
        let remote = parse_remote(url).unwrap_or_else(|| panic!("{url}"));
        assert_eq!(remote.host, "github.com", "{url}");
        assert!(remote.path.eq_ignore_ascii_case("acme/widgets"), "{url}: {}", remote.path);
    }
    let gl = parse_remote("git@gitlab.com:o/r.git").unwrap();
    assert_eq!((gl.host.as_str(), gl.path.as_str()), ("gitlab.com", "o/r"));
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
    assert!(t3_codemode::board::ask_brief("q", Some(&[])).contains("all: true"));
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
    assert!(
        msg.contains(r#"wf.join({ parent: "board", label: "widgets#41", role: "ticket", issue: "Acme/widgets#41" })"#),
        "{msg}"
    );
    let pending = h.app.store.pending_all().unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].parent, BOARD);
    assert_eq!(pending[0].thread.as_deref(), Some("launch:1"));
    let tree = h.app.wf.tree().unwrap();
    assert_eq!(tree[0]["joined"], false);
    assert_eq!(tree[0]["status"], "running");
    assert_eq!(tree[0]["repo"], "acme/WIDGETS");
    assert_eq!(tree[0]["project"], json!({"id": "p-widgets", "title": "p-widgets"}));
    assert_eq!(tree[0]["title"], "widgets#41: Fix the grader");
    assert_eq!(tree[0]["issue"], "Acme/widgets#41");
    let again = h.exec("async () => wf.launchTicket({ issue: 'Acme/widgets#41' })").await;
    assert!(err_text(&again).contains("has not joined yet"), "{}", again.text);
    h.wf("launch:1", "join", json!({"parent": BOARD, "label": "widgets#41", "role": "ticket"})).await;
    assert!(h.app.store.pending_all().unwrap().is_empty());
    let ln = h.app.store.link_get("launch:1").unwrap().unwrap();
    assert_eq!(ln.params.down, 75);
    assert_eq!(ln.identity.issue.as_deref(), Some("Acme/widgets#41"));
}

#[tokio::test]
async fn a_relaunched_issue_keeps_its_label_after_each_ticket_closes() {
    let h = H::new();
    h.fake.set_projects(vec![project("p-widgets", "/src/widgets")]);
    h.remotes.set("/src/widgets", "git@github.com:o/widgets.git");
    for _ in 0..3 {
        let r = result(&h.exec("async () => wf.launchTicket({ issue: 'o/widgets#7' })").await);
        assert_eq!(r["label"], "widgets#7", "{r}");
        let thread = r["thread"].as_str().unwrap().to_string();
        h.wf(&thread, "join", json!({"parent": BOARD, "label": "widgets#7", "role": "ticket"})).await;
        let done = h.wf(&thread, "send", json!({"kind": "handoff", "body": "done"})).await;
        assert_eq!(done["closed"], true, "{done}");
    }
}

#[tokio::test]
async fn launch_ticket_carries_the_parent_issue_and_part_to_the_ticket_link() {
    let h = H::new();
    h.fake.set_projects(vec![project("p-widgets", "/src/widgets")]);
    h.remotes.set("/src/widgets", "https://github.com/acme/widgets.git");
    let out = h
        .exec(
            "async () => wf.launchTicket({ issue: 'acme/widgets#115', \
             parentIssue: 'https://github.com/acme/widgets/issues/105', part: '1/4' })",
        )
        .await;
    assert_eq!(result(&out)["ok"], true, "{}", out.text);
    let msg = h.fake.launched()[0]["message"].as_str().unwrap().to_string();
    let call = msg.lines().find(|l| l.starts_with("async () =>")).expect("join call in brief");
    assert!(
        call.contains(r#"role: "ticket", issue: "acme/widgets#115", parentIssue: "acme/widgets#105", part: "1/4" })"#),
        "{call}"
    );
    let tree = h.app.wf.tree().unwrap();
    assert_eq!(
        (&tree[0]["issue"], &tree[0]["parentIssue"], &tree[0]["part"]),
        (&json!("acme/widgets#115"), &json!("acme/widgets#105"), &json!("1/4"))
    );
    h.exec_as("launch:1", call).await;
    let tree = h.app.wf.tree().unwrap();
    assert_eq!(tree[0]["joined"], true);
    assert_eq!(tree[0]["issue"], "acme/widgets#115");
    assert_eq!(tree[0]["parentIssue"], "acme/widgets#105");
    assert_eq!(tree[0]["part"], "1/4");
}

#[tokio::test]
async fn launch_ticket_refuses_a_bad_part_before_launching() {
    let h = H::new();
    h.fake.set_projects(vec![project("p-widgets", "/src/widgets")]);
    h.remotes.set("/src/widgets", "https://github.com/acme/widgets.git");
    let out = h.exec("async () => wf.launchTicket({ issue: 'acme/widgets#115', part: '5/4' })").await;
    assert!(err_text(&out).contains("part \"5/4\" must be n/N"), "{}", out.text);
    assert!(h.fake.launched().is_empty());
    assert!(h.app.store.pending_all().unwrap().is_empty());
}

fn dispatch_projects(h: &H) {
    let mut codemode = project("p-codemode", "/src/t3-codemode");
    codemode["title"] = json!("t3-codemode");
    h.fake.set_projects(vec![project("p-widgets", "/src/widgets"), codemode]);
    h.remotes.set("/src/t3-codemode", "git@github.com:Acme/t3-codemode.git");
}

#[tokio::test]
async fn dispatch_launches_the_request_in_the_named_project_and_it_joins_the_board() {
    let h = H::new();
    dispatch_projects(&h);
    let out = h
        .exec("async () => wf.dispatch({ project: 'T3-Codemode', request: 'Cut a new release for t3-codemode.', title: 'release' })")
        .await;
    let r = result(&out);
    assert_eq!(r["ok"], true);
    assert_eq!(r["label"], "t3-codemode: release");
    assert_eq!(r["projectId"], "p-codemode");
    assert_eq!(r["thread"], "launch:1");
    assert_eq!(r["modelSource"], "default");
    let l = &h.fake.launched()[0];
    assert_eq!(l["projectId"], "p-codemode");
    assert_eq!(l["title"], "t3-codemode: release");
    assert_eq!(l["modelSelection"]["model"], "claude-opus-5-5");
    let msg = l["message"].as_str().unwrap();
    assert!(msg.starts_with("Cut a new release for t3-codemode.\n\n## t3-codemode link\n"), "{msg}");
    assert!(msg.contains(r#"wf.join({ parent: "board", label: "t3-codemode: release", role: "ticket" })"#), "{msg}");
    assert!(!msg.contains("t3-ticket skill"), "{msg}");
    let pending = h.app.store.pending_all().unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!((pending[0].parent.as_str(), pending[0].label.as_str()), (BOARD, "t3-codemode: release"));
    assert_eq!(pending[0].thread.as_deref(), Some("launch:1"));
    h.wf("launch:1", "join", json!({"parent": BOARD, "label": "t3-codemode: release", "role": "ticket"})).await;
    assert!(h.app.store.pending_all().unwrap().is_empty());
    let b = h.wf(BOSS, "board", json!({"parent": BOARD})).await;
    assert_eq!(b["children"][0]["label"], "t3-codemode: release");
    assert_eq!(b["children"][0]["thread"], "launch:1");
}

#[tokio::test]
async fn dispatch_suffixes_a_label_already_on_the_board() {
    let h = H::new();
    dispatch_projects(&h);
    let go = "async () => wf.dispatch({ project: 'p-codemode', request: 'Cut a new release.', title: 'release' })";
    assert_eq!(result(&h.exec(go).await)["label"], "t3-codemode: release");
    assert_eq!(result(&h.exec(go).await)["label"], "t3-codemode: release (2)");
    h.wf("launch:1", "join", json!({"parent": BOARD, "label": "t3-codemode: release", "role": "ticket"})).await;
    h.wf("launch:1", "send", json!({"kind": "handoff", "body": "done"})).await;
    assert_eq!(result(&h.exec(go).await)["label"], "t3-codemode: release (3)");
    let l = &h.fake.launched()[2];
    assert!(l["message"].as_str().unwrap().contains(r#"label: "t3-codemode: release (3)""#));
}

#[tokio::test]
async fn dispatch_without_a_title_labels_by_the_request() {
    let h = H::new();
    dispatch_projects(&h);
    let r = result(
        &h.exec("async () => wf.dispatch({ project: 'p-widgets', request: '  Bump   the grader to v2 and open a PR for it  ' })")
            .await,
    );
    assert_eq!(r["label"], "p-widgets: Bump the grader to v2 and");
    assert!(
        h.fake.launched()[0]["message"]
            .as_str()
            .unwrap()
            .starts_with("  Bump   the grader to v2 and open a PR for it  \n\n")
    );
}

#[tokio::test]
async fn dispatch_to_an_unknown_project_launches_nothing() {
    let h = H::new();
    dispatch_projects(&h);
    let out = h.exec("async () => wf.dispatch({ project: 'gadgets', request: 'Cut a release.' })").await;
    assert!(err_text(&out).contains(r#"no T3 project with id or title "gadgets""#), "{}", out.text);
    let out = h.exec("async () => wf.dispatch({ project: 'p-widgets', request: '   ' })").await;
    assert!(err_text(&out).contains("request must not be empty"), "{}", out.text);
    assert!(h.fake.launched().is_empty());
    assert!(h.app.store.pending_all().unwrap().is_empty());
    assert!(cards(&h).is_empty());
}

#[tokio::test]
async fn dispatch_prefers_an_exact_project_id_over_a_title() {
    let h = H::new();
    let mut decoy = project("p-decoy", "/src/decoy");
    decoy["title"] = json!("p-widgets");
    h.fake.set_projects(vec![decoy, project("p-widgets", "/src/widgets")]);
    let out = h.exec("async () => wf.dispatch({ project: 'p-widgets', request: 'Cut a release.' })").await;
    assert_eq!(result(&out)["projectId"], "p-widgets");
    assert_eq!(h.fake.launched()[0]["projectId"], "p-widgets");
}

#[tokio::test]
async fn a_failed_dispatch_launch_frees_its_label_and_leaves_an_error_card() {
    let h = H::new();
    dispatch_projects(&h);
    h.fake.set_launch_fails(true);
    let out =
        h.exec("async () => wf.dispatch({ project: 'p-codemode', request: 'Cut a release.', title: 'release' })").await;
    assert!(err_text(&out).contains("Launching t3-codemode: release failed"), "{}", out.text);
    assert!(h.app.store.pending_all().unwrap().is_empty());
    let c = cards(&h);
    assert_eq!(c.len(), 1);
    assert_eq!(c[0]["kind"], "error");
    assert_eq!(c[0]["label"], "t3-codemode: release");
}

#[tokio::test]
async fn an_ask_that_dispatches_records_the_launched_thread() {
    let h = H::new();
    dispatch_projects(&h);
    h.app.wf.ask("Cut a new release for t3-codemode").await.unwrap();
    let r = result(
        &h.exec_as("launch:1", "async () => wf.dispatch({ project: 't3-codemode', request: 'Cut a new release for t3-codemode', title: 'release' })")
            .await,
    );
    assert_eq!(r["thread"], "launch:2");
    let asks = h.app.wf.asks().unwrap();
    assert_eq!(
        asks[0]["dispatched"],
        json!({"thread": "launch:2", "link": r["link"], "projectId": "p-codemode", "project": "t3-codemode", "label": "t3-codemode: release"})
    );
    result(&h.exec_as(WORKER, "async () => wf.dispatch({ project: 'p-widgets', request: 'Tidy up.' })").await);
    assert_eq!(h.app.wf.asks().unwrap().len(), 1);
}

#[tokio::test]
async fn an_ask_brief_lists_the_projects_and_sends_work_to_wf_dispatch() {
    let h = H::new();
    dispatch_projects(&h);
    let mut named = project("p-gadgets", "/src/gadgets");
    named["repositoryIdentity"] = json!({"owner": "Acme", "name": "gadgets"});
    let mut gone = project("p-old", "/src/old");
    gone["archivedAt"] = json!("2026-11-02T00:00:00Z");
    let mut codemode = project("p-codemode", "/src/t3-codemode");
    codemode["title"] = json!("t3-codemode");
    h.fake.set_projects(vec![project("p-widgets", "/src/widgets"), codemode, named, gone]);
    h.app.wf.ask("Cut a new release for t3-codemode").await.unwrap();
    let l = &h.fake.launched()[0];
    assert_eq!(l["scratch"], true);
    assert!(l.get("projectId").is_none());
    let msg = l["message"].as_str().unwrap();
    assert!(msg.contains("\n\nCut a new release for t3-codemode\n\n"), "{msg}");
    assert!(msg.contains(r#"- id "p-widgets", title "p-widgets", repository null"#), "{msg}");
    assert!(msg.contains(r#"- id "p-codemode", title "t3-codemode", repository "Acme/t3-codemode""#), "{msg}");
    assert!(msg.contains(r#"- id "p-gadgets", title "p-gadgets", repository "Acme/gadgets""#), "{msg}");
    assert!(!msg.contains("p-old"), "{msg}");
    assert!(msg.contains("wf.dispatch({ project: \"<project id>\""), "{msg}");
    assert!(msg.contains("do not do it in this thread"), "{msg}");
    assert!(msg.contains("or more than one could, launch nothing"), "{msg}");
    assert!(msg.contains("answer it read-only"), "{msg}");
    assert_eq!(h.fake.launched().len(), 1);
    assert!(h.app.store.pending_all().unwrap().is_empty());
}

#[tokio::test]
async fn an_ask_still_asks_when_t3_lists_no_projects() {
    let h = H::new();
    h.app.wf.ask("Which tickets wait on me?").await.unwrap();
    h.fake.set_projects_fail(true);
    h.app.wf.ask("Which tickets wait on me?").await.unwrap();
    let msgs: Vec<String> = h.fake.launched().iter().map(|l| l["message"].as_str().unwrap().to_string()).collect();
    assert_eq!(msgs.len(), 2);
    assert!(msgs[0].contains("T3 projects:\n- (none: T3 listed no projects)\n"), "{}", msgs[0]);
    assert!(
        msgs[1].contains("T3 projects:\n- (unavailable: T3 could not list its projects, so launch nothing"),
        "{}",
        msgs[1]
    );
    assert!(msgs[1].contains("answer it read-only"), "{}", msgs[1]);
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
async fn a_failed_project_lookup_leaves_an_error_card() {
    let h = H::new();
    h.fake.set_projects_fail(true);
    let out = h.exec("async () => wf.launchTicket({ issue: 'Acme/widgets#41' })").await;
    assert!(err_text(&out).contains("Cannot launch widgets#41: "), "{}", out.text);
    let c = cards(&h);
    assert_eq!(c.len(), 1);
    assert_eq!(c[0]["kind"], "error");
    assert_eq!(c[0]["label"], "widgets#41");
    assert!(h.fake.launched().is_empty());
    assert!(h.app.store.pending_all().unwrap().is_empty());
}

#[tokio::test]
async fn a_ticket_that_joins_before_its_launch_returns_is_not_reported() {
    let h = H::new();
    h.fake.set_projects(vec![project("p-widgets", "/src/widgets")]);
    h.remotes.set("/src/widgets", "git@github.com:o/widgets.git");
    h.wf("launch:1", "init", json!({"role": "ticket"})).await;
    h.exec("async () => wf.launchTicket({ issue: 'o/widgets#9', part: '2/3' })").await;
    let ln = h.app.store.link_get("launch:1").unwrap().unwrap();
    assert_eq!((ln.identity.issue.as_deref(), ln.identity.part.as_deref()), (Some("o/widgets#9"), Some("2/3")));
    let again = h.exec("async () => wf.launchTicket({ issue: 'o/widgets#9' })").await;
    assert!(err_text(&again).contains("widgets#9 is already on the board"), "{}", again.text);
    assert!(!h.tick(76).await.iter().any(|a| a.starts_with("report never-joined")));
    assert!(cards(&h).iter().all(|c| c["kind"] != "report"), "{:?}", cards(&h));
}

#[tokio::test]
async fn a_launched_ticket_keeps_its_issue_label_over_a_custom_one() {
    let h = H::new();
    h.fake.set_projects(vec![project("p-widgets", "/src/widgets")]);
    h.remotes.set("/src/widgets", "git@github.com:o/widgets.git");
    h.exec("async () => wf.launchTicket({ issue: 'o/widgets#9' })").await;
    let r = result(&h.exec_as("launch:1", "async () => wf.init({ role: 'ticket', label: 'Custom' })").await);
    assert_eq!(r["label"], "widgets#9", "{r}");
    let again = h.exec("async () => wf.launchTicket({ issue: 'o/widgets#9' })").await;
    assert!(err_text(&again).contains("widgets#9 is already on the board"), "{}", again.text);
}

#[tokio::test]
async fn a_ticket_that_finishes_before_its_launch_returns_is_not_reported() {
    let h = H::new();
    h.fake.set_projects(vec![project("p-widgets", "/src/widgets")]);
    h.remotes.set("/src/widgets", "git@github.com:o/widgets.git");
    h.wf("launch:1", "init", json!({"role": "ticket"})).await;
    h.wf("launch:1", "send", json!({"kind": "handoff", "body": "done"})).await;
    h.exec("async () => wf.launchTicket({ issue: 'o/widgets#9' })").await;
    assert!(!h.tick(76).await.iter().any(|a| a.starts_with("report never-joined")));
    assert!(cards(&h).iter().all(|c| c["kind"] != "report"), "{:?}", cards(&h));
}

#[tokio::test]
async fn a_launched_ticket_that_calls_init_clears_its_own_launch() {
    let h = H::new();
    h.fake.set_projects(vec![project("p-widgets", "/src/widgets")]);
    h.remotes.set("/src/widgets", "git@github.com:o/widgets.git");
    h.exec("async () => wf.launchTicket({ issue: 'o/widgets#9', title: 'Fix the grader' })").await;
    let r = result(&h.exec_as("launch:1", "async () => wf.init({ role: 'ticket' })").await);
    assert_eq!(r["parent"], BOARD, "{r}");
    assert_eq!(r["label"], "widgets#9", "{r}");
    let tree = h.app.wf.tree().unwrap();
    assert_eq!(tree.len(), 1, "{tree:?}");
    assert_eq!(tree[0]["joined"], true);
    assert!(!h.tick(76).await.iter().any(|a| a.starts_with("report never-joined")));
    assert!(cards(&h).iter().all(|c| c["kind"] != "report"), "{:?}", cards(&h));
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

pub(crate) struct Srv {
    pub(crate) base: String,
    pub(crate) http: reqwest::Client,
    pub(crate) h: H,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
}

impl Drop for Srv {
    fn drop(&mut self) {
        if let Some(s) = self.stop.take() {
            let _ = s.send(());
        }
    }
}

pub(crate) async fn start(h: H) -> Srv {
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
    pub(crate) async fn post(
        &self,
        path: &str,
        body: Value,
        csrf: Option<&str>,
        origin: Option<&str>,
    ) -> reqwest::Response {
        let mut req = self.http.post(format!("{}{path}", self.base)).json(&body);
        if let Some(c) = csrf {
            req = req.header("X-T3CM-CSRF", c);
        }
        if let Some(o) = origin {
            req = req.header("Origin", o);
        }
        req.send().await.unwrap()
    }

    async fn put(&self, path: &str, body: Value, csrf: Option<&str>) -> reqwest::Response {
        let mut req = self.http.put(format!("{}{path}", self.base)).json(&body);
        if let Some(c) = csrf {
            req = req.header("X-T3CM-CSRF", c);
        }
        req.send().await.unwrap()
    }

    pub(crate) fn csrf(&self) -> String {
        self.h.app.csrf.clone()
    }

    pub(crate) async fn get(&self, path: &str) -> Value {
        self.http.get(format!("{}{path}", self.base)).send().await.unwrap().json().await.unwrap()
    }

    async fn context(&self, method: reqwest::Method, id: &str, body: Value, csrf: Option<&str>) -> (u16, Value) {
        let url = format!("{}/api/contexts/{}", self.base, id.replace(' ', "%20"));
        let mut req = self.http.request(method, url).json(&body).header("Origin", &self.base);
        if let Some(c) = csrf {
            req = req.header("X-T3CM-CSRF", c);
        }
        let r = req.send().await.unwrap();
        (r.status().as_u16(), r.json().await.unwrap_or(Value::Null))
    }

    async fn restart(mut self) -> Srv {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        let mut h = std::mem::replace(&mut self.h, H::new());
        h.reopen();
        start(h).await
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
    let css = &lower[lower.find("<style>").unwrap()..lower.find("</style>").unwrap()];
    for (i, _) in css.match_indices("url(") {
        assert!(css[i..].starts_with("url(\"data:"), "external load: {}", &css[i..i + 40]);
    }
    for tag in ["<script", "<link", "<img", "<iframe", "@import"] {
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
    answered_ask(h, question, "all quiet").await
}

async fn answered_ask(h: &H, question: &str, answer: &str) -> String {
    let a = h.app.wf.ask(question).await.unwrap();
    let thread = a["thread"].as_str().unwrap().to_string();
    h.fake.update(&thread, json!({"status": "completed"}));
    h.fake.set_items(&thread, vec![json!({"type": "assistant_message", "position": 1, "text": answer})]);
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

#[tokio::test]
async fn links_stored_before_pausing_load_unpaused() {
    let h = H::new();
    join_board(&h, TICKET, "widgets#1").await;
    let ln = h.app.store.link_get(TICKET).unwrap().unwrap();
    let raw = serde_json::to_value(&ln).unwrap();
    assert!(raw.get("pausedAt").is_none(), "{raw}");
    let old: t3_codemode::store::Link = serde_json::from_value(raw).unwrap();
    assert_eq!(old.paused_at, None);
}

#[tokio::test]
async fn pause_and_resume_through_the_routes_show_in_state() {
    let s = start(H::new()).await;
    join_board(&s.h, TICKET, "widgets#41").await;
    s.h.wf(WORKER, "join", json!({"parent": TICKET, "label": "#41 PR 1/1", "role": "worker"})).await;
    let pause = format!("/api/links/{TICKET}/pause");
    assert_eq!(s.post(&pause, json!({}), None, None).await.status(), 403);
    assert_eq!(snapshot(&s.h)["tree"][0]["status"], "idle");

    assert_eq!(s.post(&pause, json!({}), Some(&s.csrf()), None).await.status(), 200);
    let worker_pause = format!("/api/links/{WORKER}/pause");
    assert_eq!(s.post(&worker_pause, json!({}), Some(&s.csrf()), None).await.status(), 200);
    let st = snapshot(&s.h);
    let t = &st["tree"][0];
    assert_eq!((t["status"].clone(), t["paused"].clone()), (json!("paused"), json!(true)));
    assert!(t["pausedAt"].is_string());
    assert_eq!(t["workers"][0]["status"], "paused");
    assert!(s.h.tick(600).await.is_empty());
    s.h.fake.update(TICKET, json!({"status": "running"}));
    assert!(s.h.tick(601).await.is_empty());
    assert_eq!(snapshot(&s.h)["tree"][0]["status"], "running");
    assert_eq!(snapshot(&s.h)["tree"][0]["paused"], true);

    s.h.fake.update(TICKET, json!({"status": "idle"}));
    assert!(s.h.tick(602).await.is_empty());
    let resume = format!("/api/links/{TICKET}/resume");
    assert_eq!(s.post(&resume, json!({}), Some(&s.csrf()), None).await.status(), 200);
    let t = snapshot(&s.h)["tree"][0].clone();
    assert_eq!(
        (t["status"].clone(), t["paused"].clone(), t["pausedAt"].clone()),
        (json!("idle"), json!(false), Value::Null)
    );
    assert_eq!(s.post("/api/links/mcp:nobody/pause", json!({}), Some(&s.csrf()), None).await.status(), 404);
}

#[tokio::test]
async fn close_idle_open_ticket_through_route_then_settle() {
    let s = start(H::new()).await;
    join_board(&s.h, TICKET, "widgets#41").await;
    s.h.wf(WORKER, "join", json!({"parent": TICKET, "label": "#41 PR 1/1", "role": "worker"})).await;
    // Reproduce #213: ticket raised a gate, the gate card was dismissed, ticket sits idle.
    let gate =
        s.h.wf(TICKET, "send", json!({"kind": "gate", "gate": "e2e", "body": "**ACTION NEEDED:** reply \"close\""}))
            .await;
    s.h.app.wf.dismiss(gate["card"].as_str().unwrap()).await.unwrap();

    let close = format!("/api/tickets/{TICKET}/close");
    assert_eq!(s.post(&close, json!({}), None, None).await.status(), 403);
    assert_eq!(s.post(&close, json!({}), Some("nope"), None).await.status(), 403);
    assert_eq!(s.post("/api/tickets/mcp:nobody/close", json!({}), Some(&s.csrf()), None).await.status(), 404);
    assert_eq!(s.post(&format!("/api/tickets/{WORKER}/close"), json!({}), Some(&s.csrf()), None).await.status(), 404);
    assert_eq!(s.post(&close, json!({"bogus": true}), Some(&s.csrf()), None).await.status(), 400);
    assert_eq!(s.post(&format!("/api/tickets/{TICKET}/settle"), json!({}), Some(&s.csrf()), None).await.status(), 400);

    let r = s.post(&close, json!({}), Some(&s.csrf()), Some(&s.base)).await;
    assert_eq!(r.status(), 200);
    let body: Value = r.json().await.unwrap();
    assert_eq!(body, json!({"thread": TICKET, "closed": true, "notified": false}));
    assert!(s.h.fake.sent().is_empty());

    let st = snapshot(&s.h);
    let t = &st["tree"][0];
    assert_eq!(
        (t["thread"].clone(), t["done"].clone(), t["status"].clone(), t["closedHow"].clone(), t["gate"].clone()),
        (json!(TICKET), json!(true), json!("done"), json!("closed-by-board"), Value::Null)
    );
    assert_eq!(st["counts"]["tickets"], 0);
    assert_eq!(s.post(&close, json!({}), Some(&s.csrf()), None).await.status(), 400);

    // Settle now accepts the closed ticket.
    let settled = s.post(&format!("/api/tickets/{TICKET}/settle"), json!({}), Some(&s.csrf()), None).await;
    assert_eq!(settled.status(), 200);
    let body: Value = settled.json().await.unwrap();
    assert_eq!((body["settled"].clone(), body["t3"]["ok"].clone()), (json!(true), json!(true)));
    let st = snapshot(&s.h);
    assert_eq!(st["tree"], json!([]));
    assert_eq!(st["settled"]["tickets"][0]["thread"], TICKET);

    // Closing a paused ticket with an open card resolves its card and clears paused.
    s.h.fake.put(thread("mcp:t2", "idle", T0));
    join_board(&s.h, "mcp:t2", "widgets#42").await;
    s.h.wf("mcp:t2", "send", json!({"kind": "report", "body": "stuck"})).await;
    assert_eq!(s.post("/api/links/mcp:t2/pause", json!({}), Some(&s.csrf()), None).await.status(), 200);
    assert_eq!(snapshot(&s.h)["cards"].as_array().unwrap().len(), 1);
    assert_eq!(s.post("/api/tickets/mcp:t2/close", json!({}), Some(&s.csrf()), None).await.status(), 200);
    let st = snapshot(&s.h);
    assert!(st["cards"].as_array().unwrap().is_empty());
    assert_eq!((st["tree"][0]["done"].clone(), st["tree"][0]["paused"].clone()), (json!(true), json!(false)));

    // A ticket that resumed running or waiting cannot be closed, leaving its link and cards intact.
    s.h.fake.put(thread("mcp:t3", "idle", T0));
    join_board(&s.h, "mcp:t3", "widgets#43").await;
    s.h.wf("mcp:t3", "send", json!({"kind": "report", "body": "check"})).await;
    s.h.fake.update("mcp:t3", json!({"status": "running"}));
    let running = s.post("/api/tickets/mcp:t3/close", json!({}), Some(&s.csrf()), None).await;
    assert_eq!(running.status(), 400);
    let err: Value = running.json().await.unwrap();
    assert!(err["error"].as_str().unwrap().contains("running"), "{err}");
    s.h.fake.update("mcp:t3", json!({"status": "waiting"}));
    let waiting = s.post("/api/tickets/mcp:t3/close", json!({}), Some(&s.csrf()), None).await;
    assert_eq!(waiting.status(), 400);
    let err: Value = waiting.json().await.unwrap();
    assert!(err["error"].as_str().unwrap().contains("waiting"), "{err}");
    let st = snapshot(&s.h);
    assert_eq!(st["cards"].as_array().unwrap().len(), 1);
    assert_eq!(st["tree"].as_array().unwrap().iter().find(|t| t["thread"] == "mcp:t3").unwrap()["done"], false);

    // Transient T3 read error returns 503 and leaves link and cards untouched.
    s.h.fake.update("mcp:t3", json!({"status": "idle"}));
    s.h.fake.set_read_fails(true);
    let unavailable = s.post("/api/tickets/mcp:t3/close", json!({}), Some(&s.csrf()), None).await;
    assert_eq!(unavailable.status(), 503);
    s.h.fake.set_read_fails(false);
    let st = snapshot(&s.h);
    assert_eq!(st["cards"].as_array().unwrap().len(), 1);
    assert_eq!(st["tree"].as_array().unwrap().iter().find(|t| t["thread"] == "mcp:t3").unwrap()["done"], false);

    // A confirmed missing (gone) thread can still be closed from the board.
    s.h.fake.remove("mcp:t3");
    let gone = s.post("/api/tickets/mcp:t3/close", json!({}), Some(&s.csrf()), None).await;
    assert_eq!(gone.status(), 200);
    let st = snapshot(&s.h);
    assert!(st["cards"].as_array().unwrap().is_empty());
    assert_eq!(st["tree"].as_array().unwrap().iter().find(|t| t["thread"] == "mcp:t3").unwrap()["done"], true);
}

#[tokio::test]
async fn close_ticket_optionally_notifies_the_thread() {
    let s = start(H::new()).await;
    join_board(&s.h, TICKET, "widgets#41").await;
    let r = s.post(&format!("/api/tickets/{TICKET}/close"), json!({"notify": true}), Some(&s.csrf()), None).await;
    assert_eq!(r.status(), 200);
    let body: Value = r.json().await.unwrap();
    assert_eq!(
        (body["closed"].clone(), body["notified"].clone(), body["t3"]["ok"].clone()),
        (json!(true), json!(true), json!(true))
    );
    let handle = body["handle"].as_str().unwrap();
    let sent = s.h.fake.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0]["threadId"], TICKET);
    let msg = sent[0]["message"].as_str().unwrap();
    assert!(msg.starts_with(&format!("[t3cm reply from=board msg={handle}]")), "{msg}");
    assert!(msg.contains("widgets#41 was closed from the board"), "{msg}");

    // A T3 send failure still closes the ticket and reports the error in t3.
    s.h.fake.put(thread("mcp:t2", "failed", T0));
    join_board(&s.h, "mcp:t2", "widgets#42").await;
    s.h.fake.set_send_fails(true);
    let r = s.post("/api/tickets/mcp:t2/close", json!({"notify": true}), Some(&s.csrf()), None).await;
    assert_eq!(r.status(), 200);
    let body: Value = r.json().await.unwrap();
    assert_eq!(
        (body["closed"].clone(), body["notified"].clone(), body["t3"]["ok"].clone()),
        (json!(true), json!(false), json!(false))
    );
    assert!(body["t3"]["error"].as_str().is_some_and(|e| !e.is_empty()), "{body}");
    assert_eq!(
        snapshot(&s.h)["tree"].as_array().unwrap().iter().find(|t| t["thread"] == "mcp:t2").unwrap()["done"],
        true
    );
}

#[tokio::test]
async fn close_ticket_refuses_when_a_newer_running_observation_overtakes_a_stale_idle_read() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use t3_codemode::app::App;
    use t3_codemode::clock::FakeClock;
    use t3_codemode::fake::{FakeDesktop, FakeGitHub, FakeRemotes, FakeT3, FakeTailscale};
    use t3_codemode::store::Store;
    use t3_codemode::t3::{BoxFut, T3Client, T3Error};

    struct BlockingPrListT3 {
        inner: Arc<FakeT3>,
        arm: AtomicBool,
        entered: Arc<tokio::sync::Notify>,
        release: Arc<tokio::sync::Notify>,
    }

    impl T3Client for BlockingPrListT3 {
        fn call<'a>(&'a self, tool: &'a str, args: Value) -> BoxFut<'a, Result<Value, T3Error>> {
            Box::pin(async move {
                if tool == "list_thread_pull_requests" && self.arm.swap(false, Ordering::AcqRel) {
                    self.entered.notify_one();
                    self.release.notified().await;
                }
                self.inner.call(tool, args).await
            })
        }

        fn list_tools(&self) -> BoxFut<'_, Result<Vec<Value>, T3Error>> {
            self.inner.list_tools()
        }
    }

    let dir = tempfile::tempdir().unwrap();
    let fake = Arc::new(FakeT3::new());
    fake.put(thread(TICKET, "idle", T0));
    fake.set_prs(
        TICKET,
        vec![json!({"number": 1, "url": "https://github.com/acme/widgets/pull/1", "state": "merged", "repository": "acme/widgets"})],
    );
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let t3 = Arc::new(BlockingPrListT3 {
        inner: fake.clone(),
        arm: AtomicBool::new(false),
        entered: entered.clone(),
        release: release.clone(),
    });
    let app = App::with_host_and_tailscale(
        Config::default(),
        Arc::new(Store::open(dir.path()).unwrap()),
        t3.clone(),
        Arc::new(FakeClock::at(T0)),
        Arc::new(FakeRemotes::default()),
        Arc::new(FakeGitHub::default()),
        Arc::new(FakeDesktop::default()),
        Arc::new(FakeTailscale::default()),
    );
    app.wf
        .dispatch("join", Some(TICKET), json!({"parent": "board", "label": "widgets#41", "role": "ticket"}))
        .await
        .unwrap();
    app.wf.dispatch("send", Some(TICKET), json!({"kind": "report", "body": "open report"})).await.unwrap();

    // Arm the pause on the close request's closed-PR lookup, after it has read "idle" from T3.
    fake.set_prs(
        TICKET,
        vec![
            json!({"number": 1, "url": "https://github.com/acme/widgets/pull/1", "state": "merged", "repository": "acme/widgets"}),
            json!({"number": 2, "url": "https://github.com/acme/widgets/pull/2", "state": "closed", "repository": "acme/widgets"}),
        ],
    );
    t3.arm.store(true, Ordering::Release);

    let wf_close = app.wf.clone();
    let close_task = tokio::spawn(async move { wf_close.close_ticket(TICKET, false).await });
    entered.notified().await;

    // A newer read observes the ticket running and publishes that status before the stale idle read finishes.
    fake.update(TICKET, json!({"status": "running"}));
    app.wf.board_view("board", false).await.unwrap();
    assert_eq!(app.wf.snapshot().unwrap()["tree"][0]["status"], "running");

    // Release the older read; it must not overwrite "running" or close the ticket.
    release.notify_one();
    let err = close_task.await.unwrap().expect_err("stale idle read must not close a running ticket");
    assert!(err.to_string().contains("running"), "{err:#}");

    let st = app.wf.snapshot().unwrap();
    assert_eq!((st["tree"][0]["done"].clone(), st["tree"][0]["status"].clone()), (json!(false), json!("running")));
    assert_eq!(st["cards"].as_array().unwrap().len(), 1);
}

fn run_page(html: &str, scenario: &str) -> Value {
    let open = html.find("<script>").unwrap() + "<script>".len();
    let close = html.rfind("</script>").unwrap();
    let src =
        format!("{}\n{}\nJSON.stringify(({scenario})())", include_str!("../support/board_dom.js"), &html[open..close]);
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
    run_page_async_with(html, "", scenario)
}

pub(crate) fn run_page_async_with(html: &str, prelude: &str, scenario: &str) -> Value {
    let open = html.find("<script>").unwrap() + "<script>".len();
    let close = html.rfind("</script>").unwrap();
    let src = format!(
        "{prelude}\n{}\n{}\nPromise.resolve(({scenario})()).then((v) => {{ globalThis.__out = JSON.stringify(v); }}, (e) => {{ globalThis.__out = JSON.stringify({{ error: String(e) }}); }});",
        include_str!("../support/board_dom.js"),
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
async fn the_page_is_a_fixed_shell_with_view_tabs_and_skip_links() {
    let (html, _) = two_gate_page().await;
    for needle in [
        "height: 100dvh",
        "html, body { height: 100%; margin: 0; overflow: hidden; }",
        "grid-template-rows: 56px auto minmax(0, 1fr);",
        ".views-main { grid-row: 3;",
        r#"role="tablist" aria-label="Board views""#,
        r#"id="vt-dispatch" aria-controls="view-dispatch""#,
        r#"id="vt-issues" aria-controls="view-issues""#,
        r#"id="vt-queue" aria-controls="view-queue""#,
        r#"id="vt-activity" aria-controls="view-activity""#,
        r#"role="tabpanel" aria-labelledby="vt-dispatch""#,
        r#"role="tablist" aria-label="Ticket lists""#,
        r#"id="tab-done""#,
        r#"id="tab-settled""#,
        r#"id="new-cards""#,
        r#"id="new-tix""#,
        r#"id="new-feed""#,
        r#"aria-live="polite""#,
        r#"role="alert""#,
        "Skip to Needs you",
        "Skip to Launch",
        "Skip to Ask",
        "Skip to Tickets",
        "Skip to Activity",
        r#"<aside id="composer" class="drawer cdr" role="dialog" aria-modal="true" aria-labelledby="composer-h""#,
        r#"<div class="scrim" id="scrim" hidden></div>"#,
        ".top .tools > * { flex-shrink: 0; }",
        ".top:has(.upd:not([hidden])) .vtab .long { display: none; }",
        ".top:has(.upd:not([hidden])) :is(.tools, .upd) > .btn .w { display: none; }",
        "@media (max-width: 940px) {",
    ] {
        assert!(html.contains(needle), "missing {needle}");
    }
    assert!(html.find("Skip to Needs you").unwrap() < html.find("Skip to Launch").unwrap());
    assert!(html.find("Skip to Ask").unwrap() < html.find("Skip to Tickets").unwrap());
    for banned in ["gradient", "btn primary", "backdrop-filter", "requestPermission();\n}, { once: true }"] {
        assert!(!html.contains(banned), "found {banned}");
    }
    for line in html.lines().filter(|l| l.contains("box-shadow")) {
        assert!(line.contains("box-shadow: none") || line.contains("box-shadow: var(--shadow-pop)"), "{line}");
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
async fn a_card_resolved_from_its_own_button_leaves_no_placeholder() {
    let (html, st) = two_gate_page().await;
    let r = page_scenario(
        &html,
        &st,
        r##"
const list = document.getElementById("cards");
emitState(base);
list.dispatch("pointerenter");
card("a1").querySelector('[data-opt="1"]').dispatch("click");
advance(5000);
const sent = answers().length;
emitState({ ...base, cards: [base.cards[1]] });
const held = { gone: card("a1").classList.contains("gone"), focus: document.activeElement === card("a2") };
advance(3999);
const kept = !!card("a1");
advance(1);
return { sent, held, kept, after: ids(), focus: document.activeElement === card("a2") };
"##,
    );
    assert_eq!(r["sent"], 1, "{r}");
    assert_eq!(r["held"], json!({"gone": true, "focus": true}), "{r}");
    assert_eq!(r["kept"], true, "{r}");
    assert_eq!(r["after"], json!(["a2"]), "{r}");
    assert_eq!(r["focus"], true, "{r}");
}

#[tokio::test]
async fn the_last_card_resolved_from_its_button_hands_focus_to_the_list() {
    let (html, st) = two_gate_page().await;
    let r = page_scenario(
        &html,
        &st,
        r##"
const list = document.getElementById("cards");
emitState({ ...base, cards: [base.cards[0]] });
list.dispatch("pointerenter");
card("a1").querySelector('[data-opt="1"]').dispatch("click");
advance(5000);
emitState({ ...base, cards: [] });
const focus = document.activeElement === list;
advance(4000);
return { focus, after: ids(), empty: !document.getElementById("cards-empty").hidden };
"##,
    );
    assert_eq!(r, json!({"focus": true, "after": [], "empty": true}), "{r}");
}

#[tokio::test]
async fn a_placeholder_expires_while_the_panel_is_busy_but_not_while_focused() {
    let (html, st) = two_gate_page().await;
    let r = page_scenario(
        &html,
        &st,
        r##"
const list = document.getElementById("cards");
emitState(base);
list.dispatch("pointerenter");
card("a2").querySelector("input").focus();
emitState({ ...base, cards: [base.cards[1]] });
const placed = card("a1").classList.contains("gone");
advance(4000);
const expired = ids();
emitState(base);
document.getElementById("new-cards").dispatch("click");
card("a2").querySelector("input").focus();
emitState({ ...base, cards: [base.cards[1]] });
card("a1").focus();
advance(8000);
const focused = ids();
card("a2").querySelector("input").focus();
advance(4000);
return { placed, expired, focused, after: ids() };
"##,
    );
    assert_eq!(r, json!({"placed": true, "expired": ["a2"], "focused": ["a1", "a2"], "after": ["a2"]}), "{r}");
}

#[tokio::test]
async fn a_held_card_leaves_once_focus_moves_to_another_card() {
    let (html, st) = two_gate_page().await;
    let r = page_scenario(
        &html,
        &st,
        r##"
const list = document.getElementById("cards");
emitState(base);
list.dispatch("pointerenter");
const input = card("a1").querySelector("input");
input.focus();
emitState({ ...base, cards: [base.cards[1]] });
const held = card("a1").classList.contains("stale-item");
card("a2").querySelector("input").focus();
input.dispatch("focusout");
advance(0);
const gone = card("a1").classList.contains("gone");
advance(4000);
return { held, gone, after: ids() };
"##,
    );
    assert_eq!(r, json!({"held": true, "gone": true, "after": ["a2"]}), "{r}");
}

#[tokio::test]
async fn placeholders_are_swept_when_the_page_returns_or_the_stream_reconnects() {
    let (html, st) = two_gate_page().await;
    let r = page_scenario(
        &html,
        &st,
        r##"
const list = document.getElementById("cards");
const open = document.getElementById("open");
const tombs = () => document.querySelectorAll(".gone[data-key]").length;
emitState(base);
list.dispatch("pointerenter");
open.dispatch("pointerenter");
card("a2").querySelector("input").focus();
emitState({ ...base, cards: [base.cards[1]], tree: [base.tree[1]] });
const before = tombs();
document.hidden = true;
for (const f of document.listeners.visibilitychange) f();
const hidden = tombs();
document.hidden = false;
for (const f of document.listeners.visibilitychange) f();
const shown = tombs();
emitState(base);
document.getElementById("new-cards").dispatch("click");
card("a2").querySelector("input").focus();
emitState({ ...base, cards: [base.cards[1]] });
const again = tombs();
eventSource.onerror();
advance(1000);
emitState({ ...base, cards: [base.cards[1]] });
return { before, hidden, shown, again, reconnected: tombs(), order: ids() };
"##,
    );
    assert_eq!(r, json!({"before": 2, "hidden": 2, "shown": 0, "again": 1, "reconnected": 0, "order": ["a2"]}), "{r}");
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
const fallback = document.activeElement === document.getElementById("issues");
const sa = document.createElement("button");
document.getElementById("p-arch").append(sa);
await act(sa, "/api/settle-done");
const arch = [document.getElementById("arch-err").hidden, document.getElementById("arch-err-text").textContent, document.getElementById("act-err-text").textContent];
return { shown, back, unfocusable, hidden: err.hidden, fallback, arch };
"##
        ),
    );
    assert_eq!(r["shown"], true, "{r}");
    assert_eq!(r["back"], true, "{r}");
    assert_eq!(r["unfocusable"], true, "{r}");
    assert_eq!(r["hidden"], true, "{r}");
    assert_eq!(r["fallback"], true, "{r}");
    assert_eq!(r["arch"], json!([false, "settle failed", ""]), "{r}");
}

#[tokio::test]
async fn a_merge_gate_reads_as_its_pull_request_and_title_whatever_its_options() {
    let (html, st) = two_gate_page().await;
    let r = page_scenario(
        &html,
        &st,
        r##"
const c0 = base.cards[0];
const pr = { number: 81, url: "https://github.com/acme/widgets/pull/81", state: "open" };
const tree = base.tree.map((t) => t.thread === c0.ticket ? { ...t, humanTitle: "Fix the grader", pr } : t);
const ask = (opts) => {
  emitState({ ...base, tree, cards: [{ ...c0, options: opts }] });
  return card(c0.id).querySelector('[data-part="ask"]').textContent;
};
return { two: ask(["merge", "merge and deploy"]), one: ask(["merge"]), plain: (emitState({ ...base, cards: [c0] }), card(c0.id).querySelector('[data-part="ask"]').textContent) };
"##,
    );
    assert_eq!(r["two"], "Merge PR #81: Fix the grader", "{r}");
    assert_eq!(r["one"], "Merge PR #81: Fix the grader", "{r}");
    assert!(r["plain"].as_str().unwrap().contains("merge #81"), "{r}");
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
q("#open .tk").focus();
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
async fn the_archive_search_filters_done_and_settled_together() {
    let (html, st) = two_gate_page().await;
    let r = page_scenario(
        &html,
        &st,
        &format!(
            r##"{ROWS}
const tree = [T("acme/widgets#7", {{ done: true, status: "done" }}), T("acme/gadgets#8", {{ done: true, status: "done" }})];
const settled = {{ tickets: [{{ thread: "th-9", label: "acme/gadgets#9", settledAt: "2026-01-01T00:00:00Z" }}], asks: [] }};
emitState({{ ...base, cards: [], tree, settled }});
const keys = (id) => [...document.getElementById(id).children].filter((e) => e.dataset.key && !e.classList.contains("gone")).map((e) => e.dataset.key);
const before = {{ done: keys("done"), settled: keys("settled") }};
const find = document.getElementById("settled-filter");
find.value = "gadgets";
find.dispatch("input");
const found = {{ done: keys("done"), settled: keys("settled") }};
find.value = "nothing";
find.dispatch("input");
const none = {{ done: keys("done"), empty: document.getElementById("done-empty").textContent }};
return {{ before, found, none }};
"##
        ),
    );
    assert_eq!(r["before"], json!({"done": ["t-acme/widgets#7", "t-acme/gadgets#8"], "settled": ["t:th-9"]}), "{r}");
    assert_eq!(r["found"], json!({"done": ["t-acme/gadgets#8"], "settled": ["t:th-9"]}), "{r}");
    assert_eq!(r["none"], json!({"done": [], "empty": "No finished tickets match the search."}), "{r}");
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
q("#open .tk").focus();
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
    assert_eq!(r["classes"], json!(["btn answer"]), "{r}");
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
q("#open").children.find((el) => el.dataset.key && el.dataset.thread !== T).focus();
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
    assert_eq!(r["note"], "Build green", "{r}");
    assert_eq!(r["placeholder"], "widgets#41 finished and moved to Done", "{r}");
    assert_eq!(r["inDone"], true, "{r}");
    assert_eq!(r["settle"], "Settle widgets#41", "{r}");
    assert_eq!(r["doneCount"], "1", "{r}");
}

#[tokio::test]
async fn ticket_and_worker_rows_pause_and_resume_from_the_board() {
    let s = start(H::new()).await;
    join_board(&s.h, TICKET, "widgets#41").await;
    s.h.wf(WORKER, "join", json!({"parent": TICKET, "label": "#41 PR 1/1", "role": "worker"})).await;
    let html = s.http.get(format!("{}/", s.base)).send().await.unwrap().text().await.unwrap();
    let st: Value = s.http.get(format!("{}/api/state", s.base)).send().await.unwrap().json().await.unwrap();
    let r = run_page_async(
        &html,
        &format!(
            r##"async () => {{
{PAGE_HELPERS}
const base = {st};
fetchReply = async () => ({{ ok: true, json: async () => ({{}}) }});
const T = "{TICKET}";
const tk = () => q('#open [data-thread="' + T + '"]');
const side = () => tk().querySelector(".side");
const wi = () => tk().querySelector(".wi");
const look = () => ({{
  btn: side().querySelector('[data-part="pause"]').textContent,
  label: side().querySelector('[data-part="pause"]').getAttribute("aria-label"),
  pill: side().querySelector('[data-part="paused"]').hidden ? null : side().querySelector('[data-part="paused"]').textContent,
  wbtn: wi().querySelector('[data-part="wpause"]').textContent,
  wpill: !wi().querySelector('[data-part="wpaused"]').hidden,
  aria: tk().getAttribute("aria-label"),
  wtitle: wi().getAttribute("title"),
  group: q("#open .tgroup") && q("#open .tgroup").textContent,
}});
emitState(base);
const before = look();
side().querySelector('[data-part="pause"]').dispatch("click");
const flush = async () => {{ for (let i = 0; i < 20; i++) await Promise.resolve(); }};
await flush();
wi().querySelector('[data-part="wpause"]').dispatch("click");
await flush();
const paused = base.tree.map((t) => ({{ ...t, status: "paused", paused: true, workers: t.workers.map((w) => ({{ ...w, status: "paused", paused: true }})) }}));
emitState({{ ...base, tree: paused }});
const after = look();
side().querySelector('[data-part="pause"]').dispatch("click");
await flush();
return {{ before, after, posts: fetches.filter((f) => f.method === "POST").map((f) => f.url) }};
}}"##
        ),
    );
    assert_eq!(
        r["before"],
        json!({"btn": "Pause", "label": "Pause widgets#41", "pill": null, "wbtn": "Pause", "wpill": false,
            "aria": "widgets#41, idle", "wtitle": "#41 PR 1/1, idle", "group": "Idle1"}),
        "{r}"
    );
    assert_eq!(
        r["after"],
        json!({"btn": "Resume", "label": "Resume widgets#41", "pill": "paused", "wbtn": "Resume", "wpill": true,
            "aria": "widgets#41, paused", "wtitle": "#41 PR 1/1, paused", "group": "Paused1"}),
        "{r}"
    );
    assert_eq!(
        r["posts"],
        json!([
            format!("/api/links/{}/pause", TICKET.replace(':', "%3A")),
            format!("/api/links/{}/pause", WORKER.replace(':', "%3A")),
            format!("/api/links/{}/resume", TICKET.replace(':', "%3A")),
        ]),
        "{r}"
    );
}

#[tokio::test]
async fn ticket_rows_show_close_on_idle_paused_and_failed_open_tickets() {
    let s = start(H::new()).await;
    join_board(&s.h, TICKET, "widgets#41").await;
    let html = s.http.get(format!("{}/", s.base)).send().await.unwrap().text().await.unwrap();
    let st: Value = s.http.get(format!("{}/api/state", s.base)).send().await.unwrap().json().await.unwrap();
    let r = run_page_async(
        &html,
        &format!(
            r##"async () => {{
{PAGE_HELPERS}
const base = {st};
fetchReply = async () => ({{ ok: true, json: async () => ({{}}) }});
const T = "{TICKET}";
const mk = (id, label, extra) => ({{ ...base.tree[0], ticket: id, thread: id, label, workers: [], ...extra }});
const tree = [
  mk("mcp:idle", "widgets#1", {{ status: "idle", paused: false, done: false, joined: true }}),
  mk("mcp:pause", "widgets#2", {{ status: "paused", paused: true, done: false, joined: true }}),
  mk("mcp:fail", "widgets#3", {{ status: "failed", paused: false, done: false, joined: true }}),
  mk("mcp:run", "widgets#4", {{ status: "running", paused: false, done: false, joined: true }}),
  mk("mcp:wait", "widgets#5", {{ status: "waiting", paused: false, done: false, joined: true }}),
  mk("mcp:paused-report", "widgets#6", {{ status: "paused", paused: true, done: false, joined: true }}),
  mk("mcp:run-fail-worker", "widgets#9", {{
    status: "running", paused: false, done: false, joined: true,
    workers: [{{ thread: "mcp:w1", label: "#9 PR 1/1", status: "failed", paused: false, done: false, joined: true }}],
  }}),
  mk(null, "widgets#7", {{ status: "launching", paused: false, done: false, joined: false }}),
  mk("mcp:done", "widgets#8", {{ status: "done", paused: false, done: true, joined: true }}),
];
const cards = [{{ id: "a1", kind: "report", label: "widgets#6", ticket: "mcp:paused-report", text: "Stuck report", options: [], at: "2026-04-16T10:00:00Z", atLocal: "10:00 MST", token: "1" }}];
emitState({{ ...base, tree, cards }});
const closeVisible = (list, key) => {{
  const el = q("#" + list + ' [data-key="' + key + '"]');
  const b = el && el.querySelector('[data-part="close"]');
  return !!(b && !b.hidden);
}};
const shown = {{
  idle: closeVisible("open", "mcp:idle"),
  pause: closeVisible("open", "mcp:pause"),
  fail: closeVisible("open", "mcp:fail"),
  run: closeVisible("open", "mcp:run"),
  wait: closeVisible("open", "mcp:wait"),
  pausedReport: closeVisible("open", "mcp:paused-report"),
  runFailWorker: closeVisible("open", "mcp:run-fail-worker"),
  pending: closeVisible("open", "pending:widgets#7"),
  done: closeVisible("done", "mcp:done"),
}};
const idleBtn = q('#open [data-key="mcp:idle"] [data-part="close"]');
const idleLabel = idleBtn.getAttribute("aria-label");
idleBtn.dispatch("click");
const flush = async () => {{ for (let i = 0; i < 20; i++) await Promise.resolve(); }};
await flush();
// Transitioning idle -> running hides Close; back to idle shows it again; closing moves it to Done with Settle.
emitState({{ ...base, cards, tree: tree.map((t) => t.thread === "mcp:idle" ? {{ ...t, status: "running" }} : t) }});
const whileRunning = closeVisible("open", "mcp:idle");
emitState({{ ...base, cards, tree: tree.map((t) => t.thread === "mcp:idle" ? {{ ...t, status: "idle" }} : t) }});
const backToIdle = closeVisible("open", "mcp:idle");
emitState({{ ...base, cards, tree: tree.map((t) => t.thread === "mcp:idle" ? {{ ...t, status: "done", done: true }} : t) }});
const inDoneSettle = !!q('#done [data-key="mcp:idle"] [data-part="settle"]');
const inDoneClose = closeVisible("done", "mcp:idle");
return {{
  shown, idleLabel, whileRunning, backToIdle, inDoneSettle, inDoneClose,
  posts: fetches.filter((f) => f.method === "POST").map((f) => f.url),
}};
}}"##
        ),
    );
    assert_eq!(
        r["shown"],
        json!({"idle": true, "pause": true, "fail": true, "run": false, "wait": false, "pausedReport": true, "runFailWorker": false, "pending": false, "done": false}),
        "{r}"
    );
    assert_eq!(r["idleLabel"], "Close widgets#1", "{r}");
    assert_eq!(r["whileRunning"], false, "{r}");
    assert_eq!(r["backToIdle"], true, "{r}");
    assert_eq!(r["inDoneSettle"], true, "{r}");
    assert_eq!(r["inDoneClose"], false, "{r}");
    assert_eq!(r["posts"], json!(["/api/tickets/mcp%3Aidle/close"]), "{r}");
}

#[tokio::test]
async fn state_carries_the_flow_cap_only_when_a_flow_is_configured() {
    assert_eq!(snapshot(&H::new())["flowMaxTickets"], Value::Null);
    let cfg = Config::from_value(
        json!({"flow": {"repos": ["acme/widgets"], "optIn": {"label": "flow"}, "maxTickets": 3, "launch": "off"}}),
    )
    .unwrap();
    assert_eq!(snapshot(&H::build(Vec::new(), cfg))["flowMaxTickets"], 3);
}

#[tokio::test]
async fn contexts_save_the_execution_statuses() {
    use reqwest::Method;
    let s = start(H::new()).await;
    let spec = json!({"statuses": ["external", "alert", "pr-review", "paused", "ready", "pr-green", "running"]});
    let (code, r) = s.context(Method::PUT, "c1", json!({"name": "Shipping", "spec": spec}), Some(&s.csrf())).await;
    assert_eq!(code, 201, "{r}");
    assert_eq!(
        r["context"]["spec"]["statuses"],
        json!(["running", "paused", "pr-green", "pr-review", "alert", "ready", "external"]),
        "{r}"
    );
}

const PR_TREE: &str = r##"
const pr = (n, extra) => ({ number: n, url: "https://github.com/acme/a/pull/" + n, state: "open", ...extra });
const ready = (o) => ({ isDraft: false, mergeStateStatus: "CLEAN", reviewDecision: "APPROVED", checks: "passing", ...o });
const tree = [
  T("acme/a#1", { workers: [W("#1 PR 1/1", "running")] }),
  T("acme/a#2", { pr: pr(2, { readiness: ready() }) }),
  T("acme/a#3", { pr: pr(3, { state: "queued" }) }),
  T("acme/a#4", { workers: [W("#4 PR 1/1", "idle", { pr: pr(4, { readiness: ready({ checks: "pending", reviewDecision: "REVIEW_REQUIRED", mergeStateStatus: "BLOCKED" }) }) })] }),
  T("acme/a#5", { status: "paused", paused: true }),
  T("acme/a#6", { status: "failed" }),
  T("acme/a#7", { pr: pr(7, { readiness: ready({ checks: "failing", mergeStateStatus: "UNSTABLE" }) }) }),
  T("acme/a#8", { pr: pr(8) }),
  T("acme/a#9", { status: "running", paused: true, pr: pr(90, { readiness: ready({ checks: "failing" }) }), workers: [W("#9 PR 1/2", "idle", { pr: pr(90, { readiness: ready({ checks: "failing" }) }) }), W("#9 PR 2/2", "idle", { pr: pr(91, { readiness: ready() }) })] }),
];
const open = () => [...document.getElementById("open").children].filter((e) => e.dataset.thread && !e.classList.contains("gone")).map((e) => e.dataset.thread.slice(2));
const statusBtn = () => q('[data-facet="statuses"] [data-part="fbtn"]').textContent;
"##;

#[tokio::test]
async fn the_route_marks_merge_gates_only_and_keeps_unmerged_finished_workers_grey() {
    let (html, st) = two_gate_page().await;
    let r = page_scenario(
        &html,
        &st,
        &format!(
            r##"{ROWS}
{PR_TREE}
const step = (n) => ({{ n, of: 4 }});
const rt = [T("acme/r#1", {{ progress: {{ step: 3, of: 4, merged: 1 }}, workers: [
  W("#1 PR 1/4", "done", {{ step: step(1), pr: pr(11, {{ state: "merged" }}) }}),
  W("#1 PR 2/4", "done", {{ step: step(2) }}),
  W("#1 PR 3/4", "idle", {{ step: step(3), pr: pr(13) }}),
  W("#1 PR 4/4", "idle", {{ step: step(4), pr: pr(14) }}),
] }}), T("acme/q#2", {{ workers: [
  W("#2 PR 1/2", "idle", {{ step: {{ n: 1, of: 2 }}, pr: pr(13) }}),
  W("#2 PR 2/2", "idle", {{ step: {{ n: 2, of: 2 }}, pr: {{ number: 13, url: "https://github.com/acme/other/pull/13", state: "open" }} }}),
] }})];
const gate = (id, text) => ({{ ...base.cards[0], id, kind: "gate", ticket: "t-acme/r#1", label: "acme/r#1", text }});
emitState({{ ...base, tree: rt, cards: [
  gate("g1", '**ACTION NEEDED:** reply "merge" to merge #13, here: https://github.com/acme/a/pull/13'),
  gate("g2", "**ACTION NEEDED:** decide each open review finding on #14"),
  {{ ...gate("g3", '**ACTION NEEDED:** reply "merge" to merge #13, here: https://github.com/acme/other/pull/13'), ticket: "t-acme/q#2", label: "acme/q#2" }},
] }});
const st = (label) => [...row(label).querySelectorAll(".station")].map((b) => [b.children[1].textContent, b.children[0].className, b.children[2].textContent, b.getAttribute("data-seg-r")]);
return {{ r: st("acme/r#1"), q: st("acme/q#2").slice(0, 2).map((x) => x[2]) }};
"##
        ),
    );
    let st = &r["r"];
    assert_eq!(st[0], json!(["#11", "st-dot merged", "merged", null]), "{r}");
    assert_eq!((&st[1][1], &st[1][2], &st[1][3]), (&json!("st-dot done"), &json!("done"), &Value::Null), "{r}");
    assert_eq!(st[2], json!(["#13", "st-dot gate", "merge gate", null]), "{r}");
    assert_eq!(st[3][0], "#14", "{r}");
    assert_ne!(st[3][2], "merge gate", "{r}");
    assert_eq!(r["q"], json!(["open", "merge gate"]), "{r}");
}

#[tokio::test]
async fn pr_rows_match_worker_titles_done_rows_wait_behind_a_count_and_ask_flags_an_answer() {
    let (html, st) = two_gate_page().await;
    let r = page_scenario(
        &html,
        &st,
        &format!(
            r##"{ROWS}
{PR_TREE}
const rt = [
  T("acme/s#1", {{ title: "s#1: Release tooling", humanTitle: "Release tooling", workers: [W("#1 PR 1/2", "running", {{ issueTitle: "Retry indexing", pr: pr(5) }})] }}),
  T("acme/z#2", {{ title: "z#2: Other work", workers: [W("#2 PR 1/1", "running", {{ pr: pr(6) }})] }}),
  T("acme/d#3", {{ status: "done", done: true }}),
];
emitState({{ ...base, cards: [], tree: rt }});
setFilter({{ q: "indexing" }});
const keys = [...document.getElementById("prs").children].filter((e) => !e.classList.contains("gone")).map((e) => e.dataset.key);
setFilter({{}});
selectView("activity");
selectTab("done");
q("#done .tk").focus();
emitState({{ ...base, cards: [], tree: [...rt, T("acme/d#4", {{ status: "done", done: true }})] }});
const nd = q("#new-done");
const held = [nd.classList.contains("off"), nd.textContent, q('#done [data-thread="t-acme/d#4"]') ? true : false];
nd.dispatch("click");
const shown = !!q('#done [data-thread="t-acme/d#4"]');
const ask = (status) => emitState({{ ...base, cards: [], tree: rt, asks: [{{ id: "q1", seq: 1, question: "Which tickets wait?", status, answer: status === "answered" ? "None" : null }}] }});
ask("running");
ask("answered");
const dot = document.getElementById("ask-open").getAttribute("data-unread");
openComposer("ask");
return {{ keys, held, shown, dot, cleared: document.getElementById("ask-open").getAttribute("data-unread") }};
"##
        ),
    );
    assert_eq!(r["keys"], json!(["pr:https://github.com/acme/a/pull/5"]), "{r}");
    assert_eq!(r["held"], json!([false, "1 new", false]), "{r}");
    assert_eq!(r["shown"], true, "{r}");
    assert_eq!(r["dot"], "", "{r}");
    assert_eq!(r["cleared"], Value::Null, "{r}");
}

#[tokio::test]
async fn rows_and_pills_show_pr_readiness_and_the_pause_tone() {
    let (html, st) = two_gate_page().await;
    let r = page_scenario(
        &html,
        &st,
        &format!(
            r##"{ROWS}
{PR_TREE}
emitState({{ ...base, cards: [], tree }});
const pill = (label, sel) => {{
  const p = row(label).querySelector(sel || ".side .pr .spill");
  return [p.textContent, p.className.split(" ").find((c) => c.startsWith("s-")), p.parentNode.getAttribute("aria-label")];
}};
return {{
  green: pill("acme/a#2"), queued: pill("acme/a#3"), worker: pill("acme/a#4", ".wi .spill"),
  alert: pill("acme/a#7"), plain: pill("acme/a#8"), shownWorker: pill("acme/a#9", ".wi .prl .spill"),
  paused: row("acme/a#5").querySelector(".mk .mark").className,
}};
"##
        ),
    );
    assert_eq!(r["green"], json!(["#2 ready", "s-pr-green", "PR #2 ready to merge for acme/a#2"]), "{r}");
    assert_eq!(r["queued"], json!(["#3 queued", "s-pr-green", "PR #3 ready to merge for acme/a#3"]), "{r}");
    assert_eq!(r["worker"], json!(["#4", "s-pr-review", "PR #4 CI for #4 PR 1/1"]), "{r}");
    assert_eq!(r["alert"], json!(["#7 failing", "s-alert", "PR #7 approved with failing checks for acme/a#7"]), "{r}");
    assert_eq!(r["plain"], json!(["#8 open", "s-pr-open", "PR #8 open for acme/a#8"]), "{r}");
    assert_eq!(r["paused"], "mark s-pause", "{r}");
    assert_eq!(
        r["shownWorker"],
        json!(["#91", "s-pr-green", "PR #91 ready to merge for #9 PR 2/2"]),
        "of two idle workers the row shows the one with a PR the ticket row does not already show: {r}"
    );
    let css =
        |sel: &str| html.lines().find(|l| l.starts_with(sel)).unwrap_or_else(|| panic!("no rule {sel}")).to_string();
    assert!(css(".s-pause {").contains("var(--pause)"), "paused has its own tone");
    assert!(!css(".s-idle {").contains("s-pause"), "idle no longer shares its rule with paused");
}

const DRAWER: &str = r##"
const dr = () => document.getElementById("drawer");
const dp = (name) => dr().querySelector('[data-part="' + name + '"]');
const shown = (name) => dp(name).hidden ? null : dp(name).textContent;
const flush = async () => { for (let i = 0; i < 20; i++) await Promise.resolve(); };
const click = (el) => {
  const ev = { type: "click", target: el, prevented: false, preventDefault() { this.prevented = true; } };
  for (const f of document.listeners.click) f(ev);
  return ev.prevented;
};
const press = (el, key) => {
  const ev = el.dispatch("keydown", { key, stopPropagation() { this.stopped = true; } });
  if (!ev.stopped) for (const f of document.listeners.keydown) f(ev);
};
const view = () => ({
  open: dr().classList.contains("open"), id: dp("id").textContent, part: shown("part"), title: dp("title").textContent,
  st: dp("st").textContent, pr: dp("pr").textContent, issue: dp("gh-issue").hidden ? null : dp("gh-issue").getAttribute("href"),
  ghpr: shown("gh-pr"), pause: shown("pause"), focus: document.activeElement === dp("title"),
});
const inspected = () => [...document.querySelectorAll("[data-inspected]")].filter((e) => !dr().contains(e)).map((e) => e.dataset.thread || e.dataset.key);
"##;

fn drawer_scenario(html: &str, st: &Value, body: &str) -> Value {
    run_page_async(
        html,
        &format!("async () => {{\n{PAGE_HELPERS}\nconst base = {st};\n{ROWS}\n{PR_TREE}\n{DRAWER}\n{body}\n}}"),
    )
}

#[tokio::test]
async fn the_inspector_opens_from_rows_workers_and_pr_chips_and_esc_returns_focus() {
    let (html, st) = two_gate_page().await;
    let r = drawer_scenario(
        &html,
        &st,
        r##"
emitState({ ...base, cards: [], tree });
const list = document.getElementById("open");
list.scrollTop = 120;
const before = dr().classList.contains("open");
const fromRow = click(row("acme/a#2").querySelector('[data-part="title"]'));
const ticket = { ...view(), prevented: fromRow, marked: inspected(), scroll: list.scrollTop };
dp("body").scrollTop = 40;
emitState({ ...base, cards: [], tree: tree.map((t) => t.label === "acme/a#2" ? { ...t, status: "running" } : t) });
const live = { open: dr().classList.contains("open"), st: dp("st").textContent, body: dp("body").scrollTop, scroll: list.scrollTop };
press(document.activeElement, "Escape");
const closed = { open: dr().classList.contains("open"), back: document.activeElement === row("acme/a#2"), marked: inspected() };
const wi = row("acme/a#9").querySelector(".wi");
click(wi.querySelector(".mark"));
const worker = { ...view(), key: wi.dataset.key, rows: [...dp("workers-l").children].map((w) => [w.querySelector('[data-part="name"]').textContent, w.hasAttribute("data-inspected")]) };
const chip = row("acme/a#7").querySelector(".side .pr a");
const fromChip = click(chip);
const pill = { ...view(), prevented: fromChip };
press(document.activeElement, "Escape");
const chipBack = document.activeElement === chip;
row("acme/a#5").focus();
press(row("acme/a#5"), "Enter");
const entered = view();
dp("pause").dispatch("click");
await flush();
press(document.activeElement, "Escape");
return { before, ticket, live, closed, worker, pill, chipBack, entered, back: document.activeElement === row("acme/a#5"),
  posts: fetches.filter((f) => f.method === "POST").map((f) => f.url) };
"##,
    );
    assert_eq!(r["before"], false, "{r}");
    assert_eq!(
        r["ticket"],
        json!({"open": true, "id": "acme/a#2", "part": null, "title": "acme/a#2", "st": "idle", "pr": "#2 ready",
               "issue": "https://github.com/acme/a/issues/2", "ghpr": "Open PR #2 on GitHub", "pause": "Pause", "focus": true,
               "prevented": false, "marked": ["t-acme/a#2", "t-acme/a#2"], "scroll": 120}),
        "{r}"
    );
    assert_eq!(
        r["live"],
        json!({"open": true, "st": "running", "body": 40, "scroll": 120}),
        "a re-render keeps the drawer: {r}"
    );
    assert_eq!(r["closed"], json!({"open": false, "back": true, "marked": []}), "{r}");
    assert_eq!((&r["worker"]["key"], &r["worker"]["part"]), (&json!("w-#9 PR 2/2"), &json!("PR 2/2")), "{r}");
    assert_eq!(r["worker"]["rows"], json!([["PR 1/2", false], ["PR 2/2", true]]), "the clicked worker is shaded: {r}");
    assert_eq!(r["worker"]["pause"], "Resume", "a paused ticket offers Resume: {r}");
    assert_eq!(
        (&r["pill"]["prevented"], &r["pill"]["id"], &r["pill"]["ghpr"], &r["pill"]["pr"]),
        (&json!(true), &json!("acme/a#7"), &json!("Open PR #7 on GitHub"), &json!("#7 failing")),
        "{r}"
    );
    assert_eq!(r["chipBack"], true, "{r}");
    assert_eq!(
        (&r["entered"]["id"], &r["entered"]["focus"], &r["entered"]["pause"]),
        (&json!("acme/a#5"), &json!(true), &json!("Resume")),
        "{r}"
    );
    assert_eq!(r["back"], true, "{r}");
    assert_eq!(r["posts"], json!(["/api/links/t-acme%2Fa%235/resume"]), "{r}");
}

#[tokio::test]
async fn rows_workers_and_the_inspector_show_issue_pr_and_parent_chips() {
    let s = start(H::new()).await;
    join_board(&s.h, TICKET, "widgets#105").await;
    join_board(&s.h, BOSS, "widgets#50").await;
    let identity = json!({"parent": TICKET, "label": "#105 worker", "role": "worker",
        "issue": "acme/widgets#115", "parentIssue": "acme/widgets#105", "part": "2/3"});
    s.h.wf(WORKER, "join", identity).await;
    s.h.fake.set_prs(WORKER, vec![pull_request("acme/widgets", 136, "open")]);
    s.h.fake.set_prs(BOSS, vec![pull_request("acme/widgets", 51, "open")]);
    s.h.tick(1).await;
    let html = s.http.get(format!("{}/", s.base)).send().await.unwrap().text().await.unwrap();
    let st: Value = s.http.get(format!("{}/api/state", s.base)).send().await.unwrap().json().await.unwrap();
    let r = drawer_scenario(
        &html,
        &st,
        r##"
emitState(base);
const tk = (label) => [...document.getElementById("open").children].find((e) => e.getAttribute("aria-label") && e.getAttribute("aria-label").startsWith(label + ","));
const chipsOf = (slot) => slot.hidden ? null : [...slot.querySelectorAll("a")].map((a) => [a.textContent, a.getAttribute("href")]);
const text = (slot) => slot.hidden ? null : slot.textContent;
const rowIds = tk("widgets#105").querySelector('.tr [data-part="ids"]');
const wids = tk("widgets#105").querySelector('[data-part="wids"]');
const plainRow = tk("widgets#50");
const first = { row: chipsOf(rowIds), rowText: text(rowIds), worker: chipsOf(wids), workerText: text(wids) };
const navigated = !click(rowIds.querySelector("a"));
const closedAfterChip = !dr().classList.contains("open");
click(tk("widgets#105").querySelector('[data-part="title"]'));
const head = { ids: chipsOf(dp("ids")), part: shown("part") };
press(document.activeElement, "Escape");
click(plainRow.querySelector('[data-part="title"]'));
const plainHead = chipsOf(dp("ids"));
press(document.activeElement, "Escape");
const other = { number: 99, url: "https://github.com/acme/widgets/pull/99", state: "merged" };
emitState({ ...base, tree: base.tree.map((t) => t.label === "widgets#105" ? { ...t, pr: other, workers: t.workers.map((w) => ({ ...w, pr: null })) } : t) });
const noPr = chipsOf(tk("widgets#105").querySelector('.tr [data-part="ids"]'));
return {
  ...first,
  plain: chipsOf(plainRow.querySelector('.tr [data-part="ids"]')), plainPr: plainRow.querySelector(".side .pr").textContent,
  navigated, closedAfterChip, head, plainHead, noPr,
};
"##,
    );
    let chips = json!([
        ["#115", "https://github.com/acme/widgets/issues/115"],
        ["PR #136", "https://github.com/acme/widgets/pull/136"],
        ["of #105", "https://github.com/acme/widgets/issues/105"],
    ]);
    assert_eq!(r["row"], chips, "{r}");
    assert_eq!(r["rowText"], "#115·PR #136·of #105", "{r}");
    assert_eq!(r["worker"], chips, "{r}");
    assert_eq!(r["head"], json!({"ids": chips, "part": "PR 2/3"}), "{r}");
    assert_eq!((&r["navigated"], &r["closedAfterChip"]), (&json!(true), &json!(true)), "an id chip opens GitHub: {r}");
    assert_eq!((&r["plain"], &r["plainHead"]), (&Value::Null, &Value::Null), "{r}");
    assert_eq!(r["plainPr"], "#51 open", "a row with no identity keeps its PR pill: {r}");
    assert_eq!(r["noPr"], json!([chips[0], chips[2]]), "a worker without a PR shows no other worker's PR: {r}");
}

#[tokio::test]
async fn the_inspector_closes_before_search_clears_and_survives_its_subject_leaving() {
    let (html, st) = two_gate_page().await;
    let r = drawer_scenario(
        &html,
        &st,
        r##"
emitState({ ...base, cards: [], tree });
const sq = document.getElementById("filter-q");
sq.value = "acme";
sq.dispatch("input");
advance(1000);
click(row("acme/a#8").querySelector('[data-part="title"]'));
sq.focus();
press(sq, "Escape");
const first = { open: dr().classList.contains("open"), q: sq.value, focus: document.activeElement === sq };
press(sq, "Escape");
const second = { q: sq.value };
sq.value = "acme";
sq.dispatch("input");
advance(1000);
click(row("acme/a#8").querySelector('[data-part="title"]'));
press(document.activeElement, "Escape");
const fromTitle = { open: dr().classList.contains("open"), q: sq.value, onRow: document.activeElement === row("acme/a#8") };
press(document.activeElement, "Escape");
fromTitle.after = sq.value;
sq.value = "acme";
sq.dispatch("input");
advance(1000);
const chip = row("acme/a#7").querySelector(".side .pr a");
click(chip);
press(document.activeElement, "Escape");
fromTitle.chip = document.activeElement === chip;
press(document.activeElement, "Escape");
fromTitle.chipAfter = sq.value;
let fail;
fetchReply = () => new Promise((_, reject) => { fail = reject; });
click(row("acme/a#2").querySelector('[data-part="title"]'));
dp("pause").dispatch("click");
click(row("acme/a#8").querySelector('[data-part="title"]'));
fail(new Error("store is busy"));
await flush();
const stray = { strip: document.getElementById("dr-err").hidden, said: document.getElementById("alert").textContent };
advance(100);
stray.said = document.getElementById("alert").textContent;
fetchReply = () => Promise.reject(new Error("store is busy"));
dp("pause").dispatch("click");
await flush();
stray.own = document.getElementById("dr-err-text").textContent;
document.getElementById("dr-err-close").dispatch("click");
stray.back = document.activeElement === dp("pause");
fetchReply = null;
click(row("acme/a#8").querySelector('[data-part="title"]'));
emitState({ ...base, cards: [], tree: tree.filter((t) => t.label !== "acme/a#8") });
const gone = { open: dr().classList.contains("open"), note: shown("gone"), pause: dp("pause").getAttribute("aria-disabled"), id: dp("id").textContent };
emitState({ ...base, cards: [], tree });
const back = { note: shown("gone"), pause: dp("pause").getAttribute("aria-disabled") };
dp("close").dispatch("click");
const closed = dr().classList.contains("open"), focus = document.activeElement === row("acme/a#8");
const st = T("acme/s#1", { done: true, settledIso: "2026-11-18T10:00:00Z", settledAt: "03:00 MST" });
emitState({ ...base, cards: [], tree, settled: { tickets: [st], asks: [] } });
for (const t of ["open", "done", "settled"]) document.getElementById("tp-" + t).append(document.getElementById(t));
selectTab("settled");
click(document.getElementById("settled").firstChild.querySelector('[data-part="label"]'));
const settled = { restore: shown("restore"), settle: shown("settle") };
emitState({ ...base, cards: [], tree: [...tree, { ...st, settledIso: undefined }], settled: { tickets: [], asks: [] } });
press(document.activeElement, "Escape");
const hiddenRow = document.querySelector('#done [data-thread="t-acme/s#1"]');
settled.focusHidden = document.activeElement === hiddenRow;
settled.focusSettled = document.activeElement === document.getElementById("settled");
return { first, second, fromTitle, stray, gone, back, closed, focus, settled };
"##,
    );
    assert_eq!(r["first"], json!({"open": false, "q": "acme", "focus": true}), "Esc closes the drawer first: {r}");
    assert_eq!(r["second"], json!({"q": ""}), "a second Esc clears search: {r}");
    assert_eq!(
        r["fromTitle"],
        json!({"open": false, "q": "acme", "onRow": true, "after": "", "chip": true, "chipAfter": ""}),
        "from the drawer, Esc goes back to the row, then clears search: {r}"
    );
    assert_eq!(
        r["stray"],
        json!({"strip": true, "said": "Pause acme/a#2: store is busy", "own": "Pause acme/a#8: store is busy", "back": true}),
        "a failure for another ticket is announced under its own name, not shown under this one: {r}"
    );
    assert_eq!(
        r["gone"],
        json!({"open": true, "note": "This ticket is no longer on the board. Its last known state is below.", "pause": "true", "id": "acme/a#8"}),
        "{r}"
    );
    assert_eq!(r["back"], json!({"note": null, "pause": null}), "{r}");
    assert_eq!((&r["closed"], &r["focus"]), (&json!(false), &json!(true)), "{r}");
    assert_eq!(
        r["settled"],
        json!({"restore": "Restore", "settle": null, "focusHidden": false, "focusSettled": true}),
        "a restored ticket's hidden row does not take focus: {r}"
    );
}

#[tokio::test]
async fn the_inspector_shows_cards_new_issues_and_the_full_latest_message() {
    let (html, st) = two_gate_page().await;
    let r = drawer_scenario(
        &html,
        &st,
        &format!(
            r##"
fetchReply = async (url) => url === "/api/msg/m9"
  ? {{ ok: true, status: 200, json: async () => ({{ body: "Full **gate** text, here: https://github.com/acme/a/pull/3" }}) }}
  : {{ ok: true, status: 200, json: async () => ({{}}) }};
const ticket = base.tree.find((t) => t.thread === "{TICKET}");
const last = {{ handle: "m9", kind: "gate", head: "x".repeat(140) + "...", at: "10:00 MST" }};
const nis = [{{ id: "n1", label: "acme/a#30", repo: "acme/a", number: 30, title: "Flaky test", note: "Seen **twice**",
  url: "https://github.com/acme/a/issues/30", threadTitle: "Release", threadLink: "https://app.t3.codes/threads/p/t", at: "2026-11-18T10:00:00Z" }}];
const s1 = {{ ...base, tree: base.tree.map((t) => t.thread === ticket.thread ? {{ ...t, last }} : t), newIssues: nis }};
emitState(s1);
const cid = s1.cards.find((c) => c.ticket === ticket.thread).id;
click(card(cid).querySelector('[data-part="ask"]'));
const fromCard = {{ id: dp("id").textContent, needs: [...dp("needs-l").children].map((c) => [c.querySelector('[data-part="go"]').textContent, c.hasAttribute("data-inspected")]),
  marked: inspected(), loading: dp("latest-m").textContent, latest: dp("latest-h").textContent }};
await flush();
emitState(s1);
await flush();
const full = {{ body: dp("latest-b").textContent, loads: fetches.filter((f) => f.url === "/api/msg/m9").length, meta: dp("latest-m").textContent }};
dp("needs-l").firstChild.querySelector('[data-part="go"]').dispatch("click");
const jumped = document.activeElement === card(cid);
click(document.getElementById("nis").firstChild.querySelector('[data-part="title"]'));
const issue = {{ ...view(), note: dp("note-b").textContent, meta: dp("note-m").textContent, launch: shown("launch"), flip: shown("nisflip"),
  t3: dp("t3").textContent, workers: dp("workers").hidden }};
dp("launch").dispatch("click");
await flush();
return {{ fromCard, full, jumped, issue, posts: fetches.filter((f) => f.method === "POST").map((f) => f.url) }};
"##
        ),
    );
    assert_eq!(r["fromCard"]["id"], "widgets#41", "a card with a ticket opens its ticket: {r}");
    assert_eq!(r["fromCard"]["needs"].as_array().unwrap().iter().filter(|c| c[1] == true).count(), 1, "{r}");
    assert_eq!(r["fromCard"]["marked"].as_array().unwrap().len(), 2, "the row and the card are marked: {r}");
    assert!(r["fromCard"]["loading"].as_str().unwrap().contains("Loading the full message."), "{r}");
    assert_eq!(r["fromCard"]["latest"], "Latest gate", "{r}");
    assert_eq!(
        r["full"],
        json!({"body": "Full gate text, here: https://github.com/acme/a/pull/3", "loads": 1, "meta": "10:00"}),
        "the full body replaces the cut head once: {r}"
    );
    assert_eq!(r["jumped"], true, "{r}");
    assert_eq!(
        r["issue"],
        json!({"open": true, "id": "acme/a#30", "part": null, "title": "Flaky test", "st": "new issue", "pr": "",
               "issue": "https://github.com/acme/a/issues/30", "ghpr": null, "pause": null, "focus": true,
               "note": "Seen twice", "meta": format!("Filed by Release · {}", r["issue"]["meta"].as_str().unwrap().rsplit(" · ").next().unwrap()),
               "launch": "Launch", "flip": "Dismiss", "t3": "Open filing thread", "workers": true}),
        "{r}"
    );
    assert_eq!(r["posts"], json!(["/api/new-issues/n1/launch"]), "{r}");
}

#[tokio::test]
async fn the_inspector_is_a_modal_slide_over_documented_on_the_board_page() {
    let (html, _) = two_gate_page().await;
    for needle in [
        ".drawer {",
        "width: 460px; max-width: 100vw;",
        "transform: translateX(100%); visibility: hidden;",
        r#"role: "dialog", "aria-modal": "true", "aria-labelledby": "dr-title""#,
        "<dt>Enter</dt><dd>Open the focused card's or row's details</dd>",
    ] {
        assert!(html.contains(needle), "missing {needle:?}");
    }
    let doc = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/docs/src/board.md")).unwrap();
    assert!(doc.contains("## The inspector"), "docs/src/board.md documents the drawer");
}

#[tokio::test]
async fn the_drawers_are_modal_trap_tab_and_hand_focus_back_to_their_trigger() {
    let (html, st) = two_gate_page().await;
    let r = drawer_scenario(
        &html,
        &st,
        r##"
emitState({ ...base, cards: [], tree });
const scrim = () => !document.getElementById("scrim").hidden;
const trigger = row("acme/a#2");
trigger.focus();
press(trigger, "Enter");
const opened = { open: dr().classList.contains("open"), scrim: scrim(), modal: dr().getAttribute("aria-modal"), role: dr().getAttribute("role"),
  focus: document.activeElement === dp("title") };
const close = dp("close");
document.getElementById("filter-q").focus();
press(document.activeElement, "Tab");
const tabbed = dr().contains(document.activeElement);
const stops = tabbable(dr());
const last = stops[stops.length - 1];
last.focus();
press(last, "Tab");
const wrapped = stops.length > 1 && document.activeElement === stops[0] && stops[0] === close;
const shiftEv = close.dispatch("keydown", { key: "Tab", shiftKey: true });
for (const f of document.listeners.keydown) f(shiftEv);
const back = document.activeElement === last;
press(document.activeElement, "j");
const kept = dr().contains(document.activeElement);
close.dispatch("click");
const closed = { open: dr().classList.contains("open"), scrim: scrim(), back: document.activeElement === trigger };
const launch = document.getElementById("launch-open");
launch.focus();
openComposer("launch");
const composer = { scrim: scrim(), focus: document.activeElement === document.getElementById("issues"),
  title: document.getElementById("composer-h").textContent, expanded: launch.getAttribute("aria-expanded") };
trigger.focus();
press(trigger, "Enter");
const swapped = { composer: composerFrom.open, drawer: dr().classList.contains("open") };
press(document.activeElement, "Escape");
trigger.focus();
openComposer("ask");
press(document.getElementById("question"), "Escape");
const esc = { open: composerFrom.open, scrim: scrim(), back: document.activeElement === trigger,
  title: document.getElementById("composer-h").textContent };
return { opened, tabbed, wrapped, back, kept, closed, composer, swapped, esc };
"##,
    );
    assert_eq!(
        r["opened"],
        json!({"open": true, "scrim": true, "modal": "true", "role": "dialog", "focus": true}),
        "{r}"
    );
    assert_eq!(r["tabbed"], true, "Tab from outside a modal drawer lands back inside it: {r}");
    assert_eq!(r["wrapped"], true, "Tab past the last control wraps to the first: {r}");
    assert_eq!(r["back"], true, "Shift+Tab from the first control wraps to the last: {r}");
    assert_eq!(r["kept"], true, "single-key shortcuts do not pull focus out of a modal drawer: {r}");
    assert_eq!(r["closed"], json!({"open": false, "scrim": false, "back": true}), "{r}");
    assert_eq!(
        r["composer"],
        json!({"scrim": true, "focus": true, "title": "Launch tickets", "expanded": "true"}),
        "{r}"
    );
    assert_eq!(r["swapped"], json!({"composer": false, "drawer": true}), "one drawer at a time: {r}");
    assert_eq!(
        r["esc"],
        json!({"open": false, "scrim": false, "back": true, "title": "Ask your agent"}),
        "Esc closes the composer and focus returns to what opened it: {r}"
    );
}

#[tokio::test]
async fn g_t_and_g_d_close_a_full_width_drawer_over_the_row_they_focus() {
    let (html, st) = two_gate_page().await;
    let r = drawer_scenario(
        &html,
        &st,
        r##"
emitState({ ...base, tree });
selectTab("open");
const phone = { left: 0, right: 390, top: 100, bottom: 160, width: 390, height: 60 };
const go = (key) => {
  click(row("acme/a#2").querySelector('[data-part="title"]'));
  dr().rect = { left: 0, right: 390, top: 48, bottom: 800, width: 390, height: 752 };
  for (const id of ["open", "cards"]) for (const el of document.getElementById(id).children) el.rect = phone;
  press(document.activeElement, "g");
  press(document.activeElement, key);
  const f = document.activeElement;
  return { open: dr().classList.contains("open"), list: f.parentNode && f.parentNode.id };
};
return { tickets: go("t"), decisions: go("d") };
"##,
    );
    assert_eq!(r["tickets"], json!({"open": false, "list": "open"}), "g t uncovers the ticket row it focuses: {r}");
    assert_eq!(r["decisions"], json!({"open": false, "list": "cards"}), "g d uncovers the card it focuses: {r}");
}

#[tokio::test]
async fn a_card_link_off_github_is_not_labelled_as_on_github() {
    let (html, st) = two_gate_page().await;
    let r = drawer_scenario(
        &html,
        &st,
        r##"
const mk = (id, url) => ({ ...base.cards[0], id, ticket: null, label: null, text: "**ACTION NEEDED:** approve the deploy, here: " + url });
emitState({ ...base, cards: [mk("c1", "https://ci.example.com/runs/5"), mk("c2", "https://github.com/acme/a/actions/runs/5"), mk("c3", "https://github.com/acme/a/pull/3")] });
const link = (id) => {
  click(card(id).querySelector('[data-part="ask"]'));
  return [dp("id").textContent, dp("gh-issue").textContent, dp("gh-issue").getAttribute("aria-label")];
};
return { off: link("c1"), runs: link("c2"), pr: link("c3") };
"##,
    );
    assert_eq!(r["off"], json!(["Card c1", "Open link", "Open link for card c1"]), "{r}");
    assert_eq!(r["runs"], json!(["Card c2", "Open link on GitHub", "Open link for card c2"]), "{r}");
    assert_eq!(r["pr"], json!(["Card c3", "Review #3 on GitHub", "Review #3 for card c3"]), "{r}");
}

#[tokio::test]
async fn enter_on_a_removed_row_that_kept_focus_opens_nothing() {
    let (html, st) = two_gate_page().await;
    let r = drawer_scenario(
        &html,
        &st,
        r##"
emitState({ ...base, cards: [], tree });
const leave = (label) => {
  row(label).focus();
  emitState({ ...base, cards: [], tree: tree.filter((t) => t.label !== label) });
  const stale = row(label);
  press(stale, "Enter");
  advance(100);
  const out = { stale: stale.classList.contains("stale-item"), ...view(), gone: shown("gone"), said: document.getElementById("live").textContent };
  emitState({ ...base, cards: [], tree });
  return out;
};
const closed = leave("acme/a#8");
click(row("acme/a#2").querySelector('[data-part="title"]'));
const kept = leave("acme/a#7");
return { closed, open: kept };
"##,
    );
    assert_eq!(
        (&r["closed"]["stale"], &r["closed"]["open"], &r["closed"]["said"]),
        (&json!(true), &json!(false), &json!("This ticket is no longer on the board.")),
        "a closed drawer stays closed: {r}"
    );
    assert_eq!(
        (&r["open"]["open"], &r["open"]["id"], &r["open"]["gone"], &r["open"]["said"]),
        (&json!(true), &json!("acme/a#2"), &json!(null), &json!("This ticket is no longer on the board.")),
        "an open drawer keeps its subject, with no gone note over it: {r}"
    );
}

#[tokio::test]
async fn a_late_message_load_never_overwrites_a_newer_one_or_grows_the_cache() {
    let (html, _) = two_gate_page().await;
    let r = run_page_async(
        &html,
        r##"async () => {
const flush = async () => { for (let i = 0; i < 20; i++) await Promise.resolve(); };
const waits = {};
fetchReply = (url) => new Promise((ok, fail) => { (waits[url] ||= []).push({ ok, fail }); });
const body = (b) => ({ ok: true, status: 200, json: async () => ({ body: b }) });
const fill = (p) => { for (let i = 1; i <= 64; i++) loadMsg(p + i); };
loadMsg("m0");
fill("a");
const evicted = !dr.msgs.has("m0");
waits["/api/msg/m0"][0].fail(new Error("slow"));
await flush();
const late = { evicted, size: dr.msgs.size, has: dr.msgs.has("m0") };
loadMsg("m1");
fill("b");
loadMsg("m1");
waits["/api/msg/m1"][1].ok(body("fresh"));
await flush();
waits["/api/msg/m1"][0].fail(new Error("slow"));
await flush();
return { late, newer: dr.msgs.get("m1"), size: dr.msgs.size };
}"##,
    );
    assert_eq!(r["late"], json!({"evicted": true, "size": 64, "has": false}), "an evicted load stays evicted: {r}");
    assert_eq!(r["newer"], json!({"body": "fresh"}), "a late failure leaves the newer result: {r}");
    assert_eq!(r["size"], 64, "{r}");
}

#[tokio::test]
async fn a_pending_drawer_action_does_not_disable_the_next_subjects_button() {
    let (html, st) = two_gate_page().await;
    let r = drawer_scenario(
        &html,
        &st,
        r##"
emitState({ ...base, cards: [], tree });
const waits = [];
fetchReply = () => new Promise((ok) => waits.push(ok));
const done = { ok: true, status: 200, json: async () => ({}) };
const pick = (label) => { click(row(label).querySelector('[data-part="title"]')); return dp("pause").getAttribute("aria-disabled"); };
pick("acme/a#2");
dp("pause").dispatch("click");
const first = dp("pause").getAttribute("aria-disabled");
const other = pick("acme/a#8");
dp("pause").dispatch("click");
const second = dp("pause").getAttribute("aria-disabled");
const bothA = pick("acme/a#2");
dp("pause").dispatch("click");
waits[0](done);
await flush();
const doneA = dp("pause").getAttribute("aria-disabled");
const againB = pick("acme/a#8");
if (waits[1]) waits[1](done);
await flush();
return { first, other, second, bothA, doneA, againB, settled: dp("pause").getAttribute("aria-disabled"),
  posts: fetches.filter((f) => f.method === "POST").map((f) => f.url) };
"##,
    );
    assert_eq!(
        r,
        json!({"first": "true", "other": null, "second": "true", "bothA": "true", "doneA": null, "againB": "true", "settled": null,
               "posts": ["/api/links/t-acme%2Fa%232/pause", "/api/links/t-acme%2Fa%238/pause"]}),
        "a pending action disables the button only for its own subject: {r}"
    );
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
    assert_eq!(r["text"], "Choose", "{r}");
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
async fn api_state_reports_an_open_prs_readiness_from_the_last_reconcile() {
    use t3_codemode::fake::{check_run, github_pr};
    let s = start(H::new()).await;
    ticket_with_worker(&s.h).await;
    s.h.fake.set_prs(WORKER, vec![pull_request("acme/widgets", 81, "open")]);
    s.h.tick(1).await;
    let head = "1111111111111111111111111111111111111111";
    s.h.github.set_pr("acme/widgets", 81, github_pr(head, "CLEAN", false));
    s.h.github.update_pr("acme/widgets", 81, json!({"reviewDecision": "APPROVED"}));
    s.h.github.set_checks("acme/widgets", 81, json!([check_run("build", "SUCCESS")]));
    let worker = |st: &Value| {
        let t = st["tree"].as_array().unwrap().iter().find(|t| t["label"] == "widgets#41").unwrap();
        t["workers"][0]["pr"].clone()
    };
    assert!(worker(&s.get("/api/state").await).get("readiness").is_none());

    s.h.app.wf.reconcile().await.unwrap();

    let pr = worker(&s.get("/api/state").await);
    assert_eq!(pr["state"], "open");
    assert_eq!(
        pr["readiness"],
        json!({"isDraft": false, "mergeStateStatus": "CLEAN", "reviewDecision": "APPROVED", "checks": "passing"})
    );
    assert_eq!(s.h.github.operations(), ["reconcile"]);
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
    assert_eq!(by_label(&again["tree"], "widgets#41")["title"], "widgets#41: Fix the grader");
}

const FILTER_HELPERS: &str = r##"
const $$ = (id) => document.getElementById(id);
const keysOf = (id) => [...$$(id).children].filter((e) => e.dataset.key && !e.classList.contains("gone")).map((e) => e.dataset.key);
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
    page_scenario(html, st, &format!("{}\n{body}", filter_setup()))
}

fn filter_setup() -> String {
    format!(
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
}};"##
    )
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
pick("projects", "p-widgets");
const one = { ...view(), btn: facet("projects").querySelector('[data-part="fbtn"]').textContent, clear: !$$("filter-clear").hidden };
pick("projects", "p-gadgets");
const two = view();
$$("filter-clear").dispatch("click");
return { all, opts, found, one, two, after: view(), btn: facet("projects").querySelector('[data-part="fbtn"]').textContent };
"##,
    );
    let t = json!(TICKET);
    let w = json!(WORKER);
    assert_eq!(r["all"]["cards"], json!(["a1", "a2"]), "{r}");
    assert_eq!(r["all"]["n"], json!(["2", "2", "0", "2", "2"]), "{r}");
    assert_eq!(r["opts"], json!(["p-gadgets", "p-widgets"]), "{r}");
    assert_eq!(r["found"], json!(["p-gadgets"]), "{r}");
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
pick("projects", "p-widgets");
const before = { hidden: $$("hidden-new").hidden };
const a3 = { ...s0.cards[1], id: "a3", at: "2026-11-17T12:30:00Z", kind: "blocker", token: "t-a3" };
emitState({ ...s0, cards: [...s0.cards, a3] });
const after = { hidden: $$("hidden-new").hidden, text: $$("hidden-new").textContent, urgent: $$("hidden-new").getAttribute("data-urgent"), cards: ids(), n: $$("n-cards").textContent };
emitState({ ...s0, cards: [s0.cards[0], { ...s0.cards[1], token: "t-new" }, a3] });
const replaced = $$("hidden-new").textContent;
$$("hidden-new").dispatch("click");
return { before, after, replaced, shown: ids(), still: $$("hidden-new").hidden };
"##,
    );
    assert_eq!(r["before"]["hidden"], true, "{r}");
    assert_eq!(
        r["after"],
        json!({"hidden": false, "text": "1 hidden", "urgent": "1", "cards": ["a1"], "n": "1/3"}),
        "{r}"
    );
    assert_eq!(r["replaced"], "2 hidden", "{r}");
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
const old = { handle: "m94", at: "08:00 MST", iso: "2026-11-17T15:00:00Z", label: "gizmos#5", kind: "note", head: "merged", ticket: "th-old", ticketStatus: "done", repo: null, project: null };
const s1 = { ...s0, cards: [...s0.cards, a3], feed: [...s0.feed, old], tree: s0.tree.map((t) => t.thread === W ? { ...t, done: true, status: "done", gate: null } : t) };
emitState(s1);
pick("statuses", "done");
const done = view();
pick("statuses", "done");
pick("statuses", "blocked");
const blocked = view();
pick("statuses", "blocked");
pick("statuses", "waiting");
const needs = view();
pick("statuses", "waiting");
pick("kinds", "gate");
const gates = view();
pick("kinds", "gate");
emitState({ ...s1, cards: s0.cards, tree: s1.tree.map((x) => x.thread === T ? { ...x, status: "failed" } : x) });
pick("statuses", "blocked");
const failedGate = view();
return { done, blocked, needs, gates, failedGate };
"##,
    );
    let t = json!(TICKET);
    let w = json!(WORKER);
    assert_eq!(r["done"]["cards"], json!(["a2"]), "{r}");
    assert_eq!(r["done"]["open"], json!([]), "{r}");
    assert_eq!(r["done"]["done"], json!([w]), "{r}");
    assert_eq!(r["done"]["settled"], json!(["t:th-s"]), "{r}");
    assert_eq!(r["done"]["feed"], json!(["m92", "m94"]), "{r}");
    assert_eq!(r["blocked"]["cards"], json!(["a3", "a1"]), "{r}");
    assert_eq!(r["blocked"]["open"], json!([t]), "{r}");
    assert_eq!(r["blocked"]["feed"], json!(["m91"]), "{r}");
    assert_eq!(
        (&r["needs"]["cards"], &r["needs"]["open"]),
        (&json!(["a3", "a1"]), &json!([t])),
        "a blocked ticket with cards also needs you: {r}"
    );
    assert_eq!(r["gates"]["cards"], json!(["a1", "a2"]), "{r}");
    assert_eq!(r["gates"]["open"], json!([t]), "{r}");
    assert_eq!(r["failedGate"]["open"], json!([t]), "a failed ticket with only a gate is still blocked: {r}");
}

#[tokio::test]
async fn slash_focuses_search_and_a_focuses_ask_only_with_single_key_shortcuts_on() {
    let (html, st) = two_gate_page().await;
    let r = page_scenario(
        &html,
        &st,
        r##"
emitState(base);
const press = (key) => { for (const f of document.listeners.keydown) f({ key, target: document.body, preventDefault() {} }); };
const searchBox = document.getElementById("filter-q");
const askBox = document.getElementById("question");
press("/");
const search = document.activeElement === searchBox;
document.activeElement = document.body;
press("a");
const asked = document.activeElement === askBox;
document.activeElement = document.body;
press("f");
const f = document.activeElement === searchBox;
document.activeElement = document.body;
const box = document.getElementById("keys-on");
box.checked = false;
box.dispatch("change");
press("/");
press("a");
return { search, asked, f, off: document.activeElement !== document.body };
"##,
    );
    assert_eq!(r, json!({"search": true, "asked": true, "f": false, "off": false}));
}

#[tokio::test]
async fn the_theme_starts_dark_cycles_persists_and_survives_missing_storage() {
    let (html, st) = two_gate_page().await;
    let prelude = r#"
globalThis.__dark = false;
globalThis.__mq = [];
globalThis.matchMedia = (m) => ({ get matches() { return /prefers-color-scheme: dark/.test(m) && globalThis.__dark; }, addEventListener: (_, f) => globalThis.__mq.push(f) });
"#;
    let body = format!(
        r##"() => {{
{PAGE_HELPERS}
const base = {st};
emitState(base);
const root = document.documentElement;
const btn = document.getElementById("theme-btn");
const shown = () => btn.querySelectorAll("[data-theme-icon]").filter((s) => !s.hasAttribute("hidden")).map((s) => s.getAttribute("data-theme-icon"));
const snap = () => ({{ theme: root.getAttribute("data-theme"), stored: localStorage.getItem("t3board.theme"), label: btn.getAttribute("aria-label") }});
const loaded = snap();
btn.dispatch("click");
const light = snap();
for (const f of document.listeners.keydown) f({{ key: "t", target: document.body, preventDefault() {{}} }});
const system = snap();
globalThis.__dark = true;
for (const f of globalThis.__mq) f();
const osDark = snap();
for (const k of ["system", "light", "dark"]) btn.append(h("svg", {{ "data-theme-icon": k }}));
localStorage.removeItem = () => {{ throw new Error("blocked"); }};
btn.dispatch("click");
const noStore = {{ ...snap(), icons: shown() }};
return {{ loaded, light, system, osDark, noStore }};
}}"##
    );
    let r = run_page_async_with(&html, prelude, &body);
    assert_eq!(
        r["loaded"],
        json!({"theme": "dark", "stored": null, "label": "Theme: Dark. Switch to Light"}),
        "dark is the default even when the OS is light: {r}"
    );
    assert_eq!(
        r["light"],
        json!({"theme": "light", "stored": "light", "label": "Theme: Light. Switch to System"}),
        "{r}"
    );
    assert_eq!(
        r["system"],
        json!({"theme": "light", "stored": "system", "label": "Theme: System. Switch to Dark"}),
        "t cycles the theme like the button: {r}"
    );
    assert_eq!(r["osDark"]["theme"], json!("dark"), "System follows the OS: {r}");
    assert_eq!(r["noStore"]["theme"], json!("dark"), "a storage error still switches the theme: {r}");
    assert_eq!(r["noStore"]["icons"], json!(["dark"]), "{r}");
    let legacy = run_page_async_with(
        &html,
        &format!("{prelude}\nglobalThis.__storage = {{ \"t3board.theme\": \"dark\" }};"),
        "() => document.documentElement.getAttribute(\"data-theme\")",
    );
    assert_eq!(legacy, json!("dark"), "a saved dark choice still loads dark");
}

#[tokio::test]
async fn the_density_toggle_compacts_rows_and_is_remembered() {
    let (html, st) = two_gate_page().await;
    let body = format!(
        r##"() => {{
{PAGE_HELPERS}
const base = {st};
emitState(base);
const root = document.documentElement;
const btn = document.getElementById("density-btn");
const snap = () => [root.getAttribute("data-density"), btn.getAttribute("aria-pressed"), localStorage.getItem("t3board.density")];
const before = snap();
btn.dispatch("click");
const compact = snap();
for (const f of document.listeners.keydown) f({{ key: "d", target: document.body, preventDefault() {{}} }});
return {{ before, compact, back: snap() }};
}}"##
    );
    let r = run_page_async_with(&html, "", &body);
    assert_eq!(r["before"], json!([null, "false", null]), "{r}");
    assert_eq!(r["compact"], json!(["compact", "true", "compact"]), "{r}");
    assert_eq!(r["back"], json!([null, "false", null]), "d toggles it back: {r}");
    assert!(html.contains(r#":root[data-density="compact"] { --row-pad: 6px; --gap: 12px; }"#));
}

#[tokio::test]
async fn the_layout_switches_to_workspace_is_remembered_and_survives_missing_storage() {
    let (html, st) = two_gate_page().await;
    let body = format!(
        r##"() => {{
{PAGE_HELPERS}
const base = {st};
emitState(base);
const root = document.documentElement;
const btn = document.getElementById("layout-btn");
const snap = () => ({{ layout: root.getAttribute("data-layout"), stored: localStorage.getItem("t3board.layout"), label: btn.getAttribute("aria-label") }});
const loaded = snap();
const cards = ids();
btn.dispatch("click");
const ws = {{ ...snap(), cards: ids(), color: document.getElementById("theme-color").getAttribute("content") }};
btn.dispatch("click");
const back = snap();
localStorage.setItem = () => {{ throw new Error("blocked"); }};
btn.dispatch("click");
const noStore = snap();
return {{ loaded, cards, ws, back, noStore }};
}}"##
    );
    let r = run_page_async_with(&html, "", &body);
    assert_eq!(
        r["loaded"],
        json!({"layout": "dispatch", "stored": null, "label": "Layout: Dispatch. Switch to Workspace"}),
        "Dispatch is the default: {r}"
    );
    assert_eq!(
        (&r["ws"]["layout"], &r["ws"]["stored"], &r["ws"]["label"]),
        (&json!("workspace"), &json!("workspace"), &json!("Layout: Workspace. Switch to Dispatch")),
        "{r}"
    );
    assert_eq!(r["ws"]["cards"], r["cards"], "switching keeps every card: {r}");
    assert_eq!(r["ws"]["color"], json!("#0d1117"), "{r}");
    assert_eq!(r["back"]["layout"], json!("dispatch"), "{r}");
    assert_eq!(
        r["back"]["stored"],
        json!("dispatch"),
        "a Dispatch pick is saved, so it beats a Workspace default: {r}"
    );
    assert_eq!(r["noStore"]["layout"], json!("workspace"), "a storage error still switches the layout: {r}");
    let saved = run_page_async_with(
        &html,
        r#"globalThis.__storage = { "t3board.layout": "workspace" };"#,
        "() => document.documentElement.getAttribute(\"data-layout\")",
    );
    assert_eq!(saved, json!("workspace"), "a saved Workspace pick survives a reload");
    let boot = &html[..html.find("</script>").unwrap()];
    assert!(boot.contains(r#"localStorage.getItem("t3board.layout")"#), "the head script sets the layout before paint");
    assert!(html.contains(r#":root[data-layout="workspace"] .wnav {"#));
    assert!(html.contains(".wnav, .tr .side .wstep { display: none; }"), "Dispatch never shows the Workspace parts");
}

#[tokio::test]
async fn the_workspace_nav_counts_routes_views_and_ticket_rows_carry_a_step_pill() {
    let (html, st) = two_gate_page().await;
    let r = page_scenario(
        &html,
        &st,
        &format!(
            r##"{ROWS}
{PR_TREE}
const rt = [...tree, T("acme/r#10", {{ progress: {{ step: 2, of: 3, merged: 1 }}, workers: [
  W("#10 PR 1/3", "done", {{ step: {{ n: 1, of: 3 }}, pr: pr(101, {{ state: "merged" }}) }}),
  W("#10 PR 2/3", "running", {{ step: {{ n: 2, of: 3 }}, pr: pr(102) }}),
] }})];
emitState({{ ...base, tree: rt }});
document.getElementById("layout-btn").dispatch("click");
const app = document.getElementById("app");
const count = (k) => document.getElementById("wc-" + k).textContent;
const counts = {{ overview: count("overview"), needs: count("needs"), hot: document.getElementById("wc-needs").classList.contains("on"), prs: count("prs") }};
const go = (k) => {{
  document.getElementById("wn-" + k).dispatch("click");
  return [app.getAttribute("data-view"), app.getAttribute("data-wsnav"), document.getElementById("wn-" + k).getAttribute("aria-current")];
}};
const nav = {{ graph: go("graph"), activity: go("activity"), issues: go("issues"), prs: go("prs"), needs: go("needs"), overview: go("overview") }};
selectView("queue");
const followed = app.getAttribute("data-wsnav");
jump("decisions");
const jumped = app.getAttribute("data-wsnav");
const step = (label) => {{
  const el = row(label);
  const sp = el.querySelector('[data-part="wstep"]');
  return [el.getAttribute("data-step"), sp ? sp.hidden : null, sp ? sp.textContent : null];
}};
return {{ counts, nav, followed, jumped, routed: step("acme/r#10"), plain: step("acme/a#8") }};
"##
        ),
    );
    assert_eq!(
        r["counts"],
        json!({"overview": "10", "needs": "2", "hot": true, "prs": "9"}),
        "PRs counts 8 open pull requests and the WIP branch of #1's running worker: {r}"
    );
    assert_eq!(r["nav"]["graph"], json!(["queue", "graph", "page"]), "{r}");
    assert_eq!(r["nav"]["activity"], json!(["activity", "activity", "page"]), "{r}");
    assert_eq!(r["nav"]["issues"], json!(["issues", "issues", "page"]), "{r}");
    assert_eq!(r["nav"]["prs"], json!(["dispatch", "prs", "page"]), "{r}");
    assert_eq!(r["nav"]["needs"], json!(["dispatch", "needs", "page"]), "{r}");
    assert_eq!(r["nav"]["overview"], json!(["dispatch", "overview", "page"]), "{r}");
    assert_eq!(r["followed"], json!("graph"), "a view picked another way moves the nav: {r}");
    assert_eq!(r["jumped"], json!("needs"), "g d shows Needs you even when it is clear: {r}");
    assert_eq!(r["routed"], json!(["", false, "Step 2/3"]), "{r}");
    assert_eq!(r["plain"], json!([null, null, null]), "a one-PR ticket keeps its status pill: {r}");
}

const HASH_PRELUDE: &str = r#"globalThis.__location = { hostname: "127.0.0.1", hash: "", pathname: "/", origin: "http://127.0.0.1:47731", reload() {} };
globalThis.history = { replaceState(_s, _t, tag) { globalThis.__location.hash = tag; } };"#;

#[tokio::test]
async fn workspace_nav_views_keep_their_address_and_reveal_a_hidden_list() {
    let (html, st) = two_gate_page().await;
    let body = format!(
        r##"() => {{
{PAGE_HELPERS}
const base = {st};
{ROWS}
{PR_TREE}
emitState({{ ...base, tree }});
document.getElementById("layout-btn").dispatch("click");
const app = document.getElementById("app");
const go = (k) => {{ document.getElementById("wn-" + k).dispatch("click"); return [app.getAttribute("data-wsnav"), location.hash]; }};
const hashes = {{ needs: go("needs"), prs: go("prs"), overview: go("overview"), activity: go("activity") }};
go("needs");
jump("tickets");
const tickets = app.getAttribute("data-wsnav");
go("prs");
focusList(document.getElementById("open"));
const openFromPrs = app.getAttribute("data-wsnav");
go("prs");
focusList(document.getElementById("prs"));
const prsStays = app.getAttribute("data-wsnav");
go("needs");
document.getElementById("layout-btn").dispatch("click");
const dispatchHash = location.hash;
document.getElementById("layout-btn").dispatch("click");
const backHash = location.hash;
return {{ hashes, tickets, openFromPrs, prsStays, dispatchHash, backHash }};
}}"##
    );
    let r = run_page_async_with(&html, HASH_PRELUDE, &body);
    assert_eq!(
        r["hashes"],
        json!({"needs": ["needs", "#needs"], "prs": ["prs", "#prs"], "overview": ["overview", "#dispatch"], "activity": ["activity", "#activity"]}),
        "{r}"
    );
    assert_eq!(r["tickets"], json!("overview"), "g t from Needs you shows Issues in progress on the Overview: {r}");
    assert_eq!(r["openFromPrs"], json!("overview"), "focusing a list PRs hides goes back to the Overview: {r}");
    assert_eq!(r["prsStays"], json!("prs"), "{r}");
    assert_eq!(r["dispatchHash"], json!("#dispatch"), "Dispatch has no Needs you address: {r}");
    assert_eq!(r["backHash"], json!("#needs"), "{r}");

    let reload = |hash: &str| {
        run_page_async_with(
            &html,
            &format!(
                r#"{}
globalThis.__location.hash = "{hash}";
globalThis.__storage = {{ "t3board.layout": "workspace" }};"#,
                HASH_PRELUDE
            ),
            r#"() => [document.getElementById("app").getAttribute("data-view"), document.getElementById("app").getAttribute("data-wsnav"), document.getElementById("wn-prs").getAttribute("aria-current")]"#,
        )
    };
    assert_eq!(reload("#prs"), json!(["dispatch", "prs", "page"]), "a reload keeps the PRs view");
    assert_eq!(reload("#needs")[1], json!("needs"));

    for rule in [
        r#":root[data-layout="workspace"] .app[data-wsnav="needs"] .dgrid,"#,
        r#":root[data-layout="workspace"] .app[data-wsnav="prs"] #p-dec,"#,
        r#":root[data-layout="workspace"] .app[data-wsnav="prs"] #p-tix { display: none; }"#,
        r#":root[data-layout="workspace"] .app:not([data-wsnav="needs"]) #p-dec > .ph h2 { position: absolute;"#,
        r#":root[data-layout="workspace"] .app[data-wsnav="needs"] .card > .detail { display: block;"#,
        r#":root[data-layout="workspace"] .tr { grid-template-columns: minmax(0, 1fr) auto; column-gap: 16px; row-gap: 2px; align-items: center; padding: var(--row-pad) 18px; }"#,
        r#":root[data-layout="workspace"] .prow { gap: 2px; padding: var(--row-pad) 18px; }"#,
        r#":root[data-layout="workspace"] .app { grid-template-columns: minmax(0, 1fr); grid-template-rows: 56px auto auto minmax(0, 1fr); }"#,
        r#"grid-column: 1; grid-row: 2; flex-direction: row; gap: 4px; padding: 6px 12px; overflow-x: auto;"#,
    ] {
        assert!(html.contains(rule), "missing {rule}");
    }
    assert!(
        !html.contains(
            r#":root[data-layout="workspace"] .top .brand, :root[data-layout="workspace"] .views { display: flex; }"#
        ),
        "Workspace never brings the Dispatch view tabs back"
    );
}

#[tokio::test]
async fn the_server_default_layout_applies_until_this_browser_picks_one() {
    let (plain, _) = two_gate_page().await;
    let boot = &plain[..plain.find("</script>").unwrap()];
    assert!(boot.contains(r#"if (l !== "workspace" && l !== "dispatch") l = "dispatch";"#), "{boot}");

    let config = Config::from_value(json!({"board": {"layout": "workspace"}})).unwrap();
    let s = start(H::build(Vec::new(), config)).await;
    let html = s.http.get(format!("{}/", s.base)).send().await.unwrap().text().await.unwrap();
    assert!(!html.contains("{{LAYOUT}}"));
    let boot = &html[..html.find("</script>").unwrap()];
    assert!(
        boot.contains(r#"if (l !== "workspace" && l !== "dispatch") l = "workspace";"#),
        "the head script applies the default before paint"
    );
    let read = r#"() => [document.documentElement.getAttribute("data-layout"), document.getElementById("layout-btn").getAttribute("aria-label"), localStorage.getItem("t3board.layout")]"#;
    assert_eq!(
        run_page_async_with(&html, "", read),
        json!(["workspace", "Layout: Workspace. Switch to Dispatch", null]),
        "a browser with no pick opens in the server default"
    );
    assert_eq!(
        run_page_async_with(&html, r#"globalThis.__storage = { "t3board.layout": "dispatch" };"#, read)[0],
        json!("dispatch"),
        "this browser's pick wins over the default"
    );
    assert_eq!(
        run_page_async_with(&html, r#"globalThis.__storage = { "t3board.layout": "grid" };"#, read)[0],
        json!("workspace"),
        "an unknown saved value falls back to the default"
    );
    assert!(Config::from_value(json!({"board": {"layout": "grid"}})).is_err());
    assert_eq!(Config::default().board.layout, t3_codemode::config::BoardLayout::Dispatch);
}

#[tokio::test]
async fn cards_and_feed_filter_by_a_context_or_projects_as_the_page_does() {
    let h = H::new();
    let mut gadgets = project("p-gadgets", "/src/gadgets");
    gadgets["title"] = json!("Gadgets");
    h.fake.set_projects(vec![project("p-widgets", "/src/widgets"), gadgets]);
    h.remotes.set("/src/widgets", "git@github.com:acme/widgets.git");
    h.remotes.set("/src/gadgets", "git@github.com:acme/gadgets.git");
    h.fake.update(TICKET, json!({"projectId": "p-widgets"}));
    h.fake.update(WORKER, json!({"projectId": "p-gadgets"}));
    h.fake.set_prs(BOSS, vec![pull_request("Acme/Widgets", 90, "open")]);
    join_board(&h, TICKET, "widgets#41").await;
    join_board(&h, WORKER, "gadgets#7").await;
    join_board(&h, BOSS, "widgets#90").await;
    for who in [TICKET, WORKER, BOSS] {
        h.wf(who, "send", json!({"kind": "gate", "body": GATE})).await;
        h.wf(who, "send", json!({"kind": "note", "body": format!("{who} is green")})).await;
    }
    h.app.wf.board_tick().await.unwrap();
    for (id, name, projects) in [
        ("c-ids", "Widget work", json!(["p-widgets"])),
        ("c-repo", "Old repo", json!(["ACME/Gadgets"])),
        ("c-title", "Old title", json!(["gadgets"])),
        ("c-none", "Waiting", json!([])),
    ] {
        let body =
            serde_json::from_value(json!({"name": name, "spec": {"projects": projects, "statuses": ["waiting"]}}));
        h.app.wf.put_context(id, body.unwrap()).unwrap();
    }
    let r = result(
        &h.exec(
            r#"async () => {
const cards = async (x) => (await wf.cards(x)).map((c) => c.label + " " + (c.project && c.project.id));
const feed = async (x) => (await wf.feed(x)).map((f) => f.label);
const err = async (f) => { try { await f(); return null; } catch (e) { return String(e.message || e); } };
return {
  all: await cards(),
  ids: await cards({ context: "c-ids" }),
  byName: await cards({ context: "widget WORK" }),
  repo: await cards({ context: "c-repo" }),
  title: await feed({ context: "c-title" }),
  none: (await wf.cards({ context: "c-none" })).length,
  picked: await feed({ projects: [" p-gadgets ", "p-gizmos"] }),
  limited: await feed({ projects: ["p-widgets"], limit: 1 }),
  empty: (await wf.feed({ projects: [] })).length,
  missing: await err(() => wf.cards({ context: "c-gone" })),
  both: await err(() => wf.feed({ context: "c-ids", projects: ["p-widgets"] })),
};
}"#,
        )
        .await,
    );
    let widgets = json!(["widgets#41 p-widgets", "widgets#90 null"]);
    assert_eq!(r["all"], json!(["widgets#41 p-widgets", "gadgets#7 p-gadgets", "widgets#90 null"]), "{r}");
    assert_eq!(r["ids"], widgets, "{r}");
    assert_eq!(r["byName"], widgets, "{r}");
    assert_eq!(r["repo"], json!(["gadgets#7 p-gadgets"]), "{r}");
    assert_eq!(r["title"], json!(["gadgets#7"]), "{r}");
    assert_eq!(r["none"], 3, "{r}");
    assert_eq!(r["picked"], json!(["gadgets#7"]), "{r}");
    assert_eq!(r["limited"], json!(["widgets#90"]), "{r}");
    assert_eq!(r["empty"], 3, "{r}");
    assert_eq!(r["missing"], "no saved context with the id or name \"c-gone\"", "{r}");
    assert_eq!(r["both"], "give a context or projects, not both", "{r}");
}

#[tokio::test]
async fn the_filter_learns_a_projects_repo_from_the_items_the_page_sees_first() {
    let h = H::new();
    h.fake.set_projects(vec![project("p-widgets", "/src/widgets")]);
    h.fake.update(TICKET, json!({"projectId": "p-widgets"}));
    h.fake.update(WORKER, json!({"projectId": "p-widgets"}));
    h.fake.set_prs(TICKET, vec![pull_request("acme/old-widgets", 1, "merged")]);
    h.fake.set_prs(WORKER, vec![pull_request("acme/widgets", 2, "open")]);
    h.fake.set_prs(BOSS, vec![pull_request("acme/widgets", 3, "open")]);
    join_board(&h, TICKET, "old-widgets#1").await;
    h.wf(TICKET, "send", json!({"kind": "handoff", "body": "done"})).await;
    join_board(&h, WORKER, "widgets#2").await;
    join_board(&h, BOSS, "widgets#3").await;
    h.wf(BOSS, "send", json!({"kind": "gate", "body": GATE})).await;
    h.app.wf.board_tick().await.unwrap();
    let r = result(&h.exec("async () => (await wf.cards({ projects: ['p-widgets'] })).map((c) => c.label)").await);
    assert_eq!(r, json!(["widgets#3"]));
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
pick("projects", "p-gadgets");
const during = { cards: ids(), undo: !card("a1").querySelector('[data-part="undo"]').hidden };
advance(6000);
const sent = answers().map((f) => f.url);
card("a2").querySelector("[data-opt]").dispatch("click");
pick("projects", "p-gadgets");
pick("projects", "p-widgets");
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

#[tokio::test]
async fn a_project_whose_repo_was_unknown_is_looked_up_again_later() {
    let h = H::new();
    h.fake.set_projects(vec![project("p-widgets", "/src/widgets")]);
    h.fake.update(TICKET, json!({"projectId": "p-widgets"}));
    join_board(&h, TICKET, "widgets#41").await;
    h.app.wf.board_tick().await.unwrap();
    assert_eq!(by_label(&snapshot(&h)["tree"], "widgets#41")["repo"], Value::Null);
    h.remotes.set("/src/widgets", "git@github.com:acme/widgets.git");
    h.clock.advance_minutes(10);
    h.app.wf.board_tick().await.unwrap();
    assert_eq!(by_label(&snapshot(&h)["tree"], "widgets#41")["repo"], Value::Null);
    h.clock.advance_minutes(25);
    h.app.wf.board_tick().await.unwrap();
    assert_eq!(by_label(&snapshot(&h)["tree"], "widgets#41")["repo"], "acme/widgets");
}

#[tokio::test]
async fn a_gate_with_options_shows_exactly_those_buttons() {
    let h = H::new();
    join_board(&h, TICKET, "widgets#41").await;
    let opts = json!(["merge", " hold for review ", "close it"]);
    let r = h.wf(TICKET, "send", json!({"kind": "gate", "gate": "merge", "body": GATE, "options": opts})).await;
    assert_eq!(r["delivery"], "board");
    assert_eq!(r["options"], json!(["merge", "hold for review", "close it"]));
    assert!(r.get("warning").is_none(), "{r}");
    assert_eq!(cards(&h)[0]["options"], r["options"]);
}

#[tokio::test]
async fn a_board_send_reports_the_parsed_buttons() {
    let h = H::new();
    join_board(&h, TICKET, "widgets#41").await;
    let r = h.wf(TICKET, "send", json!({"kind": "gate", "body": GATE})).await;
    assert_eq!(r["options"], json!(["merge", "merge and deploy"]));
    assert!(r.get("warning").is_none(), "{r}");
    let r = h.wf(TICKET, "send", json!({"kind": "blocker", "body": "Need an AWS SSO login"})).await;
    assert_eq!(r["options"], json!([]));
    assert!(r.get("warning").is_none(), "a blocker needs no buttons: {r}");
}

#[tokio::test]
async fn a_gate_with_no_buttons_comes_back_with_a_warning() {
    let h = H::new();
    join_board(&h, TICKET, "widgets#41").await;
    let body = "**ACTION NEEDED:** merge 'feat/x' or hold it, here: https://github.com/o/widgets/pull/81";
    let r = h.wf(TICKET, "send", json!({"kind": "gate", "body": body})).await;
    assert_eq!(r["options"], json!([]));
    let w = r["warning"].as_str().unwrap();
    assert!(w.contains("options") && w.contains("reply"), "{w}");
    let r = h.wf(TICKET, "send", json!({"kind": "gate", "body": GATE, "options": []})).await;
    assert_eq!(r["options"], json!([]));
    assert!(r["warning"].is_string(), "{r}");
    assert_eq!(cards(&h)[0]["options"], json!([]));
}

#[tokio::test]
async fn bad_options_are_rejected_and_nothing_is_sent() {
    let h = H::new();
    join_board(&h, TICKET, "widgets#41").await;
    let long = "x".repeat(61);
    let e = h
        .wf_err(
            TICKET,
            "send",
            json!({"kind": "gate", "body": GATE, "options": ["merge", "", long, "two\nlines", "MERGE"]}),
        )
        .await;
    for want in ["options[1]", "empty", "options[2]", "60", "options[3]", "line", "options[4]", "repeats"] {
        assert!(e.contains(want), "{want:?} missing from {e}");
    }
    assert!(!e.contains("options[0]"), "{e}");
    let nine: Vec<String> = (1..=9).map(|n| format!("option {n}")).collect();
    let e = h.wf_err(TICKET, "send", json!({"kind": "gate", "body": GATE, "options": nine})).await;
    assert!(e.contains("at most 8"), "{e}");
    let e = h
        .wf_err(TICKET, "send", json!({"kind": "gate", "body": GATE, "options": ["\u{c9}t\u{e9}", "\u{e9}T\u{c9}"]}))
        .await;
    assert!(e.contains("options[1]") && e.contains("repeats"), "{e}");
    let e = h.wf_err(TICKET, "send", json!({"kind": "note", "body": "hi", "options": ["ok"]})).await;
    assert!(e.contains("gate, blocker, report"), "{e}");
    assert!(cards(&h).is_empty());
    assert!(h.wf(TICKET, "feed", json!({})).await.as_array().unwrap().is_empty());
}

fn context_names(v: &Value) -> Vec<Value> {
    v["contexts"].as_array().unwrap().iter().map(|c| c["name"].clone()).collect()
}

#[tokio::test]
async fn contexts_are_created_renamed_edited_and_deleted_and_survive_a_restart() {
    use reqwest::Method;
    let s = start(H::new()).await;
    let csrf = s.csrf();
    let spec = json!({"projects": ["p-widgets", " p-gadgets ", "P-Widgets", "p-widgets"], "statuses": ["done", "waiting"], "kinds": [], "q": "  grader "});
    let (code, made) = s.context(Method::PUT, "c1", json!({"name": " Widgets ", "spec": spec}), Some(&csrf)).await;
    assert_eq!(code, 201, "{made}");
    let made = &made["context"];
    assert_eq!((&made["id"], &made["name"]), (&json!("c1"), &json!("Widgets")));
    assert_eq!(
        made["spec"],
        json!({"projects": ["P-Widgets", "p-gadgets", "p-widgets"], "statuses": ["waiting", "done"], "kinds": [], "q": "grader"})
    );
    assert_eq!((&made["createdAt"], &made["updatedAt"]), (&json!(T0), &json!(T0)));
    s.h.clock.advance_minutes(5);
    let (code, renamed) = s.context(Method::PUT, "c1", json!({"name": "Widget work"}), Some(&csrf)).await;
    assert_eq!(code, 200, "{renamed}");
    assert_eq!(renamed["context"]["name"], "Widget work");
    assert_eq!(renamed["context"]["spec"], made["spec"]);
    assert_eq!((&renamed["context"]["createdAt"], &renamed["context"]["updatedAt"]), (&json!(T0), &json!(at(5))));
    let (code, edited) = s.context(Method::PUT, "c1", json!({"spec": {"kinds": ["gate"]}}), Some(&csrf)).await;
    assert_eq!(code, 200, "{edited}");
    assert_eq!(edited["context"]["name"], "Widget work");
    assert_eq!(edited["context"]["spec"], json!({"projects": [], "statuses": [], "kinds": ["gate"]}));
    assert_eq!(s.context(Method::PUT, "c2", json!({"name": "AI research", "spec": {}}), Some(&csrf)).await.0, 201);
    assert_eq!(context_names(&s.get("/api/contexts").await), [json!("AI research"), json!("Widget work")]);

    let s = s.restart().await;
    let listed = s.get("/api/contexts").await;
    assert_eq!(context_names(&listed), [json!("AI research"), json!("Widget work")]);
    assert_eq!(listed["contexts"][1], edited["context"]);
    assert_eq!(context_names(&s.get("/api/state").await), [json!("AI research"), json!("Widget work")]);
    let csrf = s.csrf();
    let (code, gone) = s.context(Method::DELETE, "c1", json!({}), Some(&csrf)).await;
    assert_eq!((code, &gone["deleted"]), (200, &json!("c1")), "{gone}");
    assert!(gone["rev"].is_u64(), "{gone}");
    let (code, again) = s.context(Method::DELETE, "c1", json!({}), Some(&csrf)).await;
    assert_eq!((code, &again["error"]), (404, &json!("no context \"c1\"")));

    let s = s.restart().await;
    assert_eq!(context_names(&s.get("/api/contexts").await), [json!("AI research")]);
}

#[tokio::test]
async fn context_routes_check_the_token_the_origin_and_the_input() {
    use reqwest::Method;
    let s = start(H::new()).await;
    let csrf = s.csrf();
    let ok = json!({"name": "Widgets", "spec": {}});
    assert_eq!(s.context(Method::PUT, "c1", ok.clone(), None).await.0, 403);
    assert_eq!(s.context(Method::PUT, "c1", ok.clone(), Some("nope")).await.0, 403);
    let foreign = s
        .http
        .put(format!("{}/api/contexts/c1", s.base))
        .json(&ok)
        .header("X-T3CM-CSRF", &csrf)
        .header("Origin", "https://evil.example")
        .send()
        .await
        .unwrap();
    assert_eq!(foreign.status(), 403);
    let r =
        s.http.get(format!("{}/api/contexts", s.base)).header("Origin", "https://evil.example").send().await.unwrap();
    assert_eq!(r.status(), 403);
    let long = "x".repeat(61);
    let bad = [
        ("c 1", ok.clone(), "context ids are 1 to 64"),
        ("c1", json!({"name": "  ", "spec": {}}), "a context needs a name"),
        ("c1", json!({"name": "All", "spec": {}}), "\"All\" is the built-in context"),
        ("c1", json!({"name": long, "spec": {}}), "at most 60 characters"),
        ("c1", json!({"name": "W", "spec": {"statuses": ["stuck"]}}), "unknown status \"stuck\""),
        ("c1", json!({"name": "W", "spec": {"kinds": ["note"]}}), "unknown card kind \"note\""),
        ("c1", json!({"name": "W", "spec": {"projects": [""]}}), "each project is one line"),
        ("c1", json!({"name": "W", "spec": {"project": ["a/b"]}}), "unknown field `project`"),
    ];
    for (id, body, want) in bad {
        let (code, r) = s.context(Method::PUT, id, body.clone(), Some(&csrf)).await;
        assert_eq!(code, 400, "{body}: {r}");
        assert!(r["error"].as_str().unwrap().contains(want), "{body}: {r}");
    }
    let gone = json!("no context \"c1\"; give a name and a spec to create one");
    let (code, r) = s.context(Method::PUT, "c1", json!({"name": "W"}), Some(&csrf)).await;
    assert_eq!((code, &r["error"]), (404, &gone));
    let (code, r) = s.context(Method::PUT, "c1", json!({"spec": {}}), Some(&csrf)).await;
    assert_eq!((code, &r["error"]), (404, &gone));
    let (code, r) = s.context(Method::PUT, "c1", json!({}), Some(&csrf)).await;
    assert_eq!((code, &r["error"]), (400, &json!("give a name, a spec or both")));
    assert_eq!(s.context(Method::PUT, "c1", ok.clone(), Some(&csrf)).await.0, 201);
    let (code, r) = s.context(Method::PUT, "c2", json!({"name": "WIDGETS", "spec": {}}), Some(&csrf)).await;
    assert_eq!((code, &r["error"]), (409, &json!("a context named \"Widgets\" exists")));
    assert_eq!(s.context(Method::DELETE, "c1", json!({}), None).await.0, 403);
    assert_eq!(s.context(Method::DELETE, "c9", json!({}), Some(&csrf)).await.0, 404);
    assert_eq!(context_names(&s.get("/api/contexts").await), [json!("Widgets")]);
    for n in 2..=t3_codemode::contexts::MAX_CONTEXTS {
        let body = serde_json::from_value(json!({"name": format!("n{n}"), "spec": {}})).unwrap();
        s.h.app.wf.put_context(&format!("c{n}"), body).unwrap();
    }
    let (code, r) = s.context(Method::PUT, "c999", json!({"name": "One more", "spec": {}}), Some(&csrf)).await;
    assert_eq!((code, &r["error"]), (400, &json!("the board keeps at most 100 contexts")));
    assert_eq!(s.context(Method::PUT, "c1", json!({"name": "Renamed", "spec": {}}), Some(&csrf)).await.0, 200);
}

#[tokio::test]
async fn context_names_that_match_under_full_case_folding_are_taken() {
    use reqwest::Method;
    let s = start(H::new()).await;
    let csrf = s.csrf();
    for (id, name) in [("c1", "Σ"), ("c2", "Straße")] {
        assert_eq!(s.context(Method::PUT, id, json!({"name": name, "spec": {}}), Some(&csrf)).await.0, 201);
    }
    for (id, name, taken) in
        [("c3", "ς", "Σ"), ("c3", "σ", "Σ"), ("c3", "STRASSE", "Straße"), ("c1", "strasse", "Straße")]
    {
        let (code, r) = s.context(Method::PUT, id, json!({"name": name, "spec": {}}), Some(&csrf)).await;
        assert_eq!((code, &r["error"]), (409, &json!(format!("a context named {taken:?} exists"))), "{name}");
    }
    assert_eq!(s.context(Method::PUT, "c1", json!({"name": "ς"}), Some(&csrf)).await.0, 200);
    assert_eq!(context_names(&s.get("/api/contexts").await), [json!("Straße"), json!("ς")]);
}

async fn next_state(r: &mut reqwest::Response, buf: &mut Vec<u8>) -> Value {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        while let Some(end) = buf.windows(2).position(|w| w == b"\n\n") {
            let raw: Vec<u8> = buf.drain(..end + 2).collect();
            let event = String::from_utf8(raw).unwrap();
            if event.lines().any(|l| l == "event: state") {
                let data: String = event.lines().filter_map(|l| l.strip_prefix("data: ")).collect();
                return serde_json::from_str(&data).unwrap();
            }
        }
        let chunk =
            tokio::time::timeout_at(deadline, r.chunk()).await.expect("a state event in time").unwrap().unwrap();
        buf.extend_from_slice(&chunk);
    }
}

#[tokio::test]
async fn a_saved_context_reaches_every_open_board() {
    let s = start(H::new()).await;
    let mut r = s.http.get(format!("{}/api/events", s.base)).send().await.unwrap();
    let mut buf = Vec::new();
    assert_eq!(next_state(&mut r, &mut buf).await["contexts"], json!([]));
    let body = json!({"name": "Widgets", "spec": {"projects": ["p-widgets"]}});
    assert_eq!(s.context(reqwest::Method::PUT, "c1", body, Some(&s.csrf())).await.0, 201);
    let st = next_state(&mut r, &mut buf).await;
    assert_eq!(context_names(&st), [json!("Widgets")], "{st}");
    assert_eq!(st["contexts"][0]["spec"]["projects"], json!(["p-widgets"]), "{st}");
}

#[tokio::test]
async fn every_state_carries_a_revision_that_each_context_write_reaches() {
    let s = start(H::new()).await;
    let mut r = s.http.get(format!("{}/api/events", s.base)).send().await.unwrap();
    let mut buf = Vec::new();
    let first = next_state(&mut r, &mut buf).await["rev"].as_u64().unwrap();
    let body = json!({"name": "Widgets", "spec": {"projects": ["p-widgets"]}});
    let (_, made) = s.context(reqwest::Method::PUT, "c1", body, Some(&s.csrf())).await;
    let wrote = made["rev"].as_u64().unwrap();
    assert!(wrote > first, "{made}");
    let pushed = next_state(&mut r, &mut buf).await;
    assert!(pushed["rev"].as_u64().unwrap() >= wrote, "{pushed}");
    assert_eq!(context_names(&pushed), [json!("Widgets")], "{pushed}");
    let (_, gone) = s.context(reqwest::Method::DELETE, "c1", json!({}), Some(&s.csrf())).await;
    let deleted = gone["rev"].as_u64().unwrap();
    assert!(deleted > wrote, "{gone}");
    let st = s.get("/api/state").await;
    assert!(st["rev"].as_u64().unwrap() >= deleted, "{st}");
    assert_eq!(st["contexts"], json!([]), "{st}");
    drop(r);
    s.h.clock.advance_minutes(1);
    let s = s.restart().await;
    let after = s.get("/api/state").await["rev"].as_u64().unwrap();
    assert!(after > deleted, "a restarted service keeps counting up: {after} after {deleted}");
}

const SAVED_CONTEXTS: &str = r##"[
  { id: "c1", name: "Widgets", spec: { projects: ["p-widgets"], statuses: [], kinds: [] }, createdAt: "2026-11-17T12:00:00Z", updatedAt: "2026-11-17T12:00:00Z" },
  { id: "c2", name: "Gadget gates", spec: { projects: ["p-gadgets"], statuses: [], kinds: ["gate"], q: "gadget" }, createdAt: "2026-11-17T12:00:00Z", updatedAt: "2026-11-17T12:00:00Z" },
]"##;

const CONTEXT_HELPERS: &str = r##"
const ctxBtn = () => $$("ctx-btn");
const ctxPop = () => $$("ctx-pop");
const ctxOpts = () => [...ctxPop().querySelectorAll(".ctxopt")].map((o) => [o.dataset.ctx, o.textContent, o.getAttribute("aria-pressed")]);
const openCtxMenu = () => { if (ctxPop().hidden) ctxBtn().dispatch("click"); };
const choose = (id) => { openCtxMenu(); ctxPop().querySelector('[data-ctx="' + id + '"]').dispatch("click"); };
const ctxAct = (a) => ctxPop().querySelector('[data-act="' + a + '"]');
const stored = () => localStorage.getItem("t3board.context");
const flush = async () => { for (let i = 0; i < 20; i++) await null; };
let rev = s0.rev;
const later = () => ++rev;
"##;

fn context_scenario(html: &str, st: &Value, storage: Value, body: &str) -> Value {
    context_scenario_with(html, st, storage, "", body)
}

fn context_scenario_with(html: &str, st: &Value, storage: Value, prelude: &str, body: &str) -> Value {
    run_page_async_with(
        html,
        &format!("globalThis.__storage = {storage};\n{prelude}"),
        &format!(
            "async () => {{\n{PAGE_HELPERS}\nconst base = {st};\n{}\n{CONTEXT_HELPERS}\nconst ctxs = {SAVED_CONTEXTS};\nconst s1 = {{ ...s0, contexts: ctxs }};\n{body}\n}}",
            filter_setup()
        ),
    )
}

#[tokio::test]
async fn switching_contexts_applies_their_filters_and_all_clears_them() {
    let (html, st) = two_gate_page().await;
    let r = context_scenario(
        &html,
        &st,
        json!({}),
        r##"
emitState({ ...s0, contexts: [] });
$$("settled-filter").value = "gadgets";
const before = { btn: ctxBtn().textContent, cards: ids() };
openCtxMenu();
const lonely = ctxOpts();
emitState(s1);
const live = ctxOpts();
choose("c1");
const one = { ...view(), btn: ctxBtn().textContent, stored: stored(), menu: !ctxPop().hidden, proj: facet("projects").querySelector('[data-part="fbtn"]').textContent };
choose("c2");
const two = { cards: ids(), q: $$("filter-q").value, kinds: facet("kinds").querySelector('[data-part="fbtn"]').textContent, btn: ctxBtn().textContent };
pick("statuses", "waiting");
const edited = ctxBtn().textContent;
openCtxMenu();
const acts = ["new", "update", "rename", "delete"].filter((a) => !ctxAct(a).hidden);
choose("");
const all = { ...view(), btn: ctxBtn().textContent, stored: stored(), q: $$("filter-q").value, find: $$("settled-filter").value };
pick("projects", "p-widgets");
const loose = ctxBtn().textContent;
choose("c1");
$$("filter-clear").dispatch("click");
return { before, lonely, live, one, two, edited, acts, all, loose, cleared: { btn: ctxBtn().textContent, stored: stored(), cards: ids() } };
"##,
    );
    let t = json!(TICKET);
    assert_eq!(r["before"], json!({"btn": "Context: All", "cards": ["a1", "a2"]}), "{r}");
    assert_eq!(r["lonely"], json!([["", "All", "true"]]), "{r}");
    assert_eq!(
        r["live"],
        json!([["", "All", "true"], ["c2", "Gadget gates", "false"], ["c1", "Widgets", "false"]]),
        "{r}"
    );
    assert_eq!(r["one"]["cards"], json!(["a1"]), "{r}");
    assert_eq!(r["one"]["open"], json!([t]), "{r}");
    assert_eq!(r["one"]["feed"], json!(["m91"]), "{r}");
    assert_eq!(r["one"]["btn"], "Context: Widgets", "{r}");
    assert_eq!(r["one"]["stored"], "c1", "{r}");
    assert_eq!(r["one"]["menu"], false, "{r}");
    assert_eq!(r["one"]["proj"], "Project: acme/widgets", "{r}");
    assert_eq!(
        r["two"],
        json!({"cards": ["a2"], "q": "gadget", "kinds": "Card kind: gate", "btn": "Context: Gadget gates"}),
        "{r}"
    );
    assert_eq!(r["edited"], "Context: Gadget gates (edited)", "{r}");
    assert_eq!(r["acts"], json!(["new", "update", "rename", "delete"]), "{r}");
    assert_eq!(r["all"]["cards"], json!(["a1", "a2"]), "{r}");
    assert_eq!(r["all"]["n"], json!(["2", "2", "0", "2", "2"]), "{r}");
    assert_eq!(
        (&r["all"]["btn"], &r["all"]["stored"], &r["all"]["q"]),
        (&json!("Context: All"), &Value::Null, &json!(""))
    );
    assert_eq!(r["all"]["find"], "", "{r}");
    assert_eq!(r["loose"], "Context: Unsaved filter", "{r}");
    assert_eq!(r["cleared"], json!({"btn": "Context: All", "stored": null, "cards": ["a1", "a2"]}), "{r}");
}

#[tokio::test]
async fn the_active_context_comes_back_after_a_reload_or_falls_back_to_all() {
    let (html, st) = two_gate_page().await;
    let body = r##"
emitState(s1);
return { ...view(), btn: ctxBtn().textContent, stored: stored(), q: $$("filter-q").value };
"##;
    let kept = context_scenario(&html, &st, json!({"t3board.context": "c2"}), body);
    assert_eq!(kept["cards"], json!(["a2"]), "{kept}");
    assert_eq!(
        (&kept["btn"], &kept["stored"], &kept["q"]),
        (&json!("Context: Gadget gates"), &json!("c2"), &json!("gadget"))
    );
    let stale = context_scenario(&html, &st, json!({"t3board.context": "c-gone"}), body);
    assert_eq!(stale["cards"], json!(["a1", "a2"]), "{stale}");
    assert_eq!((&stale["btn"], &stale["stored"]), (&json!("Context: All"), &Value::Null));
}

#[tokio::test]
async fn a_context_changed_or_deleted_elsewhere_updates_this_page() {
    let (html, st) = two_gate_page().await;
    let r = context_scenario(
        &html,
        &st,
        json!({"t3board.context": "c1"}),
        r##"
emitState(s1);
const first = ids();
const a3 = { ...s0.cards[0], id: "a3", at: "2026-11-17T12:30:00Z", kind: "blocker", token: "t-a3" };
emitState({ ...s1, cards: [...s1.cards, a3], contexts: [ctxs[1], { ...ctxs[0], spec: { projects: ["p-gadgets"], statuses: [], kinds: [] } }] });
const edited = { cards: ids(), btn: ctxBtn().textContent, hidden: $$("hidden-new").textContent };
const find = $$("settled-filter");
find.value = "gadgets";
find.dispatch("input");
emitState({ ...s1, contexts: [ctxs[1]] });
return { first, edited, gone: { cards: ids(), btn: ctxBtn().textContent, stored: stored(), find: find.value, settled: keysOf("settled") } };
"##,
    );
    assert_eq!(r["first"], json!(["a1"]), "{r}");
    assert_eq!(r["edited"], json!({"cards": ["a2"], "btn": "Context: Widgets", "hidden": "1 hidden"}), "{r}");
    assert_eq!(
        r["gone"],
        json!({"cards": ["a1", "a2"], "btn": "Context: All", "stored": null, "find": "", "settled": ["t:th-s", "a:q1"]}),
        "{r}"
    );
}

#[tokio::test]
async fn contexts_are_saved_renamed_edited_and_deleted_from_the_page() {
    let (html, st) = two_gate_page().await;
    let r = context_scenario(
        &html,
        &st,
        json!({}),
        r##"
emitState({ ...s0, contexts: [] });
let reply = null;
fetchReply = (url, opts) => reply(url, opts);
let lastSpec = null, lastName = null;
const echo = (status) => async (url, opts) => {
  const b = JSON.parse(opts.body);
  if (b.spec) lastSpec = b.spec;
  if (b.name) lastName = b.name;
  return { ok: true, status, json: async () => ({ context: { id: decodeURIComponent(url.split("/").pop()), name: lastName, spec: lastSpec, createdAt: "x", updatedAt: "x" }, rev: later() }) };
};
const writes = () => fetches.filter((f) => f.method !== "GET").map((f) => [f.method, f.url, f.body ? JSON.parse(f.body) : null]);
openCtxMenu();
const idle = ["new", "update", "rename", "delete"].filter((a) => !ctxAct(a).hidden);
pick("projects", "p-widgets");
openCtxMenu();
ctxAct("new").dispatch("click");
const name = ctxPop().querySelector('[data-part="cname"]');
const focused = document.activeElement === name;
name.value = "  ";
name.dispatch("keydown", { key: "Enter" });
const empty = ctxPop().querySelector('[data-part="cerr"]').textContent;
name.value = "Widgets";
reply = echo(201);
name.dispatch("keydown", { key: "Enter" });
await flush();
const made = writes()[0];
const id = made[1].split("/").pop();
const saved = { btn: ctxBtn().textContent, stored: stored() === id, closed: ctxPop().hidden, cards: ids() };
emitState({ ...s0, contexts: [{ id, name: "Widgets", spec: made[2].spec }] });
const pushed = { btn: ctxBtn().textContent, cards: ids() };
openCtxMenu();
ctxAct("rename").dispatch("click");
const prefill = name.value;
name.value = "Widget work";
ctxPop().querySelector('[data-part="cok"]').dispatch("click");
await flush();
const renamed = ctxBtn().textContent;
pick("statuses", "waiting");
openCtxMenu();
const editedActs = ["new", "update", "rename", "delete"].filter((a) => !ctxAct(a).hidden);
ctxAct("update").dispatch("click");
await flush();
const updated = ctxBtn().textContent;
openCtxMenu();
ctxAct("new").dispatch("click");
name.value = "widget WORK";
reply = async () => ({ ok: false, status: 409, json: async () => ({ error: "a context named \"Widget work\" exists" }) });
name.dispatch("keydown", { key: "Enter" });
await flush();
const taken = { err: ctxPop().querySelector('[data-part="cerr"]').textContent, open: !ctxPop().hidden, alert: $$("alert").textContent };
ctxPop().dispatch("keydown", { key: "Escape", stopPropagation() {} });
const formClosed = { form: ctxPop().querySelector('[data-part="cform"]').hidden, open: !ctxPop().hidden };
ctxAct("delete").dispatch("click");
const armed = { text: ctxAct("delete").textContent, writes: writes().length };
reply = async () => ({ ok: true, status: 200, json: async () => ({ deleted: id, rev: later() }) });
ctxAct("delete").dispatch("click");
await flush();
const deleted = { btn: ctxBtn().textContent, stored: stored(), cards: ids(), q: facet("projects").querySelector('[data-part="fbtn"]').textContent };
return { idle, focused, empty, made, saved, pushed, prefill, renamed, editedActs, updated, taken, formClosed, armed, deleted, writes: writes().slice(1).map((w) => [w[0], w[1] === made[1], w[2]]) };
"##,
    );
    assert_eq!(r["idle"], json!([]), "{r}");
    assert_eq!(r["focused"], true, "{r}");
    assert_eq!(r["empty"], "Name the context first.", "{r}");
    assert!(r["made"][1].as_str().unwrap().starts_with("/api/contexts/c"), "{r}");
    assert_eq!(r["made"][0], "PUT", "{r}");
    assert_eq!(
        r["made"][2],
        json!({"name": "Widgets", "spec": {"projects": ["p-widgets"], "statuses": [], "kinds": [], "q": ""}}),
        "{r}"
    );
    assert_eq!(r["saved"], json!({"btn": "Context: Widgets", "stored": true, "closed": true, "cards": ["a1"]}), "{r}");
    assert_eq!(r["pushed"], json!({"btn": "Context: Widgets", "cards": ["a1"]}), "{r}");
    assert_eq!(r["prefill"], "Widgets", "{r}");
    assert_eq!(r["renamed"], "Context: Widget work", "{r}");
    assert_eq!(r["editedActs"], json!(["new", "update", "rename", "delete"]), "{r}");
    assert_eq!(r["updated"], "Context: Widget work", "{r}");
    assert_eq!(r["taken"]["err"], "a context named \"Widget work\" exists", "{r}");
    assert_eq!(r["taken"]["open"], true, "{r}");
    assert_eq!(r["formClosed"], json!({"form": true, "open": true}), "{r}");
    assert_eq!(r["armed"], json!({"text": "Click again to delete Widget work", "writes": 4}), "{r}");
    assert_eq!(
        r["deleted"],
        json!({"btn": "Context: All", "stored": null, "cards": ["a1", "a2"], "q": "Project"}),
        "{r}"
    );
    let waiting = json!({"projects": ["p-widgets"], "statuses": ["waiting"], "kinds": [], "q": ""});
    assert_eq!(
        r["writes"],
        json!([
            ["PUT", true, {"name": "Widget work"}],
            ["PUT", true, {"spec": waiting}],
            ["PUT", false, {"name": "widget WORK", "spec": waiting}],
            ["DELETE", true, {}],
        ]),
        "{r}"
    );
}

#[tokio::test]
async fn a_context_saved_before_the_repo_resolves_still_matches_after_it_arrives() {
    let (html, st) = two_gate_page().await;
    let r = context_scenario(
        &html,
        &st,
        json!({}),
        r##"
const early = (x) => x.thread === T || x.ticket === T ? { ...x, repo: null } : x;
const s2 = { ...s0, tree: s0.tree.map(early), cards: s0.cards.map(early), feed: s0.feed.map(early), contexts: [] };
emitState(s2);
fetchReply = async (url, opts) => ({ ok: true, status: 201, json: async () => ({ context: { id: decodeURIComponent(url.split("/").pop()), ...JSON.parse(opts.body), createdAt: "x", updatedAt: "x" }, rev: later() }) });
facet("projects").querySelector('[data-part="fbtn"]').dispatch("click");
const opts = [...facet("projects").querySelectorAll(".fopt")].map((o) => [o.dataset.val, o.textContent]);
pick("projects", "p-widgets");
openCtxMenu();
ctxAct("new").dispatch("click");
const name = ctxPop().querySelector('[data-part="cname"]');
name.value = "Widgets";
name.dispatch("keydown", { key: "Enter" });
await flush();
const put = JSON.parse(fetches.filter((f) => f.method === "PUT")[0].body);
const before = { ...view(), btn: ctxBtn().textContent, proj: facet("projects").querySelector('[data-part="fbtn"]').textContent };
emitState({ ...s0, contexts: [{ id: stored(), name: "Widgets", spec: put.spec }] });
const after = { ...view(), btn: ctxBtn().textContent, proj: facet("projects").querySelector('[data-part="fbtn"]').textContent };
return { opts, spec: put.spec.projects, before, after };
"##,
    );
    let t = json!(TICKET);
    assert_eq!(r["opts"], json!([["p-gadgets", "acme/gadgets"], ["p-widgets", "widgets"]]), "{r}");
    assert_eq!(r["spec"], json!(["p-widgets"]), "{r}");
    assert_eq!(r["before"]["cards"], json!(["a1"]), "{r}");
    assert_eq!(r["before"]["open"], json!([t]), "{r}");
    assert_eq!(r["before"]["proj"], "Project: widgets", "{r}");
    assert_eq!(r["after"]["cards"], json!(["a1"]), "{r}");
    assert_eq!(r["after"]["open"], json!([t]), "{r}");
    assert_eq!(r["after"]["feed"], json!(["m91"]), "{r}");
    assert_eq!(r["after"]["btn"], "Context: Widgets", "{r}");
    assert_eq!(r["after"]["proj"], "Project: acme/widgets", "{r}");
}

#[tokio::test]
async fn contexts_saved_under_a_repo_or_a_title_still_match() {
    let (html, st) = two_gate_page().await;
    let r = context_scenario(
        &html,
        &st,
        json!({}),
        r##"
const old = [
  { id: "c-repo", name: "Old repo", spec: { projects: ["acme/gadgets"], statuses: [], kinds: [] } },
  { id: "c-title", name: "Old title", spec: { projects: ["widgets"], statuses: [], kinds: [] } },
  { id: "c-gone", name: "Old gizmos", spec: { projects: ["acme/gizmos", "widgets"], statuses: [], kinds: [] } },
];
emitState({ ...s0, contexts: old });
const optsOf = () => {
  if (facet("projects").querySelector('[data-part="fpop"]').hidden) facet("projects").querySelector('[data-part="fbtn"]').dispatch("click");
  return [...facet("projects").querySelectorAll(".fopt")].map((o) => [o.dataset.val, o.querySelector("input").checked]);
};
choose("c-repo");
const repo = { ...view(), btn: ctxBtn().textContent, proj: facet("projects").querySelector('[data-part="fbtn"]').textContent, opts: optsOf() };
choose("c-title");
const title = { ...view(), btn: ctxBtn().textContent, proj: facet("projects").querySelector('[data-part="fbtn"]').textContent, opts: optsOf() };
choose("c-gone");
const gone = { cards: ids(), opts: optsOf() };
pick("projects", "p-widgets");
const unpicked = { cards: ids(), opts: optsOf(), btn: ctxBtn().textContent, spec: filterSpec().projects };
return { repo, title, gone, unpicked };
"##,
    );
    let t = json!(TICKET);
    let w = json!(WORKER);
    assert_eq!(r["repo"]["cards"], json!(["a2"]), "{r}");
    assert_eq!(r["repo"]["open"], json!([w]), "{r}");
    assert_eq!(r["repo"]["btn"], "Context: Old repo", "{r}");
    assert_eq!(r["repo"]["proj"], "Project: acme/gadgets", "{r}");
    assert_eq!(r["repo"]["opts"], json!([["p-gadgets", true], ["p-widgets", false]]), "{r}");
    assert_eq!(r["title"]["cards"], json!(["a1"]), "{r}");
    assert_eq!(r["title"]["open"], json!([t]), "{r}");
    assert_eq!(r["title"]["btn"], "Context: Old title", "{r}");
    assert_eq!(r["title"]["proj"], "Project: acme/widgets", "{r}");
    assert_eq!(r["title"]["opts"], json!([["p-gadgets", false], ["p-widgets", true]]), "{r}");
    assert_eq!(r["gone"]["cards"], json!(["a1"]), "{r}");
    assert_eq!(r["gone"]["opts"], json!([["p-gadgets", false], ["acme/gizmos", true], ["p-widgets", true]]), "{r}");
    assert_eq!(r["unpicked"]["cards"], json!([]), "{r}");
    assert_eq!(
        r["unpicked"]["opts"],
        json!([["p-gadgets", false], ["acme/gizmos", true], ["p-widgets", false]]),
        "{r}"
    );
    assert_eq!(r["unpicked"]["btn"], "Context: Old gizmos (edited)", "{r}");
    assert_eq!(r["unpicked"]["spec"], json!(["acme/gizmos"]), "{r}");
}

#[tokio::test]
async fn notifications_skip_cards_outside_the_active_context() {
    let (html, st) = two_gate_page().await;
    let r = context_scenario(
        &html,
        &st,
        json!({}),
        r##"
const notes = [];
globalThis.Notification = class { constructor(title) { notes.push(title.match(/\((a\d+)\)$/)[1]); } };
Notification.permission = "granted";
document.hidden = true;
emitState(s1);
choose("c1");
const a3 = { ...s0.cards[1], id: "a3", at: "2026-11-17T12:30:00Z", kind: "blocker", token: "t-a3" };
const a4 = { ...s0.cards[0], id: "a4", at: "2026-11-17T12:31:00Z", kind: "blocker", token: "t-a4" };
emitState({ ...s1, cards: [...s1.cards, a3, a4] });
const inside = { notes: notes.slice(), hidden: $$("hidden-new").textContent, cards: ids() };
choose("");
const a5 = { ...s0.cards[1], id: "a5", at: "2026-11-17T12:32:00Z", token: "t-a5" };
emitState({ ...s1, cards: [...s1.cards, a3, a4, a5] });
return { inside, all: notes };
"##,
    );
    assert_eq!(r["inside"], json!({"notes": ["a4"], "hidden": "1 hidden", "cards": ["a4", "a1"]}), "{r}");
    assert_eq!(r["all"], json!(["a4", "a5"]), "{r}");
}

#[tokio::test]
async fn a_save_that_finishes_after_a_switch_keeps_the_new_context() {
    let (html, st) = two_gate_page().await;
    let r = context_scenario(
        &html,
        &st,
        json!({}),
        r##"
emitState(s1);
const waiting = [];
fetchReply = () => new Promise((resolve) => waiting.push(resolve));
const answer = () => {
  const f = fetches[fetches.length - 1];
  const b = JSON.parse(f.body);
  const id = f.url.split("/").pop();
  const name = b.name || ctxs.find((c) => c.id === id).name;
  waiting.shift()({ ok: true, status: b.name ? 201 : 200, json: async () => ({ context: { id, name, spec: b.spec, createdAt: "x", updatedAt: "x" }, rev: later() }) });
};
pick("projects", "p-widgets");
openCtxMenu();
ctxAct("new").dispatch("click");
const name = ctxPop().querySelector('[data-part="cname"]');
name.value = "Mine";
name.dispatch("keydown", { key: "Enter" });
choose("c2");
openCtxMenu();
ctxAct("rename").dispatch("click");
name.value = "Gadgets draft";
answer();
await flush();
const draft = { form: !ctxPop().querySelector('[data-part="cform"]').hidden, value: name.value };
ctxPop().dispatch("keydown", { key: "Escape", stopPropagation() {} });
const saved = { btn: ctxBtn().textContent, stored: stored(), cards: ids(), names: ctxOpts().map((o) => o[1]) };
choose("c1");
pick("statuses", "waiting");
openCtxMenu();
ctxAct("update").dispatch("click");
choose("c2");
pick("statuses", "waiting");
answer();
await flush();
const c1 = contexts.list.find((c) => c.id === "c1");
const updatedC1 = { name: c1.name, statuses: c1.spec.statuses };
emitState({ ...s1, contexts: [...ctxs, { id: "mine", name: "Mine", spec: { projects: ["p-widgets"], statuses: [], kinds: [] } }] });
const updated = ctxBtn().textContent;
choose("c1");
pick("statuses", "running");
openCtxMenu();
ctxAct("update").dispatch("click");
const sent = JSON.parse(fetches[fetches.length - 1].body).spec;
pick("kinds", "gate");
emitState({ ...s1, contexts: [{ ...ctxs[0], spec: sent }, ctxs[1]] });
const echoed = { kinds: facet("kinds").querySelector('[data-part="fbtn"]').textContent, btn: ctxBtn().textContent };
answer();
await flush();
return { draft, saved, updatedC1, updated, echoed };
"##,
    );
    assert_eq!(r["draft"], json!({"form": true, "value": "Gadgets draft"}), "{r}");
    assert_eq!(
        r["saved"],
        json!({"btn": "Context: Gadget gates", "stored": "c2", "cards": ["a2"], "names": ["All", "Gadget gates", "Mine", "Widgets"]}),
        "{r}"
    );
    assert_eq!(r["updatedC1"], json!({"name": "Widgets", "statuses": ["waiting"]}), "{r}");
    assert_eq!(r["updated"], "Context: Gadget gates (edited)", "{r}");
    assert_eq!(r["echoed"], json!({"kinds": "Card kind: gate", "btn": "Context: Widgets (edited)"}), "{r}");
}

#[tokio::test]
async fn a_failed_first_state_request_keeps_the_saved_context() {
    let (html, st) = two_gate_page().await;
    let r = context_scenario_with(
        &html,
        &st,
        json!({"t3board.context": "c1"}),
        r#"globalThis.__fetchReply = async () => ({ ok: false, status: 500, json: async () => ({ error: "store is busy" }) });"#,
        r##"
await flush();
const early = { stored: stored(), btn: ctxBtn().textContent };
emitState(s1);
return { early, cards: ids(), btn: ctxBtn().textContent, stored: stored() };
"##,
    );
    assert_eq!(r["early"], json!({"stored": "c1", "btn": "Context: All"}), "{r}");
    assert_eq!(r["cards"], json!(["a1"]), "{r}");
    assert_eq!((&r["btn"], &r["stored"]), (&json!("Context: Widgets"), &json!("c1")), "{r}");
}

const CONTEXT_WRITES: &str = r##"
const waiting = [];
fetchReply = () => new Promise((resolve) => waiting.push(resolve));
const lastPut = () => fetches.filter((f) => f.method === "PUT").pop();
const reply = (at, status) => {
  const f = lastPut();
  const b = JSON.parse(f.body);
  const id = decodeURIComponent(f.url.split("/").pop());
  const was = ctxs.find((c) => c.id === id);
  const context = { id, name: b.name || was.name, spec: b.spec || was.spec, createdAt: "x", updatedAt: "x" };
  waiting.shift()(status >= 400
    ? { ok: false, status, json: async () => ({ error: "a context named \"" + b.name + "\" exists" }) }
    : { ok: true, status: status || 200, json: async () => ({ context, rev: at }) });
};
const fbtn = (k) => facet(k).querySelector('[data-part="fbtn"]').textContent;
const shows = () => ({ btn: ctxBtn().textContent, statuses: fbtn("statuses"), kinds: fbtn("kinds") });
const newCtx = (n) => {
  openCtxMenu();
  ctxAct("new").dispatch("click");
  const name = ctxPop().querySelector('[data-part="cname"]');
  name.value = n;
  name.dispatch("keydown", { key: "Enter" });
};
const gateOnly = { projects: ["p-widgets"], statuses: [], kinds: ["gate"] };
"##;

fn context_writes(html: &str, st: &Value, storage: Value, body: &str) -> Value {
    context_scenario(html, st, storage, &format!("{CONTEXT_WRITES}\n{body}"))
}

fn shown(btn: &str, statuses: &str, kinds: &str) -> Value {
    json!({"btn": btn, "statuses": statuses, "kinds": kinds})
}

#[tokio::test]
async fn a_new_context_outlives_failing_snapshots_until_a_newer_one_lands() {
    let (html, st) = two_gate_page().await;
    let r = context_writes(
        &html,
        &st,
        json!({}),
        r##"
emitState(s1);
pick("projects", "p-widgets");
newCtx("Mine");
reply(later(), 201);
await flush();
const id = stored();
const saved = Date.now();
Date.now = () => saved + 6000;
advance(6000);
pick("statuses", "waiting");
emitState(s1);
openCtxMenu();
const held = { ...shows(), stored: stored() === id, menu: ctxOpts().map((o) => o[1]) };
const mine = contexts.list.find((c) => c.id === id);
emitState({ ...s1, rev, contexts: [...ctxs, mine] });
const confirmed = { ...shows(), stored: stored() === id };
emitState({ ...s1, rev: later(), contexts: ctxs });
return { held, confirmed, gone: { btn: ctxBtn().textContent, stored: stored() } };
"##,
    );
    let mut held = shown("Context: Mine (edited)", "Status: needs you", "Card kind");
    held["stored"] = json!(true);
    held["menu"] = json!(["All", "Gadget gates", "Mine", "Widgets"]);
    assert_eq!(r["held"], held, "{r}");
    assert_eq!(
        (&r["confirmed"]["btn"], &r["confirmed"]["stored"]),
        (&json!("Context: Mine (edited)"), &json!(true)),
        "{r}"
    );
    assert_eq!(r["gone"], json!({"btn": "Context: All", "stored": null}), "{r}");
}

#[tokio::test]
async fn another_browsers_edit_during_a_save_shows_once_the_save_finishes() {
    let (html, st) = two_gate_page().await;
    let r = context_writes(
        &html,
        &st,
        json!({"t3board.context": "c1"}),
        r##"
emitState(s1);
pick("statuses", "waiting");
openCtxMenu();
ctxAct("update").dispatch("click");
const mine = later();
emitState({ ...s1, rev: later(), contexts: [{ ...ctxs[0], spec: gateOnly }, ctxs[1]] });
const during = shows();
reply(mine);
await flush();
const newer = shows();

pick("statuses", "running");
openCtxMenu();
ctxAct("update").dispatch("click");
const sent = JSON.parse(lastPut().body).spec;
emitState({ ...s1, rev: later(), contexts: [{ ...ctxs[0], spec: gateOnly }, ctxs[1]] });
reply(later());
await flush();
const ours = shows();
emitState({ ...s1, rev, contexts: [{ ...ctxs[0], spec: sent }, ctxs[1]] });
const echoed = shows();

pick("statuses", "waiting");
openCtxMenu();
ctxAct("update").dispatch("click");
const stale = later();
emitState({ ...s1, rev: later(), contexts: [ctxs[1]] });
reply(stale);
await flush();
openCtxMenu();
return { during, newer, ours, echoed, deleted: { btn: ctxBtn().textContent, stored: stored(), menu: ctxOpts().map((o) => o[0]) } };
"##,
    );
    assert_eq!(r["during"], shown("Context: Widgets (edited)", "Status: needs you", "Card kind"), "{r}");
    assert_eq!(r["newer"], shown("Context: Widgets", "Status", "Card kind: gate"), "{r}");
    assert_eq!(r["ours"], shown("Context: Widgets", "Status: running", "Card kind: gate"), "{r}");
    assert_eq!(r["echoed"], r["ours"], "{r}");
    assert_eq!(r["deleted"], json!({"btn": "Context: All", "stored": null, "menu": ["", "c2"]}), "{r}");
}

#[tokio::test]
async fn an_edit_to_the_original_context_during_a_create_shows_once_it_finishes() {
    let (html, st) = two_gate_page().await;
    let r = context_writes(
        &html,
        &st,
        json!({"t3board.context": "c1"}),
        r##"
emitState(s1);
pick("statuses", "waiting");
newCtx("Waiting widgets");
emitState({ ...s1, rev: later(), contexts: [{ ...ctxs[0], spec: gateOnly }, ctxs[1]] });
const during = shows();
reply(later(), 201);
await flush();
openCtxMenu();
const made = { ...shows(), c1: ctxPop().querySelector('[data-ctx="c1"]').getAttribute("title") };

choose("c1");
pick("statuses", "running");
newCtx("Widgets");
emitState({ ...s1, rev: later(), contexts: [{ ...ctxs[0], spec: { ...gateOnly, statuses: ["done"] } }, ctxs[1]] });
reply(later(), 409);
await flush();
return { during, made, failed: shows() };
"##,
    );
    assert_eq!(r["during"], shown("Context: Widgets (edited)", "Status: needs you", "Card kind"), "{r}");
    let mut made = shown("Context: Waiting widgets", "Status: needs you", "Card kind");
    made["c1"] = json!("Project: acme/widgets; Card kind: gate");
    assert_eq!(r["made"], made, "{r}");
    assert_eq!(r["failed"], shown("Context: Widgets", "Status: done", "Card kind: gate"), "{r}");
}

#[tokio::test]
async fn an_update_stops_when_saving_the_search_box_changes_the_selection() {
    let (html, st) = two_gate_page().await;
    let r = context_writes(
        &html,
        &st,
        json!({"t3board.context": "c1"}),
        r##"
emitState(s1);
pick("statuses", "waiting");
const box = $$("filter-q");
box.value = "grader";
box.dispatch("input");
state = { ...state, contexts: [ctxs[1]] };
openCtxMenu();
ctxAct("update").dispatch("click");
await flush();
return { puts: fetches.filter((f) => f.method === "PUT").length, btn: ctxBtn().textContent, stored: stored(), q: box.value };
"##,
    );
    assert_eq!(r, json!({"puts": 0, "btn": "Context: All", "stored": null, "q": ""}), "{r}");
}

#[tokio::test]
async fn the_state_carries_iso_times_for_relative_ages() {
    let h = H::new();
    join_board(&h, TICKET, "widgets#41").await;
    h.wf(TICKET, "send", json!({"kind": "note", "body": "tests green"})).await;
    let st = snapshot(&h);
    let t = by_label(&st["tree"], "widgets#41");
    assert!(t["upAt"].as_str().is_some_and(|s| s.ends_with('Z') && s.contains('T')), "{t}");
}

#[tokio::test]
async fn a_dismissed_gate_is_marked_on_the_ticket_last_message() {
    let h = H::new();
    join_board(&h, TICKET, "widgets#41").await;
    h.wf(TICKET, "send", json!({"kind": "gate", "gate": "merge", "body": GATE})).await;
    let last = |h: &H| by_label(&snapshot(h)["tree"], "widgets#41")["last"].clone();
    assert!(last(&h).get("dismissed").is_none(), "{}", last(&h));
    let id = cards(&h)[0]["id"].as_str().unwrap().to_string();
    h.app.wf.dismiss(&id).await.unwrap();
    assert_eq!(last(&h)["dismissed"], true, "{}", last(&h));
}

const ROWS: &str = r##"
const W = (label, status, extra) => ({ thread: "w-" + label, label, status, done: status === "done", gate: null, pr: null, link: null, lastUp: "10:00 MST", ...extra });
const T = (label, extra) => ({ thread: "t-" + label, ticket: "t-" + label, label, status: "idle", done: false, gate: null, pr: null, link: null, joined: true, lastUp: "10:00 MST", last: null, workers: [], ...extra });
const row = (label) => q('#open [data-thread="t-' + label + '"]');
const groups = () => [...document.getElementById("open").children].filter((e) => e.classList.contains("tgroup")).map((g) => g.textContent);
"##;

#[tokio::test]
async fn ticket_state_comes_from_its_workers_and_rows_group_by_state() {
    let (html, st) = two_gate_page().await;
    let r = page_scenario(
        &html,
        &st,
        &format!(
            r##"{ROWS}
const c1 = {{ ...base.cards[0], ticket: "t-acme/a#1", label: "acme/a#1" }};
const tree = [
  T("acme/d#4", {{ workers: [W("#4 PR 1/1", "idle")] }}),
  T("acme/b#2", {{ workers: [W("#2 PR 1/2", "done"), W("#2 PR 2/2", "running")] }}),
  T("acme/c#3", {{ workers: [W("#3 PR 1/1", "failed")] }}),
  T("acme/a#1", {{ gate: "merge", workers: [W("#1 PR 1/1", "idle", {{ gate: "merge" }})] }}),
  T("acme/e#5", {{ workers: [W("#5 PR 1/1", "waiting")] }}),
];
emitState({{ ...base, cards: [c1], tree, counts: {{ ...base.counts, cards: 1, running: 0 }} }});
const keys = [...document.getElementById("open").children].filter((e) => e.dataset.key).map((e) => e.dataset.key);
return {{
  keys, groups: groups(),
  states: keys.map((k) => q('#open [data-key="' + k + '"]').dataset.state),
  marks: ["acme/a#1", "acme/b#2"].map((l) => row(l).querySelector(".mk .mark").className),
  worker: row("acme/a#1").querySelector(".wi .mark").className,
  running: String(document.querySelectorAll("#open .stp").filter((p) => p.textContent === "Running").length),
  need: document.getElementById("vn-dispatch").textContent,
  lead: (() => {{
    emitState({{ ...base, cards: [c1, {{ ...base.cards[1], id: "a9", kind: "blocker", ticket: "t-acme/a#1", label: "acme/a#1" }}], tree, counts: {{ ...base.counts, cards: 2, running: 0 }} }});
    return row("acme/a#1").querySelector('[data-part="needs"]').textContent;
  }})(),
}};
"##
        ),
    );
    assert_eq!(r["keys"], json!(["t-acme/a#1", "t-acme/c#3", "t-acme/b#2", "t-acme/e#5", "t-acme/d#4"]), "{r}");
    assert_eq!(r["states"], json!(["need", "fail", "run", "wait", "idle"]), "{r}");
    assert_eq!(r["lead"], "Blocked +1", "a newer blocker leads the row like it leads the cards: {r}");
    assert_eq!(r["groups"], json!(["Needs you1", "Failed1", "Running1", "Waiting1", "Idle1"]), "{r}");
    assert_eq!(r["marks"], json!(["mark s-need", "mark s-run"]), "{r}");
    assert_eq!(r["worker"], "mark s-need", "{r}");
    assert_eq!((&r["running"], &r["need"]), (&json!("1"), &json!("1")), "{r}");
}

#[tokio::test]
async fn rows_show_a_readable_title_and_say_when_a_gate_was_answered() {
    let (html, st) = two_gate_page().await;
    let r = page_scenario(
        &html,
        &st,
        &format!(
            r##"{ROWS}
const tree = [
  T("acme/a#1", {{ title: "acme/a#1: Fix the grader" }}),
  T("acme/b#2", {{ gate: "acme/api#86_owns_the_next_deploy" }}),
  T("acme/c#3", {{ title: "acme/c#3: https://github.com/acme/c/issues/3", gate: "windows-signing",
    last: {{ kind: "gate", head: '**ACTION NEEDED:** reply "approve" to approve the 2-PR split, or reply with changes, here: https://github.com/acme/c/issues/3', at: "10:00 MST" }} }}),
  T("acme/e#5", {{ last: {{ kind: "gate", dismissed: true, head: '**ACTION NEEDED:** reply "merge" to merge #12, here: https://github.com/acme/e/pull/12', at: "10:00 MST" }} }}),
  T("acme/d#4", {{ last: {{ kind: "event", head: "PR 1/2 opened: https://github.com/acme/d/pull/9, head 0123456789abcdef0123 Publishing: template", at: "10:00 MST" }} }}),
];
emitState({{ ...base, cards: [], tree }});
const text = (l, p) => row(l).querySelector('[data-part="' + p + '"]').textContent;
return {{
  titles: ["acme/a#1", "acme/b#2", "acme/c#3"].map((l) => text(l, "title")),
  answered: text("acme/c#3", "note"),
  dismissed: text("acme/e#5", "note"),
  needsHidden: row("acme/c#3").querySelector('[data-part="needs"]').hidden,
  state: row("acme/c#3").dataset.state,
  note: text("acme/d#4", "note"),
}};
"##
        ),
    );
    assert_eq!(r["titles"], json!(["Fix the grader", "Acme/api#86 owns the next deploy", "Windows signing"]), "{r}");
    assert_eq!(r["answered"], "Answered: approve the 2-PR split", "{r}");
    assert_eq!(r["dismissed"], "Dismissed: merge #12", "{r}");
    assert_eq!((&r["needsHidden"], &r["state"]), (&json!(true), &json!("idle")), "{r}");
    assert_eq!(r["note"], "PR 1/2 opened: #9, head 0123456", "{r}");
}

#[tokio::test]
async fn a_card_leads_with_the_ask_and_keeps_the_context_below_it() {
    let (html, st) = two_gate_page().await;
    let r = page_scenario(
        &html,
        &st,
        r##"
const one = { ...base.cards[0], options: ["merge"], text: 'reply "merge" to merge #64, or review it first, here: https://github.com/o/widgets/pull/64 . Verified: approved at 905fcaa7d21dfe19.' };
const blocker = { ...base.cards[1], kind: "blocker", options: [], text: base.cards[1].label + " worker cannot reach the registry; fix the token, here: https://github.com/o/widgets/actions/runs/7" };
emitState({ ...base, cards: [one, blocker] });
const p = (id, part) => card(id).querySelector('[data-part="' + part + '"]');
const out = {
  order: ids(),
  ask: [p("a1", "ask").textContent, p("a2", "ask").textContent],
  word: [p("a1", "kw").hidden, p("a2", "kw").textContent],
  detail: p("a1", "detail").textContent,
  review: p("a1", "rev").textContent,
  open: p("a2", "rev").textContent,
  cls: card("a2").className,
  title: p("a1", "q").getAttribute("title").slice(0, 13),
  label: p("a1", "id").textContent,
};
emitState({ ...base, cards: [{ ...one, text: 'reopen #81, then reply "merge" to merge it, here: https://github.com/o/widgets/pull/81' }] });
out.then = p("a1", "ask").textContent;
emitState({ ...base, cards: [{ ...one, options: [], text: "**ACTION NEEDED:** start the code-mode service, here: `t3cm serve`, then reply once it is healthy" }] });
out.cmd = p("a1", "ask").textContent;
emitState({ ...base, cards: [{ ...one, text: 'reply "merge" to merge #81, here: `https://github.com/o/widgets/pull/81`.' }] });
const rev = p("a1", "rev").querySelector("a");
out.href = rev && rev.getAttribute("href");
emitState({ ...base, cards: [{ ...one, options: [], text: "Restart job 12345678 after the fix" }] });
out.num = p("a1", "ask").textContent;
emitState({ ...base, cards: [{ ...one, options: [], text: "Rebase head 390604428 (390604428) onto main" }] });
out.digits = p("a1", "ask").textContent;
emitState({ ...base, cards: [{ ...one, options: [], text: "**ACTION NEEDED:** run the check here: npm test" }] });
out.bare = p("a1", "ask").textContent;
emitState({ ...base, cards: [{ ...one, options: [], text: "**ACTION NEEDED:** run `printf '%s' deadbeef0123` on the host" }] });
out.code = p("a1", "ask").textContent;
return out;
"##,
    );
    assert_eq!(r["order"], json!(["a2", "a1"]), "{r}");
    assert_eq!(r["ask"], json!(["Merge #64", "Worker cannot reach the registry; fix the token"]), "{r}");
    assert_eq!(r["word"], json!([true, "Blocked"]), "{r}");
    assert_eq!(r["detail"], "Verified: approved at 905fcaa.", "{r}");
    assert_eq!((&r["review"], &r["open"]), (&json!("Review #64"), &json!("Open link")), "{r}");
    assert_eq!(r["cls"], "card k-fail", "{r}");
    assert_eq!(r["title"], "reply \"merge\"", "{r}");
    assert_eq!(r["label"], "widgets#41", "{r}");
    assert_eq!(r["then"], "Reopen #81, then reply \"merge\" to merge it", "{r}");
    assert_eq!(r["cmd"], "Start the code-mode service: t3cm serve, then reply once it is healthy", "{r}");
    assert_eq!(r["href"], "https://github.com/o/widgets/pull/81", "{r}");
    assert_eq!(r["num"], "Restart job 12345678 after the fix", "{r}");
    assert_eq!(r["digits"], "Rebase head 3906044 (3906044) onto main", "{r}");
    assert_eq!(r["bare"], "Run the check here: npm test", "{r}");
    assert_eq!(r["code"], "Run printf '%s' deadbeef0123 on the host", "inline code keeps its hex: {r}");
}

#[tokio::test]
async fn activity_groups_runs_and_shows_the_message_without_its_kind() {
    let (html, st) = two_gate_page().await;
    let r = page_scenario(
        &html,
        &st,
        r##"
const F = (n, label, kind, head) => ({ handle: "m" + n, at: "10:0" + n + " MST", iso: "2026-11-17T17:0" + n + ":00Z", label, kind, head, ticket: null, ticketLabel: "acme/w#7", ticketTitle: null });
const feed = [
  F(5, "#7 PR 3/3", "event", "PR 3/3 updated: #12, head 4f2a9c1e0b9d3d6a1f00c2b7e1a8d55e2c9f0a11. Fixed both findings"),
  F(4, "#7 PR 3/3", "note", "#12: review requested changes"),
  F(3, "#7 PR 3/3", "event", "PR 3/3 opened: https://github.com/acme/w/pull/12, head 1c0ffee Publishing: template filled"),
  F(2, "acme/w#7", "note", "split approved"),
];
emitState({ ...base, feed });
const li = () => [...document.getElementById("feed").children];
const first = li()[0];
const before = { rows: li().map((e) => e.dataset.key), who: first.querySelector(".who").textContent, sub: first.querySelector(".fsub").textContent,
  msg: first.querySelector(".msg").textContent, more: first.querySelector(".more").textContent, earlier: first.querySelector(".earlier").hidden };
first.querySelector(".more").dispatch("click");
const open = [...first.querySelector(".earlier").children].map((e) => e.querySelector(".msg").textContent);
emitState({ ...base, feed });
const kept = { still: !first.querySelector(".earlier").hidden, toggle: first.querySelector(".more").getAttribute("aria-expanded") };
emitState({ ...base, feed: feed.filter((f) => f.handle !== "m3") });
kept.evicted = !first.querySelector(".earlier").hidden;
emitState({ ...base, feed: [feed[0], { ...feed[1], ticket: "t-x", ticketLabel: "acme/x#7" }] });
return { before, open, ...kept, split: li().length };
"##,
    );
    assert_eq!(r["before"]["rows"], json!(["m5", "m2"]), "{r}");
    assert_eq!((&r["before"]["who"], &r["before"]["sub"]), (&json!("acme/w#7"), &json!("PR 3/3")), "{r}");
    assert_eq!(r["before"]["msg"], "Updated: #12, head 4f2a9c1. Fixed both findings", "{r}");
    assert_eq!(r["before"]["more"], "+ 2 earlier from PR 3/3", "{r}");
    assert_eq!(r["before"]["earlier"], true, "{r}");
    assert_eq!(r["open"], json!(["#12: review requested changes", "Opened: #12, head 1c0ffee"]), "{r}");
    assert_eq!((&r["still"], &r["toggle"]), (&json!(true), &json!("true")), "{r}");
    assert_eq!(r["split"], 2, "same worker label on another ticket must not fold: {r}");
    assert_eq!(r["evicted"], true, "an open group stays open when its oldest row leaves the feed: {r}");
}

#[tokio::test]
async fn an_age_tooltip_names_the_zone_its_clock_reads_in() {
    let (html, st) = two_gate_page().await;
    let r = page_scenario(
        &html,
        &st,
        &format!(
            r##"{ROWS}
const tree = [T("acme/a#1", {{ status: "running", lastUp: "16:21 MST", upAt: "2026-11-18T23:21:00Z" }}), T("acme/b#2", {{ lastUp: "16:00 MST" }})];
const tip = () => row("acme/a#1").querySelector('[data-part="tm"]').getAttribute("title");
globalThis.Intl = {{ DateTimeFormat: function () {{ return {{ formatToParts: () => [{{ type: "hour", value: "23" }}, {{ type: "timeZoneName", value: "UTC" }}] }}; }} }};
emitState({{ ...base, cards: [], tree }});
const browser = tip();
delete globalThis.Intl;
emitState({{ ...base, cards: [], tree: [{{ ...tree[0], upAt: "2026-11-18T23:22:00Z" }}, tree[1]] }});
return {{ browser, bare: tip() }};
"##
        ),
    );
    let browser = r["browser"].as_str().unwrap_or_default();
    assert!(browser.ends_with(" UTC"), "the label names the browser zone the time is read in: {r}");
    let bare = r["bare"].as_str().unwrap_or_default();
    assert!(!bare.contains("MST") && !bare.contains("UTC"), "no zone label without Intl: {r}");
}

#[tokio::test]
async fn the_check_age_in_the_header_keeps_counting() {
    let (html, st) = two_gate_page().await;
    let r = page_scenario(
        &html,
        &st,
        r##"
const t0 = Date.parse("2026-11-19T17:00:00Z");
Date.now = () => t0;
const service = { ...base.service, watcher: { ...base.service.watcher, alive: true, lastPassAt: "2026-11-19T17:00:00Z" } };
emitState({ ...base, service });
const fresh = document.getElementById("svc-text").textContent;
Date.now = () => t0 + 5 * 60000;
for (const f of intervals) f();
return { fresh, later: document.getElementById("svc-text").textContent };
"##,
    );
    assert!(r["fresh"].as_str().unwrap_or_default().contains("checked just now"), "{r}");
    assert!(r["later"].as_str().unwrap_or_default().contains("checked 5m ago"), "the timer repaints the age: {r}");
}

#[tokio::test]
async fn group_headings_wait_while_the_ticket_list_is_held() {
    let (html, st) = two_gate_page().await;
    let r = page_scenario(
        &html,
        &st,
        &format!(
            r##"{ROWS}
const tree = [T("acme/a#1", {{ status: "running" }}), T("acme/b#2", {{ status: "running" }})];
emitState({{ ...base, cards: [], tree }});
const shape = () => [...document.getElementById("open").children].map((e) => e.dataset.key || e.textContent);
const before = shape();
row("acme/b#2").focus();
const c1 = {{ ...base.cards[0], ticket: "t-acme/a#1", label: "acme/a#1" }};
emitState({{ ...base, cards: [c1], tree }});
const held = shape();
document.activeElement = document.body;
emitState({{ ...base, cards: [c1], tree }});
return {{ before, held, live: shape() }};
"##
        ),
    );
    assert_eq!(r["held"], r["before"], "{r}");
    assert_eq!(r["live"], json!(["Needs you1", "t-acme/a#1", "Running1", "t-acme/b#2"]), "{r}");
}

#[tokio::test]
async fn done_workers_fold_into_a_count_that_expands() {
    let (html, st) = two_gate_page().await;
    let r = page_scenario(
        &html,
        &st,
        &format!(
            r##"{ROWS}
const tree = [T("acme/b#2", {{ workers: [W("#2 PR 1/4", "done"), W("#2 PR 2/4", "done"), W("#2 PR 3/4", "idle"), W("#2 PR 4/4", "running")] }})];
emitState({{ ...base, cards: [], tree }});
const shown = () => [...row("acme/b#2").querySelectorAll(".wl .wi")].map((w) => w.querySelector('[data-part="wth"]').textContent);
const more = row("acme/b#2").querySelector('[data-part="wmore"]');
const done = row("acme/b#2").querySelector('[data-part="wdone"]');
const before = {{ shown: shown(), more: more.textContent, done: done.textContent }};
more.dispatch("click");
const after = {{ shown: shown(), less: more.textContent, expanded: more.getAttribute("aria-expanded"), doneHidden: done.hidden }};
more.dispatch("click");
done.dispatch("click");
const viaDone = shown().length;
const focusMoved = document.activeElement === more;
more.dispatch("click");
return {{ before, after, viaDone, focusMoved, focusStays: document.activeElement === more && !more.hidden }};
"##
        ),
    );
    assert_eq!(r["before"], json!({"shown": ["PR 4/4"], "more": "+1 worker", "done": "2 done"}), "{r}");
    assert_eq!(
        r["after"],
        json!({"shown": ["PR 1/4", "PR 2/4", "PR 3/4", "PR 4/4"], "less": "Less", "expanded": "true", "doneHidden": true}),
        "{r}"
    );
    assert_eq!(r["viaDone"], 4, "{r}");
    assert_eq!((&r["focusMoved"], &r["focusStays"]), (&json!(true), &json!(true)), "{r}");
}

async fn new_issue_page() -> (String, Value) {
    let s = start(H::new()).await;
    join_board(&s.h, TICKET, "widgets#41").await;
    let report = "async () => wf.issue({ url: 'https://github.com/Acme/widgets/issues/43', title: 'Fix the grader', note: 'found while testing' })";
    result(&s.h.exec_as(WORKER, report).await);
    result(&s.h.exec_as(WORKER, "async () => wf.issue({ url: 'Acme/gadgets#7' })").await);
    let html = s.http.get(format!("{}/", s.base)).send().await.unwrap().text().await.unwrap();
    let st: Value = s.http.get(format!("{}/api/state", s.base)).send().await.unwrap().json().await.unwrap();
    (html, st)
}

const NEW_HELPERS: &str = r##"
const row = (id) => q('#nis [data-key="' + id + '"]');
const rows = () => [...document.getElementById("nis").children].map((r) => r.dataset.key);
const btn = (id, p) => row(id).querySelector('[data-part="' + p + '"]');
const posts = () => fetches.filter((f) => f.method === "POST").map((f) => f.url);
const sec = () => document.getElementById("p-new");
const flush = async () => { for (let i = 0; i < 20; i++) await null; };
const answer = (body) => async () => ({ ok: true, json: async () => body });
"##;

fn new_scenario(html: &str, st: &Value, body: &str) -> Value {
    run_page_async(html, &format!("async () => {{\n{PAGE_HELPERS}\n{NEW_HELPERS}\nconst base = {st};\n{body}\n}}"))
}

#[tokio::test]
async fn a_project_merges_what_every_item_knows_and_catches_repo_only_issues() {
    let (html, st) = new_issue_page().await;
    let r = run_page_async(
        &html,
        &format!(
            r##"async () => {{
{PAGE_HELPERS}
{NEW_HELPERS}
{FILTER_HELPERS}
const base = {st};
const proj = {{ id: "p-widgets", title: "widgets" }};
const s0 = {{
  ...base,
  tree: base.tree.map((t) => ({{ ...t, repo: null, project: proj }})),
  feed: [{{ handle: "m91", at: "10:00 MST", iso: "2026-11-17T17:00:00Z", label: "widgets#41", kind: "note", head: "tests green", ticket: base.tree[0].thread, repo: "acme/widgets", project: proj }}],
  contexts: [{{ id: "c-old", name: "Old", spec: {{ projects: ["acme/widgets"], statuses: [], kinds: [] }} }}],
}};
emitState(s0);
const all = rows();
facet("projects").querySelector('[data-part="fbtn"]').dispatch("click");
const opts = [...facet("projects").querySelectorAll(".fopt")].map((o) => [o.dataset.val, o.textContent]);
pick("projects", "p-widgets");
const picked = rows();
pick("projects", "p-widgets");
$$("ctx-btn").dispatch("click");
$$("ctx-pop").querySelector('[data-ctx="c-old"]').dispatch("click");
facet("projects").querySelector('[data-part="fbtn"]').dispatch("click");
const legacy = [...facet("projects").querySelectorAll(".fopt")].map((o) => [o.dataset.val, o.querySelector("input").checked]);
return {{ all, opts, picked, legacy, legacyRows: rows(), tickets: [...keysOf("open"), ...keysOf("done")], feed: keysOf("feed") }};
}}"##
        ),
    );
    assert_eq!(r["all"].as_array().unwrap().len(), 2, "{r}");
    assert_eq!(r["opts"], json!([["p-widgets", "acme/widgets"]]), "{r}");
    assert_eq!(r["picked"].as_array().unwrap().len(), 1, "{r}");
    assert_eq!(r["legacy"], json!([["p-widgets", true]]), "{r}");
    assert_eq!(r["legacyRows"], r["picked"], "{r}");
    assert_eq!(r["tickets"], json!([TICKET]), "{r}");
    assert_eq!(r["feed"], json!(["m91"]), "{r}");
}

#[tokio::test]
async fn the_event_stream_pushes_an_issue_as_soon_as_a_thread_reports_it() {
    let s = start(H::new()).await;
    let mut r = s.http.get(format!("{}/api/events", s.base)).send().await.unwrap();
    let mut buf = Vec::new();
    assert_eq!(next_state(&mut r, &mut buf).await["newIssues"], json!([]));
    result(&s.h.exec_as(WORKER, "async () => wf.issue({ url: 'Acme/widgets#43', title: 'Fix the grader' })").await);
    let st = next_state(&mut r, &mut buf).await;
    assert_eq!(st["newIssues"][0]["label"], "widgets#43", "{st}");
    assert_eq!(st["counts"]["newIssues"], 1, "{st}");
}

#[tokio::test]
async fn new_issues_get_their_own_view_with_their_links_and_a_count() {
    let (html, st) = new_issue_page().await;
    assert!(html.find(r#"id="view-issues""#).unwrap() < html.find(r#"id="p-new""#).unwrap());
    let r = new_scenario(
        &html,
        &st,
        r##"
emitState(base);
const one = row("n1");
const label = btn("n1", "label").querySelector("a");
const thread = btn("n1", "th").querySelector("a");
const shown = {
  rows: rows(), chip: q("#n-new").textContent, empty: sec().hasAttribute("data-empty"),
  label: [label.textContent, label.getAttribute("href")], title: btn("n1", "title").textContent,
  verb: btn("n1", "verb").textContent, at: btn("n1", "when").getAttribute("data-at"),
  thread: [thread.textContent, thread.getAttribute("href")],
  names: [btn("n1", "launch").getAttribute("aria-label"), btn("n1", "dismiss").getAttribute("aria-label")],
  gadget: btn("n2", "title").textContent, view: q("#new-view").hidden,
};
emitState({ ...base, newIssues: [], counts: { ...base.counts, newIssues: 0 } });
return { shown, after: { rows: rows(), chip: q("#n-new").textContent, empty: sec().hasAttribute("data-empty") } };
"##,
    );
    assert_eq!(r["shown"]["rows"], json!(["n2", "n1"]), "{r}");
    assert_eq!(r["shown"]["chip"], "2", "{r}");
    assert_eq!(r["shown"]["empty"], false, "{r}");
    assert_eq!(r["shown"]["label"], json!(["widgets#43", "https://github.com/Acme/widgets/issues/43"]), "{r}");
    assert_eq!(r["shown"]["title"], "Fix the grader", "{r}");
    assert_eq!(r["shown"]["verb"], "filed", "{r}");
    assert_eq!(r["shown"]["at"], T0, "{r}");
    assert_eq!(r["shown"]["thread"], json!([WORKER, format!("https://t3/{WORKER}")]), "{r}");
    assert_eq!(r["shown"]["names"], json!(["Launch a ticket for widgets#43", "Dismiss widgets#43"]), "{r}");
    assert_eq!(r["shown"]["gadget"], "", "{r}");
    assert_eq!(r["shown"]["view"], true, "{r}");
    assert_eq!(r["after"], json!({"rows": [], "chip": "", "empty": true}), "{r}");
}

#[tokio::test]
async fn a_covered_new_issue_names_what_covers_it_and_launch_asks_twice() {
    let (html, st) = new_issue_page().await;
    let r = new_scenario(
        &html,
        &st,
        r##"
const pr = (n, state) => ({ repository: "acme/widgets", number: n, url: "https://github.com/acme/widgets/pull/" + n, state });
const cov = {
  n1: [{ kind: "merged", ticket: "widgets#50", ticketThread: "t-50", worker: "#50 PR 1/3", thread: "w-1", pr: pr(84, "merged") }],
  n2: [{ kind: "pr", ticket: "widgets#50", ticketThread: "gone", worker: "#50 PR 2/3", thread: "w-2", pr: pr(83, "open") },
       { kind: "worker", ticket: "widgets#50", ticketThread: "t-50", worker: "#50 PR 3/3", thread: "w-3", pr: null }],
};
const tree = [{ thread: "t-50", ticket: "t-50", label: "widgets#50", status: "running", done: false, workers: [], joined: true, last: null, link: null }];
emitState({ ...base, tree, newIssues: base.newIssues.map((i) => ({ ...i, coveredBy: cov[i.id] || [] })) });
const badges = (id) => [...btn(id, "cov").children].map((b) => [b.tagName, b.textContent, b.getAttribute("href")]);
const shown = { n1: badges("n1"), n2: badges("n2"), hidden: btn("n1", "cov").hidden,
  launch: [btn("n1", "launch").textContent, btn("n1", "launch").getAttribute("aria-label")], covered: row("n1").classList.contains("covered") };
btn("n1", "cov").firstChild.dispatch("click");
const d = document.getElementById("drawer");
const opened = { open: d.classList.contains("open"), sub: dr.sub && dr.sub.key, worker: dr.sub && dr.sub.worker };
closeDrawer(false);
emitState({ ...base, tree, newIssues: base.newIssues.map((i) => ({ ...i, coveredBy: [] })) });
const plain = { hidden: btn("n1", "cov").hidden, launch: btn("n1", "launch").textContent, covered: row("n1").classList.contains("covered") };
emitState({ ...base, tree, newIssues: base.newIssues.map((i) => ({ ...i, coveredBy: cov[i.id] || [] })) });
openDrawer({ kind: "new", key: "n1" });
const part = (n) => d.querySelector('[data-part="' + n + '"]');
const drawer = { cover: part("cover").hidden, n: part("cover-n").textContent, launch: part("launch").textContent };
return { shown, opened, plain, drawer };
"##,
    );
    let s = &r["shown"];
    assert_eq!(
        s["n1"],
        json!([["BUTTON", "Fixed by PR #84", null]]),
        "a ticket on the board opens in the inspector: {r}"
    );
    assert_eq!(
        s["n2"],
        json!([
            ["A", "PR #83 open", "https://github.com/acme/widgets/pull/83"],
            ["BUTTON", "Running in #50 PR 3/3", null]
        ]),
        "{r}"
    );
    assert_eq!(s["hidden"], false, "{r}");
    assert_eq!(
        s["launch"],
        json!([
            "Launch anyway",
            "Launch another ticket for widgets#43, already covered: Fixed by PR #84 from #50 PR 1/3"
        ]),
        "{r}"
    );
    assert_eq!(s["covered"], true, "{r}");
    assert_eq!(r["opened"], json!({"open": true, "sub": "t-50", "worker": "w-1"}), "the badge opens its worker: {r}");
    assert_eq!(r["plain"], json!({"hidden": true, "launch": "Launch", "covered": false}), "{r}");
    assert_eq!(r["drawer"], json!({"cover": false, "n": "1", "launch": "Launch anyway"}), "{r}");
}

#[tokio::test]
async fn launch_and_dismiss_post_to_the_entry_and_a_failed_launch_says_why() {
    let (html, st) = new_issue_page().await;
    let r = new_scenario(
        &html,
        &st,
        r##"
emitState(base);
fetchReply = answer({ ok: true, label: "widgets#43", thread: "launch:1" });
btn("n1", "launch").dispatch("click");
const busy = [btn("n1", "launch").textContent, btn("n1", "launch").getAttribute("aria-disabled")];
await flush();
const launched = { posts: posts(), err: q("#new-err").hidden, label: btn("n1", "launch").textContent };
fetchReply = answer({ ok: true });
btn("n2", "dismiss").dispatch("click");
await flush();
const dismissed = posts().slice(1);
fetchReply = answer({ ok: false, label: "widgets#43", error: "Cannot launch widgets#43: no T3 project", card: "a3" });
btn("n1", "launch").dispatch("click");
await flush();
const failed = { err: q("#new-err").hidden, text: q("#new-err-text").textContent, enabled: btn("n1", "launch").getAttribute("aria-disabled") };
return { busy, launched, dismissed, failed };
"##,
    );
    assert_eq!(r["busy"], json!(["Launching...", "true"]), "{r}");
    assert_eq!(r["launched"], json!({"posts": ["/api/new-issues/n1/launch"], "err": true, "label": "Launch"}), "{r}");
    assert_eq!(r["dismissed"], json!(["/api/new-issues/n2/dismiss"]), "{r}");
    assert_eq!(
        r["failed"],
        json!({"err": false, "text": "Cannot launch widgets#43: no T3 project (card a3)", "enabled": null}),
        "{r}"
    );
}

#[tokio::test]
async fn a_launched_issue_leaves_a_placeholder_while_the_reader_is_in_the_column() {
    let (html, st) = new_issue_page().await;
    let r = new_scenario(
        &html,
        &st,
        r##"
emitState(base);
q("#p-new").dispatch("pointerenter");
fetchReply = answer({ ok: true, label: "widgets#43" });
btn("n1", "launch").dispatch("click");
await flush();
emitState({ ...base, newIssues: base.newIssues.filter((i) => i.id !== "n1") });
const held = { rows: rows(), note: row("n1").textContent, chip: q("#n-new").textContent };
q("#p-new").dispatch("pointerleave");
advance(10);
return { held, after: rows() };
"##,
    );
    assert_eq!(r["held"], json!({"rows": ["n2", "n1"], "note": "widgets#43 launched", "chip": "1"}), "{r}");
    assert_eq!(r["after"], json!(["n2"]), "{r}");
}

#[tokio::test]
async fn a_new_issue_waits_behind_the_count_while_focus_is_in_new_issues() {
    let (html, st) = new_issue_page().await;
    let r = new_scenario(
        &html,
        &st,
        r##"
const one = { ...base, newIssues: base.newIssues.filter((i) => i.id === "n1"), counts: { ...base.counts, newIssues: 1 } };
sec().append(q("#nis"));
selectView("issues");
emitState(one);
btn("n1", "launch").focus();
emitState(base);
const held = { rows: rows(), slot: q("#new-nis").textContent, off: q("#new-nis").classList.contains("off"),
  focus: document.activeElement === btn("n1", "launch") };
q("#new-nis").dispatch("click");
return { held, after: rows(), off: q("#new-nis").classList.contains("off") };
"##,
    );
    assert_eq!(r["held"], json!({"rows": ["n1"], "slot": "1 new", "off": false, "focus": true}), "{r}");
    assert_eq!(r["after"], json!(["n2", "n1"]), "{r}");
    assert_eq!(r["off"], true, "{r}");
}

#[tokio::test]
async fn show_dismissed_lists_dismissed_issues_and_restore_brings_one_back() {
    let s = start(H::new()).await;
    result(&s.h.exec_as(WORKER, "async () => wf.issue({ url: 'Acme/widgets#43', title: 'Fix the grader' })").await);
    result(&s.h.exec_as(WORKER, "async () => wf.issue({ url: 'Acme/widgets#44' })").await);
    s.h.app.wf.dismiss_new_issue("n1").await.unwrap();
    let html = s.http.get(format!("{}/", s.base)).send().await.unwrap().text().await.unwrap();
    let st: Value = s.http.get(format!("{}/api/state", s.base)).send().await.unwrap().json().await.unwrap();
    let r = new_scenario(
        &html,
        &st,
        r##"
emitState(base);
const open = { rows: rows(), view: [q("#new-view").hidden, q("#new-view-t").textContent + " " + q("#n-dismissed").textContent, q("#new-view").getAttribute("aria-pressed")] };
q("#new-view").dispatch("click");
const view = {
  rows: rows(), head: q("#h-new-t").textContent, chip: q("#n-new").textContent,
  launch: btn("n1", "launch").hidden, restore: [btn("n1", "dismiss").textContent, btn("n1", "dismiss").getAttribute("aria-label")],
  verb: btn("n1", "verb").textContent, toggle: [q("#new-view-t").textContent, q("#n-dismissed").textContent, q("#new-view").getAttribute("aria-pressed")],
};
fetchReply = answer({ id: "n1", dismissed: false });
btn("n1", "dismiss").dispatch("click");
await flush();
const restored = { ...base.dismissedIssues[0], dismissed: false, dismissedAt: null };
emitState({ ...base, newIssues: [restored, ...base.newIssues], dismissedIssues: [] });
return { open, view, posts: posts(), back: { rows: rows(), head: q("#h-new-t").textContent, toggle: q("#new-view").hidden } };
"##,
    );
    assert_eq!(r["open"], json!({"rows": ["n2"], "view": [false, "Show dismissed 1", "false"]}), "{r}");
    assert_eq!(
        r["view"],
        json!({"rows": ["n1"], "head": "Dismissed issues", "chip": "", "launch": true,
               "restore": ["Restore", "Restore widgets#43"], "verb": "dismissed", "toggle": ["Show new", "", "true"]}),
        "{r}"
    );
    assert_eq!(r["posts"], json!(["/api/new-issues/n1/restore"]), "{r}");
    assert_eq!(r["back"], json!({"rows": ["n1", "n2"], "head": "New issues", "toggle": true}), "{r}");
}

#[tokio::test]
async fn the_board_offers_a_newer_release_and_a_reload_after_one() {
    let s = start(H::new()).await;
    let st: Value = s.http.get(format!("{}/api/state", s.base)).send().await.unwrap().json().await.unwrap();
    assert!(st["service"]["update"].is_null(), "no release is known before a check: {st}");
    s.h.app.wf.set_release(t3_codemode::release::Version::parse("9.9.9").unwrap());
    let st: Value = s.http.get(format!("{}/api/state", s.base)).send().await.unwrap().json().await.unwrap();
    let u = &st["service"]["update"];
    assert_eq!((u["latest"].as_str(), u["available"].as_bool()), (Some("9.9.9"), Some(true)), "{st}");
    assert!(u["notes"].as_str().unwrap().ends_with("/releases/tag/v9.9.9"), "{u}");
    let health: Value = s.http.get(format!("{}/healthz", s.base)).send().await.unwrap().json().await.unwrap();
    assert_eq!(health["update"]["latest"], "9.9.9");
    let html = s.http.get(format!("{}/", s.base)).send().await.unwrap().text().await.unwrap();
    assert!(html.contains(&format!("const PAGE_VERSION = \"{}\"", env!("CARGO_PKG_VERSION"))));
    let scenario = format!(
        r#"() => {{
  const base = {st};
  const $ = (id) => document.getElementById(id);
  const withUpdate = (latest) => ({{ ...base, service: {{ ...base.service, update: {{ ...base.service.update, latest, available: true }} }} }});
  const out = {{}};
  emitState(base);
  out.offered = !$("upd").hidden && $("upd-text").textContent;
  $("upd-btn").dispatch("click");
  out.opened = !$("upd-pop").hidden && $("upd-btn").getAttribute("aria-expanded");
  out.cmd = $("upd-cmd").textContent;
  out.notes = $("upd-notes").getAttribute("href");
  $("upd-hide").dispatch("click");
  out.hidden = $("upd").hidden && $("upd-pop").hidden;
  out.stored = localStorage.getItem("t3board.update.hidden");
  emitState(base);
  out.stillHidden = $("upd").hidden;
  emitState(withUpdate("9.9.10"));
  out.newer = !$("upd").hidden && $("upd-text").textContent;
  emitState({{ ...base, service: {{ ...base.service, version: "9.9.10", update: null }} }});
  out.reload = !$("upd").hidden && $("upd-text").textContent;
  out.go = $("upd-btn").classList.contains("go");
  emitState({{ ...base, service: {{ ...base.service, update: {{ ...base.service.update, available: false }} }} }});
  out.current = $("upd").hidden;
  return out;
}}"#
    );
    let r = run_page(&html, &scenario);
    assert_eq!(r["offered"], "Update to 9.9.9", "{r}");
    assert_eq!(r["opened"], "true", "{r}");
    assert_eq!(r["cmd"], t3_codemode::release::UPDATE_COMMAND, "{r}");
    assert!(r["notes"].as_str().unwrap().ends_with("/releases/tag/v9.9.9"), "{r}");
    assert_eq!((r["hidden"].as_bool(), r["stored"].as_str()), (Some(true), Some("9.9.9")), "{r}");
    assert_eq!(r["stillHidden"], true, "{r}");
    assert_eq!(r["newer"], "Update to 9.9.10", "{r}");
    assert_eq!(r["reload"], "Reload for 9.9.10", "{r}");
    assert_eq!(r["go"], true, "{r}");
    assert_eq!(r["current"], true, "{r}");
}

#[tokio::test]
async fn with_no_new_issues_the_view_says_so() {
    let (html, st) = new_issue_page().await;
    assert!(html.contains(r#"data-view="open" data-empty="">"#));
    let r = new_scenario(
        &html,
        &st,
        r##"
const none = { ...base, newIssues: [], counts: { ...base.counts, newIssues: 0 } };
const tab = () => [document.getElementById("vn-issues").textContent, document.getElementById("vt-issues").getAttribute("aria-label")];
emitState(none);
const empty = { none: q("#nis-none").hidden, route: q("#dismissed-open").hidden, tab: tab() };
emitState(base);
const shown = { none: q("#nis-none").hidden, route: q("#dismissed-open").hidden, view: q("#new-view").hidden, tab: tab() };
emitState(none);
return { empty, shown, again: q("#nis-none").hidden };
"##,
    );
    assert_eq!(r["empty"], json!({"none": false, "route": true, "tab": ["", "New Issues"]}), "{r}");
    assert_eq!(
        r["shown"],
        json!({"none": true, "route": true, "view": true, "tab": ["2", "New Issues, 2 new"]}),
        "{r}"
    );
    assert_eq!(r["again"], false, "{r}");
}

#[tokio::test]
async fn only_issues_and_tickets_that_arrive_live_flash_and_the_flash_fades() {
    let (html, st) = new_issue_page().await;
    let r = new_scenario(
        &html,
        &st,
        r##"
const lit = (el) => !!el && el.classList.contains("arrive");
const tk = (k) => q('#open [data-key="' + k + '"]');
emitState(base);
const atLoad = { rows: rows().map((k) => lit(row(k))), ticket: lit(tk(base.tree[0].thread)) };
const issue = { ...base.newIssues[0], id: "n9", label: "widgets#49", number: 49, url: "https://github.com/Acme/widgets/issues/49" };
const ticket = { ...base.tree[0], thread: "t-new", label: "widgets#50" };
emitState({ ...base, newIssues: [issue, ...base.newIssues], tree: [...base.tree, ticket] });
const arrived = { issue: lit(row("n9")), ticket: lit(tk("t-new")), old: lit(row("n1")) };
advance(4000);
const fading = lit(row("n9")) && lit(tk("t-new"));
advance(200);
return { atLoad, arrived, fading, faded: lit(row("n9")) || lit(tk("t-new")) };
"##,
    );
    assert_eq!(r["atLoad"], json!({"rows": [false, false], "ticket": false}), "{r}");
    assert_eq!(r["arrived"], json!({"issue": true, "ticket": true, "old": false}), "{r}");
    assert_eq!(r["fading"], true, "{r}");
    assert_eq!(r["faded"], false, "{r}");
    assert!(html.contains(".nrow.arrive, .tk.arrive { animation: flash 4s ease-in; }"));
}

#[tokio::test]
async fn an_issue_held_behind_the_count_flashes_when_revealed() {
    let (html, st) = new_issue_page().await;
    let r = new_scenario(
        &html,
        &st,
        r##"
emitState(base);
q("#p-new").dispatch("pointerenter");
const issue = { ...base.newIssues[0], id: "n9", label: "widgets#49", number: 49 };
emitState({ ...base, newIssues: [issue, ...base.newIssues] });
const held = { rows: rows(), slot: q("#new-nis").textContent };
q("#new-nis").dispatch("click");
return { held, rows: rows(), lit: row("n9").classList.contains("arrive") };
"##,
    );
    assert_eq!(r["held"], json!({"rows": ["n2", "n1"], "slot": "1 new"}), "{r}");
    assert_eq!(r["rows"], json!(["n9", "n2", "n1"]), "{r}");
    assert_eq!(r["lit"], true, "{r}");
}

#[tokio::test]
async fn reduced_motion_gets_a_short_static_tint() {
    let (html, st) = new_issue_page().await;
    assert!(html.contains("  .nrow.arrive, .tk.arrive { background-color: var(--flash); }\n}"));
    let prelude = r#"globalThis.matchMedia = (m) => ({ matches: /reduce/.test(m), addEventListener() {} });"#;
    let body = r##"
emitState(base);
const issue = { ...base.newIssues[0], id: "n9", label: "widgets#49", number: 49 };
emitState({ ...base, newIssues: [issue, ...base.newIssues] });
const lit = row("n9").classList.contains("arrive");
advance(1500);
return { lit, after: row("n9").classList.contains("arrive") };
"##;
    let r = run_page_async_with(
        &html,
        prelude,
        &format!("async () => {{\n{PAGE_HELPERS}\n{NEW_HELPERS}\nconst base = {st};\n{body}\n}}"),
    );
    assert_eq!(r, json!({"lit": true, "after": false}));
}

#[tokio::test]
async fn dismissed_issues_stay_reachable_from_settled_while_none_are_new() {
    let s = start(H::new()).await;
    result(&s.h.exec_as(WORKER, "async () => wf.issue({ url: 'Acme/widgets#43', title: 'Fix the grader' })").await);
    s.h.app.wf.dismiss_new_issue("n1").await.unwrap();
    let html = s.http.get(format!("{}/", s.base)).send().await.unwrap().text().await.unwrap();
    let st: Value = s.http.get(format!("{}/api/state", s.base)).send().await.unwrap().json().await.unwrap();
    let r = new_scenario(
        &html,
        &st,
        r##"
emitState(base);
const hidden = { none: q("#nis-none").hidden, route: q("#dismissed-open").hidden, n: q("#n-dismissed-s").textContent,
  name: q("#dismissed-open").getAttribute("aria-label") };
sec().append(q("#new-view"));
selectView("activity");
q("#dismissed-open").dispatch("click");
const view = { none: q("#nis-none").hidden, rows: rows(), head: q("#h-new-t").textContent, route: q("#dismissed-open").hidden,
  focus: document.activeElement === q("#new-view"), on: q("#app").dataset.view };
q("#new-view").dispatch("click");
return { hidden, view, back: q("#nis-none").hidden, refocus: document.activeElement === q("#new-view") };
"##,
    );
    assert_eq!(r["hidden"], json!({"none": false, "route": false, "n": "1", "name": "Show 1 dismissed issues"}), "{r}");
    assert_eq!(
        r["view"],
        json!({"none": true, "rows": ["n1"], "head": "Dismissed issues", "route": true, "focus": true, "on": "issues"}),
        "{r}"
    );
    assert_eq!((&r["back"], &r["refocus"]), (&json!(false), &json!(true)), "{r}");
}

#[tokio::test]
async fn clearing_the_filter_over_tickets_brings_issues_back_at_once_without_a_flash() {
    let (html, st) = new_issue_page().await;
    let r = new_scenario(
        &html,
        &st,
        r##"
emitState(base);
q("#p-new").dispatch("pointerenter");
const box = document.getElementById("filter-q");
box.focus();
typeKeys("gadgets");
advance(200);
const narrowed = rows();
box.value = "";
box.dispatch("input");
advance(200);
return { narrowed, rows: rows(), slot: q("#new-nis").textContent, lit: rows().map((k) => row(k).classList.contains("arrive")) };
"##,
    );
    assert_eq!(r, json!({"narrowed": ["n2"], "rows": ["n2", "n1"], "slot": "", "lit": [false, false]}));
}

fn skills_home(h: &H) -> std::path::PathBuf {
    h.dir.path().join("home")
}

fn install_skills(home: &std::path::Path, release: &str) {
    use t3_codemode::skills::{self, Bundle, Mode, Options, Paths};
    let emb = Bundle::embedded();
    let files: Vec<(String, Vec<u8>)> =
        emb.expected().keys().map(|p| (p.clone(), emb.raw(p).unwrap().to_vec())).collect();
    let opts = Options { mode: Mode::Install, force: false, take_over: false };
    skills::sync(&Paths::under(home, None), &Bundle::new(release, files), opts).unwrap();
}

fn setup_done(home: &std::path::Path) {
    std::fs::create_dir_all(home.join(".claude")).unwrap();
    let cfg = home.join(".config").join("t3-workflow").join("config.json");
    let steps: Vec<String> = t3_codemode::steps::catalog(&t3_codemode::skills::Bundle::embedded())
        .unwrap()
        .into_iter()
        .filter(|s| s.kind == t3_codemode::steps::Kind::Interview)
        .map(|s| s.id)
        .collect();
    std::fs::write(cfg, json!({"version": 1, "passes": ["pr"], "steps": steps}).to_string()).unwrap();
}

async fn state_and_health(s: &Srv) -> (Value, Value) {
    let st: Value = s.http.get(format!("{}/api/state", s.base)).send().await.unwrap().json().await.unwrap();
    let health: Value = s.http.get(format!("{}/healthz", s.base)).send().await.unwrap().json().await.unwrap();
    (st, health)
}

const DRIFT_SCENARIO: &str = r#"() => {
  const $ = (id) => document.getElementById(id);
  emitState(base);
  const out = { shown: !$("drift").hidden, label: $("drift-text").textContent, aria: $("drift-btn").getAttribute("aria-label") };
  if (!out.shown) return out;
  $("drift-btn").dispatch("click");
  out.opened = !$("drift-pop").hidden && $("drift-btn").getAttribute("aria-expanded");
  out.msg = $("drift-msg").textContent;
  out.cmds = $("drift-fixes").querySelectorAll("code").map((c) => c.textContent);
  for (const f of document.listeners.click) f({ type: "click", target: $("counts") });
  out.outside = $("drift-pop").hidden;
  emitState({ ...base, service: { ...base.service, skills: { ...base.service.skills, fixes: [], missingSteps: [], fix: null, foreign: null, current: true, installed: base.service.skills.release } } });
  out.cleared = $("drift").hidden && $("drift-pop").hidden;
  return out;
}"#;

fn drift_page(html: &str, st: &Value) -> Value {
    run_page(html, &format!("(() => {{ const base = {st}; return ({DRIFT_SCENARIO})(); }})"))
}

#[tokio::test]
async fn stale_skills_and_a_missing_rule_show_in_healthz_and_the_header() {
    let s = start(H::new()).await;
    let home = skills_home(&s.h);
    install_skills(&home, "0.0.1");
    setup_done(&home);
    s.h.app.wf.refresh_skills();
    let (st, health) = state_and_health(&s).await;
    let k = &health["skills"];
    assert_eq!(k, &st["service"]["skills"], "/healthz and /api/state report one view");
    assert_eq!(
        (k["release"].as_str(), k["installed"].as_str()),
        (Some(env!("CARGO_PKG_VERSION")), Some("0.0.1")),
        "{k}"
    );
    assert_eq!((k["present"].as_bool(), k["current"].as_bool()), (Some(true), Some(false)), "{k}");
    assert_eq!(k["fixes"], json!(["t3cm skills update"]), "{k}");
    let missing: Vec<(&str, &str)> = k["missingSteps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| (m["id"].as_str().unwrap(), m["fix"].as_str().unwrap()))
        .collect();
    assert_eq!(missing, [("rule:filed-issues", "t3cm skills update")], "{k}");
    let html = s.http.get(format!("{}/", s.base)).send().await.unwrap().text().await.unwrap();
    let r = drift_page(&html, &st);
    assert_eq!(r["label"], "Skills out of date", "{r}");
    assert_eq!(r["aria"], "Skills out of date. Fix: t3cm skills update", "{r}");
    assert_eq!(r["opened"], "true", "{r}");
    let msg = r["msg"].as_str().unwrap();
    assert!(msg.contains("are at 0.0.1; t3cm runs") && msg.contains("The filed-issue rule is missing"), "{r}");
    assert_eq!(r["cmds"], json!(["t3cm skills update"]), "{r}");
    assert_eq!(r["outside"], true, "a click elsewhere closes the panel: {r}");
    assert_eq!(r["cleared"], true, "{r}");
}

#[tokio::test]
async fn skills_newer_than_the_service_name_the_restart_not_a_skills_update() {
    let restart = t3_codemode::platform::SERVICE.restart;
    let s = start(H::new()).await;
    let home = skills_home(&s.h);
    install_skills(&home, "99.0.0");
    setup_done(&home);
    s.h.app.wf.refresh_skills();
    let (st, health) = state_and_health(&s).await;
    let k = &health["skills"];
    assert_eq!((k["installed"].as_str(), k["serviceStale"].as_bool()), (Some("99.0.0"), Some(true)), "{k}");
    assert_eq!(
        (k["current"].as_bool(), &k["fix"], &k["fixes"]),
        (Some(false), &json!(restart), &json!([restart])),
        "{k}"
    );
    let html = s.http.get(format!("{}/", s.base)).send().await.unwrap().text().await.unwrap();
    let r = drift_page(&html, &st);
    assert_eq!(r["label"], "Service needs a restart", "{r}");
    assert_eq!(r["aria"], format!("Service needs a restart. Fix: {restart}"), "{r}");
    let msg = r["msg"].as_str().unwrap();
    assert!(msg.contains("are at 99.0.0, newer than the t3cm") && !msg.contains("skills update"), "{r}");
    assert_eq!(r["cmds"], json!([restart]), "{r}");
    assert_eq!(r["cleared"], true, "{r}");
}

#[test]
fn doctor_names_the_restart_when_the_service_is_older_than_the_skills() {
    use t3_codemode::{doctor, drift::Home, skills::Bundle};
    let restart = t3_codemode::platform::SERVICE.restart;
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    install_skills(home, env!("CARGO_PKG_VERSION"));
    setup_done(home);
    t3_codemode::steps::apply(&t3_codemode::steps::Env::under(home, None), &Bundle::embedded()).unwrap();
    let f = doctor::skills(&Home::under(home), &Bundle::embedded(), Some("0.0.1"));
    let want = format!(
        "service: runs t3cm 0.0.1, but the T3 workflow skills are at {v}; restart the service to load {v}: {restart}",
        v = env!("CARGO_PKG_VERSION")
    );
    assert_eq!(f.problems, [want], "{:?}", f.problems);
    assert!(f.warnings.is_empty(), "{:?}", f.warnings);
    let f = doctor::skills(&Home::under(home), &Bundle::embedded(), Some(env!("CARGO_PKG_VERSION")));
    assert!(f.problems.is_empty() && f.warnings.is_empty(), "{:?} {:?}", f.problems, f.warnings);
    let emb = Bundle::embedded();
    let old = Bundle::new("0.0.1", emb.expected().keys().map(|p| (p.clone(), emb.raw(p).unwrap().to_vec())));
    let f = doctor::skills(&Home::under(home), &old, Some("0.0.1"));
    assert_eq!(f.problems.len(), 1, "one finding, not a second skills line: {:?}", f.problems);
    assert!(f.problems[0].ends_with(restart) && !f.problems[0].contains("skills update"), "{:?}", f.problems);
    let f = doctor::skills(&Home::under(home), &old, Some(env!("CARGO_PKG_VERSION")));
    assert!(f.problems.is_empty(), "an older CLI does not blame a current service: {:?}", f.problems);
    assert_eq!(f.warnings.len(), 1, "{:?}", f.warnings);
    assert!(
        f.warnings[0].contains("newer than this t3cm 0.0.1") && !f.warnings[0].contains(restart),
        "{:?}",
        f.warnings
    );
}

#[tokio::test]
async fn the_rule_alone_missing_is_a_setup_step_and_clears_after_update() {
    let s = start(H::new()).await;
    let home = skills_home(&s.h);
    install_skills(&home, env!("CARGO_PKG_VERSION"));
    setup_done(&home);
    let st = s.h.exec("async () => wf.status()").await;
    let k = result(&st)["skills"].clone();
    assert_eq!((k["fix"].is_null(), k["current"].as_bool()), (true, Some(false)), "{k}");
    assert_eq!(k["fixes"], json!(["t3cm skills update"]), "{k}");
    let (state, health) = state_and_health(&s).await;
    assert_eq!(health["skills"], k);
    let html = s.http.get(format!("{}/", s.base)).send().await.unwrap().text().await.unwrap();
    let r = drift_page(&html, &state);
    assert_eq!(r["label"], "Setup step missing", "{r}");
    assert_eq!(r["cmds"], json!(["t3cm skills update"]), "{r}");
    t3_codemode::steps::apply(&t3_codemode::steps::Env::under(&home, None), &t3_codemode::skills::Bundle::embedded())
        .unwrap();
    let (_, health) = state_and_health(&s).await;
    assert_eq!(health["skills"]["current"], false, "a read between watcher passes is served from the cache");
    let mut events = s.h.app.wf.subscribe();
    let fresh = result(&s.h.exec("async () => (await wf.status()).skills").await);
    assert_eq!((fresh["current"].as_bool(), &fresh["fixes"]), (Some(true), &json!([])), "{fresh}");
    assert!(events.try_recv().is_ok(), "a changed report pushes a board update");
    let (state, health) = state_and_health(&s).await;
    assert_eq!(health["skills"], fresh);
    assert_eq!(drift_page(&html, &state)["shown"], false);
}

#[tokio::test]
async fn an_npx_skills_install_shows_as_skew_with_the_take_over_fix() {
    let s = start(H::new()).await;
    let home = skills_home(&s.h);
    install_skills(&home, env!("CARGO_PKG_VERSION"));
    std::fs::remove_file(home.join(".config").join("t3-workflow").join("installed.json")).unwrap();
    let lock = json!({"version": 3, "skills": {"t3-worker": {"source": "OttoOps/t3-codemode", "commit": "8e179c3"}}});
    std::fs::write(home.join(".agents").join(".skill-lock.json"), lock.to_string()).unwrap();
    s.h.app.wf.refresh_skills();
    let (st, health) = state_and_health(&s).await;
    let k = &health["skills"];
    assert_eq!(k["foreign"]["tool"], "npx skills", "{k}");
    assert_eq!(k["installed"], Value::Null, "{k}");
    assert_eq!(k["fixes"][0], "t3cm skills install --take-over", "{k}");
    let html = s.http.get(format!("{}/", s.base)).send().await.unwrap().text().await.unwrap();
    let r = drift_page(&html, &st);
    assert_eq!(r["label"], "Skills not managed", "{r}");
    assert!(r["msg"].as_str().unwrap().contains("installed by npx skills and match t3cm"), "{r}");
    assert_eq!(r["cmds"][0], "t3cm skills install --take-over", "{r}");
}

#[tokio::test]
async fn a_machine_without_the_skills_shows_no_skills_chip() {
    let s = start(H::new()).await;
    let (st, health) = state_and_health(&s).await;
    let k = &health["skills"];
    assert_eq!((k["present"].as_bool(), k["current"].as_bool()), (Some(false), Some(false)), "{k}");
    assert_eq!(k["fixes"][0], "t3cm skills install", "{k}");
    let html = s.http.get(format!("{}/", s.base)).send().await.unwrap().text().await.unwrap();
    assert_eq!(drift_page(&html, &st)["shown"], false);
}

#[tokio::test]
async fn a_new_slot_or_a_broken_config_names_the_setup_skill() {
    let s = start(H::new()).await;
    let home = skills_home(&s.h);
    install_skills(&home, env!("CARGO_PKG_VERSION"));
    setup_done(&home);
    let env = t3_codemode::steps::Env::under(&home, None);
    t3_codemode::steps::apply(&env, &t3_codemode::skills::Bundle::embedded()).unwrap();
    let mut cfg: Value = serde_json::from_str(&std::fs::read_to_string(&env.config).unwrap()).unwrap();
    cfg["steps"].as_array_mut().unwrap().retain(|id| id != "slot:models");
    std::fs::write(&env.config, cfg.to_string()).unwrap();
    let k = result(&s.h.exec("async () => (await wf.status()).skills").await);
    assert_eq!(
        (k["current"].as_bool(), &k["fixes"]),
        (Some(true), &json!(["run setup-t3-workflow: new slot models"])),
        "{k}"
    );
    let (st, _) = state_and_health(&s).await;
    let html = s.http.get(format!("{}/", s.base)).send().await.unwrap().text().await.unwrap();
    let r = drift_page(&html, &st);
    assert_eq!(r["label"], "Setup step missing", "{r}");
    assert!(r["msg"].as_str().unwrap().contains("Setup has a new slot to pick: models."), "{r}");
    std::fs::write(&env.config, "{not json").unwrap();
    let k = result(&s.h.exec("async () => (await wf.status()).skills").await);
    assert_eq!(k["configState"], "problem", "{k}");
    let fix = k["fixes"][0].as_str().unwrap();
    assert!(fix.starts_with("fix ") && fix.ends_with("config.json, then run setup-t3-workflow"), "{k}");
    let (st, _) = state_and_health(&s).await;
    let r = drift_page(&html, &st);
    assert_eq!(r["label"], "Workflow config ignored", "{r}");
    assert!(r["msg"].as_str().unwrap().contains("The T3 workflow config is ignored: not JSON"), "{r}");
}

fn write_workflow_config(h: &H, cfg: Value) {
    let p = h.dir.path().join("home/.config/t3-workflow/config.json");
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, cfg.to_string()).unwrap();
}

fn codex_provider(id: &str, models: &[&str]) -> Value {
    let mut p = t3_codemode::fake::provider(id, "codex", models);
    for m in p["models"].as_array_mut().unwrap() {
        m["options"] = json!([{"id": "reasoningEffort", "values": ["low", "medium", "high", "xhigh"]}]);
    }
    p
}

#[tokio::test]
async fn board_launch_resolves_user_config_repo_agents_md_and_precedence() {
    use t3_codemode::fake::provider;

    let s = start(H::new()).await;
    let repo = s.h.dir.path().join("widgets");
    std::fs::create_dir_all(&repo).unwrap();
    s.h.fake.set_projects(vec![project("p-widgets", &repo.to_string_lossy())]);
    s.h.remotes.set(&repo.to_string_lossy(), "git@github.com:acme/widgets.git");
    s.h.fake.set_providers(vec![
        provider("claudeAgent", "claudeAgent", &["claude-opus-5-5", "claude-haiku-5-5", "claude-catalog-only"]),
        provider("claude-2", "claudeAgent", &["claude-opus-5-5", "claude-haiku-5-5", "claude-catalog-only"]),
        codex_provider("codex", &["gpt-6.2-sol"]),
    ]);

    // 1. User config sets instance, model and effort (even though claudeAgent is listed first).
    write_workflow_config(
        &s.h,
        json!({"models": {"ticket": {"provider": "claude-2", "model": "claude-opus-5-5", "effort": "medium"}}}),
    );
    let r = s.post("/api/launch", json!({"issues": ["acme/widgets#1"]}), Some(&s.csrf()), None).await;
    assert_eq!(r.status(), 200);
    let body: Value = r.json().await.unwrap();
    let res = &body["results"][0];
    assert_eq!(res["ok"], true, "{res}");
    assert_eq!(res["instanceId"], "claude-2");
    assert_eq!(res["model"], "claude-opus-5-5");
    assert_eq!(res["effort"], "medium");
    assert_eq!(res["modelSource"], "user");
    assert_eq!(
        s.h.fake.launched()[0]["modelSelection"],
        json!({"instanceId": "claude-2", "model": "claude-opus-5-5", "options": [{"id": "effort", "value": "medium"}]})
    );

    // 2. Repo AGENTS.md overrides effort while keeping user's provider for the same model.
    std::fs::write(repo.join("AGENTS.md"), "## Agent workflow\n\n- Ticket model: claude-opus-5-5, low effort\n")
        .unwrap();
    let res = result(&s.h.exec("async () => wf.launchTicket({ issue: 'acme/widgets#2' })").await);
    assert_eq!(res["instanceId"], "claude-2");
    assert_eq!(res["model"], "claude-opus-5-5");
    assert_eq!(res["effort"], "low");
    assert_eq!(res["modelSource"], "AGENTS.md");
    assert_eq!(
        s.h.fake.launched()[1]["modelSelection"],
        json!({"instanceId": "claude-2", "model": "claude-opus-5-5", "options": [{"id": "effort", "value": "low"}]})
    );

    // 3. Repo AGENTS.md switches to a Codex model (no restart needed).
    std::fs::write(repo.join("AGENTS.md"), "## Agent workflow\n\n- Ticket model: gpt-6.2-sol, xhigh effort\n").unwrap();
    let res = result(&s.h.exec("async () => wf.launchTicket({ issue: 'acme/widgets#3' })").await);
    assert_eq!(res["instanceId"], "codex");
    assert_eq!(res["model"], "gpt-6.2-sol");
    assert_eq!(res["effort"], "xhigh");
    assert_eq!(res["modelSource"], "AGENTS.md");
    assert_eq!(
        s.h.fake.launched()[2]["modelSelection"],
        json!({"instanceId": "codex", "model": "gpt-6.2-sol", "options": [{"id": "reasoningEffort", "value": "xhigh"}]})
    );

    // 4. Repo .t3-workflow.json overrides AGENTS.md.
    std::fs::write(
        repo.join(".t3-workflow.json"),
        json!({"models": {"ticket": {"provider": "claude-2", "model": "claude-opus-5-5", "effort": "high"}}})
            .to_string(),
    )
    .unwrap();
    let res = result(&s.h.exec("async () => wf.launchTicket({ issue: 'acme/widgets#4' })").await);
    assert_eq!(res["instanceId"], "claude-2");
    assert_eq!(res["model"], "claude-opus-5-5");
    assert_eq!(res["effort"], "high");
    assert_eq!(res["modelSource"], ".t3-workflow.json");

    // 5. Launch arg overrides .t3-workflow.json (both model+effort and effort-only).
    let res = result(
        &s.h.exec("async () => wf.launchTicket({ issue: 'acme/widgets#5', model: 'gpt-6.2-sol', effort: 'medium' })")
            .await,
    );
    assert_eq!(res["instanceId"], "codex");
    assert_eq!(res["model"], "gpt-6.2-sol");
    assert_eq!(res["effort"], "medium");
    assert_eq!(res["modelSource"], "argument");

    let res = result(&s.h.exec("async () => wf.launchTicket({ issue: 'acme/widgets#6', effort: 'low' })").await);
    assert_eq!(res["instanceId"], "claude-2");
    assert_eq!(res["model"], "claude-opus-5-5");
    assert_eq!(res["effort"], "low");
    assert_eq!(res["modelSource"], "argument");

    // 6. Catalog-only model beyond T3's short list resolves via orchestrator_capabilities search.
    s.h.fake.set_short_list(1);
    std::fs::write(
        repo.join(".t3-workflow.json"),
        json!({"models": {"ticket": {"provider": "claude-2", "model": "claude-catalog-only", "effort": "high"}}})
            .to_string(),
    )
    .unwrap();
    let res = result(&s.h.exec("async () => wf.launchTicket({ issue: 'acme/widgets#7' })").await);
    assert_eq!(res["instanceId"], "claude-2");
    assert_eq!(res["model"], "claude-catalog-only");
    assert_eq!(res["effort"], "high");
    assert_eq!(res["modelSource"], ".t3-workflow.json");
}

#[tokio::test]
async fn legacy_board_ticket_keys_sit_below_user_slot_and_above_default() {
    use t3_codemode::fake::provider;

    let cfg = Config::from_value(json!({
        "board": {"instanceId": "claude-2", "ticketModel": "claude-haiku-5-5", "ticketEffort": "low"}
    }))
    .unwrap();
    let h = H::build(Vec::new(), cfg);
    let repo = h.dir.path().join("widgets");
    std::fs::create_dir_all(&repo).unwrap();
    h.fake.set_projects(vec![project("p-widgets", &repo.to_string_lossy())]);
    h.remotes.set(&repo.to_string_lossy(), "git@github.com:acme/widgets.git");
    h.fake.set_providers(vec![
        provider("claudeAgent", "claudeAgent", &["claude-opus-5-5", "claude-haiku-5-5"]),
        provider("claude-2", "claudeAgent", &["claude-opus-5-5", "claude-haiku-5-5"]),
    ]);

    // With no slot set, legacy board.* keys apply.
    let res = result(&h.exec("async () => wf.launchTicket({ issue: 'acme/widgets#1' })").await);
    assert_eq!(res["instanceId"], "claude-2");
    assert_eq!(res["model"], "claude-haiku-5-5");
    assert_eq!(res["effort"], "low");
    assert_eq!(res["modelSource"], "board");

    // Once user config sets models.ticket, it outranks legacy board.* keys.
    write_workflow_config(
        &h,
        json!({"models": {"ticket": {"provider": "claudeAgent", "model": "claude-opus-5-5", "effort": "high"}}}),
    );
    let res = result(&h.exec("async () => wf.launchTicket({ issue: 'acme/widgets#2' })").await);
    assert_eq!(res["instanceId"], "claudeAgent");
    assert_eq!(res["model"], "claude-opus-5-5");
    assert_eq!(res["effort"], "high");
    assert_eq!(res["modelSource"], "user");
}

#[tokio::test]
async fn board_launch_picks_through_slot_pool_from_user_config_and_agents_md() {
    use t3_codemode::fake::provider;

    let cfg = Config::from_value(json!({
        "pools": {
            "opus": {
                "driver": "claudeAgent",
                "model": "claude-opus-5-5",
                "effort": "high",
                "members": [{"instanceId": "claude-a"}, {"instanceId": "claude-b"}]
            },
            "frontier": {
                "mixed": true,
                "strategy": "priority",
                "members": [{"instanceId": "codex-a", "model": "gpt-6.2-sol", "effort": "xhigh"}]
            }
        }
    }))
    .unwrap();
    let s = start(H::build(Vec::new(), cfg)).await;
    let repo = s.h.dir.path().join("widgets");
    std::fs::create_dir_all(&repo).unwrap();
    s.h.fake.set_projects(vec![project("p-widgets", &repo.to_string_lossy())]);
    s.h.remotes.set(&repo.to_string_lossy(), "git@github.com:acme/widgets.git");
    s.h.fake.set_providers(vec![
        provider("claude-a", "claudeAgent", &["claude-opus-5-5"]),
        provider("claude-b", "claudeAgent", &["claude-opus-5-5"]),
        codex_provider("codex-a", &["gpt-6.2-sol"]),
    ]);

    // User config pool picks via wf.pick and records the pick on the feed.
    write_workflow_config(&s.h, json!({"models": {"ticket": {"pool": "opus"}}}));
    let r = s.post("/api/launch", json!({"issues": ["acme/widgets#1", "acme/widgets#2"]}), Some(&s.csrf()), None).await;
    let body: Value = r.json().await.unwrap();
    let r1 = &body["results"][0];
    let r2 = &body["results"][1];
    assert_eq!(r1["ok"], true, "{r1}");
    assert_eq!(r1["pool"], "opus");
    assert_eq!(r1["model"], "claude-opus-5-5");
    assert_eq!(r1["effort"], "high");
    assert_eq!(r1["modelSource"], "user");
    assert!(r1["reason"].as_str().is_some_and(|s| s.contains("claude-opus-5-5")), "{r1}");
    assert_ne!(r1["instanceId"], r2["instanceId"], "weighted 1:1 alternates pool members");
    assert!(s.h.app.wf.feed(10).unwrap().iter().any(|f| f["instanceId"] == r1["instanceId"]));

    // Matching effort arg over a non-mixed pool still picks through the pool.
    let res = result(&s.h.exec("async () => wf.launchTicket({ issue: 'acme/widgets#4', effort: 'high' })").await);
    assert_eq!(res["pool"], "opus");
    assert_eq!(res["modelSource"], "argument");
    assert!(res["reason"].is_string(), "{res}");

    // Repo AGENTS.md pool overrides user config pool (and an invalid Worker model line does not block tickets).
    std::fs::write(repo.join("AGENTS.md"), "## Agent workflow\n\n- Ticket model: pool `frontier`\n- Worker model: \n")
        .unwrap();
    let res = result(&s.h.exec("async () => wf.launchTicket({ issue: 'acme/widgets#3' })").await);
    assert_eq!(res["instanceId"], "codex-a");
    assert_eq!(res["model"], "gpt-6.2-sol");
    assert_eq!(res["effort"], "xhigh");
    assert_eq!(res["pool"], "frontier");
    assert_eq!(res["modelSource"], "AGENTS.md");
    assert_eq!(
        s.h.fake.launched()[3]["modelSelection"],
        json!({"instanceId": "codex-a", "model": "gpt-6.2-sol", "options": [{"id": "reasoningEffort", "value": "xhigh"}]})
    );
}

#[tokio::test]
async fn board_launch_fails_with_error_card_on_invalid_slot_choices_never_falling_back() {
    use t3_codemode::fake::{disabled_provider, provider};

    let s = start(H::new()).await;
    let repo = s.h.dir.path().join("widgets");
    std::fs::create_dir_all(&repo).unwrap();
    s.h.fake.set_projects(vec![project("p-widgets", &repo.to_string_lossy())]);
    s.h.remotes.set(&repo.to_string_lossy(), "git@github.com:acme/widgets.git");
    s.h.fake.set_providers(vec![
        provider("claudeAgent", "claudeAgent", &["claude-opus-5-5"]),
        disabled_provider("claude-off", "claudeAgent"),
    ]);

    let expect_error_card = |res: &Value, needle: &str| {
        assert_eq!(res["ok"], false, "{res}");
        assert!(res["card"].is_string(), "{res}");
        assert!(res["error"].as_str().is_some_and(|e| e.contains(needle)), "expected {needle:?} in {res}");
    };

    // 1. Instance gone.
    write_workflow_config(
        &s.h,
        json!({"models": {"ticket": {"provider": "claude-gone", "model": "claude-opus-5-5", "effort": "high"}}}),
    );
    let body: Value =
        s.post("/api/launch", json!({"issues": ["acme/widgets#1"]}), Some(&s.csrf()), None).await.json().await.unwrap();
    expect_error_card(&body["results"][0], "claude-gone");

    // 2. Instance disabled.
    write_workflow_config(
        &s.h,
        json!({"models": {"ticket": {"provider": "claude-off", "model": "claude-opus-5-5", "effort": "high"}}}),
    );
    let body: Value =
        s.post("/api/launch", json!({"issues": ["acme/widgets#2"]}), Some(&s.csrf()), None).await.json().await.unwrap();
    expect_error_card(&body["results"][0], "claude-off");

    // 3. Model not listed (via AGENTS.md).
    std::fs::write(repo.join("AGENTS.md"), "## Agent workflow\n\n- Ticket model: claude-missing, high effort\n")
        .unwrap();
    let out = s.h.exec("async () => wf.launchTicket({ issue: 'acme/widgets#3' })").await;
    assert!(err_text(&out).contains("claude-missing"), "{}", out.text);

    // 4. Effort not offered.
    write_workflow_config(
        &s.h,
        json!({"models": {"ticket": {"provider": "claudeAgent", "model": "claude-opus-5-5", "effort": "high"}}}),
    );
    std::fs::write(repo.join("AGENTS.md"), "## Agent workflow\n\n- Ticket model: claude-opus-5-5, max effort\n")
        .unwrap();
    let out = s.h.exec("async () => wf.launchTicket({ issue: 'acme/widgets#4' })").await;
    assert!(err_text(&out).contains("effort max is not offered"), "{}", out.text);

    // 5. Unknown pool.
    std::fs::write(repo.join("AGENTS.md"), "## Agent workflow\n\n- Ticket model: pool `ghost`\n").unwrap();
    let body: Value =
        s.post("/api/launch", json!({"issues": ["acme/widgets#5"]}), Some(&s.csrf()), None).await.json().await.unwrap();
    expect_error_card(&body["results"][0], "no pool \"ghost\"");

    // 6. Broken user workflow config never falls back silently to default.
    std::fs::remove_file(repo.join("AGENTS.md")).unwrap();
    std::fs::write(s.h.dir.path().join("home/.config/t3-workflow/config.json"), "{broken json").unwrap();
    let out = s.h.exec("async () => wf.launchTicket({ issue: 'acme/widgets#6' })").await;
    assert!(err_text(&out).contains("workflow config problem"), "{}", out.text);

    // 7. No config and T3 offers no Claude -> fails with error card instead of sending claudeAgent.
    std::fs::remove_file(s.h.dir.path().join("home/.config/t3-workflow/config.json")).unwrap();
    s.h.fake.set_providers(vec![codex_provider("codex", &["gpt-6.2-sol"])]);
    let out = s.h.exec("async () => wf.launchTicket({ issue: 'acme/widgets#7' })").await;
    assert!(err_text(&out).contains("no enabled Claude provider"), "{}", out.text);

    // 8. T3 capabilities unreachable -> fails with error card.
    s.h.fake.set_caps_fail(true);
    let out = s.h.exec("async () => wf.launchTicket({ issue: 'acme/widgets#8' })").await;
    assert!(err_text(&out).contains("T3 capabilities unreachable"), "{}", out.text);

    // Nothing launched and no pending links left; every failure left an error card on the board.
    assert!(s.h.fake.launched().is_empty(), "no thread should launch on invalid choices");
    assert!(s.h.app.store.pending_all().unwrap().is_empty());
    let board_cards = cards(&s.h);
    assert_eq!(board_cards.len(), 8);
    assert!(board_cards.iter().all(|c| c["kind"] == "error"), "{board_cards:?}");

    // 9. Legacy board.instanceId disabled does not silently switch to another instance of the same driver.
    let cfg = Config::from_value(json!({"board": {"instanceId": "claudeAgent"}})).unwrap();
    let h2 = H::build(Vec::new(), cfg);
    h2.fake.set_projects(vec![project("p-widgets", &repo.to_string_lossy())]);
    h2.remotes.set(&repo.to_string_lossy(), "git@github.com:acme/widgets.git");
    h2.fake.set_providers(vec![
        disabled_provider("claudeAgent", "claudeAgent"),
        provider("claude-2", "claudeAgent", &["claude-opus-5-5"]),
    ]);
    let out = h2.exec("async () => wf.launchTicket({ issue: 'acme/widgets#9' })").await;
    assert!(err_text(&out).contains("claudeAgent"), "{}", out.text);
    assert!(h2.fake.launched().is_empty());
    assert_eq!(cards(&h2).len(), 1);
    assert_eq!(cards(&h2)[0]["kind"], "error");
}

#[tokio::test]
async fn board_models_route_shows_effective_models_sources_providers_pools_and_repo_overrides() {
    use t3_codemode::fake::{disabled_provider, provider};

    let cfg = Config::from_value(json!({
        "pools": {
            "opus": {
                "driver": "claudeAgent",
                "model": "claude-opus-5-5",
                "effort": "high",
                "members": [{"instanceId": "claudeAgent"}, {"instanceId": "claude-2"}]
            }
        }
    }))
    .unwrap();
    let s = start(H::build(Vec::new(), cfg)).await;
    let widgets = s.h.dir.path().join("widgets");
    let lms = s.h.dir.path().join("lms");
    std::fs::create_dir_all(&widgets).unwrap();
    std::fs::create_dir_all(&lms).unwrap();
    std::fs::write(lms.join("AGENTS.md"), "## Agent workflow\n\n- Ticket model: claude-opus-5-5, medium effort\n")
        .unwrap();
    s.h.fake
        .set_projects(vec![project("p-widgets", &widgets.to_string_lossy()), project("p-lms", &lms.to_string_lossy())]);
    s.h.remotes.set(&widgets.to_string_lossy(), "git@github.com:acme/widgets.git");
    s.h.remotes.set(&lms.to_string_lossy(), "git@github.com:acme/lms.git");
    let mut haiku_only = provider("claudeAgent", "claudeAgent", &["claude-opus-5-5", "claude-haiku-5-5"]);
    haiku_only["models"][1]["options"] = json!([]);
    s.h.fake.set_providers(vec![
        haiku_only,
        provider("claude-2", "claudeAgent", &["claude-opus-5-5"]),
        disabled_provider("claude-off", "claudeAgent"),
        codex_provider("codex", &["gpt-6.2-sol"]),
    ]);

    let body = s.get("/api/models").await;
    assert_eq!(body["ticket"]["mode"], "model");
    assert_eq!(body["ticket"]["source"], "default");
    assert_eq!(body["ticket"]["model"], "claude-opus-5-5");
    assert_eq!(body["ticket"]["effort"], "high");
    assert_eq!(body["ticket"]["error"], Value::Null);
    assert_eq!(body["ask"]["mode"], "model");
    assert_eq!(body["ask"]["source"], "default");
    assert_eq!(body["ask"]["model"], "claude-haiku-5-5");
    assert_eq!(body["ask"]["error"], Value::Null);

    let provider_ids: Vec<&str> =
        body["providers"].as_array().unwrap().iter().filter_map(|p| p["id"].as_str()).collect();
    assert_eq!(provider_ids, vec!["claudeAgent", "claude-2", "codex"]);
    let haiku =
        body["providers"][0]["models"].as_array().unwrap().iter().find(|m| m["id"] == "claude-haiku-5-5").unwrap();
    assert_eq!(haiku["efforts"], json!([]));
    assert_eq!(body["pools"].as_array().unwrap().len(), 1);
    assert_eq!(body["pools"][0]["name"], "opus");

    let overrides = body["repoOverrides"].as_array().unwrap();
    assert_eq!(overrides.len(), 1, "{body}");
    assert_eq!(overrides[0]["projectId"], "p-lms");
    assert_eq!(overrides[0]["repo"], "acme/lms");
    assert_eq!(overrides[0]["source"], "AGENTS.md");
    assert_eq!(overrides[0]["model"], "claude-opus-5-5");
    assert_eq!(overrides[0]["effort"], "medium");
    assert_eq!(overrides[0]["error"], Value::Null);
}

#[tokio::test]
async fn board_ticket_model_picker_persists_both_modes_and_launches_follow_without_restart() {
    use t3_codemode::fake::{disabled_provider, provider};

    let cfg = Config::from_value(json!({
        "pools": {
            "opus": {
                "driver": "claudeAgent",
                "model": "claude-opus-5-5",
                "effort": "high",
                "strategy": "weighted",
                "members": [
                    {"instanceId": "claudeAgent", "weight": 1},
                    {"instanceId": "claude-2", "weight": 1}
                ]
            }
        }
    }))
    .unwrap();
    let s = start(H::build(Vec::new(), cfg)).await;
    let widgets = s.h.dir.path().join("widgets");
    let lms = s.h.dir.path().join("lms");
    std::fs::create_dir_all(&widgets).unwrap();
    std::fs::create_dir_all(&lms).unwrap();
    std::fs::write(lms.join("AGENTS.md"), "## Agent workflow\n\n- Ticket model: claude-opus-5-5, low effort\n")
        .unwrap();
    s.h.fake
        .set_projects(vec![project("p-widgets", &widgets.to_string_lossy()), project("p-lms", &lms.to_string_lossy())]);
    s.h.remotes.set(&widgets.to_string_lossy(), "git@github.com:acme/widgets.git");
    s.h.remotes.set(&lms.to_string_lossy(), "git@github.com:acme/lms.git");
    let mut claude_a = provider("claudeAgent", "claudeAgent", &["claude-opus-5-5", "claude-haiku-5-5"]);
    claude_a["models"][1]["options"] = json!([]);
    s.h.fake.set_providers(vec![
        claude_a,
        provider("claude-2", "claudeAgent", &["claude-opus-5-5"]),
        disabled_provider("claude-off", "claudeAgent"),
        codex_provider("codex", &["gpt-6.2-sol"]),
    ]);

    // Pre-existing workflow user config keys must be preserved when the board updates models.ticket.
    write_workflow_config(
        &s.h,
        json!({
            "version": 1,
            "review": "cbrip:sol-loop",
            "passes": ["unslop", "pr"],
            "models": {"worker": {"provider": "claude-2", "model": "claude-opus-5-5", "effort": "high"}},
            "steps": ["slot:review", "slot:models"]
        }),
    );

    // CSRF required on PUT.
    let r = s
        .put(
            "/api/models/ticket",
            json!({"mode": "model", "provider": "codex", "model": "gpt-6.2-sol", "effort": "xhigh"}),
            Some("wrong"),
        )
        .await;
    assert_eq!(r.status(), 403);

    // Invalid choices are rejected with 400 and do not clobber config.
    for bad in [
        json!({"mode": "t3-default"}),
        json!({"mode": "model", "provider": "claude-off", "model": "claude-opus-5-5", "effort": "high"}),
        json!({"mode": "model", "provider": "claudeAgent", "model": "claude-haiku-5-5", "effort": "high"}),
        json!({"mode": "pool", "pool": "ghost"}),
        json!({"mode": "pool", "pool": "opus", "model": "claude-opus-5-5"}),
    ] {
        let r = s.put("/api/models/ticket", bad.clone(), Some(&s.csrf())).await;
        assert_eq!(r.status(), 400, "expected 400 for {bad}");
    }

    // 1. Switch to Specific model mode through the board route.
    let r = s
        .put(
            "/api/models/ticket",
            json!({"mode": "model", "provider": "codex", "model": "gpt-6.2-sol", "effort": "xhigh"}),
            Some(&s.csrf()),
        )
        .await;
    assert_eq!(r.status(), 200);
    let models: Value = r.json().await.unwrap();
    assert_eq!(models["ticket"]["mode"], "model");
    assert_eq!(models["ticket"]["source"], "user");
    assert_eq!(models["ticket"]["instanceId"], "codex");
    assert_eq!(models["ticket"]["model"], "gpt-6.2-sol");
    assert_eq!(models["ticket"]["effort"], "xhigh");
    assert_eq!(models["modelsRev"], 1);

    // Persisted to workflow user config while preserving unrelated slots and worker model.
    let wf_path = s.h.dir.path().join("home/.config/t3-workflow/config.json");
    let on_disk: Value = serde_json::from_str(&std::fs::read_to_string(&wf_path).unwrap()).unwrap();
    assert_eq!(on_disk["review"], "cbrip:sol-loop");
    assert_eq!(on_disk["passes"], json!(["unslop", "pr"]));
    assert_eq!(on_disk["models"]["worker"]["provider"], "claude-2");
    assert_eq!(on_disk["models"]["ticket"], json!({"provider": "codex", "model": "gpt-6.2-sol", "effort": "xhigh"}));

    // Next board launch follows the new setting without a restart.
    let launch_res: Value =
        s.post("/api/launch", json!({"issues": ["acme/widgets#1"]}), Some(&s.csrf()), None).await.json().await.unwrap();
    let r1 = &launch_res["results"][0];
    assert_eq!(r1["ok"], true, "{r1}");
    assert_eq!(r1["instanceId"], "codex");
    assert_eq!(r1["model"], "gpt-6.2-sol");
    assert_eq!(r1["effort"], "xhigh");
    assert_eq!(r1["modelSource"], "user");

    // Repo override still takes precedence over user config on launch.
    let lms_res: Value =
        s.post("/api/launch", json!({"issues": ["acme/lms#2"]}), Some(&s.csrf()), None).await.json().await.unwrap();
    let r2 = &lms_res["results"][0];
    assert_eq!(r2["ok"], true, "{r2}");
    assert_eq!(r2["model"], "claude-opus-5-5");
    assert_eq!(r2["effort"], "low");
    assert_eq!(r2["modelSource"], "AGENTS.md");

    // 2. Switch to Pool mode through the board route; next launch uses the pool without restart.
    let r = s.put("/api/models/ticket", json!({"mode": "pool", "pool": "opus"}), Some(&s.csrf())).await;
    assert_eq!(r.status(), 200);
    let models: Value = r.json().await.unwrap();
    assert_eq!(models["ticket"]["mode"], "pool");
    assert_eq!(models["ticket"]["pool"], "opus");
    assert_eq!(models["ticket"]["source"], "user");
    assert_eq!(models["modelsRev"], 2);

    let on_disk: Value = serde_json::from_str(&std::fs::read_to_string(&wf_path).unwrap()).unwrap();
    assert_eq!(on_disk["models"]["ticket"], json!({"pool": "opus"}));

    let pool_res: Value =
        s.post("/api/launch", json!({"issues": ["acme/widgets#3"]}), Some(&s.csrf()), None).await.json().await.unwrap();
    let r3 = &pool_res["results"][0];
    assert_eq!(r3["ok"], true, "{r3}");
    assert_eq!(r3["pool"], "opus");
    assert_eq!(r3["model"], "claude-opus-5-5");
    assert_eq!(r3["effort"], "high");
    assert_eq!(r3["modelSource"], "user");

    // Explicit model arg still works even if models.ticket only holds effort (advisory note from PR 1).
    write_workflow_config(&s.h, json!({"version": 1, "models": {"ticket": {"effort": "high"}}}));
    let out = s.h.exec("async () => wf.launchTicket({ issue: 'acme/widgets#4', model: 'gpt-6.2-sol' })").await;
    let r4 = result(&out);
    assert_eq!(r4["instanceId"], "codex");
    assert_eq!(r4["model"], "gpt-6.2-sol");
    assert_eq!(r4["modelSource"], "argument");
}

#[tokio::test]
async fn board_ask_model_picker_persists_both_modes_and_ignores_legacy_board_instance_and_pool() {
    use t3_codemode::fake::{disabled_provider, provider};

    let raw_cfg = json!({
        "deny": ["t3_project_delete"],
        "pools": {
            "opus": {
                "driver": "claudeAgent",
                "model": "claude-opus-5-5",
                "effort": "high",
                "members": [{"instanceId": "claude-2"}]
            },
            "cheap": {
                "driver": "claudeAgent",
                "model": "claude-haiku-5-5",
                "members": [{"instanceId": "claudeAgent"}, {"instanceId": "claude-2"}]
            }
        },
        "board": {
            "instanceId": "claude-2",
            "pool": "opus",
            "settleAfterHours": 48
        }
    });
    let cfg = Config::from_value(raw_cfg.clone()).unwrap();
    let s = start(H::build(Vec::new(), cfg)).await;
    let cfg_path = s.h.dir.path().join("t3cm-config.json");
    std::fs::write(&cfg_path, serde_json::to_string_pretty(&raw_cfg).unwrap()).unwrap();

    let mut claude_a = provider("claudeAgent", "claudeAgent", &["claude-opus-5-5", "claude-haiku-5-5"]);
    claude_a["models"][1]["options"] = json!([]);
    let mut claude_2 = provider("claude-2", "claudeAgent", &["claude-opus-5-5", "claude-haiku-5-5"]);
    claude_2["models"][1]["options"] = json!([]);
    s.h.fake.set_providers(vec![
        claude_a,
        claude_2,
        disabled_provider("claude-off", "claudeAgent"),
        codex_provider("codex", &["gpt-6-mini"]),
    ]);

    // Legacy board.instanceId and board.pool do NOT drive the ask box anymore.
    let models = s.get("/api/models").await;
    assert_eq!(models["ask"]["mode"], "model");
    assert_eq!(models["ask"]["source"], "default");
    assert_eq!(models["ask"]["instanceId"], "claudeAgent");
    assert_eq!(models["ask"]["model"], "claude-haiku-5-5");
    assert_eq!(models["ask"]["pool"], Value::Null);

    let r = s.post("/api/ask", json!({"question": "Default ask?"}), Some(&s.csrf()), None).await;
    assert_eq!(r.status(), 200);
    let a1: Value = r.json().await.unwrap();
    assert_eq!(a1["instanceId"], "claudeAgent");
    assert_eq!(a1["model"], "claude-haiku-5-5");
    assert_eq!(a1["modelSource"], "default");
    assert!(a1.get("pool").is_none(), "{a1}");

    // Reject invalid ask settings without changing config or runtime state.
    for bad in [
        json!({"mode": "t3-default"}),
        json!({"mode": "model", "provider": "claude-off", "model": "claude-haiku-5-5"}),
        json!({"mode": "model", "provider": "claudeAgent", "model": "claude-haiku-5-5", "effort": "high"}),
        json!({"mode": "pool", "pool": "missing"}),
    ] {
        let r = s.put("/api/models/ask", bad.clone(), Some(&s.csrf())).await;
        assert_eq!(r.status(), 400, "expected 400 for {bad}");
    }

    // 1. Switch ask box to Specific model mode on codex.
    let r = s
        .put(
            "/api/models/ask",
            json!({"mode": "model", "provider": "codex", "model": "gpt-6-mini", "effort": "low"}),
            Some(&s.csrf()),
        )
        .await;
    assert_eq!(r.status(), 200);
    let models: Value = r.json().await.unwrap();
    assert_eq!(models["ask"]["mode"], "model");
    assert_eq!(models["ask"]["source"], "board");
    assert_eq!(models["ask"]["instanceId"], "codex");
    assert_eq!(models["ask"]["model"], "gpt-6-mini");
    assert_eq!(models["ask"]["effort"], "low");

    // Persisted cleanly in t3cm-config.json (preserving other keys) and valid for Config::load.
    let reloaded = Config::load(&cfg_path).unwrap();
    assert_eq!(reloaded.deny, vec!["t3_project_delete"]);
    assert_eq!(reloaded.board.settle_after_hours, 48);
    assert_eq!(reloaded.board.ask_instance_id.as_deref(), Some("codex"));
    assert_eq!(reloaded.board.ask_model, "gpt-6-mini");
    assert_eq!(reloaded.board.ask_effort.as_deref(), Some("low"));
    assert_eq!(reloaded.board.ask_pool, None);

    // Next /api/ask uses the new setting with no restart.
    let a2: Value =
        s.post("/api/ask", json!({"question": "Codex ask?"}), Some(&s.csrf()), None).await.json().await.unwrap();
    assert_eq!(a2["instanceId"], "codex");
    assert_eq!(a2["model"], "gpt-6-mini");
    assert_eq!(a2["effort"], "low");
    assert_eq!(a2["modelSource"], "board");
    assert_eq!(
        s.h.fake.launched()[1]["modelSelection"],
        json!({"instanceId": "codex", "model": "gpt-6-mini", "options": [{"id": "reasoningEffort", "value": "low"}]})
    );

    // 2. Switch ask box to Pool mode.
    let r = s.put("/api/models/ask", json!({"mode": "pool", "pool": "cheap"}), Some(&s.csrf())).await;
    assert_eq!(r.status(), 200);
    let models: Value = r.json().await.unwrap();
    assert_eq!(models["ask"]["mode"], "pool");
    assert_eq!(models["ask"]["pool"], "cheap");
    assert_eq!(models["ask"]["source"], "board");

    let reloaded = Config::load(&cfg_path).unwrap();
    assert_eq!(reloaded.board.ask_pool.as_deref(), Some("cheap"));
    assert_eq!(reloaded.board.ask_instance_id, None);

    let a3: Value =
        s.post("/api/ask", json!({"question": "Pooled ask?"}), Some(&s.csrf()), None).await.json().await.unwrap();
    assert_eq!(a3["pool"], "cheap");
    assert_eq!(a3["model"], "claude-haiku-5-5");
    assert_eq!(a3["modelSource"], "board");
    assert!(a3["reason"].is_string(), "{a3}");

    // If the configured ask instance later disappears, ask fails with an error card and never falls back.
    s.put(
        "/api/models/ask",
        json!({"mode": "model", "provider": "claude-2", "model": "claude-haiku-5-5"}),
        Some(&s.csrf()),
    )
    .await;
    s.h.fake.set_providers(vec![
        provider("claudeAgent", "claudeAgent", &["claude-haiku-5-5"]),
        disabled_provider("claude-2", "claudeAgent"),
    ]);
    let r = s.post("/api/ask", json!({"question": "Should fail?"}), Some(&s.csrf()), None).await;
    assert_eq!(r.status(), 400);
    assert_eq!(s.h.fake.launched().len(), 3, "no fallback launch");
    assert_eq!(cards(&s.h).len(), 1);
    assert_eq!(cards(&s.h)[0]["kind"], "error");
}

#[tokio::test]
async fn board_page_shows_launch_model_repo_overrides_and_saves_picker_modes() {
    let (html, st) = two_gate_page().await;
    let initial = json!({
        "ticket": {"mode": "model", "source": "user", "provider": "claude-2", "instanceId": "claude-2", "model": "claude-opus-5-5", "effort": "high", "pool": null, "error": null},
        "ask": {"mode": "model", "source": "default", "provider": "claudeAgent", "instanceId": "claudeAgent", "model": "claude-haiku-5-5", "effort": null, "pool": null, "error": null},
        "providers": [
            {"id": "claudeAgent", "driver": "claudeAgent", "name": "Claude Prime", "models": [
                {"id": "claude-opus-5-5", "efforts": ["low", "medium", "high"], "defaultEffort": "medium"},
                {"id": "claude-haiku-5-5", "efforts": [], "defaultEffort": null}
            ]},
            {"id": "claude-2", "driver": "claudeAgent", "name": "Claude 2", "models": [
                {"id": "claude-opus-5-5", "efforts": ["low", "medium", "high"], "defaultEffort": "medium"}
            ]}
        ],
        "pools": [
            {"name": "opus", "mixed": false, "driver": "claudeAgent", "model": "claude-opus-5-5", "effort": "high", "strategy": "weighted", "members": [{"instanceId": "claudeAgent", "model": "claude-opus-5-5", "effort": "high"}]}
        ],
        "repoOverrides": [
            {"projectId": "p-lms", "title": "lms", "repo": "acme/lms", "mode": "model", "source": "AGENTS.md", "provider": "claude-2", "instanceId": "claude-2", "model": "claude-opus-5-5", "effort": "medium", "pool": null, "error": null}
        ],
        "capabilitiesError": null,
        "projectsError": null,
        "modelsRev": 0
    });
    let after_ticket = json!({
        "ticket": {"mode": "pool", "source": "user", "provider": null, "instanceId": null, "model": "claude-opus-5-5", "effort": "high", "pool": "opus", "mixed": false, "error": null},
        "ask": initial["ask"],
        "providers": initial["providers"],
        "pools": initial["pools"],
        "repoOverrides": initial["repoOverrides"],
        "capabilitiesError": null,
        "projectsError": null,
        "modelsRev": 1
    });
    let after_ask = json!({
        "ticket": after_ticket["ticket"],
        "ask": {"mode": "pool", "source": "board", "provider": null, "instanceId": null, "model": "claude-opus-5-5", "effort": "high", "pool": "opus", "mixed": false, "error": null},
        "providers": initial["providers"],
        "pools": initial["pools"],
        "repoOverrides": initial["repoOverrides"],
        "capabilitiesError": null,
        "projectsError": null,
        "modelsRev": 2
    });
    let r = run_page_async(
        &html,
        &format!(
            "async () => {{\n{PAGE_HELPERS}\nconst base = {st};\nconst initial = {initial};\nconst afterTicket = {after_ticket};\nconst afterAsk = {after_ask};\n{}\n}}",
            r##"
const tick = async () => { for (let i = 0; i < 6; i++) await Promise.resolve(); };
let curModels = initial;
fetchReply = (url, opts) => {
  const m = (opts && opts.method) || "GET";
  if (url === "/api/models" && m === "GET") return Promise.resolve({ ok: true, json: async () => curModels });
  if (url === "/api/models/ticket" && m === "PUT") { curModels = afterTicket; return Promise.resolve({ ok: true, json: async () => afterTicket }); }
  if (url === "/api/models/ask" && m === "PUT") { curModels = afterAsk; return Promise.resolve({ ok: true, json: async () => afterAsk }); }
  return Promise.resolve({ ok: true, json: async () => ({}) });
};
emitState(base);
await refreshModels(true);
const beforeLaunch = document.getElementById("launch-model-text").textContent;
const beforeAsk = document.getElementById("ask-model-text").textContent;

// Typing an issue from an overridden repo previews that repo's override before launch.
const issues = document.getElementById("issues");
issues.value = "https://github.com/acme/lms/issues/42";
issues.dispatch("input");
const withOverride = document.getElementById("launch-model-text").textContent;
issues.value = "";
issues.dispatch("input");

// Open picker dialog, check repo overrides list and save ticket pool mode + ask pool mode.
document.getElementById("launch-model-edit").dispatch("click");
await tick();
const ovrList = document.getElementById("md-repo-list").textContent;
document.getElementById("tm-mode-pool").dispatch("click");
document.getElementById("tm-save").dispatch("click");
await tick();
const afterTicketLine = document.getElementById("launch-model-text").textContent;

document.getElementById("am-mode-pool").dispatch("click");
document.getElementById("am-save").dispatch("click");
await tick();
const afterAskLine = document.getElementById("ask-model-text").textContent;
const puts = fetches.filter((f) => f.method === "PUT").map((f) => [f.url, JSON.parse(f.body)]);
return { beforeLaunch, beforeAsk, withOverride, ovrList, afterTicketLine, afterAskLine, puts };
"##
        ),
    );
    assert!(r["beforeLaunch"].as_str().unwrap().contains("claude-2 · claude-opus-5-5 · high (user config)"), "{r}");
    assert!(r["beforeLaunch"].as_str().unwrap().contains("lms overrides via AGENTS.md"), "{r}");
    assert!(r["beforeAsk"].as_str().unwrap().contains("claudeAgent · claude-haiku-5-5 (default)"), "{r}");
    assert!(
        r["withOverride"]
            .as_str()
            .unwrap()
            .contains("lms: claude-2 · claude-opus-5-5 · medium (AGENTS.md overrides user config)"),
        "{r}"
    );
    assert!(
        r["ovrList"].as_str().unwrap().contains("lms (acme/lms): AGENTS.md sets claude-2 · claude-opus-5-5 · medium"),
        "{r}"
    );
    assert!(r["afterTicketLine"].as_str().unwrap().contains("pool opus (claude-opus-5-5 · high) (user config)"), "{r}");
    assert!(r["afterAskLine"].as_str().unwrap().contains("pool opus (claude-opus-5-5 · high) (board setting)"), "{r}");
    assert_eq!(
        r["puts"],
        json!([
            ["/api/models/ticket", {"mode": "pool", "pool": "opus"}],
            ["/api/models/ask", {"mode": "pool", "pool": "opus"}]
        ])
    );
}

#[tokio::test]
async fn board_models_picker_preserves_catalog_only_models_exact_instance_and_validates_pools() {
    use t3_codemode::fake::{disabled_provider, provider};

    let raw_cfg = json!({
        "pools": {
            "opus": {
                "driver": "claudeAgent",
                "model": "claude-opus-5-5",
                "effort": "high",
                "members": [{"instanceId": "claudeAgent"}, {"instanceId": "claude-2"}]
            },
            "stale": {
                "driver": "claudeAgent",
                "model": "claude-opus-5-5",
                "effort": "high",
                "members": [{"instanceId": "claude-gone"}]
            },
            "wrong-driver": {
                "driver": "claudeAgent",
                "model": "claude-opus-5-5",
                "effort": "high",
                "members": [{"instanceId": "codex"}]
            }
        }
    });
    let cfg = Config::from_value(raw_cfg.clone()).unwrap();
    let s = start(H::build(Vec::new(), cfg)).await;
    let cfg_path = s.h.dir.path().join("t3cm-config.json");
    std::fs::write(&cfg_path, serde_json::to_string_pretty(&raw_cfg).unwrap()).unwrap();
    let repo = s.h.dir.path().join("widgets");
    std::fs::create_dir_all(&repo).unwrap();
    s.h.fake.set_projects(vec![project("p-widgets", &repo.to_string_lossy())]);
    s.h.remotes.set(&repo.to_string_lossy(), "git@github.com:acme/widgets.git");
    s.h.fake.set_providers(vec![
        provider("claudeAgent", "claudeAgent", &["claude-opus-5-5", "claude-haiku-5-5"]),
        provider("claude-2", "claudeAgent", &["claude-opus-5-5", "claude-catalog-only"]),
        codex_provider("codex", &["gpt-6.2-sol"]),
    ]);
    s.h.fake.set_short_list(1);

    // 1. Catalog-only saved model stays selectable in GET /api/models providers with its effort options.
    write_workflow_config(
        &s.h,
        json!({"version": 1, "models": {"ticket": {"provider": "claude-2", "model": "claude-catalog-only", "effort": "high"}}}),
    );
    let body = s.get("/api/models").await;
    assert_eq!(body["ticket"]["instanceId"], "claude-2");
    assert_eq!(body["ticket"]["model"], "claude-catalog-only");
    assert_eq!(body["ticket"]["effort"], "high");
    assert_eq!(body["ticket"]["error"], Value::Null);
    let claude2 = body["providers"].as_array().unwrap().iter().find(|p| p["id"] == "claude-2").unwrap();
    let cat = claude2["models"].as_array().unwrap().iter().find(|m| m["id"] == "claude-catalog-only").unwrap();
    assert!(cat["efforts"].as_array().unwrap().iter().any(|e| e == "medium"), "{cat}");

    // User can keep the catalog-only model and change only effort.
    let r = s
        .put(
            "/api/models/ticket",
            json!({"mode": "model", "provider": "claude-2", "model": "claude-catalog-only", "effort": "medium"}),
            Some(&s.csrf()),
        )
        .await;
    assert_eq!(r.status(), 200);
    let updated: Value = r.json().await.unwrap();
    assert_eq!(updated["ticket"]["model"], "claude-catalog-only");
    assert_eq!(updated["ticket"]["effort"], "medium");

    // 2. Pools with missing members or wrong driver are rejected before replacing a working choice.
    for bad_pool in ["stale", "wrong-driver"] {
        let rt = s.put("/api/models/ticket", json!({"mode": "pool", "pool": bad_pool}), Some(&s.csrf())).await;
        assert_eq!(rt.status(), 400, "ticket should reject {bad_pool}");
        let ra = s.put("/api/models/ask", json!({"mode": "pool", "pool": bad_pool}), Some(&s.csrf())).await;
        assert_eq!(ra.status(), 400, "ask should reject {bad_pool}");
    }
    let after_bad = s.get("/api/models").await;
    assert_eq!(after_bad["ticket"]["model"], "claude-catalog-only");

    // Temporary usage limits do not block saving an otherwise valid pool.
    s.h.exec("async () => wf.limit({ instance: 'claudeAgent', until: '2026-04-16T02:00:00Z' })").await;
    s.h.exec("async () => wf.limit({ instance: 'claude-2', until: '2026-04-16T02:00:00Z' })").await;
    let r = s.put("/api/models/ticket", json!({"mode": "pool", "pool": "opus"}), Some(&s.csrf())).await;
    assert_eq!(r.status(), 200);

    // 3. Removing a pool from disk prevents PUT /api/models/ask from writing an invalid config.
    std::fs::write(&cfg_path, serde_json::to_string_pretty(&json!({"pools": {}})).unwrap()).unwrap();
    let r = s.put("/api/models/ask", json!({"mode": "pool", "pool": "opus"}), Some(&s.csrf())).await;
    assert_eq!(r.status(), 400);
    let err: Value = r.json().await.unwrap();
    assert!(err["error"].as_str().unwrap().contains("no longer defined"), "{err}");
    assert!(Config::load(&cfg_path).is_ok(), "disk config must remain valid");
    let still_default = s.get("/api/models").await;
    assert_eq!(still_default["ask"]["mode"], "model");

    // 4. Saved ticket instance never falls back silently to a sibling instance with the same driverKind.
    s.h.fake.set_short_list(10);
    let r = s
        .put(
            "/api/models/ticket",
            json!({"mode": "model", "provider": "claudeAgent", "model": "claude-opus-5-5", "effort": "high"}),
            Some(&s.csrf()),
        )
        .await;
    assert_eq!(r.status(), 200);
    s.h.fake.set_providers(vec![
        disabled_provider("claudeAgent", "claudeAgent"),
        provider("claude-2", "claudeAgent", &["claude-opus-5-5"]),
    ]);
    let view = s.get("/api/models").await;
    assert!(view["ticket"]["error"].as_str().unwrap().contains("claudeAgent"), "{view}");
    assert_eq!(view["ticket"]["instanceId"], Value::Null);
    let launch: Value = s
        .post("/api/launch", json!({"issues": ["acme/widgets#42"]}), Some(&s.csrf()), None)
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(launch["results"][0]["ok"], false, "{launch}");
    assert!(launch["results"][0]["error"].as_str().unwrap().contains("claudeAgent"), "{launch}");
    assert!(s.h.fake.launched().is_empty(), "must not fall back to claude-2");
}

#[tokio::test]
async fn board_page_rejects_stale_model_snapshots_and_retries_failed_refreshes() {
    let (html, st) = two_gate_page().await;
    let rev1 = json!({
        "ticket": {"mode": "model", "source": "user", "provider": "claudeAgent", "instanceId": "claudeAgent", "model": "claude-opus-5-5", "effort": "high", "pool": null, "error": null},
        "ask": {"mode": "model", "source": "default", "provider": "claudeAgent", "instanceId": "claudeAgent", "model": "claude-haiku-5-5", "effort": null, "pool": null, "error": null},
        "providers": [{"id": "claudeAgent", "driver": "claudeAgent", "name": "Claude", "models": [{"id": "claude-opus-5-5", "efforts": ["high"], "defaultEffort": "high"}]}],
        "pools": [{"name": "opus", "mixed": false, "driver": "claudeAgent", "model": "claude-opus-5-5", "effort": "high", "strategy": "weighted", "members": [{"instanceId": "claudeAgent", "model": "claude-opus-5-5", "effort": "high"}]}],
        "repoOverrides": [],
        "capabilitiesError": null,
        "projectsError": null,
        "modelsRev": 1,
        "modelsSession": "sess-a"
    });
    let rev2 = json!({
        "ticket": {"mode": "pool", "source": "user", "provider": null, "instanceId": null, "model": "claude-opus-5-5", "effort": "high", "pool": "opus", "mixed": false, "error": null},
        "ask": rev1["ask"],
        "providers": rev1["providers"],
        "pools": rev1["pools"],
        "repoOverrides": [],
        "capabilitiesError": null,
        "projectsError": null,
        "modelsRev": 2,
        "modelsSession": "sess-a"
    });
    let rev3 = json!({
        "ticket": {"mode": "model", "source": "user", "provider": "claudeAgent", "instanceId": "claudeAgent", "model": "claude-opus-5-5", "effort": "high", "pool": null, "error": null},
        "ask": rev1["ask"],
        "providers": rev1["providers"],
        "pools": rev1["pools"],
        "repoOverrides": [],
        "capabilitiesError": null,
        "projectsError": null,
        "modelsRev": 3,
        "modelsSession": "sess-a"
    });
    let rev0_new = json!({
        "ticket": rev2["ticket"],
        "ask": rev1["ask"],
        "providers": rev1["providers"],
        "pools": rev1["pools"],
        "repoOverrides": [],
        "capabilitiesError": null,
        "projectsError": null,
        "modelsRev": 0,
        "modelsSession": "sess-b"
    });
    let r = run_page_async(
        &html,
        &format!(
            "async () => {{\n{PAGE_HELPERS}\nconst base = {st};\nconst rev1 = {rev1};\nconst rev2 = {rev2};\nconst rev3 = {rev3};\nconst rev0New = {rev0_new};\n{}\n}}",
            r##"
const tick = async () => { for (let i = 0; i < 6; i++) await Promise.resolve(); };
const stateWith = (rev, sess) => {
  const copy = JSON.parse(JSON.stringify(base));
  copy.service.modelsRev = rev;
  copy.service.modelsSession = sess;
  return copy;
};
let resolveLateGet = null;
let mode = "init";
let getCount = 0;
fetchReply = (url, opts) => {
  const m = (opts && opts.method) || "GET";
  if (url === "/api/models" && m === "GET") {
    getCount++;
    if (mode === "init") return Promise.resolve({ ok: true, json: async () => rev1 });
    if (mode === "late") return new Promise((res) => { resolveLateGet = () => res({ ok: true, json: async () => rev1 }); });
    if (mode === "fail") return Promise.resolve({ ok: false, status: 500, json: async () => ({ error: "boom" }) });
    if (mode === "rev3") return Promise.resolve({ ok: true, json: async () => rev3 });
    if (mode === "newSession") return Promise.resolve({ ok: true, json: async () => rev0New });
  }
  if (url === "/api/models/ticket" && m === "PUT") {
    return Promise.resolve({ ok: true, json: async () => rev2 });
  }
  return Promise.resolve({ ok: true, json: async () => ({}) });
};
emitState(stateWith(1, "sess-a"));
await refreshModels(true);

// 1. Start a GET (resolving to rev1), save via PUT (resolving to rev2), then finish the late GET.
mode = "late";
const lateP = refreshModels(false);
document.getElementById("tm-mode-pool").dispatch("click");
document.getElementById("tm-save").dispatch("click");
await tick();
resolveLateGet();
await lateP;
await tick();
const afterLateLine = document.getElementById("launch-model-text").textContent;
const afterLateRev = modelsState.appliedRev;

// 2. Revision moves to 3, GET fails -> error is surfaced and rev stays at 2.
mode = "fail";
emitState(stateWith(3, "sess-a"));
await tick();
const errBanner = document.getElementById("models-cap-err").textContent;
const errLine = document.getElementById("launch-model-text").textContent;
const revDuringFail = modelsState.appliedRev;

// 3. Next state event with same revision 3 retries and succeeds -> error clears and rev3 renders.
mode = "rev3";
emitState(stateWith(3, "sess-a"));
await tick();
const afterRetryLine = document.getElementById("launch-model-text").textContent;
const afterRetryErr = document.getElementById("models-cap-err").textContent;
const afterRetryRev = modelsState.appliedRev;

// 4. Repeated failures stay bounded at MAX_MODEL_RETRIES.
mode = "fail";
const beforeBound = getCount;
for (let i = 0; i < 6; i++) {
  emitState(stateWith(4, "sess-a"));
  await tick();
}
const boundedAttempts = getCount - beforeBound;

// 5. New service session resets modelsRev to 0 and is accepted.
mode = "newSession";
emitState(stateWith(0, "sess-b"));
await tick();
const afterRestartLine = document.getElementById("launch-model-text").textContent;
const afterRestartRev = modelsState.appliedRev;

return {
  afterLateLine, afterLateRev, errBanner, errLine, revDuringFail,
  afterRetryLine, afterRetryErr, afterRetryRev, boundedAttempts,
  afterRestartLine, afterRestartRev
};
"##
        ),
    );
    assert!(r["afterLateLine"].as_str().unwrap().contains("pool opus"), "{r}");
    assert_eq!(r["afterLateRev"], 2, "{r}");
    assert!(r["errBanner"].as_str().unwrap().contains("Could not refresh models"), "{r}");
    assert!(r["errLine"].as_str().unwrap().contains("Could not refresh models"), "{r}");
    assert_eq!(r["revDuringFail"], 2, "{r}");
    assert!(r["afterRetryLine"].as_str().unwrap().contains("claudeAgent · claude-opus-5-5 · high"), "{r}");
    assert_eq!(r["afterRetryErr"], "", "{r}");
    assert_eq!(r["afterRetryRev"], 3, "{r}");
    assert_eq!(r["boundedAttempts"], 4, "1 initial + 3 bounded retries: {r}");
    assert!(r["afterRestartLine"].as_str().unwrap().contains("pool opus"), "{r}");
    assert_eq!(r["afterRestartRev"], 0, "{r}");
}

#[test]
fn board_remote_config_defaults_off_and_validates() {
    use t3_codemode::config::{RemoteMode, RemoteOrigin};

    let def = Config::default();
    assert!(def.board.remote.is_none());
    assert!(def.board.active_remote().is_none());

    for off in [
        json!({"board": {"remote": null}}),
        json!({"board": {"remote": {}}}),
        json!({"board": {"remote": {"mode": "off"}}}),
        json!({"board": {"remote": {"mode": "off", "origin": "https://board.acme.example:47732"}}}),
    ] {
        let cfg = Config::from_value(off.clone()).unwrap_or_else(|e| panic!("{off}: {e:#}"));
        assert!(cfg.board.active_remote().is_none(), "{off}");
    }

    let pairing = Config::from_value(json!({
        "board": {"remote": {"mode": "pairing", "origin": "https://board.acme.example:47732"}}
    }))
    .unwrap();
    let (mode, origin) = pairing.board.active_remote().unwrap();
    assert_eq!(mode, RemoteMode::Pairing);
    assert_eq!(origin, RemoteOrigin { host: "board.acme.example".into(), port: 47732 });
    assert_eq!(origin.canonical(), "https://board.acme.example:47732");

    let ts = Config::from_value(json!({
        "board": {"remote": {"mode": "tailscale", "origin": "https://BOX.ACME.TS.NET"}}
    }))
    .unwrap();
    let (mode, origin) = ts.board.active_remote().unwrap();
    assert_eq!(mode, RemoteMode::Tailscale);
    assert_eq!(origin, RemoteOrigin { host: "box.acme.ts.net".into(), port: 443 });
    assert_eq!(origin.canonical(), "https://box.acme.ts.net");
    assert!(origin.matches_host("box.acme.ts.net"));
    assert!(origin.matches_host("BOX.ACME.TS.NET:443"));
    assert!(!origin.matches_host("box.acme.ts.net:80"));
    assert!(origin.matches_origin("https://box.acme.ts.net"));
    assert!(origin.matches_origin("https://box.acme.ts.net:443"));
    assert!(!origin.matches_origin("http://box.acme.ts.net"));

    let v6 = Config::from_value(json!({
        "board": {"remote": {"mode": "pairing", "origin": "https://[2001:0db8::1]:47732"}}
    }))
    .unwrap();
    let (_, origin) = v6.board.active_remote().unwrap();
    assert_eq!(origin.host, "[2001:db8::1]");
    assert_eq!(origin.port, 47732);
    assert!(origin.matches_host("[2001:db8::1]:47732"));

    let v4mapped = Config::from_value(json!({
        "board": {"remote": {"mode": "pairing", "origin": "https://[::ffff:192.0.2.1]:47732"}}
    }))
    .unwrap();
    let (_, origin) = v4mapped.board.active_remote().unwrap();
    assert!(origin.matches_host("[::ffff:192.0.2.1]:47732"));
    assert!(origin.matches_origin("https://[::ffff:192.0.2.1]:47732"));

    for bad in [
        json!({"board": {"remote": {"mode": "pairing"}}}),
        json!({"board": {"remote": {"mode": "tailscale"}}}),
        json!({"board": {"remote": {"mode": "pairing", "origin": "http://board.acme.example:47732"}}}),
        json!({"board": {"remote": {"mode": "pairing", "origin": "https://board.acme.example/"}}}),
        json!({"board": {"remote": {"mode": "pairing", "origin": "https://board.acme.example/board"}}}),
        json!({"board": {"remote": {"mode": "pairing", "origin": "https://board.acme.example?x=1"}}}),
        json!({"board": {"remote": {"mode": "pairing", "origin": "https://board.acme.example#frag"}}}),
        json!({"board": {"remote": {"mode": "pairing", "origin": "https://user:pass@board.acme.example"}}}),
        json!({"board": {"remote": {"mode": "pairing", "origin": "https://board.acme.example:0"}}}),
        json!({"board": {"remote": {"mode": "pairing", "origin": "https://board.acme.example:"}}}),
        json!({"board": {"remote": {"mode": "pairing", "origin": "https://board.acme.example:0443"}}}),
        json!({"board": {"remote": {"mode": "pairing", "origin": "https://192.0.513:47732"}}}),
        json!({"board": {"remote": {"mode": "pairing", "origin": "https://0177.0.0.1:47732"}}}),
        json!({"board": {"remote": {"mode": "pairing", "origin": " https://board.acme.example "}}}),
        json!({"board": {"remote": {"mode": "off", "origin": "http://board.acme.example"}}}),
        json!({"board": {"remote": {"mode": "bogus", "origin": "https://board.acme.example"}}}),
        json!({"board": {"remote": {"mode": "off", "unknown": true}}}),
    ] {
        assert!(Config::from_value(bad.clone()).is_err(), "expected invalid config: {bad}");
    }
}

#[test]
fn request_classifier_marks_only_direct_loopback_as_local() {
    use axum::http::{HeaderMap, HeaderName, HeaderValue, header};
    use t3_codemode::server::{RequestKind, classify_request};

    let make = |pairs: &[(&str, &str)]| {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.append(HeaderName::from_bytes(k.as_bytes()).unwrap(), HeaderValue::from_str(v).unwrap());
        }
        h
    };

    for host in ["127.0.0.1:47731", "localhost:47731", "LOCALHOST:47731", "[::1]:47731"] {
        assert_eq!(classify_request(&make(&[("host", host)]), 47731), RequestKind::Local, "Host={host}");
    }

    for host in [
        "127.0.0.1:47732",
        "localhost",
        "board.acme.example:47732",
        "127.0.0.1:0",
        "[::1]junk",
        "127.0.0.1@evil.example:47731",
    ] {
        assert_eq!(classify_request(&make(&[("host", host)]), 47731), RequestKind::Remote, "Host={host}");
    }

    // Missing Host or duplicate Host headers are always remote.
    assert_eq!(classify_request(&HeaderMap::new(), 47731), RequestKind::Remote);
    let mut dup = HeaderMap::new();
    dup.append(header::HOST, HeaderValue::from_static("127.0.0.1:47731"));
    dup.append(header::HOST, HeaderValue::from_static("127.0.0.1:47731"));
    assert_eq!(classify_request(&dup, 47731), RequestKind::Remote);

    // Any proxy header classifies an otherwise-loopback request as remote.
    for (k, v) in [
        ("tailscale-user-login", "alice@acme.example"),
        ("tailscale-user-login", ""),
        ("x-forwarded-host", "board.acme.example:47732"),
        ("x-forwarded-for", "100.64.0.1"),
        ("x-forwarded-proto", "https"),
        ("x-forwarded-port", "47732"),
        ("forwarded", "for=100.64.0.1;proto=https"),
    ] {
        let h = make(&[("host", "127.0.0.1:47731"), (k, v)]);
        assert_eq!(classify_request(&h, 47731), RequestKind::Remote, "{k}={v:?}");
    }
}

#[tokio::test]
async fn board_refuses_remote_and_proxied_requests_when_remote_is_off() {
    let s = start(H::new()).await;
    join_board(&s.h, TICKET, "widgets#41").await;
    let sent = s.h.wf(TICKET, "send", json!({"kind": "gate", "gate": "merge", "body": GATE})).await;
    let handle = sent["handle"].as_str().unwrap().to_string();
    let port = s.base.rsplit(':').next().unwrap().to_string();
    let csrf = s.csrf();

    let read_routes = vec![
        "/".to_string(),
        "/api/state".to_string(),
        "/api/events".to_string(),
        format!("/api/msg/{handle}"),
        "/api/models".to_string(),
        "/api/contexts".to_string(),
        "/api/new-issues".to_string(),
    ];

    // Direct loopback on 127.0.0.1, localhost, and [::1] works as today.
    for host in [format!("127.0.0.1:{port}"), format!("localhost:{port}"), format!("[::1]:{port}")] {
        let origin = format!("http://{host}");
        for route in &read_routes {
            if route == "/api/events" {
                let r = s
                    .http
                    .get(format!("{}{route}", s.base))
                    .header("Host", &host)
                    .header("Origin", &origin)
                    .send()
                    .await
                    .unwrap();
                assert_eq!(r.status(), 200, "local {route} Host={host}");
                drop(r);
            } else {
                let r = s
                    .http
                    .get(format!("{}{route}", s.base))
                    .header("Host", &host)
                    .header("Origin", &origin)
                    .send()
                    .await
                    .unwrap();
                assert_eq!(r.status(), 200, "local {route} Host={host}");
            }
        }
    }

    // With board.remote off, every read route refuses non-loopback hosts and every proxy header with 403 JSON.
    let remote_cases: Vec<(&str, Vec<(&str, &str)>)> = vec![
        ("non-loopback host", vec![("Host", "board.acme.example:47732")]),
        ("wrong loopback port", vec![("Host", "127.0.0.1:1")]),
        ("Tailscale-User-Login", vec![("Tailscale-User-Login", "alice@acme.example")]),
        ("X-Forwarded-Host", vec![("X-Forwarded-Host", "board.acme.example:47732")]),
        ("X-Forwarded-For", vec![("X-Forwarded-For", "100.64.0.1")]),
        ("X-Forwarded-Proto", vec![("X-Forwarded-Proto", "https")]),
        ("X-Forwarded-Port", vec![("X-Forwarded-Port", "47732")]),
        ("Forwarded", vec![("Forwarded", "for=100.64.0.1;proto=https")]),
    ];

    for (label, hdrs) in &remote_cases {
        for route in &read_routes {
            let mut req = s.http.get(format!("{}{route}", s.base));
            for (k, v) in hdrs {
                req = req.header(*k, *v);
            }
            let r = req.send().await.unwrap();
            assert_eq!(r.status(), 403, "{label} GET {route}");
            let body: Value = r.json().await.unwrap();
            assert!(body["error"].is_string(), "{label} GET {route}: {body}");
        }
        // Mutating routes are refused even when carrying a valid X-T3CM-CSRF token.
        let mut req = s
            .http
            .post(format!("{}/api/cards/a1/answer", s.base))
            .header("X-T3CM-CSRF", &csrf)
            .json(&json!({"text": "merge"}));
        for (k, v) in hdrs {
            req = req.header(*k, *v);
        }
        let r = req.send().await.unwrap();
        assert_eq!(r.status(), 403, "{label} POST /api/cards/a1/answer");

        let mut req = s
            .http
            .put(format!("{}/api/contexts/c1", s.base))
            .header("X-T3CM-CSRF", &csrf)
            .json(&json!({"name": "Work", "spec": {"projects": [], "statuses": [], "kinds": []}}));
        for (k, v) in hdrs {
            req = req.header(*k, *v);
        }
        let r = req.send().await.unwrap();
        assert_eq!(r.status(), 403, "{label} PUT /api/contexts/c1");
    }

    // Nothing was mutated.
    assert!(s.h.sent().is_empty());
    assert_eq!(cards(&s.h).len(), 1);
    assert!(s.h.app.wf.contexts().unwrap().is_empty());
}

#[tokio::test]
async fn board_validates_configured_remote_origin_and_refuses_without_auth() {
    for mode in ["pairing", "tailscale"] {
        let cfg = Config::from_value(json!({
            "board": {
                "remote": {
                    "mode": mode,
                    "origin": "https://board.acme.example:47732"
                }
            }
        }))
        .unwrap();
        let s = start(H::build(Vec::new(), cfg)).await;
        join_board(&s.h, TICKET, "widgets#41").await;
        let sent = s.h.wf(TICKET, "send", json!({"kind": "gate", "gate": "merge", "body": GATE})).await;
        let handle = sent["handle"].as_str().unwrap().to_string();
        let port = s.base.rsplit(':').next().unwrap().to_string();
        let csrf = s.csrf();
        let read_routes = vec![
            "/".to_string(),
            "/api/state".to_string(),
            "/api/events".to_string(),
            format!("/api/msg/{handle}"),
            "/api/contexts".to_string(),
            "/api/new-issues".to_string(),
        ];

        // 1. Mismatched Host, X-Forwarded-Host, X-Forwarded-Proto, or Origin gets 403.
        let bad_cases: Vec<(&str, Vec<(&str, &str)>)> = vec![
            ("wrong host", vec![("Host", "evil.example:47732")]),
            ("wrong port on host", vec![("Host", "board.acme.example:443")]),
            ("wrong X-Forwarded-Host", vec![("X-Forwarded-Host", "evil.example:47732")]),
            (
                "host good but X-Forwarded-Host evil",
                vec![("Host", "board.acme.example:47732"), ("X-Forwarded-Host", "evil.example:47732")],
            ),
            (
                "host evil but X-Forwarded-Host good",
                vec![("Host", "evil.example:47732"), ("X-Forwarded-Host", "board.acme.example:47732")],
            ),
            ("loopback host + X-Forwarded-For without X-Forwarded-Host", vec![("X-Forwarded-For", "100.64.0.1")]),
            (
                "loopback host + Tailscale-User-Login without X-Forwarded-Host",
                vec![("Tailscale-User-Login", "alice@acme.example")],
            ),
            ("X-Forwarded-Proto http", vec![("Host", "board.acme.example:47732"), ("X-Forwarded-Proto", "http")]),
            (
                "wrong Origin on remote host",
                vec![("Host", "board.acme.example:47732"), ("Origin", "https://evil.example")],
            ),
            (
                "plain http Origin on remote host",
                vec![("Host", "board.acme.example:47732"), ("Origin", "http://board.acme.example:47732")],
            ),
            ("null Origin on remote host", vec![("Host", "board.acme.example:47732"), ("Origin", "null")]),
            ("remote Origin on direct loopback host", vec![("Origin", "https://board.acme.example:47732")]),
        ];
        for (label, hdrs) in &bad_cases {
            let mut req = s.http.get(format!("{}/api/state", s.base));
            for (k, v) in hdrs {
                req = req.header(*k, *v);
            }
            let r = req.send().await.unwrap();
            assert_eq!(r.status(), 403, "mode={mode} {label}");
            let body: Value = r.json().await.unwrap();
            assert!(body["error"].is_string(), "mode={mode} {label}: {body}");
        }

        // 2. Valid remote Host + Origin (preserved Host or loopback Host + X-Forwarded-Host) reaches
        //    the auth check and is refused with 401 JSON on every board read and write route.
        let good_remote_forms: Vec<(&str, Vec<(&str, &str)>)> = vec![
            (
                "preserved Host",
                vec![
                    ("Host", "board.acme.example:47732"),
                    ("Origin", "https://board.acme.example:47732"),
                    ("X-Forwarded-Proto", "https"),
                    ("Tailscale-User-Login", "alice@acme.example"),
                ],
            ),
            (
                "rewritten loopback Host + X-Forwarded-Host",
                vec![
                    ("X-Forwarded-Host", "board.acme.example:47732"),
                    ("Origin", "https://board.acme.example:47732"),
                    ("X-Forwarded-Proto", "https"),
                    ("X-Forwarded-For", "100.64.0.1"),
                    ("Tailscale-User-Login", "alice@acme.example"),
                ],
            ),
        ];
        for (form, hdrs) in &good_remote_forms {
            for route in &read_routes {
                let mut req = s.http.get(format!("{}{route}", s.base));
                for (k, v) in hdrs {
                    req = req.header(*k, *v);
                }
                let r = req.send().await.unwrap();
                assert_eq!(r.status(), 401, "mode={mode} {form} GET {route}");
                let body: Value = r.json().await.unwrap();
                assert!(body["error"].is_string(), "mode={mode} {form} GET {route}: {body}");
            }
            let mut req = s
                .http
                .post(format!("{}/api/cards/a1/answer", s.base))
                .header("X-T3CM-CSRF", &csrf)
                .json(&json!({"text": "merge"}));
            for (k, v) in hdrs {
                req = req.header(*k, *v);
            }
            let r = req.send().await.unwrap();
            assert_eq!(r.status(), 401, "mode={mode} {form} POST /api/cards/a1/answer");

            let mut req = s
                .http
                .put(format!("{}/api/contexts/c1", s.base))
                .header("X-T3CM-CSRF", &csrf)
                .json(&json!({"name": "Work", "spec": {"projects": [], "statuses": [], "kinds": []}}));
            for (k, v) in hdrs {
                req = req.header(*k, *v);
            }
            let r = req.send().await.unwrap();
            assert_eq!(r.status(), 401, "mode={mode} {form} PUT /api/contexts/c1");
        }

        // 3. Direct loopback still works with board.remote enabled, and CSRF is still required.
        let r = s.http.get(format!("{}/api/state", s.base)).send().await.unwrap();
        assert_eq!(r.status(), 200, "mode={mode} direct loopback GET /api/state");
        let r = s
            .http
            .get(format!("{}/api/state", s.base))
            .header("Host", format!("[::1]:{port}"))
            .header("Origin", format!("http://[::1]:{port}"))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 200, "mode={mode} ipv6 loopback GET /api/state");
        assert_eq!(
            s.post("/api/cards/a1/answer", json!({"text": "merge"}), None, None).await.status(),
            403,
            "mode={mode} local POST without CSRF"
        );
        assert_eq!(
            s.post("/api/cards/a1/answer", json!({"text": "merge"}), Some(&csrf), Some(&s.base)).await.status(),
            200,
            "mode={mode} local POST with CSRF"
        );
    }
}

fn pairing_config() -> Config {
    Config::from_value(json!({
        "board": {
            "remote": {
                "mode": "pairing",
                "origin": "https://board.acme.example:47732"
            }
        }
    }))
    .unwrap()
}

fn cookie_pair_from_set_cookie(header_val: &str) -> String {
    header_val.split(';').next().unwrap().trim().to_string()
}

#[test]
fn pairing_tokens_and_sessions_are_stored_hashed_and_enforce_ttls() {
    use t3_codemode::clock::{Clock, FakeClock, iso};
    use t3_codemode::config::RemoteOrigin;
    use t3_codemode::pairing;
    use t3_codemode::store::Store;

    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let clock = FakeClock::at(T0);
    let origin = RemoteOrigin::parse("https://board.acme.example:47732").unwrap();

    // Mint a pairing token with a label; verify URL uses fragment and only the SHA-256 digest is stored.
    let minted = pairing::mint_pairing_token(&store, &origin, Some("Phone"), clock.now()).unwrap();
    assert_eq!(minted.url, format!("https://board.acme.example:47732/pair#{}", minted.token));
    assert_eq!(minted.label.as_deref(), Some("Phone"));
    assert_eq!(minted.ttl_seconds, 300);
    let conn = rusqlite::Connection::open(store.path()).unwrap();
    let stored_tok: String = conn.query_row("SELECT token_hash FROM pairing_tokens", [], |r| r.get(0)).unwrap();
    assert_eq!(stored_tok, pairing::hash_token(&minted.token));
    assert_ne!(stored_tok, minted.token);

    // Redeem once succeeds and produces a 30-day session storing only the SHA-256 digest.
    let (raw_session, session) =
        pairing::redeem_pairing_token(&store, &minted.token, Some("Ignored"), clock.now()).unwrap().unwrap();
    assert_eq!(session.id, "s1");
    assert_eq!(session.label.as_deref(), Some("Phone"));
    let stored_sess: String = conn.query_row("SELECT token_hash FROM board_sessions", [], |r| r.get(0)).unwrap();
    assert_eq!(stored_sess, pairing::hash_token(&raw_session));
    assert_ne!(stored_sess, raw_session);

    // Redeeming the same pairing token a second time fails.
    assert!(pairing::redeem_pairing_token(&store, &minted.token, None, clock.now()).unwrap().is_none());

    // Expired pairing token (at exact 5-minute TTL) fails.
    let exp_tok = pairing::mint_pairing_token(&store, &origin, None, clock.now()).unwrap();
    clock.advance_minutes(5);
    assert!(pairing::redeem_pairing_token(&store, &exp_tok.token, None, clock.now()).unwrap().is_none());

    // Session verifies and updates last_seen_at before 30 days, then expires at exact 30-day boundary.
    let sess_hash = pairing::hash_token(&raw_session);
    let verified = store.board_session_verify(&sess_hash, &iso(clock.now())).unwrap().unwrap();
    assert_eq!(verified.id, "s1");
    assert_eq!(verified.last_seen_at, iso(clock.now()));
    assert_eq!(store.board_sessions_active(&iso(clock.now())).unwrap().len(), 1);

    clock.set("2026-12-17T11:59:59Z");
    assert!(store.board_session_verify(&sess_hash, &iso(clock.now())).unwrap().is_some());
    clock.set("2026-12-17T12:00:00Z");
    assert!(store.board_session_verify(&sess_hash, &iso(clock.now())).unwrap().is_none());
    assert!(store.board_sessions_active(&iso(clock.now())).unwrap().is_empty());

    // QR rendering produces a non-empty Unicode matrix.
    let qr = pairing::render_qr(&minted.url).unwrap();
    assert!(qr.lines().count() > 10 && qr.contains('\u{2588}') || qr.contains('\u{2580}') || qr.contains('\u{2584}'));
}

#[tokio::test]
async fn pairing_flow_sets_secure_cookie_and_authenticates_remote_board() {
    use t3_codemode::clock::Clock;
    use t3_codemode::config::RemoteOrigin;
    use t3_codemode::pairing;

    // 1. When board.remote is off, /pair and /api/pair are refused with 403.
    let off = start(H::new()).await;
    let r = off.http.get(format!("{}/pair", off.base)).header("Host", "board.acme.example:47732").send().await.unwrap();
    assert_eq!(r.status(), 403);
    let r = off
        .http
        .post(format!("{}/api/pair", off.base))
        .header("Host", "board.acme.example:47732")
        .json(&json!({"token": "anything"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 403);
    let r = off.http.get(format!("{}/pair", off.base)).send().await.unwrap();
    assert_eq!(r.status(), 403);

    // 2. In pairing mode, /pair and /api/pair still enforce Host, X-Forwarded-Proto, and Origin.
    let s = start(H::build(Vec::new(), pairing_config())).await;
    join_board(&s.h, TICKET, "widgets#41").await;
    let sent = s.h.wf(TICKET, "send", json!({"kind": "gate", "gate": "merge", "body": GATE})).await;
    let handle = sent["handle"].as_str().unwrap().to_string();
    let csrf = s.csrf();
    let origin = RemoteOrigin::parse("https://board.acme.example:47732").unwrap();

    for (label, hdrs) in [
        ("wrong host on /pair", vec![("Host", "evil.example:47732"), ("X-Forwarded-Proto", "https")]),
        ("http proto on /pair", vec![("Host", "board.acme.example:47732"), ("X-Forwarded-Proto", "http")]),
        (
            "wrong origin on /pair",
            vec![
                ("Host", "board.acme.example:47732"),
                ("X-Forwarded-Proto", "https"),
                ("Origin", "https://evil.example"),
            ],
        ),
    ] {
        let mut req = s.http.get(format!("{}/pair", s.base));
        for (k, v) in &hdrs {
            req = req.header(*k, *v);
        }
        assert_eq!(req.send().await.unwrap().status(), 403, "{label}");
    }

    // 3. Valid remote Host + https proto can load GET /pair without a session cookie, and /pair never leaks CSRF.
    let r = s
        .http
        .get(format!("{}/pair", s.base))
        .header("Host", "board.acme.example:47732")
        .header("X-Forwarded-Proto", "https")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(r.headers().get("cache-control").and_then(|v| v.to_str().ok()), Some("no-store"));
    assert_eq!(r.headers().get("referrer-policy").and_then(|v| v.to_str().ok()), Some("no-referrer"));
    assert!(r.headers().contains_key("content-security-policy"));
    let pair_html = r.text().await.unwrap();
    assert!(pair_html.contains("/api/pair"));
    assert!(pair_html.contains("history.replaceState"));
    assert!(!pair_html.contains(&csrf), "/pair must never leak the board CSRF token");

    // 4. POST /api/pair rejects non-JSON Content-Type and bogus tokens.
    let r = s
        .http
        .post(format!("{}/api/pair", s.base))
        .header("Host", "board.acme.example:47732")
        .header("Origin", "https://board.acme.example:47732")
        .header("X-Forwarded-Proto", "https")
        .header("Content-Type", "text/plain")
        .body(r#"{"token":"bogus"}"#)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 415);

    let r = s
        .http
        .post(format!("{}/api/pair", s.base))
        .header("Host", "board.acme.example:47732")
        .header("Origin", "https://board.acme.example:47732")
        .header("X-Forwarded-Proto", "https")
        .json(&json!({"token": "not-a-real-token"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 401);
    assert!(r.headers().get("set-cookie").is_none());

    // 5. Mint a pairing token and redeem it via POST /api/pair.
    let minted = pairing::mint_pairing_token(&s.h.app.store, &origin, Some("Phone"), s.h.clock.now()).unwrap();
    let r = s
        .http
        .post(format!("{}/api/pair", s.base))
        .header("Host", "board.acme.example:47732")
        .header("Origin", "https://board.acme.example:47732")
        .header("X-Forwarded-Proto", "https")
        .json(&json!({"token": minted.token}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(r.headers().get("cache-control").and_then(|v| v.to_str().ok()), Some("no-store"));
    let set_cookie = r.headers().get("set-cookie").expect("Set-Cookie header").to_str().unwrap().to_string();
    assert!(set_cookie.starts_with(&format!("{}=", pairing::SESSION_COOKIE)), "{set_cookie}");
    assert!(set_cookie.contains("HttpOnly"), "{set_cookie}");
    assert!(set_cookie.contains("Secure"), "{set_cookie}");
    assert!(set_cookie.contains("SameSite=Strict"), "{set_cookie}");
    assert!(set_cookie.contains("Path=/"), "{set_cookie}");
    assert!(set_cookie.contains("Max-Age=2592000"), "{set_cookie}");
    let body: Value = r.json().await.unwrap();
    assert_eq!(body["ok"], true);
    assert_eq!(body["session"]["id"], "s1");
    assert_eq!(body["session"]["label"], "Phone");
    let cookie = cookie_pair_from_set_cookie(&set_cookie);

    // 6. The same pairing token cannot be redeemed a second time.
    let r = s
        .http
        .post(format!("{}/api/pair", s.base))
        .header("Host", "board.acme.example:47732")
        .header("Origin", "https://board.acme.example:47732")
        .header("X-Forwarded-Proto", "https")
        .json(&json!({"token": minted.token}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 401);
    assert!(r.headers().get("set-cookie").is_none());

    // 7. A pairing token past its 5-minute TTL cannot be redeemed.
    let expired = pairing::mint_pairing_token(&s.h.app.store, &origin, None, s.h.clock.now()).unwrap();
    s.h.clock.advance_minutes(5);
    let r = s
        .http
        .post(format!("{}/api/pair", s.base))
        .header("Host", "board.acme.example:47732")
        .header("Origin", "https://board.acme.example:47732")
        .header("X-Forwarded-Proto", "https")
        .json(&json!({"token": expired.token}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 401);

    // 8. With the session cookie, every remote board read route succeeds, and /mcp + /healthz stay 403.
    for route in ["/", "/api/state", &format!("/api/msg/{handle}"), "/api/contexts", "/api/new-issues", "/api/flow"] {
        let r = s
            .http
            .get(format!("{}{route}", s.base))
            .header("Host", "board.acme.example:47732")
            .header("X-Forwarded-Proto", "https")
            .header("Cookie", &cookie)
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 200, "paired GET {route}");
    }
    let r = s
        .http
        .post(format!("{}/mcp", s.base))
        .header("Host", "board.acme.example:47732")
        .header("X-Forwarded-Proto", "https")
        .header("Cookie", &cookie)
        .json(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 403, "/mcp stays strictly loopback-only even with a valid pairing cookie");
    let r = s
        .http
        .get(format!("{}/healthz", s.base))
        .header("Host", "board.acme.example:47732")
        .header("X-Forwarded-Proto", "https")
        .header("Cookie", &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 403, "/healthz stays strictly loopback-only even with a valid pairing cookie");

    // 9. Duplicate session cookies fail closed with 401.
    let r = s
        .http
        .get(format!("{}/api/state", s.base))
        .header("Host", "board.acme.example:47732")
        .header("X-Forwarded-Proto", "https")
        .header("Cookie", format!("{cookie}; {}=bogus", pairing::SESSION_COOKIE))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 401);

    // 10. Mutating routes still require X-T3CM-CSRF even with a valid session cookie.
    let r = s
        .http
        .post(format!("{}/api/cards/a1/answer", s.base))
        .header("Host", "board.acme.example:47732")
        .header("Origin", "https://board.acme.example:47732")
        .header("X-Forwarded-Proto", "https")
        .header("Cookie", &cookie)
        .json(&json!({"text": "merge"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 403);

    let r = s
        .http
        .post(format!("{}/api/cards/a1/answer", s.base))
        .header("Host", "board.acme.example:47732")
        .header("Origin", "https://board.acme.example:47732")
        .header("X-Forwarded-Proto", "https")
        .header("Cookie", &cookie)
        .header("X-T3CM-CSRF", &csrf)
        .json(&json!({"text": "merge"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(s.h.sent().len(), 1);

    // 11. After 30 days the session expires and remote requests return 401.
    s.h.clock.advance_minutes(30 * 24 * 60);
    let r = s
        .http
        .get(format!("{}/api/state", s.base))
        .header("Host", "board.acme.example:47732")
        .header("X-Forwarded-Proto", "https")
        .header("Cookie", &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 401);
}

#[tokio::test]
async fn revoking_or_expiring_a_session_closes_its_open_sse_stream_and_cuts_off_api() {
    use t3_codemode::clock::{Clock, iso};
    use t3_codemode::config::RemoteOrigin;
    use t3_codemode::pairing;

    let s = start(H::build(Vec::new(), pairing_config())).await;
    join_board(&s.h, TICKET, "widgets#41").await;
    let origin = RemoteOrigin::parse("https://board.acme.example:47732").unwrap();

    let pair = |label: &'static str| {
        let minted = pairing::mint_pairing_token(&s.h.app.store, &origin, Some(label), s.h.clock.now()).unwrap();
        let (raw_session, sess) =
            pairing::redeem_pairing_token(&s.h.app.store, &minted.token, None, s.h.clock.now()).unwrap().unwrap();
        (sess, format!("{}={raw_session}", pairing::SESSION_COOKIE))
    };
    let (sess1, cookie1) = pair("Phone");
    let (sess2, cookie2) = pair("Laptop");

    let mut sse1 = s
        .http
        .get(format!("{}/api/events", s.base))
        .header("Host", "board.acme.example:47732")
        .header("X-Forwarded-Proto", "https")
        .header("Cookie", &cookie1)
        .send()
        .await
        .unwrap();
    assert_eq!(sse1.status(), 200);
    let first1 = tokio::time::timeout(Duration::from_secs(5), sse1.chunk()).await.unwrap().unwrap().unwrap();
    assert!(String::from_utf8_lossy(&first1).contains("event: state"));

    let mut sse2 = s
        .http
        .get(format!("{}/api/events", s.base))
        .header("Host", "board.acme.example:47732")
        .header("X-Forwarded-Proto", "https")
        .header("Cookie", &cookie2)
        .send()
        .await
        .unwrap();
    assert_eq!(sse2.status(), 200);
    let first2 = tokio::time::timeout(Duration::from_secs(5), sse2.chunk()).await.unwrap().unwrap().unwrap();
    assert!(String::from_utf8_lossy(&first2).contains("event: state"));

    // Revoke session 1 directly in store.db (as `t3cm board revoke` does from another process).
    assert!(s.h.app.store.board_session_revoke(sess1.seq, &iso(s.h.clock.now())).unwrap());

    // Session 1's API access fails immediately with 401.
    let r = s
        .http
        .get(format!("{}/api/state", s.base))
        .header("Host", "board.acme.example:47732")
        .header("X-Forwarded-Proto", "https")
        .header("Cookie", &cookie1)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 401);

    // Session 1's open SSE stream closes cleanly without sending new state.
    let closed1 = tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(chunk) = sse1.chunk().await.unwrap() {
            let text = String::from_utf8_lossy(&chunk);
            assert!(!text.contains("event: state"), "revoked stream received state: {text}");
        }
    })
    .await;
    assert!(closed1.is_ok(), "revoked SSE stream stayed open");

    // Session 2 is unaffected and receives new state events.
    s.h.wf(TICKET, "send", json!({"kind": "gate", "gate": "merge", "body": GATE})).await;
    let next2 = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let chunk = sse2.chunk().await.unwrap().expect("active stream closed early");
            let text = String::from_utf8_lossy(&chunk);
            if text.contains("event: state") {
                break text.into_owned();
            }
        }
    })
    .await;
    assert!(next2.is_ok(), "active session 2 did not receive state event");

    // Expiring session 2 closes its open SSE stream and cuts off API access.
    assert_eq!(sess2.id, "s2");
    s.h.clock.advance_minutes(30 * 24 * 60 + 1);
    let closed2 = tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(chunk) = sse2.chunk().await.unwrap() {
            let text = String::from_utf8_lossy(&chunk);
            assert!(!text.contains("event: state"), "expired stream received state: {text}");
        }
    })
    .await;
    assert!(closed2.is_ok(), "expired SSE stream stayed open");
    let r = s
        .http
        .get(format!("{}/api/state", s.base))
        .header("Host", "board.acme.example:47732")
        .header("X-Forwarded-Proto", "https")
        .header("Cookie", &cookie2)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 401);
}

fn tailscale_config(allow_users: Option<Vec<&str>>, require_pairing: bool) -> Config {
    let mut remote = serde_json::Map::new();
    remote.insert("mode".into(), json!("tailscale"));
    remote.insert("origin".into(), json!("https://box.acme.ts.net:47732"));
    if let Some(users) = allow_users {
        remote.insert("allowUsers".into(), json!(users));
    }
    if require_pairing {
        remote.insert("requirePairing".into(), json!(true));
    }
    Config::from_value(json!({"board": {"remote": Value::Object(remote)}})).unwrap()
}

#[test]
fn board_remote_config_validates_tailscale_allow_users_and_require_pairing() {
    let cfg = tailscale_config(Some(vec!["alice@acme.example", "bob@acme.example"]), true);
    let (remote, origin) = cfg.board.active_remote_config().unwrap();
    assert_eq!(remote.allow_users, vec!["alice@acme.example", "bob@acme.example"]);
    assert!(remote.require_pairing);
    assert!(remote.pairing_enabled());
    assert_eq!(origin.canonical(), "https://box.acme.ts.net:47732");

    let no_pair = tailscale_config(None, false);
    let (remote, _) = no_pair.board.active_remote_config().unwrap();
    assert!(remote.allow_users.is_empty());
    assert!(!remote.require_pairing);
    assert!(!remote.pairing_enabled());

    for bad in [
        json!({"board": {"remote": {"mode": "tailscale", "origin": "https://box.acme.ts.net:47732", "allowUsers": [""]}}}),
        json!({"board": {"remote": {"mode": "tailscale", "origin": "https://box.acme.ts.net:47732", "allowUsers": [" alice@acme.example"]}}}),
        json!({"board": {"remote": {"mode": "tailscale", "origin": "https://box.acme.ts.net:47732", "allowUsers": ["alice bob"]}}}),
        json!({"board": {"remote": {"mode": "tailscale", "origin": "https://box.acme.ts.net:47732", "allowUsers": ["alice\n@acme.example"]}}}),
    ] {
        assert!(Config::from_value(bad.clone()).is_err(), "expected invalid config: {bad}");
    }
}

#[tokio::test]
async fn tailscale_mode_authenticates_host_login_and_allow_users() {
    // 1. Default allowUsers: admits the host's own Tailscale login (case-insensitive), refuses others.
    let h = H::build(Vec::new(), tailscale_config(None, false));
    h.tailscale.set_host_login("alice@acme.example");
    let s = start(h).await;
    join_board(&s.h, TICKET, "widgets#41").await;
    let sent = s.h.wf(TICKET, "send", json!({"kind": "gate", "gate": "merge", "body": GATE})).await;
    let handle = sent["handle"].as_str().unwrap().to_string();
    let csrf = s.csrf();

    // Host login gets into every board read route and mutating route (with CSRF).
    for route in ["/", "/api/state", &format!("/api/msg/{handle}"), "/api/contexts", "/api/new-issues", "/api/flow"] {
        let r = s
            .http
            .get(format!("{}{route}", s.base))
            .header("Host", "box.acme.ts.net:47732")
            .header("X-Forwarded-Proto", "https")
            .header("Tailscale-User-Login", "ALICE@ACME.EXAMPLE")
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 200, "host login GET {route}");
    }
    let mut sse = s
        .http
        .get(format!("{}/api/events", s.base))
        .header("Host", "box.acme.ts.net:47732")
        .header("X-Forwarded-Proto", "https")
        .header("Tailscale-User-Login", "alice@acme.example")
        .send()
        .await
        .unwrap();
    assert_eq!(sse.status(), 200);
    let first = tokio::time::timeout(Duration::from_secs(5), sse.chunk()).await.unwrap().unwrap().unwrap();
    assert!(String::from_utf8_lossy(&first).contains("event: state"));
    drop(sse);

    // Mutating route still requires CSRF even for an allowed Tailscale user.
    let r = s
        .http
        .post(format!("{}/api/cards/a1/answer", s.base))
        .header("Host", "box.acme.ts.net:47732")
        .header("Origin", "https://box.acme.ts.net:47732")
        .header("X-Forwarded-Proto", "https")
        .header("Tailscale-User-Login", "alice@acme.example")
        .json(&json!({"text": "merge"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 403);

    let r = s
        .http
        .post(format!("{}/api/cards/a1/answer", s.base))
        .header("Host", "box.acme.ts.net:47732")
        .header("Origin", "https://box.acme.ts.net:47732")
        .header("X-Forwarded-Proto", "https")
        .header("Tailscale-User-Login", "alice@acme.example")
        .header("X-T3CM-CSRF", &csrf)
        .json(&json!({"text": "merge"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);

    // Another Tailscale login is refused with 403; missing/empty/duplicate login is refused with 401.
    let r = s
        .http
        .get(format!("{}/api/state", s.base))
        .header("Host", "box.acme.ts.net:47732")
        .header("X-Forwarded-Proto", "https")
        .header("Tailscale-User-Login", "bob@acme.example")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 403);

    let r = s
        .http
        .get(format!("{}/api/state", s.base))
        .header("Host", "box.acme.ts.net:47732")
        .header("X-Forwarded-Proto", "https")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 401);

    let r = s
        .http
        .get(format!("{}/api/state", s.base))
        .header("Host", "box.acme.ts.net:47732")
        .header("X-Forwarded-Proto", "https")
        .header("Tailscale-User-Login", "alice@acme.example")
        .header("Tailscale-User-Login", "bob@acme.example")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 401);

    // /pair is disabled when requirePairing is false, and /mcp + /healthz remain loopback-only.
    let r = s
        .http
        .get(format!("{}/pair", s.base))
        .header("Host", "box.acme.ts.net:47732")
        .header("X-Forwarded-Proto", "https")
        .header("Tailscale-User-Login", "alice@acme.example")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 403);

    let r = s
        .http
        .post(format!("{}/mcp", s.base))
        .header("Host", "box.acme.ts.net:47732")
        .header("X-Forwarded-Proto", "https")
        .header("Tailscale-User-Login", "alice@acme.example")
        .json(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 403);

    let r = s
        .http
        .get(format!("{}/healthz", s.base))
        .header("Host", "box.acme.ts.net:47732")
        .header("X-Forwarded-Proto", "https")
        .header("Tailscale-User-Login", "alice@acme.example")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 403);

    // 2. Explicit allowUsers overrides host login.
    let h2 = H::build(Vec::new(), tailscale_config(Some(vec!["bob@acme.example"]), false));
    h2.tailscale.set_host_login("alice@acme.example");
    let s2 = start(h2).await;

    let r = s2
        .http
        .get(format!("{}/api/state", s2.base))
        .header("Host", "box.acme.ts.net:47732")
        .header("X-Forwarded-Proto", "https")
        .header("Tailscale-User-Login", "bob@acme.example")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);

    let r = s2
        .http
        .get(format!("{}/api/state", s2.base))
        .header("Host", "box.acme.ts.net:47732")
        .header("X-Forwarded-Proto", "https")
        .header("Tailscale-User-Login", "alice@acme.example")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 403);
}

#[tokio::test]
async fn tailscale_header_grants_nothing_when_tailscale_mode_is_off() {
    // 1. Mode "off": Tailscale-User-Login is refused with 403 even when matching host login.
    let h_off = H::new();
    h_off.tailscale.set_host_login("alice@acme.example");
    let s_off = start(h_off).await;
    let r = s_off
        .http
        .get(format!("{}/api/state", s_off.base))
        .header("Tailscale-User-Login", "alice@acme.example")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 403);

    // 2. Mode "pairing": Tailscale-User-Login does not bypass pairing cookie auth (returns 401).
    let h_pair = H::build(Vec::new(), pairing_config());
    h_pair.tailscale.set_host_login("alice@acme.example");
    let s_pair = start(h_pair).await;
    let r = s_pair
        .http
        .get(format!("{}/api/state", s_pair.base))
        .header("Host", "board.acme.example:47732")
        .header("X-Forwarded-Proto", "https")
        .header("Tailscale-User-Login", "alice@acme.example")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 401);
}

#[tokio::test]
async fn tailscale_mode_with_require_pairing_layers_cookie_auth() {
    use t3_codemode::clock::{Clock, iso};
    use t3_codemode::config::RemoteOrigin;
    use t3_codemode::pairing;

    let h = H::build(Vec::new(), tailscale_config(None, true));
    h.tailscale.set_host_login("alice@acme.example");
    let s = start(h).await;
    let origin = RemoteOrigin::parse("https://box.acme.ts.net:47732").unwrap();

    // 1. Allowed Tailscale user without pairing cookie gets 401 on board routes.
    let r = s
        .http
        .get(format!("{}/api/state", s.base))
        .header("Host", "box.acme.ts.net:47732")
        .header("X-Forwarded-Proto", "https")
        .header("Tailscale-User-Login", "alice@acme.example")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 401);

    // 2. Disallowed Tailscale user is refused on GET /pair and POST /api/pair before consuming tokens.
    let minted = pairing::mint_pairing_token(&s.h.app.store, &origin, Some("Phone"), s.h.clock.now()).unwrap();
    let r = s
        .http
        .get(format!("{}/pair", s.base))
        .header("Host", "box.acme.ts.net:47732")
        .header("X-Forwarded-Proto", "https")
        .header("Tailscale-User-Login", "bob@acme.example")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 403);

    let r = s
        .http
        .post(format!("{}/api/pair", s.base))
        .header("Host", "box.acme.ts.net:47732")
        .header("Origin", "https://box.acme.ts.net:47732")
        .header("X-Forwarded-Proto", "https")
        .header("Tailscale-User-Login", "bob@acme.example")
        .json(&json!({"token": minted.token}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 403);

    // Direct loopback also cannot consume the pairing token.
    let r = s.http.post(format!("{}/api/pair", s.base)).json(&json!({"token": minted.token})).send().await.unwrap();
    assert_eq!(r.status(), 403);

    // 3. Allowed Tailscale user can load /pair and redeem the untouched token for a session cookie.
    let r = s
        .http
        .get(format!("{}/pair", s.base))
        .header("Host", "box.acme.ts.net:47732")
        .header("X-Forwarded-Proto", "https")
        .header("Tailscale-User-Login", "alice@acme.example")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);

    let r = s
        .http
        .post(format!("{}/api/pair", s.base))
        .header("Host", "box.acme.ts.net:47732")
        .header("Origin", "https://box.acme.ts.net:47732")
        .header("X-Forwarded-Proto", "https")
        .header("Tailscale-User-Login", "alice@acme.example")
        .json(&json!({"token": minted.token}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let set_cookie = r.headers().get("set-cookie").unwrap().to_str().unwrap().to_string();
    let cookie = cookie_pair_from_set_cookie(&set_cookie);

    // 4. Both allowed Tailscale login and valid cookie -> 200; wrong login even with valid cookie -> 403.
    let r = s
        .http
        .get(format!("{}/api/state", s.base))
        .header("Host", "box.acme.ts.net:47732")
        .header("X-Forwarded-Proto", "https")
        .header("Tailscale-User-Login", "alice@acme.example")
        .header("Cookie", &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);

    let r = s
        .http
        .get(format!("{}/api/state", s.base))
        .header("Host", "box.acme.ts.net:47732")
        .header("X-Forwarded-Proto", "https")
        .header("Tailscale-User-Login", "bob@acme.example")
        .header("Cookie", &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 403);

    // 5. Revoking the session cuts off access (401) even with valid Tailscale-User-Login.
    assert!(s.h.app.store.board_session_revoke(1, &iso(s.h.clock.now())).unwrap());
    let r = s
        .http
        .get(format!("{}/api/state", s.base))
        .header("Host", "box.acme.ts.net:47732")
        .header("X-Forwarded-Proto", "https")
        .header("Tailscale-User-Login", "alice@acme.example")
        .header("Cookie", &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 401);
}

#[tokio::test]
async fn healthz_reports_remote_board_state_and_stays_loopback_only() {
    use t3_codemode::clock::Clock;
    use t3_codemode::config::RemoteOrigin;
    use t3_codemode::pairing;

    // 1. Default off mode.
    let s_off = start(H::new()).await;
    let v: Value = s_off.http.get(format!("{}/healthz", s_off.base)).send().await.unwrap().json().await.unwrap();
    assert_eq!(v["remote"]["mode"], "off");
    assert_eq!(v["remote"]["enabled"], false);
    assert_eq!(v["remote"]["origin"], Value::Null);
    assert_eq!(v["remote"]["activeSessions"], 0);

    // 2. Tailscale mode with host login and an active pairing session.
    let h = H::build(Vec::new(), tailscale_config(None, true));
    h.tailscale.set_host_login("alice@acme.example");
    let origin = RemoteOrigin::parse("https://box.acme.ts.net:47732").unwrap();
    let minted = pairing::mint_pairing_token(&h.app.store, &origin, Some("Phone"), h.clock.now()).unwrap();
    assert!(pairing::redeem_pairing_token(&h.app.store, &minted.token, None, h.clock.now()).unwrap().is_some());
    let s = start(h).await;

    let v: Value = s.http.get(format!("{}/healthz", s.base)).send().await.unwrap().json().await.unwrap();
    assert_eq!(v["remote"]["mode"], "tailscale");
    assert_eq!(v["remote"]["enabled"], true);
    assert_eq!(v["remote"]["origin"], "https://box.acme.ts.net:47732");
    assert_eq!(v["remote"]["requirePairing"], true);
    assert_eq!(v["remote"]["allowUsers"], json!(["alice@acme.example"]));
    assert_eq!(v["remote"]["allowUsersSource"], "tailscale");
    assert_eq!(v["remote"]["hostLogin"], "alice@acme.example");
    assert_eq!(v["remote"]["tailscaleError"], Value::Null);
    assert_eq!(v["remote"]["activeSessions"], 1);

    // Loopback isolation still holds on /healthz.
    let r = s
        .http
        .get(format!("{}/healthz", s.base))
        .header("Host", "box.acme.ts.net:47732")
        .header("Tailscale-User-Login", "alice@acme.example")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 403);
}

#[test]
fn doctor_reports_tailscale_serve_cert_origin_and_config_drift() {
    use t3_codemode::doctor;
    use t3_codemode::fake::FakeTailscale;
    use t3_codemode::tailscale::{ServeStatus, TailscaleStatus, parse_serve_status_json};

    let clean_serve = || -> ServeStatus {
        parse_serve_status_json(
            &json!({
                "TCP": { "47732": { "HTTPS": true } },
                "Web": {
                    "box.acme.ts.net:47732": {
                        "Handlers": { "/": { "Proxy": "http://127.0.0.1:47731" } }
                    }
                }
            })
            .to_string(),
        )
        .unwrap()
    };

    let cfg = tailscale_config(None, false);
    let ts = FakeTailscale::default();
    ts.set_host_login("alice@acme.example");
    ts.set_serve_status(clean_serve());

    let matching_health = json!({
        "remote": {
            "mode": "tailscale",
            "origin": "https://box.acme.ts.net:47732",
            "requirePairing": false,
            "allowUsers": ["alice@acme.example"],
            "allowUsersSource": "tailscale"
        }
    });

    // 1. Clean state: zero problems, zero warnings.
    let f = doctor::remote_board(Some(&cfg), 47731, Some(&matching_health), &ts);
    assert!(f.problems.is_empty(), "{:?}", f.problems);
    assert!(f.warnings.is_empty(), "{:?}", f.warnings);

    // 2. Service vs config drift (running service still off, or older binary without `remote`).
    let stale_health = json!({"remote": {"mode": "off", "origin": null, "requirePairing": false}});
    let f = doctor::remote_board(Some(&cfg), 47731, Some(&stale_health), &ts);
    assert!(f.problems.iter().any(|p| p.contains("restart t3cm")), "{:?}", f.problems);

    let legacy_health = json!({"version": "0.6.0", "t3": {"connected": true}});
    let f = doctor::remote_board(Some(&cfg), 47731, Some(&legacy_health), &ts);
    assert!(
        f.problems.iter().any(|p| p.contains("does not report remote status") && p.contains("restart t3cm")),
        "{:?}",
        f.problems
    );

    // 3. Missing CLI.
    ts.set_missing();
    let f = doctor::remote_board(Some(&cfg), 47731, None, &ts);
    assert!(f.problems.iter().any(|p| p.contains("tailscale CLI was not found")), "{:?}", f.problems);

    // 4. Tailscale stopped, wrong DNSName, missing cert domain, missing host login.
    ts.set_status(TailscaleStatus {
        backend_state: Some("Stopped".into()),
        dns_name: Some("other.acme.ts.net".into()),
        cert_domains: vec![],
        host_login: None,
        ipv4: None,
    });
    ts.set_serve_status(ServeStatus::default());
    let f = doctor::remote_board(Some(&cfg), 47731, None, &ts);
    assert!(f.problems.iter().any(|p| p.contains("not running")), "{:?}", f.problems);
    assert!(f.problems.iter().any(|p| p.contains("does not match")), "{:?}", f.problems);
    assert!(f.problems.iter().any(|p| p.contains("HTTPS certificates are not enabled")), "{:?}", f.problems);
    assert!(f.problems.iter().any(|p| p.contains("allowUsers is empty")), "{:?}", f.problems);
    assert!(f.problems.iter().any(|p| p.contains("not serving https://box.acme.ts.net:47732")), "{:?}", f.problems);

    // 5. Wrong backend proxy port + Funnel enabled.
    ts.set_host_login("alice@acme.example");
    ts.set_serve_status(
        parse_serve_status_json(
            &json!({
                "TCP": { "47732": { "HTTPS": true } },
                "Web": {
                    "box.acme.ts.net:47732": {
                        "Handlers": { "/": { "Proxy": "http://127.0.0.1:9999" } }
                    }
                },
                "AllowFunnel": { "box.acme.ts.net:47732": true }
            })
            .to_string(),
        )
        .unwrap(),
    );
    let f = doctor::remote_board(Some(&cfg), 47731, None, &ts);
    assert!(f.problems.iter().any(|p| p.contains("http://127.0.0.1:9999")), "{:?}", f.problems);
    assert!(f.problems.iter().any(|p| p.contains("Tailscale Funnel is enabled")), "{:?}", f.problems);

    // 6. Config off while Tailscale Serve still exposes local port -> drift problem.
    let off_cfg = Config::default();
    ts.set_serve_status(clean_serve());
    let f = doctor::remote_board(Some(&off_cfg), 47731, None, &ts);
    assert!(f.problems.iter().any(|p| p.contains("board.remote is off")), "{:?}", f.problems);

    // 7. Config off and no Serve routes -> clean even when Tailscale is installed.
    ts.set_serve_status(ServeStatus::default());
    let f = doctor::remote_board(Some(&off_cfg), 47731, None, &ts);
    assert!(f.problems.is_empty(), "{:?}", f.problems);
    assert!(f.warnings.is_empty(), "{:?}", f.warnings);

    // 8. Mount overriding a board route (e.g. /api/state) is flagged while unrelated mount (/files) is allowed.
    ts.set_host_login("alice@acme.example");
    ts.set_serve_status(
        parse_serve_status_json(
            &json!({
                "TCP": { "47732": { "HTTPS": true } },
                "Web": {
                    "box.acme.ts.net:47732": {
                        "Handlers": {
                            "/": { "Proxy": "http://127.0.0.1:47731" },
                            "/files": { "Path": "/srv/files" }
                        }
                    }
                }
            })
            .to_string(),
        )
        .unwrap(),
    );
    let f = doctor::remote_board(Some(&cfg), 47731, None, &ts);
    assert!(f.problems.is_empty(), "unrelated /files mount should be allowed: {:?}", f.problems);

    ts.set_serve_status(
        parse_serve_status_json(
            &json!({
                "TCP": { "47732": { "HTTPS": true } },
                "Web": {
                    "box.acme.ts.net:47732": {
                        "Handlers": {
                            "/": { "Proxy": "http://127.0.0.1:47731" },
                            "/api/state": { "Text": "override" }
                        }
                    }
                }
            })
            .to_string(),
        )
        .unwrap(),
    );
    let f = doctor::remote_board(Some(&cfg), 47731, None, &ts);
    assert!(
        f.problems.iter().any(|p| p.contains("/api/state") && p.contains("intercepts board routes")),
        "{:?}",
        f.problems
    );

    // 9. Foreground-only raw TCP forwarder in `serve status --json` is detected by doctor.
    ts.set_serve_status(
        parse_serve_status_json(
            &json!({
                "Foreground": {
                    "sess-1": {
                        "TCP": { "9000": { "TCPForward": "127.0.0.1:47731" } }
                    }
                }
            })
            .to_string(),
        )
        .unwrap(),
    );
    let f = doctor::remote_board(Some(&off_cfg), 47731, None, &ts);
    assert!(f.problems.iter().any(|p| p.contains("raw TCP port 9000")), "{:?}", f.problems);

    // 10. Funnel on an additional board listener (8443) is flagged as a problem while a tailnet-only
    //     alternate listener stays a warning.
    ts.set_serve_status(
        parse_serve_status_json(
            &json!({
                "TCP": { "47732": { "HTTPS": true }, "8443": { "HTTPS": true } },
                "Web": {
                    "box.acme.ts.net:47732": { "Handlers": { "/": { "Proxy": "http://127.0.0.1:47731" } } },
                    "box.acme.ts.net:8443": { "Handlers": { "/": { "Proxy": "http://127.0.0.1:47731" } } }
                },
                "AllowFunnel": { "box.acme.ts.net:8443": true }
            })
            .to_string(),
        )
        .unwrap(),
    );
    let f = doctor::remote_board(Some(&cfg), 47731, None, &ts);
    assert!(
        f.problems.iter().any(|p| p.contains("Tailscale Funnel is enabled for box.acme.ts.net:8443")),
        "{:?}",
        f.problems
    );

    ts.set_serve_status(
        parse_serve_status_json(
            &json!({
                "TCP": { "47732": { "HTTPS": true }, "8443": { "HTTPS": true } },
                "Web": {
                    "box.acme.ts.net:47732": { "Handlers": { "/": { "Proxy": "http://127.0.0.1:47731" } } },
                    "box.acme.ts.net:8443": { "Handlers": { "/": { "Proxy": "http://127.0.0.1:47731" } } }
                }
            })
            .to_string(),
        )
        .unwrap(),
    );
    let f = doctor::remote_board(Some(&cfg), 47731, None, &ts);
    assert!(f.problems.is_empty(), "{:?}", f.problems);
    assert!(f.warnings.iter().any(|w| w.contains("box.acme.ts.net:8443")), "{:?}", f.warnings);
}

#[tokio::test]
async fn tailscale_sse_stream_closes_when_host_login_is_no_longer_authorized() {
    let h = H::build(Vec::new(), tailscale_config(None, false));
    h.tailscale.set_host_login("alice@acme.example");
    let s = start(h).await;
    join_board(&s.h, TICKET, "widgets#41").await;

    let mut sse = s
        .http
        .get(format!("{}/api/events", s.base))
        .header("Host", "box.acme.ts.net:47732")
        .header("X-Forwarded-Proto", "https")
        .header("Tailscale-User-Login", "alice@acme.example")
        .send()
        .await
        .unwrap();
    assert_eq!(sse.status(), 200);
    let first = tokio::time::timeout(Duration::from_secs(5), sse.chunk()).await.unwrap().unwrap().unwrap();
    assert!(String::from_utf8_lossy(&first).contains("event: state"));

    // Host login changes to another user: open SSE stream closes cleanly without delivering new state.
    s.h.tailscale.set_host_login("bob@acme.example");
    s.h.wf(TICKET, "send", json!({"kind": "gate", "gate": "merge", "body": GATE})).await;
    let closed = tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(chunk) = sse.chunk().await.unwrap() {
            let text = String::from_utf8_lossy(&chunk);
            assert!(!text.contains("event: state"), "unauthorized tailscale stream received state: {text}");
        }
    })
    .await;
    assert!(closed.is_ok(), "tailscale SSE stream stayed open after host login changed");
}

#[test]
fn expose_and_unexpose_drive_tailscale_serve_and_config() {
    use t3_codemode::fake::FakeTailscale;
    use t3_codemode::tailscale::{
        DEFAULT_HTTPS_PORT, ExposeOptions, expose_tailscale, parse_serve_status_json, unexpose_board,
    };

    let dir = tempfile::tempdir().unwrap();
    let cfg_path = dir.path().join("config.json");
    std::fs::write(&cfg_path, json!({"board": {"settleAfterHours": 48}}).to_string()).unwrap();

    let ts = FakeTailscale::default();
    ts.set_host_login("alice@acme.example");

    // 1. Refuse to overwrite a port already serving a different target (e.g. T3 Code on :443).
    ts.set_serve_status(
        parse_serve_status_json(
            &json!({
                "TCP": { "443": { "HTTPS": true } },
                "Web": { "box.acme.ts.net:443": { "Handlers": { "/": { "Proxy": "http://127.0.0.1:3773" } } } }
            })
            .to_string(),
        )
        .unwrap(),
    );
    let err = expose_tailscale(
        &ts,
        &cfg_path,
        &ExposeOptions { https_port: 443, local_port: 47731, allow_users: vec![], require_pairing: false },
    )
    .unwrap_err();
    assert!(format!("{err:#}").contains("already in use"), "{err:#}");

    // 2. Expose on default 47732 preserves existing config keys and calls `tailscale serve`.
    let report = expose_tailscale(
        &ts,
        &cfg_path,
        &ExposeOptions {
            https_port: DEFAULT_HTTPS_PORT,
            local_port: 47731,
            allow_users: vec!["bob@acme.example".into()],
            require_pairing: true,
        },
    )
    .unwrap();
    assert_eq!(report.origin, "https://box.acme.ts.net:47732");
    assert_eq!(report.host_login.as_deref(), Some("alice@acme.example"));
    assert_eq!(report.allow_users, vec!["bob@acme.example"]);
    assert!(report.require_pairing);
    assert!(ts.calls().iter().any(|c| c == "serve --bg --https=47732 http://127.0.0.1:47731"));

    let loaded = Config::load(&cfg_path).unwrap();
    assert_eq!(loaded.board.settle_after_hours, 48);
    let (remote, origin) = loaded.board.active_remote_config().unwrap();
    assert_eq!(remote.mode, t3_codemode::config::RemoteMode::Tailscale);
    assert_eq!(origin.canonical(), "https://box.acme.ts.net:47732");
    assert_eq!(remote.allow_users, vec!["bob@acme.example"]);
    assert!(remote.require_pairing);

    // 3. Unexpose only removes mounts targeting the board using exact --set-path, preserving unrelated mounts.
    ts.set_serve_status(
        parse_serve_status_json(
            &json!({
                "TCP": { "47732": { "HTTPS": true } },
                "Web": {
                    "box.acme.ts.net:47732": {
                        "Handlers": {
                            "/": { "Proxy": "http://127.0.0.1:47731" },
                            "/other": { "Proxy": "http://127.0.0.1:9000" }
                        }
                    }
                }
            })
            .to_string(),
        )
        .unwrap(),
    );
    let un = unexpose_board(&ts, &cfg_path, None, 47731).unwrap();
    assert_eq!(un.previous_mode, "tailscale");
    assert_eq!(un.disabled_tailscale_ports, vec![47732]);
    assert!(un.config_updated);
    assert!(ts.calls().iter().any(|c| c == "serve --https=47732 --set-path=/ off"));
    assert!(!ts.calls().iter().any(|c| c.contains("/other")));

    let loaded_after = Config::load(&cfg_path).unwrap();
    assert_eq!(loaded_after.board.settle_after_hours, 48);
    assert!(loaded_after.board.active_remote().is_none());

    // 4. Unexpose succeeds and turns off config even when the Serve handler was already removed
    //    (without calling `off` on an absent route).
    std::fs::write(
        &cfg_path,
        json!({"board": {"remote": {"mode": "tailscale", "origin": "https://box.acme.ts.net:47732"}}}).to_string(),
    )
    .unwrap();
    ts.set_serve_status(t3_codemode::tailscale::ServeStatus::default());
    ts.set_disable_error(Some("error: handler does not exist"));
    let un_absent = unexpose_board(&ts, &cfg_path, None, 47731).unwrap();
    assert_eq!(un_absent.previous_mode, "tailscale");
    assert!(un_absent.disabled_tailscale_ports.is_empty());
    assert!(un_absent.config_updated);
    assert!(Config::load(&cfg_path).unwrap().board.active_remote().is_none());

    // 5. Explicit --https-port refuses to remove a port serving an unrelated target.
    ts.set_serve_status(
        parse_serve_status_json(
            &json!({
                "TCP": { "443": { "HTTPS": true } },
                "Web": { "box.acme.ts.net:443": { "Handlers": { "/": { "Proxy": "http://127.0.0.1:3773" } } } }
            })
            .to_string(),
        )
        .unwrap(),
    );
    let err = unexpose_board(&ts, &cfg_path, Some(443), 47731).unwrap_err();
    assert!(format!("{err:#}").contains("unrelated handler"), "{err:#}");

    // 6. Renamed-node protection: a stale route under an old hostname never triggers `off`
    //    against an unrelated route at the same port/path on the current hostname.
    std::fs::write(
        &cfg_path,
        json!({"board": {"remote": {"mode": "tailscale", "origin": "https://box.acme.ts.net:47732"}}}).to_string(),
    )
    .unwrap();
    ts.set_disable_error(None);
    ts.set_serve_status(
        parse_serve_status_json(
            &json!({
                "TCP": { "47732": { "HTTPS": true } },
                "Web": {
                    "old-box.acme.ts.net:47732": { "Handlers": { "/": { "Proxy": "http://127.0.0.1:47731" } } },
                    "box.acme.ts.net:47732": { "Handlers": { "/": { "Proxy": "http://127.0.0.1:9000" } } }
                }
            })
            .to_string(),
        )
        .unwrap(),
    );
    let calls_before_rename = ts.calls().len();
    let un_renamed = unexpose_board(&ts, &cfg_path, None, 47731).unwrap();
    assert!(un_renamed.disabled_tailscale_ports.is_empty());
    assert!(!ts.calls()[calls_before_rename..].iter().any(|c| c.ends_with(" off")));

    // 7. In Tailscale mode, a missing CLI or a --https-port override that leaves the configured route live
    //    returns an error and preserves board.remote in config.json.
    std::fs::write(
        &cfg_path,
        json!({"board": {"remote": {"mode": "tailscale", "origin": "https://box.acme.ts.net:47732"}}}).to_string(),
    )
    .unwrap();
    ts.set_serve_status(
        parse_serve_status_json(
            &json!({
                "TCP": { "47732": { "HTTPS": true } },
                "Web": { "box.acme.ts.net:47732": { "Handlers": { "/": { "Proxy": "http://127.0.0.1:47731" } } } }
            })
            .to_string(),
        )
        .unwrap(),
    );
    let err = unexpose_board(&ts, &cfg_path, Some(48000), 47731).unwrap_err();
    assert!(format!("{err:#}").contains("port 47732"), "{err:#}");
    assert!(Config::load(&cfg_path).unwrap().board.active_remote().is_some(), "config must stay intact on error");

    ts.set_missing();
    let err = unexpose_board(&ts, &cfg_path, None, 47731).unwrap_err();
    assert!(format!("{err:#}").contains("not found"), "{err:#}");
    assert!(
        Config::load(&cfg_path).unwrap().board.active_remote().is_some(),
        "config must stay intact when CLI missing"
    );

    // 8. If the configured port drifted to HTTP-only while still proxying to the board, unexpose fails
    //    and preserves config.json both with default arguments and with a different --https-port.
    ts.set_host_login("alice@acme.example");
    ts.set_serve_status(
        parse_serve_status_json(
            &json!({
                "TCP": { "47732": { "HTTP": true } },
                "Web": { "box.acme.ts.net:47732": { "Handlers": { "/": { "Proxy": "http://127.0.0.1:47731" } } } }
            })
            .to_string(),
        )
        .unwrap(),
    );
    let err = unexpose_board(&ts, &cfg_path, None, 47731).unwrap_err();
    assert!(format!("{err:#}").contains("plain HTTP"), "{err:#}");
    assert!(format!("{err:#}").contains("tailscale serve --http=47732 --set-path=/ off"), "{err:#}");
    assert!(Config::load(&cfg_path).unwrap().board.active_remote().is_some());
    let err = unexpose_board(&ts, &cfg_path, Some(48000), 47731).unwrap_err();
    assert!(format!("{err:#}").contains("plain HTTP"), "{err:#}");
    assert!(format!("{err:#}").contains("tailscale serve --http=47732 --set-path=/ off"), "{err:#}");
    assert!(Config::load(&cfg_path).unwrap().board.active_remote().is_some());

    // A mount path containing spaces is shell-quoted in the suggested removal command.
    ts.set_serve_status(
        parse_serve_status_json(
            &json!({
                "TCP": { "47732": { "HTTP": true } },
                "Web": { "box.acme.ts.net:47732": { "Handlers": { "/team board": { "Proxy": "http://127.0.0.1:47731" } } } }
            })
            .to_string(),
        )
        .unwrap(),
    );
    let err = unexpose_board(&ts, &cfg_path, None, 47731).unwrap_err();
    assert!(format!("{err:#}").contains("tailscale serve --http=47732 --set-path='/team board' off"), "{err:#}");
    assert!(Config::load(&cfg_path).unwrap().board.active_remote().is_some());

    // 9. Re-exposing over an existing working board route does not roll back that route if config write fails.
    let bad_cfg_dir = dir.path().join("bad-cfg-dir");
    std::fs::create_dir_all(&bad_cfg_dir).unwrap();
    ts.set_host_login("alice@acme.example");
    ts.set_disable_error(None);
    ts.set_serve_status(
        parse_serve_status_json(
            &json!({
                "TCP": { "47732": { "HTTPS": true } },
                "Web": { "box.acme.ts.net:47732": { "Handlers": { "/": { "Proxy": "http://127.0.0.1:47731" } } } }
            })
            .to_string(),
        )
        .unwrap(),
    );
    let calls_before = ts.calls().len();
    assert!(
        expose_tailscale(
            &ts,
            &bad_cfg_dir,
            &ExposeOptions {
                https_port: DEFAULT_HTTPS_PORT,
                local_port: 47731,
                allow_users: vec![],
                require_pairing: false
            },
        )
        .is_err()
    );
    let calls_after = ts.calls();
    assert!(
        !calls_after[calls_before..].iter().any(|c| c.ends_with(" off")),
        "must not disable pre-existing working route: {:?}",
        &calls_after[calls_before..]
    );
}

#[test]
fn expose_reuses_an_existing_board_root_route_without_a_serve_write() {
    use t3_codemode::fake::FakeTailscale;
    use t3_codemode::tailscale::{DEFAULT_HTTPS_PORT, ExposeOptions, expose_tailscale, parse_serve_status_json};

    let dir = tempfile::tempdir().unwrap();
    let cfg_path = dir.path().join("config.json");
    std::fs::write(&cfg_path, json!({"board": {"settleAfterHours": 48}}).to_string()).unwrap();

    let ts = FakeTailscale::default();
    ts.set_host_login("alice@acme.example");
    ts.set_serve_status(
        parse_serve_status_json(
            &json!({
                "TCP": { "8443": { "HTTPS": true }, "47732": { "HTTPS": true } },
                "Web": {
                    "box.acme.ts.net:8443": { "Handlers": { "/": { "Path": "/srv/widgets" } } },
                    "box.acme.ts.net:47732": { "Handlers": { "/": { "Proxy": "http://127.0.0.1:47731" } } }
                }
            })
            .to_string(),
        )
        .unwrap(),
    );
    ts.set_enable_error(Some(
        "`tailscale serve --bg --https=47732 http://127.0.0.1:47731` failed: 401 Unauthorized: must be root, \
         or be an operator and able to run 'sudo tailscale' to serve a path or Unix socket",
    ));

    let report = expose_tailscale(
        &ts,
        &cfg_path,
        &ExposeOptions {
            https_port: DEFAULT_HTTPS_PORT,
            local_port: 47731,
            allow_users: vec![],
            require_pairing: false,
        },
    )
    .unwrap();
    assert_eq!(report.origin, "https://box.acme.ts.net:47732");
    assert!(!ts.calls().iter().any(|c| c.starts_with("serve --")), "no serve write expected: {:?}", ts.calls());

    let loaded = Config::load(&cfg_path).unwrap();
    assert_eq!(loaded.board.settle_after_hours, 48);
    let (remote, origin) = loaded.board.active_remote_config().unwrap();
    assert_eq!(remote.mode, t3_codemode::config::RemoteMode::Tailscale);
    assert_eq!(origin.canonical(), "https://box.acme.ts.net:47732");
}

fn plain_http_squatter() -> u16 {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for mut conn in listener.incoming().flatten() {
            let mut buf = [0u8; 2048];
            let _ = conn.read(&mut buf);
            let _ = conn.write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        }
    });
    port
}

#[test]
fn expose_fails_when_a_plain_http_listener_holds_the_serve_https_port() {
    use t3_codemode::fake::FakeTailscale;
    use t3_codemode::tailscale::{ExposeOptions, ListenerOwner, expose_tailscale};

    let dir = tempfile::tempdir().unwrap();
    let cfg_path = dir.path().join("config.json");
    std::fs::write(&cfg_path, json!({"board": {"settleAfterHours": 48}}).to_string()).unwrap();
    let port = plain_http_squatter();

    let ts = FakeTailscale::default();
    ts.set_host_login("alice@acme.example");
    ts.set_ipv4(Some(std::net::Ipv4Addr::LOCALHOST));
    let opts = ExposeOptions { https_port: port, local_port: 47731, allow_users: vec![], require_pairing: false };

    let err = format!("{:#}", expose_tailscale(&ts, &cfg_path, &opts).unwrap_err());
    assert!(err.contains(&format!("127.0.0.1:{port} answers")), "{err}");
    assert!(err.contains("something other than TLS"), "{err}");
    assert!(err.contains("stop the other process"), "{err}");
    assert!(err.contains("rerun `t3cm board expose --tailscale`"), "{err}");
    assert_eq!(
        ts.calls().iter().filter(|c| c.starts_with("serve --")).collect::<Vec<_>>(),
        vec![
            &format!("serve --bg --https={port} http://127.0.0.1:47731"),
            &format!("serve --https={port} --set-path=/ off")
        ]
    );
    assert!(Config::load(&cfg_path).unwrap().board.active_remote_config().is_none());

    ts.set_listener_owner(Some(ListenerOwner { pid: 4242, name: "widget-proxy".into() }));
    let err = format!("{:#}", expose_tailscale(&ts, &cfg_path, &opts).unwrap_err());
    assert!(err.contains("belongs to widget-proxy (pid 4242), not tailscaled"), "{err}");
    assert!(err.contains("rerun `t3cm board expose --tailscale`"), "{err}");
}

#[test]
fn expose_reapplies_an_existing_board_route_when_its_ipv4_listener_is_not_tailscaled() {
    use t3_codemode::fake::FakeTailscale;
    use t3_codemode::tailscale::{ExposeOptions, expose_tailscale, parse_serve_status_json};

    let dir = tempfile::tempdir().unwrap();
    let cfg_path = dir.path().join("config.json");
    std::fs::write(&cfg_path, json!({"board": {"settleAfterHours": 48}}).to_string()).unwrap();
    let port = plain_http_squatter();

    let ts = FakeTailscale::default();
    ts.set_host_login("alice@acme.example");
    ts.set_ipv4(Some(std::net::Ipv4Addr::LOCALHOST));
    ts.set_serve_status(
        parse_serve_status_json(
            &json!({
                "TCP": { port.to_string(): { "HTTPS": true } },
                "Web": {
                    format!("box.acme.ts.net:{port}"): { "Handlers": { "/": { "Proxy": "http://127.0.0.1:47731" } } }
                }
            })
            .to_string(),
        )
        .unwrap(),
    );
    let opts = ExposeOptions { https_port: port, local_port: 47731, allow_users: vec![], require_pairing: false };

    let err = format!("{:#}", expose_tailscale(&ts, &cfg_path, &opts).unwrap_err());
    assert!(err.contains("something other than TLS"), "{err}");
    assert_eq!(
        ts.calls().iter().filter(|c| c.starts_with("serve --")).collect::<Vec<_>>(),
        vec![&format!("serve --bg --https={port} http://127.0.0.1:47731")],
        "the existing route is re-applied and never removed"
    );
    assert!(Config::load(&cfg_path).unwrap().board.active_remote_config().is_none());
}

#[test]
fn doctor_reports_a_plain_http_listener_on_the_serve_https_port() {
    use t3_codemode::doctor;
    use t3_codemode::fake::FakeTailscale;
    use t3_codemode::tailscale::parse_serve_status_json;

    let port = plain_http_squatter();
    let cfg = Config::from_value(json!({
        "board": {"remote": {"mode": "tailscale", "origin": format!("https://box.acme.ts.net:{port}")}}
    }))
    .unwrap();
    let ts = FakeTailscale::default();
    ts.set_host_login("alice@acme.example");
    ts.set_serve_status(
        parse_serve_status_json(
            &json!({
                "TCP": { port.to_string(): { "HTTPS": true } },
                "Web": {
                    format!("box.acme.ts.net:{port}"): { "Handlers": { "/": { "Proxy": "http://127.0.0.1:47731" } } }
                }
            })
            .to_string(),
        )
        .unwrap(),
    );

    let f = doctor::remote_board(Some(&cfg), 47731, None, &ts);
    assert!(f.problems.is_empty(), "no IPv4 address, no probe: {:?}", f.problems);

    ts.set_ipv4(Some(std::net::Ipv4Addr::LOCALHOST));
    let f = doctor::remote_board(Some(&cfg), 47731, None, &ts);
    assert!(
        f.problems.iter().any(|p| p.starts_with("remote: ")
            && p.contains("something other than TLS")
            && p.contains("rerun `t3cm board expose --tailscale`")),
        "{:?}",
        f.problems
    );
}

#[test]
fn expose_explains_a_root_only_serve_handler_when_the_write_is_refused() {
    use t3_codemode::fake::FakeTailscale;
    use t3_codemode::tailscale::{DEFAULT_HTTPS_PORT, ExposeOptions, expose_tailscale, parse_serve_status_json};

    let dir = tempfile::tempdir().unwrap();
    let cfg_path = dir.path().join("config.json");
    std::fs::write(&cfg_path, json!({"board": {"settleAfterHours": 48}}).to_string()).unwrap();

    let ts = FakeTailscale::default();
    ts.set_host_login("alice@acme.example");
    ts.set_serve_status(
        parse_serve_status_json(
            &json!({
                "TCP": { "8443": { "HTTPS": true } },
                "Web": { "box.acme.ts.net:8443": { "Handlers": { "/": { "Path": "/srv/widgets" } } } }
            })
            .to_string(),
        )
        .unwrap(),
    );
    ts.set_enable_error(Some(
        "`tailscale serve --bg --https=47732 http://127.0.0.1:47731` failed: 401 Unauthorized: must be root, \
         or be an operator and able to run 'sudo tailscale' to serve a path or Unix socket",
    ));

    let err = expose_tailscale(
        &ts,
        &cfg_path,
        &ExposeOptions {
            https_port: DEFAULT_HTTPS_PORT,
            local_port: 47731,
            allow_users: vec![],
            require_pairing: false,
        },
    )
    .unwrap_err();
    let msg = format!("{err:#}");
    assert!(ts.calls().iter().any(|c| c == "serve --bg --https=47732 http://127.0.0.1:47731"));
    assert!(msg.contains("box.acme.ts.net:8443/ -> path /srv/widgets"), "{msg}");
    assert!(msg.contains("sudo tailscale serve --bg --https=47732 http://127.0.0.1:47731"), "{msg}");
    assert!(msg.contains("rerun the same `t3cm board expose --tailscale` command"), "{msg}");
    assert!(!msg.contains("set --operator"), "{msg}");
    assert!(Config::load(&cfg_path).unwrap().board.active_remote().is_none());
}

#[tokio::test]
async fn remote_open_endpoint_is_refused_server_side_in_both_pairing_and_tailscale_modes() {
    use t3_codemode::clock::Clock;
    use t3_codemode::config::RemoteOrigin;
    use t3_codemode::pairing;

    // 1. Pairing mode: authenticated remote POST /api/open is refused with 403, while direct loopback works.
    let s = start(H::build(Vec::new(), pairing_config())).await;
    s.h.desktop.running.store(true, std::sync::atomic::Ordering::SeqCst);
    let origin = RemoteOrigin::parse("https://board.acme.example:47732").unwrap();
    let minted = pairing::mint_pairing_token(&s.h.app.store, &origin, Some("Phone"), s.h.clock.now()).unwrap();
    let (raw_session, _) =
        pairing::redeem_pairing_token(&s.h.app.store, &minted.token, None, s.h.clock.now()).unwrap().unwrap();
    let cookie = format!("{}={raw_session}", pairing::SESSION_COOKIE);
    let csrf = s.csrf();

    let r = s
        .http
        .post(format!("{}/api/open", s.base))
        .header("Host", "board.acme.example:47732")
        .header("Origin", "https://board.acme.example:47732")
        .header("X-Forwarded-Proto", "https")
        .header("Cookie", &cookie)
        .header("X-T3CM-CSRF", &csrf)
        .json(&json!({"link": THREAD_LINK}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 403);
    assert!(s.h.desktop.opened.lock().unwrap().is_empty(), "remote /api/open must never open desktop app");

    // Direct loopback still opens the desktop app.
    let local = s.post("/api/open", json!({"link": THREAD_LINK}), Some(&csrf), Some(&s.base)).await;
    assert_eq!(local.status(), 200);
    assert_eq!(local.json::<Value>().await.unwrap(), json!({"opened": true}));
    assert_eq!(*s.h.desktop.opened.lock().unwrap(), [APP_LINK]);

    // 2. Tailscale mode: authenticated remote POST /api/open is refused with 403.
    let h = H::build(Vec::new(), tailscale_config(None, false));
    h.tailscale.set_host_login("alice@acme.example");
    h.desktop.running.store(true, std::sync::atomic::Ordering::SeqCst);
    let ts = start(h).await;
    let ts_csrf = ts.csrf();

    let r = ts
        .http
        .post(format!("{}/api/open", ts.base))
        .header("Host", "box.acme.ts.net:47732")
        .header("Origin", "https://box.acme.ts.net:47732")
        .header("X-Forwarded-Proto", "https")
        .header("Tailscale-User-Login", "alice@acme.example")
        .header("X-T3CM-CSRF", &ts_csrf)
        .json(&json!({"link": THREAD_LINK}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 403);
    assert!(ts.h.desktop.opened.lock().unwrap().is_empty());
}

#[tokio::test]
async fn remote_board_manifest_and_icons_respect_guards_and_csp() {
    use t3_codemode::clock::Clock;
    use t3_codemode::config::RemoteOrigin;
    use t3_codemode::pairing;

    let assets = ["/manifest.webmanifest", "/icon.svg", "/icon-192.png", "/icon-512.png"];

    // 1. Loopback serves manifest and icons within zero-external-request CSP, and remote mode off refuses them.
    let off = start(H::new()).await;
    let page = off.http.get(format!("{}/", off.base)).send().await.unwrap();
    assert_eq!(page.status(), 200);
    let csp = page.headers()["content-security-policy"].to_str().unwrap().to_string();
    assert!(csp.contains("default-src 'none'"), "{csp}");
    assert!(csp.contains("manifest-src 'self'"), "{csp}");
    assert!(csp.contains("img-src 'self' data:"), "{csp}");
    let html = page.text().await.unwrap();
    assert!(html.contains(r#"<link rel="manifest" href="/manifest.webmanifest" crossorigin="use-credentials">"#));
    assert!(html.contains(r#"<link rel="icon" href="/icon.svg" type="image/svg+xml">"#));
    assert!(html.contains(r#"<link rel="apple-touch-icon" href="/icon-192.png">"#));

    let mf = off.http.get(format!("{}/manifest.webmanifest", off.base)).send().await.unwrap();
    assert_eq!(mf.status(), 200);
    assert!(mf.headers()["content-type"].to_str().unwrap().starts_with("application/manifest+json"));
    assert_eq!(mf.headers()["cache-control"], "no-store");
    assert_eq!(mf.headers()["x-content-type-options"], "nosniff");
    assert_eq!(mf.headers()["content-security-policy"], "default-src 'none'; frame-ancestors 'none'");
    let mf_json: Value = mf.json().await.unwrap();
    assert_eq!(mf_json["name"], "t3 board");
    assert_eq!(mf_json["start_url"], "/");
    assert_eq!(mf_json["display"], "standalone");
    let icon_srcs: Vec<&str> =
        mf_json["icons"].as_array().unwrap().iter().map(|i| i["src"].as_str().unwrap()).collect();
    assert_eq!(icon_srcs, vec!["/icon.svg", "/icon-192.png", "/icon-512.png"]);

    let svg = off.http.get(format!("{}/icon.svg", off.base)).send().await.unwrap();
    assert_eq!(svg.status(), 200);
    assert!(svg.headers()["content-type"].to_str().unwrap().starts_with("image/svg+xml"));
    assert_eq!(svg.headers()["content-security-policy"], "default-src 'none'; frame-ancestors 'none'");
    let svg_body = svg.text().await.unwrap();
    let svg_no_ns = svg_body.replace(r#"xmlns="http://www.w3.org/2000/svg""#, "");
    assert!(
        svg_body.starts_with("<svg ")
            && !svg_no_ns.contains("http://")
            && !svg_no_ns.contains("https://")
            && !svg_body.to_lowercase().contains("<script")
    );

    for png_route in ["/icon-192.png", "/icon-512.png"] {
        let r = off.http.get(format!("{}{png_route}", off.base)).send().await.unwrap();
        assert_eq!(r.status(), 200, "{png_route}");
        assert_eq!(r.headers()["content-type"], "image/png");
        let bytes = r.bytes().await.unwrap();
        assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"), "{png_route} must be a valid PNG");
    }

    for route in assets {
        let r = off
            .http
            .get(format!("{}{route}", off.base))
            .header("Host", "board.acme.example:47732")
            .header("X-Forwarded-Proto", "https")
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 403, "remote off GET {route}");
    }

    // 2. Pairing mode requires a valid session cookie for manifest and icons.
    let sp = start(H::build(Vec::new(), pairing_config())).await;
    let origin = RemoteOrigin::parse("https://board.acme.example:47732").unwrap();
    let minted = pairing::mint_pairing_token(&sp.h.app.store, &origin, Some("Phone"), sp.h.clock.now()).unwrap();
    let (raw_session, _) =
        pairing::redeem_pairing_token(&sp.h.app.store, &minted.token, None, sp.h.clock.now()).unwrap().unwrap();
    let cookie = format!("{}={raw_session}", pairing::SESSION_COOKIE);
    for route in assets {
        let unauth = sp
            .http
            .get(format!("{}{route}", sp.base))
            .header("Host", "board.acme.example:47732")
            .header("X-Forwarded-Proto", "https")
            .send()
            .await
            .unwrap();
        assert_eq!(unauth.status(), 401, "pairing unauth GET {route}");

        let auth = sp
            .http
            .get(format!("{}{route}", sp.base))
            .header("Host", "board.acme.example:47732")
            .header("X-Forwarded-Proto", "https")
            .header("Cookie", &cookie)
            .send()
            .await
            .unwrap();
        assert_eq!(auth.status(), 200, "pairing auth GET {route}");
    }

    // 3. Tailscale mode enforces Tailscale-User-Login for manifest and icons.
    let ht = H::build(Vec::new(), tailscale_config(None, false));
    ht.tailscale.set_host_login("alice@acme.example");
    let st = start(ht).await;
    for route in assets {
        let missing = st
            .http
            .get(format!("{}{route}", st.base))
            .header("Host", "box.acme.ts.net:47732")
            .header("X-Forwarded-Proto", "https")
            .send()
            .await
            .unwrap();
        assert_eq!(missing.status(), 401, "tailscale missing GET {route}");

        let denied = st
            .http
            .get(format!("{}{route}", st.base))
            .header("Host", "box.acme.ts.net:47732")
            .header("X-Forwarded-Proto", "https")
            .header("Tailscale-User-Login", "bob@acme.example")
            .send()
            .await
            .unwrap();
        assert_eq!(denied.status(), 403, "tailscale denied GET {route}");

        let ok = st
            .http
            .get(format!("{}{route}", st.base))
            .header("Host", "box.acme.ts.net:47732")
            .header("X-Forwarded-Proto", "https")
            .header("Tailscale-User-Login", "alice@acme.example")
            .send()
            .await
            .unwrap();
        assert_eq!(ok.status(), 200, "tailscale allowed GET {route}");
    }
}

#[tokio::test]
async fn remote_html_navigation_without_auth_returns_signin_or_denied_screen() {
    // 1. Pairing mode: browser Accept: text/html gets 401 HTML sign-in screen with no CSRF leak.
    let sp = start(H::build(Vec::new(), pairing_config())).await;
    join_board(&sp.h, TICKET, "widgets#41").await;
    let csrf = sp.csrf();

    let missing_html = sp
        .http
        .get(format!("{}/", sp.base))
        .header("Host", "board.acme.example:47732")
        .header("X-Forwarded-Proto", "https")
        .header("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8")
        .send()
        .await
        .unwrap();
    assert_eq!(missing_html.status(), 401);
    assert!(missing_html.headers()["content-type"].to_str().unwrap().starts_with("text/html"));
    assert_eq!(missing_html.headers()["cache-control"], "no-store");
    assert!(missing_html.headers()["content-security-policy"].to_str().unwrap().contains("default-src 'none'"));
    let body = missing_html.text().await.unwrap();
    assert!(body.contains("t3cm board pair"), "{body}");
    assert!(body.contains(r#"href="/pair""#), "{body}");
    assert!(!body.contains(&csrf), "auth page must never leak CSRF");
    assert!(!body.contains("widgets#41"), "auth page must never leak board data");

    // Revoked/invalid cookie gets expired/revoked copy.
    let expired_html = sp
        .http
        .get(format!("{}/", sp.base))
        .header("Host", "board.acme.example:47732")
        .header("X-Forwarded-Proto", "https")
        .header("Accept", "text/html")
        .header("Cookie", format!("{}=stale-token", t3_codemode::pairing::SESSION_COOKIE))
        .send()
        .await
        .unwrap();
    assert_eq!(expired_html.status(), 401);
    let body = expired_html.text().await.unwrap();
    assert!(body.contains("Session expired or revoked"), "{body}");
    assert!(body.contains(r#"href="/pair""#), "{body}");
    assert!(!body.contains(&csrf));

    // Non-HTML Accept (or q=0) still returns JSON with structured auth metadata.
    let json_resp = sp
        .http
        .get(format!("{}/", sp.base))
        .header("Host", "board.acme.example:47732")
        .header("X-Forwarded-Proto", "https")
        .header("Accept", "text/html;q=0, application/json")
        .send()
        .await
        .unwrap();
    assert_eq!(json_resp.status(), 401);
    assert!(json_resp.headers()["content-type"].to_str().unwrap().starts_with("application/json"));
    let j: Value = json_resp.json().await.unwrap();
    assert_eq!(j["auth"]["kind"], "pairing");
    assert_eq!(j["auth"]["pairingUrl"], "/pair");

    // 2. Tailscale mode: disallowed login gets 403 HTML screen naming escaped caller login, never host login or /pair.
    let ht = H::build(Vec::new(), tailscale_config(None, false));
    ht.tailscale.set_host_login("alice@acme.example");
    let st = start(ht).await;
    let ts_csrf = st.csrf();

    let denied = st
        .http
        .get(format!("{}/", st.base))
        .header("Host", "box.acme.ts.net:47732")
        .header("X-Forwarded-Proto", "https")
        .header("Accept", "text/html")
        .header("Tailscale-User-Login", "bob<script>@acme.example")
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), 403);
    assert!(denied.headers()["content-type"].to_str().unwrap().starts_with("text/html"));
    let body = denied.text().await.unwrap();
    assert!(body.contains("Access denied"), "{body}");
    assert!(body.contains("bob&lt;script&gt;@acme.example"), "caller login must be HTML-escaped: {body}");
    assert!(!body.contains("<script>"), "{body}");
    assert!(!body.contains("alice@acme.example"), "must never leak host login: {body}");
    assert!(!body.contains(r#"href="/pair""#), "tailscale-only denial must not link /pair: {body}");
    assert!(
        body.contains("board.remote.allowUsers")
            && body.contains("host&#39;s own Tailscale login")
            && body.contains("restart t3cm"),
        "{body}"
    );
    assert!(!body.contains("t3cm board expose --tailscale --allow-user"), "{body}");
    assert!(!body.contains(&ts_csrf));

    // 3. Remote off stays 403 JSON even when Accept: text/html is sent.
    let off = start(H::new()).await;
    let r = off
        .http
        .get(format!("{}/", off.base))
        .header("Host", "board.acme.example:47732")
        .header("X-Forwarded-Proto", "https")
        .header("Accept", "text/html")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 403);
    assert!(r.headers()["content-type"].to_str().unwrap().starts_with("application/json"));
}

#[tokio::test]
async fn tailscale_live_auth_expiry_closes_sse_and_api_state_reports_tailscale_denied() {
    let h = H::build(Vec::new(), tailscale_config(None, false));
    h.tailscale.set_host_login("alice@acme.example");
    let s = start(h).await;
    join_board(&s.h, TICKET, "widgets#41").await;

    let mut sse = s
        .http
        .get(format!("{}/api/events", s.base))
        .header("Host", "box.acme.ts.net:47732")
        .header("X-Forwarded-Proto", "https")
        .header("Tailscale-User-Login", "alice@acme.example")
        .send()
        .await
        .unwrap();
    assert_eq!(sse.status(), 200);
    let first = tokio::time::timeout(Duration::from_secs(5), sse.chunk()).await.unwrap().unwrap().unwrap();
    assert!(String::from_utf8_lossy(&first).contains("event: state"));

    // Host login changes while alice's stream is open.
    s.h.tailscale.set_host_login("bob@acme.example");
    let closed = tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(chunk) = sse.chunk().await.unwrap() {
            let text = String::from_utf8_lossy(&chunk);
            assert!(!text.contains("event: state"), "unexpected state after auth revocation: {text}");
        }
    })
    .await;
    assert!(closed.is_ok(), "SSE stream did not close after host login changed");

    // The board's /api/state probe then gets 403 with structured tailscale_denied auth metadata.
    let probe = s
        .http
        .get(format!("{}/api/state", s.base))
        .header("Host", "box.acme.ts.net:47732")
        .header("X-Forwarded-Proto", "https")
        .header("Tailscale-User-Login", "alice@acme.example")
        .send()
        .await
        .unwrap();
    assert_eq!(probe.status(), 403);
    let body: Value = probe.json().await.unwrap();
    assert_eq!(body["auth"]["kind"], "tailscale_denied");
    assert_eq!(body["auth"]["login"], "alice@acme.example");
}

#[tokio::test]
async fn thread_links_skip_api_open_on_remote_hosts_and_keep_loopback_behavior() {
    let (html, st) = two_gate_page().await;

    // 1. Loopback hosts (127.0.0.1, localhost, [::1]) intercept click and POST /api/open.
    for host in ["127.0.0.1", "localhost", "[::1]"] {
        let prelude = format!(
            r#"
globalThis.__location = {{ hostname: "{host}", hash: "", pathname: "/", origin: "http://{host}:47731", reload() {{}} }};
globalThis.__fetchReply = (url) => url === "/api/open"
  ? Promise.resolve({{ ok: true, status: 200, json: () => Promise.resolve({{ opened: true }}) }})
  : new Promise(() => {{}});
"#
        );
        let r = run_page_async_with(
            &html,
            &prelude,
            &format!(
                r#"async () => {{
  emitState({st});
  const a = threadLink("{THREAD_LINK}", "thread", "Open thread");
  const ev = a.dispatch("click", {{ button: 0, ctrlKey: false, metaKey: false, shiftKey: false, altKey: false }});
  await Promise.resolve();
  const opens = fetches.filter((f) => f.url === "/api/open");
  return {{ prevented: ev.defaultPrevented, openCount: opens.length, openBody: opens[0] ? JSON.parse(opens[0].body) : null }};
}}"#
            ),
        );
        assert_eq!(r["prevented"], true, "host={host}: {r}");
        assert_eq!(r["openCount"], 1, "host={host}: {r}");
        assert_eq!(r["openBody"]["link"], THREAD_LINK, "host={host}: {r}");
    }

    // 2. Remote hosts never call /api/open and leave the native <a href="https://app.t3.codes/threads/..."> click unprevented.
    for host in ["box.acme.ts.net", "board.acme.example"] {
        let prelude = format!(
            r#"
globalThis.__location = {{ hostname: "{host}", hash: "", pathname: "/", origin: "https://{host}:47732", reload() {{}} }};
globalThis.__openedTabs = [];
globalThis.open = (u, target, features) => {{ globalThis.__openedTabs.push({{ u, target, features }}); }};
"#
        );
        let r = run_page_async_with(
            &html,
            &prelude,
            &format!(
                r#"async () => {{
  emitState({st});
  const a = threadLink("{THREAD_LINK}", "thread", "Open thread");
  const ev = a.dispatch("click", {{ button: 0, ctrlKey: false, metaKey: false, shiftKey: false, altKey: false }});
  await openThread("{THREAD_LINK}");
  const opens = fetches.filter((f) => f.url === "/api/open");
  return {{
    href: a.href,
    target: a.getAttribute("target"),
    prevented: ev.defaultPrevented,
    openCount: opens.length,
    directTabs: globalThis.__openedTabs,
  }};
}}"#
            ),
        );
        assert_eq!(r["href"], THREAD_LINK, "host={host}: {r}");
        assert_eq!(r["target"], "_blank", "host={host}: {r}");
        assert_eq!(r["prevented"], false, "host={host}: {r}");
        assert_eq!(r["openCount"], 0, "host={host}: {r}");
        assert_eq!(r["directTabs"][0]["u"], THREAD_LINK, "host={host}: {r}");
    }
}

#[tokio::test]
async fn remote_auth_loss_shows_signin_screen_and_stops_sse_retries() {
    let (html, st) = two_gate_page().await;

    // 1. Revoked/expired pairing session closes SSE -> probes /api/state -> 401 -> sign-in screen, no retry loop.
    let pairing_prelude = r#"
globalThis.__location = { hostname: "board.acme.example", hash: "", pathname: "/", origin: "https://board.acme.example:47732", reload() { globalThis.__reloaded = true; } };
globalThis.__stateReply = null;
globalThis.__fetchReply = (url) => {
  if (url === "/api/state" && globalThis.__stateReply) {
    const r = globalThis.__stateReply;
    return Promise.resolve({ ok: r.status >= 200 && r.status < 300, status: r.status, json: () => Promise.resolve(r.body) });
  }
  return new Promise(() => {});
};
"#;
    let r = run_page_async_with(
        &html,
        pairing_prelude,
        &format!(
            r#"async () => {{
  emitState({st});
  const beforeEs = eventSources.length;
  globalThis.__stateReply = {{
    status: 401,
    body: {{ error: "remote board session is invalid or expired", auth: {{ kind: "pairing", reason: "expired", pairingUrl: "/pair" }} }},
  }};
  eventSource.onerror();
  await Promise.resolve();
  await Promise.resolve();
  await Promise.resolve();
  advance(120000);
  return {{
    screenHidden: document.getElementById("auth-screen").hidden,
    appHidden: document.getElementById("app").hidden,
    title: document.getElementById("auth-title").textContent,
    cmd: document.getElementById("auth-cmd").textContent,
    pairHidden: document.getElementById("auth-pair").hidden,
    pairHref: document.getElementById("auth-pair").getAttribute("href"),
    esCountBefore: beforeEs,
    esCountAfter: eventSources.length,
    esClosed: eventSources[0].closed,
  }};
}}"#
        ),
    );
    assert_eq!(r["screenHidden"], false, "{r}");
    assert_eq!(r["appHidden"], true, "{r}");
    assert_eq!(r["title"], "Session expired or revoked", "{r}");
    assert_eq!(r["cmd"], "t3cm board pair", "{r}");
    assert_eq!(r["pairHidden"], false, "{r}");
    assert_eq!(r["pairHref"], "/pair", "{r}");
    assert_eq!(r["esCountBefore"], 1, "{r}");
    assert_eq!(r["esCountAfter"], 1, "SSE must not reconnect after 401: {r}");
    assert_eq!(r["esClosed"], true, "{r}");

    // 2. Tailscale live auth recheck closes SSE -> probes /api/state -> 403 tailscale_denied -> access denied screen, no retry loop.
    let ts_prelude = r#"
globalThis.__location = { hostname: "box.acme.ts.net", hash: "", pathname: "/", origin: "https://box.acme.ts.net:47732", reload() {} };
globalThis.__stateReply = null;
globalThis.__fetchReply = (url) => {
  if (url === "/api/state" && globalThis.__stateReply) {
    const r = globalThis.__stateReply;
    return Promise.resolve({ ok: r.status >= 200 && r.status < 300, status: r.status, json: () => Promise.resolve(r.body) });
  }
  return new Promise(() => {});
};
"#;
    let r = run_page_async_with(
        &html,
        ts_prelude,
        &format!(
            r#"async () => {{
  emitState({st});
  globalThis.__stateReply = {{
    status: 403,
    body: {{ error: "Tailscale user is not allowed to access this board", auth: {{ kind: "tailscale_denied", login: "bob@acme.example" }} }},
  }};
  eventSource.onerror();
  await Promise.resolve();
  await Promise.resolve();
  await Promise.resolve();
  advance(120000);
  return {{
    screenHidden: document.getElementById("auth-screen").hidden,
    appHidden: document.getElementById("app").hidden,
    title: document.getElementById("auth-title").textContent,
    desc: document.getElementById("auth-desc").textContent,
    pairHidden: document.getElementById("auth-pair").hidden,
    esCount: eventSources.length,
  }};
}}"#
        ),
    );
    assert_eq!(r["screenHidden"], false, "{r}");
    assert_eq!(r["appHidden"], true, "{r}");
    assert_eq!(r["title"], "Access denied", "{r}");
    assert!(r["desc"].as_str().unwrap().contains("bob@acme.example"), "{r}");
    assert_eq!(r["pairHidden"], true, "{r}");
    assert_eq!(r["esCount"], 1, "SSE must not reconnect after Tailscale 403: {r}");

    // 3. Initial /api/state 401 on a remote host lands on the sign-in screen and closes SSE immediately.
    let init_401_prelude = r#"
globalThis.__location = { hostname: "board.acme.example", hash: "", pathname: "/", origin: "https://board.acme.example:47732", reload() {} };
globalThis.__fetchReply = (url) => url === "/api/state"
  ? Promise.resolve({
      ok: false,
      status: 401,
      json: () => Promise.resolve({ error: "remote board authentication is required", auth: { kind: "pairing", reason: "missing", pairingUrl: "/pair" } }),
    })
  : new Promise(() => {});
"#;
    let r = run_page_async_with(
        &html,
        init_401_prelude,
        r#"async () => {
  await Promise.resolve();
  await Promise.resolve();
  await Promise.resolve();
  advance(120000);
  return {
    screenHidden: document.getElementById("auth-screen").hidden,
    title: document.getElementById("auth-title").textContent,
    esCount: eventSources.length,
    esClosed: eventSources[0].closed,
  };
}"#,
    );
    assert_eq!(r["screenHidden"], false, "{r}");
    assert_eq!(r["title"], "Sign in to t3 board", "{r}");
    assert_eq!(r["esCount"], 1, "{r}");
    assert_eq!(r["esClosed"], true, "{r}");

    // 4. Transient 503 on remote host keeps reconnecting; loopback SSE error reconnects without auth screen.
    let transient_prelude = r#"
globalThis.__location = { hostname: "board.acme.example", hash: "", pathname: "/", origin: "https://board.acme.example:47732", reload() {} };
globalThis.__fetchReply = (url) => url === "/api/state"
  ? Promise.resolve({ ok: false, status: 503, json: () => Promise.resolve({ error: "restarting" }) })
  : new Promise(() => {});
"#;
    let r = run_page_async_with(
        &html,
        transient_prelude,
        &format!(
            r#"async () => {{
  emitState({st});
  eventSource.onerror();
  await Promise.resolve();
  await Promise.resolve();
  await Promise.resolve();
  advance(2000);
  return {{
    screenHidden: document.getElementById("auth-screen").hidden,
    esCount: eventSources.length,
  }};
}}"#
        ),
    );
    assert_eq!(r["screenHidden"], true, "{r}");
    assert_eq!(r["esCount"], 2, "transient 503 must still reconnect: {r}");

    let loopback_prelude = r#"
globalThis.__location = { hostname: "127.0.0.1", hash: "", pathname: "/", origin: "http://127.0.0.1:47731", reload() {} };
"#;
    let r = run_page_async_with(
        &html,
        loopback_prelude,
        &format!(
            r#"async () => {{
  emitState({st});
  const stateFetchesBefore = fetches.filter((f) => f.url === "/api/state").length;
  eventSource.onerror();
  advance(1500);
  const stateFetchesAfter = fetches.filter((f) => f.url === "/api/state").length;
  return {{
    screenHidden: document.getElementById("auth-screen").hidden,
    probed: stateFetchesAfter - stateFetchesBefore,
    esCount: eventSources.length,
  }};
}}"#
        ),
    );
    assert_eq!(r["screenHidden"], true, "{r}");
    assert_eq!(r["probed"], 0, "loopback SSE error must not probe auth: {r}");
    assert_eq!(r["esCount"], 2, "loopback SSE must reconnect on backoff: {r}");

    // 5. S3: Remote /api/state probe sets a timeout signal and does not reset SSE backoff when SSE keeps failing.
    let backoff_prelude = r#"
globalThis.__location = { hostname: "board.acme.example", hash: "", pathname: "/", origin: "https://board.acme.example:47732", reload() {} };
globalThis.__probeSignals = [];
globalThis.AbortSignal = { timeout: (ms) => ({ timeoutMs: ms }) };
globalThis.__fetchReply = (url, opts) => {
  if (url === "/api/state") {
    if (opts && opts.signal) globalThis.__probeSignals.push(opts.signal.timeoutMs);
    return Promise.resolve({ ok: true, status: 200, json: () => Promise.resolve(globalThis.__snap) });
  }
  return new Promise(() => {});
};
"#;
    let r = run_page_async_with(
        &html,
        backoff_prelude,
        &format!(
            r#"async () => {{
  globalThis.__snap = {st};
  emitState({st});
  eventSource.onerror();
  await Promise.resolve();
  await Promise.resolve();
  await Promise.resolve();
  advance(1000);
  const afterFirst = eventSources.length;
  // Second SSE failure without delivering a state event must back off to 2000ms (not reset to 1000ms).
  eventSource.onerror();
  await Promise.resolve();
  await Promise.resolve();
  await Promise.resolve();
  advance(1500);
  const beforeBackoff = eventSources.length;
  advance(600);
  const afterBackoff = eventSources.length;
  return {{
    probeSignals: globalThis.__probeSignals,
    afterFirst,
    beforeBackoff,
    afterBackoff,
  }};
}}"#
        ),
    );
    assert_eq!(r["probeSignals"], json!([5000, 5000]), "{r}");
    assert_eq!(r["afterFirst"], 2, "{r}");
    assert_eq!(r["beforeBackoff"], 2, "200 /api/state probe must not reset SSE backoff: {r}");
    assert_eq!(r["afterBackoff"], 3, "{r}");

    // 6. S1: Browsers where `new Notification()` throws TypeError (e.g. Chrome on Android) mark notifications unavailable.
    let notify_prelude = r#"
globalThis.Notification = class {
  constructor() { throw new TypeError("Illegal constructor"); }
};
Notification.permission = "granted";
"#;
    let r = run_page_async_with(
        &html,
        notify_prelude,
        &format!(
            r#"async () => {{
  emitState({st});
  document.hidden = true;
  const fresh = {{ ...{st}, cards: [...{st}.cards, {{ ...{st}.cards[0], id: "a9", at: "2099-01-01T00:00:00Z" }}] }};
  emitState(fresh);
  return {{
    label: document.getElementById("notify-text").textContent,
    disabled: document.getElementById("notify").getAttribute("aria-disabled"),
  }};
}}"#
        ),
    );
    assert_eq!(r["label"], "Notifications unavailable", "{r}");
    assert_eq!(r["disabled"], "true", "{r}");
}

const WAITING: &str = "Four tickets wait on you:

1. [widgets#2](https://app.t3.codes/threads/env-1/launch%3Aa1) waits on a **merge** of `a1`.
2. [widgets#17](https://app.t3.codes/threads/env-1/launch%3Ab2) is blocked on a login.
3. [gadgets#657](https://app.t3.codes/threads/env-1/launch%3Ac3) waits on review.
4. [widgets#96](https://app.t3.codes/threads/env-1/launch%3Ad4) waits on a deploy.";

const ANSWER_PROBE: &str = r##"
emitState({ ...base, asks: [] });
emitState(base);
advance(100);
const aa = q("#asks .aa");
const links = (el) => el.querySelectorAll("a").map((a) => [a.textContent, a.getAttribute("href"), a.getAttribute("title"), a.getAttribute("target"), a.getAttribute("rel")]);
"##;

async fn ask_page(answers: &[(&str, &str)]) -> (String, Value) {
    let s = start(H::new()).await;
    for (q, a) in answers {
        answered_ask(&s.h, q, a).await;
    }
    let html = s.http.get(format!("{}/", s.base)).send().await.unwrap().text().await.unwrap();
    let st: Value = s.http.get(format!("{}/api/state", s.base)).send().await.unwrap().json().await.unwrap();
    (html, st)
}

#[tokio::test]
async fn an_ask_answer_shows_its_links_as_labels_with_no_urls() {
    let (html, st) = ask_page(&[("Which tickets wait on me and why?", WAITING)]).await;
    assert!(html.contains(".md ol { list-style: decimal; }") && html.contains(".md ul { list-style: disc; }"));
    let r = page_scenario(
        &html,
        &st,
        &format!(
            r##"{ANSWER_PROBE}
return {{
  items: aa.querySelectorAll("ol li").map((li) => li.textContent),
  lead: aa.querySelector("p").textContent,
  links: links(aa),
  bold: aa.querySelector("strong").textContent,
  code: aa.querySelector("code").textContent,
  text: aa.textContent,
  title: aa.getAttribute("title"),
  more: !q('#asks [data-part="more"]').hidden,
  live: document.getElementById("live").textContent,
}};"##
        ),
    );
    let thread = |id: &str| format!("https://app.t3.codes/threads/env-1/launch%3A{id}");
    let link = |label: &str, id: &str| json!([label, thread(id), thread(id), "_blank", "noopener noreferrer"]);
    assert_eq!(
        r["items"],
        json!([
            "widgets#2 waits on a merge of a1.",
            "widgets#17 is blocked on a login.",
            "gadgets#657 waits on review.",
            "widgets#96 waits on a deploy."
        ]),
        "{r}"
    );
    assert_eq!(r["lead"], "Four tickets wait on you:");
    assert_eq!(
        r["links"],
        json!([link("widgets#2", "a1"), link("widgets#17", "b2"), link("gadgets#657", "c3"), link("widgets#96", "d4")])
    );
    assert_eq!((&r["bold"], &r["code"], &r["more"]), (&json!("merge"), &json!("a1"), &json!(true)));
    for key in ["text", "title", "live"] {
        let t = r[key].as_str().unwrap();
        assert!(!t.contains("http") && !t.contains("](") && !t.contains("**"), "{key}: {t}");
    }
    assert!(r["live"].as_str().unwrap().contains("1. widgets#2 waits on a merge of a1.\n2. widgets#17"), "{r}");
}

#[tokio::test]
async fn an_answer_that_dispatched_shows_the_thread_and_project_and_the_ticket_row() {
    let s = start(H::new()).await;
    dispatch_projects(&s.h);
    let a = s.h.app.wf.ask("Cut a new release for t3-codemode").await.unwrap();
    let thread = a["thread"].as_str().unwrap().to_string();
    let go = "async () => wf.dispatch({ project: 'p-codemode', request: 'Cut a new release for t3-codemode', title: 'release' })";
    let r = result(&s.h.exec_as(&thread, go).await);
    s.h.fake.update(&thread, json!({"status": "completed"}));
    s.h.fake.set_items(&thread, vec![json!({"type": "assistant_message", "position": 1, "text": "Launched it."})]);
    s.h.app.wf.check_asks(None).await.unwrap();
    answered_ask(&s.h, "Which tickets wait on me?", "None.").await;
    let html = s.http.get(format!("{}/", s.base)).send().await.unwrap().text().await.unwrap();
    let st: Value = s.http.get(format!("{}/api/state", s.base)).send().await.unwrap().json().await.unwrap();
    assert_eq!(st["asks"].as_array().unwrap().iter().filter(|a| a["dispatched"].is_object()).count(), 1, "{st}");
    let link = r["link"].as_str().unwrap();
    assert!(link.starts_with("https://"), "{link}");
    assert!(html.contains(r#"<h2 id="h-ask">Ask your agent</h2>"#));
    assert!(html.contains(r#"placeholder="Ask about the board, or ask for work: cut a release of t3-codemode""#));
    let out = page_scenario(
        &html,
        &st,
        r##"
emitState(base);
const rows = [...document.getElementById("asks").children];
const row = (q) => rows.find((li) => li.querySelector(".aq").textContent === q);
const d = row("Cut a new release for t3-codemode").querySelector('[data-part="d"]');
const a = d.querySelector("a");
return {
  text: d.textContent, hidden: d.hidden, href: a && a.getAttribute("href"),
  other: row("Which tickets wait on me?").querySelector('[data-part="d"]').hidden,
  open: document.getElementById("open").textContent,
};"##,
    );
    assert_eq!(out["text"], "Launched t3-codemode: release in t3-codemode", "{out}");
    assert_eq!((&out["hidden"], &out["other"]), (&json!(false), &json!(true)));
    assert_eq!(out["href"], link);
    assert!(out["open"].as_str().unwrap().contains("t3-codemode: release"), "{out}");
}

#[tokio::test]
async fn bare_urls_link_and_unsafe_hrefs_show_only_their_label() {
    let answer = "See https://github.com/acme/widgets/pull/41. Skip [the trap](javascript:alert(1)) and [ftp](ftp://files.example/x).";
    let (html, st) = ask_page(&[("Where is the PR?", answer)]).await;
    let r = page_scenario(
        &html,
        &st,
        &format!(
            r##"{ANSWER_PROBE}
return {{ links: links(aa), text: aa.textContent, live: document.getElementById("live").textContent }};"##
        ),
    );
    let pr = "https://github.com/acme/widgets/pull/41";
    assert_eq!(r["links"], json!([[pr, pr, pr, "_blank", "noopener noreferrer"]]));
    assert_eq!(r["text"], format!("See {pr}. Skip the trap and ftp."));
    assert_eq!(r["live"], "Answer to “Where is the PR?”: See github.com. Skip the trap and ftp.");
}

#[tokio::test]
async fn a_link_the_cap_cut_shows_as_its_label_in_plain_text() {
    let item = "\n\n1. [widgets#96](https://app.t3.codes/threads/env-1/launch%3Ad4) waits on a deploy.";
    let in_url = format!("{}{item}", "x".repeat(1460));
    let in_label = format!("{}{item}", "x".repeat(1490));
    let after_label = format!("{}{item}", "x".repeat(1483));
    let in_parens = format!("{}\n\n1. [widgets#96](https://acme.example/w(1)/x) waits.", "x".repeat(1457));
    let spaced = format!("{}\n\n1. [widgets#96]( https://acme.example/docs) waits.", "x".repeat(1470));
    let titled = format!("{}\n\n1. [widgets#96](https://acme.example/docs \"Docs page\") waits.", "x".repeat(1450));
    let bare = format!("{} https://github.com/acme/widgets/pull/41", "x".repeat(1480));
    let (html, st) = ask_page(&[
        ("In the URL?", &in_url),
        ("In the label?", &in_label),
        ("After the label?", &after_label),
        ("After the parens?", &in_parens),
        ("After a space?", &spaced),
        ("In the title?", &titled),
        ("Bare?", &bare),
    ])
    .await;
    let stored: Vec<&Value> = st["asks"].as_array().unwrap().iter().collect();
    assert!(
        stored.iter().all(|a| a["answerCut"] == true && a["answer"].as_str().unwrap().chars().count() == 1503),
        "{stored:?}"
    );
    assert!(stored.iter().any(|a| a["answer"].as_str().unwrap().ends_with("[widgets#96]...")));
    assert!(stored.iter().any(|a| a["answer"].as_str().unwrap().ends_with("/w(1)...")));
    let r = page_scenario(
        &html,
        &st,
        r##"
emitState(base);
return [...document.getElementById("asks").children].map((li) => {
  const aa = li.querySelector(".aa");
  return { q: li.querySelector(".aq").textContent, anchors: aa.querySelectorAll("a").length, tail: aa.textContent.slice(-16) };
});"##,
    );
    let mut tails: Vec<(String, u64, String)> = r
        .as_array()
        .unwrap()
        .iter()
        .map(|a| (a["q"].as_str().unwrap().into(), a["anchors"].as_u64().unwrap(), a["tail"].as_str().unwrap().into()))
        .collect();
    tails.sort();
    assert_eq!(
        tails,
        [
            ("After a space?".into(), 0, format!("{}widgets#96...", "x".repeat(3))),
            ("After the label?".into(), 0, format!("{}widgets#96...", "x".repeat(3))),
            ("After the parens?".into(), 0, format!("{}widgets#96...", "x".repeat(3))),
            ("Bare?".into(), 0, format!("{} ...", "x".repeat(12))),
            ("In the URL?".into(), 0, format!("{}widgets#96...", "x".repeat(3))),
            ("In the label?".into(), 0, format!("{}widg...", "x".repeat(9))),
            ("In the title?".into(), 0, format!("{}widgets#96...", "x".repeat(3))),
        ]
    );
}

#[tokio::test]
async fn a_cut_answer_keeps_the_text_after_a_closed_parenthesis() {
    let answer = format!("[note](see above) and {}", "more ".repeat(400));
    let (html, st) = ask_page(&[("Anything else?", &answer)]).await;
    assert_eq!(st["asks"][0]["answerCut"], true);
    let r = page_scenario(&html, &st, &format!(r##"{ANSWER_PROBE}return aa.textContent.slice(0, 32);"##));
    assert_eq!(r, "[note](see above) and more more ");
}

#[tokio::test]
async fn code_and_bold_in_a_link_label_render_without_their_markers() {
    let answer = "See [`src/board.rs`](https://acme.example/board), [**the PR**](https://acme.example/pr) and [`setup`](file:setup).";
    let (html, st) = ask_page(&[("Where is it?", answer)]).await;
    let r = page_scenario(
        &html,
        &st,
        &format!(
            r##"{ANSWER_PROBE}
return {{ links: aa.querySelectorAll("a").map((a) => a.textContent), code: aa.querySelectorAll("code").map((c) => c.textContent),
  bold: aa.querySelector("strong") && aa.querySelector("strong").textContent, text: aa.textContent, live: document.getElementById("live").textContent }};"##
        ),
    );
    assert_eq!(r["links"], json!(["src/board.rs", "the PR"]));
    assert_eq!(r["code"], json!(["src/board.rs", "setup"]));
    assert_eq!(r["bold"], "the PR");
    assert_eq!(r["text"], "See src/board.rs, the PR and setup.");
    assert!(r["live"].as_str().unwrap().ends_with("See src/board.rs, the PR and setup."), "{r}");
}

#[tokio::test]
async fn settled_ask_rows_render_the_answer_on_one_line() {
    let s = start(H::new()).await;
    answered_ask(&s.h, "Which tickets wait on me and why?", WAITING).await;
    s.h.clock.advance_minutes(24 * 60);
    s.h.app.wf.board_tick().await.unwrap();
    let html = s.http.get(format!("{}/", s.base)).send().await.unwrap().text().await.unwrap();
    let st: Value = s.http.get(format!("{}/api/state", s.base)).send().await.unwrap().json().await.unwrap();
    assert_eq!(st["settled"]["asks"].as_array().unwrap().len(), 1, "{st}");
    let r = page_scenario(
        &html,
        &st,
        r#"
emitState(base);
const an = q('#settled .srow [data-part="a"]');
return { text: an.textContent, blocks: an.querySelectorAll("li, p, ol, br").length, labels: an.querySelectorAll("a").map((a) => a.textContent), title: an.getAttribute("title") };"#,
    );
    assert_eq!(
        r["text"],
        "Four tickets wait on you: 1. widgets#2 waits on a merge of a1. 2. widgets#17 is blocked on a login. 3. gadgets#657 waits on review. 4. widgets#96 waits on a deploy."
    );
    assert_eq!(r["blocks"], 0);
    assert_eq!(r["labels"], json!(["widgets#2", "widgets#17", "gadgets#657", "widgets#96"]));
    assert!(!r["title"].as_str().unwrap().contains("http"), "{r}");
}

#[tokio::test]
async fn a_short_answer_keeps_a_link_before_dots_and_shows_all_lines() {
    let (html, st) = ask_page(&[("Docs?", "See https://acme.example/docs...\nsecond\nthird")]).await;
    assert_eq!(st["asks"][0]["answerCut"], false);
    let r = page_scenario(
        &html,
        &st,
        &format!(
            r##"{ANSWER_PROBE}
return {{ links: links(aa), more: !q('#asks [data-part="more"]').hidden, title: aa.getAttribute("title") }};"##
        ),
    );
    let docs = "https://acme.example/docs";
    assert_eq!(r["links"], json!([[docs, docs, docs, "_blank", "noopener noreferrer"]]));
    assert_eq!(r["more"], true);
    assert_eq!(r["title"], "See acme.example...\nsecond\nthird");
}

#[test]
fn answers_stored_before_the_cut_flag_infer_it_from_their_shape() {
    let view = |d: Value| t3_codemode::board::ask_view(1, &d)["answerCut"].clone();
    let capped = format!("{}...", "x".repeat(1500));
    assert_eq!(view(json!({"answer": capped})), true);
    assert_eq!(view(json!({"answer": "See https://acme.example/docs..."})), false);
    assert_eq!(view(json!({"answer": capped, "answerCut": false})), false);
    assert_eq!(view(json!({"answer": null})), Value::Null);
}

#[tokio::test]
async fn without_a_flow_the_graph_view_draws_open_tickets_one_lane_per_repo_and_is_remembered() {
    let (html, st) = two_gate_page().await;
    let r = run_page_async_with(
        &html,
        r#"globalThis.__storage = { "t3board.view": "queue" };"#,
        &format!(
            r##"async () => {{
{PAGE_HELPERS}
const base = {st};
{ROWS}
const tree = [
  T("acme/b#2", {{ repo: "acme/b", status: "running" }}),
  T("acme/a#1", {{ repo: "acme/a" }}),
  T("loose#3"),
  T("acme/a#4", {{ repo: "acme/a", done: true }}),
];
const restored = document.getElementById("app").getAttribute("data-view");
emitState({{ ...base, cards: [], tree }});
const btn = (id) => q('.gnode[data-id="' + id + '"]');
const lanes = document.querySelectorAll(".glt").map((t) => t.childNodes.filter((n) => n.nodeType === 3).map((n) => n.data).join(""));
const nodes = document.querySelectorAll(".gnode").map((b) => [b.dataset.id, b.getAttribute("tabindex"), b.querySelector('[data-part="st"]').textContent]);
btn("t:t-acme/a#1").focus();
btn("t:t-acme/a#1").dispatch("keydown", {{ key: "ArrowDown" }});
const moved = document.activeElement.dataset.id;
btn("t:t-acme/b#2").dispatch("keydown", {{ key: "End" }});
const end = document.activeElement.dataset.id;
prefs.keys = false;
btn("t:t-loose#3").dispatch("keydown", {{ key: "k" }});
const keysOff = document.activeElement.dataset.id;
prefs.keys = true;
btn("t:t-loose#3").dispatch("keydown", {{ key: "k" }});
const keysOn = document.activeElement.dataset.id;
const tabs = nodes.length && document.querySelectorAll(".gnode").filter((b) => b.getAttribute("tabindex") === "0").map((b) => b.dataset.id);
btn("t:t-acme/a#1").dispatch("click");
const opened = document.getElementById("drawer").getAttribute("data-kind") + " " + q('#drawer [data-part="id"]').textContent;
const depsHidden = q('#drawer [data-part="deps"]').hidden;
selectView("dispatch");
const pref = localStorage.getItem("t3board.view");
return {{ restored, lanes, nodes, moved, end, keysOff, keysOn, tabs, opened, depsHidden, pref, note: q("#graph-note").textContent,
  edges: document.querySelectorAll(".ge").length, flowFetches: fetches.filter((f) => f.url === "/api/flow").length }};
}}"##
        ),
    );
    assert_eq!(r["restored"], "queue", "the viewer's last view comes back from localStorage: {r}");
    assert_eq!(r["lanes"], json!(["acme/a", "acme/b", "Tickets"]), "{r}");
    assert_eq!(
        r["nodes"],
        json!([["t:t-acme/a#1", "0", "idle"], ["t:t-acme/b#2", "-1", "running"], ["t:t-loose#3", "-1", "idle"]]),
        "open tickets only, one roving tab stop: {r}"
    );
    assert!(r["note"].as_str().unwrap().starts_with("Reading the dependency graph"), "{r}");
    assert_eq!((&r["edges"], &r["flowFetches"]), (&json!(0), &json!(1)), "{r}");
    assert_eq!((&r["moved"], &r["end"]), (&json!("t:t-acme/b#2"), &json!("t:t-loose#3")), "{r}");
    assert_eq!(
        (&r["keysOff"], &r["keysOn"]),
        (&json!("t:t-loose#3"), &json!("t:t-acme/b#2")),
        "k moves only while single-key shortcuts are on: {r}"
    );
    assert_eq!(r["tabs"], json!(["t:t-acme/b#2"]), "{r}");
    assert_eq!(r["opened"], "ticket acme/a#1", "{r}");
    assert_eq!(r["depsHidden"], true, "{r}");
    assert_eq!(r["pref"], "dispatch", "{r}");
    for needle in ["id=\"vt-queue\"", "id=\"tp-graph\"", "t3board.view", "createElementNS(SVG_NS"] {
        assert!(html.contains(needle), "missing {needle:?}");
    }
    let doc = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/docs/src/board.md")).unwrap();
    assert!(doc.contains("### The Queue & Graph view"), "docs/src/board.md documents the graph view");
}

#[tokio::test]
async fn without_a_flow_the_graph_view_shows_pending_launching_tickets_and_migrates_them_on_join() {
    let (html, st) = two_gate_page().await;
    let r = run_page_async_with(
        &html,
        r#"globalThis.__storage = { "t3board.view": "queue" };"#,
        &format!(
            r##"async () => {{
{PAGE_HELPERS}
const base = {st};
{ROWS}
const emptyGraph = {{ enabled: false, launch: null, maxTickets: 0, free: 0, running: [], queue: [], blocked: [], skipped: [], graph: {{ nodes: [], edges: [], warnings: [] }} }};
let resolveFlow = null;
fetchReply = (url) => url === "/api/flow"
  ? new Promise((resolve) => {{ resolveFlow = resolve; }})
  : new Promise(() => {{}});
const P1 = {{ ...T("acme/a#9", {{ repo: "acme/a" }}), thread: null, ticket: null, link: null, joined: false, status: "launching", title: null, humanTitle: null, issueTitle: null }};
const P2 = {{ ...T("loose#10", {{}}), thread: null, ticket: null, link: null, joined: false, status: null, title: null, humanTitle: null, issueTitle: null }};
const live = T("acme/a#1", {{ repo: "acme/a", status: "paused", paused: true }});
const done = T("acme/a#4", {{ repo: "acme/a", done: true }});
const snap = {{ ...base, cards: [], tree: [live, P1, P2, done] }};
emitState(snap);
const btn = (id) => q('.gnode[data-id="' + id + '"]');
const gnodes = () => document.querySelectorAll(".gnode").map((b) => [b.dataset.id, b.querySelector('[data-part="st"]').textContent]);
const openKeys = () => [...document.getElementById("open").children].filter((el) => el.dataset.key).map((el) => el.dataset.key);
const loading = {{ nOpen: q("#n-open").textContent, nGraph: q("#n-graph").textContent, openKeys: openKeys(), nodes: gnodes() }};

// When /api/flow responds with empty graph.nodes (no repos discovered), degraded mode still keeps pending tickets.
resolveFlow({{ ok: true, status: 200, json: async () => emptyGraph }});
for (let i = 0; i < 8; i++) await Promise.resolve();
const emptyFlow = {{ nOpen: q("#n-open").textContent, nGraph: q("#n-graph").textContent, nodes: gnodes(), note: q("#graph-note").textContent }};

// Status filter "running" matches pending launching tickets on both Open and Graph tabs.
setFilter({{ statuses: ["running"] }});
const filtered = {{
  nOpen: q("#n-open").textContent,
  nGraph: q("#n-graph").textContent,
  outIds: document.querySelectorAll(".gnode.out").map((b) => b.dataset.id),
}};
setFilter({{}});

// Clicking a pending node announces "not joined yet" and opens the launching view instead of a missing thread.
btn("pending:acme/a#9").dispatch("click");
advance(100);
const clicked = {{
  live: q("#live").textContent,
  open: q("#drawer").classList.contains("open"),
  kind: q("#drawer").getAttribute("data-kind"),
  goneHidden: q('#drawer [data-part="gone"]').hidden,
  id: q('#drawer [data-part="id"]').textContent,
  st: q('#drawer [data-part="st"]').textContent,
  note: q('#drawer [data-part="note-m"]').textContent,
  t3Empty: !q('#drawer [data-part="t3"]').firstChild,
}};

// When the thread joins while the drawer is open, it transitions to the real ticket key with no duplicate.
const joined1 = {{ ...P1, thread: "th-9", ticket: "th-9", joined: true, status: "running", link: "https://t3/th-9" }};
emitState({{ ...snap, tree: [live, joined1, P2, done] }});
const afterJoinOpenDrawer = {{
  nodes: gnodes(),
  nOpen: q("#n-open").textContent,
  nGraph: q("#n-graph").textContent,
  drawerKind: q("#drawer").getAttribute("data-kind"),
  drawerSt: q('#drawer [data-part="st"]').textContent,
  drawerGoneHidden: q('#drawer [data-part="gone"]').hidden,
}};
closeDrawer(true);
const restoredFocus = document.activeElement && document.activeElement.dataset ? document.activeElement.dataset.id : null;

// When a pending node has focus inside #gscroll and joins, its button migrates in place without duplicating.
const p2Btn = btn("pending:loose#10");
p2Btn.focus();
const joined2 = {{ ...P2, thread: "th-10", ticket: "th-10", joined: true, status: "running", link: "https://t3/th-10" }};
emitState({{ ...snap, tree: [live, joined1, joined2, done] }});
const migratedInPlace = {{
  sameBtn: btn("t:th-10") === p2Btn,
  oldGone: !btn("pending:loose#10"),
  nodes: gnodes(),
}};
btn("t:th-10").dispatch("click");
const clickedAfterJoin = {{ kind: q("#drawer").getAttribute("data-kind"), id: q('#drawer [data-part="id"]').textContent }};
closeDrawer(false);

// Ambiguous join (two threads with the same label) never guesses which gets the pending node and leaves no pending duplicate.
const P3 = {{ ...T("acme/a#11", {{ repo: "acme/a" }}), thread: null, ticket: null, link: null, joined: false, status: "launching" }};
p2Btn.blur();
q("#gscroll").dispatch("focusout");
advance(1);
emitState({{ ...snap, tree: [live, P3] }});
const p3Btn = btn("pending:acme/a#11");
p3Btn.focus();
emitState({{ ...snap, tree: [live, {{ ...P3, thread: "th-11a", joined: true, status: "running" }}, {{ ...P3, thread: "th-11b", joined: true, status: "running" }}] }});
const ambiguousHeldId = p3Btn.dataset.id;
p3Btn.blur();
q("#gscroll").dispatch("focusout");
advance(1);
const ambiguous = gnodes();

return {{ loading, emptyFlow, filtered, clicked, afterJoinOpenDrawer, restoredFocus, migratedInPlace, clickedAfterJoin, ambiguousHeldId, ambiguous }};
}}"##
        ),
    );
    assert_eq!(r["loading"]["nOpen"], "3", "{r}");
    assert_eq!(r["loading"]["nGraph"], "3", "{r}");
    assert_eq!(r["loading"]["openKeys"], json!(["pending:acme/a#9", "pending:loose#10", "t-acme/a#1"]), "{r}");
    assert_eq!(
        r["loading"]["nodes"],
        json!([["t:t-acme/a#1", "paused"], ["pending:acme/a#9", "launching"], ["pending:loose#10", "launching"]]),
        "pending launching tickets appear alongside threaded tickets with pending:<label> keys: {r}"
    );
    assert_eq!(
        (&r["emptyFlow"]["nOpen"], &r["emptyFlow"]["nGraph"]),
        (&json!("3"), &json!("3")),
        "empty /api/flow graph keeps degraded pending tickets: {r}"
    );
    assert!(
        r["emptyFlow"]["note"].as_str().unwrap().starts_with("No GitHub repositories found"),
        "empty /api/flow switches to degraded tickets mode: {r}"
    );
    assert_eq!(r["emptyFlow"]["nodes"], r["loading"]["nodes"], "{r}");
    assert_eq!(
        (&r["filtered"]["nOpen"], &r["filtered"]["nGraph"], &r["filtered"]["outIds"]),
        (&json!("2/3"), &json!("2/3"), &json!(["t:t-acme/a#1"])),
        "running status filter matches launching tickets identically on Open and Graph tabs: {r}"
    );
    assert!(
        r["clicked"]["live"].as_str().unwrap().contains("has not joined yet"),
        "clicking a pending node announces it has not joined yet: {r}"
    );
    assert_eq!(
        (
            &r["clicked"]["open"],
            &r["clicked"]["kind"],
            &r["clicked"]["goneHidden"],
            &r["clicked"]["id"],
            &r["clicked"]["st"],
            &r["clicked"]["t3Empty"]
        ),
        (&json!(true), &json!("issue"), &json!(true), &json!("acme/a#9"), &json!("launching"), &json!(true)),
        "{r}"
    );
    assert!(
        r["clicked"]["note"].as_str().unwrap().contains("has not joined yet"),
        "drawer says the thread has not joined yet: {r}"
    );
    assert_eq!(
        r["afterJoinOpenDrawer"],
        json!({
            "nodes": [["t:t-acme/a#1", "paused"], ["t:th-9", "running"], ["pending:loose#10", "launching"]],
            "nOpen": "3",
            "nGraph": "3",
            "drawerKind": "ticket",
            "drawerSt": "running",
            "drawerGoneHidden": true,
        }),
        "joining replaces pending:<label> with t:<thread> without duplicates and upgrades open drawer: {r}"
    );
    assert_eq!(r["restoredFocus"], "t:th-9", "{r}");
    assert_eq!(
        r["migratedInPlace"],
        json!({
            "sameBtn": true,
            "oldGone": true,
            "nodes": [["t:t-acme/a#1", "paused"], ["t:th-9", "running"], ["t:th-10", "running"]],
        }),
        "{r}"
    );
    assert_eq!(r["clickedAfterJoin"], json!({"kind": "ticket", "id": "loose#10"}), "{r}");
    assert_eq!(
        r["ambiguousHeldId"], "pending:acme/a#11",
        "ambiguous join must not migrate the pending button to one of two matching threads: {r}"
    );
    assert_eq!(
        r["ambiguous"],
        json!([["t:t-acme/a#1", "paused"], ["t:th-11b", "running"], ["t:th-11a", "running"]]),
        "{r}"
    );
}

const ADVISORY_GRAPH: &str = r##"
const N = (n, state) => ({ id: "acme/a#" + n, repo: "acme/a", number: n, label: "a#" + n, title: "Issue " + n, state, tracked: true });
const view = { enabled: true, launch: "ask", maxTickets: 2, free: 2, running: [], queue: [], blocked: [], skipped: [],
  graph: { nodes: [N(1, "open"), N(2, "open"), N(3, "open"), N(4, "closed"), N(5, "closed")], warnings: [], edges: [
    { from: "acme/a#2", to: "acme/a#1", kind: "blockedBy", source: "github" },
    { from: "acme/a#3", to: "acme/a#1", kind: "codeDep", source: "code", modules: ["src/c.rs -> src/a.rs"] },
    { from: "acme/a#2", to: "acme/a#3", kind: "fileConflict", source: "code", files: ["src/b.rs"] },
    { from: "acme/a#5", to: "acme/a#4", kind: "codeDep", source: "code", modules: ["src/e.rs -> src/d.rs"] },
  ] } };
fetchReply = (url) => url === "/api/flow" ? Promise.resolve({ ok: true, status: 200, json: async () => view }) : new Promise(() => {});
emitState({ ...base, cards: [] });
for (let i = 0; i < 8; i++) await Promise.resolve();
const btn = (id) => q('.gnode[data-id="' + id + '"]');
const edges = () => Object.fromEntries(document.querySelectorAll(".ge").map((p) => [p.getAttribute("data-kind") + " " + p.getAttribute("data-from").slice(-1) + p.getAttribute("data-to").slice(-1), p.getAttribute("class")]));
const keys = () => [q("#graph-show-conflict").getAttribute("aria-pressed"), q("#graph-show-codedep").getAttribute("aria-pressed")];
"##;

#[tokio::test]
async fn advisory_graph_edges_hide_until_traced_or_toggled_and_the_toggle_is_remembered() {
    let (html, st) = two_gate_page().await;
    let r = run_page_async_with(
        &html,
        r#"globalThis.__storage = { "t3board.view": "queue" };"#,
        &format!(
            r##"async () => {{
{PAGE_HELPERS}
const base = {st};
{ADVISORY_GRAPH}
const rest = {{ edges: edges(), keys: keys() }};
btn("acme/a#3").dispatch("pointerover");
const hover = edges();
q("#gscroll").dispatch("pointerleave");
btn("acme/a#3").dispatch("click");
const pinned = edges();
q("#graph-clear").dispatch("click");
q("#graph-show-codedep").dispatch("click");
const toggled = {{ edges: edges(), keys: keys(), saved: localStorage.getItem("t3board.graph.advisory") }};
return {{ rest, hover, pinned, toggled }};
}}"##
        ),
    );
    assert_eq!(
        r["rest"]["edges"],
        json!({
            "blockedBy 21": "ge",
            "codeDep 31": "ge codedep src-code off",
            "fileConflict 23": "ge conflict src-code off",
            "codeDep 54": "ge codedep src-code off",
        }),
        "at rest only hard edges draw: {r}"
    );
    assert_eq!(r["rest"]["keys"], json!(["false", "false"]), "{r}");
    for traced in ["hover", "pinned"] {
        assert_eq!(r[traced]["codeDep 31"], "ge codedep src-code hot", "{traced}: {r}");
        assert_eq!(r[traced]["fileConflict 23"], "ge conflict src-code hot", "{traced}: {r}");
        assert_eq!(r[traced]["codeDep 54"], "ge codedep src-code off", "{traced}: {r}");
    }
    assert_eq!(
        r["toggled"],
        json!({
            "edges": {
                "blockedBy 21": "ge",
                "codeDep 31": "ge codedep src-code",
                "fileConflict 23": "ge conflict src-code off",
                "codeDep 54": "ge codedep src-code off",
            },
            "keys": ["false", "true"],
            "saved": "codeDep",
        }),
        "the code dep toggle draws every code dep between open issues and is saved: {r}"
    );

    let again = run_page_async_with(
        &html,
        r#"globalThis.__storage = { "t3board.view": "queue", "t3board.graph.advisory": "codeDep" };"#,
        &format!(
            r##"async () => {{
{PAGE_HELPERS}
const base = {st};
{ADVISORY_GRAPH}
const restored = {{ edges: edges(), keys: keys() }};
q("#graph-show-codedep").dispatch("click");
q("#graph-show-conflict").dispatch("click");
return {{ restored, flipped: {{ edges: edges(), keys: keys(), saved: localStorage.getItem("t3board.graph.advisory") }} }};
}}"##
        ),
    );
    assert_eq!(again["restored"]["keys"], json!(["false", "true"]), "the toggle survives a refresh: {again}");
    assert_eq!(again["restored"]["edges"]["codeDep 31"], "ge codedep src-code", "{again}");
    assert_eq!(again["flipped"]["keys"], json!(["true", "false"]), "{again}");
    assert_eq!(again["flipped"]["saved"], "fileConflict", "{again}");
    assert_eq!(again["flipped"]["edges"]["fileConflict 23"], "ge conflict src-code", "{again}");
    assert_eq!(again["flipped"]["edges"]["codeDep 31"], "ge codedep src-code off", "{again}");
}

const MENU_GRAPH: &str = r##"
const N = (n, extra) => ({ id: "acme/a#" + n, repo: "acme/a", number: n, label: "a#" + n, title: "Issue " + n, state: "open", tracked: true,
  url: "https://github.com/acme/a/issues/" + n, ...extra });
const view = { enabled: true, launch: "ask", maxTickets: 2, free: 2, running: [], queue: [], blocked: [], skipped: [],
  graph: { nodes: [N(1, { track: "core", trackBy: "label", unlocks: ["acme/a#2", "acme/a#3"] }), N(2, { track: "core", trackBy: "label", blockedBy: ["acme/a#1"] }),
    N(3, { track: "docs", trackBy: "label", blockedBy: ["acme/a#1"] })], warnings: [], edges: [
    { from: "acme/a#2", to: "acme/a#1", kind: "blockedBy", source: "github" },
    { from: "acme/a#3", to: "acme/a#1", kind: "blockedBy", source: "body" },
  ] } };
const edited = { ok: true, status: 200, json: async () => ({ ok: true, action: "dismiss", kind: "blockedBy" }) };
fetchReply = (url) => url === "/api/flow" ? Promise.resolve({ ok: true, status: 200, json: async () => view })
  : url === "/api/flow/edge" ? Promise.resolve(edited) : new Promise(() => {});
emitState({ ...base, cards: [] });
for (let i = 0; i < 8; i++) await Promise.resolve();
const btn = (id) => q('.gnode[data-id="' + id + '"]');
const menu = () => q("#cmenu");
const entries = () => menu().querySelectorAll('[role="menuitem"]');
const names = () => entries().map((x) => x.textContent);
const roles = () => menu().children.map((x) => x.getAttribute("role"));
const at = () => entries().indexOf(document.activeElement);
const fire = (type, target, extra) => {
  const ev = { type, target, defaultPrevented: false, preventDefault() { this.defaultPrevented = true; }, stopPropagation() {}, ...extra };
  for (const f of document.listeners[type] || []) f(ev);
  return ev;
};
const key = (k, extra) => menu().dispatch("keydown", { key: k, ...extra });
const pick = (name) => {
  if (!names().includes(name)) throw new Error("no menu item " + name + " in " + names().join(", "));
  while (entries()[at()].textContent !== name) key("ArrowDown");
  key("Enter");
};
const settle = async () => { for (let i = 0; i < 12; i++) await Promise.resolve(); };
"##;

#[tokio::test]
async fn graph_node_and_lane_menus_open_by_pointer_and_keyboard_and_run_their_actions() {
    let (html, st) = two_gate_page().await;
    let r = run_page_async_with(
        &html,
        r#"globalThis.__storage = { "t3board.view": "queue" };"#,
        &format!(
            r##"async () => {{
{PAGE_HELPERS}
const base = {st};
{MENU_GRAPH}
const copied = [];
globalThis.navigator = {{ clipboard: {{ writeText: async (t) => {{ copied.push(t); }} }} }};
const b3 = btn("acme/a#3");
const rc = fire("contextmenu", b3, {{ clientX: 40, clientY: 50 }});
const right = {{ prevented: rc.defaultPrevented, shown: !menu().hidden, role: menu().getAttribute("role"), label: menu().getAttribute("aria-label"),
  names: names(), roles: roles(), focus: at(), place: [menu().style.left, menu().style.top],
  github: [entries()[1].getAttribute("href"), entries()[1].getAttribute("target")] }};
key("Escape");
const escaped = {{ hidden: menu().hidden, back: document.activeElement === b3 }};

b3.focus();
const sf = fire("keydown", b3, {{ key: "F10", shiftKey: true }});
const keys = [sf.defaultPrevented && !menu().hidden];
key("ArrowDown"); keys.push(at());
key("ArrowUp"); key("ArrowUp"); keys.push(at() === entries().length - 1);
key("Tab"); keys.push(at());
key("Tab", {{ shiftKey: true }}); keys.push(at() === entries().length - 1);
key("Home"); keys.push(at());
key("s"); keys.push(document.activeElement.textContent);
key("Enter");
const inspected = {{ hidden: menu().hidden, open: q("#drawer").classList.contains("open"), id: q('#drawer [data-part="id"]').textContent,
  edit: document.activeElement.getAttribute("data-edit") }};
closeDrawer(false);

b3.focus();
fire("keydown", b3, {{ key: "ContextMenu" }});
pick("Copy reference");
await settle();
fire("keydown", b3, {{ key: "ContextMenu" }});
pick("Launch…");
const launched = {{ composer: !q("#composer").hidden, draft: q("#issues").value, focus: document.activeElement.id }};
closeComposer(false);

b3.focus();
fire("keydown", b3, {{ key: "ContextMenu" }});
pick("Blocked by a#1 (body)…");
const sub = {{ label: menu().getAttribute("aria-label"), names: names(), focus: at() }};
key("Escape");
const subBack = document.activeElement === b3;

const lane = document.querySelectorAll(".glt").find((t) => t.childNodes.some((n) => n.nodeType === 3 && n.data === "docs"));
fire("contextmenu", lane, {{ clientX: 5, clientY: 5 }});
const laneMenu = {{ label: menu().getAttribute("aria-label"), names: names() }};
pick("Go to first issue");
const laneGo = {{ hidden: menu().hidden, focus: document.activeElement.dataset.id }};
lane.focus();
fire("keydown", lane, {{ key: "F10", shiftKey: true }});
const shortcut = [key("l").defaultPrevented, key("/").defaultPrevented, menu().hidden];
pick("Copy lane name");
await settle();
return {{ right, escaped, keys, inspected, copied, launched, sub, subBack, laneMenu, laneGo, shortcut, laneBack: document.activeElement === lane }};
}}"##
        ),
    );
    assert_eq!(r["right"]["prevented"], true, "{r}");
    assert_eq!(r["right"]["shown"], true, "{r}");
    assert_eq!(r["right"]["role"], "menu", "{r}");
    assert_eq!(r["right"]["label"], "Actions for a#3", "{r}");
    assert_eq!(
        r["right"]["names"],
        json!([
            "Inspect",
            "Open on GitHub",
            "Launch…",
            "Add blocker…",
            "Set parent…",
            "Set track…",
            "Set priority…",
            "Copy reference",
            "Blocked by a#1 (body)…",
            "Lane docs…"
        ]),
        "an issue node lists only what applies to it: {r}"
    );
    assert_eq!(
        r["right"]["roles"],
        json!([
            "menuitem",
            "menuitem",
            "menuitem",
            "separator",
            "menuitem",
            "menuitem",
            "menuitem",
            "menuitem",
            "separator",
            "menuitem",
            "separator",
            "menuitem",
            "menuitem"
        ]),
        "{r}"
    );
    assert_eq!(r["right"]["focus"], 0, "the first item takes focus: {r}");
    assert_eq!(r["right"]["place"], json!(["40px", "50px"]), "{r}");
    assert_eq!(r["right"]["github"], json!(["https://github.com/acme/a/issues/3", "_blank"]), "{r}");
    assert_eq!(r["escaped"], json!({"hidden": true, "back": true}), "Escape closes and returns focus: {r}");
    assert_eq!(r["keys"], json!([true, 1, true, 0, true, 0, "Set parent…"]), "arrows wrap, Tab stays in the menu: {r}");
    assert_eq!(
        r["inspected"],
        json!({"hidden": true, "open": true, "id": "a#3", "edit": "parent"}),
        "Set parent opens the inspector on its parent field: {r}"
    );
    assert_eq!(r["copied"], json!(["acme/a#3", "docs"]), "{r}");
    assert_eq!(
        r["launched"],
        json!({"composer": true, "draft": "acme/a#3", "focus": "launch-btn"}),
        "Launch fills the launch form and waits on its button: {r}"
    );
    assert_eq!(
        r["sub"]["names"],
        json!(["Confirm: mark a#3 blocked by a#1 on GitHub", "Dismiss inferred edge", "Go to a#3", "Go to a#1"]),
        "{r}"
    );
    assert_eq!(r["sub"]["focus"], 0, "{r}");
    assert_eq!(r["subBack"], true, "a link menu opened from a node returns focus to the node: {r}");
    assert_eq!(r["laneMenu"], json!({"label": "Lane docs", "names": ["Go to first issue", "Copy lane name"]}), "{r}");
    assert_eq!(r["laneGo"], json!({"hidden": true, "focus": "acme/a#3"}), "{r}");
    assert_eq!(r["shortcut"], json!([true, true, false]), "board shortcuts stay out of an open menu: {r}");
    assert_eq!(r["laneBack"], true, "{r}");
}

#[tokio::test]
async fn an_inferred_edge_menu_runs_the_same_edit_as_the_inspector() {
    let (html, st) = two_gate_page().await;
    let r = run_page_async_with(
        &html,
        r#"globalThis.__storage = { "t3board.view": "queue" };"#,
        &format!(
            r##"async () => {{
{PAGE_HELPERS}
const base = {st};
{MENU_GRAPH}
const edits = () => fetches.filter((f) => f.url === "/api/flow/edge").map((f) => [f.method, JSON.parse(f.body)]);
const body = document.querySelectorAll(".ge").find((p) => p.getAttribute("data-source") === "body").nextSibling;
const native = document.querySelectorAll(".ge").find((p) => p.getAttribute("data-source") === "github").nextSibling;
fire("contextmenu", native, {{ clientX: 9, clientY: 9 }});
const github = names();
key("Escape");
const rc = fire("contextmenu", body, {{ clientX: 9, clientY: 9 }});
const opened = {{ prevented: rc.defaultPrevented, cls: body.getAttribute("class"), label: menu().getAttribute("aria-label"), names: names() }};
pick("Dismiss inferred edge");
await settle();
const fromMenu = edits();
btn("acme/a#3").dispatch("click");
q('#drawer [data-edge-act="dismiss"]').dispatch("click");
await settle();
closeDrawer(false);
fire("contextmenu", body, {{ clientX: 9, clientY: 9 }});
key("Escape");
const edgeBack = document.activeElement === body;
fire("contextmenu", body, {{ clientX: 9, clientY: 9 }});
pick("Go to a#1");
return {{ github, opened, fromMenu, both: edits(), edgeBack, went: document.activeElement.dataset.id }};
}}"##
        ),
    );
    assert_eq!(r["github"], json!(["Remove native GitHub link", "Go to a#2", "Go to a#1"]), "{r}");
    assert_eq!(r["opened"]["prevented"], true, "{r}");
    assert_eq!(r["opened"]["cls"], "geh", "{r}");
    assert_eq!(
        r["opened"]["names"],
        json!(["Confirm: mark a#3 blocked by a#1 on GitHub", "Dismiss inferred edge", "Go to a#3", "Go to a#1"]),
        "{r}"
    );
    let want = json!(["POST", {"action": "dismiss", "from": "acme/a#3", "to": "acme/a#1", "kind": "blockedBy"}]);
    assert_eq!(r["fromMenu"], json!([want.clone()]), "{r}");
    assert_eq!(r["both"], json!([want.clone(), want]), "the menu and the inspector send the same edit: {r}");
    assert_eq!(r["edgeBack"], true, "Escape returns focus to the right-clicked edge: {r}");
    assert_eq!(r["went"], "acme/a#1", "{r}");
}

#[tokio::test]
async fn the_context_menu_leaves_native_menus_alone_stays_on_screen_and_opens_on_long_press() {
    let (html, st) = two_gate_page().await;
    let r = run_page_async_with(
        &html,
        r#"globalThis.__storage = { "t3board.view": "queue" };"#,
        &format!(
            r##"async () => {{
{PAGE_HELPERS}
const base = {st};
{MENU_GRAPH}
const b1 = btn("acme/a#1");
const field = fire("contextmenu", q("#filter-q"), {{ clientX: 1, clientY: 1 }});
const onField = [field.defaultPrevented, menu().hidden];
globalThis.getSelection = () => ({{ isCollapsed: false, toString: () => "Issue 1", containsNode: () => true }});
const sel = fire("contextmenu", b1, {{ clientX: 1, clientY: 1 }});
const selKey = fire("keydown", b1, {{ key: "F10", shiftKey: true }});
const onSelection = [sel.defaultPrevented, selKey.defaultPrevented, menu().hidden];
globalThis.getSelection = undefined;

globalThis.innerWidth = 400;
globalThis.innerHeight = 300;
menu().rect = {{ left: 0, top: 0, right: 220, bottom: 180, width: 220, height: 180 }};
fire("contextmenu", b1, {{ clientX: 390, clientY: 290 }});
const clamped = [menu().style.left, menu().style.top];
fire("pointerdown", q("#filter-q"), {{ pointerType: "mouse", clientX: 1, clientY: 1 }});
const outside = menu().hidden;

fire("pointerdown", b1, {{ pointerType: "touch", clientX: 30, clientY: 40 }});
fire("pointermove", b1, {{ pointerType: "touch", clientX: 70, clientY: 40 }});
advance(600);
const moved = menu().hidden;
fire("pointerdown", b1, {{ pointerType: "touch", clientX: 30, clientY: 40 }});
advance(600);
const pressed = {{ shown: !menu().hidden, label: menu().getAttribute("aria-label"), place: [menu().style.left, menu().style.top] }};
fire("pointerup", b1, {{ pointerType: "touch" }});
const click = fire("click", b1, {{ button: 0 }});
return {{ onField, onSelection, clamped, outside, moved, pressed, swallowed: click.defaultPrevented, stillOpen: !menu().hidden }};
}}"##
        ),
    );
    assert_eq!(r["onField"], json!([false, true]), "a text field keeps the browser menu: {r}");
    assert_eq!(r["onSelection"], json!([false, false, true]), "selected text keeps the browser menu: {r}");
    assert_eq!(r["clamped"], json!(["172px", "110px"]), "the menu stays inside the viewport: {r}");
    assert_eq!(r["outside"], true, "a press outside closes it: {r}");
    assert_eq!(r["moved"], true, "a drag cancels the long press: {r}");
    assert_eq!(r["pressed"], json!({"shown": true, "label": "Actions for a#1", "place": ["30px", "40px"]}), "{r}");
    assert_eq!(r["swallowed"], true, "the click that ends a long press does not also open the node: {r}");
    assert_eq!(r["stillOpen"], true, "{r}");
}

#[test]
fn human_titles_drop_the_label_and_skip_urls_refs_and_slugs() {
    use t3_codemode::board::human_title;
    let cases = [
        ("widgets#41: Fix the grader", "widgets#41", Some("Fix the grader")),
        ("#287 PR 2/4: feat: enrich board titles", "#287 PR 2/4", Some("feat: enrich board titles")),
        ("other#9: Tidy the docs", "widgets#41", Some("Tidy the docs")),
        ("widgets#41: https://github.com/acme/widgets/issues/41", "widgets#41", None),
        ("widgets#41: merge-gate", "widgets#41", None),
        ("widgets#41: acme/widgets#41", "widgets#41", None),
        ("widgets#41", "widgets#41", None),
        ("", "widgets#41", None),
    ];
    for (title, label, want) in cases {
        assert_eq!(human_title(title, label), want, "{title:?} under {label:?}");
    }
}

#[test]
fn a_pr_verdict_reads_its_state_and_readiness() {
    use t3_codemode::board::pr_verdict;
    let pr = |state: &str, r: Value| {
        let mut p = json!({"number": 7, "url": "https://github.com/acme/widgets/pull/7", "state": state});
        if !r.is_null() {
            p["readiness"] = r;
        }
        p
    };
    let r = |draft: bool, merge: &str, review: Value, checks: Value| json!({"isDraft": draft, "mergeStateStatus": merge, "reviewDecision": review, "checks": checks});
    let cases = [
        (pr("merged", Value::Null), "merged"),
        (pr("closed", Value::Null), "closed"),
        (pr("queued", Value::Null), "queued"),
        (pr("open", Value::Null), "open"),
        (pr("open", r(false, "CLEAN", json!("APPROVED"), json!("passing"))), "ready"),
        (pr("open", r(false, "UNSTABLE", Value::Null, Value::Null)), "ready"),
        (pr("open", r(false, "BLOCKED", json!("APPROVED"), json!("failing"))), "failing"),
        (pr("open", r(true, "CLEAN", Value::Null, json!("passing"))), "draft"),
        (pr("open", r(false, "BLOCKED", json!("CHANGES_REQUESTED"), json!("passing"))), "changes"),
        (pr("open", r(false, "BLOCKED", Value::Null, json!("failing"))), "checksFailing"),
        (pr("open", r(false, "BLOCKED", Value::Null, json!("pending"))), "checksPending"),
        (pr("open", r(false, "DIRTY", Value::Null, json!("passing"))), "conflict"),
        (pr("open", r(false, "BEHIND", Value::Null, json!("passing"))), "behind"),
        (pr("open", r(false, "BLOCKED", json!("REVIEW_REQUIRED"), json!("passing"))), "review"),
    ];
    for (p, want) in cases {
        assert_eq!(pr_verdict(&p), Some(want), "{p}");
    }
    assert_eq!(pr_verdict(&Value::Null), None);
}

#[tokio::test]
async fn a_url_titled_ticket_is_served_with_its_issue_title() {
    let s = start(H::new()).await;
    s.h.fake.set_projects(vec![project("p-widgets", "/src/widgets")]);
    s.h.remotes.set("/src/widgets", "https://github.com/acme/widgets.git");
    s.h.github.set_issue("acme/widgets", t3_codemode::fake::github_issue(41, "Fix the grader", &[], T0));
    let url = "https://github.com/acme/widgets/issues/41";
    let r = result(&s.h.exec(&format!("async () => wf.launchTicket({{ issue: '{url}' }})")).await);
    assert_eq!(r["title"], format!("widgets#41: {url}"), "{r}");
    let mut th = thread("launch:1", "running", T0);
    th["title"] = json!(format!("widgets#41: {url}"));
    s.h.fake.put(th);
    let state =
        || async { s.http.get(format!("{}/api/state", s.base)).send().await.unwrap().json::<Value>().await.unwrap() };

    let st = state().await;
    assert_eq!((&st["tree"][0]["humanTitle"], &st["tree"][0]["issueTitle"]), (&Value::Null, &Value::Null), "{st}");

    s.h.app.wf.board_tick().await.unwrap();
    let st = state().await;
    let t = &st["tree"][0];
    assert_eq!(t["title"], format!("widgets#41: {url}"), "the thread title itself is unchanged: {t}");
    assert_eq!((&t["humanTitle"], &t["issueTitle"]), (&json!("Fix the grader"), &json!("Fix the grader")), "{t}");
    let reads = || s.h.github.operations().iter().filter(|o| *o == "issueNode").count();
    assert_eq!(reads(), 1);

    s.h.app.wf.board_tick().await.unwrap();
    assert_eq!(reads(), 1, "a known title is not read again");
    s.h.wf(
        "launch:1",
        "join",
        json!({"parent": BOARD, "label": "widgets#41", "role": "ticket", "issue": "acme/widgets#41"}),
    )
    .await;
    assert_eq!(state().await["tree"][0]["humanTitle"], "Fix the grader", "the joined ticket keeps it");
}

#[tokio::test]
async fn a_missing_issue_title_is_read_once_and_a_slug_title_falls_back_to_the_report() {
    let h = H::new();
    h.wf(TICKET, "join", json!({"parent": BOARD, "label": "widgets#42", "role": "ticket", "issue": "acme/widgets#42"}))
        .await;
    let mut th = thread(TICKET, "running", T0);
    th["title"] = json!("widgets#42: merge-gate");
    h.fake.put(th);
    h.app.wf.board_tick().await.unwrap();
    h.app.wf.board_tick().await.unwrap();
    let reads = h.github.operations().iter().filter(|o| *o == "issueNode").count();
    assert_eq!(reads, 1, "an issue GitHub cannot find is not read on every tick");
    assert_eq!(h.app.wf.tree().unwrap()[0]["humanTitle"], Value::Null);

    h.exec_as(BOSS, "async () => wf.issue({ url: 'acme/widgets#42', title: 'Tidy the merge gate' })").await;
    let t = &h.app.wf.tree().unwrap()[0];
    assert_eq!(
        (&t["humanTitle"], &t["issueTitle"]),
        (&json!("Tidy the merge gate"), &json!("Tidy the merge gate")),
        "{t}"
    );
}

const SECOND: &str = "mcp:worker-2";
const PR_HEAD: &str = "1111111111111111111111111111111111111111";

async fn two_step_ticket(h: &H) {
    h.wf(TICKET, "join", json!({"parent": BOARD, "label": "widgets#50", "role": "ticket", "issue": "acme/widgets#50"}))
        .await;
    h.wf(
        WORKER,
        "join",
        json!({"parent": TICKET, "label": "#50 PR 1/3", "role": "worker",
                                "issue": "acme/widgets#50", "part": "1/3"}),
    )
    .await;
    h.wf(
        SECOND,
        "join",
        json!({"parent": TICKET, "label": "#50 PR 2/3", "role": "worker",
                                "issue": "acme/widgets#282", "parentIssue": "acme/widgets#50", "part": "2/3"}),
    )
    .await;
    h.fake.set_prs(WORKER, vec![pull_request("acme/widgets", 6, "merged")]);
    h.fake.set_prs(SECOND, vec![pull_request("acme/widgets", 7, "open")]);
    let mut th = thread(SECOND, "running", T0);
    th["title"] = json!("#50 PR 2/3: feat: wire the grader");
    h.fake.put(th);
    h.tick(0).await;
}

#[tokio::test]
async fn workers_carry_their_step_pr_verdict_and_the_ticket_its_progress() {
    let h = H::new();
    two_step_ticket(&h).await;
    h.github.set_pr("acme/widgets", 7, t3_codemode::fake::github_pr(PR_HEAD, "CLEAN", false));
    h.github.update_pr("acme/widgets", 7, json!({"reviewDecision": "APPROVED"}));
    h.github.set_checks("acme/widgets", 7, json!([t3_codemode::fake::check_run("build", "SUCCESS")]));
    h.clock.set(&at(1));
    h.app.wf.reconcile().await.unwrap();

    let t = &h.app.wf.tree().unwrap()[0];
    assert_eq!(t["progress"], json!({"step": 2, "of": 3, "merged": 1}), "{t}");
    let w: Vec<&Value> = t["workers"].as_array().unwrap().iter().collect();
    assert_eq!((&w[0]["step"], &w[1]["step"]), (&json!({"n": 1, "of": 3}), &json!({"n": 2, "of": 3})));
    assert_eq!((&w[0]["pr"]["verdict"], &w[0]["pr"]["ready"]), (&json!("merged"), &json!(false)), "{}", w[0]);
    assert_eq!((&w[1]["pr"]["verdict"], &w[1]["pr"]["ready"]), (&json!("ready"), &json!(true)), "{}", w[1]);
    assert_eq!(w[1]["humanTitle"], "feat: wire the grader");
    assert_eq!(h.app.wf.snapshot().unwrap()["tree"][0]["progress"], t["progress"]);
}

#[tokio::test]
async fn a_new_issue_a_worker_already_covers_names_that_worker_and_pr() {
    let h = H::new();
    h.exec_as(BOSS, "async () => wf.issue({ url: 'acme/widgets#282', title: 'Seed 1080 race' })").await;
    h.exec_as(BOSS, "async () => wf.issue({ url: 'acme/widgets#283', title: 'Unrelated' })").await;
    two_step_ticket(&h).await;

    let snap = h.app.wf.snapshot().unwrap();
    let issues = snap["newIssues"].as_array().unwrap();
    let by = |n: u64| issues.iter().find(|i| i["number"] == n).unwrap()["coveredBy"].clone();
    let covered = by(282);
    assert_eq!(covered.as_array().unwrap().len(), 1, "{covered}");
    let c = &covered[0];
    assert_eq!(
        (&c["kind"], &c["ticket"], &c["worker"]),
        (&json!("pr"), &json!("widgets#50"), &json!("#50 PR 2/3")),
        "{c}"
    );
    assert_eq!((&c["thread"], &c["ticketThread"]), (&json!(SECOND), &json!(TICKET)));
    assert_eq!((&c["pr"]["repository"], &c["pr"]["number"]), (&json!("acme/widgets"), &json!(7)), "{c}");
    assert_eq!(by(283), json!([]));
    assert_eq!(snap["counts"]["newIssues"], 2, "covered issues stay on the list, marked");

    h.fake.set_prs(SECOND, vec![pull_request("acme/widgets", 7, "merged")]);
    h.tick(2).await;
    let snap = h.app.wf.snapshot().unwrap();
    let issue = snap["newIssues"].as_array().unwrap().iter().find(|i| i["number"] == 282).unwrap().clone();
    assert_eq!(issue["coveredBy"][0]["kind"], "merged", "{issue}");
}
