use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use t3_codemode::app::App;
use t3_codemode::board::BOARD;
use t3_codemode::clock::{Clock, SystemClock, iso};
use t3_codemode::config::Config;
use t3_codemode::fake::{FakeDesktop, FakeRemotes, FakeT3, pull_request, thread};
use t3_codemode::github::GhCli;
use t3_codemode::store::Store;

const TICKET: &str = "mcp:live-ticket";
const WORKER: &str = "mcp:live-worker";
const QUEUE: &str = "T3CM_LIVE_QUEUE_REPO";
const DIRECT: &str = "T3CM_LIVE_DIRECT_REPO";
const TIMELINE: &str = "query($o: String!, $r: String!, $n: Int!) { repository(owner: $o, name: $r) { \
pullRequest(number: $n) { state headRefOid mergeStateStatus isInMergeQueue mergeCommit { oid } \
timelineItems(first: 50, itemTypes: [ADDED_TO_MERGE_QUEUE_EVENT, AUTO_MERGE_ENABLED_EVENT, \
REMOVED_FROM_MERGE_QUEUE_EVENT]) { nodes { __typename ... on RemovedFromMergeQueueEvent { reason } } } } } }";

fn run(dir: &Path, cmd: &str, args: &[&str]) -> String {
    let out = Command::new(cmd).args(args).current_dir(dir).output().unwrap();
    assert!(out.status.success(), "{cmd} {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

struct Live {
    _dir: tempfile::TempDir,
    app: Arc<App>,
    fake: Arc<FakeT3>,
    repo: String,
    clone: PathBuf,
    prs: std::sync::Mutex<Vec<u64>>,
}

impl Live {
    async fn new(var: &str) -> Live {
        let repo = std::env::var(var).unwrap_or_else(|_| panic!("set {var} to a sandbox repo as owner/name"));
        let dir = tempfile::tempdir().unwrap();
        run(dir.path(), "git", &["clone", "-q", &format!("https://github.com/{repo}.git"), "clone"]);
        let clone = dir.path().join("clone");
        let fake = Arc::new(FakeT3::with_tools(Vec::new()));
        let now = iso(SystemClock.now());
        for id in [TICKET, WORKER] {
            fake.put(thread(id, "idle", &now));
        }
        std::fs::create_dir(dir.path().join("store")).unwrap();
        let store = Arc::new(Store::open(&dir.path().join("store")).unwrap());
        let app = App::with_host(
            Config::default(),
            store,
            fake.clone(),
            Arc::new(SystemClock),
            Arc::new(FakeRemotes::default()),
            Arc::new(GhCli),
            Arc::new(FakeDesktop::default()),
        );
        let live = Live { _dir: dir, app, fake, repo, clone, prs: Default::default() };
        live.wf(TICKET, "join", json!({"parent": BOARD, "label": "sandbox#1", "role": "ticket"})).await;
        live.wf(WORKER, "join", json!({"parent": TICKET, "label": "sandbox PR 1/1", "role": "worker"})).await;
        live
    }

    async fn wf(&self, me: &str, method: &str, args: Value) -> Value {
        self.app.wf.dispatch(method, Some(me), args).await.unwrap()
    }

    fn commit(&self, branch: &str, result: &str, delay: u32) -> String {
        let c = &self.clone;
        std::fs::write(c.join("ci/result"), format!("{result}\n")).unwrap();
        std::fs::write(c.join("ci/delay"), format!("{delay}\n")).unwrap();
        std::fs::create_dir_all(c.join("runs")).unwrap();
        let file = format!("runs/{}.txt", branch.replace('/', "-"));
        std::fs::write(c.join(&file), format!("{}\n", iso(SystemClock.now()))).unwrap();
        run(c, "git", &["add", "ci/result", "ci/delay", &file]);
        run(
            c,
            "git",
            &["-c", "user.name=t3cm live test", "-c", "user.email=noreply@example.com", "commit", "-qm", branch],
        );
        run(c, "git", &["push", "-q", "origin", branch]);
        run(c, "git", &["rev-parse", "HEAD"])
    }

    async fn open_pr(&self, name: &str, result: &str, delay: u32) -> (u64, String) {
        let branch = format!("live/{name}-{}", SystemClock.now().as_second());
        run(&self.clone, "git", &["fetch", "-q", "origin", "main"]);
        run(&self.clone, "git", &["switch", "-q", "-C", &branch, "origin/main"]);
        let head = self.commit(&branch, result, delay);
        let url = run(
            &self.clone,
            "gh",
            &[
                "pr",
                "create",
                "-R",
                &self.repo,
                "--base",
                "main",
                "--head",
                &branch,
                "--title",
                &branch,
                "--body",
                "t3cm live test",
            ],
        );
        let n: u64 = url.rsplit('/').next().unwrap().parse().unwrap();
        self.prs.lock().unwrap().push(n);
        self.fake.set_prs(WORKER, vec![pull_request(&self.repo, n, "open")]);
        self.app.wf.tick().await.unwrap();
        (n, head)
    }

    fn pr(&self, n: u64) -> Value {
        let (o, r) = self.repo.split_once('/').unwrap();
        let q = format!("query={TIMELINE}");
        let out = run(
            &self.clone,
            "gh",
            &["api", "graphql", "-f", &q, "-f", &format!("o={o}"), "-f", &format!("r={r}"), "-F", &format!("n={n}")],
        );
        serde_json::from_str::<Value>(&out).unwrap()["data"]["repository"]["pullRequest"].clone()
    }

    fn timeline(&self, n: u64) -> Vec<String> {
        let pr = self.pr(n);
        let nodes = pr["timelineItems"]["nodes"].as_array().cloned().unwrap_or_default();
        nodes
            .iter()
            .map(|e| format!("{} {}", e["__typename"].as_str().unwrap(), e["reason"].as_str().unwrap_or("")))
            .collect()
    }

    fn events(&self) -> Vec<String> {
        self.fake
            .sent()
            .iter()
            .filter(|s| s["threadId"] == TICKET)
            .filter_map(|s| s["message"].as_str().map(String::from))
            .filter(|m| m.contains("[t3cm event from=github"))
            .collect()
    }

    fn cards(&self) -> Vec<(String, String)> {
        self.app.wf.cards().unwrap().into_iter().map(|c| (c.kind, c.text)).collect()
    }

    async fn until(&self, what: &str, secs: u64, done: impl Fn(&Live) -> bool) {
        let start = Instant::now();
        loop {
            if let Err(e) = self.app.wf.reconcile().await {
                eprintln!("reconcile: {e:#}");
            }
            if done(self) {
                eprintln!("{what}: after {}s", start.elapsed().as_secs());
                return;
            }
            assert!(start.elapsed() < Duration::from_secs(secs), "timed out after {secs}s waiting for {what}");
            tokio::time::sleep(Duration::from_secs(10)).await;
        }
    }

    async fn checks_pass(&self, n: u64) {
        let start = Instant::now();
        while self.pr(n)["mergeStateStatus"] != "CLEAN" {
            assert!(start.elapsed() < Duration::from_secs(300), "#{n} never turned CLEAN: {}", self.pr(n));
            tokio::time::sleep(Duration::from_secs(10)).await;
        }
    }

    async fn merge(&self, n: u64, head: &str) -> Value {
        self.wf(TICKET, "merge", json!({"repo": self.repo, "pr": n, "head": head})).await
    }

    async fn gate(&self, n: u64) {
        let body = format!(
            "**ACTION NEEDED:** reply \"merge\" to merge #{n}, here: https://github.com/{}/pull/{n}",
            self.repo
        );
        self.wf(TICKET, "send", json!({"kind": "gate", "gate": "merge", "body": body})).await;
    }
}

impl Drop for Live {
    fn drop(&mut self) {
        for n in self.prs.lock().unwrap().iter() {
            let _ = Command::new("gh")
                .args(["pr", "close", &n.to_string(), "-R", &self.repo, "--delete-branch"])
                .current_dir(&self.clone)
                .output();
        }
    }
}

#[tokio::test]
#[ignore = "live GitHub: set T3CM_LIVE_QUEUE_REPO to a sandbox with a merge queue"]
async fn live_a_clean_pr_is_queued_at_once_and_its_merge_wakes_the_ticket() {
    let l = Live::new(QUEUE).await;
    let (n, head) = l.open_pr("clean", "pass", 0).await;
    l.checks_pass(n).await;

    let out = l.merge(n, &head).await;
    assert_eq!(out["status"], "queued", "{out}");

    l.until("the merge event", 600, |l| !l.events().is_empty()).await;
    let timeline = l.timeline(n);
    assert!(timeline.iter().any(|e| e.starts_with("AddedToMergeQueueEvent")), "{timeline:?}");
    assert!(!timeline.iter().any(|e| e.starts_with("AutoMergeEnabledEvent")), "{timeline:?}");
    assert_eq!(l.events().len(), 1);
    assert!(l.app.store.merge_waits().unwrap().is_empty());
}

#[tokio::test]
#[ignore = "live GitHub: set T3CM_LIVE_QUEUE_REPO to a sandbox with a merge queue"]
async fn live_a_waiting_pr_is_queued_once_its_checks_pass() {
    let l = Live::new(QUEUE).await;
    let (n, head) = l.open_pr("waiting", "pass", 60).await;

    let out = l.merge(n, &head).await;
    assert_eq!(out["status"], "waiting", "{out}");

    l.until("the merge event", 900, |l| !l.events().is_empty()).await;
    let timeline = l.timeline(n);
    assert_eq!(timeline.iter().filter(|e| e.starts_with("AddedToMergeQueueEvent")).count(), 1, "{timeline:?}");
    assert!(!timeline.iter().any(|e| e.starts_with("AutoMergeEnabledEvent")), "{timeline:?}");
    assert_eq!(l.pr(n)["headRefOid"], head);
    assert!(l.app.store.merge_waits().unwrap().is_empty());
}

#[tokio::test]
#[ignore = "live GitHub: set T3CM_LIVE_QUEUE_REPO to a sandbox with a merge queue"]
async fn live_a_waiting_pr_whose_head_moves_gets_a_new_gate_and_is_not_queued() {
    let l = Live::new(QUEUE).await;
    let (n, head) = l.open_pr("moved", "pass", 60).await;
    assert_eq!(l.merge(n, &head).await["status"], "waiting");

    let branch = run(&l.clone, "git", &["branch", "--show-current"]);
    let moved = l.commit(&branch, "pass", 0);
    l.until("the new gate", 300, |l| l.cards().iter().any(|(k, t)| k == "gate" && t.contains(&moved[..7]))).await;

    assert!(l.app.store.merge_waits().unwrap().is_empty());
    assert_eq!(l.app.store.link_get(TICKET).unwrap().unwrap().gate.as_deref(), Some("merge"));
    tokio::time::sleep(Duration::from_secs(60)).await;
    l.app.wf.reconcile().await.unwrap();
    assert!(l.timeline(n).is_empty(), "{:?}", l.timeline(n));
}

#[tokio::test]
#[ignore = "live GitHub: set T3CM_LIVE_QUEUE_REPO to a sandbox with a merge queue"]
async fn live_a_pr_the_queue_drops_gets_one_report_with_the_reason() {
    let l = Live::new(QUEUE).await;
    let (n, head) = l.open_pr("dropped", "fail-in-queue", 0).await;
    l.checks_pass(n).await;
    assert_eq!(l.merge(n, &head).await["status"], "queued");

    l.until("the drop report", 900, |l| l.cards().iter().any(|(k, _)| k == "report")).await;
    let cards = l.cards();
    assert_eq!(cards.len(), 1, "{cards:?}");
    eprintln!("report: {}", cards[0].1);
    assert!(cards[0].1.contains("dropped from the merge queue"), "{cards:?}");
    assert!(l.app.store.merge_waits().unwrap().is_empty());
    eprintln!("timeline: {:?}", l.timeline(n));
    l.app.wf.reconcile().await.unwrap();
    assert_eq!(l.cards().len(), 1);
    assert_eq!(l.pr(n)["isInMergeQueue"], false);
}

#[tokio::test]
#[ignore = "live GitHub: set T3CM_LIVE_DIRECT_REPO to a sandbox without a merge queue"]
async fn live_a_hand_merge_resolves_the_gate_and_wakes_the_ticket_once() {
    let l = Live::new(DIRECT).await;
    let (n, _) = l.open_pr("by-hand", "pass", 0).await;
    l.gate(n).await;
    l.app.wf.reconcile().await.unwrap();
    l.checks_pass(n).await;

    run(&l.clone, "gh", &["pr", "merge", &n.to_string(), "-R", &l.repo, "--rebase"]);
    l.until("the merge event", 300, |l| !l.events().is_empty()).await;

    assert!(l.cards().is_empty(), "{:?}", l.cards());
    assert_eq!(l.app.store.link_get(TICKET).unwrap().unwrap().gate, None);
    l.app.wf.reconcile().await.unwrap();
    assert_eq!(l.events().len(), 1);
}

#[tokio::test]
#[ignore = "live GitHub: set T3CM_LIVE_DIRECT_REPO to a sandbox without a merge queue"]
async fn live_a_pr_closed_on_github_resolves_its_card_and_tells_the_ticket() {
    let l = Live::new(DIRECT).await;
    let (n, _) = l.open_pr("closed", "pass", 0).await;
    l.gate(n).await;
    l.app.wf.reconcile().await.unwrap();

    run(&l.clone, "gh", &["pr", "close", &n.to_string(), "-R", &l.repo]);
    l.until("the close event", 300, |l| !l.events().is_empty()).await;

    assert!(l.cards().is_empty(), "{:?}", l.cards());
    assert!(l.events()[0].contains("closed on GitHub without merging"), "{:?}", l.events());
}

#[tokio::test]
#[ignore = "live GitHub: set T3CM_LIVE_DIRECT_REPO to a sandbox without a merge queue"]
async fn live_a_repo_without_a_queue_merges_directly_at_the_approved_head() {
    let l = Live::new(DIRECT).await;
    let (n, head) = l.open_pr("direct", "pass", 30).await;
    let blocked = l.merge(n, &head).await;
    assert_eq!(blocked["status"], "refused", "{blocked}");
    l.checks_pass(n).await;

    let stale = l.merge(n, &"0".repeat(40)).await;
    assert_eq!(stale["status"], "refused", "{stale}");
    let out = l.merge(n, &head).await;

    assert_eq!(out["status"], "merged", "{out}");
    assert_eq!(out["method"], "rebase", "{out}");
    let pr = l.pr(n);
    assert_eq!(pr["state"], "MERGED");
    assert_eq!(pr["headRefOid"], head);
    l.app.wf.reconcile().await.unwrap();
    assert!(l.events().is_empty(), "{:?}", l.events());
}
