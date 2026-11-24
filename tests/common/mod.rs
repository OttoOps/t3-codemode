#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::Value;
use t3_codemode::app::{App, ExecuteRequest, ToolOutput};
use t3_codemode::clock::FakeClock;
use t3_codemode::config::Config;
use t3_codemode::drift::Home;
use t3_codemode::fake::{FakeDesktop, FakeGitHub, FakeRemotes, FakeT3, FakeTailscale, provider, thread};
use t3_codemode::store::Store;

pub const BOSS: &str = "mcp:boss";
pub const TICKET: &str = "mcp:ticket";
pub const WORKER: &str = "mcp:worker";
pub const T0: &str = "2026-11-17T12:00:00Z";

pub fn at(minutes: i64) -> String {
    let total = 12 * 60 + minutes;
    format!("2026-11-17T{:02}:{:02}:00Z", total / 60, total % 60)
}

pub fn fixture_tools() -> Vec<Value> {
    let raw = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/t3-tools.json")).unwrap();
    serde_json::from_str(&raw).unwrap()
}

pub struct H {
    pub dir: tempfile::TempDir,
    pub fake: Arc<FakeT3>,
    pub clock: Arc<FakeClock>,
    pub app: Arc<App>,
    pub remotes: Arc<FakeRemotes>,
    pub github: Arc<FakeGitHub>,
    pub desktop: Arc<FakeDesktop>,
    pub tailscale: Arc<FakeTailscale>,
    pub config: Config,
    in_memory: bool,
}

impl H {
    pub fn new() -> H {
        H::build(Vec::new(), Config::default())
    }

    pub fn new_memory() -> H {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_memory().unwrap();
        H::build_with_store(dir, Vec::new(), Config::default(), store, true)
    }

    pub fn with_tools() -> H {
        H::build(fixture_tools(), Config::default())
    }

    pub fn build(tools: Vec<Value>, config: Config) -> H {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        H::build_with_store(dir, tools, config, store, false)
    }

    fn build_with_store(dir: tempfile::TempDir, tools: Vec<Value>, config: Config, store: Store, in_memory: bool) -> H {
        let fake = Arc::new(FakeT3::with_tools(tools));
        for id in [BOSS, TICKET, WORKER] {
            fake.put(thread(id, "idle", T0));
        }
        fake.set_providers(vec![provider("claudeAgent", "claudeAgent", &["claude-opus-5-5", "claude-haiku-5-5"])]);
        let clock = Arc::new(FakeClock::at(T0));
        let store = Arc::new(store);
        let remotes = Arc::new(FakeRemotes::default());
        let github = Arc::new(FakeGitHub::default());
        let desktop = Arc::new(FakeDesktop::default());
        let tailscale = Arc::new(FakeTailscale::default());
        let app = App::with_host_and_tailscale(
            config.clone(),
            store,
            fake.clone(),
            clock.clone(),
            remotes.clone(),
            github.clone(),
            desktop.clone(),
            tailscale.clone(),
        );
        app.wf.set_skills_home(Home::under(&dir.path().join("home")));
        app.wf.set_slot_folders(t3_codemode::slots::Folders::under(&dir.path().join("home"), None));
        app.wf.set_config_path(dir.path().join("t3cm-config.json"));
        H { dir, fake, clock, app, remotes, github, desktop, tailscale, config, in_memory }
    }

    pub async fn load_catalog(&self) {
        self.app.refresh_catalog().await.unwrap();
    }

    pub fn reopen(&mut self) {
        assert!(!self.in_memory, "H::reopen requires a file-backed WAL store; use H::new or H::build");
        let store = Arc::new(Store::open(self.dir.path()).unwrap());
        self.app = App::with_host_and_tailscale(
            self.config.clone(),
            store,
            self.fake.clone(),
            self.clock.clone(),
            self.remotes.clone(),
            self.github.clone(),
            self.desktop.clone(),
            self.tailscale.clone(),
        );
        self.app.wf.set_skills_home(Home::under(&self.dir.path().join("home")));
        self.app.wf.set_slot_folders(t3_codemode::slots::Folders::under(&self.dir.path().join("home"), None));
        self.app.wf.set_config_path(self.dir.path().join("t3cm-config.json"));
    }

    pub fn peer(&self) -> Arc<App> {
        assert!(!self.in_memory, "H::peer requires a file-backed WAL store; use H::new or H::build");
        let app = App::with_host_and_tailscale(
            self.config.clone(),
            Arc::new(Store::open(self.dir.path()).unwrap()),
            self.fake.clone(),
            self.clock.clone(),
            self.remotes.clone(),
            self.github.clone(),
            self.desktop.clone(),
            self.tailscale.clone(),
        );
        app.wf.set_skills_home(Home::under(&self.dir.path().join("home")));
        app.wf.set_slot_folders(t3_codemode::slots::Folders::under(&self.dir.path().join("home"), None));
        app.wf.set_config_path(self.dir.path().join("t3cm-config.json"));
        app
    }

    pub async fn exec(&self, code: &str) -> ToolOutput {
        self.run(None, code, None).await
    }

    pub async fn exec_as(&self, caller: &str, code: &str) -> ToolOutput {
        self.run(Some(caller), code, None).await
    }

    pub async fn run(&self, caller: Option<&str>, code: &str, timeout_ms: Option<u64>) -> ToolOutput {
        self.app
            .execute(ExecuteRequest {
                code: code.into(),
                session: None,
                timeout_ms,
                caller: caller.map(String::from),
                reported: Default::default(),
            })
            .await
    }

    pub async fn wf(&self, me: &str, method: &str, args: Value) -> Value {
        match self.app.wf.dispatch(method, Some(me), args).await {
            Ok(v) => v,
            Err(e) => panic!("wf.{method} as {me} failed: {e:#}"),
        }
    }

    pub async fn wf_err(&self, me: &str, method: &str, args: Value) -> String {
        match self.app.wf.dispatch(method, Some(me), args).await {
            Ok(v) => panic!("wf.{method} as {me} should fail, got {v}"),
            Err(e) => format!("{e:#}"),
        }
    }

    pub async fn tick(&self, minutes: i64) -> Vec<String> {
        self.clock.set(&at(minutes));
        self.app.wf.tick().await.unwrap()
    }

    pub async fn advance_tick(&self, minutes: i64) -> Vec<String> {
        assert!(minutes >= 0, "advance_tick requires non-negative minutes, got {minutes}");
        self.clock.advance_minutes(minutes);
        self.app.wf.tick().await.unwrap()
    }

    pub fn sent(&self) -> Vec<Value> {
        self.fake.sent()
    }

    pub fn last_message(&self) -> String {
        self.sent().last().expect("a send")["message"].as_str().unwrap().to_string()
    }
}

pub fn ok_json(out: &ToolOutput) -> Value {
    assert!(!out.is_error, "expected success, got: {}", out.text);
    serde_json::from_str(&out.text).unwrap_or_else(|e| panic!("not JSON ({e}): {}", out.text))
}

pub fn result(out: &ToolOutput) -> Value {
    ok_json(out)["result"].clone()
}

pub fn err_text(out: &ToolOutput) -> &str {
    assert!(out.is_error, "expected an error, got: {}", out.text);
    &out.text
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct CardView {
    pub kind: String,
    pub ticket: Option<String>,
    pub worker: Option<(String, String)>,
    pub pr: Option<(String, u64)>,
    pub head: Option<String>,
    pub ready: bool,
}

pub type PrRef = (String, u64);

pub type LinkPrView = (String, u64, String, bool, Vec<PrRef>);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkView {
    pub parent: String,
    pub closed: bool,
    pub paused: bool,
    pub gate: Option<String>,
    pub gate_pr: Option<PrRef>,
    pub pr: Option<LinkPrView>,
}

pub type WaitView = (String, String, Option<String>, u32);

pub type SeenView = (bool, String, Option<String>, Option<String>);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub cards: Vec<CardView>,
    pub links: BTreeMap<String, LinkView>,
    pub merge_waits: BTreeMap<(String, u64), WaitView>,
    pub pr_events: BTreeSet<(String, u64, String, String)>,
    pub pr_seen: BTreeMap<(String, u64), SeenView>,
    pub outbox: BTreeMap<String, usize>,
    pub delivered: Vec<(String, String, String)>,
}

pub struct WakeLine {
    pub to: String,
    pub kind: String,
    pub from: String,
    pub subject: String,
}

pub struct Record {
    pub to: String,
    pub kind: String,
    pub body: String,
}

pub type Known<'a> = &'a dyn Fn(&str) -> Option<Record>;

fn header_line(line: &str) -> Option<(String, String, String)> {
    let rest = line.strip_prefix("[t3cm ")?.strip_suffix(']')?;
    let (head, tail) = rest.rsplit_once(" msg=")?;
    let handle = tail.split(' ').next()?;
    let (kind, from) = head.split_once(" from=").filter(|(k, _)| !k.is_empty() && !k.contains(' '))?;
    let from = if from.starts_with('"') {
        serde_json::from_str::<String>(from).ok()?
    } else {
        Some(from.to_string()).filter(|f| !f.contains(' '))?
    };
    (!from.is_empty() && !handle.is_empty()).then(|| (kind.to_string(), from, handle.to_string()))
}

fn body_len(after: &str, body: &str, handle: &str) -> Option<usize> {
    let n = body.chars().count();
    let capped = after.match_indices("\n... (").find_map(|(at, _)| {
        let more: usize = after[at + 6..].split(' ').next()?.parse().ok()?;
        let cap = n.checked_sub(more)?;
        let head: String = body.chars().take(cap).collect();
        let full = format!("{}\n... ({} more chars; full: wf.show(\"{handle}\"))", head.trim_end(), n - cap);
        after.starts_with(&full).then_some(full.len())
    });
    capped.or_else(|| after.starts_with(body).then_some(body.len()))
}

struct OracleCapture {
    is_oracle: AtomicBool,
    silent_all: bool,
    message: std::sync::Mutex<Option<String>>,
    location: std::sync::Mutex<Option<String>>,
}

tokio::task_local! {
    static ORACLE_CAPTURE: Arc<OracleCapture>;
}

static PANIC_HOOK_INIT: std::sync::Once = std::sync::Once::new();

fn install_quiet_oracle_panic_hook() {
    PANIC_HOOK_INIT.call_once(|| {
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let quiet = ORACLE_CAPTURE
                .try_with(|cap| {
                    if let Some(loc) = info.location()
                        && let Ok(mut slot) = cap.location.lock()
                    {
                        *slot = Some(format!("{}:{}:{}", loc.file(), loc.line(), loc.column()));
                    }
                    cap.silent_all || cap.is_oracle.load(Ordering::Relaxed)
                })
                .unwrap_or(false);
            if !quiet {
                prev(info);
            }
        }));
    });
}

fn panic_payload_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else if let Some(s) = payload.downcast_ref::<&'static str>() {
        (*s).to_string()
    } else {
        "<non-string panic payload>".to_string()
    }
}

pub fn oracle_fail(msg: String) -> ! {
    let _ = ORACLE_CAPTURE.try_with(|cap| {
        cap.is_oracle.store(true, Ordering::Relaxed);
        if let Ok(mut slot) = cap.message.lock() {
            *slot = Some(msg.clone());
        }
    });
    panic!("{msg}");
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProbeResult<T> {
    Ok(T),
    OracleFailure(String),
    OtherPanic { message: String, location: Option<String> },
}

enum CaptureOutcome<T> {
    Ok(T),
    OracleFailure(String),
    OtherPanic { payload: Box<dyn std::any::Any + Send>, location: Option<String> },
}

async fn run_captured<T, F>(fut: F, silent_all: bool) -> CaptureOutcome<T>
where
    T: Send + 'static,
    F: std::future::Future<Output = T> + Send + 'static,
{
    install_quiet_oracle_panic_hook();
    let cap = Arc::new(OracleCapture {
        is_oracle: AtomicBool::new(false),
        silent_all,
        message: std::sync::Mutex::new(None),
        location: std::sync::Mutex::new(None),
    });
    let seen = cap.clone();
    match tokio::spawn(ORACLE_CAPTURE.scope(cap, fut)).await {
        Ok(value) => CaptureOutcome::Ok(value),
        Err(e) if e.is_panic() => {
            let payload = e.into_panic();
            if seen.is_oracle.load(Ordering::Relaxed) {
                let msg = seen
                    .message
                    .lock()
                    .ok()
                    .and_then(|mut g| g.take())
                    .unwrap_or_else(|| panic_payload_message(payload.as_ref()));
                CaptureOutcome::OracleFailure(msg)
            } else {
                let location = seen.location.lock().ok().and_then(|mut g| g.take());
                CaptureOutcome::OtherPanic { payload, location }
            }
        }
        Err(e) => panic!("oracle task aborted unexpectedly: {e}"),
    }
}

pub async fn probe_oracle<T, F>(fut: F) -> ProbeResult<T>
where
    T: Send + 'static,
    F: std::future::Future<Output = T> + Send + 'static,
{
    match run_captured(fut, true).await {
        CaptureOutcome::Ok(value) => ProbeResult::Ok(value),
        CaptureOutcome::OracleFailure(msg) => ProbeResult::OracleFailure(msg),
        CaptureOutcome::OtherPanic { payload, location } => {
            ProbeResult::OtherPanic { message: panic_payload_message(payload.as_ref()), location }
        }
    }
}

#[cfg(feature = "test-sentinels")]
pub async fn caught_by_oracle<F>(fut: F) -> bool
where
    F: std::future::Future + Send + 'static,
{
    match run_captured(
        async move {
            let _ = fut.await;
        },
        false,
    )
    .await
    {
        CaptureOutcome::Ok(()) => false,
        CaptureOutcome::OracleFailure(_) => true,
        CaptureOutcome::OtherPanic { payload, .. } => std::panic::resume_unwind(payload),
    }
}

pub fn wake_lines(sent: &[Value], known: Known) -> Vec<WakeLine> {
    let mut out = Vec::new();
    for s in sent {
        let to = s["threadId"].as_str().unwrap_or_default();
        let mut rest = s["message"].as_str().unwrap_or_default();
        loop {
            let (line, after) = rest.split_once('\n').unwrap_or((rest, ""));
            let Some((kind, from, handle)) = header_line(line) else { break };
            let r = known(&handle)
                .unwrap_or_else(|| oracle_fail(format!("{to} got a wake header for unknown message {handle}")));
            if (r.to.as_str(), r.kind.as_str()) != (to, kind.as_str()) {
                oracle_fail(format!(
                    "message {handle} was delivered to the wrong recipient or as the wrong kind: expected ({:?}, {:?}), got ({to:?}, {kind:?})",
                    r.to, r.kind
                ));
            }
            let used = body_len(after, &r.body, &handle)
                .unwrap_or_else(|| oracle_fail(format!("{to} got {handle} with a body that differs from its record")));
            let subject = after.lines().next().unwrap_or_default().to_string();
            out.push(WakeLine { to: to.into(), kind, from, subject });
            match after[used..].strip_prefix("\n\n") {
                Some(next) if next.starts_with("[t3cm ") => rest = next,
                _ => break,
            }
        }
    }
    out
}

pub fn delivered_headers(sent: &[Value], known: Known) -> Vec<(String, String, String)> {
    wake_lines(sent, known).into_iter().map(|w| (w.to, w.kind, w.from)).collect()
}

impl H {
    fn message_record(&self, handle: &str) -> Option<Record> {
        self.app.store.msg_get(handle).unwrap().map(|m| Record { to: m.to, kind: m.kind, body: m.body })
    }

    pub fn delivered(&self, sent: &[Value]) -> Vec<(String, String, String)> {
        let known = |handle: &str| self.message_record(handle);
        delivered_headers(sent, &known)
    }

    pub fn wake_lines(&self, sent: &[Value]) -> Vec<WakeLine> {
        let known = |handle: &str| self.message_record(handle);
        wake_lines(sent, &known)
    }

    pub fn snapshot(&self) -> Snapshot {
        let cards: Vec<CardView> = self
            .app
            .wf
            .cards()
            .unwrap()
            .into_iter()
            .map(|c| CardView {
                kind: c.kind,
                ticket: c.ticket,
                worker: c.worker.map(|w| (w.thread, w.gate)),
                pr: c.pr.as_ref().map(|p| (p.repo.clone(), p.number)),
                head: c.pr.as_ref().and_then(|p| p.head.clone()),
                ready: c.pr.as_ref().is_some_and(|p| p.ready),
            })
            .collect();
        let links = self
            .app
            .store
            .links_all()
            .unwrap()
            .into_iter()
            .map(|l| {
                let view = LinkView {
                    parent: l.parent,
                    closed: l.closed,
                    paused: l.paused_at.is_some(),
                    gate: l.gate,
                    gate_pr: l.gate_pr,
                    pr: l.pr.map(|p| (p.repo, p.number, p.head, p.waiting, p.followups)),
                };
                (l.child, view)
            })
            .collect();
        let merge_waits = self
            .app
            .store
            .merge_wait_rows()
            .unwrap()
            .into_iter()
            .map(|(repo, pr, w)| {
                if (&w.repo, w.pr) != (&repo, pr) {
                    oracle_fail(format!(
                        "merge wait row key ({repo}, {pr}) disagrees with its payload ({}, {})",
                        w.repo, w.pr
                    ));
                }
                ((repo, pr), (format!("{:?}", w.status), w.head, w.thread, w.failures))
            })
            .collect();
        let pr_events = self.app.store.pr_events_all().unwrap().into_iter().collect();
        let pr_seen = self
            .app
            .store
            .pr_seen_all()
            .unwrap()
            .into_iter()
            .map(|r| ((r.repo, r.pr), (r.queued, r.since, r.left, r.queued_at)))
            .collect();
        let mut outbox = BTreeMap::new();
        for item in self.app.store.outbox_all().unwrap() {
            *outbox.entry(item.to).or_insert(0) += 1;
        }
        Snapshot { cards, links, merge_waits, pr_events, pr_seen, outbox, delivered: self.delivered(&self.sent()) }
    }
}
