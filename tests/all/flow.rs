use crate::common::*;
use serde_json::{Value, json};
use t3_codemode::board::BOARD;
use t3_codemode::config::Config;
use t3_codemode::fake::{github_issue, project};
use t3_codemode::field;
use t3_codemode::store::Pending;

const REPO: &str = "acme/widgets";
const TOOLS: &str = "acme/tools";
const FLOW: &str = "flow";

fn flowing(max: usize) -> H {
    flowing_with(max, "ask")
}

fn flowing_with(max: usize, launch: &str) -> H {
    let cfg = Config::from_value(
        json!({"flow": {"repos": [REPO, TOOLS], "optIn": {"label": FLOW}, "maxTickets": max, "launch": launch}}),
    );
    let h = H::build(Vec::new(), cfg.unwrap());
    h.github.add_repo(REPO);
    h.github.add_repo(TOOLS);
    h.fake.set_projects(vec![project("p-widgets", "/src/widgets")]);
    h.remotes.set("/src/widgets", "git@github.com:acme/widgets.git");
    h
}

fn issue(h: &H, repo: &str, number: u64, labels: &[&str], day: u32) {
    let created = format!("2026-10-{day:02}T09:00:00Z");
    h.github.set_issue(repo, github_issue(number, &format!("Issue {number}"), labels, &created));
}

async fn flow(h: &H) -> Value {
    result(&h.exec("async () => wf.flow()").await)
}

async fn graph(h: &H) -> Value {
    h.app.wf.flow(true).await.unwrap()
}

fn labels(list: &Value) -> Vec<String> {
    list.as_array().unwrap().iter().map(|i| i["label"].as_str().unwrap().to_string()).collect()
}

async fn ticket(h: &H, thread: &str, label: &str) {
    h.fake.put(t3_codemode::fake::thread(thread, "running", T0));
    h.wf(thread, "join", json!({"parent": BOARD, "label": label, "role": "ticket"})).await;
}

#[tokio::test]
async fn the_queue_runs_by_priority_then_milestone_due_date_then_age() {
    let h = flowing(3);
    issue(&h, REPO, 1, &[FLOW], 1);
    issue(&h, REPO, 2, &[FLOW, "priority: low"], 2);
    issue(&h, REPO, 3, &[FLOW, "priority: high"], 9);
    issue(&h, TOOLS, 4, &[FLOW, "Priority: Medium"], 8);
    issue(&h, REPO, 5, &[FLOW, "priority: medium"], 3);
    issue(&h, REPO, 6, &[FLOW, "priority: medium"], 1);
    issue(&h, REPO, 7, &[FLOW, "priority: medium"], 2);
    h.github.set_milestone(REPO, 5, "v0.7.0", Some("2026-11-30T00:00:00Z"));
    h.github.set_milestone(TOOLS, 4, "v0.8.0", Some("2026-12-31T00:00:00Z"));
    h.github.set_milestone(REPO, 7, "someday", None);

    let f = flow(&h).await;

    assert_eq!(
        labels(&f["queue"]),
        ["widgets#3", "widgets#5", "tools#4", "widgets#6", "widgets#7", "widgets#2", "widgets#1"]
    );
    let first = &f["queue"][0];
    assert_eq!(first["repo"], REPO);
    assert_eq!(first["url"], "https://github.com/acme/widgets/issues/3");
    assert_eq!(first["priority"], "high");
    assert_eq!(f["queue"][1]["milestone"], json!({"title": "v0.7.0", "dueOn": "2026-11-30T00:00:00Z"}));
    assert_eq!(f["queue"][6]["priority"], Value::Null);
    assert_eq!(f["enabled"], true);
    assert_eq!(f["launch"], "ask");
}

#[tokio::test]
async fn blocked_external_ticketed_and_unlabelled_issues_stay_out_of_the_queue() {
    let h = flowing(3);
    issue(&h, REPO, 10, &[FLOW], 1);
    issue(&h, REPO, 11, &[FLOW, "priority: high"], 2);
    issue(&h, REPO, 12, &[FLOW, "blocked: external"], 3);
    issue(&h, REPO, 13, &["priority: high"], 4);
    issue(&h, REPO, 14, &[FLOW], 5);
    issue(&h, REPO, 15, &[FLOW], 6);
    issue(&h, REPO, 16, &[FLOW], 7);
    issue(&h, TOOLS, 17, &[FLOW], 8);
    issue(&h, REPO, 18, &[FLOW], 9);
    issue(&h, REPO, 19, &[FLOW], 10);
    h.github.set_blockers(REPO, 11, &[(REPO, 10), (TOOLS, 17)]);
    h.github.set_blockers(REPO, 18, &[(REPO, 19)]);
    h.github.close_issue(REPO, 19);
    h.github.set_parent(REPO, 15, (REPO, 14));
    h.github.set_parent(REPO, 16, (REPO, 10));
    ticket(&h, "mcp:t14", "widgets#14").await;

    let f = flow(&h).await;

    assert_eq!(labels(&f["queue"]), ["widgets#10", "widgets#16", "tools#17", "widgets#18"]);
    assert_eq!(f["queue"][3]["unblockedBy"][0]["label"], "widgets#19");
    assert_eq!(f["queue"][1]["parent"]["label"], "widgets#10");
    assert_eq!(labels(&f["blocked"]), ["widgets#11", "widgets#12"]);
    assert_eq!(labels(&f["blocked"][0]["blockers"]), ["widgets#10", "tools#17"]);
    assert_eq!(f["blocked"][0]["external"], false);
    assert_eq!(f["blocked"][1]["external"], true);
    assert_eq!(f["blocked"][1]["blockers"], json!([]));
    assert_eq!(labels(&f["running"]), ["widgets#14"]);
}

#[tokio::test]
async fn free_slots_are_max_tickets_minus_open_flow_tickets() {
    let h = flowing(3);
    issue(&h, REPO, 1, &[FLOW], 1);
    ticket(&h, "mcp:t1", "widgets#1").await;
    ticket(&h, "mcp:t2", "tools#2").await;
    ticket(&h, "mcp:t3", "gadgets#3").await;
    ticket(&h, "mcp:t4", "widgets#4").await;
    h.wf("mcp:t4", "send", json!({"kind": "handoff", "body": "done"})).await;

    let f = flow(&h).await;

    assert_eq!(f["maxTickets"], 3);
    assert_eq!(f["free"], 1);
    assert_eq!(f["queue"], json!([]));
    let running = f["running"].as_array().unwrap();
    assert_eq!(labels(&f["running"]), ["widgets#1", "tools#2"]);
    assert_eq!(running[0]["thread"], "mcp:t1");
    assert_eq!(running[0]["repo"], REPO);
    assert_eq!(running[1]["url"], "https://github.com/acme/tools/issues/2");
}

#[tokio::test]
async fn a_launch_not_yet_joined_takes_a_slot_and_free_slots_never_go_below_zero() {
    let h = flowing(1);
    issue(&h, TOOLS, 9, &[FLOW], 1);
    ticket(&h, "mcp:t1", "widgets#1").await;
    let params = h.app.wf.config().link_params("overseer", "ticket").unwrap();
    let pending = Pending {
        parent: BOARD.into(),
        label: "tools#9".into(),
        role: "ticket".into(),
        params,
        at: T0.into(),
        reported: false,
        thread: None,
        link: None,
        project: None,
        identity: Default::default(),
    };
    h.app.store.pending_put(&pending).unwrap();

    let f = flow(&h).await;

    assert_eq!(f["free"], 0);
    assert_eq!(labels(&f["running"]), ["widgets#1", "tools#9"]);
    assert_eq!(f["running"][1]["thread"], Value::Null);
    assert_eq!(f["queue"], json!([]));
}

#[tokio::test]
async fn the_queue_reads_every_page_of_a_repo() {
    let h = flowing(2);
    for n in 1..=5 {
        issue(&h, REPO, n, &[FLOW], n as u32);
    }
    h.github.set_issue_page(2);

    let f = flow(&h).await;

    assert_eq!(labels(&f["queue"]), ["widgets#1", "widgets#2", "widgets#3", "widgets#4", "widgets#5"]);
    let reads: Vec<Value> = h.github.calls().into_iter().filter(|c| c["operation"] == "flowIssues").collect();
    assert_eq!(reads.len(), 4);
    assert_eq!(reads[0]["variables"]["label"], FLOW);
}

fn node<'a>(f: &'a Value, id: &str) -> &'a Value {
    f["graph"]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["id"] == id)
        .unwrap_or_else(|| panic!("no node {id}: {f}"))
}

fn ids(list: &Value) -> Vec<String> {
    list.as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect()
}

async fn gadget_ticket(h: &H) {
    h.fake.set_projects(vec![project("p-widgets", "/src/widgets"), project("p-gadgets", "/src/gadgets")]);
    h.remotes.set("/src/gadgets", "git@github.com:acme/gadgets.git");
    h.github.set_issue("acme/gadgets", github_issue(3, "Gadget work", &[], "2026-10-20T09:00:00Z"));
    let mut th = t3_codemode::fake::thread("mcp:g3", "running", T0);
    th["projectId"] = json!("p-gadgets");
    h.fake.put(th);
    h.wf("mcp:g3", "join", json!({"parent": BOARD, "label": "gadgets#3", "role": "ticket"})).await;
    h.app.wf.board_tick().await.unwrap();
}

#[tokio::test]
async fn the_graph_covers_flow_issues_and_active_tickets_with_their_edges() {
    let h = flowing(4);
    issue(&h, REPO, 1, &[FLOW, "track: board"], 1);
    issue(&h, REPO, 2, &[FLOW], 2);
    issue(&h, REPO, 3, &[], 3);
    issue(&h, REPO, 4, &[FLOW], 4);
    issue(&h, REPO, 5, &[], 5);
    issue(&h, REPO, 6, &["priority: high"], 6);
    issue(&h, TOOLS, 7, &[], 7);
    issue(&h, REPO, 10, &[FLOW], 10);
    issue(&h, REPO, 11, &[FLOW], 11);
    h.github.set_blockers(REPO, 2, &[(REPO, 1), (REPO, 3)]);
    h.github.close_issue(REPO, 3);
    h.github.set_parent(REPO, 4, (REPO, 5));
    h.github.set_milestone(REPO, 4, "v0.9.0", None);
    h.github.set_blockers(REPO, 6, &[(TOOLS, 7)]);
    h.github.set_parent(REPO, 11, (REPO, 10));
    ticket(&h, "mcp:t6", "widgets#6").await;
    gadget_ticket(&h).await;
    h.github.set_blockers("acme/gadgets", 3, &[(REPO, 11)]);
    let before = h.github.calls().len();

    let f = graph(&h).await;

    let ops: Vec<String> = h.github.operations()[before..].to_vec();
    assert_eq!(ops, ["flowIssues", "flowIssues", "flowTickets"]);
    let g = &f["graph"];
    let all: Vec<&str> = g["nodes"].as_array().unwrap().iter().map(|n| n["id"].as_str().unwrap()).collect();
    assert_eq!(
        all,
        [
            "acme/gadgets#3",
            "acme/tools#7",
            "acme/widgets#1",
            "acme/widgets#2",
            "acme/widgets#3",
            "acme/widgets#4",
            "acme/widgets#5",
            "acme/widgets#6",
            "acme/widgets#10",
            "acme/widgets#11"
        ]
    );
    assert_eq!(
        g["edges"],
        json!([
            {"from": "acme/gadgets#3", "to": "acme/widgets#11", "kind": "blockedBy"},
            {"from": "acme/widgets#2", "to": "acme/widgets#1", "kind": "blockedBy"},
            {"from": "acme/widgets#2", "to": "acme/widgets#3", "kind": "blockedBy"},
            {"from": "acme/widgets#4", "to": "acme/widgets#5", "kind": "parent"},
            {"from": "acme/widgets#6", "to": "acme/tools#7", "kind": "blockedBy"},
            {"from": "acme/widgets#11", "to": "acme/widgets#10", "kind": "parent"},
        ])
    );

    let one = node(&f, "acme/widgets#1");
    assert_eq!(one["tracked"], true);
    assert_eq!(one["flowList"], "queue");
    assert_eq!(one["ready"], true);
    assert_eq!(one["external"], false);
    assert_eq!(ids(&one["unlocks"]), ["acme/widgets#2"]);
    assert_eq!((one["track"].as_str(), one["trackBy"].as_str()), (Some("board"), Some("label")));
    assert_eq!(one["ticket"], Value::Null);

    let two = node(&f, "acme/widgets#2");
    assert_eq!(two["flowList"], "blocked");
    assert_eq!((two["ready"].as_bool(), two["external"].as_bool()), (Some(false), Some(false)));
    assert_eq!(ids(&two["blockedBy"]), ["acme/widgets#1", "acme/widgets#3"]);
    assert_eq!((two["track"].as_str(), two["trackBy"].as_str()), (Some("board"), Some("component")));

    let three = node(&f, "acme/widgets#3");
    assert_eq!(three["tracked"], false);
    assert_eq!(three["state"], "closed");
    assert_eq!(three["title"], "Issue 3");
    assert_eq!(three["url"], "https://github.com/acme/widgets/issues/3");

    let four = node(&f, "acme/widgets#4");
    assert_eq!(four["parent"], "acme/widgets#5");
    assert_eq!(four["milestone"]["title"], "v0.9.0");
    assert_eq!((four["track"].as_str(), four["trackBy"].as_str()), (Some("v0.9.0"), Some("milestone")));
    let five = node(&f, "acme/widgets#5");
    assert_eq!((five["tracked"].as_bool(), five["state"].as_str()), (Some(false), Some("open")));
    assert_eq!(ids(&five["subIssues"]), ["acme/widgets#4"]);
    assert_eq!((five["track"].as_str(), five["trackBy"].as_str()), (Some("v0.9.0"), Some("component")));

    let six = node(&f, "acme/widgets#6");
    assert_eq!(six["tracked"], true);
    assert_eq!(six["flowList"], Value::Null);
    assert_eq!(six["priority"], "high");
    assert_eq!(six["ticket"]["thread"], "mcp:t6");
    assert_eq!(six["ticket"]["status"], "running");
    assert_eq!(six["external"], false);
    assert_eq!((six["track"].as_str(), six["trackBy"].as_str()), (Some("tools#7"), Some("component")));

    let ten = node(&f, "acme/widgets#10");
    assert_eq!(ids(&ten["subIssues"]), ["acme/widgets#11"]);
    assert_eq!(node(&f, "acme/widgets#11")["parent"], "acme/widgets#10");
    assert_eq!(ids(&node(&f, "acme/widgets#11")["unlocks"]), ["acme/gadgets#3"]);

    let gadget = node(&f, "acme/gadgets#3");
    assert_eq!(gadget["title"], "Gadget work");
    assert_eq!(gadget["ticket"]["thread"], "mcp:g3");
    assert_eq!(gadget["tracked"], true);
}

#[tokio::test]
async fn a_node_is_external_by_label_or_when_every_open_blocker_is_outside_the_tracked_repos() {
    let h = flowing(4);
    issue(&h, REPO, 1, &[FLOW, "Blocked: External"], 1);
    issue(&h, REPO, 2, &[FLOW], 2);
    issue(&h, REPO, 3, &[FLOW], 3);
    issue(&h, REPO, 4, &[FLOW], 4);
    issue(&h, REPO, 5, &["priority: low"], 5);
    h.github.set_blockers(REPO, 2, &[("acme/outside", 9)]);
    h.github.set_blockers(REPO, 3, &[("acme/outside", 9), (TOOLS, 7)]);
    h.github.set_issue(TOOLS, github_issue(8, "Tool fix", &[], "2026-10-01T09:00:00Z"));
    h.github.set_blockers(REPO, 4, &[(TOOLS, 8)]);
    h.github.close_issue(TOOLS, 8);
    h.github.set_blockers(REPO, 5, &[("acme/outside", 9)]);
    ticket(&h, "mcp:t5", "widgets#5").await;

    let f = graph(&h).await;

    let ext = |id: &str| (node(&f, id)["external"].as_bool().unwrap(), node(&f, id)["ready"].as_bool().unwrap());
    assert_eq!(ext("acme/widgets#1"), (true, false));
    assert_eq!(ext("acme/widgets#2"), (true, false));
    assert_eq!(ext("acme/widgets#3"), (false, false));
    assert_eq!(ext("acme/widgets#4"), (false, true));
    assert_eq!(ext("acme/widgets#5"), (true, false));
    assert_eq!(node(&f, "acme/widgets#5")["ticket"]["thread"], "mcp:t5");
    assert_eq!(node(&f, "acme/outside#9")["tracked"], false);
    assert_eq!(f["blocked"].as_array().unwrap().iter().find(|b| b["label"] == "widgets#2").unwrap()["external"], false);
}

#[tokio::test]
async fn the_ticket_reads_share_one_query_per_fifty_tickets() {
    let h = flowing(80);
    for n in 1..=60 {
        issue(&h, REPO, n, &[], 1);
        ticket(&h, &format!("mcp:t{n}"), &format!("widgets#{n}")).await;
    }
    let before = h.github.calls().len();

    let f = graph(&h).await;

    let ops: Vec<String> = h.github.operations()[before..].to_vec();
    assert_eq!(ops, ["flowIssues", "flowIssues", "flowTickets", "flowTickets"]);
    assert_eq!(f["graph"]["nodes"].as_array().unwrap().len(), 60);
    let missing: Vec<&Value> =
        f["graph"]["nodes"].as_array().unwrap().iter().filter(|n| !n["ticket"]["thread"].is_string()).collect();
    assert!(missing.is_empty(), "{missing:?}");
}

#[tokio::test]
async fn with_no_flow_block_the_graph_holds_the_active_tickets_and_reads_nothing() {
    let h = H::new();
    h.remotes.set("/src/widgets", "git@github.com:acme/widgets.git");
    h.fake.set_projects(vec![project("p-widgets", "/src/widgets")]);
    let mut th = t3_codemode::fake::thread("mcp:t1", "running", T0);
    th["projectId"] = json!("p-widgets");
    h.fake.put(th);
    h.wf("mcp:t1", "join", json!({"parent": BOARD, "label": "widgets#1", "role": "ticket"})).await;
    h.app.wf.board_tick().await.unwrap();
    let before = h.github.calls().len();

    let f = graph(&h).await;

    assert_eq!(h.github.calls().len(), before);
    assert_eq!(f["enabled"], false);
    let n = node(&f, "acme/widgets#1");
    assert_eq!(n["tracked"], true);
    assert_eq!(n["title"], Value::Null);
    assert_eq!(n["ticket"]["thread"], "mcp:t1");
    assert_eq!((n["track"].as_str(), n["trackBy"].as_str()), (Some("widgets#1"), Some("component")));
    assert_eq!(f["graph"]["edges"], json!([]));
}

#[tokio::test]
async fn with_no_flow_block_the_flow_is_off_and_reads_nothing() {
    let h = H::new();

    let f = flow(&h).await;

    assert_eq!(
        f,
        json!({"enabled": false, "launch": null, "maxTickets": 0, "free": 0, "running": [], "queue": [], "blocked": [],
               "skipped": []})
    );
    assert_eq!(graph(&h).await["graph"], json!({"nodes": [], "edges": []}));
    assert!(h.github.calls().is_empty());
}

#[tokio::test]
async fn a_repo_github_cannot_read_fails_the_call() {
    let cfg = Config::from_value(json!({"flow": {"repos": [REPO, TOOLS], "optIn": {"label": FLOW}, "maxTickets": 2}}));
    let h = H::build(Vec::new(), cfg.unwrap());
    issue(&h, REPO, 1, &[FLOW], 1);

    let out = h.exec("async () => wf.flow()").await;

    assert!(err_text(&out).contains("reading acme/tools's issues failed"), "{}", out.text);
    h.github.add_repo(TOOLS);
    h.github.set_down(true);
    let out = h.exec("async () => wf.flow()").await;
    assert!(err_text(&out).contains("not logged in"), "{}", out.text);
}

#[tokio::test]
async fn wf_flow_is_in_the_catalog_with_its_signature() {
    let h = H::new();
    let out = h.app.search(r#"async () => catalog.find(t => t.name === "wf.flow").signature"#).await;
    let sig = ok_json(&out)["result"].as_str().unwrap().to_string();
    assert!(sig.contains("wf.flow(input?: { graph?: boolean }): Promise<{ enabled: boolean;"), "{sig}");
    assert!(sig.contains("graph?: { nodes:"), "{sig}");
    assert!(sig.contains("queue:"), "{sig}");
}

fn config_error(flow: Value) -> (String, String) {
    let e = Config::from_value(json!({ "flow": flow })).unwrap_err();
    (field::of(&e).to_string(), format!("{e:#}"))
}

#[test]
fn the_flow_block_is_validated() {
    let ok = json!({"repos": [REPO], "optIn": {"label": FLOW}, "maxTickets": 2});
    let cfg = Config::from_value(json!({ "flow": ok })).unwrap();
    let flow = cfg.flow.unwrap();
    assert_eq!(flow.launch, t3_codemode::config::Launch::Ask);
    assert_eq!(flow.max_tickets, 2);

    let auto = Config::from_value(
        json!({"flow": {"repos": [REPO], "optIn": {"label": FLOW}, "maxTickets": 1, "launch": "auto"}}),
    );
    assert_eq!(auto.unwrap().flow.unwrap().launch, t3_codemode::config::Launch::Auto);

    let (at, msg) = config_error(json!({"repos": [], "optIn": {"label": FLOW}, "maxTickets": 1}));
    assert_eq!(at, "flow.repos");
    assert!(msg.contains("at least one"), "{msg}");
    let (at, msg) = config_error(json!({"repos": [REPO, "widgets"], "optIn": {"label": FLOW}, "maxTickets": 1}));
    assert_eq!(at, "flow.repos.1");
    assert!(msg.contains("not an owner/name"), "{msg}");
    let (at, msg) = config_error(json!({"repos": [REPO, "Acme/Widgets"], "optIn": {"label": FLOW}, "maxTickets": 1}));
    assert_eq!(at, "flow.repos.1");
    assert!(msg.contains("twice"), "{msg}");
    let (at, msg) = config_error(json!({"repos": [REPO, "other/widgets"], "optIn": {"label": FLOW}, "maxTickets": 1}));
    assert_eq!(at, "flow.repos.1");
    assert!(msg.contains("share the name widgets"), "{msg}");
    let (at, _) = config_error(json!({"repos": [REPO], "optIn": {"label": " "}, "maxTickets": 1}));
    assert_eq!(at, "flow.optIn.label");
    let (at, _) = config_error(json!({"repos": [REPO], "optIn": {"label": FLOW}, "maxTickets": 0}));
    assert_eq!(at, "flow.maxTickets");
    let (_, msg) = config_error(json!({"repos": [REPO], "optIn": {"label": FLOW}, "maxTickets": 1, "launch": "now"}));
    assert!(msg.contains("now"), "{msg}");
    let (_, msg) = config_error(json!({"repos": [REPO], "optIn": {"label": FLOW}, "maxTickets": 1, "project": 3}));
    assert!(msg.contains("project"), "{msg}");
    let (_, msg) = config_error(json!({"repos": [REPO], "maxTickets": 1}));
    assert!(msg.contains("optIn"), "{msg}");
}

fn flow_cards(h: &H) -> Vec<Value> {
    h.app
        .wf
        .cards()
        .unwrap()
        .into_iter()
        .map(|c| serde_json::to_value(c).unwrap())
        .filter(|c| c["kind"] == "flow")
        .collect()
}

async fn next(h: &H) -> Value {
    result(&h.exec("async () => wf.flow.next()").await)
}

async fn reconcile(h: &H) {
    h.app.wf.reconcile().await.unwrap();
}

#[tokio::test]
async fn in_ask_mode_a_handoff_raises_a_launch_card_for_the_top_ready_issue() {
    let h = flowing(2);
    issue(&h, REPO, 1, &[FLOW], 1);
    issue(&h, REPO, 2, &[FLOW], 2);
    issue(&h, REPO, 3, &[FLOW, "priority: low"], 3);
    issue(&h, REPO, 4, &[FLOW, "priority: high"], 4);
    issue(&h, REPO, 9, &[], 5);
    h.github.set_blockers(REPO, 4, &[(REPO, 9)]);
    h.github.close_issue(REPO, 9);
    ticket(&h, "mcp:t1", "widgets#1").await;
    ticket(&h, "mcp:t2", "widgets#2").await;

    reconcile(&h).await;
    assert!(flow_cards(&h).is_empty());
    h.wf("mcp:t1", "send", json!({"kind": "handoff", "body": "done"})).await;

    let c = flow_cards(&h);
    assert_eq!(c.len(), 1);
    assert_eq!(
        c[0]["text"],
        "Launch widgets#4 next? (priority: high, unblocked by #9), here: https://github.com/acme/widgets/issues/4"
    );
    assert_eq!(c[0]["options"], json!(["launch", "skip"]));
    assert_eq!(c[0]["label"], "widgets#4");
    assert_eq!(c[0]["ticket"], Value::Null);
    assert_eq!(c[0]["flow"]["url"], "https://github.com/acme/widgets/issues/4");
    reconcile(&h).await;
    assert_eq!(flow_cards(&h).len(), 1);
    assert!(h.fake.launched().is_empty());

    let id = c[0]["id"].as_str().unwrap();
    let r = h.app.wf.answer(id, "launch").await.unwrap();
    assert_eq!(r["launched"]["ok"], true);
    assert_eq!(h.fake.launched()[0]["title"], "widgets#4: Issue 4");
    assert!(flow_cards(&h).is_empty());
    reconcile(&h).await;
    assert!(flow_cards(&h).is_empty());
    let f = flow(&h).await;
    assert_eq!(f["free"], 0);
    assert_eq!(labels(&f["queue"]), ["widgets#3"]);
    assert_eq!(labels(&f["skipped"]), ["widgets#1"]);
}

#[tokio::test]
async fn in_auto_mode_a_handoff_launches_the_next_issue_and_posts_a_feed_event() {
    let h = flowing_with(1, "auto");
    issue(&h, REPO, 1, &[FLOW], 1);
    issue(&h, REPO, 2, &[FLOW, "priority: high"], 2);
    issue(&h, REPO, 3, &[FLOW], 3);
    ticket(&h, "mcp:t1", "widgets#1").await;

    h.wf("mcp:t1", "send", json!({"kind": "handoff", "body": "done"})).await;

    let launched = h.fake.launched();
    assert_eq!(launched.len(), 1);
    assert_eq!(launched[0]["title"], "widgets#2: Issue 2");
    assert_eq!(launched[0]["projectId"], "p-widgets");
    assert!(flow_cards(&h).is_empty());
    let feed = h.app.wf.feed(10).unwrap();
    assert!(
        feed.iter().any(|m| m["kind"] == "event" && m["head"] == "Flow launched widgets#2 (priority: high)"),
        "{feed:?}"
    );
    reconcile(&h).await;
    assert_eq!(h.fake.launched().len(), 1);
}

#[tokio::test]
async fn an_auto_launch_that_fails_skips_the_issue_instead_of_retrying_it() {
    let h = flowing_with(1, "auto");
    issue(&h, TOOLS, 5, &[FLOW], 1);

    let r = next(&h).await;

    assert_eq!(r["picked"][0]["launched"]["ok"], false);
    assert!(h.fake.launched().is_empty());
    let f = flow(&h).await;
    assert_eq!(labels(&f["skipped"]), ["tools#5"]);
    assert_eq!(next(&h).await["reason"], "no ready issue to launch");
}

#[tokio::test]
async fn with_launch_off_nothing_launches_and_nothing_is_read() {
    let h = flowing_with(1, "off");
    issue(&h, REPO, 1, &[FLOW], 1);
    ticket(&h, "mcp:t1", "widgets#1").await;

    h.wf("mcp:t1", "send", json!({"kind": "handoff", "body": "done"})).await;
    let r = next(&h).await;

    assert_eq!(r["launch"], "off");
    assert_eq!(r["picked"], json!([]));
    assert!(r["reason"].as_str().unwrap().contains("off"), "{r}");
    assert!(flow_cards(&h).is_empty());
    assert!(h.fake.launched().is_empty());
    assert!(h.github.calls().is_empty());
}

#[tokio::test]
async fn an_issue_becomes_ready_on_the_next_reconcile_tick_after_its_last_blocker_closes() {
    let h = flowing(1);
    issue(&h, REPO, 5, &[FLOW], 1);
    issue(&h, REPO, 6, &[], 2);
    issue(&h, TOOLS, 7, &[], 3);
    h.github.set_blockers(REPO, 5, &[(REPO, 6), (TOOLS, 7)]);

    reconcile(&h).await;
    assert!(flow_cards(&h).is_empty());
    h.github.close_issue(REPO, 6);
    reconcile(&h).await;
    assert!(flow_cards(&h).is_empty());
    h.github.close_issue(TOOLS, 7);
    reconcile(&h).await;

    let c = flow_cards(&h);
    assert_eq!(c.len(), 1);
    assert_eq!(c[0]["label"], "widgets#5");
    assert!(c[0]["text"].as_str().unwrap().contains("(priority: none, unblocked by #6, tools#7)"), "{}", c[0]["text"]);
}

#[tokio::test]
async fn a_flow_tickets_issue_closing_opens_its_slot() {
    let h = flowing(1);
    issue(&h, REPO, 1, &[FLOW], 1);
    issue(&h, REPO, 2, &[FLOW], 2);
    ticket(&h, "mcp:t1", "widgets#1").await;

    reconcile(&h).await;
    assert!(flow_cards(&h).is_empty());
    h.github.close_issue(REPO, 1);
    reconcile(&h).await;

    assert_eq!(labels(&json!(flow_cards(&h))), ["widgets#2"]);
    let f = flow(&h).await;
    assert_eq!(f["running"], json!([]));
    assert_eq!(f["free"], 1);
}

#[tokio::test]
async fn a_skipped_issue_stays_out_of_the_picks_until_its_labels_or_blockers_change() {
    let h = flowing(1);
    issue(&h, REPO, 1, &[FLOW, "priority: high"], 1);
    issue(&h, REPO, 2, &[FLOW], 2);
    issue(&h, REPO, 3, &[], 3);

    let first = next(&h).await;
    let id = first["picked"][0]["card"].as_str().unwrap().to_string();
    assert_eq!(first["picked"][0]["label"], "widgets#1");
    let r = h.app.wf.answer(&id, "skip").await.unwrap();

    assert_eq!(r["skipped"], "widgets#1");
    assert_eq!(r["next"]["picked"][0]["label"], "widgets#2");
    assert_eq!(labels(&json!(flow_cards(&h))), ["widgets#2"]);
    let f = flow(&h).await;
    assert_eq!(labels(&f["skipped"]), ["widgets#1"]);
    assert_eq!(labels(&f["queue"]), ["widgets#2"]);
    let two = flow_cards(&h)[0]["id"].as_str().unwrap().to_string();
    h.app.wf.answer(&two, "skip").await.unwrap();
    reconcile(&h).await;
    assert!(flow_cards(&h).is_empty());

    h.github.update_issue(REPO, 1, json!({"labels": {"nodes": [{"name": FLOW}, {"name": "priority: medium"}]}}));
    h.github.set_blockers(REPO, 2, &[(REPO, 3)]);
    h.github.close_issue(REPO, 3);
    let f = flow(&h).await;
    assert_eq!(labels(&f["queue"]), ["widgets#1", "widgets#2"]);
    assert_eq!(f["skipped"], json!([]));
    reconcile(&h).await;
    h.github.update_issue(REPO, 1, json!({"labels": {"nodes": [{"name": FLOW}, {"name": "priority: high"}]}}));
    assert_eq!(labels(&flow(&h).await["queue"]), ["widgets#1", "widgets#2"]);
}

#[tokio::test]
async fn wf_flow_next_runs_one_pick_and_open_cards_hold_their_slots() {
    let h = flowing(3);
    for n in 1..=4 {
        issue(&h, REPO, n, &[FLOW], n as u32);
    }

    assert_eq!(next(&h).await["picked"][0]["label"], "widgets#1");
    let second = next(&h).await;
    assert_eq!(second["picked"][0]["label"], "widgets#2");
    assert_eq!(second["offered"][0]["label"], "widgets#1");
    reconcile(&h).await;
    assert_eq!(labels(&json!(flow_cards(&h))), ["widgets#1", "widgets#2", "widgets#3"]);
    let full = next(&h).await;
    assert_eq!(full["picked"], json!([]));
    assert_eq!(full["reason"], "no free ticket slot");

    h.github.close_issue(REPO, 2);
    reconcile(&h).await;
    assert_eq!(labels(&json!(flow_cards(&h))), ["widgets#1", "widgets#3", "widgets#4"]);
    let resolved = h.app.wf.cards_with(true).unwrap();
    let gone = resolved.iter().find(|c| c.label.as_deref() == Some("widgets#2")).unwrap();
    assert_eq!(gone.resolution.as_deref(), Some("no longer ready"));
}

#[tokio::test]
async fn a_flow_card_takes_only_launch_or_skip() {
    let h = flowing(1);
    issue(&h, REPO, 1, &[FLOW], 1);
    let id = next(&h).await["picked"][0]["card"].as_str().unwrap().to_string();

    let e = h.app.wf.answer(&id, "maybe").await.unwrap_err();

    assert!(format!("{e:#}").contains("takes \"launch\" or \"skip\""), "{e:#}");
    assert_eq!(flow_cards(&h).len(), 1);
}

#[tokio::test]
async fn wf_flow_next_is_in_the_catalog() {
    let h = H::new();
    let out = h.app.search(r#"async () => catalog.find(t => t.name === "wf.flow.next").signature"#).await;
    let sig = ok_json(&out)["result"].as_str().unwrap().to_string();
    assert!(sig.contains("wf.flow.next(): Promise<{ launch:"), "{sig}");
}

#[tokio::test]
async fn a_launch_card_for_an_issue_no_longer_ready_launches_nothing() {
    let h = flowing(1);
    issue(&h, REPO, 1, &[FLOW], 1);
    let id = next(&h).await["picked"][0]["card"].as_str().unwrap().to_string();
    h.github.close_issue(REPO, 1);

    let e = h.app.wf.answer(&id, "launch").await.unwrap_err();

    assert!(format!("{e:#}").contains("widgets#1 is no longer ready"), "{e:#}");
    assert!(h.fake.launched().is_empty());
    assert!(flow_cards(&h).is_empty());
}

#[tokio::test]
async fn a_card_whose_issue_launched_elsewhere_no_longer_holds_a_slot() {
    let h = flowing(2);
    issue(&h, REPO, 1, &[FLOW], 1);
    issue(&h, REPO, 2, &[FLOW], 2);
    reconcile(&h).await;
    let cards = flow_cards(&h);
    assert_eq!(labels(&json!(cards)), ["widgets#1", "widgets#2"]);
    result(&h.exec("async () => wf.launchTicket({ issue: 'acme/widgets#1' })").await);

    let r = h.app.wf.answer(cards[1]["id"].as_str().unwrap(), "launch").await.unwrap();

    assert_eq!(r["launched"]["ok"], true);
    assert_eq!(h.fake.launched().len(), 2);
}

#[tokio::test]
async fn a_skip_expires_when_a_blocker_reopens_and_closes_again() {
    let h = flowing(1);
    issue(&h, REPO, 5, &[FLOW], 1);
    issue(&h, REPO, 6, &[], 2);
    h.github.set_blockers(REPO, 5, &[(REPO, 6)]);
    h.github.close_issue(REPO, 6);
    let id = next(&h).await["picked"][0]["card"].as_str().unwrap().to_string();
    h.app.wf.answer(&id, "skip").await.unwrap();

    h.github.update_issue(REPO, 6, json!({"state": "OPEN"}));
    reconcile(&h).await;
    h.github.close_issue(REPO, 6);

    assert_eq!(labels(&flow(&h).await["queue"]), ["widgets#5"]);
}

#[tokio::test]
async fn a_ticket_that_finishes_while_its_issue_is_blocked_frees_it_once_the_blocker_closes() {
    let h = flowing(1);
    issue(&h, REPO, 1, &[FLOW], 1);
    issue(&h, REPO, 2, &[], 2);
    h.github.set_blockers(REPO, 1, &[(REPO, 2)]);
    ticket(&h, "mcp:t1", "widgets#1").await;

    h.wf("mcp:t1", "send", json!({"kind": "handoff", "body": "done"})).await;
    h.github.close_issue(REPO, 2);

    let f = flow(&h).await;
    assert_eq!(labels(&f["queue"]), ["widgets#1"]);
    assert_eq!(f["skipped"], json!([]));
}

#[tokio::test]
async fn with_more_cards_than_slots_the_oldest_ready_card_keeps_the_slot() {
    let h = flowing(2);
    for n in 1..=3 {
        issue(&h, REPO, n, &[FLOW], n as u32);
    }
    reconcile(&h).await;
    let cards = flow_cards(&h);
    assert_eq!(labels(&json!(cards)), ["widgets#1", "widgets#2"]);
    result(&h.exec("async () => wf.launchTicket({ issue: 'acme/widgets#3' })").await);

    let r = h.app.wf.answer(cards[0]["id"].as_str().unwrap(), "launch").await.unwrap();

    assert_eq!(r["launched"]["ok"], true);
    assert_eq!(h.fake.launched().len(), 2);
}

#[tokio::test]
async fn switching_to_auto_retires_the_open_launch_cards_and_launches() {
    let mut h = flowing(1);
    issue(&h, REPO, 1, &[FLOW], 1);
    next(&h).await;
    assert_eq!(flow_cards(&h).len(), 1);
    h.config = Config::from_value(
        json!({"flow": {"repos": [REPO, TOOLS], "optIn": {"label": FLOW}, "maxTickets": 1, "launch": "auto"}}),
    )
    .unwrap();
    h.reopen();

    reconcile(&h).await;

    assert!(flow_cards(&h).is_empty());
    assert_eq!(h.fake.launched()[0]["title"], "widgets#1: Issue 1");
}

#[tokio::test]
async fn an_auto_pick_does_not_launch_a_sub_issue_after_launching_its_parent() {
    let h = flowing_with(2, "auto");
    issue(&h, REPO, 1, &[FLOW], 1);
    issue(&h, REPO, 2, &[FLOW], 2);
    h.github.set_parent(REPO, 2, (REPO, 1));

    reconcile(&h).await;

    let launched: Vec<String> = h.fake.launched().iter().map(|l| l["title"].as_str().unwrap().to_string()).collect();
    assert_eq!(launched, ["widgets#1: Issue 1"]);
}

#[tokio::test]
async fn an_auto_pick_launches_a_sub_issue_when_only_a_fork_ticket_shares_its_parent_label() {
    let h = flowing_with(1, "auto");
    issue(&h, REPO, 1, &[], 1);
    issue(&h, REPO, 2, &[FLOW], 2);
    h.github.set_parent(REPO, 2, (REPO, 1));
    h.fake.set_projects(vec![project("p-widgets", "/src/widgets"), project("p-other", "/src/other")]);
    h.remotes.set("/src/other", "git@github.com:other/widgets.git");
    let mut th = t3_codemode::fake::thread("mcp:o1", "running", T0);
    th["projectId"] = json!("p-other");
    h.fake.put(th);
    h.wf("mcp:o1", "join", json!({"parent": BOARD, "label": "widgets#1", "role": "ticket"})).await;
    h.app.wf.board_tick().await.unwrap();

    reconcile(&h).await;

    let launched: Vec<String> = h.fake.launched().iter().map(|l| l["title"].as_str().unwrap().to_string()).collect();
    assert_eq!(launched, ["widgets#2: Issue 2"]);
}

#[tokio::test]
async fn an_auto_pick_launches_an_issue_whose_label_a_fork_ticket_holds_and_refuses_it_twice() {
    let h = flowing_with(2, "auto");
    issue(&h, REPO, 5, &[FLOW], 1);
    h.fake.set_projects(vec![project("p-widgets", "/src/widgets"), project("p-other", "/src/other")]);
    h.remotes.set("/src/other", "git@github.com:other/widgets.git");
    let mut th = t3_codemode::fake::thread("mcp:o5", "running", T0);
    th["projectId"] = json!("p-other");
    h.fake.put(th);
    h.wf(
        "mcp:o5",
        "join",
        json!({"parent": BOARD, "label": "widgets#5", "role": "ticket", "issue": "other/widgets#5"}),
    )
    .await;
    h.app.wf.board_tick().await.unwrap();

    reconcile(&h).await;

    let launched: Vec<String> = h.fake.launched().iter().map(|l| l["title"].as_str().unwrap().to_string()).collect();
    assert_eq!(launched, ["acme/widgets#5: Issue 5"]);
    assert!(h.app.store.pending_all().unwrap().iter().any(|p| p.label == "acme/widgets#5"));
    let again = h.exec("async () => wf.launchTicket({ issue: 'acme/widgets#5' })").await;
    assert!(err_text(&again).contains("acme/widgets#5 was launched and has not joined yet"), "{}", again.text);
    let fork = h.exec("async () => wf.launchTicket({ issue: 'other/widgets#5' })").await;
    assert!(err_text(&fork).contains("widgets#5 is already on the board"), "{}", fork.text);
    let f = graph(&h).await;
    assert_eq!(labels(&f["running"]), ["acme/widgets#5"]);
    assert_eq!(f["queue"], json!([]));
}

#[tokio::test]
async fn an_auto_launched_sub_issue_names_its_parent_on_the_ticket() {
    let h = flowing_with(1, "auto");
    issue(&h, REPO, 1, &[], 1);
    issue(&h, REPO, 2, &[FLOW], 2);
    h.github.set_parent(REPO, 2, (REPO, 1));

    reconcile(&h).await;

    let msg = h.fake.launched()[0]["message"].as_str().unwrap().to_string();
    assert!(msg.contains(r#"issue: "acme/widgets#2", parentIssue: "acme/widgets#1" })"#), "{msg}");
    let tree = h.app.wf.tree().unwrap();
    assert_eq!(tree[0]["issue"], "acme/widgets#2");
    assert_eq!(tree[0]["parentIssue"], "acme/widgets#1");
    assert_eq!(tree[0]["part"], Value::Null);
}

async fn board_queue(h: &H) {
    issue(h, REPO, 1, &[FLOW, "priority: high"], 1);
    issue(h, REPO, 2, &[FLOW], 2);
    issue(h, REPO, 3, &[], 3);
    issue(h, REPO, 4, &[FLOW, "priority: low"], 4);
    issue(h, TOOLS, 5, &[], 5);
    issue(h, REPO, 6, &[FLOW, "blocked: external"], 6);
    issue(h, REPO, 7, &[FLOW], 7);
    h.github.set_blockers(REPO, 2, &[(REPO, 3)]);
    h.github.close_issue(REPO, 3);
    h.github.set_blockers(REPO, 4, &[(TOOLS, 5)]);
    h.github.set_blockers(REPO, 6, &[(TOOLS, 5)]);
    h.github.set_milestone(REPO, 7, "v0.7.0", Some("2026-11-30T00:00:00Z"));
    let first = next(h).await;
    let id = first["picked"][0]["card"].as_str().unwrap().to_string();
    h.app.wf.answer(&id, "skip").await.unwrap();
}

#[tokio::test]
async fn the_board_route_returns_the_queue_wf_flow_returns() {
    let h = flowing(1);
    board_queue(&h).await;
    let want = result(&h.exec("async () => wf.flow({ graph: true })").await);
    let lean = flow(&h).await;
    let s = crate::board::start(h).await;

    let got = s.get("/api/flow").await;

    assert_eq!(got, want);
    assert!(lean.get("graph").is_none(), "{lean}");
    assert!(got["graph"]["nodes"].as_array().is_some_and(|n| !n.is_empty()), "{got}");
    assert_eq!(labels(&got["queue"]), ["widgets#7", "widgets#2"]);
    assert_eq!(labels(&got["blocked"]), ["widgets#4", "widgets#6"]);
    assert_eq!(labels(&got["skipped"]), ["widgets#1"]);
    assert_eq!(s.get("/api/state").await["flow"], true);
}

#[tokio::test]
async fn the_board_route_writes_no_skip_state_and_launches_nothing() {
    let h = flowing(2);
    issue(&h, REPO, 1, &[FLOW], 1);
    issue(&h, REPO, 2, &[FLOW], 2);
    let first = next(&h).await;
    let id = first["picked"][0]["card"].as_str().unwrap().to_string();
    h.app.wf.answer(&id, "skip").await.unwrap();
    for c in flow_cards(&h) {
        h.app.wf.dismiss(c["id"].as_str().unwrap()).unwrap();
    }
    h.github.update_issue(REPO, 1, json!({"labels": {"nodes": [{"name": FLOW}, {"name": "priority: high"}]}}));
    let s = crate::board::start(h).await;

    let got = s.get("/api/flow").await;

    assert_eq!(labels(&got["queue"]), ["widgets#1", "widgets#2"]);
    assert_eq!(got["free"], 2);
    assert!(s.h.app.store.meta_get("flow_skip:acme/widgets#1").unwrap().is_some());
    assert!(flow_cards(&s.h).is_empty());
    assert!(s.h.fake.launched().is_empty());
}

const PANEL: &str = r##"
const q = (sel) => document.querySelector(sel);
const tick = async () => { for (let i = 0; i < 10; i++) await Promise.resolve(); };
const reply = (status, body) => Promise.resolve({ ok: status < 400, status, json: async () => body });
const rows = (id) => document.querySelectorAll("#" + id + " li").map((li) => ({
  n: li.querySelector(".fn").textContent,
  label: li.querySelector(".tl").textContent,
  href: (li.querySelector(".tl a") || { getAttribute: () => null }).getAttribute("href"),
  title: li.querySelector(".ttl").textContent,
  why: li.querySelector(".f2").textContent,
}));
const flowFetches = () => fetches.filter((f) => f.url === "/api/flow").length;
"##;

async fn panel_page(h: H) -> (String, Value, Value) {
    let s = crate::board::start(h).await;
    let html = s.http.get(format!("{}/", s.base)).send().await.unwrap().text().await.unwrap();
    let st = s.get("/api/state").await;
    let view = s.get("/api/flow").await;
    (html, st, view)
}

fn panel(html: &str, st: &Value, view: &Value, body: &str) -> Value {
    crate::board::run_page_async_with(
        html,
        "",
        &format!("async () => {{\n{PANEL}\nconst base = {st};\nconst view = {view};\n{body}\n}}"),
    )
}

#[tokio::test]
async fn the_up_next_panel_lists_the_queue_and_folds_blocked_and_skipped_issues() {
    let h = flowing(1);
    board_queue(&h).await;
    let (html, st, view) = panel_page(h).await;

    let r = panel(
        &html,
        &st,
        &view,
        r##"
fetchReply = (url) => url === "/api/flow" ? reply(200, view) : new Promise(() => {});
emitState(base);
await tick();
const shown = {
  hidden: q("#p-flow").hidden, pill: q("#n-flow").textContent, slots: q("#flow-slots").textContent,
  queue: rows("flow-q"), empty: q("#flow-empty").hidden,
  blocked: { open: q("#flow-blocked-btn").getAttribute("aria-expanded"), list: q("#flow-b").hidden, n: q("#n-flow-b").textContent, rows: rows("flow-b") },
  skipped: { open: q("#flow-skipped-btn").getAttribute("aria-expanded"), list: q("#flow-s").hidden, n: q("#n-flow-s").textContent, rows: rows("flow-s") },
};
q("#flow-blocked-btn").dispatch("click");
const unfolded = { open: q("#flow-blocked-btn").getAttribute("aria-expanded"), list: q("#flow-b").hidden };
emitState(base);
await tick();
const again = flowFetches();
emitState({ ...base, tree: [...base.tree, { label: "widgets#9", done: false }] });
await tick();
const link = document.querySelectorAll("#flow-b li")[1].querySelectorAll("a").find((x) => x.textContent === "tools#5");
link.focus();
paintFlow({ ...view, free: 0 });
const now = document.activeElement;
const kept = { focused: now.getAttribute("href"), row: now.closest("li").getAttribute("data-issue"), fresh: now !== link, slots: q("#flow-slots").textContent };
return { shown, unfolded, fetched: [again, flowFetches()], kept };
"##,
    );

    let s = &r["shown"];
    assert_eq!(s["hidden"], false, "{r}");
    assert_eq!(s["pill"], "2", "{r}");
    assert_eq!(s["slots"], "1 of 1 slot free · launch: ask", "{r}");
    assert_eq!(
        s["queue"],
        json!([
            {"n": "1", "label": "widgets#7", "href": "https://github.com/acme/widgets/issues/7", "title": "Issue 7",
             "why": "priority: none · v0.7.0, due 2026-11-30"},
            {"n": "2", "label": "widgets#2", "href": "https://github.com/acme/widgets/issues/2", "title": "Issue 2",
             "why": "priority: none · unblocked by #3"},
        ]),
        "{r}"
    );
    assert_eq!(s["empty"], true, "{r}");
    assert_eq!(s["blocked"]["open"], "false", "{r}");
    assert_eq!(s["blocked"]["list"], true, "{r}");
    assert_eq!(s["blocked"]["n"], "2", "{r}");
    let blocked: Vec<&str> =
        s["blocked"]["rows"].as_array().unwrap().iter().map(|x| x["why"].as_str().unwrap()).collect();
    assert_eq!(
        blocked,
        ["blocked by tools#5 · priority: low", "blocked: external; blocked by tools#5 · priority: none"],
        "{r}"
    );
    assert_eq!(s["skipped"]["n"], "1", "{r}");
    assert_eq!(s["skipped"]["list"], true, "{r}");
    assert_eq!(s["skipped"]["rows"][0]["label"], "widgets#1", "{r}");
    assert_eq!(s["skipped"]["rows"][0]["why"], "priority: high", "{r}");
    assert_eq!(r["unfolded"], json!({"open": "true", "list": false}), "{r}");
    assert_eq!(r["fetched"], json!([1, 2]), "{r}");
    assert_eq!(
        r["kept"],
        json!({"focused": "https://github.com/acme/tools/issues/5", "row": "acme/widgets#6", "fresh": true, "slots": "0 of 1 slot free · launch: ask"}),
        "{r}"
    );
}

#[tokio::test]
async fn the_up_next_panel_hides_without_a_flow_and_says_when_github_fails() {
    let (html, st, view) = panel_page(H::new()).await;
    assert_eq!(st["flow"], false);
    assert_eq!(view["enabled"], false);

    let r = panel(
        &html,
        &st,
        &view,
        r##"
fetchReply = (url) => url === "/api/flow" ? reply(502, { error: "wf.flow: GitHub said no" }) : new Promise(() => {});
emitState(base);
await tick();
const off = { hidden: q("#p-flow").hidden, fetched: flowFetches() };
emitState({ ...base, flow: true });
await tick();
const failed = { hidden: q("#p-flow").hidden, err: q("#flow-err").hidden, text: q("#flow-err-text").textContent };
emitState(base);
await tick();
return { off, failed, gone: { hidden: q("#p-flow").hidden, err: q("#flow-err").hidden } };
"##,
    );

    assert_eq!(r["off"], json!({"hidden": true, "fetched": 0}), "{r}");
    assert_eq!(
        r["failed"],
        json!({"hidden": false, "err": false, "text": "Could not read the up-next queue: wf.flow: GitHub said no"}),
        "{r}"
    );
    assert_eq!(r["gone"], json!({"hidden": true, "err": true}), "{r}");
}

#[tokio::test]
async fn rows_pills_and_the_pulse_strip_show_ready_and_external_work() {
    let h = flowing(3);
    issue(&h, REPO, 1, &[FLOW], 1);
    issue(&h, REPO, 2, &[FLOW, "blocked: external"], 2);
    issue(&h, REPO, 3, &[], 3);
    issue(&h, REPO, 4, &[], 4);
    h.github.set_blockers(REPO, 3, &[("acme/outside", 9)]);
    ticket(&h, "mcp:t3", "widgets#3").await;
    ticket(&h, "mcp:t4", "widgets#4").await;
    let (html, st, view) = panel_page(h).await;

    let r = panel(
        &html,
        &st,
        &view,
        r##"
fetchReply = (url) => url === "/api/flow" ? reply(200, view) : new Promise(() => {});
const slot = (k) => q('#pslots [data-slot="' + k + '"]');
const pill = (thread) => { const p = q('#open [data-thread="' + thread + '"] [data-part="ext"]'); return p ? !p.hidden : false; };
const open = () => [...document.getElementById("open").children].filter((e) => e.dataset.thread && !e.classList.contains("gone")).map((e) => e.dataset.thread);
const flowRows = (id) => [...document.querySelectorAll("#" + id + " li")].map((li) => ({
  label: li.querySelector(".tl").textContent, hidden: li.hidden,
  pill: li.querySelector(".spill") ? li.querySelector(".spill").textContent : null }));
emitState(base);
await tick();
const before = {
  pills: [pill("mcp:t3"), pill("mcp:t4")],
  active: slot("active").querySelector(".pd").textContent,
  held: [slot("held").querySelector(".pn").textContent, slot("held").querySelector(".pd").textContent],
  queue: flowRows("flow-q"), blocked: flowRows("flow-b"),
};
setFilter({ statuses: ["external"] });
await tick();
const external = { open: open(), queue: flowRows("flow-q"), blocked: flowRows("flow-b"), n: q("#n-flow").textContent };
setFilter({ statuses: ["ready"] });
await tick();
const ready = { open: open(), queue: flowRows("flow-q"), n: q("#n-flow").textContent, nb: q("#n-flow-b").textContent };
setFilter({});
slot("held").dispatch("click");
const held = open();
setFilter({});
emitState({ ...base, flow: false });
const off = { pill: pill("mcp:t3"), held: slot("held").querySelector(".pd").textContent };
return { before, external, ready, held, off };
"##,
    );

    let b = &r["before"];
    assert_eq!(b["pills"], json!([true, false]), "{r}");
    assert_eq!(b["active"], "2 threads running, 2 of 3 tickets, 1 ready to launch", "{r}");
    assert_eq!(b["held"], json!(["1", "2 external"]), "{r}");
    assert_eq!(b["queue"], json!([{"label": "widgets#1", "hidden": false, "pill": "ready"}]), "{r}");
    assert_eq!(b["blocked"], json!([{"label": "widgets#2", "hidden": false, "pill": "external"}]), "{r}");
    let e = &r["external"];
    assert_eq!(e["open"], json!(["mcp:t3"]), "{r}");
    assert_eq!(e["queue"][0]["hidden"], true, "{r}");
    assert_eq!(e["blocked"][0]["hidden"], false, "{r}");
    assert_eq!(e["n"], "0/1", "{r}");
    let rd = &r["ready"];
    assert_eq!(rd["open"], json!([]), "{r}");
    assert_eq!(rd["queue"][0]["hidden"], false, "{r}");
    assert_eq!((rd["n"].as_str(), rd["nb"].as_str()), (Some("1/1"), Some("0/1")), "{r}");
    assert_eq!(r["held"], json!(["mcp:t3"]), "{r}");
    assert_eq!(r["off"], json!({"pill": false, "held": "Nothing parked"}), "{r}");
}

#[tokio::test]
async fn a_ticket_whose_repo_the_board_knows_is_read_there_and_picks_read_only_state() {
    let h = flowing(3);
    issue(&h, REPO, 5, &[FLOW], 1);
    h.fake.set_projects(vec![project("p-widgets", "/src/widgets"), project("p-other", "/src/other")]);
    h.remotes.set("/src/other", "git@github.com:other/widgets.git");
    h.github.set_issue("other/widgets", github_issue(5, "Fork work", &[], "2026-10-20T09:00:00Z"));
    let mut th = t3_codemode::fake::thread("mcp:o5", "running", T0);
    th["projectId"] = json!("p-other");
    h.fake.put(th);
    h.wf("mcp:o5", "join", json!({"parent": BOARD, "label": "widgets#5", "role": "ticket"})).await;
    h.app.wf.board_tick().await.unwrap();
    issue(&h, REPO, 6, &[], 2);
    ticket(&h, "mcp:t6", "widgets#6").await;

    let f = graph(&h).await;

    assert_eq!(node(&f, "other/widgets#5")["ticket"]["thread"], "mcp:o5");
    assert_eq!(node(&f, "other/widgets#5")["title"], "Fork work");
    assert_eq!(node(&f, "acme/widgets#5")["ticket"], Value::Null);
    assert_eq!(labels(&f["running"]), ["widgets#6"]);
    assert_eq!(f["free"], 2);
    assert_eq!(labels(&f["queue"]), ["widgets#5"]);
    assert_eq!(node(&f, "acme/widgets#5")["ready"], true);
    h.github.close_issue(REPO, 6);
    let f = graph(&h).await;
    assert_eq!(f["running"], json!([]));
    assert_eq!(f["free"], 3);
    h.github.update_issue(REPO, 6, json!({"state": "OPEN"}));
    let before = h.github.calls().len();
    next(&h).await;
    let reads: Vec<Value> =
        h.github.calls()[before..].iter().filter(|c| c["operation"] == "flowTickets").cloned().collect();
    assert_eq!(reads.len(), 1);
    assert!(!reads[0]["query"].as_str().unwrap().contains("blockedBy"), "{}", reads[0]["query"]);
    assert_eq!(reads[0]["variables"]["o0"], "acme");
}

const GRAPH: &str = r##"
const btn = (id) => q('.gnode[data-id="' + id + '"]');
const nodes = () => document.querySelectorAll(".gnode").map((b) => b.dataset.id);
const spot = (id) => { const f = btn(id).parentNode; return [+f.getAttribute("x"), +f.getAttribute("y")]; };
const lanes = () => document.querySelectorAll(".glt").map((t) => t.childNodes.filter((n) => n.nodeType === 3).map((n) => n.data).join(""));
const marks = () => Object.fromEntries(nodes().map((id) => [id, ["hot", "up", "down", "dim", "out"].filter((c) => btn(id).classList.contains(c)).join(" ")]));
const edges = () => document.querySelectorAll(".ge").map((p) => p.getAttribute("class"));
const dp = (name) => q('#drawer [data-part="' + name + '"]');
const deps = () => document.querySelectorAll("#drawer .dr-dep").map((d) => d.textContent);
const chip = (label) => document.querySelectorAll("#drawer .dr-dep button").find((b) => b.textContent === label);
"##;

async fn graph_page() -> (String, Value, Value) {
    let h = flowing(4);
    issue(&h, REPO, 1, &[FLOW, "track: board"], 1);
    issue(&h, REPO, 2, &[FLOW, "track: board", "priority: high"], 2);
    issue(&h, REPO, 3, &[], 3);
    issue(&h, REPO, 4, &[FLOW], 4);
    issue(&h, REPO, 5, &[], 5);
    issue(&h, REPO, 6, &[], 6);
    issue(&h, TOOLS, 7, &[FLOW], 7);
    h.github.set_blockers(REPO, 2, &[(REPO, 1), (REPO, 3)]);
    h.github.close_issue(REPO, 3);
    h.github.set_blockers(REPO, 4, &[(REPO, 2)]);
    h.github.set_parent(REPO, 4, (REPO, 5));
    h.github.set_milestone(REPO, 2, "v0.9.0", None);
    ticket(&h, "mcp:t6", "widgets#6").await;
    panel_page(h).await
}

#[tokio::test]
async fn the_graph_tab_draws_lanes_and_traces_blockers_and_unlocks() {
    let (html, st, view) = graph_page().await;
    let r = panel(
        &html,
        &st,
        &view,
        &format!(
            r##"{GRAPH}
fetchReply = (url) => url === "/api/flow" ? reply(200, view) : new Promise(() => {{}});
emitState(base);
await tick();
selectTab("graph");
const first = {{ lanes: lanes(), nodes: nodes(), edges: edges(), n: q("#n-graph").textContent, note: q("#graph-note").textContent,
  at: Object.fromEntries(nodes().map((id) => [id, spot(id)])) }};
emitState({{ ...base }});
await tick();
const again = Object.fromEntries(nodes().map((id) => [id, spot(id)]));
btn("acme/widgets#2").dispatch("pointerover");
const hover = {{ marks: marks(), edges: edges() }};
q("#gscroll").dispatch("pointerleave");
const left = Object.values(marks()).filter(Boolean).length;
btn("acme/widgets#4").dispatch("focusin");
btn("acme/widgets#2").dispatch("pointerover");
q("#gscroll").dispatch("pointerover");
const focused = marks()["acme/widgets#4"];
q("#gscroll").dispatch("pointerleave");
const stays = marks()["acme/widgets#4"];
btn("acme/widgets#2").dispatch("pointerover");
btn("acme/widgets#1").dispatch("focusin");
const latest = [marks()["acme/widgets#1"], marks()["acme/widgets#2"]];
btn("acme/widgets#1").dispatch("focusout");
q("#gscroll").dispatch("pointerleave");
advance(1);
btn("acme/widgets#4").dispatch("focusout");
advance(1);
setFilter({{ statuses: ["ready"] }});
await tick();
const ready = {{ marks: marks(), n: q("#n-graph").textContent }};
setFilter({{}});
return {{ first, same: JSON.stringify(again) === JSON.stringify(first.at), hover, left, focused, stays, latest, ready }};
"##
        ),
    );
    let f = &r["first"];
    assert_eq!(
        f["lanes"],
        json!(["widgets#6", "board", "tools#7"]),
        "lanes follow each node's track, and lanes holding a ticket come first: {r}"
    );
    assert_eq!(
        f["nodes"],
        json!([
            "acme/widgets#6",
            "acme/widgets#1",
            "acme/widgets#3",
            "acme/widgets#5",
            "acme/widgets#2",
            "acme/widgets#4",
            "acme/tools#7"
        ]),
        "lane by lane, column by column: {r}"
    );
    assert_eq!(f["edges"], json!(["ge", "ge", "ge", "ge par"]), "three blocked-by links and one parent link: {r}");
    assert_eq!(f["n"], "7", "{r}");
    assert!(f["note"].as_str().unwrap().starts_with("3 lanes by track"), "{r}");
    let at = |id: &str| f["at"][id].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect::<Vec<_>>();
    assert!(at("acme/widgets#1")[0] < at("acme/widgets#2")[0], "a blocker sits left of what it blocks: {r}");
    assert!(at("acme/widgets#2")[0] < at("acme/widgets#4")[0], "{r}");
    assert!(at("acme/widgets#5")[0] < at("acme/widgets#4")[0], "a parent sits left of its sub-issues: {r}");
    assert_eq!(r["same"], true, "the same graph lays out the same way: {r}");
    let h = &r["hover"]["marks"];
    assert_eq!(
        (&h["acme/widgets#2"], &h["acme/widgets#1"], &h["acme/widgets#3"], &h["acme/widgets#4"], &h["acme/widgets#5"]),
        (&json!("hot"), &json!("up"), &json!("up"), &json!("down"), &json!("dim")),
        "{r}"
    );
    assert_eq!(r["hover"]["edges"], json!(["ge up", "ge up", "ge down", "ge par dim"]), "{r}");
    assert_eq!(r["left"], 0, "leaving the graph clears the trace: {r}");
    assert_eq!(
        (&r["focused"], &r["stays"]),
        (&json!("hot"), &json!("hot")),
        "with no node under the pointer, the focused node keeps its trace: {r}"
    );
    assert_eq!(r["latest"], json!(["hot", "down"]), "focus moved after the hover drives the trace: {r}");
    let rd = &r["ready"];
    assert_eq!(rd["n"], "2/7", "{r}");
    assert_eq!((&rd["marks"]["acme/widgets#1"], &rd["marks"]["acme/tools#7"]), (&json!(""), &json!("")), "{r}");
    assert_eq!(rd["marks"]["acme/widgets#2"], "out", "{r}");
}

#[tokio::test]
async fn a_graph_node_opens_the_inspector_and_its_chips_move_between_issues() {
    let (html, st, view) = graph_page().await;
    let r = panel(
        &html,
        &st,
        &view,
        &format!(
            r##"{GRAPH}
fetchReply = (url) => url === "/api/flow" ? reply(200, view) : new Promise(() => {{}});
emitState(base);
await tick();
selectTab("graph");
const box = q("#gscroll");
box.scrollTop = 80;
btn("acme/widgets#2").dispatch("click");
const issue = {{ kind: q("#drawer").getAttribute("data-kind"), id: dp("id").textContent, title: dp("title").textContent, st: dp("st").textContent,
  prio: dp("prio").hidden ? null : dp("prio").textContent, ms: dp("ms").hidden ? null : dp("ms").textContent,
  gh: dp("gh-issue").getAttribute("href"), lane: dp("deps-lane").textContent, deps: deps(),
  marked: document.querySelectorAll(".gnode[data-inspected]").map((b) => b.dataset.id) }};
chip("widgets#4").dispatch("click");
const moved = {{ id: dp("id").textContent, deps: deps(), scroll: box.scrollTop, focus: document.activeElement === dp("title") }};
chip("widgets#5").dispatch("click");
const parent = {{ id: dp("id").textContent, note: dp("note-m").textContent, deps: deps() }};
btn("acme/widgets#6").dispatch("click");
const ticket = {{ kind: q("#drawer").getAttribute("data-kind"), id: dp("id").textContent, deps: dp("deps").hidden, lane: dp("deps-lane").textContent }};
closeDrawer(true);
return {{ issue, moved, parent, ticket, back: document.activeElement === btn("acme/widgets#6") }};
"##
        ),
    );
    let i = &r["issue"];
    assert_eq!(
        (&i["kind"], &i["id"], &i["title"], &i["st"], &i["prio"], &i["ms"]),
        (
            &json!("issue"),
            &json!("widgets#2"),
            &json!("Issue 2"),
            &json!("blocked"),
            &json!("priority: high"),
            &json!("v0.9.0")
        ),
        "{r}"
    );
    assert_eq!(i["gh"], "https://github.com/acme/widgets/issues/2", "{r}");
    assert_eq!(i["lane"], "Lane board, from its track label.", "{r}");
    assert_eq!(i["deps"], json!(["Blocked bywidgets#1readywidgets#3closed", "Unlockswidgets#4blocked"]), "{r}");
    assert_eq!(i["marked"], json!(["acme/widgets#2"]), "{r}");
    let m = &r["moved"];
    assert_eq!(m["id"], "widgets#4", "{r}");
    assert_eq!(m["deps"], json!(["Blocked bywidgets#2blocked", "Parentwidgets#5open"]), "{r}");
    assert_eq!(
        (&m["scroll"], &m["focus"]),
        (&json!(80), &json!(true)),
        "a chip moves the inspector, not the board: {r}"
    );
    let p = &r["parent"];
    assert_eq!(p["deps"], json!(["Sub-issueswidgets#4blocked"]), "{r}");
    assert!(p["note"].as_str().unwrap().starts_with("Not opted into the flow"), "{r}");
    assert_eq!(
        r["ticket"],
        json!({"kind": "ticket", "id": "widgets#6", "deps": false, "lane": "Lane widgets#6."}),
        "{r}"
    );
    assert_eq!(r["back"], true, "Esc-style close returns focus to the node: {r}");
}

#[tokio::test]
async fn the_graph_says_when_a_refresh_fails_until_a_read_succeeds() {
    let (html, st, view) = graph_page().await;
    let r = panel(
        &html,
        &st,
        &view,
        r##"
let fail = false;
fetchReply = (url) => url === "/api/flow" ? (fail ? reply(502, { error: "GitHub said no" }) : reply(200, view)) : new Promise(() => {});
emitState(base);
await tick();
selectTab("graph");
const note = () => q("#graph-note").textContent.startsWith("Could not refresh");
const good = note();
fail = true;
loadFlow();
await tick();
const failed = [note(), document.querySelectorAll(".gnode").length];
closeErr("flow-err");
emitState({ ...base });
const dismissed = note();
fail = false;
loadFlow();
await tick();
return { good, failed, dismissed, back: note() };
"##,
    );
    assert_eq!(
        r,
        json!({"good": false, "failed": [true, 7], "dismissed": true, "back": false}),
        "the last graph stays, says it is stale, and recovers on the same payload: {r}"
    );
}
