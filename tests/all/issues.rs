use crate::common::*;
use serde_json::{Value, json};
use t3_codemode::board::BOARD;
use t3_codemode::config::{Config, FlowConfig, OptIn};
use t3_codemode::fake::{Failure, FakeGitHub, Nth, Rule, github_issue, project};
use t3_codemode::github::{self, EnsureLabelOutcome, LabelSpec, LinkOutcome, UnlinkOutcome};
use t3_codemode::server;

const REPORT: &str = "async () => wf.issue({ url: 'https://github.com/Acme/widgets/issues/41', title: 'Fix the grader', note: 'found while testing' })";

fn snapshot(h: &H) -> Value {
    h.app.wf.snapshot().unwrap()
}

fn new_ids(v: &Value) -> Vec<Value> {
    v.as_array().unwrap().iter().map(|i| i["id"].clone()).collect()
}

fn map_widgets(h: &H) {
    h.fake.set_projects(vec![project("p-widgets", "/src/widgets")]);
    h.remotes.set("/src/widgets", "git@github.com:Acme/widgets.git");
}

#[tokio::test]
async fn wf_issue_records_the_issue_with_its_filing_thread_and_time() {
    let h = H::new();
    let r = result(&h.exec_as(WORKER, REPORT).await);
    assert_eq!(r["added"], true);
    assert_eq!(r["message"], "https://github.com/Acme/widgets/issues/41 is on the board's new issues as n1");
    let i = &r["issue"];
    assert_eq!(i["id"], "n1");
    assert_eq!(i["url"], "https://github.com/Acme/widgets/issues/41");
    assert_eq!(i["repo"], "Acme/widgets");
    assert_eq!(i["number"], 41);
    assert_eq!(i["label"], "widgets#41");
    assert_eq!(i["title"], "Fix the grader");
    assert_eq!(i["note"], "found while testing");
    assert_eq!(i["thread"], WORKER);
    assert_eq!(i["threadLink"], format!("https://t3/{WORKER}"));
    assert_eq!(i["threadTitle"], WORKER);
    assert_eq!(i["at"], T0);
    assert_eq!(i["dismissed"], false);
    let s = snapshot(&h);
    assert_eq!(new_ids(&s["newIssues"]), [json!("n1")]);
    assert_eq!(s["counts"]["newIssues"], 1);
    assert_eq!(s["dismissedIssues"], json!([]));
    assert!(h.app.store.link_get(WORKER).unwrap().is_none(), "wf.issue must not need or make a link");
}

#[tokio::test]
async fn wf_issue_needs_a_caller_and_a_real_issue() {
    let h = H::new();
    let out = h.exec("async () => wf.issue({ url: 'Acme/widgets#41' })").await;
    assert!(err_text(&out).contains("caller's T3 thread id"), "{}", out.text);
    let out = h.exec_as(WORKER, "async () => wf.issue({ url: 'not an issue' })").await;
    assert!(err_text(&out).contains("is not a GitHub issue"), "{}", out.text);
    let out = h.exec_as(WORKER, "async () => wf.issue({ url: 'https://github.com/Acme/widgets/pull/42' })").await;
    assert!(err_text(&out).contains("is not a GitHub issue"), "a pull request is not an issue: {}", out.text);
    let long = "x".repeat(301);
    let out = h.exec_as(WORKER, &format!("async () => wf.issue({{ url: 'Acme/widgets#41', title: '{long}' }})")).await;
    assert!(err_text(&out).contains("title is over 300 chars"), "{}", out.text);
    assert!(h.app.store.new_issues().unwrap().is_empty());
}

#[tokio::test]
async fn reporting_the_same_issue_twice_adds_nothing() {
    let h = H::new();
    result(&h.exec_as(WORKER, REPORT).await);
    let again = result(&h.exec_as(TICKET, "async () => wf.issue({ url: 'acme/WIDGETS#41', title: 'Other' })").await);
    assert_eq!(again["added"], false);
    assert_eq!(again["reason"], "reported");
    assert_eq!(again["issue"]["id"], "n1");
    assert_eq!(again["issue"]["thread"], WORKER);
    assert_eq!(again["issue"]["title"], "Fix the grader");
    assert!(again["message"].as_str().unwrap().contains("already on the board's new issues as n1"), "{again}");
    assert_eq!(h.app.store.new_issues().unwrap().len(), 1);
}

#[tokio::test]
async fn reporting_again_fills_in_a_thread_link_that_t3_could_not_give() {
    let h = H::new();
    h.fake.fail_tool("t3_thread_read");
    let r = result(&h.exec_as(WORKER, REPORT).await);
    assert_eq!(r["added"], true);
    assert_eq!(r["issue"]["thread"], WORKER);
    assert_eq!(r["issue"]["threadLink"], Value::Null);
    h.fake.clear_faults_on("t3_thread_read");
    let again = result(&h.exec_as(TICKET, REPORT).await);
    assert_eq!(again["reason"], "reported");
    assert_eq!(again["issue"]["thread"], WORKER);
    assert_eq!(again["issue"]["threadLink"], format!("https://t3/{WORKER}"));
    assert_eq!(again["issue"]["threadTitle"], WORKER);
}

#[tokio::test]
async fn reporting_an_issue_that_has_a_ticket_adds_nothing() {
    let h = H::new();
    h.wf(TICKET, "join", json!({"parent": BOARD, "label": "widgets#41", "role": "ticket"})).await;
    let r = result(&h.exec_as(WORKER, REPORT).await);
    assert_eq!(r["added"], false);
    assert_eq!(r["reason"], "ticket");
    assert_eq!(r["ticket"]["label"], "widgets#41");
    assert_eq!(r["ticket"]["thread"], TICKET);
    assert!(r["message"].as_str().unwrap().contains("already has a ticket"), "{r}");
    let r = result(&h.exec_as(WORKER, "async () => wf.issue({ url: 'acme/WIDGETS#41' })").await);
    assert_eq!(r["reason"], "ticket", "labels match ignoring case: {r}");
    assert!(h.app.store.new_issues().unwrap().is_empty());

    map_widgets(&h);
    result(&h.exec("async () => wf.launchTicket({ issue: 'Acme/widgets#42' })").await);
    let r = result(&h.exec_as(WORKER, "async () => wf.issue({ url: 'Acme/widgets#42' })").await);
    assert_eq!(r["reason"], "ticket", "a launched ticket that has not joined yet still counts: {r}");
    assert_eq!(r["ticket"]["thread"], "launch:1");
    assert!(h.app.store.new_issues().unwrap().is_empty());
}

#[tokio::test]
async fn new_issues_survive_a_restart() {
    let mut h = H::new();
    result(&h.exec_as(WORKER, REPORT).await);
    h.reopen();
    assert_eq!(new_ids(&snapshot(&h)["newIssues"]), [json!("n1")]);
    let again = result(&h.exec_as(WORKER, REPORT).await);
    assert_eq!(again["reason"], "reported");
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
    async fn post(&self, path: &str, csrf: bool) -> (u16, Value) {
        let mut req = self.http.post(format!("{}{path}", self.base)).header("Origin", &self.base);
        if csrf {
            req = req.header("X-T3CM-CSRF", self.h.app.csrf.clone());
        }
        let r = req.send().await.unwrap();
        (r.status().as_u16(), r.json().await.unwrap_or(Value::Null))
    }

    async fn list(&self) -> Value {
        self.http.get(format!("{}/api/new-issues", self.base)).send().await.unwrap().json().await.unwrap()
    }
}

#[tokio::test]
async fn launch_from_the_list_starts_a_ticket_and_moves_the_entry_to_tickets() {
    let s = start(H::new()).await;
    map_widgets(&s.h);
    result(&s.h.exec_as(WORKER, REPORT).await);
    assert_eq!(new_ids(&s.list().await["issues"]), [json!("n1")]);
    let (status, r) = s.post("/api/new-issues/n1/launch", true).await;
    assert_eq!(status, 200, "{r}");
    assert_eq!(r["ok"], true);
    assert_eq!(r["id"], "n1");
    assert_eq!(r["label"], "widgets#41");
    let l = &s.h.fake.launched()[0];
    assert_eq!(l["projectId"], "p-widgets");
    assert_eq!(l["title"], "widgets#41: Fix the grader");
    assert!(
        l["message"]
            .as_str()
            .unwrap()
            .starts_with("Work https://github.com/Acme/widgets/issues/41 with the t3-ticket skill.")
    );
    let st = snapshot(&s.h);
    assert_eq!(st["newIssues"], json!([]));
    assert_eq!(st["counts"]["newIssues"], 0);
    let labels: Vec<Value> = st["tree"].as_array().unwrap().iter().map(|t| t["label"].clone()).collect();
    assert_eq!(labels, [json!("widgets#41")]);
    assert!(s.h.app.store.new_issues().unwrap().is_empty());
    let again = result(&s.h.exec_as(WORKER, REPORT).await);
    assert_eq!(again["reason"], "ticket");
    let (status, _) = s.post("/api/new-issues/n1/launch", true).await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn launching_from_the_launch_box_also_takes_the_entry_off_the_list() {
    let s = start(H::new()).await;
    map_widgets(&s.h);
    result(&s.h.exec_as(WORKER, REPORT).await);
    result(&s.h.exec("async () => wf.launchTicket({ issue: 'acme/widgets#41' })").await);
    assert_eq!(s.list().await["issues"], json!([]));
    assert!(s.h.app.store.new_issues().unwrap().is_empty());
}

#[tokio::test]
async fn a_failed_launch_keeps_the_entry_and_leaves_an_error_card() {
    let s = start(H::new()).await;
    result(&s.h.exec_as(WORKER, REPORT).await);
    let (status, r) = s.post("/api/new-issues/n1/launch", true).await;
    assert_eq!(status, 200, "{r}");
    assert_eq!(r["ok"], false);
    assert!(r["error"].as_str().unwrap().contains("no T3 project has github.com/Acme/widgets"), "{r}");
    assert_eq!(s.h.app.wf.cards().unwrap()[0].kind, "error");
    assert_eq!(new_ids(&s.list().await["issues"]), [json!("n1")]);
}

#[tokio::test]
async fn dismiss_takes_an_issue_off_the_list_and_restore_brings_it_back() {
    let s = start(H::new()).await;
    result(&s.h.exec_as(WORKER, REPORT).await);
    result(&s.h.exec_as(WORKER, "async () => wf.issue({ url: 'Acme/widgets#42' })").await);
    assert_eq!(new_ids(&s.list().await["issues"]), [json!("n2"), json!("n1")]);

    let (status, _) = s.post("/api/new-issues/n1/dismiss", false).await;
    assert_eq!(status, 403);
    let (status, r) = s.post("/api/new-issues/n1/dismiss", true).await;
    assert_eq!((status, r), (200, json!({"id": "n1", "dismissed": true})));
    let l = s.list().await;
    assert_eq!(new_ids(&l["issues"]), [json!("n2")]);
    assert_eq!(new_ids(&l["dismissed"]), [json!("n1")]);
    assert_eq!(l["dismissed"][0]["dismissedAt"], T0);
    assert_eq!(snapshot(&s.h)["counts"]["newIssues"], 1);
    let (status, r) = s.post("/api/new-issues/n1/dismiss", true).await;
    assert_eq!(status, 400, "{r}");

    let again = result(&s.h.exec_as(WORKER, REPORT).await);
    assert_eq!(again["reason"], "reported");
    assert!(again["message"].as_str().unwrap().contains("is dismissed"), "{again}");
    assert_eq!(new_ids(&s.list().await["dismissed"]), [json!("n1")]);

    let (status, r) = s.post("/api/new-issues/n1/restore", true).await;
    assert_eq!((status, r), (200, json!({"id": "n1", "dismissed": false})));
    let l = s.list().await;
    assert_eq!(new_ids(&l["issues"]), [json!("n2"), json!("n1")]);
    assert_eq!(l["dismissed"], json!([]));
    let (status, _) = s.post("/api/new-issues/n1/restore", true).await;
    assert_eq!(status, 400);
    let (status, _) = s.post("/api/new-issues/n9/dismiss", true).await;
    assert_eq!(status, 404);
    let (status, _) = s.post("/api/new-issues/bogus/restore", true).await;
    assert_eq!(status, 404);
}

fn issue_label_names(gh: &FakeGitHub, repo: &str, number: u64) -> Vec<String> {
    gh.issue(repo, number).unwrap()["labels"]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["name"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn wf_issue_wires_blockers_parent_and_labels_idempotently() {
    let h = H::new();
    h.github.set_issue("Acme/widgets", github_issue(10, "Epic", &[], T0));
    h.github.set_issue("Acme/widgets", github_issue(40, "Blocker 1", &[], T0));
    h.github.set_issue("Acme/widgets", github_issue(41, "Target issue", &[], T0));
    h.github.set_issue("Other/core", github_issue(7, "Cross-repo blocker", &[], T0));
    h.github.add_label("Acme/widgets", "priority: high", "b60205");

    let first = result(
        &h.exec_as(
            WORKER,
            r##"async () => wf.issue({
                url: "https://github.com/Acme/widgets/issues/41",
                title: "Target issue",
                blockedBy: ["#40", "Other/core#7", "https://github.com/Acme/widgets/issues/40"],
                parent: "#10",
                track: "flow-dag",
                priority: "high",
            })"##,
        )
        .await,
    );
    assert_eq!(first["added"], true, "{first}");
    let w = &first["wiring"];
    assert_eq!(w["ok"], true, "{first}");
    assert_eq!(w["errors"], json!([]));
    assert_eq!(w["parent"], json!({"ref": "Acme/widgets#10", "status": "linked"}));
    assert_eq!(
        w["blockedBy"],
        json!([
            {"ref": "Acme/widgets#40", "status": "linked"},
            {"ref": "Other/core#7", "status": "linked"},
        ])
    );
    assert_eq!(
        w["labels"],
        json!([
            {"name": "track: flow-dag", "status": "createdAndAdded"},
            {"name": "priority: high", "status": "added"},
        ])
    );

    // Sub-issue mutation sends GraphQL-Features: sub_issues header.
    let sub_call = h.github.calls().into_iter().find(|c| c["operation"] == "addSubIssue").unwrap();
    assert_eq!(sub_call["headers"]["GraphQL-Features"], "sub_issues");

    // Verify mutated fake GitHub state.
    let stored = h.github.issue("Acme/widgets", 41).unwrap();
    assert_eq!(stored["parentRef"], json!(["Acme/widgets", 10]));
    assert_eq!(stored["blockers"], json!([["Acme/widgets", 40], ["Other/core", 7]]));
    assert_eq!(issue_label_names(&h.github, "Acme/widgets", 41), vec!["track: flow-dag", "priority: high"]);

    // Second call is completely idempotent: no mutation calls needed and wiring.ok stays true.
    let calls_before = h.github.calls().len();
    let second = result(
        &h.exec_as(
            WORKER,
            r##"async () => wf.issue({
                url: "Acme/widgets#41",
                blockedBy: ["#40", "Other/core#7"],
                parent: "#10",
                track: "flow-dag",
                priority: "high",
            })"##,
        )
        .await,
    );
    assert_eq!(second["added"], false);
    assert_eq!(second["reason"], "reported");
    let w2 = &second["wiring"];
    assert_eq!(w2["ok"], true, "{second}");
    assert_eq!(w2["errors"], json!([]));
    assert_eq!(w2["parent"]["status"], "alreadyLinked");
    assert_eq!(w2["blockedBy"][0]["status"], "alreadyLinked");
    assert_eq!(w2["blockedBy"][1]["status"], "alreadyLinked");
    assert_eq!(w2["labels"][0]["status"], "alreadyApplied");
    assert_eq!(w2["labels"][1]["status"], "alreadyApplied");
    let new_ops: Vec<String> =
        h.github.calls().into_iter().skip(calls_before).map(|c| c["operation"].as_str().unwrap().to_string()).collect();
    assert_eq!(new_ops, vec!["issueNode"], "idempotent repeat should not resend mutations: {new_ops:?}");
}

#[tokio::test]
async fn wf_issue_reuses_compact_track_and_priority_labels_and_respects_sync_false() {
    let h = H::new();
    h.github.set_issue("Acme/widgets", github_issue(1, "B", &[], T0));
    h.github.set_issue("Acme/widgets", github_issue(2, "T", &[], T0));
    h.github.add_label("Acme/widgets", "track:auth", "1d76db");
    h.github.add_label("Acme/widgets", "priority:low", "0e8a16");

    // sync: false skips every GitHub call.
    let no_sync = result(
        &h.exec_as(
            WORKER,
            r##"async () => wf.issue({ url: "Acme/widgets#2", blockedBy: ["#1"], track: "auth", priority: "low", sync: false })"##,
        )
        .await,
    );
    assert_eq!(no_sync["added"], true);
    assert!(no_sync.get("wiring").is_none());
    assert!(h.github.calls().is_empty());

    // Calling again with sync default (true) wires links and reuses existing compact labels.
    let synced = result(
        &h.exec_as(
            WORKER,
            r##"async () => wf.issue({ url: "Acme/widgets#2", blockedBy: ["#1"], track: "auth", priority: "low" })"##,
        )
        .await,
    );
    assert_eq!(synced["wiring"]["ok"], true, "{synced}");
    assert_eq!(
        synced["wiring"]["labels"],
        json!([
            {"name": "track:auth", "status": "added"},
            {"name": "priority:low", "status": "added"},
        ])
    );
    assert_eq!(issue_label_names(&h.github, "Acme/widgets", 2), vec!["track:auth", "priority:low"]);
    assert!(!h.github.operations().contains(&"createLabel".to_string()));
}

#[tokio::test]
async fn wf_issue_reports_permission_and_parent_conflict_errors_without_dropping_them() {
    let h = H::new();
    h.github.set_issue("Acme/widgets", github_issue(1, "Old Epic", &[], T0));
    h.github.set_issue("Acme/widgets", github_issue(2, "New Epic", &[], T0));
    h.github.set_issue("Acme/widgets", github_issue(3, "Blocker", &[], T0));
    h.github.set_issue("Acme/widgets", github_issue(4, "Target", &[], T0));
    h.github.set_parent("Acme/widgets", 4, ("Acme/widgets", 1));
    h.github
        .fault(Rule::on("addBlockedBy").fail(Failure::typed("FORBIDDEN", "Resource not accessible by integration")));
    h.github.fault(
        Rule::on("createLabel").fail(Failure::typed("FORBIDDEN", "Viewer does not have permission to create labels")),
    );

    let r = result(
        &h.exec_as(
            WORKER,
            r##"async () => wf.issue({
                url: "Acme/widgets#4",
                parent: "#2",
                blockedBy: ["#3"],
                track: "new-track",
            })"##,
        )
        .await,
    );
    assert_eq!(r["added"], true, "issue must still be recorded on board: {r}");
    let w = &r["wiring"];
    assert_eq!(w["ok"], false, "{r}");
    assert_eq!(w["parent"]["status"], "error");
    assert!(w["parent"]["error"].as_str().unwrap().contains("already has a parent"), "{w}");
    assert_eq!(w["blockedBy"][0]["status"], "error");
    assert_eq!(w["labels"][0]["status"], "error");
    let errs = w["errors"].as_array().unwrap();
    assert_eq!(errs.len(), 3, "{errs:?}");
    let blocked_err = errs.iter().find(|e| e["op"] == "addBlockedBy").unwrap();
    assert_eq!(blocked_err["permission"], true);
    let label_err = errs.iter().find(|e| e["op"] == "ensureLabel").unwrap();
    assert_eq!(label_err["permission"], true);
    let parent_err = errs.iter().find(|e| e["op"] == "addSubIssue").unwrap();
    assert_eq!(parent_err["permission"], false);
    assert!(r["message"].as_str().unwrap().contains("GitHub wiring failed"), "{r}");
    assert!(r["message"].as_str().unwrap().contains("[permission]"), "{r}");
}

#[tokio::test]
async fn wf_issue_validates_new_fields_before_touching_store_or_github() {
    let h = H::new();
    let bad_cases = [
        ("async () => wf.issue({ url: 'Acme/widgets#41', blockedBy: ['#41'] })", "blocked by itself"),
        ("async () => wf.issue({ url: 'Acme/widgets#41', parent: 'acme/widgets#41' })", "its own parent"),
        ("async () => wf.issue({ url: 'Acme/widgets#41', priority: 'urgent' })", "priority must be"),
        ("async () => wf.issue({ url: 'Acme/widgets#41', blockedBy: ['bogus'] })", "not a GitHub issue"),
    ];
    for (code, want) in bad_cases {
        let out = h.exec_as(WORKER, code).await;
        assert!(err_text(&out).contains(want), "for {code}: {}", out.text);
    }
    assert!(h.app.store.new_issues().unwrap().is_empty());
    assert!(h.github.calls().is_empty());
}

#[tokio::test]
async fn wf_file_issues_sorts_topologically_writes_flow_blocks_wires_links_and_round_trips_into_flow_graph() {
    let cfg = Config {
        flow: Some(FlowConfig {
            repos: Some(vec!["Acme/widgets".into()]),
            opt_in: Some(OptIn { label: Some("flow".into()), milestone: None }),
            max_tickets: Some(2),
            ..Default::default()
        }),
        ..Default::default()
    };
    let h = H::build(Vec::new(), cfg);
    h.github.add_repo("Acme/widgets");
    h.github.set_issue("Acme/widgets", github_issue(10, "Existing prereq", &[], T0));

    let r = result(
        &h.exec_as(
            WORKER,
            r###"async () => wf.fileIssues({
                repo: "Acme/widgets",
                track: "flow-dag",
                priority: "high",
                labels: ["enhancement"],
                issues: [
                    {
                        id: "P2",
                        title: "Phase 2: board sync",
                        body: "## Goal\nSync parsed links.",
                        parent: "E",
                        blockedBy: ["P1"],
                        touches: ["src/web.rs", "src/board.html"],
                        queue: true,
                    },
                    {
                        id: "P1",
                        title: "Phase 1: wire mutations",
                        body: "## Goal\nWire mutations.\n\n## Flow\n- Parent: E\n- Blocked by: P0",
                        parent: "E",
                        blockedBy: ["P0", "#10"],
                        touches: ["src/issues.rs", "src/github.rs"],
                        queue: true,
                    },
                    {
                        id: "P0",
                        title: "Phase 0: foundation",
                        body: "## Goal\nParse bodies.",
                        parent: "E",
                        blockedBy: ["#10"],
                        priority: "medium",
                        touches: ["src/flow_body.rs"],
                        queue: true,
                    },
                    {
                        id: "E",
                        title: "Epic: dependency DAG",
                        body: "## Goal\nUmbrella epic.",
                        queue: false,
                    },
                ],
            })"###,
        )
        .await,
    );

    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(r["errors"], json!([]));
    assert_eq!(r["order"], json!(["E", "P0", "P1", "P2"]));
    assert_eq!(r["byId"]["E"]["number"], 11);
    assert_eq!(r["byId"]["P0"]["number"], 12);
    assert_eq!(r["byId"]["P1"]["number"], 13);
    assert_eq!(r["byId"]["P2"]["number"], 14);

    // Check generated ## Flow blocks carry substituted real #N references.
    let p1_body = h.github.issue("Acme/widgets", 13).unwrap()["body"].as_str().unwrap().to_string();
    assert!(!p1_body.contains("Parent: E"), "stale symbolic ## Flow must be replaced:\n{p1_body}");
    assert!(
        p1_body.contains("## Flow\n- Parent: #11\n- Blocked by: #12, #10\n- Track: flow-dag\n- Priority: high\n- Touches: `src/issues.rs`, `src/github.rs`"),
        "{p1_body}"
    );
    let p2_body = h.github.issue("Acme/widgets", 14).unwrap()["body"].as_str().unwrap().to_string();
    assert!(
        p2_body.contains("## Flow\n- Parent: #11\n- Blocked by: #13\n- Track: flow-dag\n- Priority: high\n- Touches: `src/web.rs`, `src/board.html`"),
        "{p2_body}"
    );

    // Check labels: epic has NO `flow` queue opt-in label; P0/P1/P2 DO have `flow`.
    let epic_labels = issue_label_names(&h.github, "Acme/widgets", 11);
    assert!(epic_labels.contains(&"track: flow-dag".into()), "{epic_labels:?}");
    assert!(epic_labels.contains(&"priority: high".into()), "{epic_labels:?}");
    assert!(epic_labels.contains(&"enhancement".into()), "{epic_labels:?}");
    assert!(!epic_labels.contains(&"flow".into()), "queue: false epic must not carry flow label: {epic_labels:?}");
    let p0_labels = issue_label_names(&h.github, "Acme/widgets", 12);
    assert!(p0_labels.contains(&"flow".into()), "{p0_labels:?}");
    assert!(p0_labels.contains(&"priority: medium".into()), "{p0_labels:?}");

    // Check board new-issues list recorded all 4 created issues.
    assert_eq!(h.app.store.new_issues().unwrap().len(), 4);

    // End-to-end check with wf.flow({ graph: true }): native GitHub edges, track, priority, no warnings.
    let flow = result(&h.exec("async () => wf.flow({ graph: true })").await);
    assert_eq!(flow["graph"]["warnings"], json!([]), "{flow}");
    let edges = flow["graph"]["edges"].as_array().unwrap();
    assert!(
        edges.contains(
            &json!({"from": "Acme/widgets#13", "to": "Acme/widgets#12", "kind": "blockedBy", "source": "github"})
        ),
        "{edges:?}"
    );
    assert!(
        edges.contains(
            &json!({"from": "Acme/widgets#13", "to": "Acme/widgets#11", "kind": "parent", "source": "github"})
        ),
        "{edges:?}"
    );
    assert!(
        edges.contains(
            &json!({"from": "Acme/widgets#14", "to": "Acme/widgets#13", "kind": "blockedBy", "source": "github"})
        ),
        "{edges:?}"
    );
}

#[tokio::test]
async fn wf_file_issues_with_sync_false_round_trips_body_flow_block_through_flow_graph() {
    let cfg = Config {
        flow: Some(FlowConfig { repos: Some(vec!["Acme/widgets".into(), "Acme/core".into()]), ..Default::default() }),
        ..Default::default()
    };
    let h = H::build(Vec::new(), cfg);
    h.github.add_repo("Acme/widgets");
    h.github.add_repo("Acme/core");

    let r = result(
        &h.exec(
            r#"async () => wf.fileIssues({
                sync: false,
                issues: [
                    { id: "epic", repo: "Acme/core", title: "Cross-repo epic", track: "platform" },
                    { id: "core-1", repo: "Acme/core", title: "Core step", parent: "epic", track: "platform" },
                    { id: "app-1", repo: "Acme/widgets", title: "App step", parent: "epic", blockedBy: ["core-1"], track: "platform", touches: ["src/app.rs"] },
                ],
            })"#,
        )
        .await,
    );
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(r["issues"][2]["wiring"], Value::Null);
    let app_body = r["issues"][2]["body"].as_str().unwrap();
    assert!(app_body.contains("- Parent: Acme/core#1"), "{app_body}");
    assert!(app_body.contains("- Blocked by: Acme/core#2"), "{app_body}");
    assert!(app_body.contains("- Track: platform"), "{app_body}");
    assert!(app_body.contains("- Touches: `src/app.rs`"), "{app_body}");

    // With sync: false, no native links exist; wf.flow({ graph: true }) must recover every edge
    // and track cleanly via src/flow_body.rs with zero warnings.
    let flow = result(&h.exec("async () => wf.flow({ graph: true })").await);
    assert_eq!(flow["graph"]["warnings"], json!([]), "{flow}");
    let edges = flow["graph"]["edges"].as_array().unwrap();
    assert!(
        edges.contains(&json!({"from": "Acme/widgets#1", "to": "Acme/core#1", "kind": "parent", "source": "body"})),
        "{edges:?}"
    );
    assert!(
        edges.contains(&json!({"from": "Acme/widgets#1", "to": "Acme/core#2", "kind": "blockedBy", "source": "body"})),
        "{edges:?}"
    );
    let app_node = flow["graph"]["nodes"].as_array().unwrap().iter().find(|n| n["id"] == "Acme/widgets#1").unwrap();
    assert_eq!(app_node["track"], "platform");
}

#[tokio::test]
async fn wf_file_issues_rejects_cycles_and_bad_plans_with_zero_partial_writes() {
    let h = H::new();
    h.github.add_repo("Acme/widgets");

    let cases = [
        (
            r#"async () => wf.fileIssues({ repo: "Acme/widgets", issues: [
                { id: "A", title: "Self blocker", blockedBy: ["A"] }
            ]})"#,
            "dependency cycle detected",
        ),
        (
            r#"async () => wf.fileIssues({ repo: "Acme/widgets", issues: [
                { id: "A", title: "Self parent", parent: "A" }
            ]})"#,
            "dependency cycle detected",
        ),
        (
            r#"async () => wf.fileIssues({ repo: "Acme/widgets", issues: [
                { id: "D", title: "Independent" },
                { id: "A", title: "A", blockedBy: ["C"] },
                { id: "B", title: "B", blockedBy: ["A"] },
                { id: "C", title: "C", blockedBy: ["B"] }
            ]})"#,
            "dependency cycle detected",
        ),
        (
            r#"async () => wf.fileIssues({ repo: "Acme/widgets", issues: [
                { id: "A", title: "A", parent: "B" },
                { id: "B", title: "B", blockedBy: ["A"] }
            ]})"#,
            "dependency cycle detected",
        ),
        (
            r#"async () => wf.fileIssues({ repo: "Acme/widgets", issues: [
                { id: "A", title: "A", blockedBy: ["MISSING"] }
            ]})"#,
            "unknown blockedBy",
        ),
        (
            r#"async () => wf.fileIssues({ repo: "Acme/widgets", issues: [
                { id: "dup", title: "First" },
                { id: "DUP", title: "Second" }
            ]})"#,
            "duplicate local id",
        ),
        (
            r#"async () => wf.fileIssues({ repo: "Acme/nonexistent", issues: [
                { id: "A", title: "First" }
            ]})"#,
            "cannot read repository",
        ),
        (
            r##"async () => wf.fileIssues({ repo: "Acme/widgets", sync: false, issues: [
                { id: "A", title: "Guessed number", blockedBy: ["#1"] }
            ]})"##,
            "external issue reference Acme/widgets#1 does not exist",
        ),
        (
            r##"async () => wf.fileIssues({ repo: "Acme/widgets", issues: [
                { id: "A", title: "Missing external parent", parent: "#999" }
            ]})"##,
            "external issue reference Acme/widgets#999 does not exist",
        ),
        (
            r###"async () => wf.fileIssues({ repo: "Acme/widgets", issues: [
                { id: "A", title: "Body-only self blocker", body: "## Flow\n- Blocked by: #1" }
            ]})"###,
            "body references nonexistent or unreadable issue Acme/widgets#1",
        ),
        (
            r###"async () => wf.fileIssues({ repo: "Acme/widgets", issues: [
                { id: "A", title: "Body prose blocker", body: "Depends on #1", track: "core" }
            ]})"###,
            "body references nonexistent or unreadable issue Acme/widgets#1",
        ),
        (
            r###"async () => wf.fileIssues({ repo: "Acme/widgets", issues: [
                { id: "A", title: "Body symbolic blocker", body: "## Flow\n- Blocked by: P0" }
            ]})"###,
            "body contains symbolic dependency",
        ),
    ];

    for (code, want) in cases {
        let out = h.exec_as(WORKER, code).await;
        assert!(err_text(&out).contains(want), "for {code}: {}", out.text);
    }

    // Zero issues created on GitHub and zero new-issues stored on board.
    assert!(h.github.issue("Acme/widgets", 1).is_none());
    assert!(!h.github.operations().contains(&"createIssue".to_string()));
    assert!(h.app.store.new_issues().unwrap().is_empty());
}

#[tokio::test]
async fn wf_file_issues_inherits_top_level_parent_without_self_linking_parent_item_and_reports_wiring_errors() {
    let h = H::new();
    h.github.add_repo("Acme/widgets");
    h.github.fault(Rule::on("addBlockedBy").fail(Failure::typed("FORBIDDEN", "No permission to add blocked-by links")));

    let r = result(
        &h.exec_as(
            WORKER,
            r#"async () => wf.fileIssues({
                repo: "Acme/widgets",
                parent: "E",
                report: false,
                issues: [
                    { id: "P1", title: "Step 1", blockedBy: ["P0"] },
                    { id: "P0", title: "Step 0" },
                    { id: "E", title: "Epic" },
                ],
            })"#,
        )
        .await,
    );

    assert_eq!(r["ok"], false, "{r}");
    assert_eq!(r["order"], json!(["E", "P0", "P1"]));
    assert_eq!(r["issues"][0]["parent"], Value::Null, "E must not inherit itself as parent");
    assert_eq!(r["issues"][1]["parent"], "Acme/widgets#1");
    assert_eq!(r["issues"][2]["parent"], "Acme/widgets#1");
    assert_eq!(r["issues"][2]["wiring"]["parent"]["status"], "linked");
    assert_eq!(r["issues"][2]["wiring"]["blockedBy"][0]["status"], "error");
    assert_eq!(r["errors"].as_array().unwrap().len(), 1);
    assert_eq!(r["errors"][0]["op"], "addBlockedBy");
    assert_eq!(r["errors"][0]["permission"], true);
    assert!(h.app.store.new_issues().unwrap().is_empty(), "report: false skips board new-issues");
}

#[tokio::test]
async fn github_dependency_and_label_helpers_are_idempotent_and_support_removal() {
    let gh = FakeGitHub::default();
    gh.set_issue("Acme/widgets", github_issue(1, "Epic", &[], T0));
    gh.set_issue("Acme/widgets", github_issue(2, "New Epic", &[], T0));
    gh.set_issue("Acme/widgets", github_issue(3, "Blocker", &[], T0));
    gh.set_issue("Acme/widgets", github_issue(4, "Task", &[], T0));

    let repo = github::read_repo(&gh, "Acme", "widgets").await.unwrap();
    let epic = github::read_issue_node(&gh, "Acme", "widgets", 1).await.unwrap();
    let new_epic = github::read_issue_node(&gh, "Acme", "widgets", 2).await.unwrap();
    let blocker = github::read_issue_node(&gh, "Acme", "widgets", 3).await.unwrap();
    let task = github::read_issue_node(&gh, "Acme", "widgets", 4).await.unwrap();

    // addBlockedBy -> Linked, repeat -> AlreadyLinked, removeBlockedBy -> Removed, repeat -> NotLinked.
    assert_eq!(github::add_blocked_by(&gh, &task.id, &blocker.id).await.unwrap(), LinkOutcome::Linked);
    assert_eq!(github::add_blocked_by(&gh, &task.id, &blocker.id).await.unwrap(), LinkOutcome::AlreadyLinked);
    assert_eq!(github::remove_blocked_by(&gh, &task.id, &blocker.id).await.unwrap(), UnlinkOutcome::Removed);
    assert_eq!(github::remove_blocked_by(&gh, &task.id, &blocker.id).await.unwrap(), UnlinkOutcome::NotLinked);

    // addSubIssue -> Linked, repeat -> AlreadyLinked, different parent without replace -> Err, with replace -> Linked.
    assert_eq!(github::add_sub_issue(&gh, &epic.id, &task.id, false).await.unwrap(), LinkOutcome::Linked);
    assert_eq!(github::add_sub_issue(&gh, &epic.id, &task.id, false).await.unwrap(), LinkOutcome::AlreadyLinked);
    let conflict = github::add_sub_issue(&gh, &new_epic.id, &task.id, false).await.unwrap_err();
    assert!(conflict.message.contains("already has a parent"), "{conflict}");
    assert_eq!(github::add_sub_issue(&gh, &new_epic.id, &task.id, true).await.unwrap(), LinkOutcome::Linked);
    assert_eq!(github::remove_sub_issue(&gh, &new_epic.id, &task.id).await.unwrap(), UnlinkOutcome::Removed);
    assert_eq!(github::remove_sub_issue(&gh, &new_epic.id, &task.id).await.unwrap(), UnlinkOutcome::NotLinked);

    // ensure_label creates once then returns Existing.
    let spec = LabelSpec { name: "track: api", alt_name: Some("track:api"), color: "1d76db", description: None };
    let first = github::ensure_label(&gh, &repo, &spec).await.unwrap();
    assert!(matches!(first, EnsureLabelOutcome::Created(_)));
    let second = github::ensure_label(&gh, &repo, &spec).await.unwrap();
    assert!(matches!(second, EnsureLabelOutcome::Existing(_)));

    // Both addSubIssue and removeSubIssue pass GraphQL-Features: sub_issues header.
    let sub_ops: Vec<Value> = gh
        .calls()
        .into_iter()
        .filter(|c| matches!(c["operation"].as_str(), Some("addSubIssue" | "removeSubIssue")))
        .collect();
    assert!(!sub_ops.is_empty());
    for c in sub_ops {
        assert_eq!(c["headers"]["GraphQL-Features"], "sub_issues", "{c}");
    }
}

#[test]
fn catalog_exposes_extended_wf_issue_and_wf_file_issues() {
    let entries = t3_codemode::catalog::builtins();
    let issue = entries.iter().find(|e| e.name == "wf.issue").unwrap();
    for field in ["blockedBy", "parent", "track", "priority", "sync"] {
        assert!(issue.signature.contains(field), "wf.issue signature missing {field}: {}", issue.signature);
        assert!(issue.input_schema["properties"].get(field).is_some(), "wf.issue schema missing {field}");
    }
    let file_issues = entries.iter().find(|e| e.name == "wf.fileIssues").unwrap();
    for field in ["blockedBy", "parent", "track", "priority", "touches", "queue"] {
        assert!(
            file_issues.signature.contains(field),
            "wf.fileIssues signature missing {field}: {}",
            file_issues.signature
        );
    }
}

#[tokio::test]
async fn wf_file_issues_returns_completed_results_when_a_later_create_issue_fails() {
    let h = H::new();
    h.github.add_repo("Acme/widgets");
    h.github.set_issue("Acme/widgets", github_issue(10, "Existing prereq", &[], T0));
    h.github.fault(Rule::on("addBlockedBy").fail(Failure::typed("FORBIDDEN", "No permission for blockedBy")));
    h.github.fault(Rule::on("createIssue").nth(Nth::From(2)).fail(Failure::typed("FORBIDDEN", "Secondary rate limit")));

    let r = result(
        &h.exec_as(
            WORKER,
            r##"async () => wf.fileIssues({
                repo: "Acme/widgets",
                issues: [
                    { id: "A", title: "First issue", blockedBy: ["#10"] },
                    { id: "B", title: "Second issue", blockedBy: ["A"] },
                    { id: "C", title: "Third issue", blockedBy: ["B"] },
                ],
            })"##,
        )
        .await,
    );

    assert_eq!(r["ok"], false, "{r}");
    assert_eq!(r["order"], json!(["A"]));
    assert_eq!(r["issues"].as_array().unwrap().len(), 1);
    assert_eq!(r["issues"][0]["url"], "https://github.com/Acme/widgets/issues/11");
    assert_eq!(r["issues"][0]["wiring"]["ok"], false);
    assert_eq!(r["issues"][0]["wiring"]["blockedBy"][0]["status"], "error");
    assert_eq!(r["issues"][0]["reported"]["added"], true);
    assert_eq!(r["byId"]["A"]["number"], 11);
    assert!(r["byId"].get("B").is_none());
    let errs = r["errors"].as_array().unwrap();
    assert_eq!(errs.len(), 2, "{errs:?}");
    assert!(
        errs.iter().any(|e| e["op"] == "addBlockedBy" && e["issue"] == "Acme/widgets#11" && e["permission"] == true)
    );
    assert!(errs.iter().any(|e| e["op"] == "createIssue" && e["issue"] == "B" && e["permission"] == true));
    assert!(h.github.issue("Acme/widgets", 12).is_none(), "C must not be created after B failed");
}

#[tokio::test]
async fn wf_file_issues_reports_board_store_failure_without_hiding_created_github_issues() {
    let h = H::new();
    h.github.add_repo("Acme/widgets");
    {
        let db = rusqlite::Connection::open(h.app.store.path()).unwrap();
        db.execute(
            "CREATE TRIGGER fail_second_issue BEFORE INSERT ON new_issues \
             WHEN NEW.key = 'acme/widgets#2' \
             BEGIN SELECT RAISE(ABORT, 'simulated store failure'); END;",
            [],
        )
        .unwrap();
    }

    let r = result(
        &h.exec_as(
            WORKER,
            r#"async () => wf.fileIssues({
                repo: "Acme/widgets",
                issues: [
                    { id: "A", title: "First issue" },
                    { id: "B", title: "Second issue", blockedBy: ["A"] },
                    { id: "C", title: "Third issue", blockedBy: ["B"] },
                ],
            })"#,
        )
        .await,
    );

    assert_eq!(r["ok"], false, "{r}");
    assert_eq!(r["order"], json!(["A", "B", "C"]));
    assert_eq!(r["issues"].as_array().unwrap().len(), 3);
    assert_eq!(r["issues"][0]["reported"]["added"], true);
    assert_eq!(r["issues"][1]["url"], "https://github.com/Acme/widgets/issues/2");
    assert_eq!(r["issues"][1]["reported"]["added"], false);
    assert!(r["issues"][1]["reported"]["error"].as_str().unwrap().contains("simulated store failure"), "{r}");
    assert_eq!(r["issues"][2]["url"], "https://github.com/Acme/widgets/issues/3");
    assert_eq!(r["issues"][2]["reported"]["added"], true);
    assert_eq!(r["errors"].as_array().unwrap().len(), 1);
    assert_eq!(r["errors"][0]["op"], "reportBoard");
    assert_eq!(r["errors"][0]["issue"], "Acme/widgets#2");
}
