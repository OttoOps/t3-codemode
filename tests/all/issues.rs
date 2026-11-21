use crate::common::*;
use serde_json::{Value, json};
use t3_codemode::board::BOARD;
use t3_codemode::fake::project;
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
    h.fake.set_read_fails(true);
    let r = result(&h.exec_as(WORKER, REPORT).await);
    assert_eq!(r["added"], true);
    assert_eq!(r["issue"]["thread"], WORKER);
    assert_eq!(r["issue"]["threadLink"], Value::Null);
    h.fake.set_read_fails(false);
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
