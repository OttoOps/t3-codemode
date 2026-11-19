use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use t3_codemode::app::App;
use t3_codemode::board::{BOARD, LaunchArgs};
use t3_codemode::clock::{FakeClock, iso, plus_minutes};
use t3_codemode::config::Config;
use t3_codemode::fake::{FakeDesktop, FakeGitHub, FakeRemotes, FakeT3, project, thread};
use t3_codemode::server;
use t3_codemode::store::Store;

struct Demo {
    app: Arc<App>,
    fake: Arc<FakeT3>,
    clock: Arc<FakeClock>,
    start: jiff::Timestamp,
}

type Pr = Option<(&'static str, u64, &'static str)>;

const REPOS: &[&str] = &["widgets", "gizmo", "ledger", "atlas", "relay"];

const NOTES: &[&str] = &[
    "build green at 4f2c9a1",
    "sol-loop round 1 started on 3 commits",
    "sol-loop agreed after 2 rounds",
    "cargo test --release passed (412 tests)",
    "rebased on origin/main, no conflicts",
    "checks pending on head 9b1e07d",
    "opened PR, waiting on CI",
    "fixing clippy::needless_borrow in src/sync.rs",
    "migration dry run finished, 0 rows changed",
    "docs build passed, 2 broken links fixed",
    "worker 2 picked up PR 2/3",
    "retrying flaky integration test (1 of 2)",
    "deploy pipeline queued for tag v0.9.4",
    "review comment answered, pushed 1 fix",
    "bench: p95 down from 182 ms to 141 ms",
];

const GATES: &[(&str, &str)] = &[
    (
        "merge",
        "PR #81 is green and reviewed.\n**ACTION NEEDED:** reply \"merge\" to merge #81, or reply \"merge and deploy\" to ship it too, here: https://github.com/acme/widgets/pull/81",
    ),
    (
        "review",
        "sol-loop is stuck on one finding after two replies: the retry budget in src/relay.rs.\n**ACTION NEEDED:** reply \"accept\" to keep the current budget, or reply \"fix\" to lower it to 3, here: https://github.com/acme/relay/pull/17",
    ),
    (
        "sso",
        "AWS SSO session for profile acme-dev expired during the deploy step.\n**ACTION NEEDED:** approve the AWS sign-in tab that is already open in your browser",
    ),
    (
        "merge",
        "PR #203 updated after review, checks passed on 7d1c2e0.\n**ACTION NEEDED:** reply \"merge\" to merge #203, here: https://github.com/acme/ledger/pull/203",
    ),
    (
        "scope",
        "Issue #57 asks for both the CSV export and the API endpoint. One PR or two?\n**ACTION NEEDED:** reply \"one\" for a single PR, or reply \"split\" for two PRs, here: https://github.com/acme/atlas/issues/57",
    ),
];

const TITLES: &[&str] = &[
    "Retry failed webhook deliveries with backoff",
    "Add CSV export to the usage report",
    "Move tenant settings into their own table",
    "Show deploy status on the release page",
    "Cache the pricing lookup per request",
    "Sign Windows builds in CI",
    "Drop the legacy v1 sync endpoint",
    "Paginate the audit log API",
    "Speed up the nightly reconcile job",
    "Fix flaky integration test teardown",
    "Let admins rotate API keys",
    "Report p95 latency per route",
];

fn title_for(label: &str) -> String {
    let n: usize = label.bytes().map(usize::from).sum();
    format!("{label}: {}", TITLES[n % TITLES.len()])
}

const ASK_ANSWER: &str = "atlas#57 has waited longest: one PR or two for the CSV export.";

fn id(kind: &str, n: usize) -> String {
    format!("mcp:{kind}:{n}")
}

impl Demo {
    fn new() -> Demo {
        let dir = std::env::var("BOARD_DEMO_STATE").unwrap_or_else(|_| {
            let d = tempfile::tempdir().unwrap();
            d.keep().to_string_lossy().into_owned()
        });
        let fake = Arc::new(FakeT3::with_tools(Vec::new()));
        let start = plus_minutes(jiff::Timestamp::now(), -180.0);
        let clock = Arc::new(FakeClock::at(&iso(start)));
        let store = Arc::new(Store::open(std::path::Path::new(&dir)).unwrap());
        let remotes = Arc::new(FakeRemotes::default());
        for r in REPOS {
            remotes.set(&format!("/src/acme/{r}"), &format!("git@github.com:acme/{r}.git"));
        }
        fake.set_projects(REPOS.iter().map(|r| project(&format!("proj:{r}"), &format!("/src/acme/{r}"))).collect());
        let app = App::with_host(
            Config::default(),
            store,
            fake.clone(),
            clock.clone(),
            remotes,
            Arc::new(FakeGitHub::default()),
            Arc::new(FakeDesktop::default()),
        );
        Demo { app, fake, clock, start }
    }

    fn at(&self, minutes: f64) {
        self.clock.set(&iso(plus_minutes(self.start, minutes)));
    }

    fn now(&self) {
        self.clock.set(&iso(jiff::Timestamp::now()));
    }

    async fn wf(&self, me: &str, method: &str, args: Value) -> Value {
        self.app.wf.dispatch(method, Some(me), args).await.unwrap_or_else(|e| panic!("wf.{method} as {me}: {e:#}"))
    }

    fn put(&self, me: &str, status: &str, title: &str, pr: Option<(&str, u64, &str)>) {
        let mut th = thread(me, status, &iso(jiff::Timestamp::now()));
        th["title"] = json!(title);
        th["link"] = json!(format!("https://t3.example/threads/{}", me.replace(':', "-")));
        if let Some((repo, n, state)) = pr {
            th["pullRequests"] = json!([{"number": n, "url": format!("https://github.com/acme/{repo}/pull/{n}"),
                                         "state": state, "repository": format!("acme/{repo}")}]);
        }
        self.fake.put(th);
    }

    async fn ticket(&self, n: usize, label: &str, status: &str, pr: Option<(&str, u64, &str)>) -> String {
        let me = id("ticket", n);
        self.put(&me, status, &title_for(label), pr);
        self.wf(&me, "join", json!({"parent": BOARD, "label": label, "role": "ticket"})).await;
        me
    }

    async fn worker(&self, ticket: &str, n: usize, label: &str, status: &str, pr: Option<(&str, u64, &str)>) -> String {
        let me = id("worker", n);
        self.put(&me, status, label, pr);
        self.wf(&me, "join", json!({"parent": ticket, "label": label, "role": "worker"})).await;
        me
    }

    async fn note(&self, me: &str, body: &str) {
        self.wf(me, "send", json!({"kind": "note", "body": body})).await;
    }

    async fn seed(&self) {
        if std::env::var("BOARD_DEMO_SCENARIO").as_deref() == Ok("empty") {
            self.now();
            return;
        }
        let mut open = Vec::new();
        let mut w = 0;
        let mut minute = 0.0;
        let plan: &[(&str, &str, Pr, usize)] = &[
            ("widgets#41", "running", Some(("widgets", 81, "open")), 2),
            ("widgets#44", "running", None, 1),
            ("gizmo#12", "waiting", Some(("gizmo", 30, "open")), 1),
            ("ledger#198", "running", Some(("ledger", 203, "open")), 2),
            ("ledger#201", "idle", None, 0),
            ("atlas#57", "waiting", None, 0),
            ("atlas#60", "running", Some(("atlas", 64, "open")), 1),
            ("relay#9", "running", Some(("relay", 17, "open")), 1),
            ("relay#11", "failed", None, 1),
            ("widgets#47", "running", None, 0),
            ("gizmo#15", "running", Some(("gizmo", 33, "draft")), 1),
            ("ledger#205", "running", None, 0),
            ("atlas#62", "running", None, 1),
            ("relay#14", "idle", None, 0),
        ];
        for (i, (label, status, pr, workers)) in plan.iter().enumerate() {
            minute += 4.0;
            self.at(minute);
            let t = self.ticket(i + 1, label, status, *pr).await;
            for k in 0..*workers {
                w += 1;
                let wl = format!("{label} PR {}/{}", k + 1, workers);
                let ws = if k + 1 == *workers { *status } else { "idle" };
                let wpr = pr.map(|(r, n, s)| (r, n + k as u64, s));
                self.worker(&t, w, &wl, ws, wpr).await;
            }
            open.push(t);
        }
        for (k, t) in open.iter().enumerate() {
            for j in 0..3 {
                minute += 1.5;
                self.at(minute);
                self.note(t, NOTES[(k * 3 + j) % NOTES.len()]).await;
            }
        }
        for (k, (gate, body)) in GATES.iter().enumerate() {
            minute += 3.0;
            self.at(minute);
            let t = &open[[0, 7, 6, 3, 5][k]];
            self.wf(t, "send", json!({"kind": "gate", "gate": gate, "body": body})).await;
        }
        minute += 2.0;
        self.at(minute);
        self.wf(&open[8], "send", json!({"kind": "blocker",
            "body": "relay#11 worker failed twice on the same migration: column \"tenant_id\" already exists. Needs a decision on whether to drop the partial migration."})).await;
        self.wf(&open[10], "send", json!({"kind": "report",
            "body": "gizmo#15 ran 3 h with no PR. Worker 1 keeps re-running the full suite. Reply \"stop\" to end it, or reply \"continue\" to give it another hour."})).await;
        for d in 0..4 {
            minute += 2.0;
            self.at(minute);
            let label = format!("{}#{}", REPOS[d % REPOS.len()], 90 + d);
            let t =
                self.ticket(100 + d, &label, "idle", Some((REPOS[d % REPOS.len()], 300 + d as u64, "merged"))).await;
            self.note(&t, "PR merged, deploy succeeded").await;
            self.wf(&t, "send", json!({"kind": "handoff", "body": "merged and deployed"})).await;
        }
        for s in 0..26 {
            self.at(minute - 170.0 + s as f64);
            let label = format!("{}#{}", REPOS[s % REPOS.len()], 2 + s);
            let t =
                self.ticket(200 + s, &label, "idle", Some((REPOS[s % REPOS.len()], 100 + s as u64, "merged"))).await;
            self.wf(&t, "send", json!({"kind": "handoff", "body": "merged"})).await;
            self.app.wf.settle_ticket(&t).await.unwrap();
        }
        self.now();
        let questions = [
            (
                "Which tickets are waiting on me, and why?",
                Some(
                    "Three: widgets#41 (merge #81), relay#9 (sol-loop dispute on the retry budget), atlas#57 (one PR or two). relay#11 is blocked on a failed migration.",
                ),
            ),
            (
                "Is anything stuck for more than an hour?",
                Some("gizmo#15 has run 3 h with no PR; its worker keeps re-running the full suite."),
            ),
            ("What merged today?", None),
        ];
        for (q, a) in questions {
            let r = self.app.wf.ask(q).await.unwrap();
            let th = r["thread"].as_str().unwrap().to_string();
            if let Some(a) = a {
                self.fake.set_items(&th, vec![json!({"type": "assistant_message", "text": a})]);
                self.fake.update(&th, json!({"status": "idle"}));
            }
        }
        self.app.wf.check_asks(None).await.unwrap();
        self.app
            .wf
            .launch_ticket(LaunchArgs {
                issue: "https://github.com/acme/gizmo/issues/88".into(),
                project: None,
                title: None,
                model: None,
                effort: None,
            })
            .await
            .unwrap();
        self.app.wf.tick().await.unwrap();
    }

    async fn live(&self) {
        let every: u64 = std::env::var("BOARD_DEMO_EVERY").ok().and_then(|s| s.parse().ok()).unwrap_or(4);
        if every == 0 {
            loop {
                self.now();
                let _ = self.app.wf.tick().await;
                tokio::time::sleep(Duration::from_secs(20)).await;
            }
        }
        let churn = std::env::var("BOARD_DEMO_CHURN").as_deref() == Ok("1");
        let mut late: std::collections::VecDeque<String> = std::collections::VecDeque::new();
        let mut gated: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut n = 0usize;
        loop {
            tokio::time::sleep(Duration::from_secs(every)).await;
            self.now();
            n += 1;
            let t = id("ticket", 1 + (n * 5) % 14);
            let _ =
                self.app.wf.dispatch("send", Some(&t), json!({"kind": "note", "body": NOTES[n % NOTES.len()]})).await;
            if n.is_multiple_of(6) {
                let (gate, body) = GATES[n / 6 % GATES.len()];
                let t = match late.iter().find(|l| !gated.contains(*l)) {
                    Some(l) => l.clone(),
                    None => id("ticket", 1 + (n * 3) % 14),
                };
                if self
                    .app
                    .wf
                    .dispatch("send", Some(&t), json!({"kind": "gate", "gate": gate, "body": body}))
                    .await
                    .is_ok()
                {
                    gated.insert(t);
                }
            }
            if n.is_multiple_of(9) {
                let label = format!("{}#{}", REPOS[n % REPOS.len()], 500 + n);
                let me = id("late", n);
                self.put(&me, "running", &title_for(&label), None);
                if self
                    .app
                    .wf
                    .dispatch("join", Some(&me), json!({"parent": BOARD, "label": label, "role": "ticket"}))
                    .await
                    .is_ok()
                {
                    late.push_back(me);
                }
            }
            if churn && n.is_multiple_of(5) {
                if let Some(c) = self.app.wf.cards().ok().and_then(|c| c.into_iter().next()) {
                    let _ = self.app.wf.dismiss(&c.id);
                }
                if let Some(t) = late.pop_front() {
                    gated.remove(&t);
                    let _ = self.app.wf.dispatch("send", Some(&t), json!({"kind": "handoff", "body": "merged"})).await;
                }
            }
            let asked: Vec<String> = self
                .app
                .wf
                .asks()
                .unwrap_or_default()
                .iter()
                .filter(|a| a["status"] == "running" && a["question"] != "What merged today?")
                .filter_map(|a| a["thread"].as_str().map(String::from))
                .collect();
            for th in &asked {
                self.fake.set_items(th, vec![json!({"type": "assistant_message", "text": ASK_ANSWER})]);
                self.fake.update(th, json!({"status": "idle"}));
            }
            if !asked.is_empty() {
                let _ = self.app.wf.check_asks(None).await;
            }
            let _ = self.app.wf.tick().await;
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "manual: serves a seeded board for design review"]
async fn serve_demo_board() {
    let port: u16 = std::env::var("BOARD_DEMO_PORT").ok().and_then(|s| s.parse().ok()).unwrap_or(47790);
    let demo = Demo::new();
    demo.seed().await;
    let (listener, addr) = server::bind(port).await.unwrap();
    eprintln!("BOARD_DEMO_READY http://{addr}/");
    let app = demo.app.clone();
    tokio::spawn(async move { server::serve(app, listener, std::future::pending::<()>()).await.unwrap() });
    demo.live().await;
}
