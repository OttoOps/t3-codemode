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
        brief: None,
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
    issue(&h, REPO, 6, &[], 6);
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
            {"from": "acme/gadgets#3", "to": "acme/widgets#11", "kind": "blockedBy", "source": "github"},
            {"from": "acme/widgets#2", "to": "acme/widgets#1", "kind": "blockedBy", "source": "github"},
            {"from": "acme/widgets#2", "to": "acme/widgets#3", "kind": "blockedBy", "source": "github"},
            {"from": "acme/widgets#4", "to": "acme/widgets#5", "kind": "parent", "source": "github"},
            {"from": "acme/widgets#6", "to": "acme/tools#7", "kind": "blockedBy", "source": "github"},
            {"from": "acme/widgets#11", "to": "acme/widgets#10", "kind": "parent", "source": "github"},
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
    assert_eq!((four["track"].as_str(), four["trackBy"].as_str()), (Some("widgets#5"), Some("epic")));
    let five = node(&f, "acme/widgets#5");
    assert_eq!((five["tracked"].as_bool(), five["state"].as_str()), (Some(true), Some("open")));
    assert_eq!(ids(&five["subIssues"]), ["acme/widgets#4"]);
    assert_eq!((five["track"].as_str(), five["trackBy"].as_str()), (Some("widgets#5"), Some("epic")));

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
        h.github.close_issue(REPO, n);
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
async fn with_no_flow_block_the_graph_discovers_t3_project_repos_and_reads_their_dag() {
    let h = H::new();
    h.remotes.set("/src/widgets", "git@github.com:acme/widgets.git");
    h.fake.set_projects(vec![project("p-widgets", "/src/widgets")]);
    issue(&h, REPO, 1, &[], 1);
    issue(&h, REPO, 2, &[], 2);
    h.github.set_blockers(REPO, 2, &[(REPO, 1)]);
    let mut th = t3_codemode::fake::thread("mcp:t1", "running", T0);
    th["projectId"] = json!("p-widgets");
    h.fake.put(th);
    h.wf("mcp:t1", "join", json!({"parent": BOARD, "label": "widgets#1", "role": "ticket"})).await;
    h.app.wf.board_tick().await.unwrap();
    let before = h.github.calls().len();

    let f = graph(&h).await;

    assert_eq!(h.github.operations()[before..], ["flowIssues"]);
    assert_eq!(f["enabled"], false);
    assert_eq!(f["launch"], Value::Null);
    assert_eq!(f["maxTickets"], 0);
    assert_eq!(f["queue"], json!([]));
    let n1 = node(&f, "acme/widgets#1");
    assert_eq!(n1["tracked"], true);
    assert_eq!(n1["title"], "Issue 1");
    assert_eq!(n1["ticket"]["thread"], "mcp:t1");
    assert_eq!((n1["track"].as_str(), n1["trackBy"].as_str()), (Some("widgets#1"), Some("component")));
    let n2 = node(&f, "acme/widgets#2");
    assert_eq!(ids(&n2["blockedBy"]), ["acme/widgets#1"]);
    assert_eq!(
        f["graph"]["edges"],
        json!([{"from": "acme/widgets#2", "to": "acme/widgets#1", "kind": "blockedBy", "source": "github"}])
    );
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
    assert_eq!(graph(&h).await["graph"], json!({"nodes": [], "edges": [], "warnings": []}));
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
    assert!(sig.contains(r#"source: "github" | "body" | "git" | "code""#), "{sig}");
    assert!(sig.contains("warnings: {"), "{sig}");
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
    assert_eq!(flow.max_tickets, Some(2));

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

    // Optional repos, optIn, includeRepos/excludeRepos, and graph-only flow blocks.
    let no_opt_in = Config::from_value(json!({"flow": {"repos": [REPO], "maxTickets": 1}})).unwrap().flow.unwrap();
    assert!(no_opt_in.opt_in.is_none());
    assert_eq!(no_opt_in.launch, t3_codemode::config::Launch::Ask);
    assert!(no_opt_in.queue_enabled());

    let discovered = Config::from_value(
        json!({"flow": {"includeRepos": [REPO], "excludeRepos": [TOOLS], "optIn": {"milestone": "v1.0"}, "maxTickets": 2}}),
    )
    .unwrap()
    .flow
    .unwrap();
    assert!(discovered.repos.is_none());
    assert_eq!(discovered.include_repos, [REPO]);
    assert_eq!(discovered.exclude_repos, [TOOLS]);
    assert_eq!(discovered.opt_in.as_ref().unwrap().milestone.as_deref(), Some("v1.0"));
    assert_eq!(discovered.launch, t3_codemode::config::Launch::Ask);

    let graph_only = Config::from_value(json!({"flow": {"excludeRepos": [TOOLS]}})).unwrap().flow.unwrap();
    assert!(!graph_only.queue_enabled());
    assert_eq!(graph_only.launch, t3_codemode::config::Launch::Off);

    let (at, _) = config_error(json!({"launch": "ask"}));
    assert_eq!(at, "flow.maxTickets");
    let (at, _) = config_error(json!({"launch": "auto"}));
    assert_eq!(at, "flow.maxTickets");
    let (at, _) = config_error(json!({"optIn": {}, "maxTickets": 1}));
    assert_eq!(at, "flow.optIn");
    let (at, _) = config_error(json!({"optIn": {"milestone": " "}, "maxTickets": 1}));
    assert_eq!(at, "flow.optIn.milestone");
    let (at, msg) = config_error(json!({"includeRepos": ["widgets"]}));
    assert_eq!(at, "flow.includeRepos.0");
    assert!(msg.contains("not an owner/name"), "{msg}");
    let (at, msg) = config_error(json!({"includeRepos": [REPO, "Acme/Widgets"]}));
    assert_eq!(at, "flow.includeRepos.1");
    assert!(msg.contains("twice"), "{msg}");
    let (at, msg) = config_error(json!({"excludeRepos": ["widgets"]}));
    assert_eq!(at, "flow.excludeRepos.0");
    assert!(msg.contains("not an owner/name"), "{msg}");
    let (at, msg) = config_error(json!({"excludeRepos": [REPO, "Acme/Widgets"]}));
    assert_eq!(at, "flow.excludeRepos.1");
    assert!(msg.contains("twice"), "{msg}");
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
        h.app.wf.dismiss(c["id"].as_str().unwrap()).await.unwrap();
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
        json!(["acme/widgets: Independent", "board", "widgets#5", "acme/tools: Independent"]),
        "lanes follow each node's track, and lanes holding a ticket come first: {r}"
    );
    assert_eq!(
        f["nodes"],
        json!([
            "acme/widgets#6",
            "acme/widgets#1",
            "acme/widgets#3",
            "acme/widgets#2",
            "acme/widgets#5",
            "acme/widgets#4",
            "acme/tools#7"
        ]),
        "lane by lane, column by column: {r}"
    );
    assert_eq!(f["edges"], json!(["ge", "ge", "ge", "ge par"]), "three blocked-by links and one parent link: {r}");
    assert_eq!(f["n"], "7", "{r}");
    assert!(f["note"].as_str().unwrap().starts_with("4 lanes by track"), "{r}");
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
        json!({"kind": "ticket", "id": "widgets#6", "deps": false, "lane": "Lane acme/widgets: Independent, from unlinked issues in its repository."}),
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

#[tokio::test]
async fn zero_config_graph_discovers_t3_project_repos_and_keeps_launching_off() {
    let h = H::new();
    let mut tools_proj = project("p-tools", "/src/tools");
    tools_proj["repositoryIdentity"] = json!({
        "canonicalKey": "github.com/acme/tools", "owner": "acme", "name": "tools",
        "locator": {"source": "git-remote", "remoteName": "origin", "remoteUrl": "https://github.com/acme/tools.git"}
    });
    let mut archived = project("p-old", "/src/old");
    archived["archivedAt"] = json!("2026-10-01T00:00:00Z");
    let gitlab = project("p-gl", "/src/gl");
    h.fake.set_projects(vec![project("p-widgets", "/src/widgets"), tools_proj, archived, gitlab]);
    h.remotes.set("/src/widgets", "git@github.com:acme/widgets.git");
    h.remotes.set("/src/old", "git@github.com:acme/old.git");
    h.remotes.set("/src/gl", "https://gitlab.example.com/team/app.git");

    issue(&h, REPO, 1, &[], 1);
    issue(&h, REPO, 2, &[], 2);
    issue(&h, REPO, 3, &[], 3);
    issue(&h, REPO, 4, &[], 4);
    issue(&h, TOOLS, 10, &[], 5);
    issue(&h, TOOLS, 11, &[], 6);
    h.github.set_blockers(REPO, 2, &[(REPO, 1)]);

    // Viewing the graph never implies launching: wf.flow() and wf.flow.next() stay off and read nothing.
    let lean = flow(&h).await;
    assert_eq!(lean["enabled"], false);
    assert_eq!(lean["launch"], Value::Null);
    assert_eq!(lean["queue"], json!([]));
    let pick = next(&h).await;
    assert_eq!(pick["launch"], Value::Null);
    assert_eq!(pick["picked"], json!([]));
    assert!(h.github.calls().is_empty(), "off flow makes no GitHub reads: {:?}", h.github.calls());
    assert!(h.fake.launched().is_empty());

    // GET /api/flow and the Graph tab discover acme/widgets and acme/tools (ignoring archived and GitLab).
    let (html, st, view) = panel_page(h).await;
    assert_eq!(st["flow"], false);
    assert_eq!(st["flowMaxTickets"], Value::Null);
    assert_eq!(view["enabled"], false);
    assert_eq!(view["launch"], Value::Null);
    assert_eq!(view["queue"], json!([]));
    assert_eq!(
        view["graph"]["edges"],
        json!([{"from": "acme/widgets#2", "to": "acme/widgets#1", "kind": "blockedBy", "source": "github"}])
    );
    for id in ["acme/widgets#3", "acme/widgets#4"] {
        assert_eq!(
            (node(&view, id)["track"].as_str(), node(&view, id)["trackBy"].as_str()),
            (Some("acme/widgets: Independent"), Some("independent")),
            "widgets singletons share one lane: {view}"
        );
    }
    for id in ["acme/tools#10", "acme/tools#11"] {
        assert_eq!(
            (node(&view, id)["track"].as_str(), node(&view, id)["trackBy"].as_str()),
            (Some("acme/tools: Independent"), Some("independent")),
            "tools singletons share one lane: {view}"
        );
    }

    let r = panel(
        &html,
        &st,
        &view,
        &format!(
            r##"{GRAPH}
let fail = false;
fetchReply = (url) => url === "/api/flow" ? (fail ? reply(502, {{ error: "gh down" }}) : reply(200, view)) : new Promise(() => {{}});
emitState(base);
await tick();
const onOpen = {{ flowHidden: q("#p-flow").hidden, fetches: flowFetches() }};
selectTab("graph");
await tick();
const onGraph = {{
  flowHidden: q("#p-flow").hidden,
  fetches: flowFetches(),
  lanes: lanes(),
  nodes: nodes(),
  edges: edges(),
  n: q("#n-graph").textContent,
  note: q("#graph-note").textContent,
}};
fail = true;
loadFlow();
await tick();
const afterErr = {{
  flowHidden: q("#p-flow").hidden,
  flowErrHidden: q("#flow-err").hidden,
  stale: q("#graph-note").textContent.startsWith("Could not refresh"),
}};
return {{ onOpen, onGraph, afterErr }};
"##
        ),
    );
    assert_eq!(r["onOpen"], json!({"flowHidden": true, "fetches": 0}), "{r}");
    assert_eq!(r["onGraph"]["flowHidden"], true, "Graph tab never reveals Up next without launch config: {r}");
    assert_eq!(r["onGraph"]["fetches"], 1, "{r}");
    assert_eq!(
        r["onGraph"]["lanes"],
        json!(["widgets#1", "acme/tools: Independent", "acme/widgets: Independent"]),
        "{r}"
    );
    assert_eq!(r["onGraph"]["edges"], json!(["ge"]), "{r}");
    assert_eq!(r["onGraph"]["n"], "6", "{r}");
    assert!(r["onGraph"]["note"].as_str().unwrap().starts_with("3 lanes by track"), "{r}");
    assert_eq!(
        r["afterErr"],
        json!({"flowHidden": true, "flowErrHidden": true, "stale": true}),
        "Graph refresh error with no flow block keeps Up next hidden: {r}"
    );
}

#[tokio::test]
async fn discovered_repos_skip_an_unreadable_repo_when_others_succeed_and_fail_when_all_fail() {
    let h = H::new();
    h.fake.set_projects(vec![project("p-widgets", "/src/widgets"), project("p-missing", "/src/missing")]);
    h.remotes.set("/src/widgets", "git@github.com:acme/widgets.git");
    h.remotes.set("/src/missing", "git@github.com:acme/missing.git");
    issue(&h, REPO, 1, &[], 1);

    let f = graph(&h).await;
    assert_eq!(node(&f, "acme/widgets#1")["title"], "Issue 1");

    let only_missing = H::new();
    only_missing.fake.set_projects(vec![project("p-missing", "/src/missing")]);
    only_missing.remotes.set("/src/missing", "git@github.com:acme/missing.git");
    let err = only_missing.app.wf.flow(true).await.unwrap_err();
    assert!(format!("{err:#}").contains("reading acme/missing's issues failed"), "{err:#}");
}

#[tokio::test]
async fn flow_repos_defaults_to_discovered_projects_with_include_and_exclude_overrides() {
    let cfg = Config::from_value(json!({
        "flow": {
            "includeRepos": ["acme/extra"],
            "excludeRepos": ["acme/archive"],
            "maxTickets": 2,
            "launch": "ask"
        }
    }))
    .unwrap();
    let h = H::build(Vec::new(), cfg);
    h.fake.set_projects(vec![project("p-widgets", "/src/widgets"), project("p-archive", "/src/archive")]);
    h.remotes.set("/src/widgets", "git@github.com:acme/widgets.git");
    h.remotes.set("/src/archive", "git@github.com:acme/archive.git");
    issue(&h, REPO, 1, &[], 1);
    issue(&h, "acme/extra", 2, &[], 2);
    // acme/archive is intentionally NOT registered on fake GitHub; excluding it prevents any query for it.

    let f = graph(&h).await;
    assert_eq!(f["enabled"], true);
    assert_eq!(labels(&f["queue"]), ["widgets#1", "extra#2"]);
    assert_eq!(node(&f, "acme/widgets#1")["ready"], true);
    assert_eq!(node(&f, "acme/extra#2")["ready"], true);

    // Graph-only override (no maxTickets) keeps launching and snapshot.flow off while still honoring overrides.
    let graph_only_cfg = Config::from_value(json!({"flow": {"excludeRepos": ["acme/archive"]}})).unwrap();
    let g = H::build(Vec::new(), graph_only_cfg);
    g.fake.set_projects(vec![project("p-widgets", "/src/widgets"), project("p-archive", "/src/archive")]);
    g.remotes.set("/src/widgets", "git@github.com:acme/widgets.git");
    g.remotes.set("/src/archive", "git@github.com:acme/archive.git");
    issue(&g, REPO, 5, &[], 1);

    assert_eq!(g.app.wf.snapshot().unwrap()["flow"], false);
    assert_eq!(g.app.wf.snapshot().unwrap()["flowMaxTickets"], Value::Null);
    assert_eq!(flow(&g).await["enabled"], false);
    assert!(g.github.calls().is_empty());
    let gv = graph(&g).await;
    assert_eq!(gv["enabled"], false);
    assert_eq!(node(&gv, "acme/widgets#5")["title"], "Issue 5");
}

#[tokio::test]
async fn optional_opt_in_supports_milestone_and_combined_label_filters() {
    let ms_cfg = Config::from_value(json!({
        "flow": {"repos": [REPO], "optIn": {"milestone": "v1.0"}, "maxTickets": 3}
    }))
    .unwrap();
    let h = H::build(Vec::new(), ms_cfg);
    issue(&h, REPO, 1, &[], 1);
    issue(&h, REPO, 2, &[], 2);
    issue(&h, REPO, 3, &[], 3);
    h.github.set_milestone(REPO, 1, "V1.0", None);
    h.github.set_milestone(REPO, 2, "v2.0", None);

    let f = flow(&h).await;
    assert_eq!(labels(&f["queue"]), ["widgets#1"]);

    let both_cfg = Config::from_value(json!({
        "flow": {"repos": [REPO], "optIn": {"label": "ready", "milestone": "v1.0"}, "maxTickets": 3}
    }))
    .unwrap();
    let h2 = H::build(Vec::new(), both_cfg);
    issue(&h2, REPO, 1, &["ready"], 1);
    issue(&h2, REPO, 2, &["ready"], 2);
    issue(&h2, REPO, 3, &[], 3);
    h2.github.set_milestone(REPO, 1, "v1.0", None);
    h2.github.set_milestone(REPO, 3, "v1.0", None);

    let f2 = flow(&h2).await;
    assert_eq!(labels(&f2["queue"]), ["widgets#1"]);
}

#[tokio::test]
async fn transitive_blocker_chains_survive_unlabelled_intermediates() {
    let h = flowing(2);
    issue(&h, REPO, 1, &[], 1);
    issue(&h, REPO, 2, &[], 2);
    issue(&h, REPO, 3, &[FLOW], 3);
    h.github.set_blockers(REPO, 2, &[(REPO, 1)]);
    h.github.set_blockers(REPO, 3, &[(REPO, 2)]);

    let f = graph(&h).await;

    assert_eq!(labels(&f["queue"]), Vec::<String>::new());
    assert_eq!(labels(&f["blocked"]), ["widgets#3"]);
    for id in ["acme/widgets#1", "acme/widgets#2", "acme/widgets#3"] {
        assert_eq!(node(&f, id)["tracked"], true, "{id} should be fully read: {f}");
    }
    assert_eq!(ids(&node(&f, "acme/widgets#3")["blockedBy"]), ["acme/widgets#2"]);
    assert_eq!(ids(&node(&f, "acme/widgets#2")["blockedBy"]), ["acme/widgets#1"]);
    assert_eq!(ids(&node(&f, "acme/widgets#2")["unlocks"]), ["acme/widgets#3"]);
    assert_eq!(ids(&node(&f, "acme/widgets#1")["unlocks"]), ["acme/widgets#2"]);
    assert_eq!(node(&f, "acme/widgets#1")["flowList"], Value::Null);
    assert_eq!(node(&f, "acme/widgets#2")["flowList"], Value::Null);
    assert_eq!(node(&f, "acme/widgets#3")["flowList"], "blocked");
    assert_eq!(
        f["graph"]["edges"],
        json!([
            {"from": "acme/widgets#2", "to": "acme/widgets#1", "kind": "blockedBy", "source": "github"},
            {"from": "acme/widgets#3", "to": "acme/widgets#2", "kind": "blockedBy", "source": "github"},
        ])
    );
}

#[tokio::test]
async fn smart_lane_grouping_follows_five_tier_precedence_and_collapses_singletons() {
    let h = H::new();
    h.fake.set_projects(vec![project("p-widgets", "/src/widgets"), project("p-tools", "/src/tools")]);
    h.remotes.set("/src/widgets", "git@github.com:acme/widgets.git");
    h.remotes.set("/src/tools", "git@github.com:acme/tools.git");

    // Tier 1: track:<name> > *-track > area:* (all beat parent epic and milestone).
    issue(&h, REPO, 1, &["workflow-track", "area: ui", "track: flow-dag"], 1);
    issue(&h, REPO, 2, &["area: ui", "workflow-track"], 2);
    issue(&h, REPO, 3, &["area: board"], 3);
    h.github.set_milestone(REPO, 1, "m1", None);
    h.github.set_milestone(REPO, 2, "m1", None);
    h.github.set_milestone(REPO, 3, "m1", None);

    // Tier 2a: sub-issue with no label inherits parent epic's explicit track even when it has a milestone.
    issue(&h, REPO, 4, &[], 4);
    h.github.set_parent(REPO, 4, (REPO, 1));
    h.github.set_milestone(REPO, 4, "m2", None);

    // Tier 2b: unlabelled epic root + sub-issues + grandchild share one epic lane, beating milestone.
    issue(&h, REPO, 10, &[], 10);
    issue(&h, REPO, 11, &[], 11);
    issue(&h, REPO, 12, &[], 12);
    h.github.set_parent(REPO, 11, (REPO, 10));
    h.github.set_parent(REPO, 12, (REPO, 11));
    h.github.set_milestone(REPO, 11, "m2", None);

    // Tier 2c: unlabelled epic root whose sub-issues all have explicit labels joins their component track.
    issue(&h, REPO, 15, &[], 15);
    issue(&h, REPO, 16, &["area: docs"], 16);
    h.github.set_parent(REPO, 16, (REPO, 15));

    // Tier 3: milestone when no track label and no parent epic.
    issue(&h, REPO, 20, &[], 20);
    h.github.set_milestone(REPO, 20, "v1.0", None);

    // Tier 4: connected component only when size >= 2.
    issue(&h, REPO, 25, &[], 25);
    issue(&h, REPO, 26, &[], 26);
    h.github.set_blockers(REPO, 26, &[(REPO, 25)]);
    issue(&h, REPO, 27, &[], 27);
    issue(&h, REPO, 28, &[], 28);
    h.github.set_milestone(REPO, 27, "v2.0", None);
    h.github.set_blockers(REPO, 28, &[(REPO, 27)]);

    // Tier 5: singleton unlinked issues share one per-repo Independent lane.
    issue(&h, REPO, 30, &[], 30);
    issue(&h, REPO, 31, &[], 31);
    issue(&h, TOOLS, 40, &[], 40);
    issue(&h, TOOLS, 41, &[], 41);

    let f = graph(&h).await;
    let lane = |id: &str| {
        let n = node(&f, id);
        (n["track"].as_str().unwrap().to_string(), n["trackBy"].as_str().unwrap().to_string())
    };

    assert_eq!(lane("acme/widgets#1"), ("flow-dag".into(), "label".into()));
    assert_eq!(lane("acme/widgets#2"), ("workflow".into(), "label".into()));
    assert_eq!(lane("acme/widgets#3"), ("board".into(), "label".into()));
    assert_eq!(lane("acme/widgets#4"), ("flow-dag".into(), "epic".into()));
    assert_eq!(lane("acme/widgets#10"), ("widgets#10".into(), "epic".into()));
    assert_eq!(lane("acme/widgets#11"), ("widgets#10".into(), "epic".into()));
    assert_eq!(lane("acme/widgets#12"), ("widgets#10".into(), "epic".into()));
    assert_eq!(lane("acme/widgets#15"), ("docs".into(), "component".into()));
    assert_eq!(lane("acme/widgets#16"), ("docs".into(), "label".into()));
    assert_eq!(lane("acme/widgets#20"), ("v1.0".into(), "milestone".into()));
    assert_eq!(lane("acme/widgets#25"), ("widgets#25".into(), "component".into()));
    assert_eq!(lane("acme/widgets#26"), ("widgets#25".into(), "component".into()));
    assert_eq!(lane("acme/widgets#27"), ("v2.0".into(), "milestone".into()));
    assert_eq!(lane("acme/widgets#28"), ("v2.0".into(), "component".into()));
    assert_eq!(lane("acme/widgets#30"), ("acme/widgets: Independent".into(), "independent".into()));
    assert_eq!(lane("acme/widgets#31"), ("acme/widgets: Independent".into(), "independent".into()));
    assert_eq!(lane("acme/tools#40"), ("acme/tools: Independent".into(), "independent".into()));
    assert_eq!(lane("acme/tools#41"), ("acme/tools: Independent".into(), "independent".into()));
}

#[tokio::test]
async fn shared_short_repo_names_from_discovered_projects_keep_separate_lanes() {
    let h = H::new();
    h.fake.set_projects(vec![project("p-acme", "/src/acme"), project("p-fork", "/src/fork")]);
    h.remotes.set("/src/acme", "git@github.com:acme/widgets.git");
    h.remotes.set("/src/fork", "git@github.com:fork/widgets.git");
    issue(&h, "acme/widgets", 1, &[], 1);
    issue(&h, "acme/widgets", 2, &[], 2);
    issue(&h, "acme/widgets", 3, &[], 3);
    issue(&h, "fork/widgets", 1, &[], 1);
    issue(&h, "fork/widgets", 2, &[], 2);
    issue(&h, "fork/widgets", 3, &[], 3);
    h.github.set_parent("acme/widgets", 2, ("acme/widgets", 1));
    h.github.set_parent("fork/widgets", 2, ("fork/widgets", 1));

    let f = graph(&h).await;

    assert_eq!(node(&f, "acme/widgets#2")["track"], "acme/widgets#1");
    assert_eq!(node(&f, "fork/widgets#2")["track"], "fork/widgets#1");
    assert_eq!(node(&f, "acme/widgets#3")["track"], "acme/widgets: Independent");
    assert_eq!(node(&f, "fork/widgets#3")["track"], "fork/widgets: Independent");
}

#[tokio::test]
async fn graph_paginates_past_ten_pages_while_queue_reads_keep_label_filter() {
    let h = flowing(2);
    for n in 1..=12 {
        let lbls: &[&str] = if n == 12 { &[FLOW] } else { &[] };
        issue(&h, REPO, n, lbls, n as u32);
    }
    h.github.set_issue_page(1);

    let before_queue = h.github.calls().len();
    let f = flow(&h).await;
    let queue_reads: Vec<Value> = h.github.calls()[before_queue..]
        .iter()
        .filter(|c| c["operation"] == "flowIssues" && c["variables"]["name"] == "widgets")
        .cloned()
        .collect();
    assert_eq!(labels(&f["queue"]), ["widgets#12"]);
    assert_eq!(queue_reads.len(), 1, "queue read filters server-side by optIn.label: {queue_reads:?}");
    assert_eq!(queue_reads[0]["variables"]["label"], FLOW);

    let before_graph = h.github.calls().len();
    let g = graph(&h).await;
    let graph_reads: Vec<Value> = h.github.calls()[before_graph..]
        .iter()
        .filter(|c| c["operation"] == "flowIssues" && c["variables"]["name"] == "widgets")
        .cloned()
        .collect();
    assert_eq!(graph_reads.len(), 12, "graph read paginates through all 12 pages: {graph_reads:?}");
    assert!(graph_reads[0]["variables"].get("label").is_none());
    assert_eq!(g["graph"]["nodes"].as_array().unwrap().len(), 12);
}

#[tokio::test]
async fn an_identity_only_discovered_project_launches_ready_issues() {
    let cfg = Config::from_value(json!({"flow": {"maxTickets": 1, "launch": "auto"}})).unwrap();
    let h = H::build(Vec::new(), cfg);
    let mut tools_proj = project("p-tools", "/src/tools");
    tools_proj["repositoryIdentity"] = json!({
        "canonicalKey": "github.com/acme/tools", "owner": "acme", "name": "tools",
        "locator": {"source": "git-remote", "remoteName": "origin", "remoteUrl": "https://github.com/acme/tools.git"}
    });
    h.fake.set_projects(vec![tools_proj]);
    issue(&h, TOOLS, 7, &[], 1);

    let pick = next(&h).await;

    assert_eq!(pick["picked"][0]["launched"]["ok"], true, "{pick}");
    assert_eq!(h.fake.launched().len(), 1);
    assert_eq!(h.fake.launched()[0]["projectId"], "p-tools");
    assert_eq!(h.fake.launched()[0]["title"], "tools#7: Issue 7");
}

#[tokio::test]
async fn switching_to_graph_only_flow_retires_open_launch_cards() {
    let mut h = flowing(1);
    issue(&h, REPO, 1, &[FLOW], 1);
    let picked = next(&h).await;
    let id = picked["picked"][0]["card"].as_str().unwrap().to_string();
    assert_eq!(flow_cards(&h).len(), 1);

    h.config = Config::from_value(json!({"flow": {"excludeRepos": [TOOLS]}})).unwrap();
    h.reopen();
    reconcile(&h).await;

    assert!(flow_cards(&h).is_empty(), "stale launch card should be retired");
    let resolved = h.app.wf.cards_with(true).unwrap();
    let card = resolved.iter().find(|c| c.id == id).unwrap();
    assert_eq!(card.resolution.as_deref(), Some("flow.launch is off"));
}

#[tokio::test]
async fn padded_opt_in_label_matches_identically_in_queue_and_graph_reads() {
    let cfg = Config::from_value(json!({
        "flow": {"repos": [REPO], "optIn": {"label": " flow "}, "maxTickets": 2}
    }))
    .unwrap();
    let h = H::build(Vec::new(), cfg);
    issue(&h, REPO, 1, &[FLOW], 1);
    issue(&h, REPO, 2, &[], 2);

    let q = flow(&h).await;
    let g = graph(&h).await;

    assert_eq!(labels(&q["queue"]), ["widgets#1"]);
    assert_eq!(labels(&g["queue"]), ["widgets#1"]);
    assert_eq!(node(&g, "acme/widgets#1")["ready"], true);
    let queue_call = h.github.calls().into_iter().find(|c| c["operation"] == "flowIssues").unwrap();
    assert_eq!(queue_call["variables"]["label"], FLOW);
}

#[tokio::test]
async fn launch_routing_prefers_matching_origin_port_before_identity_fallback() {
    let cfg = Config::from_value(json!({"flow": {"repos": [REPO], "maxTickets": 1, "launch": "auto"}})).unwrap();
    let h = H::build(Vec::new(), cfg.clone());
    let mut wrong_port = project("p-port", "/src/port");
    wrong_port["repositoryIdentity"] = json!({"owner": "acme", "name": "widgets"});
    let right_origin = project("p-origin", "/src/origin");
    h.fake.set_projects(vec![wrong_port.clone(), right_origin]);
    h.remotes.set("/src/port", "https://github.com:8443/acme/widgets.git");
    h.remotes.set("/src/origin", "https://github.com/acme/widgets.git");
    issue(&h, REPO, 4, &[], 1);

    let pick = next(&h).await;

    assert_eq!(pick["picked"][0]["launched"]["ok"], true, "{pick}");
    assert_eq!(h.fake.launched()[0]["projectId"], "p-origin");

    // When only a mismatched-origin project exists, identity fallback must not select it.
    let h2 = H::build(Vec::new(), cfg);
    h2.fake.set_projects(vec![wrong_port]);
    h2.remotes.set("/src/port", "https://github.com:8443/acme/widgets.git");
    issue(&h2, REPO, 5, &[], 1);

    let pick2 = next(&h2).await;

    assert_eq!(pick2["picked"][0]["launched"]["ok"], false, "{pick2}");
    assert!(h2.fake.launched().is_empty());
}

#[tokio::test]
async fn local_mirror_origin_falls_back_to_github_identity_for_discovery_and_launch() {
    let mut mirror = project("p-mirror", "/src/mirror");
    mirror["repositoryIdentity"] = json!({"owner": "acme", "name": "widgets"});

    // Zero-config graph discovers the repo via repositoryIdentity when origin is a local path.
    let h_graph = H::build(Vec::new(), Config::default());
    h_graph.fake.set_projects(vec![mirror.clone()]);
    h_graph.remotes.set("/src/mirror", "/mirrors/widgets.git");
    issue(&h_graph, REPO, 1, &[], 1);

    let g = graph(&h_graph).await;
    assert_eq!(g["graph"]["nodes"].as_array().unwrap().len(), 1, "{g}");
    assert_eq!(node(&g, "acme/widgets#1")["repo"], REPO);

    // Auto-launch also falls back to repositoryIdentity when origin cannot be parsed as a remote.
    let cfg = Config::from_value(json!({"flow": {"repos": [REPO], "maxTickets": 1, "launch": "auto"}})).unwrap();
    let h_launch = H::build(Vec::new(), cfg);
    h_launch.fake.set_projects(vec![mirror]);
    h_launch.remotes.set("/src/mirror", "/mirrors/widgets.git");
    issue(&h_launch, REPO, 2, &[], 1);

    let pick = next(&h_launch).await;
    assert_eq!(pick["picked"][0]["launched"]["ok"], true, "{pick}");
    assert_eq!(h_launch.fake.launched()[0]["projectId"], "p-mirror");
}

#[tokio::test]
async fn single_segment_non_github_and_unavailable_origins_block_identity_fallback() {
    let mut ssh_proj = project("p-ssh", "/src/ssh");
    ssh_proj["repositoryIdentity"] = json!({"owner": "acme", "name": "widgets"});
    let mut timeout_proj = project("p-timeout", "/src/timeout");
    timeout_proj["repositoryIdentity"] = json!({"owner": "acme", "name": "widgets"});

    // Zero-config graph must not discover acme/widgets from either project.
    let h_graph = H::build(Vec::new(), Config::default());
    h_graph.fake.set_projects(vec![ssh_proj.clone(), timeout_proj.clone()]);
    h_graph.remotes.set("/src/ssh", "ssh://git@git.example.com/widgets.git");
    h_graph.remotes.set_unavailable("/src/timeout");
    issue(&h_graph, REPO, 1, &[], 1);

    let g = graph(&h_graph).await;
    assert_eq!(g["graph"]["nodes"], json!([]), "{g}");
    assert!(h_graph.github.calls().is_empty(), "{:?}", h_graph.github.calls());

    // Auto-launch must not route acme/widgets issues to either project.
    let cfg = Config::from_value(json!({"flow": {"repos": [REPO], "maxTickets": 1, "launch": "auto"}})).unwrap();
    let h_launch = H::build(Vec::new(), cfg);
    h_launch.fake.set_projects(vec![ssh_proj, timeout_proj]);
    h_launch.remotes.set("/src/ssh", "ssh://git@git.example.com/widgets.git");
    h_launch.remotes.set_unavailable("/src/timeout");
    issue(&h_launch, REPO, 2, &[], 1);

    let pick = next(&h_launch).await;
    assert_eq!(pick["picked"][0]["launched"]["ok"], false, "{pick}");
    assert!(h_launch.fake.launched().is_empty());
}

#[tokio::test]
async fn discovery_does_not_overwrite_cached_project_repo_for_legacy_tickets() {
    let fork = "fork/widgets";
    let cfg = Config::from_value(json!({"flow": {"includeRepos": [REPO], "maxTickets": 2, "launch": "ask"}})).unwrap();
    let h = H::build(Vec::new(), cfg);
    let mut proj = project("p-widgets", "/src/widgets");
    proj["repositoryIdentity"] = json!({"owner": "acme", "name": "widgets"});
    h.fake.set_projects(vec![proj]);
    h.remotes.set("/src/widgets", "git@github.com:acme/widgets.git");

    // Open a legacy ticket without a stored issue URL, placed in p-widgets, and learn its repo.
    let mut th = t3_codemode::fake::thread("mcp:legacy41", "running", T0);
    th["projectId"] = json!("p-widgets");
    h.fake.put(th);
    h.wf("mcp:legacy41", "join", json!({"parent": BOARD, "label": "widgets#41", "role": "ticket"})).await;
    h.app.wf.board_tick().await.unwrap();

    // Now the workspace origin points at fork/widgets while both repos have an open #41.
    h.remotes.set("/src/widgets", "git@github.com:fork/widgets.git");
    issue(&h, REPO, 41, &[], 1);
    issue(&h, fork, 41, &[], 2);

    let g = graph(&h).await;

    assert_eq!(g["running"].as_array().unwrap().len(), 1, "{g}");
    assert_eq!(g["running"][0]["repo"], REPO, "legacy ticket must stay on original repo: {g}");
    assert_eq!(g["queue"].as_array().unwrap().len(), 1, "{g}");
    assert_eq!(g["queue"][0]["repo"], fork, "fork#41 must remain queued instead of suppressed: {g}");
}

#[tokio::test]
async fn renamed_repo_routes_launch_applies_canonical_exclude_and_deduplicates_issues() {
    let old = "acme/old";
    let cfg = Config::from_value(json!({"flow": {"includeRepos": [REPO], "maxTickets": 2, "launch": "auto"}})).unwrap();
    let h = H::build(Vec::new(), cfg);
    h.fake.set_projects(vec![project("p-old", "/src/old")]);
    h.remotes.set("/src/old", "https://github.com/acme/old.git");
    h.github.alias_repo(old, REPO);
    issue(&h, REPO, 1, &[], 1);

    // Discovered acme/old and included acme/widgets both return acme/widgets#1: queue lists it once.
    let f = flow(&h).await;
    assert_eq!(labels(&f["queue"]), ["widgets#1"], "{f}");

    // Launch routing uses the verified canonical alias so p-old receives the launch.
    let pick = next(&h).await;
    assert_eq!(pick["picked"][0]["launched"]["ok"], true, "{pick}");
    assert_eq!(h.fake.launched()[0]["projectId"], "p-old");

    // Excluding the canonical name acme/widgets suppresses issues discovered via stale acme/old.
    let ex_cfg =
        Config::from_value(json!({"flow": {"excludeRepos": [REPO], "maxTickets": 1, "launch": "auto"}})).unwrap();
    let h2 = H::build(Vec::new(), ex_cfg);
    h2.fake.set_projects(vec![project("p-old", "/src/old")]);
    h2.remotes.set("/src/old", "https://github.com/acme/old.git");
    h2.github.alias_repo(old, REPO);
    issue(&h2, REPO, 2, &[], 1);

    let f2 = flow(&h2).await;
    assert_eq!(f2["queue"], json!([]), "{f2}");
    let pick2 = next(&h2).await;
    assert_eq!(pick2["picked"], json!([]), "{pick2}");
    assert!(h2.fake.launched().is_empty());
}

#[tokio::test]
async fn unreadable_discovered_repo_preserves_running_ticket_capacity() {
    let cfg = Config::from_value(json!({"flow": {"maxTickets": 1, "launch": "auto"}})).unwrap();
    let h = H::build(Vec::new(), cfg);
    h.fake.set_projects(vec![project("p-widgets", "/src/widgets"), project("p-tools", "/src/tools")]);
    h.remotes.set("/src/widgets", "git@github.com:acme/widgets.git");
    h.remotes.set("/src/tools", "git@github.com:acme/tools.git");
    // Only acme/tools is readable in FakeGitHub; acme/widgets returns a GraphQL error.
    issue(&h, TOOLS, 2, &[], 1);

    // Open running ticket in acme/widgets fills the single maxTickets slot.
    let mut th = t3_codemode::fake::thread("mcp:w1", "running", T0);
    th["projectId"] = json!("p-widgets");
    h.fake.put(th);
    h.wf(
        "mcp:w1",
        "join",
        json!({"parent": BOARD, "label": "widgets#1", "role": "ticket", "issue": "https://github.com/acme/widgets/issues/1"}),
    )
    .await;

    let pick = next(&h).await;
    assert_eq!(pick["free"], 0, "unreadable repo's running ticket must keep its slot: {pick}");
    assert_eq!(pick["picked"], json!([]), "{pick}");
    assert!(h.fake.launched().is_empty());
}

#[tokio::test]
async fn unavailable_origin_keeps_auto_launch_retryable_until_git_recovers() {
    let cfg = Config::from_value(json!({"flow": {"repos": [REPO], "maxTickets": 1, "launch": "auto"}})).unwrap();
    let h = H::build(Vec::new(), cfg.clone());
    let mut timeout_proj = project("p-timeout", "/src/timeout");
    timeout_proj["repositoryIdentity"] = json!({"owner": "acme", "name": "widgets"});
    h.fake.set_projects(vec![timeout_proj]);
    h.remotes.set_unavailable("/src/timeout");
    issue(&h, REPO, 1, &[], 1);

    let pick1 = next(&h).await;
    assert_eq!(pick1["picked"][0]["launched"]["ok"], false, "{pick1}");
    assert_eq!(pick1["picked"][0]["launched"]["retryable"], true, "{pick1}");
    assert!(h.fake.launched().is_empty());
    let f = flow(&h).await;
    assert_eq!(labels(&f["skipped"]), Vec::<String>::new(), "unavailable origin must not save skip: {f}");
    assert_eq!(labels(&f["queue"]), ["widgets#1"], "{f}");

    // Once git recovers, auto-launch retries and launches without any label/blocker change.
    h.remotes.set("/src/timeout", "git@github.com:acme/widgets.git");
    let pick2 = next(&h).await;
    assert_eq!(pick2["picked"][0]["launched"]["ok"], true, "{pick2}");
    assert_eq!(h.fake.launched()[0]["projectId"], "p-timeout");

    // Confirmed mismatch still saves a skip as before.
    let h_mismatch = H::build(Vec::new(), cfg);
    let mut ssh_proj = project("p-ssh", "/src/ssh");
    ssh_proj["repositoryIdentity"] = json!({"owner": "acme", "name": "widgets"});
    h_mismatch.fake.set_projects(vec![ssh_proj]);
    h_mismatch.remotes.set("/src/ssh", "ssh://git@git.example.com/widgets.git");
    issue(&h_mismatch, REPO, 2, &[], 1);

    let pick_m = next(&h_mismatch).await;
    assert_eq!(pick_m["picked"][0]["launched"]["ok"], false, "{pick_m}");
    assert_ne!(pick_m["picked"][0]["launched"]["retryable"], true, "{pick_m}");
    assert_eq!(labels(&flow(&h_mismatch).await["skipped"]), ["widgets#2"]);
}

#[tokio::test]
async fn recreated_old_repo_evicts_cached_alias_during_launch_routing() {
    let old = "acme/old";
    let h = H::build(Vec::new(), Config::default());
    h.fake.set_projects(vec![project("p-old", "/src/old")]);
    h.remotes.set("/src/old", "https://github.com/acme/old.git");
    h.github.alias_repo(old, REPO);
    issue(&h, REPO, 1, &[], 1);

    // Graph read caches acme/old -> acme/widgets.
    let g = graph(&h).await;
    assert_eq!(node(&g, "acme/widgets#1")["repo"], REPO);

    // Owner later recreates an unrelated repo at acme/old, ending the redirect.
    h.github.unalias_repo(old);
    h.github.add_repo(old);

    let res = h
        .app
        .wf
        .launch_ticket(t3_codemode::board::LaunchArgs {
            issue: format!("{REPO}#7"),
            parent_issue: None,
            part: None,
            project: None,
            title: None,
            model: None,
            effort: None,
        })
        .await
        .unwrap();
    assert_eq!(res["ok"], false, "recreated old repo must not match stale alias: {res}");
    assert!(h.fake.launched().is_empty());
}

#[tokio::test]
async fn pre_rename_open_ticket_matches_canonical_issue_in_plan_graph_and_launch_guard() {
    let old = "acme/old";
    let cfg = Config::from_value(json!({"flow": {"maxTickets": 1, "launch": "auto"}})).unwrap();
    let h = H::build(Vec::new(), cfg);
    h.fake.set_projects(vec![project("p-old", "/src/old")]);
    h.remotes.set("/src/old", "https://github.com/acme/old.git");
    h.github.alias_repo(old, REPO);
    issue(&h, REPO, 1, &[], 1);
    issue(&h, REPO, 2, &[], 2);

    // Existing open ticket still stores the pre-rename URL acme/old#1.
    let mut th = t3_codemode::fake::thread("mcp:old1", "running", T0);
    th["projectId"] = json!("p-old");
    h.fake.put(th);
    h.wf(
        "mcp:old1",
        "join",
        json!({"parent": BOARD, "label": "old#1", "role": "ticket", "issue": "https://github.com/acme/old/issues/1"}),
    )
    .await;

    let g = graph(&h).await;
    assert_eq!(g["running"].as_array().unwrap().len(), 1, "{g}");
    assert_eq!(g["running"][0]["repo"], REPO, "{g}");
    assert_eq!(g["free"], 0, "{g}");
    assert_eq!(labels(&g["queue"]), ["widgets#2"], "widgets#1 must not re-enter queue: {g}");
    assert_eq!(node(&g, "acme/widgets#1")["ticket"]["thread"], "mcp:old1", "{g}");
    assert!(
        g["graph"]["nodes"].as_array().unwrap().iter().all(|n| n["id"] != "acme/old#1"),
        "graph must attach ticket to canonical node without duplicate orphan: {g}"
    );

    // Duplicate-launch guard also blocks launching acme/widgets#1 while acme/old#1 is open.
    let dup = h
        .app
        .wf
        .launch_ticket(t3_codemode::board::LaunchArgs {
            issue: format!("{REPO}#1"),
            parent_issue: None,
            part: None,
            project: None,
            title: None,
            model: None,
            effort: None,
        })
        .await;
    assert!(dup.is_err(), "duplicate launch of renamed issue must fail: {dup:?}");
    assert!(h.fake.launched().is_empty());
}

#[tokio::test]
async fn ambiguous_legacy_ticket_without_metadata_reserves_capacity_without_attaching() {
    let cfg = Config::from_value(json!({"flow": {"maxTickets": 1, "launch": "auto"}})).unwrap();
    let h = H::build(Vec::new(), cfg);
    h.fake.set_projects(vec![project("p-acme", "/src/acme"), project("p-fork", "/src/fork")]);
    h.remotes.set("/src/acme", "git@github.com:acme/widgets.git");
    h.remotes.set("/src/fork", "git@github.com:fork/widgets.git");
    // Only acme/widgets is readable; fork/widgets returns a GraphQL error.
    issue(&h, REPO, 41, &[], 1);
    issue(&h, REPO, 42, &[], 2);

    // Legacy running ticket has no stored issue, repo, or project metadata.
    ticket(&h, "mcp:ambig41", "widgets#41").await;

    let f = flow(&h).await;
    assert_eq!(f["free"], 0, "ambiguous legacy ticket must reserve capacity: {f}");
    assert_eq!(labels(&f["queue"]), ["widgets#41", "widgets#42"], "ambiguous ticket must not attach to acme#41: {f}");
    let pick = next(&h).await;
    assert_eq!(pick["free"], 0, "{pick}");
    assert_eq!(pick["picked"], json!([]), "{pick}");
    assert!(h.fake.launched().is_empty());
}

#[tokio::test]
async fn unavailable_origin_during_discovery_preserves_running_ticket_capacity() {
    let cfg = Config::from_value(json!({"flow": {"maxTickets": 1, "launch": "auto"}})).unwrap();
    let h = H::build(Vec::new(), cfg);
    h.fake.set_projects(vec![project("p-widgets", "/src/widgets"), project("p-tools", "/src/tools")]);
    h.remotes.set_unavailable("/src/widgets");
    h.remotes.set("/src/tools", "git@github.com:acme/tools.git");
    issue(&h, TOOLS, 2, &[], 1);

    // Open running ticket occupies the single slot on p-widgets whose origin lookup timed out.
    let mut th = t3_codemode::fake::thread("mcp:w1", "running", T0);
    th["projectId"] = json!("p-widgets");
    h.fake.put(th);
    h.wf(
        "mcp:w1",
        "join",
        json!({"parent": BOARD, "label": "widgets#1", "role": "ticket", "issue": "https://github.com/acme/widgets/issues/1"}),
    )
    .await;

    // Partial graph still renders acme/tools#2, while capacity stays occupied so nothing launches.
    let g = graph(&h).await;
    assert_eq!(node(&g, "acme/tools#2")["title"], "Issue 2", "{g}");
    assert_eq!(g["free"], 0, "unavailable discovery must preserve running ticket slot: {g}");
    let pick = next(&h).await;
    assert_eq!(pick["free"], 0, "{pick}");
    assert_eq!(pick["picked"], json!([]), "{pick}");
    assert!(h.fake.launched().is_empty());
}

#[tokio::test]
async fn pending_ticket_uses_saved_project_under_unavailable_discovery() {
    let cfg = Config::from_value(json!({"flow": {"maxTickets": 1, "launch": "auto"}})).unwrap();
    let h = H::build(Vec::new(), cfg.clone());
    h.fake.set_projects(vec![
        project("p-gitlab", "/src/gitlab"),
        project("p-timeout", "/src/timeout"),
        project("p-tools", "/src/tools"),
    ]);
    h.remotes.set("/src/gitlab", "git@gitlab.com:acme/internal.git");
    h.remotes.set_unavailable("/src/timeout");
    h.remotes.set("/src/tools", "git@github.com:acme/tools.git");
    issue(&h, TOOLS, 2, &[], 1);

    // A pending dispatch saved with confirmed non-GitHub p-gitlab must not consume the GitHub slot.
    let params = h.app.wf.config().link_params("overseer", "ticket").unwrap();
    h.app
        .store
        .pending_put(&Pending {
            parent: BOARD.into(),
            label: "gitlab: refactor".into(),
            role: "ticket".into(),
            params,
            at: T0.into(),
            reported: false,
            thread: None,
            link: None,
            project: Some("p-gitlab".into()),
            identity: Default::default(),
            brief: None,
        })
        .unwrap();

    let f = flow(&h).await;
    assert_eq!(f["free"], 1, "non-GitHub pending dispatch must not consume slot: {f}");

    // A pending dispatch saved with unavailable p-timeout must still reserve capacity.
    let h2 = H::build(Vec::new(), cfg);
    h2.fake.set_projects(vec![project("p-timeout", "/src/timeout"), project("p-tools", "/src/tools")]);
    h2.remotes.set_unavailable("/src/timeout");
    h2.remotes.set("/src/tools", "git@github.com:acme/tools.git");
    issue(&h2, TOOLS, 3, &[], 1);
    h2.app
        .store
        .pending_put(&Pending {
            parent: BOARD.into(),
            label: "timeout: task".into(),
            role: "ticket".into(),
            params,
            at: T0.into(),
            reported: false,
            thread: None,
            link: None,
            project: Some("p-timeout".into()),
            identity: Default::default(),
            brief: None,
        })
        .unwrap();

    let pick2 = next(&h2).await;
    assert_eq!(pick2["free"], 0, "pending ticket on unavailable project must keep slot: {pick2}");
    assert_eq!(pick2["picked"], json!([]), "{pick2}");
    assert!(h2.fake.launched().is_empty());
}

#[tokio::test]
async fn body_dependencies_block_the_queue_emit_body_edges_and_skip_fenced_code() {
    let h = flowing(4);
    issue(&h, REPO, 1, &[], 1); // Unlabelled epic
    issue(&h, REPO, 2, &[], 2); // Unlabelled open blocker
    issue(&h, REPO, 3, &[FLOW], 3); // Blocked by #2 via prose + Flow Track
    issue(&h, REPO, 4, &[FLOW], 4); // Blocked by cross-repo tools#7 via ## Flow + fenced noise
    issue(&h, REPO, 5, &[FLOW], 5); // Both native + body blocker on #2 (native wins)
    issue(&h, REPO, 6, &[FLOW], 6); // Ready: only mentions Depends on #2 inside fenced & inline code
    issue(&h, TOOLS, 7, &[], 7); // Cross-repo blocker
    issue(&h, REPO, 8, &[FLOW], 8); // Child via #1 tasklist + After #2

    h.github.set_body(
        REPO,
        1,
        "## Tasks\n- [ ] #8 Child task\n- [ ] Write docs for #999\n\n## Flow\n- Track: flow-dag\n",
    );
    h.github.set_body(
        REPO,
        3,
        "Depends on #2 (Phase 1).\nPhase P1 of #1.\n\n## Flow\n- Parent: #1\n- Track: flow-dag\n- Touches: `src/flow.rs`, `src/board.html`\n",
    );
    h.github.set_body(
        REPO,
        4,
        "```markdown\nDepends on #999\n## Flow\n- Track: fake-track\n```\n~~~\nBlocked by #888\n~~~\n\n## Flow\n- Blocked by: acme/tools#7\n- Track: sync-track\n",
    );
    h.github.set_blockers(REPO, 5, &[(REPO, 2)]);
    h.github.set_body(REPO, 5, "Blocked by #2\n\n## Flow\n- Blocked by: #2\n");
    h.github.set_body(
        REPO,
        6,
        "Docs note: write `Depends on #2` or ``Blocked by #7``.\n<!-- Depends on #2 -->\n```text\nDepends on #2\n```\nAfter merging, check the logs.\n",
    );
    h.github.set_body(REPO, 8, "After #2\n");

    // Queue-only read (wf.flow() with optIn.label) must still block on unlabelled body blockers (#2 and tools#7).
    let q = flow(&h).await;
    assert_eq!(labels(&q["queue"]), ["widgets#6"], "only #6 is unblocked: {q}");
    assert_eq!(labels(&q["blocked"]), ["widgets#3", "widgets#4", "widgets#5", "widgets#8"], "{q}");
    let b4 = q["blocked"].as_array().unwrap().iter().find(|x| x["label"] == "widgets#4").unwrap();
    assert_eq!(labels(&b4["blockers"]), ["tools#7"]);

    // Full graph read carries source provenance, Flow Track lanes, tasklist parent, and zero false warnings.
    let g = graph(&h).await;
    assert_eq!(g["graph"]["warnings"], json!([]), "clean DAG must have no warnings: {g}");
    assert_eq!(
        (node(&g, "acme/widgets#3")["track"].as_str(), node(&g, "acme/widgets#3")["trackBy"].as_str()),
        (Some("flow-dag"), Some("label"))
    );
    assert_eq!(
        (node(&g, "acme/widgets#4")["track"].as_str(), node(&g, "acme/widgets#4")["trackBy"].as_str()),
        (Some("sync-track"), Some("label"))
    );
    assert_eq!(node(&g, "acme/widgets#8")["parent"], "acme/widgets#1");
    assert_eq!(
        g["graph"]["edges"],
        json!([
            {"from": "acme/widgets#3", "to": "acme/widgets#2", "kind": "blockedBy", "source": "body"},
            {"from": "acme/widgets#3", "to": "acme/widgets#1", "kind": "parent", "source": "body"},
            {"from": "acme/widgets#4", "to": "acme/tools#7", "kind": "blockedBy", "source": "body"},
            {"from": "acme/widgets#5", "to": "acme/widgets#2", "kind": "blockedBy", "source": "github"},
            {"from": "acme/widgets#8", "to": "acme/widgets#2", "kind": "blockedBy", "source": "body"},
            {"from": "acme/widgets#8", "to": "acme/widgets#1", "kind": "parent", "source": "body"},
        ])
    );

    // Closing #2 unblocks #3, #5, and #8 in both wf.flow() and graph, preserving body edge + unblockedBy.
    h.github.close_issue(REPO, 2);
    let q2 = flow(&h).await;
    assert_eq!(labels(&q2["queue"]), ["widgets#3", "widgets#5", "widgets#6", "widgets#8"], "{q2}");
    let q3 = q2["queue"].as_array().unwrap().iter().find(|x| x["label"] == "widgets#3").unwrap();
    assert_eq!(labels(&q3["unblockedBy"]), ["widgets#2"]);
    let g2 = graph(&h).await;
    assert_eq!(node(&g2, "acme/widgets#2")["state"], "closed");
    assert_eq!(node(&g2, "acme/widgets#3")["ready"], true);
}

#[tokio::test]
async fn plain_blocked_label_holds_issue_with_diagnostic_while_external_keeps_its_behavior() {
    let h = flowing(4);
    issue(&h, REPO, 1, &[FLOW, "blocked"], 1);
    issue(&h, REPO, 2, &[FLOW, "Blocked"], 2);
    issue(&h, REPO, 3, &[FLOW], 3);
    issue(&h, REPO, 4, &[FLOW, "blocked: external"], 4);
    h.github.set_body(REPO, 2, "Depends on #3");

    let g = graph(&h).await;
    assert_eq!(labels(&g["queue"]), ["widgets#3"], "{g}");
    assert_eq!(labels(&g["blocked"]), ["widgets#1", "widgets#2", "widgets#4"], "{g}");

    let b = |lbl: &str| g["blocked"].as_array().unwrap().iter().find(|x| x["label"] == lbl).unwrap();
    assert_eq!(b("widgets#1")["blockedLabel"], true);
    assert_eq!(b("widgets#1")["external"], false);
    assert_eq!(b("widgets#1")["blockers"], json!([]));
    assert_eq!(b("widgets#2")["blockedLabel"], true);
    assert_eq!(labels(&b("widgets#2")["blockers"]), ["widgets#3"]);
    assert_eq!(b("widgets#4")["external"], true);
    assert!(b("widgets#4").get("blockedLabel").is_none());

    // Only #1 has no open blocker linked, so only #1 gets the blockedLabel diagnostic.
    assert_eq!(
        node(&g, "acme/widgets#1")["warnings"],
        json!([{"kind": "blockedLabel", "message": "Labelled \"blocked\" with no open blocker linked"}])
    );
    assert_eq!(node(&g, "acme/widgets#2")["warnings"], json!([]));
    assert_eq!(node(&g, "acme/widgets#4")["warnings"], json!([]));
    assert_eq!(
        g["graph"]["warnings"],
        json!([{
            "kind": "blockedLabel",
            "message": "widgets#1: Labelled \"blocked\" with no open blocker linked",
            "nodes": ["acme/widgets#1"]
        }])
    );

    // Once #3 closes, #2 still stays blocked by its plain `blocked` label and now gets the diagnostic.
    h.github.close_issue(REPO, 3);
    let g2 = graph(&h).await;
    assert_eq!(labels(&g2["queue"]), Vec::<String>::new());
    assert_eq!(labels(&g2["blocked"]), ["widgets#1", "widgets#2", "widgets#4"]);
    assert_eq!(node(&g2, "acme/widgets#2")["warnings"][0]["kind"], "blockedLabel");
}

#[tokio::test]
async fn sibling_title_and_phase_references_resolve_unambiguously_and_flag_unresolved_refs() {
    let h = flowing(4);
    // Epic #10 with unlabelled Phase 1 (#11) and flow-opted Phase 2 (#12) & title-ref (#13).
    h.github.set_issue(REPO, github_issue(10, "Epic: DAG", &[], "2026-10-01T09:00:00Z"));
    h.github.set_issue(REPO, github_issue(11, "feat: zero-config graph", &[], "2026-10-02T09:00:00Z"));
    h.github.set_issue(REPO, github_issue(12, "feat: body parser", &[FLOW], "2026-10-03T09:00:00Z"));
    h.github.set_issue(REPO, github_issue(13, "feat: UI edges", &[FLOW], "2026-10-04T09:00:00Z"));
    h.github.set_body(REPO, 11, "## Scope (Phase 1 of #10)\n");
    h.github.set_body(REPO, 12, "Part of #10\nDepends on Phase 1\n");
    h.github.set_body(REPO, 13, "Part of #10\nBlocked by \"zero-config graph\"\n");

    // Second epic #20 also has a Phase 1 (#21) so repo-wide "Phase 1" is ambiguous.
    h.github.set_issue(REPO, github_issue(20, "Epic: Auth", &[], "2026-10-05T09:00:00Z"));
    h.github.set_issue(REPO, github_issue(21, "Phase 1: auth schema", &[], "2026-10-06T09:00:00Z"));
    h.github.set_issue(REPO, github_issue(22, "Phase 2: auth UI", &[FLOW], "2026-10-07T09:00:00Z"));
    h.github.set_body(REPO, 21, "Part of #20\n");
    h.github.set_body(REPO, 22, "Part of #20\nDepends on Phase 1\n");

    // Unparented #30 references ambiguous "Phase 1"; #31 references nonexistent "Phase 9"; #32 references missing #999.
    h.github.set_issue(REPO, github_issue(30, "Ambiguous dep", &[FLOW], "2026-10-08T09:00:00Z"));
    h.github.set_issue(REPO, github_issue(31, "Missing phase", &[FLOW], "2026-10-09T09:00:00Z"));
    h.github.set_issue(REPO, github_issue(32, "Missing issue num", &[FLOW], "2026-10-10T09:00:00Z"));
    h.github.set_body(REPO, 30, "Depends on Phase 1\n");
    h.github.set_body(REPO, 31, "Depends on Phase 9\n");
    h.github.set_body(REPO, 32, "Depends on #999\n");

    // Both queue-only wf.flow() and full graph must resolve unlabelled siblings and block unresolved issues.
    let q = flow(&h).await;
    let g = graph(&h).await;
    assert_eq!(labels(&q["queue"]), Vec::<String>::new(), "{q}");
    assert_eq!(
        labels(&q["blocked"]),
        ["widgets#12", "widgets#13", "widgets#22", "widgets#30", "widgets#31", "widgets#32"]
    );
    assert_eq!(labels(&g["blocked"]), labels(&q["blocked"]));

    // Epic-scoped resolution bound #12 and #13 -> #11, and #22 -> #21.
    assert_eq!(ids(&node(&g, "acme/widgets#12")["blockedBy"]), ["acme/widgets#11"]);
    assert_eq!(ids(&node(&g, "acme/widgets#13")["blockedBy"]), ["acme/widgets#11"]);
    assert_eq!(ids(&node(&g, "acme/widgets#22")["blockedBy"]), ["acme/widgets#21"]);
    assert_eq!(node(&g, "acme/widgets#12")["warnings"], json!([]));
    assert_eq!(node(&g, "acme/widgets#13")["warnings"], json!([]));
    assert_eq!(node(&g, "acme/widgets#22")["warnings"], json!([]));

    // Ambiguous and missing references raise explicit diagnostics without creating phantom #999.
    assert!(g["graph"]["nodes"].as_array().unwrap().iter().all(|n| n["id"] != "acme/widgets#999"));
    assert_eq!(node(&g, "acme/widgets#30")["warnings"][0]["kind"], "unresolved");
    assert!(
        node(&g, "acme/widgets#30")["warnings"][0]["message"].as_str().unwrap().contains("Ambiguous"),
        "{}",
        node(&g, "acme/widgets#30")["warnings"]
    );
    assert_eq!(node(&g, "acme/widgets#31")["warnings"][0]["raw"], "Phase 9");
    assert_eq!(node(&g, "acme/widgets#32")["warnings"][0]["raw"], "#999");
    let graph_warns = g["graph"]["warnings"].as_array().unwrap();
    assert_eq!(graph_warns.len(), 3, "{graph_warns:?}");
}

#[tokio::test]
async fn cycles_unresolved_refs_and_edge_sources_surface_in_graph_warnings_and_board_ui() {
    let h = flowing(4);
    issue(&h, REPO, 1, &[FLOW], 1);
    issue(&h, REPO, 2, &[FLOW], 2);
    issue(&h, REPO, 3, &[FLOW], 3);
    issue(&h, REPO, 4, &[FLOW], 4);
    issue(&h, REPO, 5, &[FLOW, "blocked"], 5);
    issue(&h, REPO, 6, &[FLOW], 6);
    issue(&h, REPO, 7, &[FLOW], 7);
    issue(&h, REPO, 8, &[FLOW], 8);

    h.github.set_body(REPO, 1, "Depends on #2\n");
    h.github.set_body(REPO, 2, "Depends on #1\n");
    h.github.set_body(REPO, 3, "Depends on #3\n");
    h.github.set_body(REPO, 4, "Depends on Phase 9\n");
    h.github.set_blockers(REPO, 6, &[(REPO, 7)]);
    h.github.set_body(REPO, 8, "Depends on #7\n");

    let (html, st, view) = panel_page(h).await;
    assert_eq!(labels(&view["queue"]), ["widgets#7"], "{view}");
    assert_eq!(
        labels(&view["blocked"]),
        ["widgets#1", "widgets#2", "widgets#3", "widgets#4", "widgets#5", "widgets#6", "widgets#8"]
    );
    let warns = view["graph"]["warnings"].as_array().unwrap();
    assert_eq!(warns.len(), 4, "2 cycles + 1 unresolved + 1 blockedLabel: {warns:?}");
    assert!(warns.iter().any(
        |w| w["kind"] == "cycle" && w["message"].as_str().unwrap().contains("widgets#1 -> widgets#2 -> widgets#1")
    ));
    assert!(
        warns
            .iter()
            .any(|w| w["kind"] == "cycle" && w["message"].as_str().unwrap().contains("widgets#3 depends on itself"))
    );
    assert!(warns.iter().any(|w| w["kind"] == "unresolved" && w["raw"] == "Phase 9"));
    assert!(warns.iter().any(|w| w["kind"] == "blockedLabel" && w["nodes"] == json!(["acme/widgets#5"])));

    let r = panel(
        &html,
        &st,
        &view,
        &format!(
            r##"{GRAPH}
fetchReply = (url) => url === "/api/flow" ? reply(200, view) : new Promise(() => {{}});
emitState(base);
await tick();
const blockedRows = Object.fromEntries(rows("flow-b").map((row) => [row.label, row.why]));
selectTab("graph");
const banner = {{
  hidden: q("#graph-warn").hidden,
  head: q("#graph-warn .gwarn-h") ? q("#graph-warn .gwarn-h").textContent : null,
  items: document.querySelectorAll("#graph-warn li").map((li) => [li.dataset.kind, li.textContent]),
}};
const edgeMeta = document.querySelectorAll(".ge").map((p) => [p.getAttribute("class"), p.getAttribute("data-source")]);
const warnBadge = (id) => {{
  const b = btn(id);
  const w = b.querySelector('[data-part="warn"]');
  return {{ warnClass: b.classList.contains("warn"), hidden: w.hidden, text: w.textContent }};
}};
const badges = {{
  cycle: warnBadge("acme/widgets#1"),
  selfCycle: warnBadge("acme/widgets#3"),
  unresolved: warnBadge("acme/widgets#4"),
  blockedLabel: warnBadge("acme/widgets#5"),
  clean: warnBadge("acme/widgets#7"),
}};
btn("acme/widgets#8").dispatch("click");
const bodyDepDrawer = {{ deps: deps(), warnsHidden: dp("deps-w").hidden }};
btn("acme/widgets#4").dispatch("click");
const warnDrawer = {{
  warnsHidden: dp("deps-w").hidden,
  warns: document.querySelectorAll('#drawer [data-part="deps-w"] li').map((li) => li.textContent),
  note: dp("note-m").textContent,
}};
btn("acme/widgets#8").dispatch("pointerover");
const hoverBodyEdge = document.querySelectorAll(".ge").map((p) => p.getAttribute("class"));
// Clean payload hides the warning banner again.
paintFlow({{ ...view, graph: {{ ...view.graph, warnings: [] }} }});
const clearedHidden = q("#graph-warn").hidden;
return {{ blockedRows, banner, edgeMeta, badges, bodyDepDrawer, warnDrawer, hoverBodyEdge, clearedHidden }};
"##
        ),
    );

    assert_eq!(r["blockedRows"]["widgets#4"], "unresolved: Phase 9 · priority: none", "{r}");
    assert_eq!(r["blockedRows"]["widgets#5"], "labelled blocked (no blocker linked) · priority: none", "{r}");
    assert_eq!(r["banner"]["hidden"], false, "{r}");
    assert_eq!(
        r["banner"]["head"], "Graph diagnostics: 2 cycles, 1 unresolved reference, 1 unlinked blocked label",
        "{r}"
    );
    assert_eq!(r["banner"]["items"].as_array().unwrap().len(), 4, "{r}");
    assert!(
        r["edgeMeta"].as_array().unwrap().contains(&json!(["ge", "github"]))
            && r["edgeMeta"].as_array().unwrap().contains(&json!(["ge src-body", "body"])),
        "native and body edges render with distinct classes and data-source: {r}"
    );
    assert_eq!(r["badges"]["cycle"], json!({"warnClass": true, "hidden": false, "text": "cycle"}), "{r}");
    assert_eq!(r["badges"]["selfCycle"], json!({"warnClass": true, "hidden": false, "text": "cycle"}), "{r}");
    assert_eq!(r["badges"]["unresolved"], json!({"warnClass": true, "hidden": false, "text": "unresolved"}), "{r}");
    assert_eq!(r["badges"]["blockedLabel"], json!({"warnClass": true, "hidden": false, "text": "warning"}), "{r}");
    assert_eq!(r["badges"]["clean"], json!({"warnClass": false, "hidden": true, "text": ""}), "{r}");
    assert_eq!(r["bodyDepDrawer"], json!({"deps": ["Blocked bywidgets#7readybody"], "warnsHidden": true}), "{r}");
    assert_eq!(r["warnDrawer"]["warnsHidden"], false, "{r}");
    assert_eq!(r["warnDrawer"]["warns"], json!(["Unresolved blocker reference \"Phase 9\""]), "{r}");
    assert!(r["hoverBodyEdge"].as_array().unwrap().contains(&json!("ge src-body up")), "{r}");
    assert_eq!(r["clearedHidden"], true, "{r}");
}

#[tokio::test]
async fn body_blocker_changes_invalidate_skip_memory() {
    let h = flowing(1);
    issue(&h, REPO, 1, &[FLOW], 1);
    issue(&h, REPO, 2, &[], 2);
    let first = next(&h).await;
    let id = first["picked"][0]["card"].as_str().unwrap().to_string();
    h.app.wf.answer(&id, "skip").await.unwrap();
    assert_eq!(labels(&flow(&h).await["skipped"]), ["widgets#1"]);

    // Adding an open body blocker moves #1 to blocked and clears stale skip when it later closes.
    h.github.set_body(REPO, 1, "Depends on #2\n");
    assert_eq!(labels(&flow(&h).await["blocked"]), ["widgets#1"]);
    h.github.close_issue(REPO, 2);
    assert_eq!(labels(&flow(&h).await["queue"]), ["widgets#1"]);
}

#[tokio::test]
async fn closing_a_symbolic_sibling_unblocks_its_dependent_without_rebinding_or_warning() {
    let h = flowing(4);
    // Epic #10 with Phase 1 (#11) and Phase 2 (#12, Depends on Phase 1) and title-ref (#13).
    h.github.set_issue(REPO, github_issue(10, "Epic: DAG", &[], "2026-10-01T09:00:00Z"));
    h.github.set_issue(REPO, github_issue(11, "feat: zero-config graph", &[], "2026-10-02T09:00:00Z"));
    h.github.set_issue(REPO, github_issue(12, "feat: body parser", &[FLOW], "2026-10-03T09:00:00Z"));
    h.github.set_issue(REPO, github_issue(13, "feat: UI edges", &[FLOW], "2026-10-04T09:00:00Z"));
    h.github.set_body(REPO, 11, "## Scope (Phase 1 of #10)\n");
    h.github.set_body(REPO, 12, "Part of #10\nDepends on Phase 1\n");
    h.github.set_body(REPO, 13, "Part of #10\nBlocked by \"zero-config graph\"\n");

    // Second open Phase 1 in another epic must never steal resolution when #11 closes, and an
    // unparented issue referencing ambiguous "Phase 1" must stay ambiguous after #11 closes.
    h.github.set_issue(REPO, github_issue(20, "Epic: Other", &[], "2026-10-05T09:00:00Z"));
    h.github.set_issue(REPO, github_issue(21, "Phase 1: other work", &[], "2026-10-06T09:00:00Z"));
    h.github.set_issue(REPO, github_issue(30, "Ambiguous dep", &[FLOW], "2026-10-07T09:00:00Z"));
    h.github.set_body(REPO, 21, "Part of #20\n");
    h.github.set_body(REPO, 30, "Depends on Phase 1\n");

    // While #11 is open, #12 and #13 are blocked by #11.
    assert_eq!(labels(&flow(&h).await["queue"]), Vec::<String>::new());

    // Closing #11 unblocks #12 and #13 cleanly in both wf.flow() and graph, while #30 stays ambiguous.
    h.github.close_issue(REPO, 11);
    let q = flow(&h).await;
    assert_eq!(labels(&q["queue"]), ["widgets#12", "widgets#13"], "{q}");
    assert_eq!(labels(&q["queue"][0]["unblockedBy"]), ["widgets#11"]);
    assert_eq!(labels(&q["queue"][1]["unblockedBy"]), ["widgets#11"]);
    assert_eq!(labels(&q["blocked"]), ["widgets#30"], "ambiguous ref must not rebind to #21: {q}");

    let g = graph(&h).await;
    assert_eq!(labels(&g["queue"]), ["widgets#12", "widgets#13"], "{g}");
    assert_eq!(node(&g, "acme/widgets#11")["state"], "closed");
    assert_eq!(ids(&node(&g, "acme/widgets#12")["blockedBy"]), ["acme/widgets#11"]);
    assert_eq!(node(&g, "acme/widgets#12")["warnings"], json!([]));
    assert_eq!(node(&g, "acme/widgets#13")["warnings"], json!([]));
    assert_eq!(node(&g, "acme/widgets#30")["warnings"][0]["kind"], "unresolved");
}

#[tokio::test]
async fn active_unlabelled_epic_tasklist_suppresses_its_child_in_queue_and_launch() {
    let h = flowing_with(2, "auto");
    // Unlabelled epic #10 lists #11 in a tasklist; #11 has optIn label but no Parent: line of its own.
    issue(&h, REPO, 10, &[], 1);
    issue(&h, REPO, 11, &[FLOW], 2);
    issue(&h, REPO, 12, &[FLOW], 3);
    h.github.set_body(REPO, 10, "## Tasks\n- [ ] #11 Build parser\n");
    // Also verify trailing prose after an explicit ref (#10) does not leave #12 blocked after #10 closes.
    h.github.set_body(REPO, 12, "Depends on #10, which sets up the epic.\n");
    ticket(&h, "mcp:t10", "widgets#10").await;

    // Both wf.flow() and wf.flow({ graph: true }) must suppress #11 while #10 has an active ticket.
    let q = flow(&h).await;
    let g = graph(&h).await;
    assert_eq!(labels(&q["queue"]), Vec::<String>::new(), "wf.flow() must suppress tasklist child #11: {q}");
    assert_eq!(labels(&g["queue"]), Vec::<String>::new(), "graph must suppress tasklist child #11: {g}");
    assert_eq!(labels(&q["blocked"]), ["widgets#12"], "{q}");
    assert_eq!(node(&g, "acme/widgets#12")["warnings"], json!([]), "trailing prose must not warn: {g}");

    // wf.flow.next() must not launch #11 either.
    let pick = next(&h).await;
    assert_eq!(pick["picked"], json!([]), "auto launch must not pick tasklist child #11: {pick}");
    assert!(h.fake.launched().is_empty());

    // Closing #10 on GitHub while its ticket is still active unblocks #12 (blocker) while
    // keeping #11 suppressed consistently across wf.flow(), wf.flow({ graph: true }), and next().
    h.github.close_issue(REPO, 10);
    let q2 = flow(&h).await;
    let g2 = graph(&h).await;
    assert_eq!(labels(&q2["queue"]), ["widgets#12"], "{q2}");
    assert_eq!(labels(&g2["queue"]), ["widgets#12"], "graph queue must match wf.flow(): {g2}");
    assert_eq!(labels(&q2["queue"][0]["unblockedBy"]), ["widgets#10"]);
    assert_eq!(node(&g2, "acme/widgets#11")["ready"], false);

    // Once #10's board ticket hands off, auto-launch picks up both #11 and #12 consistently.
    h.wf("mcp:t10", "send", json!({"kind": "handoff", "body": "done"})).await;
    let q3 = flow(&h).await;
    let g3 = graph(&h).await;
    assert_eq!(labels(&q3["running"]), ["widgets#11", "widgets#12"], "{q3}");
    assert_eq!(labels(&g3["running"]), ["widgets#11", "widgets#12"], "{g3}");
    assert_eq!(labels(&q3["queue"]), Vec::<String>::new(), "{q3}");
    assert_eq!(labels(&g3["queue"]), Vec::<String>::new(), "{g3}");
}

#[tokio::test]
async fn closed_symbolic_sibling_paginates_past_100_and_closed_epic_never_overrides_open_epic() {
    let h = flowing(4);
    // S1: older closed blocker #1 ("Setup auth") buried behind 105 newer closed issues.
    h.github.set_issue(REPO, github_issue(1, "Setup auth", &[], "2026-01-01T00:00:00Z"));
    h.github.close_issue(REPO, 1);
    for n in 100..205u64 {
        let day = ((n - 100) % 28) + 1;
        let month = ((n - 100) / 28) + 2;
        h.github.set_issue(
            REPO,
            github_issue(n, &format!("Closed noise {n}"), &[], &format!("2026-{month:02}-{day:02}T00:00:00Z")),
        );
        h.github.close_issue(REPO, n);
    }
    h.github.set_issue(REPO, github_issue(2, "Use auth", &[FLOW], "2026-10-01T09:00:00Z"));
    h.github.set_body(REPO, 2, "Depends on \"Setup auth\"\n");

    // S7: older closed epic #3 lists closed Phase 1 (#4) AND open child #12 in its tasklist,
    // while current open epic #10 lists open Phase 1 (#11) and open child #12.
    h.github.set_issue(REPO, github_issue(3, "Old closed epic", &[], "2026-01-02T00:00:00Z"));
    h.github.set_body(REPO, 3, "- [x] #4\n- [x] #12\n");
    h.github.close_issue(REPO, 3);
    h.github.set_issue(REPO, github_issue(4, "Phase 1: old", &[], "2026-01-03T00:00:00Z"));
    h.github.close_issue(REPO, 4);

    h.github.set_issue(REPO, github_issue(10, "Current open epic", &[], "2026-10-02T09:00:00Z"));
    h.github.set_body(REPO, 10, "- [ ] #11\n- [ ] #12\n");
    h.github.set_issue(REPO, github_issue(11, "Phase 1: current", &[], "2026-10-03T09:00:00Z"));
    h.github.set_issue(REPO, github_issue(12, "Phase 2: current", &[FLOW], "2026-10-04T09:00:00Z"));
    h.github.set_body(REPO, 12, "Depends on Phase 1\n");

    let q = flow(&h).await;
    let g = graph(&h).await;
    // #2 resolves older closed #1 past 100 newer closed issues and is ready with zero warnings.
    assert_eq!(labels(&q["queue"]), ["widgets#2"], "{q}");
    assert_eq!(labels(&q["queue"][0]["unblockedBy"]), ["widgets#1"]);
    assert_eq!(node(&g, "acme/widgets#2")["warnings"], json!([]));
    // #12 stays blocked by open #11 (from open epic #10), never redirected to closed #4 via closed epic #3.
    assert_eq!(labels(&q["blocked"]), ["widgets#12"], "{q}");
    assert_eq!(labels(&q["blocked"][0]["blockers"]), ["widgets#11"]);
    assert_eq!(labels(&g["queue"]), ["widgets#2"], "{g}");
    assert_eq!(ids(&node(&g, "acme/widgets#12")["blockedBy"]), ["acme/widgets#11"]);
    assert_eq!(node(&g, "acme/widgets#12")["parent"], "acme/widgets#10");
}

#[tokio::test]
async fn sole_closed_epic_context_open_epic_precedence_and_after_prose_work_consistently() {
    let h = flowing(6);

    // S9: closed epic #1 is the sole parent of closed Phase 1 (#2) and open Phase 2 (#3);
    // another epic #20 also has Phase 1 (#21), yet #3 resolves cleanly to #2 via its sole epic #1.
    h.github.set_issue(REPO, github_issue(1, "Closed sole epic", &[], "2026-01-01T00:00:00Z"));
    h.github.set_body(REPO, 1, "- [x] #2\n- [ ] #3\n");
    h.github.close_issue(REPO, 1);
    h.github.set_issue(REPO, github_issue(2, "Phase 1: old init", &[], "2026-01-02T00:00:00Z"));
    h.github.close_issue(REPO, 2);
    h.github.set_issue(REPO, github_issue(3, "Phase 2: follow-up", &[FLOW], "2026-01-03T00:00:00Z"));
    h.github.set_body(REPO, 3, "Depends on Phase 1\n");

    h.github.set_issue(REPO, github_issue(20, "Other open epic", &[], "2026-02-01T00:00:00Z"));
    h.github.set_body(REPO, 20, "- [ ] #21\n- [ ] #30\n");
    h.github.set_issue(REPO, github_issue(21, "Phase 1: other", &[], "2026-02-02T00:00:00Z"));

    // S10: closed epic #10 with an active ticket AND open unlabelled epic #20 both list #30;
    // both wf.flow() and wf.flow({ graph: true }) must pick open epic #20 and mark #30 ready.
    h.github.set_issue(REPO, github_issue(10, "Old closed ticket epic", &[], "2026-01-04T00:00:00Z"));
    h.github.set_body(REPO, 10, "- [x] #30\n");
    h.github.close_issue(REPO, 10);
    ticket(&h, "mcp:t10", "widgets#10").await;
    h.github.set_issue(REPO, github_issue(30, "Moved child", &[FLOW], "2026-02-03T00:00:00Z"));

    // S6 + S11: colonless `After #42, Setup auth` resolves Setup auth (#43) as a real blocker;
    // `After #42, the board renders edges.` and `Depends on #42, please review before merging.`
    // stay unblocked once #42 closes; `Depends on Setup auth, Do database migration` keeps both.
    h.github.set_issue(REPO, github_issue(42, "Closed base", &[], "2026-03-01T00:00:00Z"));
    h.github.close_issue(REPO, 42);
    h.github.set_issue(REPO, github_issue(43, "Setup auth", &[], "2026-03-02T00:00:00Z"));
    h.github.set_issue(REPO, github_issue(44, "Do database migration", &[], "2026-03-03T00:00:00Z"));

    h.github.set_issue(REPO, github_issue(50, "Needs auth via After", &[FLOW], "2026-03-04T00:00:00Z"));
    h.github.set_body(REPO, 50, "After #42, Setup auth\n");

    h.github.set_issue(REPO, github_issue(51, "After with trailing prose", &[FLOW], "2026-03-05T00:00:00Z"));
    h.github.set_body(REPO, 51, "After #42, the board renders edges.\n");

    h.github.set_issue(REPO, github_issue(52, "Depends with trailing prose", &[FLOW], "2026-03-06T00:00:00Z"));
    h.github.set_body(REPO, 52, "Depends on #42, please review before merging.\n");

    h.github.set_issue(REPO, github_issue(53, "Pure title list", &[FLOW], "2026-03-07T00:00:00Z"));
    h.github.set_body(REPO, 53, "Depends on Setup auth, Do database migration\n");

    let q = flow(&h).await;
    let g = graph(&h).await;

    // Ready queue agrees across wf.flow() and wf.flow({ graph: true }): #3 (S9), #30 (S10), #51 (S6 prose), #52 (S11 prose).
    assert_eq!(labels(&q["queue"]), ["widgets#3", "widgets#30", "widgets#51", "widgets#52"], "{q}");
    assert_eq!(labels(&g["queue"]), ["widgets#3", "widgets#30", "widgets#51", "widgets#52"], "{g}");
    assert_eq!(labels(&q["queue"][0]["unblockedBy"]), ["widgets#2"]);
    assert_eq!(node(&g, "acme/widgets#3")["warnings"], json!([]));
    assert_eq!(node(&g, "acme/widgets#30")["ready"], true);
    assert_eq!(node(&g, "acme/widgets#30")["parent"], "acme/widgets#20");
    assert_eq!(node(&g, "acme/widgets#30")["track"], "widgets#20");
    assert_eq!(node(&g, "acme/widgets#51")["warnings"], json!([]));
    assert_eq!(node(&g, "acme/widgets#52")["warnings"], json!([]));

    // #50 stays blocked by open #43 ("Setup auth"); #53 stays blocked by both #43 and #44 ("Do database migration").
    assert_eq!(labels(&q["blocked"]), ["widgets#50", "widgets#53"], "{q}");
    assert_eq!(labels(&q["blocked"][0]["blockers"]), ["widgets#43"]);
    assert_eq!(labels(&q["blocked"][1]["blockers"]), ["widgets#43", "widgets#44"]);
    assert_eq!(ids(&node(&g, "acme/widgets#50")["blockedBy"]), ["acme/widgets#42", "acme/widgets#43"]);
    assert_eq!(ids(&node(&g, "acme/widgets#53")["blockedBy"]), ["acme/widgets#43", "acme/widgets#44"]);
}

#[tokio::test]
async fn failed_sibling_read_and_out_of_range_issue_numbers_keep_dependents_safe() {
    let h = flowing_with(4, "auto");
    // #1 is closed; #2 is open unlabelled ("Setup auth"); #3 is open unlabelled blocker in another repo.
    h.github.set_issue(REPO, github_issue(1, "Base", &[], "2026-03-01T00:00:00Z"));
    h.github.close_issue(REPO, 1);
    h.github.set_issue(REPO, github_issue(2, "Setup auth", &[], "2026-03-02T00:00:00Z"));
    h.github.set_issue(TOOLS, github_issue(7, "External tool", &[], "2026-03-02T00:00:00Z"));

    // #10 has an out-of-range issue number alongside a real open cross-repo blocker: only the
    // invalid number becomes unresolved, while the batch lookup for tools#7 still succeeds.
    h.github.set_issue(REPO, github_issue(10, "Huge ref", &[FLOW], "2026-03-03T00:00:00Z"));
    h.github.set_body(REPO, 10, "Depends on #9999999999999, acme/tools#7\n");

    // #11 uses `After #1, Setup auth`. When the supplemental unlabelled OPEN read fails, Flow
    // must NOT treat `OptionalSymbolic::NotFound` as trailing prose or launch #11.
    h.github.set_issue(REPO, github_issue(11, "After failed open read", &[FLOW], "2026-03-04T00:00:00Z"));
    h.github.set_body(REPO, 11, "After #1, Setup auth\n");

    // #12 uses a required symbolic ref `After #1, "Dup auth"` where closed #3 and open unlabelled
    // #4 share the title "Dup auth". When the open-sibling read fails, Flow must NOT accept closed
    // #3 as definitive and launch #12 while #4 is open.
    h.github.set_issue(REPO, github_issue(3, "Dup auth", &[], "2026-03-01T10:00:00Z"));
    h.github.close_issue(REPO, 3);
    h.github.set_issue(REPO, github_issue(4, "Dup auth", &[], "2026-03-02T10:00:00Z"));
    h.github.set_issue(REPO, github_issue(12, "Required symbolic failed open read", &[FLOW], "2026-03-05T00:00:00Z"));
    h.github.set_body(REPO, 12, "After #1, \"Dup auth\"\n");

    h.github.fail_unlabelled_open_issues(Some("transient GraphQL error on open siblings"));

    let q = flow(&h).await;
    assert_eq!(labels(&q["queue"]), Vec::<String>::new(), "none of #10, #11, #12 may enter queue: {q}");
    assert_eq!(labels(&q["blocked"]), ["widgets#10", "widgets#11", "widgets#12"], "{q}");
    assert_eq!(labels(&q["blocked"][0]["blockers"]), ["tools#7"]);
    assert_eq!(q["blocked"][0]["unresolved"], json!(["#9999999999999"]));
    assert_eq!(q["blocked"][1]["unresolved"], json!(["Setup auth"]));
    assert_eq!(q["blocked"][2]["unresolved"], json!(["Dup auth"]));

    // wf.flow.next() must refuse to launch #11 or #12 while the supplemental open-sibling read failed.
    let pick = next(&h).await;
    assert_eq!(pick["picked"], json!([]), "{pick}");
    assert!(h.fake.launched().is_empty());

    // Once the supplemental open read recovers, #10 resolves tools#7, #11 resolves open #2, and
    // #12 stays blocked as ambiguous between closed #3 and open #4.
    h.github.fail_unlabelled_open_issues(None);
    let q2 = flow(&h).await;
    assert_eq!(labels(&q2["blocked"]), ["widgets#10", "widgets#11", "widgets#12"], "{q2}");
    assert_eq!(labels(&q2["blocked"][1]["blockers"]), ["widgets#2"]);
    assert!(q2["blocked"][1].get("unresolved").is_none());
    assert_eq!(q2["blocked"][2]["unresolved"], json!(["Dup auth"]));
}

#[tokio::test]
async fn board_sync_links_materializes_body_edges_idempotently_and_refreshes_graph() {
    let h = flowing(4);
    issue(&h, REPO, 10, &[], 1);
    issue(&h, REPO, 1, &[FLOW], 2);
    issue(&h, TOOLS, 7, &[], 3);
    issue(&h, REPO, 2, &[FLOW], 4);
    issue(&h, REPO, 3, &[FLOW], 5);

    h.github.set_body(
        REPO,
        2,
        "## Goal\nSync links.\n\n## Flow\n- Parent: #10\n- Blocked by: #1, acme/tools#7\n- Track: flow-dag\n",
    );
    // Issue #3 filed via GitHub Issue Form headings (`### Parent / Epic`, `### Blocked by`).
    h.github.set_body(
        REPO,
        3,
        "### Problem or use case\nFollow-up.\n\n### Parent / Epic\n\n#10\n\n### Blocked by\n\n#2\n\n### Track\n\n_No response_\n",
    );

    let s = crate::board::start(h).await;

    // Before sync, all 5 edges are source: "body".
    let before = s.get("/api/flow").await;
    let body_before: Vec<&Value> =
        before["graph"]["edges"].as_array().unwrap().iter().filter(|e| e["source"] == "body").collect();
    assert_eq!(body_before.len(), 5, "{body_before:?}");

    // Route enforces CSRF, same-origin loopback Origin, and request body validation.
    let no_csrf = s
        .http
        .post(format!("{}/api/flow/sync-links", s.base))
        .header("Origin", &s.base)
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(no_csrf.status(), 403);

    let bad_origin = s
        .http
        .post(format!("{}/api/flow/sync-links", s.base))
        .header("Origin", "http://evil.example")
        .header("X-T3CM-CSRF", s.csrf())
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(bad_origin.status(), 403);

    let bad_json = s
        .http
        .post(format!("{}/api/flow/sync-links", s.base))
        .header("Origin", &s.base)
        .header("X-T3CM-CSRF", s.csrf())
        .body("not-json")
        .send()
        .await
        .unwrap();
    assert_eq!(bad_json.status(), 400);

    let unknown_field = s
        .http
        .post(format!("{}/api/flow/sync-links", s.base))
        .header("Origin", &s.base)
        .header("X-T3CM-CSRF", s.csrf())
        .json(&json!({"extra": true}))
        .send()
        .await
        .unwrap();
    assert_eq!(unknown_field.status(), 400);

    // Valid POST syncs all 5 body edges and returns refreshed graph with source: "github".
    let resp = s
        .http
        .post(format!("{}/api/flow/sync-links", s.base))
        .header("Origin", &s.base)
        .header("X-T3CM-CSRF", s.csrf())
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let synced: Value = resp.json().await.unwrap();
    assert_eq!(synced["ok"], true, "{synced}");
    assert_eq!(synced["total"], 5, "{synced}");
    assert_eq!(synced["synced"], 5, "{synced}");
    assert_eq!(synced["alreadyLinked"], 0, "{synced}");
    assert_eq!(synced["failed"], 0, "{synced}");
    assert_eq!(synced["errors"], json!([]), "{synced}");
    assert!(synced.get("refreshError").is_none(), "{synced}");

    // Sub-issue mutations send GraphQL-Features: sub_issues header.
    let sub_calls: Vec<Value> = s.h.github.calls().into_iter().filter(|c| c["operation"] == "addSubIssue").collect();
    assert_eq!(sub_calls.len(), 2, "{sub_calls:?}");
    for c in &sub_calls {
        assert_eq!(c["headers"]["GraphQL-Features"], "sub_issues", "{c}");
    }

    // Refreshed graph in response and fresh GET /api/flow both report every edge as source: "github".
    let refreshed_edges = synced["flow"]["graph"]["edges"].as_array().unwrap();
    assert_eq!(refreshed_edges.len(), 5, "{refreshed_edges:?}");
    assert!(refreshed_edges.iter().all(|e| e["source"] == "github"), "{refreshed_edges:?}");
    let after = s.get("/api/flow").await;
    assert!(after["graph"]["edges"].as_array().unwrap().iter().all(|e| e["source"] == "github"), "{after}");

    // Second POST (with empty body) is completely idempotent and sends zero mutations.
    let calls_before = s.h.github.calls().len();
    let again_resp = s
        .http
        .post(format!("{}/api/flow/sync-links", s.base))
        .header("Origin", &s.base)
        .header("X-T3CM-CSRF", s.csrf())
        .send()
        .await
        .unwrap();
    assert_eq!(again_resp.status(), 200);
    let again: Value = again_resp.json().await.unwrap();
    assert_eq!(again["ok"], true, "{again}");
    assert_eq!(again["total"], 0, "{again}");
    assert_eq!(again["synced"], 0, "{again}");
    assert_eq!(again["failed"], 0, "{again}");
    let mut_ops: Vec<String> =
        s.h.github
            .calls()
            .into_iter()
            .skip(calls_before)
            .filter(|c| matches!(c["operation"].as_str(), Some("addBlockedBy" | "addSubIssue")))
            .map(|c| c["operation"].as_str().unwrap().to_string())
            .collect();
    assert!(mut_ops.is_empty(), "idempotent repeat must send no mutations: {mut_ops:?}");
}

#[tokio::test]
async fn board_sync_links_reports_permission_errors_parent_conflicts_and_self_loops_per_link() {
    let h = flowing(4);
    issue(&h, REPO, 1, &[], 1);
    issue(&h, REPO, 10, &[], 2);
    issue(&h, REPO, 20, &[], 3);
    issue(&h, REPO, 2, &[FLOW], 4);
    issue(&h, REPO, 3, &[FLOW], 5);
    issue(&h, REPO, 4, &[FLOW], 6);

    // #2 already has native parent #10; body adds blocker #1 (should succeed) and conflicting parent #20.
    h.github.set_parent(REPO, 2, (REPO, 10));
    h.github.set_body(REPO, 2, "## Flow\n- Parent: #20\n- Blocked by: #1\n");
    // #3 has NO native parent, but TWO body parents (#10 and #20): #10 links, #20 conflicts with #10.
    h.github.set_body(REPO, 3, "## Flow\n- Parent: #10, #20\n");
    // #4 has a self-loop blocker (must error without calling addBlockedBy on itself).
    h.github.set_body(REPO, 4, "Depends on #4\n");

    let s = crate::board::start(h).await;
    let r: Value = s
        .http
        .post(format!("{}/api/flow/sync-links", s.base))
        .header("Origin", &s.base)
        .header("X-T3CM-CSRF", s.csrf())
        .json(&json!({}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(r["ok"], false, "{r}");
    assert_eq!(r["total"], 5, "{r}");
    assert_eq!(r["synced"], 2, "#2->#1 blocker and #3->#10 parent succeed: {r}");
    assert_eq!(r["failed"], 3, "#2->#20 conflict, #3->#20 conflict, #4->#4 self-loop: {r}");
    let errs = r["errors"].as_array().unwrap();
    assert_eq!(errs.len(), 3, "{errs:?}");

    let c2 = errs.iter().find(|e| e["from"] == "acme/widgets#2" && e["to"] == "acme/widgets#20").unwrap();
    assert_eq!(c2["kind"], "parent");
    assert_eq!(c2["permission"], false);
    assert_eq!(c2["conflict"], true);
    assert_eq!(c2["existingParent"], "acme/widgets#10");

    let c3 = errs.iter().find(|e| e["from"] == "acme/widgets#3" && e["to"] == "acme/widgets#20").unwrap();
    assert_eq!(c3["kind"], "parent");
    assert_eq!(c3["permission"], false);
    assert_eq!(c3["conflict"], true);
    assert_eq!(c3["existingParent"], "acme/widgets#10");

    let self_err = errs.iter().find(|e| e["from"] == "acme/widgets#4" && e["to"] == "acme/widgets#4").unwrap();
    assert_eq!(self_err["kind"], "blockedBy");
    assert_eq!(self_err["permission"], false);
    assert_eq!(self_err["conflict"], false);
    assert!(self_err["message"].as_str().unwrap().contains("blocked by itself"), "{self_err}");

    // Refreshed graph shows #2->#1 and #3->#10 upgraded to native "github".
    let edges = r["flow"]["graph"]["edges"].as_array().unwrap();
    assert!(
        edges.contains(
            &json!({"from": "acme/widgets#2", "to": "acme/widgets#1", "kind": "blockedBy", "source": "github"})
        )
    );
    assert!(
        edges.contains(
            &json!({"from": "acme/widgets#3", "to": "acme/widgets#10", "kind": "parent", "source": "github"})
        )
    );
    assert!(s.h.github.issue(REPO, 4).unwrap()["blockers"].as_array().unwrap().is_empty());

    // Now test FORBIDDEN permission error on addBlockedBy while addSubIssue still works.
    issue(&s.h, REPO, 5, &[FLOW], 7);
    s.h.github.set_body(REPO, 5, "## Flow\n- Parent: #10\n- Blocked by: #1\n");
    s.h.github.fail_operation_with_type("addBlockedBy", "FORBIDDEN", "Resource not accessible by integration");
    let perm_res: Value = s
        .http
        .post(format!("{}/api/flow/sync-links", s.base))
        .header("Origin", &s.base)
        .header("X-T3CM-CSRF", s.csrf())
        .json(&json!({}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let perm_err = perm_res["errors"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["from"] == "acme/widgets#5" && e["to"] == "acme/widgets#1")
        .unwrap();
    assert_eq!(perm_err["permission"], true, "{perm_err}");
    assert_eq!(perm_err["conflict"], false, "{perm_err}");
    assert_eq!(perm_err["op"], "addBlockedBy");
    assert!(
        perm_res["flow"]["graph"]["edges"].as_array().unwrap().contains(
            &json!({"from": "acme/widgets#5", "to": "acme/widgets#10", "kind": "parent", "source": "github"})
        ),
        "parent edge on #5 still synced despite blockedBy FORBIDDEN: {perm_res}"
    );

    // When GitHub is completely unreachable on initial read, endpoint returns 502.
    s.h.github.set_down(true);
    let down = s
        .http
        .post(format!("{}/api/flow/sync-links", s.base))
        .header("Origin", &s.base)
        .header("X-T3CM-CSRF", s.csrf())
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(down.status(), 502);
}

#[tokio::test]
async fn board_sync_links_writes_graph_primary_parent_and_conflicts_lower_precedence_candidates() {
    let h = flowing(4);
    issue(&h, REPO, 10, &[], 1);
    issue(&h, REPO, 20, &[], 2);
    issue(&h, REPO, 3, &[FLOW], 3);
    // Epic #10 lists #3 in its tasklist (sorts before #20), while #3 declares Parent: #20.
    // The graph picks #20 as #3's primary parent; sync must write #20 and report #10 as a conflict.
    h.github.set_body(REPO, 10, "- [ ] #3\n");
    h.github.set_body(REPO, 3, "## Flow\n- Parent: #20\n");

    let s = crate::board::start(h).await;
    let before = s.get("/api/flow").await;
    assert_eq!(node(&before, "acme/widgets#3")["parent"], "acme/widgets#20", "{before}");

    let r: Value = s
        .http
        .post(format!("{}/api/flow/sync-links", s.base))
        .header("Origin", &s.base)
        .header("X-T3CM-CSRF", s.csrf())
        .json(&json!({}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(r["ok"], false, "{r}");
    assert_eq!(r["total"], 2, "{r}");
    assert_eq!(r["synced"], 1, "{r}");
    assert_eq!(r["failed"], 1, "{r}");
    assert_eq!(
        s.h.github.issue(REPO, 3).unwrap()["parentRef"],
        json!([REPO, 20]),
        "primary parent #20 must be written to GitHub: {r}"
    );
    let errs = r["errors"].as_array().unwrap();
    assert_eq!(errs.len(), 1, "{errs:?}");
    assert_eq!(errs[0]["from"], "acme/widgets#3");
    assert_eq!(errs[0]["to"], "acme/widgets#10");
    assert_eq!(errs[0]["kind"], "parent");
    assert_eq!(errs[0]["conflict"], true);
    assert_eq!(errs[0]["existingParent"], "acme/widgets#20");
    let edges = r["flow"]["graph"]["edges"].as_array().unwrap();
    assert!(
        edges.contains(
            &json!({"from": "acme/widgets#3", "to": "acme/widgets#20", "kind": "parent", "source": "github"})
        ),
        "{edges:?}"
    );

    // If the primary parent fails with a permission error, sync must NOT fall back to writing the secondary candidate.
    issue(&s.h, REPO, 30, &[], 4);
    issue(&s.h, REPO, 4, &[FLOW], 5);
    s.h.github.set_body(REPO, 10, "- [ ] #3\n- [ ] #4\n");
    s.h.github.set_body(REPO, 4, "## Flow\n- Parent: #30\n");
    s.h.github.fail_operation_with_type("addSubIssue", "FORBIDDEN", "Resource not accessible by integration");

    let denied: Value = s
        .http
        .post(format!("{}/api/flow/sync-links", s.base))
        .header("Origin", &s.base)
        .header("X-T3CM-CSRF", s.csrf())
        .json(&json!({}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(s.h.github.issue(REPO, 4).unwrap()["parentRef"], Value::Null, "{denied}");
    let d_errs = denied["errors"].as_array().unwrap();
    let e30 = d_errs.iter().find(|e| e["from"] == "acme/widgets#4" && e["to"] == "acme/widgets#30").unwrap();
    assert_eq!(e30["permission"], true, "{e30}");
    let e10 = d_errs.iter().find(|e| e["from"] == "acme/widgets#4" && e["to"] == "acme/widgets#10").unwrap();
    assert_eq!(e10["conflict"], true, "{e10}");
    assert!(e10.get("existingParent").is_none(), "{e10}");

    // S6: if GitHub acquires a secondary parent candidate between the graph snapshot and readIssue,
    // sync must report that secondary candidate as alreadyLinked rather than conflicting with itself.
    struct ParentRaceGitHub {
        inner: std::sync::Arc<t3_codemode::fake::FakeGitHub>,
        triggered: std::sync::atomic::AtomicBool,
    }
    impl t3_codemode::github::GitHub for ParentRaceGitHub {
        fn graphql<'a>(
            &'a self,
            query: &'a str,
            variables: Value,
        ) -> t3_codemode::t3::BoxFut<'a, anyhow::Result<Value>> {
            self.graphql_with_headers(query, variables, &[])
        }
        fn graphql_with_headers<'a>(
            &'a self,
            query: &'a str,
            variables: Value,
            headers: &'a [(&'a str, &'a str)],
        ) -> t3_codemode::t3::BoxFut<'a, anyhow::Result<Value>> {
            if query.starts_with("query IssueNode(") && !self.triggered.swap(true, std::sync::atomic::Ordering::SeqCst)
            {
                self.inner.set_parent(REPO, 3, (REPO, 10));
            }
            self.inner.graphql_with_headers(query, variables, headers)
        }
    }

    let rh = flowing(4);
    issue(&rh, REPO, 10, &[], 1);
    issue(&rh, REPO, 20, &[], 2);
    issue(&rh, REPO, 3, &[FLOW], 3);
    rh.github.set_body(REPO, 10, "- [ ] #3\n");
    rh.github.set_body(REPO, 3, "## Flow\n- Parent: #20\n");
    let racy_gh = std::sync::Arc::new(ParentRaceGitHub {
        inner: rh.github.clone(),
        triggered: std::sync::atomic::AtomicBool::new(false),
    });
    let racy_app = t3_codemode::app::App::with_host_and_tailscale(
        rh.config.clone(),
        std::sync::Arc::new(t3_codemode::store::Store::open(rh.dir.path()).unwrap()),
        rh.fake.clone(),
        rh.clock.clone(),
        rh.remotes.clone(),
        racy_gh,
        rh.desktop.clone(),
        rh.tailscale.clone(),
    );
    let raced = racy_app.wf.sync_flow_links().await.unwrap();
    assert!(!raced.ok);
    assert_eq!(raced.total, 2);
    assert_eq!(raced.synced, 0);
    assert_eq!(raced.already_linked, 1);
    assert_eq!(raced.failed, 1);
    assert_eq!(raced.errors.len(), 1);
    assert_eq!(raced.errors[0].to, "acme/widgets#20");
    assert!(raced.errors[0].conflict);
    assert_eq!(raced.errors[0].existing_parent.as_deref(), Some("acme/widgets#10"));
    let r10 = raced.results.iter().find(|r| r.to == "acme/widgets#10").unwrap();
    assert_eq!(r10.status, "alreadyLinked");
    assert!(!r10.conflict);

    // When the graph already selected a native parent (#20) and the body mentions a conflicting parent (#10),
    // sync must never promote #10 even if native parent #20 is removed between the graph snapshot and readIssue.
    struct NativeParentRemovedRaceGitHub {
        inner: std::sync::Arc<t3_codemode::fake::FakeGitHub>,
        triggered: std::sync::atomic::AtomicBool,
    }
    impl t3_codemode::github::GitHub for NativeParentRemovedRaceGitHub {
        fn graphql<'a>(
            &'a self,
            query: &'a str,
            variables: Value,
        ) -> t3_codemode::t3::BoxFut<'a, anyhow::Result<Value>> {
            self.graphql_with_headers(query, variables, &[])
        }
        fn graphql_with_headers<'a>(
            &'a self,
            query: &'a str,
            variables: Value,
            headers: &'a [(&'a str, &'a str)],
        ) -> t3_codemode::t3::BoxFut<'a, anyhow::Result<Value>> {
            if query.starts_with("query IssueNode(") && !self.triggered.swap(true, std::sync::atomic::Ordering::SeqCst)
            {
                self.inner.update_issue(REPO, 3, json!({"parentRef": Value::Null}));
            }
            self.inner.graphql_with_headers(query, variables, headers)
        }
    }

    let nh = flowing(4);
    issue(&nh, REPO, 10, &[], 1);
    issue(&nh, REPO, 20, &[], 2);
    issue(&nh, REPO, 3, &[FLOW], 3);
    nh.github.set_parent(REPO, 3, (REPO, 20));
    nh.github.set_body(REPO, 3, "## Flow\n- Parent: #10\n");
    let native_gh = std::sync::Arc::new(NativeParentRemovedRaceGitHub {
        inner: nh.github.clone(),
        triggered: std::sync::atomic::AtomicBool::new(false),
    });
    let native_app = t3_codemode::app::App::with_host_and_tailscale(
        nh.config.clone(),
        std::sync::Arc::new(t3_codemode::store::Store::open(nh.dir.path()).unwrap()),
        nh.fake.clone(),
        nh.clock.clone(),
        nh.remotes.clone(),
        native_gh,
        nh.desktop.clone(),
        nh.tailscale.clone(),
    );
    let native_res = native_app.wf.sync_flow_links().await.unwrap();
    assert!(!native_res.ok);
    assert_eq!(native_res.synced, 0);
    assert_eq!(native_res.failed, 1);
    assert_eq!(native_res.errors[0].to, "acme/widgets#10");
    assert!(native_res.errors[0].conflict);
    assert_eq!(nh.github.issue(REPO, 3).unwrap()["parentRef"], Value::Null);
    assert!(
        !nh.github.operations().iter().any(|op| op == "addSubIssue"),
        "sync must never send addSubIssue when graph chose a native parent: {:?}",
        nh.github.operations()
    );
}

#[tokio::test]
async fn board_sync_links_enforces_remote_auth_and_csrf() {
    let cfg = Config::from_value(json!({
        "board": {"remote": {"mode": "tailscale", "origin": "https://box.acme.ts.net:47732"}},
        "flow": {"repos": [REPO], "optIn": {"label": FLOW}, "maxTickets": 2}
    }))
    .unwrap();
    let h = H::build(Vec::new(), cfg);
    h.tailscale.set_host_login("alice@acme.example");
    h.github.add_repo(REPO);
    issue(&h, REPO, 1, &[], 1);
    issue(&h, REPO, 2, &[FLOW], 2);
    h.github.set_body(REPO, 2, "Depends on #1\n");
    let s = crate::board::start(h).await;

    // Missing Tailscale header -> 401.
    let unauth = s
        .http
        .post(format!("{}/api/flow/sync-links", s.base))
        .header("Host", "box.acme.ts.net:47732")
        .header("Origin", "https://box.acme.ts.net:47732")
        .header("X-Forwarded-Proto", "https")
        .header("X-T3CM-CSRF", s.csrf())
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(unauth.status(), 401);

    // Disallowed user -> 403.
    let denied = s
        .http
        .post(format!("{}/api/flow/sync-links", s.base))
        .header("Host", "box.acme.ts.net:47732")
        .header("Origin", "https://box.acme.ts.net:47732")
        .header("X-Forwarded-Proto", "https")
        .header("Tailscale-User-Login", "bob@acme.example")
        .header("X-T3CM-CSRF", s.csrf())
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), 403);

    // Allowed user without CSRF -> 403.
    let no_csrf = s
        .http
        .post(format!("{}/api/flow/sync-links", s.base))
        .header("Host", "box.acme.ts.net:47732")
        .header("Origin", "https://box.acme.ts.net:47732")
        .header("X-Forwarded-Proto", "https")
        .header("Tailscale-User-Login", "alice@acme.example")
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(no_csrf.status(), 403);

    // Allowed user with CSRF -> 200 and upgrades edge to native GitHub.
    let ok = s
        .http
        .post(format!("{}/api/flow/sync-links", s.base))
        .header("Host", "box.acme.ts.net:47732")
        .header("Origin", "https://box.acme.ts.net:47732")
        .header("X-Forwarded-Proto", "https")
        .header("Tailscale-User-Login", "alice@acme.example")
        .header("X-T3CM-CSRF", s.csrf())
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(ok.status(), 200);
    let body: Value = ok.json().await.unwrap();
    assert_eq!(body["ok"], true, "{body}");
    assert_eq!(body["synced"], 1, "{body}");
    assert_eq!(body["flow"]["graph"]["edges"][0]["source"], "github");
}

#[tokio::test]
async fn graph_tab_sync_button_syncs_body_links_updates_edges_and_shows_errors() {
    let h = flowing(4);
    issue(&h, REPO, 10, &[], 1);
    issue(&h, REPO, 1, &[FLOW], 2);
    issue(&h, REPO, 2, &[FLOW], 3);
    h.github.set_body(REPO, 2, "## Flow\n- Parent: #10\n- Blocked by: #1\n");

    let s = crate::board::start(h).await;
    let html = s.http.get(format!("{}/", s.base)).send().await.unwrap().text().await.unwrap();
    let st = s.get("/api/state").await;
    let before_view = s.get("/api/flow").await;

    // Run real sync on the server to get an authentic synced payload for the browser script.
    let synced_res: Value = s
        .http
        .post(format!("{}/api/flow/sync-links", s.base))
        .header("Origin", &s.base)
        .header("X-T3CM-CSRF", s.csrf())
        .json(&json!({}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    // Build a partial-failure payload (1 synced, 1 permission error, 1 parent conflict, 1 body edge left).
    let partial_view = {
        let mut v = before_view.clone();
        v["graph"]["edges"] = json!([
            {"from": "acme/widgets#2", "to": "acme/widgets#1", "kind": "blockedBy", "source": "github"},
            {"from": "acme/widgets#2", "to": "acme/widgets#10", "kind": "parent", "source": "body"},
        ]);
        v
    };
    let partial_res = json!({
        "ok": false,
        "total": 3,
        "synced": 1,
        "alreadyLinked": 0,
        "failed": 2,
        "results": [],
        "errors": [
            {
                "from": "acme/widgets#2",
                "to": "acme/widgets#10",
                "kind": "parent",
                "op": "addSubIssue",
                "message": "Sub-issue already has a parent",
                "permission": false,
                "conflict": true,
                "existingParent": "acme/widgets#1"
            },
            {
                "from": "acme/widgets#2",
                "to": "acme/widgets#1",
                "kind": "blockedBy",
                "op": "addBlockedBy",
                "message": "Resource not accessible by integration",
                "permission": true,
                "conflict": false
            }
        ],
        "flow": partial_view,
    });

    let r = panel(
        &html,
        &st,
        &before_view,
        &format!(
            r##"{GRAPH}
const syncedRes = {synced_res};
const partialRes = {partial_res};
let mode = "ok";
const syncReqs = [];
fetchReply = (url, opts) => {{
  if (url === "/api/flow") return reply(200, view);
  if (url === "/api/flow/sync-links") {{
    syncReqs.push({{ method: opts && opts.method, csrf: opts && opts.headers && opts.headers["X-T3CM-CSRF"] }});
    if (mode === "ok") return reply(200, syncedRes);
    if (mode === "partial") return reply(200, partialRes);
    return reply(502, {{ error: "gh down" }});
  }}
  return new Promise(() => {{}});
}};
emitState(base);
await tick();
selectTab("graph");
const before = {{
  btnHidden: q("#graph-sync").hidden,
  btnText: q("#graph-sync").textContent,
  resHidden: q("#graph-sync-res").hidden,
  edges: document.querySelectorAll(".ge").map((p) => [p.getAttribute("class"), p.getAttribute("data-source")]),
}};
// Open inspector on #2 so we verify drawer drops the "body" badge after sync.
btn("acme/widgets#2").dispatch("click");
const drawerBefore = deps();

q("#graph-sync").dispatch("click");
await tick();
const syncCall = syncReqs[0];
const afterOk = {{
  method: syncCall && syncCall.method,
  csrf: syncCall && syncCall.csrf,
  btnHidden: q("#graph-sync").hidden,
  resHidden: q("#graph-sync-res").hidden,
  resOk: q("#graph-sync-res").classList.contains("ok"),
  resMsg: q("#graph-sync-res .gsync-msg") ? q("#graph-sync-res .gsync-msg").textContent : null,
  edges: document.querySelectorAll(".ge").map((p) => [p.getAttribute("class"), p.getAttribute("data-source")]),
  drawerAfter: deps(),
}};
q("#graph-sync-close").dispatch("click");
const dismissedHidden = q("#graph-sync-res").hidden;

// Reload body-edges view and trigger partial failure response.
paintFlow(view);
mode = "partial";
q("#graph-sync").dispatch("click");
await tick();
const afterPartial = {{
  btnHidden: q("#graph-sync").hidden,
  btnText: q("#graph-sync").textContent,
  resErr: q("#graph-sync-res").classList.contains("err"),
  resMsg: q("#graph-sync-res .gsync-msg") ? q("#graph-sync-res .gsync-msg").textContent : null,
  items: document.querySelectorAll("#graph-sync-res li").map((li) => [
    li.dataset.kind, li.dataset.permission, li.dataset.conflict, li.textContent
  ]),
  edges: document.querySelectorAll(".ge").map((p) => [p.getAttribute("class"), p.getAttribute("data-source")]),
}};

// Trigger HTTP 502 error.
mode = "fail";
q("#graph-sync").dispatch("click");
await tick();
const afterFail = {{
  btnDisabled: q("#graph-sync").getAttribute("aria-disabled"),
  resErr: q("#graph-sync-res").classList.contains("err"),
  resMsg: q("#graph-sync-res .gsync-msg") ? q("#graph-sync-res .gsync-msg").textContent : null,
}};
return {{ before, drawerBefore, afterOk, dismissedHidden, afterPartial, afterFail }};
"##
        ),
    );

    assert_eq!(r["before"]["btnHidden"], false, "{r}");
    assert_eq!(r["before"]["btnText"], "Sync 2 parsed links to GitHub", "{r}");
    assert_eq!(r["before"]["resHidden"], true, "{r}");
    assert!(r["before"]["edges"].as_array().unwrap().iter().all(|e| e[1] == "body"), "edges start as body: {r}");
    assert!(
        r["drawerBefore"].as_array().unwrap().iter().any(|d| d.as_str().unwrap().contains("body")),
        "drawer shows body badge before sync: {r}"
    );

    assert_eq!(r["afterOk"]["method"], "POST", "{r}");
    assert!(r["afterOk"]["csrf"].as_str().is_some_and(|c| !c.is_empty()), "{r}");
    assert_eq!(r["afterOk"]["btnHidden"], true, "button hides once 0 body edges remain: {r}");
    assert_eq!(r["afterOk"]["resHidden"], false, "{r}");
    assert_eq!(r["afterOk"]["resOk"], true, "{r}");
    assert_eq!(r["afterOk"]["resMsg"], "Synced 2 parsed links to GitHub.", "{r}");
    assert!(
        r["afterOk"]["edges"].as_array().unwrap().iter().all(|e| e[1] == "github"),
        "edges refresh to native github: {r}"
    );
    assert!(
        r["afterOk"]["drawerAfter"].as_array().unwrap().iter().all(|d| !d.as_str().unwrap().contains("body")),
        "drawer drops body badge after sync: {r}"
    );
    assert_eq!(r["dismissedHidden"], true, "{r}");

    assert_eq!(r["afterPartial"]["btnHidden"], false, "{r}");
    assert_eq!(
        r["afterPartial"]["btnText"], "Sync 1 parsed link to GitHub",
        "singular label for 1 remaining edge: {r}"
    );
    assert_eq!(r["afterPartial"]["resErr"], true, "{r}");
    assert_eq!(r["afterPartial"]["resMsg"], "Sync finished (synced 1, 2 failed of 3).", "{r}");
    let items = r["afterPartial"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 2, "{items:?}");
    assert_eq!(items[0][0], "parent");
    assert_eq!(items[0][1], "false");
    assert_eq!(items[0][2], "true");
    assert!(items[0][3].as_str().unwrap().contains("parent conflict, existing parent widgets#1"), "{items:?}");
    assert_eq!(items[1][0], "blockedBy");
    assert_eq!(items[1][1], "true");
    assert_eq!(items[1][2], "false");
    assert!(items[1][3].as_str().unwrap().contains("(permission)"), "{items:?}");

    assert_eq!(r["afterFail"]["btnDisabled"], Value::Null, "{r}");
    assert_eq!(r["afterFail"]["resErr"], true, "{r}");
    assert_eq!(r["afterFail"]["resMsg"], "Could not sync parsed links: gh down", "{r}");

    // Stale GET /api/flow started before sync must not overwrite synced edges or bring back the sync button,
    // and a refresh queued during sync must still run after sync and the stale GET settle.
    let post_sync_view = {
        let mut v = synced_res["flow"].clone();
        v["graph"]["nodes"][0]["title"] = json!("Post-sync refreshed title");
        v
    };
    let race = panel(
        &html,
        &st,
        &before_view,
        &format!(
            r##"{GRAPH}
const syncedRes = {synced_res};
const postSyncView = {post_sync_view};
let resolveStaleGet = null;
let resolvePost = null;
let getCount = 0;
fetchReply = (url) => {{
  if (url === "/api/flow") {{
    getCount += 1;
    if (getCount === 1) return reply(200, view);
    if (getCount === 2) return new Promise((resolve) => {{ resolveStaleGet = () => resolve(reply(200, view)); }});
    return reply(200, postSyncView);
  }}
  if (url === "/api/flow/sync-links") {{
    return new Promise((resolve) => {{ resolvePost = () => resolve(reply(200, syncedRes)); }});
  }}
  return new Promise(() => {{}});
}};
emitState(base);
await tick();
selectTab("graph");
// 1. Start a poll GET /api/flow that stays in flight while sync completes.
loadFlow();
q("#graph-sync").dispatch("click");
await tick();
// 2. Simulate a board state change queuing a refresh during sync.
loadFlow();
// 3. POST resolves before the stale pre-sync GET.
resolvePost();
await tick();
const afterSync = {{
  btnHidden: q("#graph-sync").hidden,
  edges: document.querySelectorAll(".ge").map((p) => p.getAttribute("data-source")),
  getCount,
}};
// 4. Resolve the pre-sync GET with stale body edges: it must be discarded, and the queued refresh must run.
resolveStaleGet();
await tick();
const afterStaleGet = {{
  btnHidden: q("#graph-sync").hidden,
  edges: document.querySelectorAll(".ge").map((p) => p.getAttribute("data-source")),
  getCount,
  nodeTitle: flow.view && flow.view.graph && flow.view.graph.nodes[0] && flow.view.graph.nodes[0].title,
}};
return {{ afterSync, afterStaleGet }};
"##
        ),
    );
    assert_eq!(race["afterSync"]["btnHidden"], true, "{race}");
    assert_eq!(race["afterSync"]["getCount"], 2, "no third GET while stale GET is still busy: {race}");
    assert!(race["afterSync"]["edges"].as_array().unwrap().iter().all(|s| s == "github"), "{race}");
    assert_eq!(race["afterStaleGet"]["btnHidden"], true, "stale poll must not bring back sync button: {race}");
    assert!(
        race["afterStaleGet"]["edges"].as_array().unwrap().iter().all(|s| s == "github"),
        "stale poll must not overwrite synced edges: {race}"
    );
    assert_eq!(race["afterStaleGet"]["getCount"], 3, "queued refresh must run after sync settles: {race}");
    assert_eq!(race["afterStaleGet"]["nodeTitle"], "Post-sync refreshed title", "{race}");
}

fn open_pr(number: u64, base: &str, head: &str, body: &str, closing: &[(&str, u64)], files: &[&str]) -> Value {
    let closing_nodes: Vec<Value> =
        closing.iter().map(|(r, n)| json!({"number": n, "repository": {"nameWithOwner": r}})).collect();
    let file_nodes: Vec<Value> = files.iter().map(|p| json!({"path": p})).collect();
    json!({
        "id": format!("PR_{number}"),
        "number": number,
        "state": "OPEN",
        "isDraft": false,
        "baseRefName": base,
        "headRefName": head,
        "headRefOid": format!("{number:040x}"),
        "isCrossRepository": false,
        "body": body,
        "closingIssuesReferences": {"nodes": closing_nodes},
        "files": {"nodes": file_nodes},
    })
}

fn run_git(dir: &std::path::Path, args: &[&str]) {
    let mut cmd = std::process::Command::new("git");
    cmd.current_dir(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "Test User")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test User")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0");
    for var in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_COMMON_DIR",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_PREFIX",
        "GIT_SUPER_PREFIX",
    ] {
        cmd.env_remove(var);
    }
    let out = cmd.output().expect("run git");
    assert!(out.status.success(), "git {args:?} failed in {}: {}", dir.display(), String::from_utf8_lossy(&out.stderr));
}

#[tokio::test]
async fn pr_stack_from_open_prs_holds_dependent_until_base_merges_and_paginates_open_prs() {
    let h = flowing_with(4, "auto");
    issue(&h, REPO, 10, &[], 1); // Epic parent of #2
    issue(&h, REPO, 1, &[FLOW], 2);
    issue(&h, REPO, 2, &[FLOW], 3);
    issue(&h, REPO, 3, &[FLOW], 4);
    h.github.set_parent(REPO, 2, (REPO, 10));

    // PR #101 closes #1 (targets main).
    h.github.set_pr(REPO, 101, open_pr(101, "main", "feat/1-base", "Closes #1", &[(REPO, 1)], &["src/a.rs"]));
    // PR #102 targets PR #101's head branch; body references sub-issue #2 AND parent epic #10 (epic pruned).
    h.github.set_pr(REPO, 102, open_pr(102, "feat/1-base", "feat/2-stack", "Refs #2\nRefs #10", &[], &["src/b.rs"]));
    // PR #103 targets PR #102's head branch; matched solely via branch `<type>/<n>-...`.
    h.github.set_pr(REPO, 103, open_pr(103, "feat/2-stack", "feat/3-branch-only", "No refs here", &[], &["src/c.rs"]));
    // PR #104 has a bogus branch issue number #999 (does not exist on GitHub -> dropped without phantom node).
    h.github.set_pr(REPO, 104, open_pr(104, "feat/1-base", "feat/999-missing", "", &[], &[]));
    // PR #105 mentions PR number #101 in `Refs #101` (not an issue -> dropped without phantom node).
    h.github.set_pr(REPO, 105, open_pr(105, "feat/1-base", "chore/followup", "Refs #101", &[], &[]));
    // Force pagination of open PRs (`flowOpenPrs`) across 2 PRs per page.
    h.github.set_pr_page(2);

    let f = flow(&h).await;
    assert_eq!(labels(&f["queue"]), ["widgets#1"], "{f}");
    assert_eq!(labels(&f["blocked"]), ["widgets#2", "widgets#3"], "{f}");
    assert_eq!(
        f["blocked"][0]["prStack"],
        json!([{
            "repo": REPO,
            "number": 1,
            "label": "widgets#1",
            "url": "https://github.com/acme/widgets/issues/1",
            "branch": "feat/2-stack",
            "baseBranch": "feat/1-base",
            "pr": 102,
            "basePr": 101
        }]),
        "{f}"
    );
    assert_eq!(labels(&f["blocked"][0]["blockers"]), ["widgets#1"]);
    assert_eq!(
        f["blocked"][1]["prStack"],
        json!([{
            "repo": REPO,
            "number": 2,
            "label": "widgets#2",
            "url": "https://github.com/acme/widgets/issues/2",
            "branch": "feat/3-branch-only",
            "baseBranch": "feat/2-stack",
            "pr": 103,
            "basePr": 102
        }]),
        "{f}"
    );
    assert!(
        h.github.operations().iter().any(|op| op == "flowOpenPrs"),
        "expected flowOpenPrs pagination query: {:?}",
        h.github.operations()
    );

    let g = graph(&h).await;
    let edges = g["graph"]["edges"].as_array().unwrap();
    assert!(
        edges.contains(&json!({
            "from": "acme/widgets#2",
            "to": "acme/widgets#1",
            "kind": "prStack",
            "source": "git",
            "branch": "feat/2-stack",
            "baseBranch": "feat/1-base",
            "pr": 102,
            "basePr": 101
        })),
        "{edges:?}"
    );
    assert!(
        edges.contains(&json!({
            "from": "acme/widgets#3",
            "to": "acme/widgets#2",
            "kind": "prStack",
            "source": "git",
            "branch": "feat/3-branch-only",
            "baseBranch": "feat/2-stack",
            "pr": 103,
            "basePr": 102
        })),
        "{edges:?}"
    );
    let node_ids: Vec<&str> =
        g["graph"]["nodes"].as_array().unwrap().iter().map(|n| n["id"].as_str().unwrap()).collect();
    assert!(
        !node_ids.contains(&"acme/widgets#999") && !node_ids.contains(&"acme/widgets#101"),
        "bogus branch/PR numbers must not produce phantom nodes: {node_ids:?}"
    );
    // Epic #10 must not receive a prStack edge from #102 because #2 is its sub-issue.
    assert!(
        edges.iter().all(|e| !(e["kind"] == "prStack" && e["from"] == "acme/widgets#10")),
        "parent epic must be pruned when sub-issue matches: {edges:?}"
    );

    // Auto-launch picks only #1 while #2 and #3 are held by prStack.
    reconcile(&h).await;
    let launched: Vec<String> = h.fake.launched().iter().map(|l| l["title"].as_str().unwrap().to_string()).collect();
    assert_eq!(launched, ["widgets#1: Issue 1"]);

    // Once PR #101 merges (and #1 closes), #2 unblocks and enters queue while #3 stays stacked on PR #102.
    h.github.update_pr(REPO, 101, json!({"state": "MERGED"}));
    h.github.close_issue(REPO, 1);
    let f2 = flow(&h).await;
    assert_eq!(labels(&f2["queue"]), ["widgets#2"], "{f2}");
    assert_eq!(labels(&f2["blocked"]), ["widgets#3"], "{f2}");

    // Once PR #102 merges too, #3 enters queue.
    h.github.update_pr(REPO, 102, json!({"state": "MERGED"}));
    let f3 = flow(&h).await;
    assert_eq!(labels(&f3["queue"]), ["widgets#2", "widgets#3"], "{f3}");
    assert_eq!(labels(&f3["blocked"]), Vec::<String>::new(), "{f3}");
}

#[tokio::test]
async fn local_worktree_branch_ancestry_and_dirty_edits_infer_pr_stack_and_file_conflict() {
    let td = tempfile::tempdir().unwrap();
    let repo_dir = td.path().join("repo");
    std::fs::create_dir_all(repo_dir.join("src")).unwrap();
    run_git(&repo_dir, &["init", "-b", "main"]);
    std::fs::write(repo_dir.join("src/alpha.rs"), "pub fn a() {}\n").unwrap();
    std::fs::write(repo_dir.join("src/beta.rs"), "pub fn b() {}\n").unwrap();
    std::fs::write(repo_dir.join("src/shared.rs"), "pub fn s() {}\n").unwrap();
    run_git(&repo_dir, &["add", "."]);
    run_git(&repo_dir, &["commit", "-m", "init"]);

    let wt1 = td.path().join("wt-1");
    run_git(&repo_dir, &["worktree", "add", "-b", "feat/1-base", wt1.to_str().unwrap(), "main"]);
    std::fs::write(wt1.join("src/alpha.rs"), "pub fn a() { /* base */ }\n").unwrap();
    run_git(&wt1, &["commit", "-am", "base"]);

    let wt2 = td.path().join("wt-2");
    run_git(&repo_dir, &["worktree", "add", "-b", "feat/2-child", wt2.to_str().unwrap(), "feat/1-base"]);
    std::fs::write(wt2.join("src/beta.rs"), "pub fn b() { /* child */ }\n").unwrap();
    run_git(&wt2, &["commit", "-am", "child"]);

    let wt3 = td.path().join("wt-3");
    run_git(&repo_dir, &["worktree", "add", "-b", "feat/3-grandchild", wt3.to_str().unwrap(), "feat/2-child"]);
    std::fs::write(wt3.join("src/shared.rs"), "pub fn s() { /* grandchild */ }\n").unwrap();
    run_git(&wt3, &["commit", "-am", "grandchild"]);

    // Independent worktree #4 branched from main with an uncommitted edit to src/beta.rs (conflicts with #2).
    let wt4 = td.path().join("wt-4");
    run_git(&repo_dir, &["worktree", "add", "-b", "feat/4-dirty", wt4.to_str().unwrap(), "main"]);
    std::fs::write(wt4.join("src/beta.rs"), "pub fn b() { /* dirty */ }\n").unwrap();

    let cfg = Config::from_value(json!({
        "flow": {"repos": [REPO], "optIn": {"label": FLOW}, "maxTickets": 4, "launch": "ask"}
    }))
    .unwrap();
    let h = H::build(Vec::new(), cfg);
    h.github.add_repo(REPO);
    h.fake.set_projects(vec![project("p-widgets", repo_dir.to_str().unwrap())]);
    h.remotes.set(repo_dir.to_str().unwrap(), "git@github.com:acme/widgets.git");
    for n in 1..=4 {
        issue(&h, REPO, n, &[FLOW], n as u32);
    }

    let g = graph(&h).await;
    assert_eq!(labels(&g["queue"]), ["widgets#1", "widgets#4"], "{g}");
    assert_eq!(labels(&g["blocked"]), ["widgets#2", "widgets#3"], "{g}");

    let edges = g["graph"]["edges"].as_array().unwrap();
    let pr_stacks: Vec<&Value> = edges.iter().filter(|e| e["kind"] == "prStack").collect();
    assert_eq!(
        pr_stacks,
        vec![
            &json!({
                "from": "acme/widgets#2",
                "to": "acme/widgets#1",
                "kind": "prStack",
                "source": "git",
                "branch": "feat/2-child",
                "baseBranch": "feat/1-base"
            }),
            &json!({
                "from": "acme/widgets#3",
                "to": "acme/widgets#2",
                "kind": "prStack",
                "source": "git",
                "branch": "feat/3-grandchild",
                "baseBranch": "feat/2-child"
            }),
        ],
        "3-deep worktree stack must transitively reduce #3 -> #1: {pr_stacks:?}"
    );
    let conflicts: Vec<&Value> = edges.iter().filter(|e| e["kind"] == "fileConflict").collect();
    assert_eq!(
        conflicts,
        vec![&json!({
            "from": "acme/widgets#2",
            "to": "acme/widgets#4",
            "kind": "fileConflict",
            "source": "code",
            "files": ["src/beta.rs"]
        })],
        "stacked worktrees diff against stack base (no false #3/#1 or #3/#2 conflict), dirty #4 conflicts with #2: {conflicts:?}"
    );

    // Once feat/1-base merges into main, #2 is no longer stacked on #1 and enters queue.
    run_git(&repo_dir, &["merge", "--ff-only", "feat/1-base"]);
    let f2 = flow(&h).await;
    assert_eq!(labels(&f2["queue"]), ["widgets#1", "widgets#2", "widgets#4"], "{f2}");
    assert_eq!(labels(&f2["blocked"]), ["widgets#3"], "{f2}");
}

#[tokio::test]
async fn sequential_ticket_part_chains_emit_pr_stack_and_unblock_when_earlier_part_closes() {
    let h = flowing(4);
    issue(&h, REPO, 100, &[], 1); // Epic
    issue(&h, REPO, 101, &[FLOW], 2);
    issue(&h, REPO, 102, &[FLOW], 3);
    issue(&h, REPO, 103, &[FLOW], 4);

    // Ticket thread for part 1/3 is open; worker/pending links hold parts 2/3 and 3/3 under parentIssue #100.
    h.fake.put(t3_codemode::fake::thread("mcp:t101", "running", T0));
    h.wf(
        "mcp:t101",
        "join",
        json!({
            "parent": BOARD,
            "label": "widgets#101",
            "role": "ticket",
            "issue": "acme/widgets#101",
            "parentIssue": "acme/widgets#100",
            "part": "1/3"
        }),
    )
    .await;
    let params = h.app.wf.config().link_params("ticket", "worker").unwrap();
    for (num, part) in [(102u64, "2/3"), (103u64, "3/3")] {
        h.app
            .store
            .pending_put(&Pending {
                parent: "mcp:overseer".into(),
                label: format!("widgets#{num}"),
                role: "worker".into(),
                params,
                at: T0.into(),
                reported: false,
                thread: None,
                link: None,
                project: None,
                identity: t3_codemode::store::Identity {
                    issue: Some(format!("acme/widgets#{num}")),
                    parent_issue: Some("acme/widgets#100".into()),
                    part: Some(part.into()),
                },
                brief: None,
            })
            .unwrap();
    }

    let g = graph(&h).await;
    assert_eq!(labels(&g["queue"]), Vec::<String>::new(), "{g}");
    assert_eq!(labels(&g["blocked"]), ["widgets#102", "widgets#103"], "{g}");
    assert_eq!(g["blocked"][0]["prStack"][0]["part"], "1/3 -> 2/3");
    assert_eq!(g["blocked"][1]["prStack"][0]["part"], "2/3 -> 3/3");
    let pr_stacks: Vec<&Value> =
        g["graph"]["edges"].as_array().unwrap().iter().filter(|e| e["kind"] == "prStack").collect();
    assert_eq!(
        pr_stacks,
        vec![
            &json!({
                "from": "acme/widgets#102",
                "to": "acme/widgets#101",
                "kind": "prStack",
                "source": "git",
                "part": "1/3 -> 2/3"
            }),
            &json!({
                "from": "acme/widgets#103",
                "to": "acme/widgets#102",
                "kind": "prStack",
                "source": "git",
                "part": "2/3 -> 3/3"
            }),
        ],
        "{pr_stacks:?}"
    );

    // Closing #101 releases the pure part 1/3 -> 2/3 hold so #102 enters queue while #103 still waits on #102.
    h.github.close_issue(REPO, 101);
    let f2 = flow(&h).await;
    assert_eq!(labels(&f2["queue"]), ["widgets#102"], "{f2}");
    assert_eq!(labels(&f2["blocked"]), ["widgets#103"], "{f2}");
}

#[tokio::test]
async fn file_conflicts_verify_paths_against_repo_tree_and_serialize_when_enabled() {
    let td = tempfile::tempdir().unwrap();
    let repo_dir = td.path().join("widgets");
    std::fs::create_dir_all(repo_dir.join("src")).unwrap();
    run_git(&repo_dir, &["init", "-b", "main"]);
    std::fs::write(repo_dir.join("src/flow.rs"), "// flow\n").unwrap();
    std::fs::write(repo_dir.join("src/board.html"), "<!-- board -->\n").unwrap();
    std::fs::write(repo_dir.join("src/other.rs"), "// other\n").unwrap();
    run_git(&repo_dir, &["add", "."]);
    run_git(&repo_dir, &["commit", "-m", "init"]);

    let setup = |serialize: bool, launch: &str, max: usize| -> H {
        let cfg = Config::from_value(json!({
            "flow": {
                "repos": [REPO, TOOLS],
                "optIn": {"label": FLOW},
                "maxTickets": max,
                "launch": launch,
                "serializeFileConflicts": serialize
            }
        }))
        .unwrap();
        let h = H::build(Vec::new(), cfg);
        h.github.add_repo(REPO);
        h.github.add_repo(TOOLS);
        h.fake.set_projects(vec![project("p-widgets", repo_dir.to_str().unwrap()), project("p-tools", "/src/tools")]);
        h.remotes.set(repo_dir.to_str().unwrap(), "git@github.com:acme/widgets.git");
        h.remotes.set("/src/tools", "git@github.com:acme/tools.git");

        issue(&h, REPO, 1, &[FLOW, "priority: high"], 1);
        issue(&h, REPO, 2, &[FLOW, "priority: medium"], 2);
        issue(&h, REPO, 3, &[FLOW, "priority: low"], 3);
        issue(&h, REPO, 4, &[FLOW, "priority: low"], 4);
        issue(&h, REPO, 5, &[FLOW, "priority: low"], 5);
        issue(&h, TOOLS, 1, &[FLOW, "priority: low"], 6);

        // #1 declares real src/flow.rs + nonexistent src/ghost.rs in Touches.
        h.github.set_body(REPO, 1, "## Flow\n- Touches: `src/flow.rs`, `src/ghost.rs`\n");
        // #2 mentions src/flow.rs and src/ghost.rs in prose + Touches src/board.html.
        h.github.set_body(
            REPO,
            2,
            "Update `src/flow.rs` and `src/ghost.rs`.\n\n## Flow\n- Touches: `src/board.html`\n",
        );
        // #3 has an open PR modifying src/board.html (overlaps with #2's Touches).
        h.github.set_pr(REPO, 201, open_pr(201, "main", "feat/3-ui", "Closes #3", &[(REPO, 3)], &["src/board.html"]));
        // #4 touches independent src/other.rs.
        h.github.set_body(REPO, 4, "## Flow\n- Touches: `src/other.rs`\n");
        // #5 is a sub-issue of #1 touching src/flow.rs (epic/sub-issue pair must NOT emit fileConflict).
        h.github.set_parent(REPO, 5, (REPO, 1));
        h.github.set_body(REPO, 5, "## Flow\n- Touches: `src/flow.rs`\n");
        // Cross-repo tools#1 mentions src/flow.rs (different repo -> must NOT conflict with widgets#1).
        h.github.set_body(TOOLS, 1, "## Flow\n- Touches: `src/flow.rs`\n");
        h
    };

    // 1. Default (serializeFileConflicts: false): edges appear in graph, but queue & auto-launch stay unblocked.
    let h_off = setup(false, "auto", 2);
    let g_off = graph(&h_off).await;
    let conflicts: Vec<&Value> =
        g_off["graph"]["edges"].as_array().unwrap().iter().filter(|e| e["kind"] == "fileConflict").collect();
    assert_eq!(
        conflicts,
        vec![
            &json!({
                "from": "acme/widgets#1",
                "to": "acme/widgets#2",
                "kind": "fileConflict",
                "source": "code",
                "files": ["src/flow.rs"]
            }),
            &json!({
                "from": "acme/widgets#2",
                "to": "acme/widgets#3",
                "kind": "fileConflict",
                "source": "code",
                "files": ["src/board.html"]
            }),
            &json!({
                "from": "acme/widgets#2",
                "to": "acme/widgets#5",
                "kind": "fileConflict",
                "source": "code",
                "files": ["src/flow.rs"]
            }),
        ],
        "ghost.rs dropped, parent-child #1/#5 skipped (while unrelated #2/#5 still conflicts), cross-repo tools#1 skipped: {conflicts:?}"
    );
    assert_eq!(labels(&g_off["queue"]), ["widgets#1", "widgets#2", "widgets#3", "widgets#4", "widgets#5", "tools#1"]);
    reconcile(&h_off).await;
    let launched_off: Vec<String> =
        h_off.fake.launched().iter().map(|l| l["title"].as_str().unwrap().to_string()).collect();
    assert_eq!(launched_off, ["widgets#1: Issue 1", "widgets#2: Issue 2"]);

    // 2. serializeFileConflicts: true in auto mode: batch pick launches #1, skips colliding #2, launches #3 and #4.
    let h_auto = setup(true, "auto", 3);
    reconcile(&h_auto).await;
    let launched_on: Vec<String> =
        h_auto.fake.launched().iter().map(|l| l["title"].as_str().unwrap().to_string()).collect();
    assert_eq!(
        launched_on,
        ["widgets#1: Issue 1", "widgets#3: Issue 3", "widgets#4: Issue 4"],
        "colliding #2 must be held while higher-priority #1 launches, non-conflicting #3 & #4 fill slots"
    );

    // 3. With #1 actively running, wf.flow() moves #2 into blocked with fileConflicts details until #1 hands off.
    let h_run = setup(true, "ask", 3);
    ticket(&h_run, "mcp:t1", "widgets#1").await;
    let f_run = flow(&h_run).await;
    assert_eq!(labels(&f_run["blocked"]), ["widgets#2"], "{f_run}");
    assert_eq!(
        f_run["blocked"][0]["fileConflicts"],
        json!([{
            "repo": REPO,
            "number": 1,
            "label": "widgets#1",
            "url": "https://github.com/acme/widgets/issues/1",
            "files": ["src/flow.rs"]
        }]),
        "{f_run}"
    );
    assert_eq!(labels(&f_run["queue"]), ["widgets#3", "widgets#4", "tools#1"], "{f_run}");

    // Hand off #1 -> #2 returns to queue cleanly.
    h_run.wf("mcp:t1", "send", json!({"kind": "handoff", "body": "done"})).await;
    let f_after = flow(&h_run).await;
    assert_eq!(labels(&f_after["blocked"]), Vec::<String>::new(), "{f_after}");
    assert!(labels(&f_after["queue"]).contains(&"widgets#2".to_string()), "{f_after}");

    // 4. serializeFileConflicts: true in ask mode offers #1 and #3 (skipping colliding #2),
    // and retires/refuses a stale lower-priority colliding card when serialization turns on later.
    let h_ask = setup(true, "ask", 2);
    reconcile(&h_ask).await;
    let offered: Vec<String> = flow_cards(&h_ask).iter().map(|c| c["label"].as_str().unwrap().to_string()).collect();
    assert_eq!(offered, ["widgets#1", "widgets#3"], "ask mode must not offer colliding #2 alongside #1");

    let mut h_retire = setup(false, "ask", 2);
    reconcile(&h_retire).await;
    let pre_cards = flow_cards(&h_retire);
    assert_eq!(pre_cards.iter().map(|c| c["label"].as_str().unwrap()).collect::<Vec<_>>(), ["widgets#1", "widgets#2"]);
    let card2_id = pre_cards[1]["id"].as_str().unwrap().to_string();
    h_retire.config = Config::from_value(json!({
        "flow": {
            "repos": [REPO, TOOLS],
            "optIn": {"label": FLOW},
            "maxTickets": 2,
            "launch": "ask",
            "serializeFileConflicts": true
        }
    }))
    .unwrap();
    h_retire.reopen();
    let refused = h_retire.app.wf.answer(&card2_id, "launch").await.unwrap_err().to_string();
    assert!(refused.contains("conflicts with a higher-priority offered launch"), "{refused}");
    assert!(h_retire.fake.launched().is_empty());
}

#[tokio::test]
async fn graph_tab_renders_pr_stack_and_file_conflict_styles_legend_tooltips_and_inspector() {
    let cfg = Config::from_value(json!({
        "flow": {
            "repos": [REPO],
            "optIn": {"label": FLOW},
            "maxTickets": 4,
            "launch": "ask",
            "serializeFileConflicts": true
        }
    }))
    .unwrap();
    let h = H::build(Vec::new(), cfg);
    h.github.add_repo(REPO);
    h.fake.set_projects(vec![project("p-widgets", "/src/widgets")]);
    h.remotes.set("/src/widgets", "git@github.com:acme/widgets.git");

    issue(&h, REPO, 1, &[FLOW, "priority: high"], 1);
    issue(&h, REPO, 2, &[FLOW], 2);
    issue(&h, REPO, 3, &[FLOW], 3);
    issue(&h, REPO, 4, &[FLOW], 4);
    h.github.set_pr(REPO, 101, open_pr(101, "main", "feat/1-base", "Closes #1", &[(REPO, 1)], &["src/flow.rs"]));
    h.github.set_pr(
        REPO,
        102,
        open_pr(102, "feat/1-base", "feat/2-stack", "Closes #2", &[(REPO, 2)], &["src/board.html"]),
    );
    h.github.set_body(REPO, 3, "## Flow\n- Touches: `src/board.html`\n");
    h.github.set_body(REPO, 4, "## Flow\n- Touches: `src/flow.rs`\n");
    ticket(&h, "mcp:t1", "widgets#1").await;

    let (html, st, view) = panel_page(h).await;
    let r = panel(
        &html,
        &st,
        &view,
        &format!(
            r##"{GRAPH}
fetchReply = (url) => url === "/api/flow" ? reply(200, view) : new Promise(() => {{}});
emitState(base);
await tick();
const blockedRows = Object.fromEntries(rows("flow-b").map((row) => [row.label, row.why]));
selectTab("graph");
const legend = document.querySelectorAll("#graph-key span").map((s) => s.textContent.trim());
const edgeInfo = document.querySelectorAll(".ge").map((p) => ({{
  cls: p.getAttribute("class"),
  kind: p.getAttribute("data-kind"),
  source: p.getAttribute("data-source"),
  marker: p.getAttribute("marker-end"),
  title: p.querySelector("title") ? p.querySelector("title").textContent : null,
}}));
btn("acme/widgets#2").dispatch("pointerover");
const hover2Edges = document.querySelectorAll(".ge").map((p) => [p.getAttribute("data-kind"), p.getAttribute("class")]);
const hover2Marks = marks();
btn("acme/widgets#2").dispatch("click");
const drawer2 = deps();
const note2 = dp("note-m").textContent;
btn("acme/widgets#1").dispatch("click");
const drawer1 = deps();
return {{ blockedRows, legend, edgeInfo, hover2Edges, hover2Marks, drawer2, note2, drawer1 }};
"##
        ),
    );

    assert!(
        html.contains(r#"class="k-stack""#)
            && html.contains("PR stack")
            && html.contains(r#"class="k-conflict""#)
            && html.contains("file conflict"),
        "Graph legend HTML must include PR stack and file conflict entries"
    );

    assert!(
        r["blockedRows"]["widgets#2"].as_str().unwrap().contains("stacked on #1"),
        "Up next blocked row must name PR stack hold: {r}"
    );
    assert!(
        r["blockedRows"]["widgets#4"].as_str().unwrap().contains("file conflict with #1 (src/flow.rs)"),
        "Up next blocked row must name file conflict hold: {r}"
    );

    let edges = r["edgeInfo"].as_array().unwrap();
    let stack_e = edges.iter().find(|e| e["kind"] == "prStack").expect("prStack edge");
    assert_eq!(stack_e["cls"], "ge stack src-git");
    assert_eq!(stack_e["source"], "git");
    assert_eq!(stack_e["marker"], "url(#gm-git)");
    let stack_title = stack_e["title"].as_str().unwrap();
    assert!(
        stack_title.contains("feat/2-stack")
            && stack_title.contains("feat/1-base")
            && stack_title.contains("#102")
            && stack_title.contains("#101"),
        "prStack tooltip must name branches and PRs: {stack_title}"
    );

    let conflict_e = edges
        .iter()
        .find(|e| e["kind"] == "fileConflict" && e["title"].as_str().unwrap().contains("src/board.html"))
        .expect("fileConflict edge");
    assert_eq!(conflict_e["cls"], "ge conflict src-code");
    assert_eq!(conflict_e["source"], "code");
    assert_eq!(conflict_e["marker"], Value::Null);

    // Hovering #2 traces upstream prStack (#1 -> up), keeps incident fileConflict peer #3 undimmed,
    // highlights incident conflict edge (#2/#3 -> hot), and dims unrelated #4 and its conflict edge.
    assert_eq!(r["hover2Marks"]["acme/widgets#2"], "hot", "{r}");
    assert_eq!(r["hover2Marks"]["acme/widgets#1"], "up", "{r}");
    assert_eq!(r["hover2Marks"]["acme/widgets#3"], "", "{r}");
    assert_eq!(r["hover2Marks"]["acme/widgets#4"], "dim", "{r}");
    let h2 = r["hover2Edges"].as_array().unwrap();
    assert!(h2.contains(&json!(["prStack", "ge stack src-git up"])), "{h2:?}");
    assert!(h2.contains(&json!(["fileConflict", "ge conflict src-code hot"])), "{h2:?}");
    assert!(h2.contains(&json!(["fileConflict", "ge conflict src-code dim"])), "{h2:?}");

    // Inspector for #2 lists PR stack + branch/PR details and File conflicts + overlapping path.
    let d2: Vec<&str> = r["drawer2"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
    assert!(
        d2.iter().any(|s| s.contains("PR stack") && s.contains("widgets#1") && s.contains("feat/2-stack")),
        "drawer #2 PR stack detail missing: {d2:?}"
    );
    assert!(
        d2.iter().any(|s| s.contains("File conflicts") && s.contains("widgets#3") && s.contains("src/board.html")),
        "drawer #2 File conflicts detail missing: {d2:?}"
    );
    assert!(r["note2"].as_str().unwrap().contains("stacked PR"), "{r}");

    // Inspector for #1 lists Stacked (incoming prStack from #2) and File conflicts with #4.
    let d1: Vec<&str> = r["drawer1"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
    assert!(
        d1.iter().any(|s| s.contains("Stacked") && s.contains("widgets#2")),
        "drawer #1 Stacked section missing: {d1:?}"
    );
    assert!(
        d1.iter().any(|s| s.contains("File conflicts") && s.contains("widgets#4") && s.contains("src/flow.rs")),
        "drawer #1 File conflicts section missing: {d1:?}"
    );
}
