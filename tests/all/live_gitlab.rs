use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use t3_codemode::app::App;
use t3_codemode::board::BOARD;
use t3_codemode::clock::{Clock, SystemClock, iso};
use t3_codemode::config::Config;
use t3_codemode::fake::{FakeDesktop, FakeRemotes, FakeT3, FakeTailscale, merge_request, thread};
use t3_codemode::github::GhCli;
use t3_codemode::gitlab::{GlabCli, project_id};
use t3_codemode::store::{Store, WaitStatus};

const TICKET: &str = "mcp:live-ticket";
const WORKER: &str = "mcp:live-worker";
const HOST: &str = "T3CM_LIVE_GITLAB_HOST";
const DIRECT: &str = "T3CM_LIVE_GITLAB_DIRECT_PROJECT";
const TRAIN: &str = "T3CM_LIVE_GITLAB_TRAIN_PROJECT";

fn run(dir: &Path, cmd: &str, args: &[&str]) -> String {
    let out = Command::new(cmd).args(args).current_dir(dir).output().unwrap();
    assert!(out.status.success(), "{cmd} {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn var(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("set {name} for the live GitLab tests"))
}

struct Live {
    _dir: tempfile::TempDir,
    app: Arc<App>,
    fake: Arc<FakeT3>,
    host: String,
    path: String,
    clone: PathBuf,
    mrs: std::sync::Mutex<Vec<u64>>,
}

impl Live {
    async fn new(project: &str) -> Live {
        let host = var(HOST);
        let path = var(project);
        let dir = tempfile::tempdir().unwrap();
        run(dir.path(), "git", &["clone", "-q", &format!("https://{host}/{path}.git"), "clone"]);
        let clone = dir.path().join("clone");
        let fake = Arc::new(FakeT3::with_tools(Vec::new()));
        let now = iso(SystemClock.now());
        for id in [TICKET, WORKER] {
            fake.put(thread(id, "idle", &now));
        }
        std::fs::create_dir(dir.path().join("store")).unwrap();
        let store = Arc::new(Store::open(&dir.path().join("store")).unwrap());
        let config = Config::from_value(json!({"forges": {&host: {"kind": "gitlab"}}})).unwrap();
        let app = App::with_forges(
            config,
            store,
            fake.clone(),
            Arc::new(SystemClock),
            Arc::new(FakeRemotes::default()),
            Arc::new(GhCli),
            Arc::new(GlabCli),
            Arc::new(FakeDesktop::default()),
            Arc::new(FakeTailscale::default()),
        );
        let live = Live { _dir: dir, app, fake, host, path, clone, mrs: Default::default() };
        live.wf(TICKET, "join", json!({"parent": BOARD, "label": "sandbox#1", "role": "ticket"})).await;
        live.wf(WORKER, "join", json!({"parent": TICKET, "label": "sandbox PR 1/1", "role": "worker"})).await;
        live
    }

    fn key(&self) -> String {
        format!("{}/{}", self.host, self.path)
    }

    async fn wf(&self, me: &str, method: &str, args: Value) -> Value {
        self.app.wf.dispatch(method, Some(me), args).await.unwrap()
    }

    fn api(&self, args: &[&str]) -> Value {
        let mut all = vec!["api", "--hostname", &self.host];
        all.extend_from_slice(args);
        serde_json::from_str(&run(&self.clone, "glab", &all)).unwrap_or(Value::Null)
    }

    fn mr(&self, iid: u64) -> Value {
        self.api(&[&format!("projects/{}/merge_requests/{iid}", project_id(&self.path))])
    }

    async fn open_mr(&self, name: &str, delay: u32) -> (u64, String) {
        let branch = format!("live/{name}-{}", SystemClock.now().as_second());
        let c = &self.clone;
        run(c, "git", &["fetch", "-q", "origin", "main"]);
        run(c, "git", &["switch", "-q", "-C", &branch, "origin/main"]);
        std::fs::create_dir_all(c.join("ci")).unwrap();
        std::fs::write(c.join("ci/delay"), format!("{delay}\n")).unwrap();
        std::fs::create_dir_all(c.join("runs")).unwrap();
        let file = format!("runs/{}.txt", branch.replace('/', "-"));
        std::fs::write(c.join(&file), format!("{}\n", iso(SystemClock.now()))).unwrap();
        run(c, "git", &["add", "ci/delay", &file]);
        run(
            c,
            "git",
            &["-c", "user.name=t3cm live test", "-c", "user.email=noreply@example.com", "commit", "-qm", &branch],
        );
        run(c, "git", &["push", "-q", "origin", &branch]);
        let head = run(c, "git", &["rev-parse", "HEAD"]);
        let created = self.api(&[
            "-X",
            "POST",
            &format!("projects/{}/merge_requests", project_id(&self.path)),
            "-f",
            &format!("source_branch={branch}"),
            "-f",
            "target_branch=main",
            "-f",
            &format!("title={branch}"),
            "-f",
            "remove_source_branch=true",
        ]);
        let iid = created["iid"].as_u64().unwrap_or_else(|| panic!("no merge request created: {created}"));
        self.mrs.lock().unwrap().push(iid);
        self.fake.set_prs(WORKER, vec![merge_request(&self.host, &self.path, iid, "open")]);
        self.app.wf.tick().await.unwrap();
        (iid, head)
    }

    fn events(&self) -> Vec<String> {
        self.fake
            .sent()
            .iter()
            .filter(|s| s["threadId"] == TICKET)
            .filter_map(|s| s["message"].as_str().map(String::from))
            .filter(|m| m.contains("[t3cm event from=gitlab"))
            .collect()
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

    async fn mergeable(&self, iid: u64) {
        let start = Instant::now();
        while self.mr(iid)["detailed_merge_status"] != "mergeable" {
            assert!(start.elapsed() < Duration::from_secs(600), "!{iid} never turned mergeable: {}", self.mr(iid));
            tokio::time::sleep(Duration::from_secs(10)).await;
        }
    }

    async fn merge(&self, iid: u64, head: &str) -> Value {
        self.wf(TICKET, "merge", json!({"repo": self.key(), "pr": iid, "head": head})).await
    }
}

impl Drop for Live {
    fn drop(&mut self) {
        for iid in self.mrs.lock().unwrap().iter() {
            let path = format!("projects/{}/merge_requests/{iid}", project_id(&self.path));
            let _ = Command::new("glab")
                .args(["api", "--hostname", &self.host, "-X", "PUT", &path, "-f", "state_event=close"])
                .current_dir(&self.clone)
                .output();
        }
    }
}

#[tokio::test]
#[ignore = "live GitLab: set T3CM_LIVE_GITLAB_HOST and T3CM_LIVE_GITLAB_DIRECT_PROJECT, a sandbox without merge trains"]
async fn live_a_mergeable_merge_request_merges_directly_at_the_approved_head() {
    let l = Live::new(DIRECT).await;
    let (iid, head) = l.open_mr("direct", 0).await;
    l.mergeable(iid).await;

    let stale = l.merge(iid, &"0".repeat(40)).await;
    assert_eq!(stale["status"], "refused", "{stale}");
    let out = l.merge(iid, &head).await;

    assert_eq!(out["status"], "merged", "{out}");
    let mr = l.mr(iid);
    assert_eq!(mr["state"], "merged");
    assert_eq!(mr["sha"], head.as_str());
    l.app.wf.reconcile().await.unwrap();
    assert!(l.events().is_empty(), "{:?}", l.events());
}

#[tokio::test]
#[ignore = "live GitLab: set T3CM_LIVE_GITLAB_HOST and T3CM_LIVE_GITLAB_DIRECT_PROJECT, a sandbox whose pipeline sleeps ci/delay seconds"]
async fn live_a_waiting_merge_request_merges_once_its_pipeline_passes() {
    let l = Live::new(DIRECT).await;
    let (iid, head) = l.open_mr("waiting", 90).await;
    tokio::time::sleep(Duration::from_secs(20)).await;

    let out = l.merge(iid, &head).await;
    assert_eq!(out["status"], "waiting", "{out}");
    assert_ne!(l.mr(iid)["merge_when_pipeline_succeeds"], true, "t3cm holds the approval, not GitLab");

    l.until("the merge", 900, |l| l.mr(iid)["state"] == "merged").await;
    assert_eq!(l.mr(iid)["sha"], head.as_str());
    assert!(l.app.store.merge_waits().unwrap().iter().all(|w| w.pr != iid || w.status == WaitStatus::Sent));
}

#[tokio::test]
#[ignore = "live GitLab: set T3CM_LIVE_GITLAB_HOST and T3CM_LIVE_GITLAB_TRAIN_PROJECT, a sandbox with merge trains"]
async fn live_a_merge_train_project_queues_the_merge_request_and_its_merge_wakes_the_ticket() {
    let l = Live::new(TRAIN).await;
    let (iid, head) = l.open_mr("train", 0).await;
    l.mergeable(iid).await;

    let out = l.merge(iid, &head).await;
    assert_eq!(out["status"], "queued", "{out}");
    eprintln!("position: {}", out["position"]);

    l.until("the merge event", 1200, |l| !l.events().is_empty()).await;
    assert_eq!(l.mr(iid)["state"], "merged");
    assert!(l.events()[0].contains("merged on GitLab"), "{:?}", l.events());
    assert!(l.app.store.merge_waits().unwrap().is_empty());
}
