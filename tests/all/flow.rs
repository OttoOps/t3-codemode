use crate::common::*;
use serde_json::{Value, json};
use t3_codemode::board::BOARD;
use t3_codemode::config::Config;
use t3_codemode::fake::github_issue;
use t3_codemode::field;
use t3_codemode::store::Pending;

const REPO: &str = "acme/widgets";
const TOOLS: &str = "acme/tools";
const FLOW: &str = "flow";

fn flowing(max: usize) -> H {
    let cfg =
        Config::from_value(json!({"flow": {"repos": [REPO, TOOLS], "optIn": {"label": FLOW}, "maxTickets": max}}));
    let h = H::build(Vec::new(), cfg.unwrap());
    h.github.add_repo(REPO);
    h.github.add_repo(TOOLS);
    h
}

fn issue(h: &H, repo: &str, number: u64, labels: &[&str], day: u32) {
    let created = format!("2026-10-{day:02}T09:00:00Z");
    h.github.set_issue(repo, github_issue(number, &format!("Issue {number}"), labels, &created));
}

async fn flow(h: &H) -> Value {
    result(&h.exec("async () => wf.flow()").await)
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

#[tokio::test]
async fn with_no_flow_block_the_flow_is_off_and_reads_nothing() {
    let h = H::new();

    let f = flow(&h).await;

    assert_eq!(
        f,
        json!({"enabled": false, "launch": null, "maxTickets": 0, "free": 0, "running": [], "queue": [], "blocked": []})
    );
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
    assert!(sig.contains("wf.flow(): Promise<{ enabled: boolean;"), "{sig}");
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
