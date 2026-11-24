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

use ops::{
    CardKind, CardSym, GhChecks, GhEndState, GhMergeState, Op, OpKind, OpWeights, PrSym, QueueDropReason, TSym, WSym,
    decode_ops, deserialize_ops, seed_bytes, serialize_ops,
};
use shadow::{AdvanceRace, Model, Predicted, St, WaitM, wakes_of};

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
        pr["files"] = json!({"pageInfo": {"hasNextPage": false}, "nodes": [{"path": "src/lib.rs"}]});
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

fn assert_invariants(
    h: &H,
    seed: impl fmt::Display,
    trace: &[String],
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

    // 3. Worker link invariants (no closed-worker wakes, no stuck waits after healthy reconcile)
    let counts = sent_counts(h);
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
    for (to, kind, from) in h.delivered(sent) {
        oracle_assert!(
            snap.links.get(&to).is_none_or(|l| !l.closed),
            "forbidden wake: {kind} from {from} reached closed link {to}\n{}",
            ctx()
        );
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
    ["CHAOS_SEED", "CHAOS_SEED_START", "CHAOS_SEEDS"]
        .iter()
        .any(|k| std::env::var(k).ok().is_some_and(|v| !v.trim().is_empty()))
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

async fn run_sequential_seed(seed: u64) -> (Vec<String>, ReachSnapshot) {
    run_sequential_seed_with(seed, None).await
}

async fn run_sequential_seed_with(seed: u64, sentinel: Option<Sentinel>) -> (Vec<String>, ReachSnapshot) {
    // Sequential chaos has no reopen or peer ops, so it runs on the in-memory store.
    run_sequential_seed_inner(seed, sentinel, true).await
}

async fn run_sequential_seed_inner(
    seed: u64,
    sentinel: Option<Sentinel>,
    in_memory: bool,
) -> (Vec<String>, ReachSnapshot) {
    let ops = decode_seed_ops(seed);
    run_sequential_ops(&ops, sentinel, in_memory, &seed.to_string()).await
}

async fn run_sequential_ops(
    ops: &[Op],
    sentinel: Option<Sentinel>,
    in_memory: bool,
    run_id: &str,
) -> (Vec<String>, ReachSnapshot) {
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
    let mut closed_workers: BTreeMap<String, usize> = BTreeMap::new();
    let mut busy_workers: BTreeSet<&'static str> = BTreeSet::new();
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
                        return;
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
                }
                Op::PrUpdated { worker, short_sha } => {
                    let Some((w, pr)) = resolve_worker(worker, &closed_workers) else {
                        trace.push(format!("#{step} noop (pr updated {worker:?})"));
                        return;
                    };
                    let head = HEADS[gh[&pr].head_idx];
                    let sha = if short_sha { &head[..7] } else { head };
                    let r = h.wf(w, "pr", json!({"event": "updated", "repo": REPO, "pr": pr, "head": sha})).await;
                    trace.push(format!("#{step} {w} pr updated #{pr}@{sha} -> {}", r["status"]));
                    let want = model.pr_updated(w, pr, sha);
                    oracle.status(&r, &want, &trace, run_id);
                }
                Op::PrReady { worker, short_sha } => {
                    let Some((w, pr)) = resolve_worker(worker, &closed_workers) else {
                        trace.push(format!("#{step} noop (pr ready {worker:?})"));
                        return;
                    };
                    let head = HEADS[gh[&pr].head_idx];
                    let sha = if short_sha { &head[..7] } else { head };
                    let r = h.wf(w, "pr", json!({"event": "ready", "repo": REPO, "pr": pr, "head": sha})).await;
                    trace.push(format!("#{step} {w} pr ready #{pr}@{sha} -> {}", r["status"]));
                    let want = model.pr_ready(w, pr, sha, &gh);
                    oracle.status(&r, &want, &trace, run_id);
                }
                Op::GhPush { pr } => {
                    let Some(pr_num) = resolve_pr(pr) else {
                        trace.push(format!("#{step} noop (gh push {pr:?})"));
                        return;
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
                }
                Op::GhStatus { pr, checks, merge_state, draft } => {
                    let Some(pr_num) = resolve_pr(pr) else {
                        trace.push(format!("#{step} noop (gh status {pr:?})"));
                        return;
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
                }
                Op::GhFinish { pr, state, predates } => {
                    let Some(pr_num) = resolve_pr(pr) else {
                        trace.push(format!("#{step} noop (gh finish {pr:?})"));
                        return;
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
                }
                Op::GhReadError { pr, fail } => {
                    let Some(pr_num) = resolve_pr(pr) else {
                        trace.push(format!("#{step} noop (gh read_err {pr:?} fail={fail})"));
                        return;
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
                }
                Op::GhRemovePr { pr } => {
                    let Some(pr_num) = resolve_pr(pr) else {
                        trace.push(format!("#{step} noop (gh remove {pr:?})"));
                        return;
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
                    } else {
                        sync_gh(&h, &gh);
                        trace.push(format!("#{step} clear read_err {cleared:?}"));
                    }
                }
                Op::Reconcile => {
                    let had_err = gh.values().any(|m| m.read_err);
                    let mutations_before = advance_mutations(&h);
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
                                let reported = actions
                                    .iter()
                                    .any(|a| a.starts_with(&format!("{label} dropped from the merge queue")));
                                if at.as_str() < after {
                                    let event = format!("removed:{at}");
                                    let marked = h.app.store.pr_event_seen(REPO, number, &event, BOARD).unwrap()
                                        || seen
                                            .tickets
                                            .iter()
                                            .any(|(t, _)| h.app.store.pr_event_seen(REPO, number, &event, t).unwrap());
                                    oracle_assert!(
                                        !reported && !marked,
                                        "stale queue drop {label} at {at} (< after {after}) was reported or marked: \
                                         actions={actions:?}\nseed={run_id}\n{}",
                                        trace.join("\n")
                                    );
                                }
                            }
                        }
                        if advance_mutations(&h) != mutations_before {
                            let settle = h.app.wf.reconcile().await;
                            let predicted = model.reconcile(&gh);
                            pull_gh_mutations(&h, &mut gh);
                            oracle.mutations(&predicted, &gh, settle.is_ok(), attempts(&h), &trace, run_id);
                            oracle_assert!(
                                settle.is_ok(),
                                "settling reconcile after advance failed: {settle:?}\nseed={run_id}\n{}",
                                trace.join("\n")
                            );
                        }
                        let retrying_prs: BTreeSet<(String, u64)> = h
                            .app
                            .store
                            .merge_waits()
                            .unwrap()
                            .into_iter()
                            .filter(|w| w.failures > 0)
                            .map(|w| (w.repo, w.pr))
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
                        let after_cards: Vec<String> = h
                            .app
                            .wf
                            .cards()
                            .unwrap()
                            .into_iter()
                            .filter(|c| {
                                before_set.contains(&c.id)
                                    || !(c.kind == "report"
                                        && c.text.contains("GitHub would not ")
                                        && c.pr
                                            .as_ref()
                                            .is_some_and(|p| retrying_prs.contains(&(p.repo.clone(), p.number))))
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
                }
                Op::Adopt { to, from } => {
                    let (Some(to_t), Some(from_t)) = (resolve_ticket(to), resolve_ticket(from)) else {
                        trace.push(format!("#{step} noop (adopt {to:?} from {from:?})"));
                        return;
                    };
                    if to_t == from_t {
                        trace.push(format!("#{step} noop (adopt self {to_t})"));
                        return;
                    }
                    let r = h.wf(to_t, "adopt", json!({"from": from_t})).await;
                    model.adopt(to_t, from_t);
                    trace.push(format!("#{step} {to_t} adopt from {from_t} -> {r}"));
                    oracle_assert!(
                        h.app.store.children(from_t, true).unwrap().is_empty(),
                        "adopt left open children on {from_t}\nseed={run_id}\n{}",
                        trace.join("\n")
                    );
                }
                Op::TicketGateClear { ticket } => {
                    let Some(t) = resolve_ticket(ticket) else {
                        trace.push(format!("#{step} noop (gate clear {ticket:?})"));
                        return;
                    };
                    let _ = h.wf(t, "gate", json!({"clear": true})).await;
                    model.ticket_gate_clear(t);
                    trace.push(format!("#{step} {t} gate clear"));
                }
                Op::TicketSendGate { ticket } => {
                    let Some(t) = resolve_ticket(ticket) else {
                        trace.push(format!("#{step} noop (send gate {ticket:?})"));
                        return;
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
                }
                Op::TicketGateOpen { ticket } => {
                    let Some(t) = resolve_ticket(ticket) else {
                        trace.push(format!("#{step} noop (gate open {ticket:?})"));
                        return;
                    };
                    let _ = h.wf(t, "gate", json!({"open": "merge"})).await;
                    model.ticket_gate_open(t, "merge");
                    trace.push(format!("#{step} {t} manual gate open merge"));
                }
                Op::AnswerCard { card } => {
                    let open = h.app.wf.cards().unwrap();
                    let Some((idx, c)) = resolve_card(&open, card) else {
                        trace.push(format!("#{step} noop (answer {card:?})"));
                        return;
                    };
                    let rank = model.card_ref(&before, idx);
                    let ans = c.options.first().map(String::as_str).unwrap_or("ok");
                    let _ = h.app.wf.answer(&c.id, ans).await;
                    model.answer(rank, c.ticket.as_deref(), ans, &c.options);
                    pull_gh_mutations(&h, &mut gh);
                    trace.push(format!("#{step} answer {} ({card:?}) with {ans:?}", c.id));
                }
                Op::DismissCard { card } => {
                    let open = h.app.wf.cards().unwrap();
                    let Some((idx, c)) = resolve_card(&open, card) else {
                        trace.push(format!("#{step} noop (dismiss {card:?})"));
                        return;
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
                }
                Op::AnswerReadyAndClearGate { nth } => {
                    // Answer a ready merge card and immediately clear the ticket's gate;
                    // every other unanswered ready merge card on that ticket must survive!
                    let open = h.app.wf.cards().unwrap();
                    let target = CardSym { kind: CardKind::ReadyMerge, nth };
                    let Some((idx, chosen)) = resolve_card(&open, target) else {
                        trace.push(format!("#{step} noop (answer_ready_and_clear nth={nth})"));
                        return;
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
                }
                Op::WorkerReviewGate { worker } => {
                    let Some((w, _)) = resolve_worker(worker, &closed_workers) else {
                        trace.push(format!("#{step} noop (review gate {worker:?})"));
                        return;
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
                }
                Op::WorkerClose { worker } => {
                    let Some((w, _)) = resolve_worker(worker, &closed_workers) else {
                        trace.push(format!("#{step} noop (close {worker:?})"));
                        return;
                    };
                    if WORKERS.len() - closed_workers.len() <= 1 {
                        trace.push(format!("#{step} noop (close last worker {w})"));
                        return;
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
                    let count = sent_counts(&h).get(w).copied().unwrap_or(0);
                    closed_workers.insert(w.to_string(), count);
                    trace.push(format!("#{step} {parent} close {w} (sent_at_close={count})"));
                }
                Op::WorkerHandoff { worker } => {
                    let Some((w, _)) = resolve_worker(worker, &closed_workers) else {
                        trace.push(format!("#{step} noop (handoff {worker:?})"));
                        return;
                    };
                    if WORKERS.len() - closed_workers.len() <= 1 {
                        trace.push(format!("#{step} noop (handoff last worker {w})"));
                        return;
                    }
                    h.wf(w, "send", json!({"kind": "handoff", "body": "chaos handoff"})).await;
                    model.handoff(w);
                    let count = sent_counts(&h).get(w).copied().unwrap_or(0);
                    closed_workers.insert(w.to_string(), count);
                    trace.push(format!("#{step} {w} handoff (sent_at_close={count})"));
                }
                Op::AdvanceTick { minutes } => {
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
                                        s["threadId"] == w
                                            && s["message"].as_str().is_some_and(|m| m.starts_with("[t3cm nudge"))
                                    })
                                    .count();
                                (w.to_string(), nudges)
                            })
                        })
                        .collect();
                    h.advance_tick(i64::from(minutes)).await;
                    model.step(&clock_now(&h));
                    model.tick();
                    oracle.loose = Some(model.tick_may());
                    trace.push(format!("#{step} advance_tick({minutes})"));
                    for (w, before) in waiting_before {
                        let after = h
                            .sent()
                            .into_iter()
                            .filter(|s| {
                                s["threadId"] == w.as_str()
                                    && s["message"].as_str().is_some_and(|m| m.starts_with("[t3cm nudge"))
                            })
                            .count();
                        oracle_assert_eq!(
                            after,
                            before,
                            "waiting worker {w} was nudged during advance_tick({minutes})\nseed={run_id}\n{}",
                            trace.join("\n")
                        );
                    }
                }
                Op::MergeWaitPut { pr, ticket, reassigned_id } => {
                    let (Some(pr_num), Some(t)) = (resolve_pr(pr), resolve_ticket(ticket)) else {
                        trace.push(format!("#{step} noop (merge_wait_put {pr:?} {ticket:?})"));
                        return;
                    };
                    let m = &gh[&pr_num];
                    if !m.exists || m.state != "OPEN" {
                        trace.push(format!("#{step} noop (merge_wait_put #{pr_num} closed/missing)"));
                        return;
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
                }
                Op::GhQueueConfig { pr, enabled, queued } => {
                    let Some(pr_num) = resolve_pr(pr) else {
                        trace.push(format!("#{step} noop (gh queue {pr:?})"));
                        return;
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
                }
                Op::GhQueueDrop { pr, reason, predates } => {
                    let Some(pr_num) = resolve_pr(pr) else {
                        trace.push(format!("#{step} noop (gh queue drop {pr:?})"));
                        return;
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
                }
                Op::FakeSetPrs { worker, pr } => {
                    let (Some((w, _)), Some(pr_num)) = (resolve_worker(worker, &closed_workers), resolve_pr(pr)) else {
                        trace.push(format!("#{step} noop (fake.set_prs {worker:?} {pr:?})"));
                        return;
                    };
                    h.fake.set_prs(w, vec![pull_request(REPO, pr_num, "open")]);
                    model.set_prs(w, pr_num);
                    trace.push(format!("#{step} fake.set_prs({w}, #{pr_num})"));
                }
                Op::WorkerSetBusy { worker, busy } => {
                    let Some((w, _)) = resolve_worker(worker, &closed_workers) else {
                        trace.push(format!("#{step} noop (set_busy {worker:?})"));
                        return;
                    };
                    if busy_workers.contains(w) == busy {
                        trace.push(format!("#{step} noop ({w} already busy={busy})"));
                        return;
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
                }
                Op::ParentReply { worker } => {
                    let Some((w, _)) = resolve_worker(worker, &closed_workers) else {
                        trace.push(format!("#{step} noop (parent reply {worker:?})"));
                        return;
                    };
                    let parent = h.app.store.link_get(w).unwrap().unwrap().parent;
                    let r = h.wf(&parent, "reply", json!({"to": w, "body": "chaos reply"})).await;
                    model.reply(w);
                    trace.push(format!("#{step} {parent} reply {w} -> {}", r["delivery"]));
                }
            }
        })
        .await;
        assert!(res.is_ok(), "step timed out (possible deadlock) at seed={run_id}\n{}", trace.join("\n"));
        assert_invariants(&h, run_id, &trace, &closed_workers, healthy_reconcile, &gh);
        oracle.check(&h, &mut model, sent_before, &trace, run_id);
    }
    (trace, h.app.wf.reach().snapshot())
}

#[tokio::test]
async fn seeded_sequential_chaos_holds_all_invariants() {
    let range = chaos_seed_range(DEFAULT_SEQ_SEEDS);
    let mut reach = ReachSnapshot::default();
    for seed in range.seeds() {
        let (_, snap) = run_sequential_seed(seed).await;
        reach.merge(&snap);
    }
    if std::env::var_os("CHAOS_REACH").is_some() {
        eprintln!("{reach}");
    }
    if range.count >= DEFAULT_SEQ_SEEDS {
        reach.assert_tier1(&format!("seeded_sequential_chaos ({})", range.describe()));
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
        assert_invariants(&h, seed, &trace, &BTreeMap::new(), false, &gh);
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
    assert_invariants(&h, seed, &trace, &BTreeMap::new(), true, &gh);
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
        Op::Reconcile,
    ];
    let (trace, _) = run_sequential_ops(&out_of_range, None, true, "out-of-range").await;
    let noop_count = trace.iter().filter(|line| line.contains(" noop (")).count();
    assert!(
        noop_count >= 25,
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
    let (err_trace, _) = run_sequential_ops(&partial_err, None, true, "partial-read-err").await;
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
