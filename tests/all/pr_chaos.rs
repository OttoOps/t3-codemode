use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::time::Duration;

use crate::common::*;
use arbitrary::Unstructured;
use futures_util::future::{join, join3};
use serde_json::{Value, json};
use t3_codemode::board::{BOARD, Card, CardRef};
use t3_codemode::clock::{Clock, iso, plus_minutes};
use t3_codemode::fake::{check_run, github_pr, project, pull_request, thread};
use t3_codemode::reach::ReachSnapshot;
use t3_codemode::sentinels::Sentinel;
use t3_codemode::store::{MergeWait, WaitStatus};
use t3_codemode::wf::LAST_PASS;

mod ops;
mod shadow;
mod shrink;

use ops::{
    CardKind, CardSym, GhChecks, GhEndState, GhMergeState, Op, OpKind, OpStats, OpWeights, PauseTarget, PrEventKind,
    PrSym, QueueDropReason, SweepTarget, TSym, WSym, decode_ops, deserialize_ops, seed_bytes, serialize_ops,
};
use shadow::{AdvanceRace, Model, Predicted, St, WaitM, wakes_of};
use shrink::{
    FailureClass, FailureKind, ShrinkOutcome, corpus_dir_from_env, ddmin, default_corpus_dir, display_source_path,
    env_flag_enabled, failure_summary, format_op_rust, format_ops_file, format_regression_test, format_shrink_report,
    list_corpus_files, load_ops_file, normalize_failure_signature, ops_content_hash, parse_env_flag, parse_env_path,
    shrink_ops, write_corpus_file,
};

fn same_head(a: &str, b: &str) -> bool {
    !a.is_empty() && !b.is_empty() && (a.starts_with(b) || b.starts_with(a))
}

const REPO: &str = "acme/widgets";
const HEADS: &[&str] = &[
    "1111111111111111111111111111111111111111",
    "2222222222222222222222222222222222222222",
    "3333333333333333333333333333333333333333",
    "4444444444444444444444444444444444444444",
];
const GOOD_BODY: &str = "## Summary\n\n```text\nwf.pr\n  chaos\n```\n\n## Evidence\n\n- **Before:** old\n  **After:** new\n\n\
## Merge Danger\n\n**Door:** two-way\n\n**Blast Radius:** board\n\nRefs #41\n";
const PUBLISHING: &str = "Publishing: pr skill; passes: none configured";
const TICKETS: &[&str] = &["mcp:t1", "mcp:t2", "mcp:t3"];
const WORKERS: &[&str] = &["mcp:w1", "mcp:w2", "mcp:w3"];
const PRS: &[u64] = &[7, 8, 9];

struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed.wrapping_add(0x9e3779b97f4a7c15))
    }

    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e3779b97f4a7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
        z ^ (z >> 31)
    }

    fn pick(&mut self, n: usize) -> usize {
        (self.next() as usize) % n
    }

    fn coin(&mut self) -> bool {
        (self.next() & 1) == 0
    }
}

#[derive(Clone, Debug)]
struct GhPrModel {
    head_idx: usize,
    state: &'static str,
    draft: bool,
    merge_state: &'static str,
    checks: GhChecks,
    exists: bool,
    read_err: bool,
    many_files: bool,
    queue_enabled: bool,
    queued: bool,
    removed: Option<(&'static str, String)>,
    ended_at: Option<String>,
    merge_commit: Option<String>,
}

impl Default for GhPrModel {
    fn default() -> Self {
        Self {
            head_idx: 0,
            state: "OPEN",
            draft: false,
            merge_state: "CLEAN",
            checks: GhChecks::Passing,
            exists: true,
            read_err: false,
            many_files: false,
            queue_enabled: false,
            queued: false,
            removed: None,
            ended_at: None,
            merge_commit: None,
        }
    }
}

fn clock_now(h: &H) -> String {
    iso(h.clock.now())
}

fn before_tracking(t0: jiff::Timestamp) -> String {
    iso(plus_minutes(t0, -10.0))
}

fn advance_mutations(h: &H) -> (usize, usize) {
    let mut merges = 0;
    let mut enqueues = 0;
    for op in h.github.operations() {
        match op.as_str() {
            "mergePullRequest" => merges += 1,
            "enqueuePullRequest" => enqueues += 1,
            _ => {}
        }
    }
    (merges, enqueues)
}

fn pull_gh_mutations(h: &H, prs: &mut BTreeMap<u64, GhPrModel>) {
    for (&number, m) in prs.iter_mut() {
        if !m.exists {
            continue;
        }
        let cur = h.github.pr(REPO, number);
        if cur["state"] == "MERGED" && m.state != "MERGED" {
            m.state = "MERGED";
            m.queued = false;
            m.merge_commit = cur["mergeCommit"]["oid"].as_str().map(String::from);
            m.ended_at = cur["mergedAt"].as_str().map(String::from);
        } else if cur["isInMergeQueue"] == true && !m.queued {
            m.queued = true;
        }
    }
}

fn pending_check(name: &str) -> Value {
    json!({"__typename": "CheckRun", "name": name, "status": "IN_PROGRESS", "conclusion": null})
}

fn setup_chaos(in_memory: bool) -> (H, BTreeMap<u64, GhPrModel>) {
    let h = if in_memory { H::new_memory() } else { H::new() };
    let home = h.dir.path().join("home");
    let repo = h.dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    for skill in ["deploy", "verify", "publish"] {
        let d = home.join(".claude/skills").join(skill);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("SKILL.md"), format!("---\nname: {skill}\ndescription: test\n---\n\nBody.\n")).unwrap();
    }
    let cfg = json!({"delivery": {"steps": [
        {"id": "merge", "kind": "builtin", "gate": "always", "scope": "pr"},
        {"id": "deploy", "skill": "deploy", "gate": "chain", "scope": "pr"},
        {"id": "publish", "skill": "publish", "gate": "chain", "scope": "ticket"}
    ]}});
    let p = home.join(".config/t3-workflow/config.json");
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, cfg.to_string()).unwrap();
    h.fake.set_projects(vec![project("proj:widgets", &repo.to_string_lossy())]);

    for (i, &t) in TICKETS.iter().enumerate() {
        let mut th = thread(t, "idle", T0);
        th["projectId"] = json!("proj:widgets");
        th["link"] = json!(format!("https://t3.local/ticket-{i}"));
        h.fake.put(th);
    }
    for (i, &w) in WORKERS.iter().enumerate() {
        let mut th = thread(w, "idle", T0);
        th["projectId"] = json!("proj:widgets");
        th["link"] = json!(format!("https://t3.local/worker-{i}"));
        h.fake.put(th);
    }
    let mut prs = BTreeMap::new();
    for &n in PRS {
        prs.insert(n, GhPrModel::default());
    }
    sync_gh(&h, &prs);
    (h, prs)
}

fn sync_gh(h: &H, prs: &BTreeMap<u64, GhPrModel>) {
    h.github.clear_read_errors();
    for (&number, m) in prs {
        if !m.exists {
            h.github.remove_pr(REPO, number);
            continue;
        }
        let mut pr = github_pr(HEADS[m.head_idx], m.merge_state, m.queue_enabled);
        pr["state"] = json!(m.state);
        pr["isDraft"] = json!(m.draft);
        pr["isInMergeQueue"] = json!(m.queued);
        if m.queued {
            pr["mergeQueueEntry"] = json!({"position": 1});
        }
        pr["body"] = json!(GOOD_BODY);
        pr["files"] = json!({"pageInfo": {"hasNextPage": m.many_files}, "nodes": [{"path": "src/lib.rs"}]});
        if let Some((reason, at)) = &m.removed {
            pr["timelineItems"] = json!({"nodes": [{"reason": reason, "createdAt": at}]});
        }
        if m.state == "MERGED" {
            let oid = m.merge_commit.as_deref().unwrap_or(HEADS[(m.head_idx + 1) % HEADS.len()]);
            pr["mergeCommit"] = json!({"oid": oid});
            if let Some(at) = &m.ended_at {
                pr["mergedAt"] = json!(at);
            }
        } else if m.state == "CLOSED"
            && let Some(at) = &m.ended_at
        {
            pr["closedAt"] = json!(at);
        }
        h.github.set_pr(REPO, number, pr);
        let checks = match m.checks {
            GhChecks::Pending => json!([pending_check("ci")]),
            GhChecks::Passing => json!([check_run("ci", "SUCCESS")]),
            GhChecks::Failing => json!([check_run("ci", "FAILURE")]),
            GhChecks::Cancelled => json!([check_run("ci", "SUCCESS"), check_run("opt", "CANCELLED")]),
            GhChecks::Unread => json!([]),
        };
        h.github.set_checks(REPO, number, checks);
        if m.read_err {
            h.github.fail_read(REPO, number, "transient chaos read error");
        }
    }
}

fn sent_counts(h: &H) -> BTreeMap<String, usize> {
    let mut m = BTreeMap::new();
    for s in h.sent() {
        let to = s["threadId"].as_str().unwrap_or_default().to_string();
        *m.entry(to).or_insert(0) += 1;
    }
    m
}

fn delivered_counts(h: &H) -> BTreeMap<String, usize> {
    let mut m = BTreeMap::new();
    for (to, ..) in h.delivered(&h.sent()) {
        *m.entry(to).or_insert(0) += 1;
    }
    m
}

macro_rules! oracle_assert {
    ($cond:expr, $($arg:tt)+) => {
        if !$cond {
            oracle_fail(format!($($arg)+));
        }
    };
}

macro_rules! oracle_assert_eq {
    ($left:expr, $right:expr, $($arg:tt)+) => {
        match (&$left, &$right) {
            (left, right) if *left != *right => {
                oracle_fail(format!(
                    "assertion `left == right` failed: {}\n  left: {left:?}\n right: {right:?}",
                    format_args!($($arg)+)
                ));
            }
            _ => {}
        }
    };
}

#[allow(clippy::too_many_arguments)]
fn assert_invariants(
    h: &H,
    seed: impl fmt::Display,
    trace: &[String],
    closed_tickets: &BTreeMap<String, usize>,
    closed_workers: &BTreeMap<String, usize>,
    after_healthy_reconcile: bool,
    gh: &BTreeMap<u64, GhPrModel>,
) {
    let ctx = || format!("seed={seed}\ntrace:\n{}", trace.join("\n"));
    let cards = h.app.wf.cards().unwrap();

    // 1. Card metadata, uniqueness, and worker ownership invariants
    let mut ready_seen: BTreeSet<(String, String, u64)> = BTreeSet::new();
    for c in &cards {
        if let Some(ticket) = c.ticket.as_deref() {
            let tlink = h.app.store.link_get(ticket).unwrap();
            if c.worker.is_none() {
                if let Some(tl) = &tlink {
                    oracle_assert_eq!(
                        c.label.as_deref(),
                        Some(tl.label.as_str()),
                        "ticket-owned card label drifted from owner: card={c:?} link={tl:?}\n{}",
                        ctx()
                    );
                    oracle_assert_eq!(
                        c.link.as_deref(),
                        tl.link.as_deref(),
                        "ticket-owned card link drifted from owner: card={c:?} link={tl:?}\n{}",
                        ctx()
                    );
                }
                if c.kind == "gate"
                    && let Some(p) = c.pr.as_ref().filter(|p| p.ready)
                {
                    let key = (ticket.to_string(), p.repo.clone(), p.number);
                    oracle_assert!(
                        ready_seen.insert(key.clone()),
                        "duplicate open ready merge card for {key:?}: {cards:?}\n{}",
                        ctx()
                    );
                }
            } else if let Some(w) = &c.worker {
                let wl = h.app.store.link_get(&w.thread).unwrap().expect("relayed worker link");
                oracle_assert!(
                    !wl.closed,
                    "open relayed card points to closed worker: card={c:?} wl={wl:?}\n{}",
                    ctx()
                );
                oracle_assert_eq!(
                    wl.parent,
                    ticket,
                    "relayed card ticket does not match worker parent: card={c:?} wl={wl:?}\n{}",
                    ctx()
                );
            }
        }
    }

    // 2. Ticket gate invariants
    for &t in TICKETS {
        let Some(tl) = h.app.store.link_get(t).unwrap() else { continue };
        if tl.closed {
            continue;
        }
        let has_ready = cards.iter().any(|c| {
            c.kind == "gate"
                && c.worker.is_none()
                && c.ticket.as_deref() == Some(t)
                && c.pr.as_ref().is_some_and(|p| p.ready)
        });
        let has_relayed = cards.iter().any(|c| c.worker.is_some() && c.ticket.as_deref() == Some(t));
        if has_ready {
            oracle_assert!(
                tl.gate.is_some(),
                "ticket {t} has open ready merge card but gate is None: cards={cards:?}\n{}",
                ctx()
            );
        }
        if has_relayed {
            oracle_assert!(
                tl.gate.is_some(),
                "ticket {t} has open relayed card but gate is None: cards={cards:?}\n{}",
                ctx()
            );
        }
        if tl.gate_pr.is_some() {
            oracle_assert_eq!(
                tl.gate.as_deref(),
                Some("merge"),
                "ticket {t} has gate_pr={:?} while gate={:?}\n{}",
                tl.gate_pr,
                tl.gate,
                ctx()
            );
        }
    }

    // 3. Closed ticket and worker link invariants (no closed-link wakes, no stuck waits after healthy reconcile)
    let counts = delivered_counts(h);
    for (t, &count_at_close) in closed_tickets {
        let tl = h.app.store.link_get(t).unwrap().unwrap();
        oracle_assert!(tl.closed, "ticket {t} expected closed\n{}", ctx());
        oracle_assert!(tl.gate.is_none(), "closed ticket {t} kept gate {:?}\n{}", tl.gate, ctx());
        oracle_assert!(tl.gate_pr.is_none(), "closed ticket {t} kept gate_pr {:?}\n{}", tl.gate_pr, ctx());
        let open_for_closed: Vec<&Card> = cards.iter().filter(|c| c.ticket.as_deref() == Some(t.as_str())).collect();
        oracle_assert!(
            open_for_closed.is_empty(),
            "closed ticket {t} still owns open board cards: {open_for_closed:?}\n{}",
            ctx()
        );
        let now_count = counts.get(t).copied().unwrap_or(0);
        oracle_assert_eq!(
            now_count,
            count_at_close,
            "closed ticket {t} received a message after closing! messages={:?}\n{}",
            h.sent().into_iter().filter(|s| s["threadId"] == t.as_str()).collect::<Vec<_>>(),
            ctx()
        );
    }
    for (w, &count_at_close) in closed_workers {
        let wl = h.app.store.link_get(w).unwrap().unwrap();
        oracle_assert!(wl.closed, "worker {w} expected closed\n{}", ctx());
        oracle_assert!(wl.gate.is_none(), "closed worker {w} kept gate {:?}\n{}", wl.gate, ctx());
        oracle_assert!(wl.gate_pr.is_none(), "closed worker {w} kept gate_pr {:?}\n{}", wl.gate_pr, ctx());
        let now_count = counts.get(w).copied().unwrap_or(0);
        oracle_assert_eq!(
            now_count,
            count_at_close,
            "closed worker {w} received a message after closing! messages={:?}\n{}",
            h.sent().into_iter().filter(|s| s["threadId"] == w.as_str()).collect::<Vec<_>>(),
            ctx()
        );
    }

    if after_healthy_reconcile {
        for &w in WORKERS {
            let Some(wl) = h.app.store.link_get(w).unwrap().filter(|l| !l.closed) else { continue };
            if let Some(p) = wl.pr.as_ref().filter(|p| p.waiting)
                && let Some(m) = gh.get(&p.number)
                && !m.read_err
            {
                let should_keep_waiting = m.exists
                    && m.state == "OPEN"
                    && !m.draft
                    && same_head(HEADS[m.head_idx], &p.head)
                    && (m.checks == GhChecks::Pending
                        || (matches!(m.checks, GhChecks::Passing | GhChecks::Cancelled) && m.merge_state == "UNKNOWN"));
                oracle_assert!(
                    should_keep_waiting,
                    "worker {w} stayed stuck in pr.waiting=true after healthy reconcile: pr={p:?} gh={m:?}\n{}",
                    ctx()
                );
            }
        }
    }
}

fn wait_model(w: &MergeWait, id_ok: bool) -> WaitM {
    WaitM {
        head: w.head.clone(),
        id_ok,
        thread: w.thread.clone().unwrap(),
        at: w.at.clone(),
        status: if w.status == WaitStatus::Sent { St::Sent } else { St::Waiting },
        approval: w.approval.clone(),
        failures: w.failures,
    }
}

fn diff<T: std::fmt::Debug + PartialEq>(what: &str, got: &T, want: &T) -> Option<String> {
    (got != want).then(|| format!("{what} diverged from the shadow model\n  sut:   {got:?}\n  model: {want:?}"))
}

fn github_events(lines: Vec<WakeLine>) -> Vec<(String, u64)> {
    lines
        .into_iter()
        .filter(|w| w.kind == "event" && w.from == "github")
        .map(|w| {
            let n = w
                .subject
                .strip_prefix(&format!("{REPO}#"))
                .and_then(|r| r.split(|c: char| !c.is_ascii_digit()).next())
                .and_then(|d| d.parse().ok())
                .unwrap_or_else(|| oracle_fail(format!("github event wake without a PR subject: {:?}", w.subject)));
            (w.to, n)
        })
        .collect()
}

fn assert_forbidden(h: &H, model: &mut Model, sent_before: usize, ctx: &dyn Fn() -> String) {
    let snap = h.snapshot();
    let sent = &h.sent()[sent_before..];
    let mut allowed_reopen = model.reopen_wakes.clone();
    for (to, kind, from) in h.delivered(sent) {
        if snap.links.get(&to).is_some_and(|l| l.closed) {
            let allowed = if kind == "reply" && from == "parent" {
                allowed_reopen.get_mut(&to).filter(|n| **n > 0).map(|n| *n -= 1).is_some()
            } else {
                false
            };
            oracle_assert!(allowed, "forbidden wake: {kind} from {from} reached closed link {to}\n{}", ctx());
        }
    }
    let forbidden = model.forbidden_github();
    for w in github_events(h.wake_lines(sent)) {
        oracle_assert!(
            !forbidden.contains(&w),
            "forbidden wake: GitHub told {} about #{} after it let go of it\n{}",
            w.0,
            w.1,
            ctx()
        );
    }
}

#[derive(Default)]
struct Oracle {
    loose: Option<Vec<shadow::Wake>>,
}

impl Oracle {
    fn status(&self, r: &Value, want: &str, trace: &[String], seed: impl fmt::Display) {
        oracle_assert_eq!(
            r["status"].as_str(),
            Some(want),
            "wf.pr status diverged from the shadow model: {r}\nseed={seed}\ntrace:\n{}",
            trace.join("\n")
        );
    }

    fn mutations(
        &self,
        predicted: &Predicted,
        actual: &BTreeMap<u64, GhPrModel>,
        ok: bool,
        tried: u64,
        trace: &[String],
        seed: impl fmt::Display,
    ) {
        oracle_assert_eq!(
            predicted.attempts,
            tried,
            "GitHub merge and enqueue attempts in this reconcile diverged from the shadow model\nseed={seed}\ntrace:\n{}",
            trace.join("\n")
        );
        oracle_assert_eq!(
            predicted.ok,
            ok,
            "reconcile success diverged from the shadow model\nseed={seed}\ntrace:\n{}",
            trace.join("\n")
        );
        if let Some(n) = mutation_diff(&predicted.gh, actual) {
            oracle_fail(format!(
                "GitHub mutations on #{n} diverged from the shadow model\nseed={seed}\ntrace:\n{}",
                trace.join("\n")
            ));
        }
    }

    fn check(&mut self, h: &H, model: &mut Model, sent_before: usize, trace: &[String], seed: impl fmt::Display) {
        let ctx = || format!("seed={seed}\ntrace:\n{}", trace.join("\n"));
        let snap = h.snapshot();
        let observed = wakes_of(&h.delivered(&h.sent()[sent_before..]));
        assert_forbidden(h, model, sent_before, &ctx);
        oracle_assert_eq!(
            attempts(h),
            model.attempts(),
            "GitHub merge and enqueue attempts diverged from the shadow model\n{}",
            ctx()
        );
        match self.loose.take() {
            Some(may) => {
                let must = wakes_of(&model.wakes);
                let extra = wakes_of(&may);
                for (w, n) in &observed {
                    let max = must.get(w).copied().unwrap_or(0) + extra.get(w).copied().unwrap_or(0);
                    oracle_assert!(
                        *n <= max,
                        "wake {w:?} x{n} is outside what the shadow model allows (must={must:?}, extra={extra:?})\n{}",
                        ctx()
                    );
                }
                for (w, m) in &must {
                    let seen = observed.get(w).copied().unwrap_or(0);
                    oracle_assert!(seen >= *m, "expected flushed wake {w:?} x{m}, saw x{seen}\n{}", ctx());
                }
            }
            None => {
                if let Some(d) = diff("wakes", &observed, &wakes_of(&model.wakes)) {
                    oracle_fail(format!("{d}\n{}", ctx()));
                }
            }
        }
        let got = shadow::project(&snap);
        let want = model.proj();
        let diffs: Vec<String> = [
            diff("links", &got.links, &want.links),
            diff("cards", &got.cards, &want.cards),
            diff("merge waits", &got.waits, &want.waits),
            diff("pr events", &got.events, &want.events),
            diff("pr seen", &got.seen, &want.seen),
            diff("outbox", &got.outbox, &want.outbox),
        ]
        .into_iter()
        .flatten()
        .collect();
        oracle_assert!(diffs.is_empty(), "{}\n{}", diffs.join("\n"), ctx());
        for (t, l) in snap.links.iter().filter(|(_, l)| !l.closed && l.parent == BOARD) {
            if let Some(p) = &l.gate_pr {
                oracle_assert!(
                    snap.cards.iter().any(|c| c.kind == "gate"
                        && c.worker.is_none()
                        && c.ticket.as_deref() == Some(t.as_str())
                        && c.pr.as_ref() == Some(p)),
                    "ticket {t} holds merge gate_pr {p:?} with no open merge card for it: {:?}\n{}",
                    snap.cards,
                    ctx()
                );
            }
        }
    }
}

#[derive(Clone, Debug)]
enum Raced {
    Ready(&'static str, u64, String),
    Updated(&'static str, u64, String),
    Opened(&'static str, u64, String),
    GateClear(&'static str),
    Adopt(&'static str, &'static str),
    Reconcile,
    Answer(Option<u64>, Option<String>, String, Vec<String>),
}

impl Raced {
    fn apply(&self, m: &mut Model, gh: BTreeMap<u64, GhPrModel>) -> (BTreeMap<u64, GhPrModel>, Option<String>) {
        let status = match self {
            Raced::Ready(w, n, sha) => m.pr_ready(w, *n, sha, &gh),
            Raced::Updated(w, n, sha) => m.pr_updated(w, *n, sha),
            Raced::Opened(w, n, sha) => m.pr_opened(w, *n, sha, false, &gh),
            Raced::Reconcile => return (m.reconcile(&gh).gh, None),
            Raced::GateClear(t) => {
                m.ticket_gate_clear(t);
                return (gh, None);
            }
            Raced::Adopt(to, from) => {
                m.adopt(to, from);
                return (gh, None);
            }
            Raced::Answer(seq, ticket, text, options) => {
                m.answer(*seq, ticket.as_deref(), text, options);
                return (gh, None);
            }
        };
        (gh, Some(status))
    }

    fn worker(&self) -> Option<&'static str> {
        match self {
            Raced::Ready(w, ..) | Raced::Updated(w, ..) | Raced::Opened(w, ..) => Some(w),
            _ => None,
        }
    }

    fn can_supersede(&self, w: &str) -> bool {
        matches!(self, Raced::Reconcile | Raced::Adopt(..)) || self.worker() == Some(w)
    }
}

fn attempts(h: &H) -> u64 {
    let (merges, enqueues) = advance_mutations(h);
    (merges + enqueues) as u64
}

fn mutation_diff(predicted: &BTreeMap<u64, GhPrModel>, actual: &BTreeMap<u64, GhPrModel>) -> Option<u64> {
    actual.iter().find_map(|(n, m)| {
        let p = &predicted[n];
        ((p.state, p.queued, &p.merge_commit) != (m.state, m.queued, &m.merge_commit)).then_some(*n)
    })
}

fn orders(n: usize) -> Vec<Vec<usize>> {
    if n == 0 {
        return vec![Vec::new()];
    }
    let mut out = Vec::new();
    for rest in orders(n - 1) {
        for pos in 0..=rest.len() {
            let mut o = rest.clone();
            o.insert(pos, n - 1);
            out.push(o);
        }
    }
    out
}

type Candidate = (Vec<usize>, Model, BTreeMap<u64, GhPrModel>, BTreeSet<(String, u64)>, BTreeSet<String>);

fn boundary(m: &Model) -> (BTreeSet<(String, u64)>, BTreeSet<String>) {
    let closed = m.links.iter().filter(|(_, l)| !l.open).map(|(id, _)| id.clone()).collect();
    (m.clone().forbidden_github(), closed)
}

#[allow(clippy::too_many_arguments)]
fn resolve_raced(
    h: &H,
    model: &mut Model,
    ops: &[Raced],
    statuses: &[Option<String>],
    gh: &BTreeMap<u64, GhPrModel>,
    actual_gh: &BTreeMap<u64, GhPrModel>,
    sent_before: usize,
    trace: &[String],
    seed: u64,
) {
    let ctx = || format!("seed={seed}\ntrace:\n{}", trace.join("\n"));
    let snap = h.snapshot();
    let got = shadow::project(&snap);
    let observed = wakes_of(&h.delivered(&h.sent()[sent_before..]));
    let (_, closed_before) = boundary(model);
    for (to, kind, from) in observed.keys() {
        oracle_assert!(
            !closed_before.contains(to),
            "forbidden wake: {kind} from {from} reached closed link {to}\n{}",
            ctx()
        );
    }
    let maybe: Vec<usize> = (0..ops.len())
        .filter(|&i| statuses[i].as_deref() == Some("superseded"))
        .filter(|&i| ops[i].worker().is_some_and(|w| ops.iter().enumerate().any(|(j, o)| j != i && o.can_supersede(w))))
        .collect();
    let mut candidates: Vec<Candidate> = Vec::new();
    for order in orders(ops.len()) {
        for mask in 0..(1usize << maybe.len()) {
            let skipped: Vec<usize> =
                maybe.iter().enumerate().filter(|(b, _)| mask & (1 << b) != 0).map(|(_, &i)| i).collect();
            let order: Vec<usize> = order.iter().copied().filter(|i| !skipped.contains(i)).collect();
            if candidates.iter().any(|(o, ..)| *o == order) {
                continue;
            }
            let mut m = model.clone();
            let mut g = gh.clone();
            let mut agrees = true;
            let (mut forbid, mut closed) = boundary(&m);
            for &i in &order {
                let (next, status) = ops[i].apply(&mut m, g);
                g = next;
                agrees &= status.is_none() || status == statuses[i];
                let (f, c) = boundary(&m);
                forbid.retain(|w| f.contains(w));
                closed.retain(|l| c.contains(l));
            }
            if agrees {
                candidates.push((order, m, g, forbid, closed));
            }
        }
    }
    oracle_assert!(
        !candidates.is_empty(),
        "raced round {ops:?} returned statuses {statuses:?} that no serial order predicts\n{}",
        ctx()
    );
    let counts: Vec<BTreeMap<shadow::Wake, usize>> = candidates.iter().map(|(_, m, ..)| wakes_of(&m.wakes)).collect();
    let tried = attempts(h);
    for (w, n) in &observed {
        let may = counts.iter().map(|c| c.get(w).copied().unwrap_or(0)).max().unwrap_or(0);
        oracle_assert!(*n <= may, "raced wake {w:?} x{n} exceeds every serial order of {ops:?} (max {may})\n{}", ctx());
    }
    for c in &counts {
        for (w, n) in c {
            let must = counts.iter().map(|o| o.get(w).copied().unwrap_or(0)).min().unwrap_or(0);
            let seen = observed.get(w).copied().unwrap_or(0);
            oracle_assert!(
                seen >= must,
                "raced wake {w:?} is owed x{must} by every serial order of {ops:?}, saw x{seen} (of {n})\n{}",
                ctx()
            );
        }
    }
    let hit = candidates.iter().zip(&counts).find(|((_, m, g, ..), c)| {
        m.proj() == got && **c == observed && mutation_diff(g, actual_gh).is_none() && m.attempts() == tried
    });
    match hit {
        Some(((_, m, _, forbid, closed), _)) => {
            *model = m.clone();
            model.forbidden_github();
            let sent = &h.sent()[sent_before..];
            for (to, kind, from) in h.delivered(sent) {
                oracle_assert!(
                    !closed.contains(&to),
                    "forbidden wake: {kind} from {from} reached {to}, closed throughout the round\n{}",
                    ctx()
                );
            }
            for w in github_events(h.wake_lines(sent)) {
                oracle_assert!(
                    !forbid.contains(&w),
                    "forbidden wake: GitHub told {} about #{}, which it let go of before the round and never regained\n{}",
                    w.0,
                    w.1,
                    ctx()
                );
            }
        }
        None => {
            let misses: Vec<String> = candidates
                .iter()
                .map(|(order, m, g, ..)| {
                    let want = m.proj();
                    let parts: Vec<String> = [
                        diff("links", &got.links, &want.links),
                        diff("cards", &got.cards, &want.cards),
                        diff("merge waits", &got.waits, &want.waits),
                        diff("pr events", &got.events, &want.events),
                        diff("pr seen", &got.seen, &want.seen),
                        diff("wakes", &observed, &wakes_of(&m.wakes)),
                        mutation_diff(g, actual_gh).map(|n| format!("GitHub mutations on #{n} differ")),
                        (m.attempts() != tried)
                            .then(|| format!("GitHub attempts: SUT {tried}, model {}", m.attempts())),
                    ]
                    .into_iter()
                    .flatten()
                    .collect();
                    format!("order {order:?}:\n{}", parts.join("\n"))
                })
                .collect();
            oracle_fail(format!("raced round {ops:?} matches no serial order\n{}\n{}", misses.join("\n"), ctx()));
        }
    }
}

const DEFAULT_SEQ_SEEDS: u64 = 60;
const DEFAULT_CONCURRENT_SEEDS: u64 = 30;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SeedRange {
    start: u64,
    count: u64,
}

impl SeedRange {
    fn seeds(self) -> impl Iterator<Item = u64> {
        (0..self.count).map(move |offset| self.start + offset)
    }

    fn describe(self) -> String {
        if self.count == 0 {
            return "0 seeds".into();
        }
        let end = self.start + (self.count - 1);
        if self.count == 1 {
            format!("seed {}", self.start)
        } else {
            format!("seeds {}..={} ({} seeds)", self.start, end, self.count)
        }
    }
}

fn parse_env_u64(name: &str, raw: Option<&str>) -> Result<Option<u64>, String> {
    let Some(raw) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    raw.parse::<u64>().map(Some).map_err(|e| format!("invalid {name}={raw:?}: {e}"))
}

fn parse_seed_range(
    default: u64,
    single: Option<&str>,
    start: Option<&str>,
    count: Option<&str>,
) -> Result<SeedRange, String> {
    if let Some(seed) = parse_env_u64("CHAOS_SEED", single)? {
        return Ok(SeedRange { start: seed, count: 1 });
    }
    let start = parse_env_u64("CHAOS_SEED_START", start)?.unwrap_or(0);
    let count = parse_env_u64("CHAOS_SEEDS", count)?.unwrap_or(default);
    if count > 0 && start.checked_add(count - 1).is_none() {
        return Err(format!("CHAOS_SEED_START={start} + CHAOS_SEEDS={count} overflows u64"));
    }
    Ok(SeedRange { start, count })
}

fn chaos_seed_range(default: u64) -> SeedRange {
    let single = std::env::var("CHAOS_SEED").ok();
    let start = std::env::var("CHAOS_SEED_START").ok();
    let count = std::env::var("CHAOS_SEEDS").ok();
    parse_seed_range(default, single.as_deref(), start.as_deref(), count.as_deref()).unwrap_or_else(|e| panic!("{e}"))
}

fn has_explicit_seed_env() -> bool {
    ["CHAOS_SEED", "CHAOS_SEED_START", "CHAOS_SEEDS", "CHAOS_OPS", "CHAOS_SENTINEL"]
        .iter()
        .any(|k| std::env::var(k).ok().is_some_and(|v| !v.trim().is_empty()))
}

#[cfg(feature = "test-sentinels")]
fn parse_chaos_sentinel(raw: Option<&str>) -> Result<Option<Sentinel>, String> {
    let Some(name) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    Sentinel::ALL.iter().copied().find(|s| s.name() == name).map(Some).ok_or_else(|| {
        let valid: Vec<&str> = Sentinel::ALL.iter().map(|s| s.name()).collect();
        format!("unknown CHAOS_SENTINEL={name:?}; expected one of: {}", valid.join(", "))
    })
}

fn chaos_sentinel_from_env() -> Option<Sentinel> {
    #[cfg(feature = "test-sentinels")]
    {
        let raw = std::env::var("CHAOS_SENTINEL").ok();
        parse_chaos_sentinel(raw.as_deref()).unwrap_or_else(|e| panic!("{e}"))
    }
    #[cfg(not(feature = "test-sentinels"))]
    {
        None
    }
}

fn chaos_ops_from_env() -> Option<std::path::PathBuf> {
    parse_env_path(std::env::var("CHAOS_OPS").ok().as_deref())
}

fn chaos_shrink_enabled() -> bool {
    env_flag_enabled("CHAOS_SHRINK")
}

fn enable_sentinel(h: &H, sentinel: Option<Sentinel>) {
    #[cfg(feature = "test-sentinels")]
    if let Some(s) = sentinel {
        h.app.wf.sentinels().set(s);
    }
    #[cfg(not(feature = "test-sentinels"))]
    let _ = (h, sentinel);
}

const INIT_WORKER_OWNERS: &[usize] = &[0, 0, 1];
const INIT_PR_QUEUE_ENABLED: &[bool] = &[false, true, false];
const INIT_PR_CHECKS: &[GhChecks] = &[GhChecks::Passing, GhChecks::Passing, GhChecks::Pending];

fn resolve_ticket(sym: TSym) -> Option<&'static str> {
    TICKETS.get(sym.0).copied()
}

fn resolve_worker(sym: WSym, closed_workers: &BTreeMap<String, usize>) -> Option<(&'static str, u64)> {
    let &w = WORKERS.get(sym.0)?;
    if closed_workers.contains_key(w) {
        return None;
    }
    Some((w, PRS[sym.0]))
}

fn resolve_pr(sym: PrSym) -> Option<u64> {
    PRS.get(sym.0).copied()
}

fn resolve_card(cards: &[Card], sym: CardSym) -> Option<(usize, &Card)> {
    cards.iter().enumerate().filter(|(_, c)| sym.kind.matches(c)).nth(sym.nth)
}

fn decode_seed_ops(seed: u64) -> Vec<Op> {
    let weights = OpWeights::swarm(seed);
    let bytes = seed_bytes(seed);
    let mut u = Unstructured::new(&bytes);
    decode_ops(&mut u, &weights)
}

async fn run_sequential_seed(seed: u64) -> (Vec<String>, ReachSnapshot, OpStats) {
    run_sequential_seed_with(seed, None).await
}

async fn run_sequential_seed_with(seed: u64, sentinel: Option<Sentinel>) -> (Vec<String>, ReachSnapshot, OpStats) {
    // Sequential chaos has no reopen or peer ops, so it runs on the in-memory store.
    run_sequential_seed_inner(seed, sentinel, true).await
}

async fn run_sequential_seed_inner(
    seed: u64,
    sentinel: Option<Sentinel>,
    in_memory: bool,
) -> (Vec<String>, ReachSnapshot, OpStats) {
    let ops = decode_seed_ops(seed);
    run_sequential_ops(&ops, sentinel, in_memory, &seed.to_string()).await
}

#[allow(clippy::too_many_arguments)]
async fn run_tick(
    h: &H,
    model: &mut Model,
    oracle: &mut Oracle,
    closed_workers: &BTreeMap<String, usize>,
    minutes: u16,
    trace: &[String],
    run_id: &str,
) {
    let waiting_before: BTreeMap<String, usize> = WORKERS
        .iter()
        .copied()
        .filter(|w| !closed_workers.contains_key(*w))
        .filter_map(|w| {
            let wl = h.app.store.link_get(w).unwrap()?;
            wl.pr.as_ref().filter(|p| p.waiting).map(|_| {
                let nudges = h
                    .sent()
                    .into_iter()
                    .filter(|s| {
                        s["threadId"] == w && s["message"].as_str().is_some_and(|m| m.starts_with("[t3cm nudge"))
                    })
                    .count();
                (w.to_string(), nudges)
            })
        })
        .collect();
    let paused_before: BTreeMap<String, usize> = TICKETS
        .iter()
        .chain(WORKERS.iter())
        .copied()
        .filter(|id| model.is_paused(id))
        .map(|id| {
            let nudges = h
                .sent()
                .into_iter()
                .filter(|s| s["threadId"] == id && s["message"].as_str().is_some_and(|m| m.starts_with("[t3cm nudge")))
                .count();
            (id.to_string(), nudges)
        })
        .collect();
    h.advance_tick(i64::from(minutes)).await;
    model.step(&clock_now(h));
    model.tick();
    oracle.loose = Some(model.tick_may());
    for (w, before) in waiting_before {
        let after = h
            .sent()
            .into_iter()
            .filter(|s| {
                s["threadId"] == w.as_str() && s["message"].as_str().is_some_and(|m| m.starts_with("[t3cm nudge"))
            })
            .count();
        oracle_assert_eq!(
            after,
            before,
            "waiting worker {w} was nudged during advance_tick({minutes})\nseed={run_id}\n{}",
            trace.join("\n")
        );
    }
    for (id, before) in paused_before {
        let after = h
            .sent()
            .into_iter()
            .filter(|s| {
                s["threadId"] == id.as_str() && s["message"].as_str().is_some_and(|m| m.starts_with("[t3cm nudge"))
            })
            .count();
        oracle_assert_eq!(
            after,
            before,
            "paused link {id} was nudged during advance_tick({minutes})\nseed={run_id}\n{}",
            trace.join("\n")
        );
    }
}

fn live_tickets_count(model: &Model) -> usize {
    TICKETS.iter().filter(|&&t| model.is_open(t) && !model.is_archived_or_gone(t)).count()
}

async fn run_sequential_ops(
    ops: &[Op],
    sentinel: Option<Sentinel>,
    in_memory: bool,
    run_id: &str,
) -> (Vec<String>, ReachSnapshot, OpStats) {
    let (h, mut gh) = setup_chaos(in_memory);
    enable_sentinel(&h, sentinel);
    for (idx, &pr) in PRS.iter().enumerate() {
        let m = gh.get_mut(&pr).unwrap();
        m.queue_enabled = INIT_PR_QUEUE_ENABLED[idx];
        m.checks = INIT_PR_CHECKS[idx];
    }
    sync_gh(&h, &gh);
    let t0 = h.clock.now();
    let mut trace: Vec<String> = Vec::new();
    let mut closed_tickets: BTreeMap<String, usize> = BTreeMap::new();
    let mut closed_workers: BTreeMap<String, usize> = BTreeMap::new();
    let mut busy_workers: BTreeSet<&'static str> = BTreeSet::new();
    let mut op_stats = OpStats::default();
    let mut model = Model::new(&clock_now(&h));
    h.app.store.meta_set(LAST_PASS, &clock_now(&h)).unwrap();
    model.set_watcher_alive(true);

    for (i, &t) in TICKETS.iter().enumerate() {
        let label = format!("widgets#{}", 41 + i);
        h.wf(t, "join", json!({"parent": BOARD, "label": label, "role": "ticket"})).await;
        model.join_ticket(t, &label);
    }
    for (i, &w) in WORKERS.iter().enumerate() {
        let owner = TICKETS[INIT_WORKER_OWNERS[i]];
        let part = format!("{}/3", i + 1);
        let label = format!("#41 PR {part}");
        h.wf(
            owner,
            "openChild",
            json!({"role": "worker", "label": label, "issue": "acme/widgets#41", "part": part,
                   "pr": {"format": "pr"}}),
        )
        .await;
        h.wf(w, "join", json!({"parent": owner, "label": label, "role": "worker"})).await;
        model.join_worker(w, owner, &label);
        trace.push(format!("init {w} under {owner} -> PR #{}", PRS[i]));
    }

    let mut oracle = Oracle::default();
    for (step, op) in ops.iter().enumerate() {
        h.clock.advance_minutes(1);
        h.app.store.meta_set(LAST_PASS, &clock_now(&h)).unwrap();
        let mut healthy_reconcile = false;
        let sent_before = h.sent().len();
        let before = h.snapshot();
        model.step(&clock_now(&h));
        let res = tokio::time::timeout(Duration::from_secs(5), async {
            match *op {
                Op::PrOpened { worker, short_sha, no_ci } => {
                    let Some((w, pr)) = resolve_worker(worker, &closed_workers) else {
                        trace.push(format!("#{step} noop (pr opened {worker:?})"));
                        return false;
                    };
                    let head = HEADS[gh[&pr].head_idx];
                    let sha = if short_sha { &head[..7] } else { head };
                    let checks = if no_ci { "none" } else { "pending" };
                    let r = h
                        .wf(
                            w,
                            "pr",
                            json!({"event": "opened", "repo": REPO, "pr": pr, "head": sha,
                                   "publishing": PUBLISHING, "checks": checks}),
                        )
                        .await;
                    trace.push(format!("#{step} {w} pr opened #{pr}@{sha} checks={checks} -> {}", r["status"]));
                    let want = model.pr_opened(w, pr, sha, no_ci, &gh);
                    oracle.status(&r, &want, &trace, run_id);
                    true
                }
                Op::PrUpdated { worker, short_sha } => {
                    let Some((w, pr)) = resolve_worker(worker, &closed_workers) else {
                        trace.push(format!("#{step} noop (pr updated {worker:?})"));
                        return false;
                    };
                    let head = HEADS[gh[&pr].head_idx];
                    let sha = if short_sha { &head[..7] } else { head };
                    let r = h.wf(w, "pr", json!({"event": "updated", "repo": REPO, "pr": pr, "head": sha})).await;
                    trace.push(format!("#{step} {w} pr updated #{pr}@{sha} -> {}", r["status"]));
                    let want = model.pr_updated(w, pr, sha);
                    oracle.status(&r, &want, &trace, run_id);
                    true
                }
                Op::PrReady { worker, short_sha } => {
                    let Some((w, pr)) = resolve_worker(worker, &closed_workers) else {
                        trace.push(format!("#{step} noop (pr ready {worker:?})"));
                        return false;
                    };
                    let head = HEADS[gh[&pr].head_idx];
                    let sha = if short_sha { &head[..7] } else { head };
                    let r = h.wf(w, "pr", json!({"event": "ready", "repo": REPO, "pr": pr, "head": sha})).await;
                    trace.push(format!("#{step} {w} pr ready #{pr}@{sha} -> {}", r["status"]));
                    let want = model.pr_ready(w, pr, sha, &gh);
                    oracle.status(&r, &want, &trace, run_id);
                    true
                }
                Op::GhPush { pr } => {
                    let Some(pr_num) = resolve_pr(pr) else {
                        trace.push(format!("#{step} noop (gh push {pr:?})"));
                        return false;
                    };
                    let changed = {
                        let m = gh.get_mut(&pr_num).unwrap();
                        if m.exists && m.state == "OPEN" {
                            m.head_idx = (m.head_idx + 1) % HEADS.len();
                            trace.push(format!("#{step} gh push #{pr_num} -> head {}", &HEADS[m.head_idx][..7]));
                            true
                        } else {
                            trace.push(format!("#{step} noop (gh push #{pr_num} closed/missing)"));
                            false
                        }
                    };
                    if changed {
                        sync_gh(&h, &gh);
                    }
                    changed
                }
                Op::GhStatus { pr, checks, merge_state, draft } => {
                    let Some(pr_num) = resolve_pr(pr) else {
                        trace.push(format!("#{step} noop (gh status {pr:?})"));
                        return false;
                    };
                    let changed = {
                        let m = gh.get_mut(&pr_num).unwrap();
                        if m.exists && m.state == "OPEN" {
                            m.checks = checks;
                            m.merge_state = merge_state.as_str();
                            m.draft = draft;
                            trace.push(format!(
                                "#{step} gh status #{pr_num} checks={:?} merge={} draft={}",
                                m.checks, m.merge_state, m.draft
                            ));
                            true
                        } else {
                            trace.push(format!("#{step} noop (gh status #{pr_num} closed/missing)"));
                            false
                        }
                    };
                    if changed {
                        sync_gh(&h, &gh);
                    }
                    changed
                }
                Op::GhFinish { pr, state, predates } => {
                    let Some(pr_num) = resolve_pr(pr) else {
                        trace.push(format!("#{step} noop (gh finish {pr:?})"));
                        return false;
                    };
                    let changed = {
                        let m = gh.get_mut(&pr_num).unwrap();
                        if m.exists && m.state == "OPEN" {
                            m.state = state.as_str();
                            m.queued = false;
                            m.ended_at = Some(if predates { before_tracking(t0) } else { clock_now(&h) });
                            trace.push(format!(
                                "#{step} gh finish #{pr_num} state={} ended_at={:?}",
                                m.state, m.ended_at
                            ));
                            true
                        } else {
                            trace.push(format!("#{step} noop (gh finish #{pr_num} closed/missing)"));
                            false
                        }
                    };
                    if changed {
                        sync_gh(&h, &gh);
                    }
                    changed
                }
                Op::GhReadError { pr, fail } => {
                    let Some(pr_num) = resolve_pr(pr) else {
                        trace.push(format!("#{step} noop (gh read_err {pr:?} fail={fail})"));
                        return false;
                    };
                    let changed = {
                        let m = gh.get_mut(&pr_num).unwrap();
                        if m.exists && m.state == "OPEN" && m.read_err != fail {
                            m.read_err = fail;
                            trace.push(format!("#{step} gh read_err #{pr_num} fail={fail}"));
                            true
                        } else {
                            trace.push(format!("#{step} noop (gh read_err #{pr_num} fail={fail})"));
                            false
                        }
                    };
                    if changed {
                        sync_gh(&h, &gh);
                    }
                    changed
                }
                Op::GhRemovePr { pr } => {
                    let Some(pr_num) = resolve_pr(pr) else {
                        trace.push(format!("#{step} noop (gh remove {pr:?})"));
                        return false;
                    };
                    let changed = {
                        let m = gh.get_mut(&pr_num).unwrap();
                        if m.exists && m.state == "OPEN" {
                            m.exists = false;
                            trace.push(format!("#{step} gh remove #{pr_num}"));
                            true
                        } else {
                            trace.push(format!("#{step} noop (gh remove #{pr_num})"));
                            false
                        }
                    };
                    if changed {
                        sync_gh(&h, &gh);
                    }
                    changed
                }
                Op::GhClearReadErrors => {
                    let mut cleared = Vec::new();
                    for (&n, m) in gh.iter_mut() {
                        if m.read_err {
                            m.read_err = false;
                            cleared.push(n);
                        }
                    }
                    if cleared.is_empty() {
                        trace.push(format!("#{step} noop (clear read_err: none active)"));
                        false
                    } else {
                        sync_gh(&h, &gh);
                        trace.push(format!("#{step} clear read_err {cleared:?}"));
                        true
                    }
                }
                Op::Reconcile => {
                    let had_err = gh.values().any(|m| m.read_err);
                    let mut mutations_before = advance_mutations(&h);
                    let mut live_before = model.live_prs();
                    let rec = h.app.wf.reconcile().await;
                    let predicted = model.reconcile(&gh);
                    pull_gh_mutations(&h, &mut gh);
                    oracle.mutations(&predicted, &gh, rec.is_ok(), attempts(&h), &trace, run_id);
                    trace.push(format!("#{step} reconcile -> ok={}", rec.is_ok()));
                    if !had_err && let Ok(actions) = &rec {
                        healthy_reconcile = true;
                        for (&number, m) in &gh {
                            let Some(seen) = h.app.store.pr_seen_get(REPO, number).unwrap() else {
                                continue;
                            };
                            let label = format!("{REPO}#{number}");
                            if seen.state != "OPEN"
                                && seen.state == m.state
                                && let Some(at) = m.ended_at.as_deref()
                            {
                                let reported = actions.iter().any(|a| {
                                    a.starts_with(&format!("{label} merged on GitHub"))
                                        || a.starts_with(&format!("{label} closed on GitHub"))
                                });
                                if at < seen.since.as_str() {
                                    oracle_assert!(
                                        !reported,
                                        "stale finished event {label} at {at} (< since {}) was reported: \
                                         actions={actions:?}\nseed={run_id}\n{}",
                                        seen.since,
                                        trace.join("\n")
                                    );
                                }
                            }
                            if seen.state == "OPEN"
                                && !seen.queued
                                && let Some((_, at)) = &m.removed
                            {
                                let after = seen
                                    .queued_at
                                    .as_deref()
                                    .filter(|q| *q >= seen.since.as_str())
                                    .unwrap_or(&seen.since);
                                let reported = actions.iter().any(|a| {
                                    a.starts_with(&format!("{label} dropped from the merge queue"))
                                        && !a.ends_with("no reason recorded")
                                });
                                if at.as_str() < after {
                                    let event = format!("removed:{at}");
                                    let newly_marked = |target: &str| {
                                        h.app.store.pr_event_seen(REPO, number, &event, target).unwrap()
                                            && !before
                                                .pr_events
                                                .contains(&(REPO.into(), number, event.clone(), target.into()))
                                    };
                                    let marked = newly_marked(BOARD) || seen.tickets.iter().any(|(t, _)| newly_marked(t));
                                    oracle_assert!(
                                        !reported && !marked,
                                        "stale queue drop {label} at {at} (< after {after}) was reported or marked: \
                                         actions={actions:?}\nseed={run_id}\n{}",
                                        trace.join("\n")
                                    );
                                }
                            }
                        }
                        // Settle any external GitHub mutations (merge/enqueue) or newly refreshed
                        // T3 live PRs (from un-ticked `FakeSetPrs` observed via `read_thread` during
                        // `retry_ready` or `flush_to`) before asserting strict pass idempotence.
                        let max_settle = WORKERS.len() + 1;
                        for pass_idx in 0..=max_settle {
                            if advance_mutations(&h) == mutations_before && model.live_prs() == live_before {
                                break;
                            }
                            oracle_assert!(
                                pass_idx < max_settle,
                                "reconcile did not settle external GitHub/live-PR changes within {max_settle} passes\nseed={run_id}\n{}",
                                trace.join("\n")
                            );
                            mutations_before = advance_mutations(&h);
                            live_before = model.live_prs();
                            let settle = h.app.wf.reconcile().await;
                            let predicted = model.reconcile(&gh);
                            pull_gh_mutations(&h, &mut gh);
                            oracle.mutations(&predicted, &gh, settle.is_ok(), attempts(&h), &trace, run_id);
                            oracle_assert!(
                                settle.is_ok(),
                                "settling reconcile after advance/live refresh failed: {settle:?}\nseed={run_id}\n{}",
                                trace.join("\n")
                            );
                        }
                        let retrying_waits = h.app.store.merge_waits().unwrap();
                        let retrying_prs: BTreeSet<(String, u64)> = retrying_waits
                            .iter()
                            .filter(|w| w.failures > 0)
                            .map(|w| (w.repo.clone(), w.pr))
                            .collect();
                        let retrying_sent_prs: BTreeSet<(String, u64)> = retrying_waits
                            .iter()
                            .filter(|w| w.status == WaitStatus::Sent && w.failures > 0)
                            .map(|w| (w.repo.clone(), w.pr))
                            .collect();
                        let before_msgs = h.sent().len();
                        let before_cards: Vec<String> =
                            h.app.wf.cards().unwrap().into_iter().map(|c| c.id).collect();
                        let before_set: BTreeSet<String> = before_cards.iter().cloned().collect();
                        let rec2 = h.app.wf.reconcile().await;
                        let predicted = model.reconcile(&gh);
                        pull_gh_mutations(&h, &mut gh);
                        oracle.mutations(&predicted, &gh, rec2.is_ok(), attempts(&h), &trace, run_id);
                        oracle_assert!(
                            rec2.is_ok(),
                            "second reconcile failed: {rec2:?}\nseed={run_id}\n{}",
                            trace.join("\n")
                        );
                        let remaining_waits: BTreeSet<(String, u64)> = h
                            .app
                            .store
                            .merge_waits()
                            .unwrap()
                            .into_iter()
                            .map(|w| (w.repo, w.pr))
                            .collect();
                        let retired_sent_prs: BTreeSet<(String, u64)> =
                            retrying_sent_prs.into_iter().filter(|k| !remaining_waits.contains(k)).collect();
                        let after_cards: Vec<String> = h
                            .app
                            .wf
                            .cards()
                            .unwrap()
                            .into_iter()
                            .filter(|c| {
                                before_set.contains(&c.id)
                                    || !(c.kind == "report"
                                        && c.pr.as_ref().is_some_and(|p| {
                                            let key = (p.repo.clone(), p.number);
                                            (c.text.contains("GitHub would not ") && retrying_prs.contains(&key))
                                                || (c.text.contains("no reason recorded")
                                                    && retired_sent_prs.contains(&key))
                                        }))
                            })
                            .map(|c| c.id)
                            .collect();
                        oracle_assert_eq!(
                            h.sent().len(),
                            before_msgs,
                            "reconcile was not idempotent (sent extra messages)\nseed={run_id}\n{}",
                            trace.join("\n")
                        );
                        oracle_assert_eq!(
                            after_cards,
                            before_cards,
                            "reconcile was not idempotent (changed cards outside retrying waits {retrying_prs:?})\nseed={run_id}\n{}",
                            trace.join("\n")
                        );
                    }
                    true
                }
                Op::Adopt { to, from } => {
                    let (Some(to_t), Some(from_t)) = (resolve_ticket(to), resolve_ticket(from)) else {
                        trace.push(format!("#{step} noop (adopt {to:?} from {from:?})"));
                        return false;
                    };
                    if to_t == from_t || !model.is_open(to_t) {
                        trace.push(format!("#{step} noop (adopt {to_t} from {from_t})"));
                        return false;
                    }
                    let r = h.wf(to_t, "adopt", json!({"from": from_t})).await;
                    model.adopt(to_t, from_t);
                    trace.push(format!("#{step} {to_t} adopt from {from_t} -> {r}"));
                    oracle_assert!(
                        h.app.store.children(from_t, true).unwrap().is_empty(),
                        "adopt left open children on {from_t}\nseed={run_id}\n{}",
                        trace.join("\n")
                    );
                    true
                }
                Op::TicketGateClear { ticket } => {
                    let Some(t) = resolve_ticket(ticket).filter(|t| model.is_open(t)) else {
                        trace.push(format!("#{step} noop (gate clear {ticket:?})"));
                        return false;
                    };
                    let _ = h.wf(t, "gate", json!({"clear": true})).await;
                    model.ticket_gate_clear(t);
                    trace.push(format!("#{step} {t} gate clear"));
                    true
                }
                Op::TicketSendGate { ticket } => {
                    let Some(t) = resolve_ticket(ticket).filter(|t| model.is_open(t)) else {
                        trace.push(format!("#{step} noop (send gate {ticket:?})"));
                        return false;
                    };
                    let r = h
                        .wf(
                            t,
                            "send",
                            json!({"kind": "gate", "gate": "delivery:verify",
                                   "body": "**ACTION NEEDED:** verify: reply \"ok\"", "options": ["ok"]}),
                        )
                        .await;
                    model.ticket_send_gate(t, "delivery:verify");
                    trace.push(format!("#{step} {t} open delivery:verify -> {}", r["card"]));
                    true
                }
                Op::TicketGateOpen { ticket } => {
                    let Some(t) = resolve_ticket(ticket).filter(|t| model.is_open(t)) else {
                        trace.push(format!("#{step} noop (gate open {ticket:?})"));
                        return false;
                    };
                    let _ = h.wf(t, "gate", json!({"open": "merge"})).await;
                    model.ticket_gate_open(t, "merge");
                    trace.push(format!("#{step} {t} manual gate open merge"));
                    true
                }
                Op::AnswerCard { card } => {
                    let open = h.app.wf.cards().unwrap();
                    let Some((idx, c)) = resolve_card(&open, card) else {
                        trace.push(format!("#{step} noop (answer {card:?})"));
                        return false;
                    };
                    let rank = model.card_ref(&before, idx);
                    let ans = c.options.first().map(String::as_str).unwrap_or("ok");
                    let _ = h.app.wf.answer(&c.id, ans).await;
                    model.answer(rank, c.ticket.as_deref(), ans, &c.options);
                    pull_gh_mutations(&h, &mut gh);
                    trace.push(format!("#{step} answer {} ({card:?}) with {ans:?}", c.id));
                    true
                }
                Op::DismissCard { card } => {
                    let open = h.app.wf.cards().unwrap();
                    let Some((idx, c)) = resolve_card(&open, card) else {
                        trace.push(format!("#{step} noop (dismiss {card:?})"));
                        return false;
                    };
                    let rank = model.card_ref(&before, idx);
                    let res = h
                        .app
                        .wf
                        .dismiss_at(&c.id, &CardRef { at: Some(&c.at), token: Some(&c.token) })
                        .await;
                    oracle_assert!(
                        res.is_ok(),
                        "dismiss_at rejected freshly read card {}: {res:?}\nseed={run_id}\n{}",
                        c.id,
                        trace.join("\n")
                    );
                    model.dismiss(rank, c.ticket.as_deref(), c.worker.is_some());
                    trace.push(format!("#{step} dismiss_at {} ({card:?})", c.id));
                    true
                }
                Op::AnswerReadyAndClearGate { nth } => {
                    // Answer a ready merge card and immediately clear the ticket's gate;
                    // every other unanswered ready merge card on that ticket must survive!
                    let open = h.app.wf.cards().unwrap();
                    let target = CardSym { kind: CardKind::ReadyMerge, nth };
                    let Some((idx, chosen)) = resolve_card(&open, target) else {
                        trace.push(format!("#{step} noop (answer_ready_and_clear nth={nth})"));
                        return false;
                    };
                    let rank = model.card_ref(&before, idx);
                    let ticket = chosen.ticket.clone().unwrap();
                    let others: Vec<String> = open
                        .iter()
                        .filter(|c| {
                            CardKind::ReadyMerge.matches(c)
                                && c.ticket.as_deref() == Some(ticket.as_str())
                                && c.id != chosen.id
                        })
                        .map(|c| c.id.clone())
                        .collect();
                    let opt = chosen.options.first().map(String::as_str).unwrap_or("merge");
                    let _ = h.app.wf.answer(&chosen.id, opt).await;
                    pull_gh_mutations(&h, &mut gh);
                    let _ = h.wf(&ticket, "gate", json!({"clear": true})).await;
                    model.answer(rank, Some(&ticket), opt, &chosen.options);
                    model.ticket_gate_clear(&ticket);
                    trace.push(format!(
                        "#{step} answer ready {} ({opt}) + {ticket} gate clear (others={others:?})",
                        chosen.id
                    ));
                    let after_ids: BTreeSet<String> =
                        h.app.wf.cards().unwrap().into_iter().map(|c| c.id).collect();
                    for other in &others {
                        oracle_assert!(
                            after_ids.contains(other),
                            "answering {} and clearing gate wiped unanswered ready card {other}!\nseed={run_id}\n{}",
                            chosen.id,
                            trace.join("\n")
                        );
                    }
                    true
                }
                Op::WorkerReviewGate { worker } => {
                    let Some((w, _)) = resolve_worker(worker, &closed_workers) else {
                        trace.push(format!("#{step} noop (review gate {worker:?})"));
                        return false;
                    };
                    let r = h
                        .wf(
                            w,
                            "send",
                            json!({"kind": "gate", "gate": "review",
                                   "body": "**ACTION NEEDED:** decide review: reply \"fix\" or \"ship\"",
                                   "options": ["fix", "ship"]}),
                        )
                        .await;
                    model.worker_gate(w, "review", &["fix", "ship"]);
                    trace.push(format!("#{step} {w} relayed review gate -> {}", r["card"]));
                    true
                }
                Op::WorkerClose { worker } => {
                    let Some((w, _)) = resolve_worker(worker, &closed_workers) else {
                        trace.push(format!("#{step} noop (close {worker:?})"));
                        return false;
                    };
                    if WORKERS.len() - closed_workers.len() <= 1 {
                        trace.push(format!("#{step} noop (close last worker {w})"));
                        return false;
                    }
                    let parent = h.app.store.link_get(w).unwrap().unwrap().parent;
                    oracle_assert_eq!(
                        parent,
                        model.parent(w),
                        "worker {w} parent drifted from the model\nseed={run_id}\n{}",
                        trace.join("\n")
                    );
                    h.wf(&parent, "close", json!({"child": w})).await;
                    model.close(w);
                    let count = delivered_counts(&h).get(w).copied().unwrap_or(0);
                    closed_workers.insert(w.to_string(), count);
                    trace.push(format!("#{step} {parent} close {w} (sent_at_close={count})"));
                    true
                }
                Op::WorkerHandoff { worker } => {
                    let Some((w, _)) = resolve_worker(worker, &closed_workers) else {
                        trace.push(format!("#{step} noop (handoff {worker:?})"));
                        return false;
                    };
                    if WORKERS.len() - closed_workers.len() <= 1 {
                        trace.push(format!("#{step} noop (handoff last worker {w})"));
                        return false;
                    }
                    h.wf(w, "send", json!({"kind": "handoff", "body": "chaos handoff"})).await;
                    model.handoff(w);
                    let count = delivered_counts(&h).get(w).copied().unwrap_or(0);
                    closed_workers.insert(w.to_string(), count);
                    trace.push(format!("#{step} {w} handoff (sent_at_close={count})"));
                    true
                }
                Op::AdvanceTick { minutes } => {
                    trace.push(format!("#{step} advance_tick({minutes})"));
                    run_tick(&h, &mut model, &mut oracle, &closed_workers, minutes, &trace, run_id).await;
                    true
                }
                Op::MergeWaitPut { pr, ticket, reassigned_id } => {
                    let (Some(pr_num), Some(t)) = (resolve_pr(pr), resolve_ticket(ticket)) else {
                        trace.push(format!("#{step} noop (merge_wait_put {pr:?} {ticket:?})"));
                        return false;
                    };
                    let m = &gh[&pr_num];
                    if !m.exists || m.state != "OPEN" {
                        trace.push(format!("#{step} noop (merge_wait_put #{pr_num} closed/missing)"));
                        return false;
                    }
                    let head = HEADS[m.head_idx].to_string();
                    let pull_request_id = if reassigned_id {
                        format!("PR_{REPO}_{pr_num}_reassigned")
                    } else {
                        format!("PR_{REPO}_{pr_num}")
                    };
                    let approval = if reassigned_id {
                        format!("appr-{step}-reassigned")
                    } else {
                        format!("appr-{step}")
                    };
                    let w = MergeWait {
                        repo: REPO.into(),
                        pr: pr_num,
                        head: head.clone(),
                        pull_request_id,
                        method: None,
                        thread: Some(t.into()),
                        label: Some(format!("widgets#{pr_num}")),
                        merge_state: m.merge_state.into(),
                        at: clock_now(&h),
                        status: if m.queued { WaitStatus::Sent } else { WaitStatus::Waiting },
                        approval,
                        failures: 0,
                        last_error: None,
                    };
                    h.app.store.merge_wait_put(&w).unwrap();
                    model.wait_put(pr_num, wait_model(&w, !reassigned_id));
                    trace.push(format!(
                        "#{step} merge_wait_put{} #{pr_num}@{head} for {t}",
                        if reassigned_id { "(reassigned_id)" } else { "" }
                    ));
                    true
                }
                Op::GhQueueConfig { pr, enabled, queued } => {
                    let Some(pr_num) = resolve_pr(pr) else {
                        trace.push(format!("#{step} noop (gh queue {pr:?})"));
                        return false;
                    };
                    let changed = {
                        let m = gh.get_mut(&pr_num).unwrap();
                        if m.exists && m.state == "OPEN" {
                            m.queue_enabled = enabled;
                            m.queued = enabled && queued;
                            m.removed = None;
                            trace.push(format!(
                                "#{step} gh queue #{pr_num} enabled={} queued={}",
                                m.queue_enabled, m.queued
                            ));
                            true
                        } else {
                            trace.push(format!("#{step} noop (gh queue #{pr_num} closed/missing)"));
                            false
                        }
                    };
                    if changed {
                        sync_gh(&h, &gh);
                    }
                    changed
                }
                Op::GhQueueDrop { pr, reason, predates } => {
                    let Some(pr_num) = resolve_pr(pr) else {
                        trace.push(format!("#{step} noop (gh queue drop {pr:?})"));
                        return false;
                    };
                    let changed = {
                        let m = gh.get_mut(&pr_num).unwrap();
                        if m.exists && m.state == "OPEN" {
                            m.queued = false;
                            let at = if predates { before_tracking(t0) } else { clock_now(&h) };
                            m.removed = Some((reason.as_str(), at));
                            trace.push(format!("#{step} gh queue drop #{pr_num} removed={:?}", m.removed));
                            true
                        } else {
                            trace.push(format!("#{step} noop (gh queue drop #{pr_num} closed/missing)"));
                            false
                        }
                    };
                    if changed {
                        sync_gh(&h, &gh);
                    }
                    changed
                }
                Op::FakeSetPrs { worker, pr } => {
                    let (Some((w, _)), Some(pr_num)) = (resolve_worker(worker, &closed_workers), resolve_pr(pr)) else {
                        trace.push(format!("#{step} noop (fake.set_prs {worker:?} {pr:?})"));
                        return false;
                    };
                    h.fake.set_prs(w, vec![pull_request(REPO, pr_num, "open")]);
                    model.set_prs(w, pr_num);
                    trace.push(format!("#{step} fake.set_prs({w}, #{pr_num})"));
                    true
                }
                Op::WorkerSetBusy { worker, busy } => {
                    let Some((w, _)) = resolve_worker(worker, &closed_workers) else {
                        trace.push(format!("#{step} noop (set_busy {worker:?})"));
                        return false;
                    };
                    if busy_workers.contains(w) == busy {
                        trace.push(format!("#{step} noop ({w} already busy={busy})"));
                        return false;
                    }
                    let status = if busy { "running" } else { "idle" };
                    h.fake.update(w, json!({"status": status, "updatedAt": clock_now(&h)}));
                    if busy {
                        busy_workers.insert(w);
                    } else {
                        busy_workers.remove(w);
                    }
                    model.set_busy(w, busy);
                    trace.push(format!("#{step} {w} status={status}"));
                    true
                }
                Op::ParentReply { worker } => {
                    let Some((w, _)) = resolve_worker(worker, &closed_workers) else {
                        trace.push(format!("#{step} noop (parent reply {worker:?})"));
                        return false;
                    };
                    let parent = h.app.store.link_get(w).unwrap().unwrap().parent;
                    let r = h.wf(&parent, "reply", json!({"to": w, "body": "chaos reply"})).await;
                    model.reply(w);
                    trace.push(format!("#{step} {parent} reply {w} -> {}", r["delivery"]));
                    true
                }
                Op::TicketClose { ticket, by_board } => {
                    let Some(t) = resolve_ticket(ticket).filter(|t| model.is_open(t)) else {
                        trace.push(format!("#{step} noop (ticket close {ticket:?})"));
                        return false;
                    };
                    if !model.is_archived_or_gone(t) && live_tickets_count(&model) <= 1 {
                        trace.push(format!("#{step} noop (ticket close last live ticket {t})"));
                        return false;
                    }
                    if by_board {
                        let r = h.app.wf.close_ticket(t, false).await.unwrap();
                        oracle_assert_eq!(
                            r["closed"].as_bool(),
                            Some(true),
                            "close_ticket failed for {t}: {r}\nseed={run_id}\n{}",
                            trace.join("\n")
                        );
                    } else {
                        let r = h.wf(t, "send", json!({"kind": "closed", "body": "chaos ticket close"})).await;
                        oracle_assert_eq!(
                            r["closed"].as_bool(),
                            Some(true),
                            "ticket send closed failed for {t}: {r}\nseed={run_id}\n{}",
                            trace.join("\n")
                        );
                    }
                    model.close_ticket(t, by_board);
                    trace.push(format!("#{step} ticket close {t} (by_board={by_board})"));
                    true
                }
                Op::TicketArchive { ticket, tick } => {
                    let Some(t) = resolve_ticket(ticket) else {
                        trace.push(format!("#{step} noop (ticket archive {ticket:?})"));
                        return false;
                    };
                    if !model.is_open(t) || model.is_archived_or_gone(t) || live_tickets_count(&model) <= 1 {
                        trace.push(format!("#{step} noop (ticket archive {t})"));
                        return false;
                    }
                    h.fake.update(t, json!({"archived": true}));
                    model.mark_archived(t);
                    trace.push(format!("#{step} ticket archive {t} (tick={tick})"));
                    if tick {
                        run_tick(&h, &mut model, &mut oracle, &closed_workers, 0, &trace, run_id).await;
                    }
                    true
                }
                Op::TicketThreadGone { ticket, tick } => {
                    let Some(t) = resolve_ticket(ticket) else {
                        trace.push(format!("#{step} noop (ticket thread_gone {ticket:?})"));
                        return false;
                    };
                    if !model.is_open(t) || model.is_archived_or_gone(t) || live_tickets_count(&model) <= 1 {
                        trace.push(format!("#{step} noop (ticket thread_gone {t})"));
                        return false;
                    }
                    h.fake.remove(t);
                    model.mark_gone(t);
                    trace.push(format!("#{step} ticket thread_gone {t} (tick={tick})"));
                    if tick {
                        run_tick(&h, &mut model, &mut oracle, &closed_workers, 0, &trace, run_id).await;
                    }
                    true
                }
                Op::WfReply { worker, by_label, reopen } => {
                    let Some(&w) = WORKERS.get(worker.0) else {
                        trace.push(format!("#{step} noop (wf.reply {worker:?})"));
                        return false;
                    };
                    let parent = h.app.store.link_get(w).unwrap().unwrap().parent;
                    oracle_assert_eq!(
                        parent,
                        model.parent(w),
                        "worker {w} parent drifted from the model\nseed={run_id}\n{}",
                        trace.join("\n")
                    );
                    let label = model.label(w);
                    let to = if by_label { label.as_str() } else { w };
                    if !model.is_open(w) && !reopen {
                        let err = h
                            .wf_err(&parent, "reply", json!({"to": to, "body": "chaos wf.reply", "reopen": false}))
                            .await;
                        oracle_assert!(
                            err.contains("closed") && err.contains("reopen"),
                            "wf.reply to closed {w} without reopen gave unexpected error: {err}\nseed={run_id}\n{}",
                            trace.join("\n")
                        );
                        trace.push(format!("#{step} {parent} wf.reply to={to:?} reopen=false -> refused (closed)"));
                    } else {
                        let r =
                            h.wf(&parent, "reply", json!({"to": to, "body": "chaos wf.reply", "reopen": reopen})).await;
                        model.reply_with_reopen(w, reopen);
                        trace.push(format!(
                            "#{step} {parent} wf.reply to={to:?} reopen={reopen} -> {}",
                            r["delivery"]
                        ));
                    }
                    true
                }
                Op::Pause { target, paused } => {
                    let (id, r) = match target {
                        PauseTarget::Ticket { ticket } => {
                            let Some(t) = resolve_ticket(ticket).filter(|t| model.is_open(t)) else {
                                trace.push(format!("#{step} noop (pause {target:?} paused={paused})"));
                                return false;
                            };
                            (t, h.wf(t, "pause", json!({"paused": paused})).await)
                        }
                        PauseTarget::WorkerSelf { worker } => {
                            let Some((w, _)) = resolve_worker(worker, &closed_workers) else {
                                trace.push(format!("#{step} noop (pause {target:?} paused={paused})"));
                                return false;
                            };
                            (w, h.wf(w, "pause", json!({"paused": paused})).await)
                        }
                        PauseTarget::WorkerByParent { worker } => {
                            let Some((w, _)) = resolve_worker(worker, &closed_workers) else {
                                trace.push(format!("#{step} noop (pause {target:?} paused={paused})"));
                                return false;
                            };
                            let parent = model.parent(w);
                            (w, h.wf(&parent, "pause", json!({"child": w, "paused": paused})).await)
                        }
                        PauseTarget::BoardTicket { ticket } => {
                            let Some(t) = resolve_ticket(ticket).filter(|t| model.is_open(t)) else {
                                trace.push(format!("#{step} noop (pause {target:?} paused={paused})"));
                                return false;
                            };
                            (t, h.app.wf.pause_link(t, paused).await.unwrap())
                        }
                        PauseTarget::BoardWorker { worker } => {
                            let Some((w, _)) = resolve_worker(worker, &closed_workers) else {
                                trace.push(format!("#{step} noop (pause {target:?} paused={paused})"));
                                return false;
                            };
                            (w, h.app.wf.pause_link(w, paused).await.unwrap())
                        }
                    };
                    oracle_assert_eq!(
                        r["paused"].as_bool(),
                        Some(paused),
                        "pause response diverged for {id}: {r}\nseed={run_id}\n{}",
                        trace.join("\n")
                    );
                    model.set_paused(id, paused);
                    trace.push(format!("#{step} pause {id} ({target:?}) paused={paused}"));
                    true
                }
                Op::Sweep { target } => {
                    let id = match target {
                        SweepTarget::Ticket { ticket } => {
                            let Some(t) = resolve_ticket(ticket) else {
                                trace.push(format!("#{step} noop (sweep {ticket:?})"));
                                return false;
                            };
                            t
                        }
                        SweepTarget::Worker { worker } => {
                            let Some(&w) = WORKERS.get(worker.0) else {
                                trace.push(format!("#{step} noop (sweep {worker:?})"));
                                return false;
                            };
                            w
                        }
                    };
                    let r = h.wf(id, "sweep", json!({})).await;
                    let mut got_open: Vec<String> = r["open"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect();
                    got_open.sort();
                    let got_dropped: Vec<String> = r["dropped"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect();
                    let (want_open, want_parent_open) = model.sweep(id);
                    trace.push(format!("#{step} {id} sweep -> open={got_open:?} parentOpen={}", r["parentOpen"]));
                    oracle_assert_eq!(
                        got_open,
                        want_open,
                        "wf.sweep open children diverged for {id}\nseed={run_id}\n{}",
                        trace.join("\n")
                    );
                    oracle_assert!(
                        got_dropped.is_empty(),
                        "wf.sweep unexpectedly dropped {got_dropped:?} for {id}\nseed={run_id}\n{}",
                        trace.join("\n")
                    );
                    oracle_assert_eq!(
                        r["parentOpen"].as_bool(),
                        Some(want_parent_open),
                        "wf.sweep parentOpen diverged for {id}\nseed={run_id}\n{}",
                        trace.join("\n")
                    );
                    true
                }
                Op::SetRelay { relay } => {
                    if model.relay() == relay {
                        trace.push(format!("#{step} noop (set_relay already {relay})"));
                        return false;
                    }
                    let cfg = json!({"roles": {"ticket": {"children": {"worker": {"relay": relay}}}}});
                    std::fs::write(h.dir.path().join("t3cm-config.json"), cfg.to_string()).unwrap();
                    let reloaded = h.app.wf.reload_config().await.unwrap();
                    oracle_assert!(
                        reloaded.restart.is_empty(),
                        "reloading worker relay unexpectedly required restart: {:?}\nseed={run_id}\n{}",
                        reloaded.restart,
                        trace.join("\n")
                    );
                    model.set_relay(relay);
                    trace.push(format!("#{step} set_relay({relay})"));
                    true
                }
                Op::WorkerSetMergeOnly { worker, merge_only } => {
                    let Some((w, _)) = resolve_worker(worker, &closed_workers) else {
                        trace.push(format!("#{step} noop (set_merge_only {worker:?})"));
                        return false;
                    };
                    let parent = model.parent(w);
                    if model.merge_only(w) == merge_only || !model.is_open(&parent) {
                        trace.push(format!("#{step} noop (set_merge_only {w} merge_only={merge_only})"));
                        return false;
                    }
                    let ln = h.app.store.link_get(w).unwrap().unwrap();
                    let part = ln.identity.part.unwrap_or_else(|| "1/3".into());
                    h.wf(
                        &parent,
                        "openChild",
                        json!({"role": "worker", "label": ln.label, "issue": "acme/widgets#41", "part": part,
                               "pr": {"format": "pr", "mergeOnly": merge_only}}),
                    )
                    .await;
                    h.wf(w, "join", json!({"parent": parent, "label": ln.label, "role": "worker"})).await;
                    model.set_merge_only(w, merge_only);
                    trace.push(format!("#{step} {w} set_merge_only({merge_only})"));
                    true
                }
                Op::PrFollowup { worker, pr, event, short_sha, no_ci } => {
                    let (Some((w, _)), Some(pr_num)) = (resolve_worker(worker, &closed_workers), resolve_pr(pr)) else {
                        trace.push(format!("#{step} noop (pr followup {worker:?} {pr:?})"));
                        return false;
                    };
                    let Some(primary) = model.primary_pr(w) else {
                        trace.push(format!("#{step} noop (pr followup {w} has no primary PR yet)"));
                        return false;
                    };
                    if primary == pr_num {
                        trace.push(format!("#{step} noop (pr followup {w} #{pr_num} matches primary)"));
                        return false;
                    }
                    let head = HEADS[gh[&pr_num].head_idx];
                    let sha = if short_sha { &head[..7] } else { head };
                    let (r, want, ev_str) = match event {
                        PrEventKind::Opened => {
                            let checks = if no_ci { "none" } else { "pending" };
                            let r = h
                                .wf(
                                    w,
                                    "pr",
                                    json!({"event": "opened", "repo": REPO, "pr": pr_num, "head": sha,
                                           "publishing": PUBLISHING, "checks": checks}),
                                )
                                .await;
                            let want = model.pr_opened(w, pr_num, sha, no_ci, &gh);
                            (r, want, "opened")
                        }
                        PrEventKind::Updated => {
                            let r = h
                                .wf(w, "pr", json!({"event": "updated", "repo": REPO, "pr": pr_num, "head": sha}))
                                .await;
                            let want = model.pr_updated(w, pr_num, sha);
                            (r, want, "updated")
                        }
                        PrEventKind::Ready => {
                            let r =
                                h.wf(w, "pr", json!({"event": "ready", "repo": REPO, "pr": pr_num, "head": sha})).await;
                            let want = model.pr_ready(w, pr_num, sha, &gh);
                            (r, want, "ready")
                        }
                    };
                    trace.push(format!("#{step} {w} pr followup {ev_str} #{pr_num}@{sha} -> {}", r["status"]));
                    oracle.status(&r, &want, &trace, run_id);
                    true
                }
                Op::GhFilesPaginated { pr, paginated } => {
                    let Some(pr_num) = resolve_pr(pr) else {
                        trace.push(format!("#{step} noop (gh files_paginated {pr:?})"));
                        return false;
                    };
                    let changed = {
                        let m = gh.get_mut(&pr_num).unwrap();
                        if m.exists && m.state == "OPEN" && m.many_files != paginated {
                            m.many_files = paginated;
                            trace.push(format!("#{step} gh files_paginated #{pr_num} paginated={paginated}"));
                            true
                        } else {
                            trace.push(format!("#{step} noop (gh files_paginated #{pr_num} paginated={paginated})"));
                            false
                        }
                    };
                    if changed {
                        sync_gh(&h, &gh);
                    }
                    changed
                }
                Op::GhReopen { pr } => {
                    let Some(pr_num) = resolve_pr(pr) else {
                        trace.push(format!("#{step} noop (gh reopen {pr:?})"));
                        return false;
                    };
                    let changed = {
                        let m = gh.get_mut(&pr_num).unwrap();
                        if m.exists && m.state == "CLOSED" {
                            m.state = "OPEN";
                            m.ended_at = None;
                            m.queued = false;
                            trace.push(format!("#{step} gh reopen #{pr_num}"));
                            true
                        } else {
                            trace.push(format!("#{step} noop (gh reopen #{pr_num} not closed)"));
                            false
                        }
                    };
                    if changed {
                        sync_gh(&h, &gh);
                    }
                    changed
                }
            }
        })
        .await;
        let executed =
            res.unwrap_or_else(|_| panic!("step timed out (possible deadlock) at seed={run_id}\n{}", trace.join("\n")));
        op_stats.record(op.kind(), executed);
        for (w, &n) in &model.reopen_wakes {
            if let Some(count) = closed_workers.get_mut(w) {
                *count += n;
            }
        }
        let now_counts = delivered_counts(&h);
        for &t in TICKETS {
            if !model.is_open(t) && !closed_tickets.contains_key(t) {
                closed_tickets.insert(t.to_string(), now_counts.get(t).copied().unwrap_or(0));
            }
        }
        assert_invariants(&h, run_id, &trace, &closed_tickets, &closed_workers, healthy_reconcile, &gh);
        oracle.check(&h, &mut model, sent_before, &trace, run_id);
    }
    (trace, h.app.wf.reach().snapshot(), op_stats)
}

async fn probe_sequential_ops(
    ops: Vec<Op>,
    sentinel: Option<Sentinel>,
    run_id: String,
) -> ProbeResult<(Vec<String>, ReachSnapshot, OpStats)> {
    probe_oracle(async move { run_sequential_ops(&ops, sentinel, true, &run_id).await }).await
}

/// Shrink a failing sequential op list using `run_sequential_ops` as the oracle,
/// keeping the candidate locked onto the original failure class.
async fn shrink_sequential_failure(
    ops: Vec<Op>,
    sentinel: Option<Sentinel>,
    class: FailureClass,
    failure_msg: &str,
) -> ShrinkOutcome {
    let target = class.clone();
    shrink_ops(ops, class, failure_msg, async move |cand: &[Op]| {
        let res = probe_sequential_ops(cand.to_vec(), sentinel, "shrink".to_string()).await;
        target.matches(&res)
    })
    .await
}

/// Verify a minimized sequence outside the shrinker memo and write its corpus file on match.
fn publish_verified_shrink<T: std::fmt::Debug>(
    shrunk: &ShrinkOutcome,
    recheck: &ProbeResult<T>,
    source: &str,
    corpus_dir: &std::path::Path,
    sentinel_name: Option<&str>,
) -> Result<(std::path::PathBuf, String), String> {
    if !shrunk.class.matches(recheck) {
        return Err(format!(
            "CHAOS_SHRINK could not reproduce {source} ({:?} / {}) on recheck; skipped writing a corpus file\n\
             recheck outcome: {recheck:?}",
            shrunk.class.kind, shrunk.class.signature
        ));
    }
    let written = write_corpus_file(corpus_dir, shrunk, source)
        .map_err(|e| format!("CHAOS_SHRINK failed to write corpus file: {e}"))?;
    let report = format_shrink_report(shrunk, source, &written, sentinel_name);
    Ok((written, report))
}

/// Execute `ops`, running `ddmin` + value simplification and writing `<corpus_dir>/<hash>.ops`
/// when `shrink` is enabled and execution fails.
async fn run_sequential_ops_maybe_shrink(
    ops: &[Op],
    sentinel: Option<Sentinel>,
    source: &str,
    shrink: bool,
    corpus_dir: &std::path::Path,
) -> (Vec<String>, ReachSnapshot, OpStats) {
    if !shrink {
        return run_sequential_ops(ops, sentinel, true, source).await;
    }
    let outcome = probe_sequential_ops(ops.to_vec(), sentinel, source.to_string()).await;
    let failure_msg = match &outcome {
        ProbeResult::Ok(res) => return res.clone(),
        ProbeResult::OracleFailure(msg) | ProbeResult::OtherPanic { message: msg, .. } => msg.clone(),
    };
    let class = FailureClass::from_probe(&outcome).expect("non-Ok probe must classify");
    let shrunk = shrink_sequential_failure(ops.to_vec(), sentinel, class, &failure_msg).await;
    let recheck = probe_sequential_ops(shrunk.ops.clone(), sentinel, format!("recheck {source}")).await;
    match publish_verified_shrink(&shrunk, &recheck, source, corpus_dir, sentinel.map(Sentinel::name)) {
        Ok((_, report)) => {
            eprintln!("{report}");
            panic!("{report}\n\nOriginal failure:\n{failure_msg}");
        }
        Err(err) => {
            eprintln!("{err}");
            panic!("{err}\n\nOriginal failure:\n{failure_msg}");
        }
    }
}

#[tokio::test]
async fn seeded_sequential_chaos_holds_all_invariants() {
    let sentinel = chaos_sentinel_from_env();
    let shrink = chaos_shrink_enabled();
    let corpus_dir = corpus_dir_from_env();
    if let Some(ops_path) = chaos_ops_from_env() {
        let (resolved, ops) = load_ops_file(&ops_path).unwrap_or_else(|e| panic!("{e}"));
        let source = format!("ops {}", display_source_path(&resolved));
        let (_, snap, stats) = run_sequential_ops_maybe_shrink(&ops, sentinel, &source, shrink, &corpus_dir).await;
        if std::env::var_os("CHAOS_REACH").is_some() {
            eprintln!("{snap}\n{stats}");
        }
        return;
    }

    let range = chaos_seed_range(DEFAULT_SEQ_SEEDS);
    let mut reach = ReachSnapshot::default();
    let mut op_stats = OpStats::default();
    for seed in range.seeds() {
        let (_, snap, stats) = if shrink {
            let ops = decode_seed_ops(seed);
            run_sequential_ops_maybe_shrink(&ops, sentinel, &format!("seed {seed}"), true, &corpus_dir).await
        } else {
            run_sequential_seed_with(seed, sentinel).await
        };
        reach.merge(&snap);
        op_stats.merge(&stats);
    }
    if std::env::var_os("CHAOS_REACH").is_some() {
        eprintln!("{reach}\n{op_stats}");
    }
    if range.count >= DEFAULT_SEQ_SEEDS {
        let ctx = format!("seeded_sequential_chaos ({})", range.describe());
        reach.assert_tier1(&ctx);
        op_stats.assert_all_executed(&ctx);
    }
}

#[tokio::test]
async fn chaos_corpus_replays_cleanly() {
    let dir = default_corpus_dir();
    let files = list_corpus_files(&dir).unwrap_or_else(|e| panic!("{e}"));
    assert!(!files.is_empty(), "expected at least one committed .ops file in {}", dir.display());
    for path in &files {
        let (_, ops) = load_ops_file(path).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            deserialize_ops(&serialize_ops(&ops)).unwrap(),
            ops,
            "corpus file {} did not round-trip through serialize_ops",
            path.display()
        );
        let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("corpus.ops");
        let _ = run_sequential_ops(&ops, None, true, name).await;
    }
}

async fn run_concurrent_seed(seed: u64) -> (Vec<String>, H) {
    run_concurrent_seed_with(seed, None).await
}

async fn run_concurrent_seed_with(seed: u64, sentinel: Option<Sentinel>) -> (Vec<String>, H) {
    let mut rng = Rng::new(seed ^ 0x55aa_1234_9876_fedc);
    let (h, mut gh) = setup_chaos(false);
    enable_sentinel(&h, sentinel);
    for m in gh.values_mut() {
        m.queue_enabled = rng.coin();
    }
    sync_gh(&h, &gh);
    let t0 = h.clock.now();
    let mut trace = Vec::new();
    let mut model = Model::new(&clock_now(&h));

    for (i, &t) in TICKETS.iter().enumerate() {
        let label = format!("widgets#{}", 41 + i);
        h.wf(t, "join", json!({"parent": BOARD, "label": label, "role": "ticket"})).await;
        model.join_ticket(t, &label);
    }
    for (i, &w) in WORKERS.iter().enumerate() {
        let owner = TICKETS[0];
        let part = format!("{}/3", i + 1);
        let label = format!("#41 PR {part}");
        h.wf(
            owner,
            "openChild",
            json!({"role": "worker", "label": label, "issue": "acme/widgets#41", "part": part,
                   "pr": {"format": "pr"}}),
        )
        .await;
        h.wf(w, "join", json!({"parent": owner, "label": label, "role": "worker"})).await;
        model.join_worker(w, owner, &label);
    }

    h.github.set_delay(Duration::from_millis(15));
    let mut oracle = Oracle::default();
    for round in 0..6 {
        h.clock.advance_minutes(1);
        let race = rng.pick(7);
        model.step(&clock_now(&h));
        let sent_before = h.sent().len();
        let before = h.snapshot();
        let mut ops: Vec<Raced> = Vec::new();
        let mut ops_gh = gh.clone();
        let mut statuses: Vec<Option<String>> = Vec::new();
        let mut actual_gh = None;
        let mut advance_race: Option<AdvanceRace> = None;
        let res = tokio::time::timeout(Duration::from_secs(5), async {
            match race {
                0 => {
                    // Two workers call ready concurrently while ticket adopts/clears gate
                    ops = vec![
                        Raced::Ready(WORKERS[0], 7, HEADS[gh[&7].head_idx].into()),
                        Raced::Ready(WORKERS[1], 8, HEADS[gh[&8].head_idx].into()),
                        Raced::GateClear(TICKETS[0]),
                    ];
                    let r1 = h.wf(
                        WORKERS[0],
                        "pr",
                        json!({"event": "ready", "repo": REPO, "pr": 7, "head": HEADS[gh[&7].head_idx]}),
                    );
                    let r2 = h.wf(
                        WORKERS[1],
                        "pr",
                        json!({"event": "ready", "repo": REPO, "pr": 8, "head": HEADS[gh[&8].head_idx]}),
                    );
                    let clr = async {
                        tokio::time::sleep(Duration::from_millis(8)).await;
                        h.wf(TICKETS[0], "gate", json!({"clear": true})).await
                    };
                    let (a, b, _) = join3(r1, r2, clr).await;
                    statuses =
                        vec![a["status"].as_str().map(String::from), b["status"].as_str().map(String::from), None];
                    trace.push(format!("round {round}: concurrent ready(#7,#8) + gate clear"));
                }
                1 => {
                    // Reconcile races with adopt + PR merge/move
                    {
                        let m = gh.get_mut(&7).unwrap();
                        if m.state == "OPEN" {
                            if rng.coin() {
                                m.head_idx = (m.head_idx + 1) % HEADS.len();
                            } else {
                                m.state = "MERGED";
                                m.queued = false;
                                let predates = rng.pick(4) == 0;
                                m.ended_at = Some(if predates { before_tracking(t0) } else { clock_now(&h) });
                            }
                        }
                    }
                    sync_gh(&h, &gh);
                    let from = TICKETS[rng.pick(2)];
                    let to = if from == TICKETS[0] { TICKETS[1] } else { TICKETS[2] };
                    ops_gh = gh.clone();
                    ops = vec![Raced::Reconcile, Raced::Adopt(to, from)];
                    let rec = h.app.wf.reconcile();
                    let ad = async {
                        tokio::time::sleep(Duration::from_millis(6)).await;
                        h.wf(to, "adopt", json!({"from": from})).await
                    };
                    let (r, _) = join(rec, ad).await;
                    r.unwrap();
                    trace.push(format!("round {round}: reconcile racing with {to} adopt({from})"));
                }
                2 => {
                    // ready races with updated to new head + reconcile
                    let old_head = HEADS[gh[&8].head_idx];
                    let next_idx = (gh[&8].head_idx + 1) % HEADS.len();
                    let new_head = HEADS[next_idx];
                    ops = vec![
                        Raced::Ready(WORKERS[1], 8, old_head.into()),
                        Raced::Updated(WORKERS[1], 8, new_head.into()),
                        Raced::Reconcile,
                    ];
                    let rdy =
                        h.wf(WORKERS[1], "pr", json!({"event": "ready", "repo": REPO, "pr": 8, "head": old_head}));
                    let upd = async {
                        tokio::time::sleep(Duration::from_millis(7)).await;
                        h.wf(WORKERS[1], "pr", json!({"event": "updated", "repo": REPO, "pr": 8, "head": new_head}))
                            .await
                    };
                    let rec = h.app.wf.reconcile();
                    let (a, b, r) = join3(rdy, upd, rec).await;
                    statuses =
                        vec![a["status"].as_str().map(String::from), b["status"].as_str().map(String::from), None];
                    r.unwrap();
                    pull_gh_mutations(&h, &mut gh);
                    actual_gh = Some(gh.clone());
                    gh.get_mut(&8).unwrap().head_idx = next_idx;
                    sync_gh(&h, &gh);
                    trace.push(format!("round {round}: ready(#8) racing with updated + reconcile"));
                }
                3 => {
                    // opened races with adopt
                    let from = TICKETS[rng.pick(2)];
                    let to = TICKETS[2];
                    ops = vec![Raced::Opened(WORKERS[2], 9, HEADS[gh[&9].head_idx].into()), Raced::Adopt(to, from)];
                    let op = h.wf(
                        WORKERS[2],
                        "pr",
                        json!({"event": "opened", "repo": REPO, "pr": 9, "head": HEADS[gh[&9].head_idx],
                               "publishing": PUBLISHING}),
                    );
                    let ad = async {
                        tokio::time::sleep(Duration::from_millis(6)).await;
                        h.wf(to, "adopt", json!({"from": from})).await
                    };
                    let (o, _) = join(op, ad).await;
                    statuses = vec![o["status"].as_str().map(String::from), None];
                    trace.push(format!("round {round}: opened(#9) racing with {to} adopt({from})"));
                }
                4 => {
                    // pending ready + reconcile + answer open cards concurrently
                    {
                        let m = gh.get_mut(&9).unwrap();
                        if m.state == "OPEN" {
                            m.checks =
                                if m.checks == GhChecks::Pending { GhChecks::Passing } else { GhChecks::Pending };
                        }
                    }
                    sync_gh(&h, &gh);
                    ops_gh = gh.clone();
                    ops = vec![Raced::Ready(WORKERS[2], 9, HEADS[gh[&9].head_idx].into()), Raced::Reconcile];
                    let rdy = h.wf(
                        WORKERS[2],
                        "pr",
                        json!({"event": "ready", "repo": REPO, "pr": 9, "head": HEADS[gh[&9].head_idx]}),
                    );
                    let rec = h.app.wf.reconcile();
                    let open = h.app.wf.cards().unwrap();
                    if let Some(c) = open.first() {
                        let opt = c.options.first().cloned().unwrap_or_else(|| "ok".into());
                        ops.push(Raced::Answer(model.card_ref(&before, 0), c.ticket.clone(), opt, c.options.clone()));
                    }
                    let ans = async {
                        if let Some(c) = open.first() {
                            let opt = c.options.first().map(String::as_str).unwrap_or("ok");
                            let _ = h.app.wf.answer(&c.id, opt).await;
                        }
                    };
                    let (a, r, _) = join3(rdy, rec, ans).await;
                    statuses = vec![a["status"].as_str().map(String::from), None, None];
                    r.unwrap();
                    trace.push(format!("round {round}: ready(#9) + reconcile + answer"));
                }
                5 => {
                    // Reconcile advance fails its 3rd attempt while a newer approval replaces the wait;
                    // CAS retire must preserve the newer approval.
                    let pr = PRS[0];
                    if gh[&pr].state == "OPEN" {
                        let head_idx = gh[&pr].head_idx;
                        let head = HEADS[head_idx].to_string();
                        {
                            let m = gh.get_mut(&pr).unwrap();
                            m.draft = false;
                            m.checks = GhChecks::Passing;
                            m.merge_state = "CLEAN";
                            m.queued = false;
                            m.removed = None;
                        }
                        sync_gh(&h, &gh);
                        ops_gh = gh.clone();
                        let w1 = MergeWait {
                            repo: REPO.into(),
                            pr,
                            head: head.clone(),
                            pull_request_id: format!("PR_{REPO}_{pr}"),
                            method: None,
                            thread: Some(TICKETS[0].into()),
                            label: Some(format!("widgets#{pr}")),
                            merge_state: "CLEAN".into(),
                            at: clock_now(&h),
                            status: WaitStatus::Waiting,
                            approval: format!("appr-cas-{seed}-{round}-old"),
                            failures: 2,
                            last_error: Some("earlier transient error".into()),
                        };
                        h.app.store.merge_wait_put(&w1).unwrap();
                        model.wait_put(pr, wait_model(&w1, true));
                        let next_idx = (head_idx + 1) % HEADS.len();
                        let next_head = HEADS[next_idx].to_string();
                        let w2 = MergeWait {
                            repo: REPO.into(),
                            pr,
                            head: next_head.clone(),
                            pull_request_id: format!("PR_{REPO}_{pr}"),
                            method: None,
                            thread: Some(TICKETS[0].into()),
                            label: Some(format!("widgets#{pr}")),
                            merge_state: "CLEAN".into(),
                            at: clock_now(&h),
                            status: WaitStatus::Waiting,
                            approval: format!("appr-cas-{seed}-{round}-new"),
                            failures: 0,
                            last_error: None,
                        };
                        let w2_model = wait_model(&w2, true);
                        let rec = h.app.wf.reconcile();
                        let replace = async {
                            tokio::time::sleep(Duration::from_millis(22)).await;
                            h.github.update_pr(REPO, pr, json!({"headRefOid": next_head}));
                            h.app.store.merge_wait_put(&w2).unwrap();
                        };
                        let (r, _) = join(rec, replace).await;
                        r.unwrap();
                        gh.get_mut(&pr).unwrap().head_idx = next_idx;
                        advance_race =
                            Some(AdvanceRace::FailAndReplace { pr, new_head_idx: next_idx, replacement: w2_model });
                        trace.push(format!(
                            "round {round}: advance(#{pr}) fails on push while newer approval replaces wait"
                        ));
                    }
                }
                _ => {
                    // Reconcile advance enqueues on GitHub while the wait is replaced or removed mid-flight;
                    // advance must honor the false merge_wait_swap CAS result instead of mutating pr_seen.
                    let pr = PRS[0];
                    if gh[&pr].state == "OPEN" {
                        let head = HEADS[gh[&pr].head_idx].to_string();
                        {
                            let m = gh.get_mut(&pr).unwrap();
                            m.draft = false;
                            m.checks = GhChecks::Passing;
                            m.merge_state = "CLEAN";
                            m.queue_enabled = true;
                            m.queued = false;
                            m.removed = None;
                        }
                        sync_gh(&h, &gh);
                        ops_gh = gh.clone();
                        let w1 = MergeWait {
                            repo: REPO.into(),
                            pr,
                            head: head.clone(),
                            pull_request_id: format!("PR_{REPO}_{pr}"),
                            method: None,
                            thread: Some(TICKETS[0].into()),
                            label: Some(format!("widgets#{pr}")),
                            merge_state: "CLEAN".into(),
                            at: clock_now(&h),
                            status: WaitStatus::Waiting,
                            approval: format!("appr-swap-{seed}-{round}-old"),
                            failures: 0,
                            last_error: None,
                        };
                        h.app.store.merge_wait_put(&w1).unwrap();
                        model.wait_put(pr, wait_model(&w1, true));
                        let replacement = rng.coin().then(|| MergeWait {
                            repo: REPO.into(),
                            pr,
                            head: head.clone(),
                            pull_request_id: format!("PR_{REPO}_{pr}"),
                            method: None,
                            thread: Some(TICKETS[0].into()),
                            label: Some(format!("widgets#{pr}")),
                            merge_state: "CLEAN".into(),
                            at: clock_now(&h),
                            status: WaitStatus::Waiting,
                            approval: format!("appr-swap-{seed}-{round}-new"),
                            failures: 0,
                            last_error: None,
                        });
                        let rep_model = replacement.as_ref().map(|w| wait_model(w, true));
                        let rec = h.app.wf.reconcile();
                        let swap_out = async {
                            tokio::time::sleep(Duration::from_millis(22)).await;
                            match &replacement {
                                Some(w2) => h.app.store.merge_wait_put(w2).unwrap(),
                                None => {
                                    h.app.store.merge_wait_remove(REPO, pr).unwrap();
                                }
                            }
                        };
                        let (r, _) = join(rec, swap_out).await;
                        r.unwrap();
                        advance_race = Some(AdvanceRace::SucceedAndReplace { pr, replacement: rep_model });
                        trace.push(format!(
                            "round {round}: advance(#{pr}) races with wait {} during enqueue",
                            if replacement.is_some() { "replacement" } else { "removal" }
                        ));
                    }
                }
            }
        })
        .await;
        assert!(res.is_ok(), "concurrent race timed out (deadlock?) at seed={seed}\n{}", trace.join("\n"));
        assert_invariants(&h, seed, &trace, &BTreeMap::new(), &BTreeMap::new(), false, &gh);
        if let Some(race) = advance_race {
            let predicted = model.reconcile_racing_advance(&ops_gh, &race);
            pull_gh_mutations(&h, &mut gh);
            oracle.mutations(&predicted, &gh, true, attempts(&h), &trace, seed);
            oracle.check(&h, &mut model, sent_before, &trace, seed);
            if matches!(race, AdvanceRace::FailAndReplace { .. }) {
                sync_gh(&h, &gh);
            }
        } else if !ops.is_empty() {
            statuses.resize(ops.len(), None);
            let actual_gh = actual_gh.unwrap_or_else(|| {
                pull_gh_mutations(&h, &mut gh);
                gh.clone()
            });
            resolve_raced(&h, &mut model, &ops, &statuses, &ops_gh, &actual_gh, sent_before, &trace, seed);
        }
    }

    h.github.set_delay(Duration::ZERO);
    let mut settled = false;
    for _ in 0..4 {
        let before = h.snapshot();
        let sent_before = h.sent().len();
        model.step(&clock_now(&h));
        let rec = h.app.wf.reconcile().await;
        oracle_assert!(rec.is_ok(), "final reconcile failed: {rec:?}\nseed={seed}\n{}", trace.join("\n"));
        let predicted = model.reconcile(&gh);
        pull_gh_mutations(&h, &mut gh);
        oracle.mutations(&predicted, &gh, rec.is_ok(), attempts(&h), &trace, seed);
        oracle.check(&h, &mut model, sent_before, &trace, seed);
        if h.snapshot() == before {
            settled = true;
            break;
        }
    }
    oracle_assert!(settled, "reconcile did not reach a fixpoint in 4 passes\nseed={seed}\n{}", trace.join("\n"));
    assert_invariants(&h, seed, &trace, &BTreeMap::new(), &BTreeMap::new(), true, &gh);
    (trace, h)
}

#[tokio::test(start_paused = true)]
async fn concurrent_chaos_races_never_deadlock_or_violate_invariants() {
    let range = chaos_seed_range(DEFAULT_CONCURRENT_SEEDS);
    for seed in range.seeds() {
        let _ = run_concurrent_seed(seed).await;
    }
}

#[cfg(feature = "test-sentinels")]
#[tokio::test(start_paused = true)]
async fn sentinel_acceptance() {
    // Baseline: Tier 1 chaos must pass cleanly with all sentinels off before we grade any sentinel.
    for seed in 0..DEFAULT_SEQ_SEEDS {
        let _ = run_sequential_seed_with(seed, None).await;
    }
    for seed in 0..DEFAULT_CONCURRENT_SEEDS {
        let _ = run_concurrent_seed_with(seed, None).await;
    }

    let mut caught: Vec<(Sentinel, String)> = Vec::new();
    let mut missed: Vec<Sentinel> = Vec::new();
    for &sentinel in Sentinel::ALL {
        let mut found: Option<String> = None;
        for seed in 0..DEFAULT_SEQ_SEEDS {
            if caught_by_oracle(run_sequential_seed_with(seed, Some(sentinel))).await {
                found = Some(format!("sequential seed {seed}"));
                break;
            }
        }
        if found.is_none() {
            for seed in 0..DEFAULT_CONCURRENT_SEEDS {
                if caught_by_oracle(run_concurrent_seed_with(seed, Some(sentinel))).await {
                    found = Some(format!("concurrent seed {seed}"));
                    break;
                }
            }
        }
        match found {
            Some(where_) => caught.push((sentinel, where_)),
            None => missed.push(sentinel),
        }
    }
    assert!(missed.is_empty(), "Tier 1 chaos missed planted sentinels: {missed:?}\ncaught: {caught:?}");
    eprintln!("sentinel_acceptance caught all {} sentinels:", caught.len());
    for (s, where_) in &caught {
        eprintln!("  {:<36} -> {where_}", s.name());
    }
}

#[cfg(feature = "test-sentinels")]
#[tokio::test]
async fn sentinel_acceptance_shrinks_sequential_failures() {
    let dir = tempfile::tempdir().unwrap();
    let mut shrunk_rows: Vec<(Sentinel, u64, usize, usize, String)> = Vec::new();
    let mut concurrent_only: Vec<Sentinel> = Vec::new();

    for &sentinel in Sentinel::ALL {
        let mut hit: Option<(u64, Vec<Op>, FailureClass, String)> = None;
        for seed in 0..DEFAULT_SEQ_SEEDS {
            let ops = decode_seed_ops(seed);
            let outcome = probe_sequential_ops(ops.clone(), Some(sentinel), format!("seed {seed}")).await;
            match &outcome {
                ProbeResult::Ok(_) => continue,
                ProbeResult::OracleFailure(msg) => {
                    let class = FailureClass::from_probe(&outcome).expect("oracle failure has class");
                    hit = Some((seed, ops, class, msg.clone()));
                    break;
                }
                ProbeResult::OtherPanic { message, location } => {
                    panic!(
                        "sentinel {} hit non-oracle panic on seed {seed} ({location:?}): {message}",
                        sentinel.name()
                    );
                }
            }
        }
        let Some((seed, orig_ops, class, failure_msg)) = hit else {
            concurrent_only.push(sentinel);
            continue;
        };

        let shrunk = shrink_sequential_failure(orig_ops, Some(sentinel), class.clone(), &failure_msg).await;
        assert!(
            shrunk.ops.len() <= 8,
            "sentinel {} shrunk to {} ops (> 8) from seed {seed}:\n{}",
            sentinel.name(),
            shrunk.ops.len(),
            format_regression_test(&shrunk, &format!("seed {seed}"))
        );

        // Verify 1-minimality directly: removing any single op must not reproduce the failure class.
        for skip in 0..shrunk.ops.len() {
            let mut sub = shrunk.ops.clone();
            sub.remove(skip);
            let res = probe_sequential_ops(sub, Some(sentinel), format!("minimality-{skip}")).await;
            assert!(
                !class.matches(&res),
                "sentinel {} shrunk sequence is not 1-minimal: dropping op {skip} still failed with {class:?}",
                sentinel.name()
            );
        }

        // Write through the CHAOS_SHRINK path and reload through the CHAOS_OPS loader.
        let source = format!("seed {seed} (sentinel {})", sentinel.name());
        let written = write_corpus_file(dir.path(), &shrunk, &source).unwrap();
        let (_, reloaded) = load_ops_file(&written).unwrap();
        assert_eq!(reloaded, shrunk.ops, "reloaded .ops diverged for sentinel {}", sentinel.name());

        // Deterministically replays the exact same failure twice with the sentinel enabled...
        let first = probe_sequential_ops(reloaded.clone(), Some(sentinel), "replay".into()).await;
        let second = probe_sequential_ops(reloaded.clone(), Some(sentinel), "replay".into()).await;
        assert!(class.matches(&first), "first replay of {} did not match {class:?}: {first:?}", sentinel.name());
        assert_eq!(first, second, "deterministic replay of {} diverged between runs", sentinel.name());

        // ...and passes cleanly on healthy code with all sentinels off.
        let healthy = probe_sequential_ops(reloaded, None, "healthy".into()).await;
        assert!(
            matches!(healthy, ProbeResult::Ok(_)),
            "shrunk replay for {} unexpectedly failed without sentinel: {healthy:?}",
            sentinel.name()
        );

        shrunk_rows.push((sentinel, seed, shrunk.original_len, shrunk.ops.len(), shrunk.class.signature));
    }

    assert_eq!(
        concurrent_only,
        vec![Sentinel::MergeWaitRemoveNotCas, Sentinel::AdvanceIgnoresSwapResult],
        "expected exactly the two mid-flight CAS sentinels to be concurrent-only, got {concurrent_only:?}"
    );
    assert_eq!(shrunk_rows.len(), 5, "expected all 5 sequential sentinels to shrink cleanly");
    eprintln!(
        "sentinel_acceptance_shrinks_sequential_failures shrunk {}/{} sentinels:",
        shrunk_rows.len(),
        Sentinel::ALL.len()
    );
    for (s, seed, orig, len, sig) in &shrunk_rows {
        eprintln!("  {:<36} seed={:<2}  {orig} -> {len} ops  ({sig})", s.name(), seed);
    }
    for s in &concurrent_only {
        eprintln!("  {:<36} concurrent-only (mid-flight CAS race; out of scope for sequential shrinker)", s.name());
    }
}

const WAKE_RECORDS: &[(&str, &str, &str, &str)] = &[
    ("m1", "mcp:t1", "event", "PR 1/3 opened"),
    ("m2", "mcp:t1", "event", "acme/widgets#8 merged on GitHub at 1234567"),
    ("m3", "mcp:w1", "reply", "A check failed"),
    ("m4", "mcp:w1", "reply", "Quoting it:\n[t3cm reply from=t3cm msg=m3]\nA check failed\n\n[t3cm note]"),
    ("m5", "mcp:w1", "reply", "abcdefghij"),
];

fn sample_wake_record(handle: &str) -> Option<Record> {
    WAKE_RECORDS.iter().find(|r| r.0 == handle).map(|r| Record { to: r.1.into(), kind: r.2.into(), body: r.3.into() })
}

fn misrouted_wake_sent() -> Vec<Value> {
    vec![json!({"threadId": "mcp:w2", "message": "[t3cm reply from=t3cm msg=m3]\nA check failed"})]
}

#[cfg(feature = "test-sentinels")]
#[tokio::test(start_paused = true)]
async fn sentinel_catcher_propagates_non_oracle_panics() {
    assert!(caught_by_oracle(async { oracle_assert!(false, "expected oracle check failure") }).await);
    assert!(
        caught_by_oracle(async {
            let sent = misrouted_wake_sent();
            let _ = delivered_headers(&sent, &sample_wake_record);
        })
        .await,
        "misrouted wake delivery must be counted as an oracle failure"
    );
    let err = tokio::spawn(caught_by_oracle(async {
        panic!("unrelated fixture unwrap");
    }))
    .await
    .unwrap_err();
    assert!(err.is_panic(), "non-oracle panic must propagate instead of counting as a sentinel catch");
}

#[test]
fn chaos_seed_range_parses_single_offset_and_count() {
    assert_eq!(parse_seed_range(60, None, None, None), Ok(SeedRange { start: 0, count: 60 }));
    assert_eq!(parse_seed_range(60, Some("42"), None, None), Ok(SeedRange { start: 42, count: 1 }));
    assert_eq!(parse_seed_range(60, Some("42"), Some("100"), Some("200")), Ok(SeedRange { start: 42, count: 1 }));
    assert_eq!(parse_seed_range(60, Some("  "), Some("100"), Some("25")), Ok(SeedRange { start: 100, count: 25 }));
    assert_eq!(parse_seed_range(60, None, Some("100"), None), Ok(SeedRange { start: 100, count: 60 }));
    assert_eq!(parse_seed_range(60, None, None, Some("10")), Ok(SeedRange { start: 0, count: 10 }));
    assert_eq!(
        parse_seed_range(60, Some(&u64::MAX.to_string()), None, None).unwrap().seeds().collect::<Vec<_>>(),
        vec![u64::MAX]
    );
    assert_eq!(
        parse_seed_range(60, None, Some("10"), Some("3")).unwrap().seeds().collect::<Vec<_>>(),
        vec![10, 11, 12]
    );
    assert_eq!(
        parse_seed_range(60, None, Some("10"), Some("0")).unwrap().seeds().collect::<Vec<_>>(),
        Vec::<u64>::new()
    );
    assert!(parse_seed_range(60, Some("abc"), None, None).is_err());
    assert!(parse_seed_range(60, None, Some(&u64::MAX.to_string()), Some("2")).is_err());
}

#[test]
fn single_seed_replay_is_deterministic() {
    if has_explicit_seed_env() {
        return;
    }
    // Single blocking thread keeps concurrent `spawn_blocking` slot reads FIFO on replay.
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .start_paused(true)
        .max_blocking_threads(1)
        .build()
        .unwrap();
    rt.block_on(async {
        for seed in [0, 7, 42] {
            let seq1 = run_sequential_seed(seed).await;
            let seq2 = run_sequential_seed(seed).await;
            assert_eq!(seq1, seq2, "sequential replay diverged on seed={seed}");

            let (t1, h1) = run_concurrent_seed(seed).await;
            let (t2, h2) = run_concurrent_seed(seed).await;
            assert_eq!(t1, t2, "concurrent trace diverged on seed={seed}");
            assert_eq!(
                h1.app.wf.reach().snapshot(),
                h2.app.wf.reach().snapshot(),
                "concurrent reach diverged on seed={seed}"
            );
            assert_eq!(h1.github.operations(), h2.github.operations(), "concurrent gh ops diverged on seed={seed}");
            assert_eq!(sent_counts(&h1), sent_counts(&h2), "concurrent sends diverged on seed={seed}");
        }
    });
}

#[test]
fn oracle_parses_wake_headers_and_github_subjects() {
    let sent = vec![
        json!({"threadId": "mcp:t1", "message": "[t3cm event from=\"#41 PR 1/3\" msg=m1]\nPR 1/3 opened\n\n[t3cm event from=github msg=m2]\nacme/widgets#8 merged on GitHub at 1234567\n\nNotes since your last wake:\n- [t3cm note] ignored"}),
        json!({"threadId": "mcp:w1", "message": "[t3cm reply from=t3cm msg=m3]\nA check failed"}),
        json!({"threadId": "mcp:w1", "message": "[t3cm reply from=parent msg=m4]\nQuoting it:\n[t3cm reply from=t3cm msg=m3]\nA check failed\n\n[t3cm note]"}),
        json!({"threadId": "mcp:w1", "message": "[t3cm reply from=t3cm msg=m5]\nabcd\n... (6 more chars; full: wf.show(\"m5\"))"}),
    ];
    assert_eq!(
        delivered_headers(&sent, &sample_wake_record),
        vec![
            ("mcp:t1".into(), "event".into(), "#41 PR 1/3".into()),
            ("mcp:t1".into(), "event".into(), "github".into()),
            ("mcp:w1".into(), "reply".into(), "t3cm".into()),
            ("mcp:w1".into(), "reply".into(), "parent".into()),
            ("mcp:w1".into(), "reply".into(), "t3cm".into()),
        ]
    );
    assert_eq!(github_events(wake_lines(&sent, &sample_wake_record)), vec![("mcp:t1".to_string(), 8)]);
    let misrouted = misrouted_wake_sent();
    let caught = std::panic::catch_unwind(|| delivered_headers(&misrouted, &sample_wake_record));
    assert!(caught.is_err(), "a delivery to the wrong recipient must fail, not be filtered out");
}

#[tokio::test]
async fn memory_store_matches_wal_store_on_sequential_replay() {
    let mem_h = H::new_memory();
    assert_eq!(mem_h.app.store.path(), std::path::Path::new(":memory:"));
    assert!(!mem_h.dir.path().join("store.db").exists());

    let wal_h = H::new();
    assert!(wal_h.dir.path().join("store.db").exists());

    for seed in [0, 7] {
        let mem = run_sequential_seed_inner(seed, None, true).await;
        let wal = run_sequential_seed_inner(seed, None, false).await;
        assert_eq!(mem, wal, "in-memory and WAL sequential chaos diverged on seed={seed}");
    }
}

#[test]
fn memory_harness_rejects_reopen_and_peer() {
    let mut h = H::new_memory();
    let reopen = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| h.reopen()));
    assert!(reopen.is_err(), "H::reopen must reject an in-memory store");
    let peer = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = h.peer();
    }));
    assert!(peer.is_err(), "H::peer must reject an in-memory store");
}

#[test]
fn op_serialization_round_trips_and_decodes_edge_cases() {
    let all_variants = vec![
        Op::PrOpened { worker: WSym(0), short_sha: true, no_ci: false },
        Op::PrUpdated { worker: WSym(1), short_sha: false },
        Op::PrReady { worker: WSym(2), short_sha: true },
        Op::GhPush { pr: PrSym(0) },
        Op::GhStatus { pr: PrSym(1), checks: GhChecks::Pending, merge_state: GhMergeState::Unknown, draft: true },
        Op::GhFinish { pr: PrSym(2), state: GhEndState::Merged, predates: true },
        Op::GhReadError { pr: PrSym(0), fail: true },
        Op::GhRemovePr { pr: PrSym(1) },
        Op::GhClearReadErrors,
        Op::Reconcile,
        Op::Adopt { to: TSym(2), from: TSym(0) },
        Op::TicketGateClear { ticket: TSym(0) },
        Op::TicketSendGate { ticket: TSym(1) },
        Op::TicketGateOpen { ticket: TSym(2) },
        Op::AnswerCard { card: CardSym { kind: CardKind::Relayed, nth: 0 } },
        Op::DismissCard { card: CardSym { kind: CardKind::Any, nth: 1 } },
        Op::AnswerReadyAndClearGate { nth: 0 },
        Op::WorkerReviewGate { worker: WSym(0) },
        Op::WorkerClose { worker: WSym(1) },
        Op::WorkerHandoff { worker: WSym(2) },
        Op::AdvanceTick { minutes: 35 },
        Op::MergeWaitPut { pr: PrSym(0), ticket: TSym(1), reassigned_id: true },
        Op::GhQueueConfig { pr: PrSym(1), enabled: true, queued: false },
        Op::GhQueueDrop { pr: PrSym(2), reason: QueueDropReason::MergeConflict, predates: false },
        Op::FakeSetPrs { worker: WSym(0), pr: PrSym(2) },
        Op::WorkerSetBusy { worker: WSym(1), busy: true },
        Op::ParentReply { worker: WSym(2) },
        Op::TicketClose { ticket: TSym(0), by_board: true },
        Op::TicketArchive { ticket: TSym(1), tick: true },
        Op::TicketThreadGone { ticket: TSym(2), tick: false },
        Op::WfReply { worker: WSym(0), by_label: true, reopen: true },
        Op::Pause { target: PauseTarget::WorkerByParent { worker: WSym(1) }, paused: true },
        Op::Sweep { target: SweepTarget::Ticket { ticket: TSym(0) } },
        Op::SetRelay { relay: false },
        Op::WorkerSetMergeOnly { worker: WSym(2), merge_only: true },
        Op::PrFollowup { worker: WSym(0), pr: PrSym(1), event: PrEventKind::Ready, short_sha: true, no_ci: false },
        Op::GhFilesPaginated { pr: PrSym(2), paginated: true },
        Op::GhReopen { pr: PrSym(0) },
    ];
    for (idx, &kind) in OpKind::ALL.iter().enumerate() {
        assert_eq!(kind as usize, idx, "OpKind::ALL order drifted at {kind:?}");
    }
    let seen_kinds: BTreeSet<OpKind> = all_variants.iter().map(Op::kind).collect();
    assert_eq!(seen_kinds.len(), OpKind::COUNT, "every OpKind must be covered in the round-trip test");

    let encoded = format!("# header comment\n\n{}\n# trailing comment\n", serialize_ops(&all_variants));
    let decoded = deserialize_ops(&encoded).unwrap();
    assert_eq!(decoded, all_variants);
    assert!(deserialize_ops("{\"op\":\"unknown\"}\n").is_err());

    // Backward compatibility: legacy NDJSON lines from the initial 27-op schema still deserialize identically.
    let legacy_ndjson = concat!(
        "{\"op\":\"pr_opened\",\"worker\":0,\"short_sha\":true,\"no_ci\":false}\n",
        "{\"op\":\"pr_updated\",\"worker\":1,\"short_sha\":false}\n",
        "{\"op\":\"pr_ready\",\"worker\":2,\"short_sha\":true}\n",
        "{\"op\":\"gh_push\",\"pr\":0}\n",
        "{\"op\":\"gh_status\",\"pr\":1,\"checks\":\"pending\",\"merge_state\":\"unknown\",\"draft\":true}\n",
        "{\"op\":\"gh_finish\",\"pr\":2,\"state\":\"merged\",\"predates\":true}\n",
        "{\"op\":\"gh_read_error\",\"pr\":0,\"fail\":true}\n",
        "{\"op\":\"gh_remove_pr\",\"pr\":1}\n",
        "{\"op\":\"gh_clear_read_errors\"}\n",
        "{\"op\":\"reconcile\"}\n",
        "{\"op\":\"adopt\",\"to\":2,\"from\":0}\n",
        "{\"op\":\"ticket_gate_clear\",\"ticket\":0}\n",
        "{\"op\":\"ticket_send_gate\",\"ticket\":1}\n",
        "{\"op\":\"ticket_gate_open\",\"ticket\":2}\n",
        "{\"op\":\"answer_card\",\"card\":{\"kind\":\"relayed\",\"nth\":0}}\n",
        "{\"op\":\"dismiss_card\",\"card\":{\"kind\":\"any\",\"nth\":1}}\n",
        "{\"op\":\"answer_ready_and_clear_gate\",\"nth\":0}\n",
        "{\"op\":\"worker_review_gate\",\"worker\":0}\n",
        "{\"op\":\"worker_close\",\"worker\":1}\n",
        "{\"op\":\"worker_handoff\",\"worker\":2}\n",
        "{\"op\":\"advance_tick\",\"minutes\":35}\n",
        "{\"op\":\"merge_wait_put\",\"pr\":0,\"ticket\":1,\"reassigned_id\":true}\n",
        "{\"op\":\"gh_queue_config\",\"pr\":1,\"enabled\":true,\"queued\":false}\n",
        "{\"op\":\"gh_queue_drop\",\"pr\":2,\"reason\":\"merge_conflict\",\"predates\":false}\n",
        "{\"op\":\"fake_set_prs\",\"worker\":0,\"pr\":2}\n",
        "{\"op\":\"worker_set_busy\",\"worker\":1,\"busy\":true}\n",
        "{\"op\":\"parent_reply\",\"worker\":2}\n",
    );
    assert_eq!(deserialize_ops(legacy_ndjson).unwrap(), all_variants[..27]);

    // Edge cases for decode_ops: empty input, tiny input, all-zero weights, and single-kind weights.
    let mut empty = Unstructured::new(&[]);
    assert!(decode_ops(&mut empty, &OpWeights::UNIFORM).is_empty());

    let mut tiny = Unstructured::new(&[0xab]);
    let tiny_ops = decode_ops(&mut tiny, &OpWeights::UNIFORM);
    assert!(!tiny_ops.is_empty());

    let bytes = seed_bytes(11);
    let zero_weights = OpWeights { weights: [0; OpKind::COUNT], fault_pct: 0, max_ops: 20 };
    assert!(decode_ops(&mut Unstructured::new(&bytes), &zero_weights).is_empty());

    let mut only_reconcile = zero_weights;
    only_reconcile.set(OpKind::Reconcile, 1);
    let mut single_byte = Unstructured::new(&[0xab]);
    assert_eq!(decode_ops(&mut single_byte, &only_reconcile), vec![Op::Reconcile]);
    assert!(single_byte.is_empty(), "singleton profile must advance Unstructured input");
    let mut full_stream = Unstructured::new(&bytes);
    let rec_ops = decode_ops(&mut full_stream, &only_reconcile);
    assert_eq!(rec_ops, vec![Op::Reconcile; 20]);
    assert_eq!(full_stream.len(), bytes.len() - 20);

    // Decoded ops must never emit an op kind whose weight is zero.
    for seed in 0..32 {
        let profile = OpWeights::swarm(seed);
        let ops = decode_seed_ops(seed);
        assert_eq!(ops.len(), profile.max_ops);
        for op in &ops {
            assert!(
                profile.get(op.kind()) > 0,
                "seed={seed} decoded {:?} which had weight 0 in its Swarm profile",
                op.kind()
            );
        }
        assert_eq!(deserialize_ops(&serialize_ops(&ops)).unwrap(), ops);
    }
}

#[test]
fn swarm_profiles_zero_about_forty_percent_keep_invariants_and_vary_faults() {
    for &kind in OpKind::ALL {
        assert_eq!(OpWeights::UNIFORM.get(kind), 1, "UNIFORM must weight {kind:?} at 1");
    }

    let sample_seeds = 200u64;
    let mut total_zeroed = 0usize;
    let mut fault_pcts: BTreeSet<u8> = BTreeSet::new();
    let mut fault_weight_sums: BTreeSet<u32> = BTreeSet::new();

    for seed in 0..sample_seeds {
        let w = OpWeights::swarm(seed);
        assert_eq!(w.enabled_count() + w.zeroed_count(), OpKind::COUNT);
        assert!(w.get(OpKind::Reconcile) > 0, "seed={seed} disabled Reconcile");
        assert!(
            OpKind::ALL.iter().copied().any(|k| k.is_creating() && w.get(k) > 0),
            "seed={seed} disabled every creating op"
        );
        total_zeroed += w.zeroed_count();
        fault_pcts.insert(w.fault_pct);
        let fault_sum: u32 = OpKind::ALL.iter().copied().filter(|k| k.is_fault()).map(|k| u32::from(w.get(k))).sum();
        fault_weight_sums.insert(fault_sum);
    }

    let avg_zero_frac = total_zeroed as f64 / (sample_seeds as f64 * OpKind::COUNT as f64);
    assert!(
        (0.32..=0.48).contains(&avg_zero_frac),
        "expected Swarm profiles to zero ~40% of op kinds on average, got {:.1}%",
        avg_zero_frac * 100.0
    );
    assert!(fault_pcts.contains(&0), "expected some fault-free Swarm seeds, saw {fault_pcts:?}");
    assert!(fault_pcts.iter().any(|&p| p >= 50), "expected some high-fault Swarm seeds, saw {fault_pcts:?}");
    assert!(fault_weight_sums.len() >= 3, "expected varying fault weights across seeds, saw {fault_weight_sums:?}");

    // Profile selection stays outside the Unstructured byte stream: the same bytes decode
    // cleanly under both UNIFORM and a Swarm profile with distinct results.
    let bytes = seed_bytes(5);
    let uniform_ops = decode_ops(&mut Unstructured::new(&bytes), &OpWeights::UNIFORM);
    let swarm_ops = decode_ops(&mut Unstructured::new(&bytes), &OpWeights::swarm(5));
    assert_eq!(uniform_ops.len(), OpWeights::UNIFORM.max_ops);
    assert_eq!(swarm_ops.len(), OpWeights::UNIFORM.max_ops);
    assert_ne!(uniform_ops, swarm_ops);
}

#[tokio::test]
async fn op_prefix_and_subsequence_replay_holds_invariants() {
    // Empty program replays cleanly.
    let _ = run_sequential_ops(&[], None, true, "empty").await;

    // Out-of-range symbolic handles and post-close/post-remove targets become recorded no-ops.
    let out_of_range = vec![
        Op::PrOpened { worker: WSym(99), short_sha: true, no_ci: false },
        Op::PrUpdated { worker: WSym(50), short_sha: false },
        Op::PrReady { worker: WSym(42), short_sha: true },
        Op::GhPush { pr: PrSym(99) },
        Op::GhStatus { pr: PrSym(7), checks: GhChecks::Failing, merge_state: GhMergeState::Dirty, draft: true },
        Op::GhFinish { pr: PrSym(8), state: GhEndState::Closed, predates: false },
        Op::GhReadError { pr: PrSym(9), fail: true },
        Op::GhRemovePr { pr: PrSym(10) },
        Op::GhClearReadErrors,
        Op::Adopt { to: TSym(0), from: TSym(0) },
        Op::Adopt { to: TSym(99), from: TSym(0) },
        Op::TicketGateClear { ticket: TSym(99) },
        Op::TicketSendGate { ticket: TSym(99) },
        Op::TicketGateOpen { ticket: TSym(99) },
        Op::AnswerCard { card: CardSym { kind: CardKind::Any, nth: 0 } },
        Op::DismissCard { card: CardSym { kind: CardKind::ReadyMerge, nth: 5 } },
        Op::AnswerReadyAndClearGate { nth: 3 },
        Op::WorkerReviewGate { worker: WSym(99) },
        Op::WorkerClose { worker: WSym(0) },
        Op::WorkerClose { worker: WSym(0) },               // already closed -> noop
        Op::PrReady { worker: WSym(0), short_sha: false }, // closed worker -> noop
        Op::WorkerClose { worker: WSym(1) },
        Op::WorkerHandoff { worker: WSym(2) }, // last active worker -> noop
        Op::MergeWaitPut { pr: PrSym(99), ticket: TSym(0), reassigned_id: false },
        Op::GhQueueConfig { pr: PrSym(99), enabled: true, queued: true },
        Op::GhQueueDrop { pr: PrSym(99), reason: QueueDropReason::FailedChecks, predates: false },
        Op::FakeSetPrs { worker: WSym(0), pr: PrSym(0) }, // closed worker -> noop
        Op::WorkerSetBusy { worker: WSym(0), busy: true }, // closed worker -> noop
        Op::ParentReply { worker: WSym(0) },              // closed worker -> noop
        Op::TicketClose { ticket: TSym(99), by_board: true },
        Op::TicketArchive { ticket: TSym(99), tick: true },
        Op::TicketThreadGone { ticket: TSym(99), tick: false },
        Op::WfReply { worker: WSym(99), by_label: true, reopen: true },
        Op::Pause { target: PauseTarget::Ticket { ticket: TSym(99) }, paused: true },
        Op::Pause { target: PauseTarget::WorkerSelf { worker: WSym(0) }, paused: true }, // closed worker -> noop
        Op::Sweep { target: SweepTarget::Ticket { ticket: TSym(99) } },
        Op::SetRelay { relay: true },                                 // already true -> noop
        Op::WorkerSetMergeOnly { worker: WSym(0), merge_only: true }, // closed worker -> noop
        Op::PrFollowup { worker: WSym(2), pr: PrSym(0), event: PrEventKind::Opened, short_sha: false, no_ci: false }, // no primary PR yet -> noop
        Op::GhFilesPaginated { pr: PrSym(99), paginated: true },
        Op::GhReopen { pr: PrSym(0) }, // already open -> noop
        Op::Reconcile,
    ];
    let (trace, _, _) = run_sequential_ops(&out_of_range, None, true, "out-of-range").await;
    let noop_count = trace.iter().filter(|line| line.contains(" noop (")).count();
    assert!(
        noop_count >= 37,
        "expected out-of-range and closed targets to record no-ops, got {noop_count}:\n{}",
        trace.join("\n")
    );

    // Partial per-PR read-error recovery leaves other failing PRs broken until cleared.
    let partial_err = vec![
        Op::PrReady { worker: WSym(0), short_sha: false },
        Op::PrReady { worker: WSym(1), short_sha: false },
        Op::GhReadError { pr: PrSym(0), fail: true },
        Op::GhReadError { pr: PrSym(1), fail: true },
        Op::GhReadError { pr: PrSym(0), fail: false },
        Op::Reconcile,
        Op::GhReadError { pr: PrSym(1), fail: false },
        Op::Reconcile,
    ];
    let (err_trace, _, _) = run_sequential_ops(&partial_err, None, true, "partial-read-err").await;
    assert!(
        err_trace.iter().any(|l| l.contains("reconcile -> ok=false"))
            && err_trace.iter().any(|l| l.contains("reconcile -> ok=true")),
        "expected partial per-PR read-error recovery to fail then succeed:\n{}",
        err_trace.join("\n")
    );

    // Every prefix and single-op deletion of a Swarm-decoded program replays cleanly.
    let swarm_ops = decode_seed_ops(3);
    for len in 1..=swarm_ops.len() {
        let _ = run_sequential_ops(&swarm_ops[..len], None, true, &format!("swarm3:prefix({len})")).await;
    }
    for skip in 0..swarm_ops.len() {
        let mut sub = swarm_ops.clone();
        sub.remove(skip);
        let _ = run_sequential_ops(&sub, None, true, &format!("swarm3:drop({skip})")).await;
    }

    // Strided and pseudorandom subsequences of both Swarm and UNIFORM programs replay cleanly.
    let uniform_ops = decode_ops(&mut Unstructured::new(&seed_bytes(7)), &OpWeights::UNIFORM);
    for (name, ops) in [("swarm0", decode_seed_ops(0)), ("swarm12", decode_seed_ops(12)), ("uniform7", uniform_ops)] {
        let evens: Vec<Op> = ops.iter().step_by(2).cloned().collect();
        let odds: Vec<Op> = ops.iter().skip(1).step_by(2).cloned().collect();
        let _ = run_sequential_ops(&evens, None, true, &format!("{name}:evens")).await;
        let _ = run_sequential_ops(&odds, None, true, &format!("{name}:odds")).await;
        let mut rng = Rng::new(0xdead_beef);
        for trial in 0..4 {
            let sub: Vec<Op> = ops.iter().filter(|_| rng.coin()).cloned().collect();
            let _ = run_sequential_ops(&sub, None, true, &format!("{name}:rand({trial})")).await;
        }
    }
}

#[tokio::test]
async fn moved_ready_card_under_non_merge_gate_reconciles_idempotently() {
    // Regression for #282: PR #7 moves while mcp:t1 holds delivery:verify, an open
    // ready merge card at the old head, and a MergeWait at that same old head.
    let five_ops = vec![
        Op::TicketSendGate { ticket: TSym(0) },
        Op::PrReady { worker: WSym(0), short_sha: false },
        Op::MergeWaitPut { pr: PrSym(0), ticket: TSym(0), reassigned_id: false },
        Op::GhPush { pr: PrSym(0) },
        Op::Reconcile,
    ];
    let (trace, _, _) = run_sequential_ops(&five_ops, None, true, "issue-282").await;
    assert!(trace.iter().any(|l| l.contains("reconcile -> ok=true")), "trace:\n{}", trace.join("\n"));

    let queued_ops = vec![
        Op::TicketSendGate { ticket: TSym(0) },
        Op::PrReady { worker: WSym(0), short_sha: false },
        Op::MergeWaitPut { pr: PrSym(0), ticket: TSym(0), reassigned_id: false },
        Op::GhPush { pr: PrSym(0) },
        Op::GhQueueConfig { pr: PrSym(0), enabled: true, queued: true },
        Op::Reconcile,
    ];
    let (queued_trace, _, _) = run_sequential_ops(&queued_ops, None, true, "issue-282-queued").await;
    assert!(
        queued_trace.iter().any(|l| l.contains("reconcile -> ok=true")),
        "queued trace:\n{}",
        queued_trace.join("\n")
    );
}

#[tokio::test]
async fn un_ticked_fake_set_prs_discovered_during_reconcile_settles_cleanly() {
    // Reproduces the harness flaw previously hit by seeds 500 (flush_to) and 1080 (retry_ready):
    // FakeSetPrs links a finished PR on FakeT3 without an intervening tick, and pass 1 of
    // Reconcile refreshes that worker's live cache via read_thread so a settling pass discovers it.
    let via_retry_ready = vec![
        Op::GhFinish { pr: PrSym(2), state: GhEndState::Closed, predates: false },
        Op::GhStatus { pr: PrSym(1), checks: GhChecks::Pending, merge_state: GhMergeState::Clean, draft: false },
        Op::PrReady { worker: WSym(1), short_sha: false },
        Op::GhStatus { pr: PrSym(1), checks: GhChecks::Passing, merge_state: GhMergeState::Clean, draft: false },
        Op::FakeSetPrs { worker: WSym(1), pr: PrSym(2) },
        Op::Reconcile,
    ];
    let (_, snap1, _) = run_sequential_ops(&via_retry_ready, None, true, "fake-set-prs-retry-ready").await;
    assert!(snap1.get(t3_codemode::reach::ReachPoint::ReadyWaitingRetried) >= 1);
    assert!(snap1.get(t3_codemode::reach::ReachPoint::FinishedEventReported) >= 1);

    let via_flush_to = vec![
        Op::GhFinish { pr: PrSym(0), state: GhEndState::Merged, predates: false },
        Op::PrReady { worker: WSym(2), short_sha: false },
        Op::GhFinish { pr: PrSym(2), state: GhEndState::Merged, predates: true },
        Op::FakeSetPrs { worker: WSym(2), pr: PrSym(0) },
        Op::Reconcile,
    ];
    let (_, snap2, _) = run_sequential_ops(&via_flush_to, None, true, "fake-set-prs-flush-to").await;
    assert!(snap2.get(t3_codemode::reach::ReachPoint::ReadyWaitingRetried) >= 1);
    assert!(snap2.get(t3_codemode::reach::ReachPoint::FinishedEventReported) >= 1);
}

#[tokio::test]
async fn widened_alphabet_ops_exercise_new_workflow_transitions() {
    let ops = vec![
        // Follow-up PRs, >100-file pagination, mergeOnly, relay: false, and PR reopen.
        Op::PrOpened { worker: WSym(0), short_sha: false, no_ci: false },
        Op::PrFollowup { worker: WSym(0), pr: PrSym(1), event: PrEventKind::Opened, short_sha: true, no_ci: false },
        Op::PrFollowup { worker: WSym(0), pr: PrSym(1), event: PrEventKind::Ready, short_sha: false, no_ci: false },
        Op::GhFilesPaginated { pr: PrSym(0), paginated: true },
        Op::PrReady { worker: WSym(0), short_sha: false },
        Op::GhFilesPaginated { pr: PrSym(0), paginated: false },
        Op::WorkerSetMergeOnly { worker: WSym(1), merge_only: true },
        Op::PrReady { worker: WSym(1), short_sha: false },
        Op::WorkerSetMergeOnly { worker: WSym(1), merge_only: false },
        Op::SetRelay { relay: false },
        Op::WorkerReviewGate { worker: WSym(0) },
        Op::PrReady { worker: WSym(1), short_sha: false },
        Op::SetRelay { relay: true },
        // PR close -> reconcile -> reopen -> reconcile -> close again.
        Op::FakeSetPrs { worker: WSym(2), pr: PrSym(2) },
        Op::AdvanceTick { minutes: 1 },
        Op::GhFinish { pr: PrSym(2), state: GhEndState::Closed, predates: false },
        Op::Reconcile,
        Op::GhReopen { pr: PrSym(2) },
        Op::Reconcile,
        // Pause suppresses tick nudges, sweep reports open children, wf.reply reopens closed worker.
        Op::Pause { target: PauseTarget::WorkerByParent { worker: WSym(2) }, paused: true },
        Op::Pause { target: PauseTarget::BoardTicket { ticket: TSym(2) }, paused: true },
        Op::AdvanceTick { minutes: 60 },
        Op::Sweep { target: SweepTarget::Ticket { ticket: TSym(0) } },
        Op::WorkerSetBusy { worker: WSym(0), busy: true },
        Op::WorkerClose { worker: WSym(0) },
        Op::WfReply { worker: WSym(0), by_label: true, reopen: false },
        Op::WfReply { worker: WSym(0), by_label: true, reopen: true },
        Op::AdvanceTick { minutes: 35 },
        // Ticket close, archive, and thread-gone (leaving at least one live ticket).
        Op::TicketClose { ticket: TSym(0), by_board: true },
        Op::Adopt { to: TSym(2), from: TSym(0) },
        Op::Sweep { target: SweepTarget::Ticket { ticket: TSym(2) } },
        Op::TicketArchive { ticket: TSym(1), tick: true },
    ];
    let (_, reach, _) = run_sequential_ops(&ops, None, true, "widened-alphabet-smoke").await;
    use t3_codemode::reach::ReachPoint;
    for p in [
        ReachPoint::TicketClosed,
        ReachPoint::TicketArchived,
        ReachPoint::ReplyReopenedClosed,
        ReachPoint::PausedLinkSkipped,
        ReachPoint::SweepExecuted,
        ReachPoint::RelayOffFallback,
        ReachPoint::MergeOnlyFallback,
        ReachPoint::PrFollowupRecorded,
        ReachPoint::PrFilesPaginatedFallback,
        ReachPoint::PrReopenedReconciled,
    ] {
        assert!(reach.get(p) >= 1, "expected smoke program to hit {}:\n{reach}", p.name());
    }

    let gone_ops = vec![
        Op::TicketSendGate { ticket: TSym(0) },
        Op::TicketThreadGone { ticket: TSym(0), tick: true },
        Op::Sweep { target: SweepTarget::Ticket { ticket: TSym(0) } },
    ];
    let (_, gone_reach, _) = run_sequential_ops(&gone_ops, None, true, "widened-alphabet-gone").await;
    assert!(gone_reach.get(ReachPoint::TicketThreadGone) >= 1, "expected thread-gone:\n{gone_reach}");

    // Focused sweep check: an unjoined openChild reservation is reported and removed on the first sweep.
    let (h, _) = setup_chaos(true);
    h.wf(TICKETS[0], "join", json!({"parent": BOARD, "label": "widgets#41", "role": "ticket"})).await;
    h.wf(
        TICKETS[0],
        "openChild",
        json!({"role": "worker", "label": "#41 PR 4/4", "issue": "acme/widgets#41", "part": "4/4",
               "pr": {"format": "pr"}}),
    )
    .await;
    let first = h.wf(TICKETS[0], "sweep", json!({})).await;
    assert_eq!(first["dropped"], json!(["#41 PR 4/4"]));
    assert_eq!(first["parentOpen"], json!(true));
    assert!(h.app.store.pending_all().unwrap().is_empty());
    let second = h.wf(TICKETS[0], "sweep", json!({})).await;
    assert_eq!(second["dropped"], json!([]));
}

#[tokio::test]
async fn sent_wait_retired_after_exit_window_reconciles_idempotently() {
    // Regression for #286: PR #8 has a Sent MergeWait, leaves the merge queue without a
    // removal reason, passes EXIT_WINDOW_MINS, and moves head before the next reconcile.
    let seven_ops = vec![
        Op::MergeWaitPut { pr: PrSym(1), ticket: TSym(0), reassigned_id: false },
        Op::Reconcile,
        Op::GhQueueConfig { pr: PrSym(1), enabled: false, queued: false },
        Op::Reconcile,
        Op::AdvanceTick { minutes: 35 },
        Op::GhPush { pr: PrSym(1) },
        Op::Reconcile,
    ];
    let (trace, _, _) = run_sequential_ops(&seven_ops, None, true, "issue-286").await;
    assert_eq!(trace.iter().filter(|l| l.contains("reconcile -> ok=true")).count(), 3, "trace:\n{}", trace.join("\n"));

    // Wf::failed retiring a Sent wait past EXIT_WINDOW_MINS while a second live ticket
    // tracks PR #8: pass 1 retires the wait, reports to the second ticket, and clears left.
    let failed_two_tickets = vec![
        Op::FakeSetPrs { worker: WSym(2), pr: PrSym(1) },
        Op::AdvanceTick { minutes: 1 },
        Op::MergeWaitPut { pr: PrSym(1), ticket: TSym(0), reassigned_id: false },
        Op::Reconcile,
        Op::GhQueueConfig { pr: PrSym(1), enabled: false, queued: false },
        Op::Reconcile,
        Op::AdvanceTick { minutes: 35 },
        Op::Reconcile,
    ];
    let (failed_trace, _, _) = run_sequential_ops(&failed_two_tickets, None, true, "issue-286-failed").await;
    assert_eq!(
        failed_trace.iter().filter(|l| l.contains("reconcile -> ok=true")).count(),
        3,
        "failed trace:\n{}",
        failed_trace.join("\n")
    );

    // Same Wf::failed two-ticket case where pass 2 of the final Reconcile hits MAX_FAILURES
    // (after an earlier read error on tracked PR #7 kept PR #8's failures at 1 instead of 2).
    let failed_on_pass2 = vec![
        Op::FakeSetPrs { worker: WSym(0), pr: PrSym(0) },
        Op::FakeSetPrs { worker: WSym(2), pr: PrSym(1) },
        Op::AdvanceTick { minutes: 1 },
        Op::MergeWaitPut { pr: PrSym(1), ticket: TSym(0), reassigned_id: false },
        Op::Reconcile,
        Op::GhQueueConfig { pr: PrSym(1), enabled: false, queued: false },
        Op::GhReadError { pr: PrSym(0), fail: true },
        Op::Reconcile,
        Op::GhClearReadErrors,
        Op::AdvanceTick { minutes: 35 },
        Op::Reconcile,
        Op::Reconcile,
    ];
    let (pass2_trace, _, _) = run_sequential_ops(&failed_on_pass2, None, true, "issue-286-failed-pass2").await;
    assert_eq!(
        pass2_trace.iter().filter(|l| l.contains("reconcile -> ok=false")).count(),
        1,
        "pass2 trace:\n{}",
        pass2_trace.join("\n")
    );
    assert_eq!(
        pass2_trace.iter().filter(|l| l.contains("reconcile -> ok=true")).count(),
        3,
        "pass2 trace:\n{}",
        pass2_trace.join("\n")
    );
}

#[tokio::test]
async fn ddmin_finds_one_minimal_subsequences_across_predicates() {
    // Empty sequence fails -> shrinks to empty.
    let empty_min = ddmin(vec![1, 2, 3], async |_: &[usize]| true).await;
    assert!(empty_min.is_empty());

    // Single required element inside 60 distractors.
    let items: Vec<usize> = (0..60).collect();
    let single = ddmin(items.clone(), async |cand: &[usize]| cand.contains(&41)).await;
    assert_eq!(single, vec![41]);

    // Order-dependent 4-element subsequence hidden in 70 items.
    let need = [3usize, 17, 44, 68];
    let items70: Vec<usize> = (0..70).collect();
    let ordered = ddmin(items70, async |cand: &[usize]| {
        let mut pos = 0;
        for &x in cand {
            if pos < need.len() && x == need[pos] {
                pos += 1;
            }
        }
        pos == need.len()
    })
    .await;
    assert_eq!(ordered, need);

    // Already minimal 2-element sequence stays intact.
    let minimal = ddmin(vec![10, 20], async |cand: &[usize]| cand.contains(&10) && cand.contains(&20)).await;
    assert_eq!(minimal, vec![10, 20]);
}

#[tokio::test]
async fn shrink_ops_simplifies_ticks_faults_and_pool_indices() {
    // Synthetic predicate requiring:
    // - some worker PrReady (index-agnostic, flag-agnostic)
    // - AdvanceTick with minutes >= 27
    // - Reconcile after AdvanceTick
    let noisy = vec![
        Op::GhReadError { pr: PrSym(2), fail: true },
        Op::PrReady { worker: WSym(2), short_sha: true },
        Op::GhClearReadErrors,
        Op::MergeWaitPut { pr: PrSym(2), ticket: TSym(2), reassigned_id: true },
        Op::AdvanceTick { minutes: 200 },
        Op::GhRemovePr { pr: PrSym(1) },
        Op::Reconcile,
    ];
    let class = FailureClass { kind: FailureKind::Oracle, signature: "synthetic".into(), location: None };
    let shrunk = shrink_ops(noisy, class, "synthetic failure", async |cand: &[Op]| {
        let mut saw_ready = false;
        let mut saw_tick = false;
        let mut saw_rec = false;
        for op in cand {
            match *op {
                Op::PrReady { .. } => saw_ready = true,
                Op::AdvanceTick { minutes } if saw_ready && minutes >= 27 => saw_tick = true,
                Op::Reconcile if saw_tick => saw_rec = true,
                _ => {}
            }
        }
        saw_rec
    })
    .await;

    assert_eq!(
        shrunk.ops,
        vec![Op::PrReady { worker: WSym(0), short_sha: false }, Op::AdvanceTick { minutes: 27 }, Op::Reconcile,]
    );

    // Coupled pool-index lowering: requires matching ticket across two ops and drops reassigned_id.
    let coupled = vec![
        Op::TicketSendGate { ticket: TSym(2) },
        Op::MergeWaitPut { pr: PrSym(2), ticket: TSym(2), reassigned_id: true },
    ];
    let class2 = FailureClass { kind: FailureKind::Oracle, signature: "coupled".into(), location: None };
    let shrunk2 = shrink_ops(coupled, class2, "coupled failure", async |cand: &[Op]| {
        matches!(cand, [Op::TicketSendGate { ticket: t1 }, Op::MergeWaitPut { ticket: t2, .. }] if t1 == t2)
    })
    .await;
    assert_eq!(
        shrunk2.ops,
        vec![
            Op::TicketSendGate { ticket: TSym(0) },
            Op::MergeWaitPut { pr: PrSym(0), ticket: TSym(0), reassigned_id: false },
        ]
    );

    // Out-of-range symbolic indices stay bounded in candidate generation (no OOM on usize::MAX).
    let huge_noop = vec![
        Op::PrReady { worker: WSym(usize::MAX), short_sha: false },
        Op::AnswerCard { card: CardSym { kind: CardKind::Any, nth: usize::MAX } },
        Op::AnswerReadyAndClearGate { nth: usize::MAX },
    ];
    let class3 = FailureClass { kind: FailureKind::Oracle, signature: "bounded".into(), location: None };
    let shrunk3 = shrink_ops(huge_noop, class3, "bounded", async |cand: &[Op]| cand.len() == 3).await;
    assert_eq!(
        shrunk3.ops,
        vec![
            Op::PrReady { worker: WSym(0), short_sha: false },
            Op::AnswerCard { card: CardSym { kind: CardKind::Any, nth: 0 } },
            Op::AnswerReadyAndClearGate { nth: 0 },
        ]
    );
}

#[tokio::test]
async fn failure_class_normalizes_entities_and_distinguishes_bug_classes() {
    let a = FailureClass::from_probe(&ProbeResult::<()>::OracleFailure(
        "closed worker mcp:w2 kept gate Some(\"review\")\nseed=2\ntrace:\n#0 foo".into(),
    ))
    .unwrap();
    let b = FailureClass::from_probe(&ProbeResult::<()>::OracleFailure(
        "closed worker mcp:w1 kept gate Some(\"merge\")\nseed=9\ntrace:\n#0 bar".into(),
    ))
    .unwrap();
    assert_eq!(a, b);

    let c = FailureClass::from_probe(&ProbeResult::<()>::OracleFailure(
        "answering a4 and clearing gate wiped unanswered ready card a1!\nseed=14".into(),
    ))
    .unwrap();
    let d = FailureClass::from_probe(&ProbeResult::<()>::OracleFailure(
        "answering a2 and clearing gate wiped unanswered ready card a3!\nseed=0".into(),
    ))
    .unwrap();
    assert_eq!(c, d);
    assert_ne!(a, c);

    let idem_msg = normalize_failure_signature(
        "assertion `left == right` failed: reconcile was not idempotent (sent extra messages)\nseed=62",
    );
    let idem_cards = normalize_failure_signature(
        "assertion `left == right` failed: reconcile was not idempotent (changed cards outside retrying waits {(\"acme/widgets\", 8)})\nseed=646",
    );
    assert_ne!(idem_msg, idem_cards);
    assert_eq!(idem_cards, "reconcile was not idempotent (changed cards)");

    let closed_ticket = normalize_failure_signature(
        "assertion `left == right` failed: closed ticket mcp:t1 received a message after closing! messages=[Object {\"threadId\": String(\"mcp:t1\")}]\nseed=1",
    );
    let closed_ticket_2 = normalize_failure_signature(
        "assertion `left == right` failed: closed ticket mcp:t3 received a message after closing! messages=[]\nseed=99",
    );
    assert_eq!(closed_ticket, closed_ticket_2);

    let row_key_1 = normalize_failure_signature(
        "merge wait row key (acme/widgets, 8) disagrees with its payload (acme/widgets, 7)",
    );
    let row_key_2 = normalize_failure_signature(
        "merge wait row key (acme/widgets, 7) disagrees with its payload (acme/widgets, 9)",
    );
    assert_eq!(row_key_1, row_key_2);
    assert_eq!(row_key_1, "merge wait row key disagrees with its payload");

    // Oracle failures and non-oracle panics never collide even with the same text.
    let oracle = FailureClass::from_probe(&ProbeResult::<()>::OracleFailure("boom".into())).unwrap();
    let panic =
        FailureClass::from_probe(&ProbeResult::<()>::OtherPanic { message: "boom".into(), location: None }).unwrap();
    assert_ne!(oracle, panic);
    assert_eq!(oracle.kind, FailureKind::Oracle);
    assert_eq!(panic.kind, FailureKind::Panic);

    // Identical Option::unwrap() payloads at different call sites produce distinct FailureClass values.
    let site_a1 = probe_oracle(async { std::hint::black_box(Option::<()>::None).unwrap() }).await;
    let site_b = probe_oracle(async { std::hint::black_box(Option::<()>::None).unwrap() }).await;
    let class_a1 = FailureClass::from_probe(&site_a1).unwrap();
    let class_b = FailureClass::from_probe(&site_b).unwrap();
    assert!(class_a1.location.is_some(), "expected panic location for site_a1: {class_a1:?}");
    assert!(class_b.location.is_some(), "expected panic location for site_b: {class_b:?}");
    assert_ne!(class_a1, class_b, "panics at different call sites must not share a failure class");

    async fn unwrap_at_one_site() {
        std::hint::black_box(Option::<()>::None).unwrap();
    }
    let same_1 = FailureClass::from_probe(&probe_oracle(unwrap_at_one_site()).await).unwrap();
    let same_2 = FailureClass::from_probe(&probe_oracle(unwrap_at_one_site()).await).unwrap();
    assert_eq!(same_1, same_2, "panics at the same call site must match");

    // Long diagnostics sharing a >180-char prefix stay distinct in FailureClass while failure_summary caps display.
    let shared_prefix = "context ".repeat(26);
    let long_a = format!("{shared_prefix}reason alpha");
    let long_b = format!("{shared_prefix}reason beta");
    assert_eq!(failure_summary(&long_a).chars().count(), 180);
    assert_eq!(failure_summary(&long_a), failure_summary(&long_b));
    let long_class_a = FailureClass::from_probe(&ProbeResult::<()>::OracleFailure(long_a)).unwrap();
    let long_class_b = FailureClass::from_probe(&ProbeResult::<()>::OracleFailure(long_b)).unwrap();
    assert_ne!(long_class_a, long_class_b, "signature must not truncate before comparison");
}

#[test]
fn shrink_formatting_hashing_and_env_parsing() {
    let ops = vec![
        Op::PrOpened { worker: WSym(0), short_sha: true, no_ci: false },
        Op::GhStatus { pr: PrSym(1), checks: GhChecks::Pending, merge_state: GhMergeState::Unknown, draft: true },
        Op::AnswerCard { card: CardSym { kind: CardKind::Relayed, nth: 0 } },
        Op::Pause { target: PauseTarget::WorkerByParent { worker: WSym(1) }, paused: true },
        Op::Sweep { target: SweepTarget::Ticket { ticket: TSym(0) } },
        Op::PrFollowup { worker: WSym(0), pr: PrSym(1), event: PrEventKind::Ready, short_sha: false, no_ci: false },
        Op::GhQueueDrop { pr: PrSym(2), reason: QueueDropReason::MergeConflict, predates: false },
        Op::GhFinish { pr: PrSym(2), state: GhEndState::Closed, predates: true },
        Op::Reconcile,
    ];
    for op in &ops {
        let rendered = format_op_rust(op);
        assert!(rendered.starts_with("Op::"), "rendered op must carry Op:: prefix: {rendered}");
    }
    assert!(format_op_rust(&ops[1]).contains("GhChecks::Pending"));
    assert!(format_op_rust(&ops[1]).contains("GhMergeState::Unknown"));
    assert!(format_op_rust(&ops[2]).contains("CardKind::Relayed"));
    assert!(format_op_rust(&ops[3]).contains("PauseTarget::WorkerByParent"));
    assert!(format_op_rust(&ops[4]).contains("SweepTarget::Ticket"));
    assert!(format_op_rust(&ops[5]).contains("PrEventKind::Ready"));
    assert!(format_op_rust(&ops[6]).contains("QueueDropReason::MergeConflict"));
    assert!(format_op_rust(&ops[7]).contains("GhEndState::Closed"));

    let hash = ops_content_hash(&ops);
    assert_eq!(hash.len(), 12);
    assert!(hash.bytes().all(|b| b.is_ascii_hexdigit()));

    // Comments do not change ops round-trip or hash.
    let file_text = format_ops_file(&ops, "seed 7", "cards diverged\nwith newline");
    assert!(file_text.starts_with("# source: seed 7\n# failure: cards diverged with newline\n"));
    assert_eq!(deserialize_ops(&file_text).unwrap(), ops);

    let dir = tempfile::tempdir().unwrap();
    let outcome = ShrinkOutcome {
        ops: ops.clone(),
        original_len: 70,
        evaluations: 42,
        hash: hash.clone(),
        class: FailureClass { kind: FailureKind::Oracle, signature: "cards diverged".into(), location: None },
        failure_summary: failure_summary("cards diverged from the shadow model\nseed=7"),
    };
    let written = write_corpus_file(dir.path(), &outcome, "seed 7").unwrap();
    assert_eq!(written.file_name().and_then(|s| s.to_str()), Some(format!("{hash}.ops").as_str()));
    let (_, loaded) = load_ops_file(&written).unwrap();
    assert_eq!(loaded, ops);
    assert_eq!(list_corpus_files(dir.path()).unwrap(), vec![written]);

    // Non-reproducing recheck must refuse to publish a corpus file.
    let transient_dir = tempfile::tempdir().unwrap();
    let non_repro = publish_verified_shrink(&outcome, &ProbeResult::Ok(()), "seed 7", transient_dir.path(), None);
    assert!(non_repro.is_err(), "passing recheck must not publish a corpus file");
    assert!(list_corpus_files(transient_dir.path()).unwrap().is_empty());
    let matching_recheck = ProbeResult::<()>::OracleFailure("cards diverged\nseed=recheck".into());
    let (repro_path, _) =
        publish_verified_shrink(&outcome, &matching_recheck, "seed 7", transient_dir.path(), None).unwrap();
    assert_eq!(list_corpus_files(transient_dir.path()).unwrap(), vec![repro_path]);

    let snippet = format_regression_test(&outcome, "seed 7");
    assert!(snippet.contains(&format!("async fn chaos_regression_{hash}()")));
    assert!(snippet.contains("run_sequential_ops(&ops, None, true"));

    assert!(!parse_env_flag(None));
    assert!(!parse_env_flag(Some("")));
    assert!(!parse_env_flag(Some("0")));
    assert!(!parse_env_flag(Some("false")));
    assert!(!parse_env_flag(Some("FALSE")));
    assert!(parse_env_flag(Some("1")));
    assert!(parse_env_flag(Some("true")));
    assert_eq!(parse_env_path(Some("  ")), None);
    assert_eq!(
        parse_env_path(Some("tests/chaos_corpus/abc.ops")),
        Some(std::path::PathBuf::from("tests/chaos_corpus/abc.ops"))
    );

    #[cfg(feature = "test-sentinels")]
    {
        assert_eq!(parse_chaos_sentinel(None), Ok(None));
        assert_eq!(parse_chaos_sentinel(Some("close_keeps_gate")), Ok(Some(Sentinel::CloseKeepsGate)));
        assert!(parse_chaos_sentinel(Some("not_a_sentinel")).is_err());
    }
}
