use std::collections::HashMap;
use std::fmt::Write as _;
use std::ops::AsyncFnMut;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use super::ops::{
    CardKind, CardSym, GhChecks, GhEndState, GhMergeState, Op, PauseTarget, PrSym, QueueDropReason, SweepTarget, TSym,
    WSym, deserialize_ops, serialize_ops,
};
use super::{PRS, TICKETS, WORKERS};
use crate::common::ProbeResult;

/// Upper bound on card selector indices tried during value simplification.
const MAX_CARD_NTH_CANDIDATES: usize = 8;

/// Whether a captured chaos failure came from an oracle assertion or another panic.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FailureKind {
    Oracle,
    Panic,
}

/// Normalized failure identity used to keep shrinking locked onto the same bug class.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FailureClass {
    pub kind: FailureKind,
    pub signature: String,
    pub location: Option<String>,
}

impl FailureClass {
    pub fn from_probe<T>(outcome: &ProbeResult<T>) -> Option<Self> {
        match outcome {
            ProbeResult::Ok(_) => None,
            ProbeResult::OracleFailure(msg) => {
                Some(Self { kind: FailureKind::Oracle, signature: normalize_failure_signature(msg), location: None })
            }
            ProbeResult::OtherPanic { message, location } => Some(Self {
                kind: FailureKind::Panic,
                signature: normalize_failure_signature(message),
                location: location.clone(),
            }),
        }
    }

    pub fn matches<T>(&self, outcome: &ProbeResult<T>) -> bool {
        Self::from_probe(outcome).as_ref() == Some(self)
    }
}

/// First-line human summary of a failure message, sanitized for `#` header comments.
pub fn failure_summary(msg: &str) -> String {
    let head = strip_trace_tail(msg);
    let raw = head.lines().find(|l| !l.trim().is_empty()).unwrap_or("unknown failure").trim();
    let line = raw.strip_prefix("assertion `left == right` failed: ").unwrap_or(raw);
    let clean = strip_debug_payload(line);
    let collapsed: String = clean.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() > 180 {
        let mut s: String = collapsed.chars().take(177).collect();
        s.push_str("...");
        s
    } else {
        collapsed
    }
}

fn strip_trace_tail(msg: &str) -> &str {
    let mut end = msg.len();
    for marker in ["\nseed=", "\ntrace:", "\n  left:", "\n  sut:"] {
        if let Some(pos) = msg.find(marker) {
            end = end.min(pos);
        }
    }
    &msg[..end]
}

fn strip_debug_payload(line: &str) -> &str {
    const CUT_BEFORE: &[&str] = &[
        " messages=[",
        ": card=",
        ": cards=",
        ": pr=",
        ": actions=",
        " (must=",
        " (max ",
        " outside retrying waits ",
        " at seed=",
    ];
    let mut s = line;
    for marker in CUT_BEFORE {
        if let Some(pos) = s.find(marker) {
            s = &s[..pos];
        }
    }
    // Trim trailing structured debug dumps like `: Object ...`, `: [Card...]`, `: Err(...)`, `: {...}`
    // while keeping prefixes like `forbidden wake: ...`.
    for sep in [": [", ": {", ": Err(", ": Ok(", ": Some(", ": Object ", ": String("] {
        if let Some(pos) = s.find(sep) {
            s = &s[..pos];
        }
    }
    if let Some(pos) = s.find(" kept gate_pr ") {
        s = &s[..pos + " kept gate_pr".len()];
    } else if let Some(pos) = s.find(" kept gate ") {
        s = &s[..pos + " kept gate".len()];
    } else if let Some(pos) = s.find(" has gate_pr=") {
        s = &s[..pos + " has gate_pr".len()];
    } else if let Some(pos) = s.find(" holds merge gate_pr ") {
        s = &s[..pos + " holds merge gate_pr".len()];
    } else if let Some(pos) = s.find(" duplicate open ready merge card for ") {
        s = &s[..pos + " duplicate open ready merge card".len()];
    } else if s.starts_with("duplicate open ready merge card for ") {
        s = "duplicate open ready merge card";
    } else if let Some(pos) = s.find(" unexpectedly dropped [") {
        s = &s[..pos + " unexpectedly dropped".len()];
    } else if s.starts_with("stale finished event ") && s.contains(" was reported") {
        s = "stale finished event was reported";
    } else if s.starts_with("stale queue drop ") && s.contains(" was reported or marked") {
        s = "stale queue drop was reported or marked";
    } else if s.starts_with("wake ") && s.contains(" is outside what the shadow model allows") {
        s = "wake is outside what the shadow model allows";
    } else if s.starts_with("expected flushed wake ") {
        s = "expected flushed wake missing";
    } else if s.starts_with("wf.pr status diverged from the shadow model:") {
        s = "wf.pr status diverged from the shadow model";
    } else if s.starts_with("second reconcile failed:") {
        s = "second reconcile failed";
    } else if s.starts_with("settling reconcile after advance/live refresh failed:") {
        s = "settling reconcile after advance/live refresh failed";
    } else if s.starts_with("dismiss_at rejected freshly read card ") {
        s = "dismiss_at rejected freshly read card";
    } else if s.starts_with("reconcile was not idempotent (changed cards") {
        s = "reconcile was not idempotent (changed cards)";
    } else if s.starts_with("forbidden wake: ") && s.contains(" reached closed link ") {
        s = "forbidden wake: reached closed link";
    } else if s.starts_with("forbidden wake: GitHub told ") {
        s = "forbidden wake: GitHub told link after it let go of it";
    } else if s.starts_with("pause response diverged for ") {
        s = "pause response diverged";
    } else if s.starts_with("close_ticket failed for ") {
        s = "close_ticket failed";
    } else if s.starts_with("ticket send closed failed for ") {
        s = "ticket send closed failed";
    } else if s.starts_with("wf.reply to closed ") {
        s = "wf.reply to closed worker gave unexpected error";
    } else if s.starts_with("reloading worker relay unexpectedly required restart:") {
        s = "reloading worker relay unexpectedly required restart";
    } else if s.starts_with("merge wait row key ") && s.contains(" disagrees with its payload ") {
        s = "merge wait row key disagrees with its payload";
    }
    s.trim()
}

/// Normalize variable tokens (entity indices, PR/card/message ids, tick deltas) so
/// shrinking and pool-index lowering stay locked on the same underlying bug class.
pub fn normalize_failure_signature(msg: &str) -> String {
    let summary = failure_summary(msg);
    let tokens: Vec<String> = summary.split_whitespace().map(normalize_token).collect();
    tokens.join(" ")
}

fn normalize_token(tok: &str) -> String {
    let (lead, core, trail) = split_punct(tok);
    let norm = if let Some(rest) = core.strip_prefix("mcp:t")
        && !rest.is_empty()
        && rest.bytes().all(|b| b.is_ascii_digit())
    {
        "mcp:tN".to_string()
    } else if let Some(rest) = core.strip_prefix("mcp:w")
        && !rest.is_empty()
        && rest.bytes().all(|b| b.is_ascii_digit())
    {
        "mcp:wN".to_string()
    } else if let Some(rest) = core.strip_prefix('#')
        && !rest.is_empty()
        && rest.bytes().all(|b| b.is_ascii_digit())
    {
        "#N".to_string()
    } else if let Some(rest) = core.strip_prefix("advance_tick(")
        && let Some(inner) = rest.strip_suffix(')')
        && !inner.is_empty()
        && inner.bytes().all(|b| b.is_ascii_digit())
    {
        "advance_tick(N)".to_string()
    } else if is_short_handle(core) {
        "<id>".to_string()
    } else if let Some(rest) = core.strip_prefix('x')
        && !rest.is_empty()
        && rest.bytes().all(|b| b.is_ascii_digit())
    {
        "xN".to_string()
    } else {
        core.to_string()
    };
    format!("{lead}{norm}{trail}")
}

fn split_punct(tok: &str) -> (&str, &str, &str) {
    let is_trim = |c: char| matches!(c, '!' | ',' | '.' | ';' | ':' | '"' | '\'' | '`');
    let start = tok.find(|c| !is_trim(c)).unwrap_or(tok.len());
    let end = tok.rfind(|c| !is_trim(c)).map_or(start, |i| i + tok[i..].chars().next().map_or(0, char::len_utf8));
    (&tok[..start], &tok[start..end], &tok[end..])
}

fn is_short_handle(s: &str) -> bool {
    let bytes = s.as_bytes();
    bytes.len() >= 2 && matches!(bytes[0], b'a' | b'b' | b'm') && bytes[1..].iter().all(u8::is_ascii_digit)
}

fn partition_bounds(len: usize, n: usize) -> impl Iterator<Item = (usize, usize)> {
    let base = len / n;
    let rem = len % n;
    (0..n).map(move |i| {
        let start = i * base + i.min(rem);
        let size = base + usize::from(i < rem);
        (start, start + size)
    })
}

/// Zeller & Hildebrandt's 1-minimal delta debugging (`ddmin`) algorithm.
///
/// Assumes `fails(&items)` is true. Returns a 1-minimal subsequence `c` of
/// `items` such that `fails(&c)` is true and no single-element deletion of `c`
/// satisfies `fails`.
pub async fn ddmin<T, F>(mut c: Vec<T>, mut fails: F) -> Vec<T>
where
    T: Clone,
    F: AsyncFnMut(&[T]) -> bool,
{
    if !c.is_empty() && fails(&[]).await {
        return Vec::new();
    }
    let mut n = 2usize;
    'outer: while c.len() >= 2 {
        let chunks: Vec<(usize, usize)> = partition_bounds(c.len(), n).collect();
        for &(start, end) in &chunks {
            let subset = c[start..end].to_vec();
            if fails(&subset).await {
                c = subset;
                n = 2;
                continue 'outer;
            }
        }
        if n > 2 {
            for &(start, end) in &chunks {
                let mut complement = Vec::with_capacity(c.len() - (end - start));
                complement.extend_from_slice(&c[..start]);
                complement.extend_from_slice(&c[end..]);
                if fails(&complement).await {
                    c = complement;
                    n = (n - 1).max(2);
                    continue 'outer;
                }
            }
        }
        if n < c.len() {
            n = (n * 2).min(c.len());
        } else {
            break;
        }
    }
    c
}

struct MemoOracle<F> {
    eval: F,
    cache: HashMap<Vec<Op>, bool>,
    evaluations: usize,
}

impl<F> MemoOracle<F>
where
    F: AsyncFnMut(&[Op]) -> bool,
{
    fn new(eval: F) -> Self {
        Self { eval, cache: HashMap::new(), evaluations: 0 }
    }

    async fn test(&mut self, candidate: &[Op]) -> bool {
        if let Some(&hit) = self.cache.get(candidate) {
            return hit;
        }
        self.evaluations += 1;
        let ok = (self.eval)(candidate).await;
        self.cache.insert(candidate.to_vec(), ok);
        ok
    }
}

/// Result of running `ddmin` plus value simplification on a failing op sequence.
#[derive(Clone, Debug)]
pub struct ShrinkOutcome {
    pub ops: Vec<Op>,
    pub original_len: usize,
    pub evaluations: usize,
    pub hash: String,
    pub class: FailureClass,
    pub failure_summary: String,
}

/// Shrink `ops` with 1-minimal `ddmin` followed by value simplification
/// (smaller tick deltas, drop redundant faults/flags, lower pool indices)
/// to a fixed point.
pub async fn shrink_ops<F>(ops: Vec<Op>, class: FailureClass, failure_msg: &str, eval: F) -> ShrinkOutcome
where
    F: AsyncFnMut(&[Op]) -> bool,
{
    let original_len = ops.len();
    let mut oracle = MemoOracle::new(eval);
    oracle.cache.insert(ops.clone(), true);

    let mut cur = ddmin(ops, async |cand| oracle.test(cand).await).await;
    for _ in 0..6 {
        let before = cur.clone();
        cur = simplify_ops_once(cur, &mut oracle).await;
        if cur != before {
            cur = ddmin(cur, async |cand| oracle.test(cand).await).await;
        }
        if cur == before {
            break;
        }
    }

    let hash = ops_content_hash(&cur);
    ShrinkOutcome {
        ops: cur,
        original_len,
        evaluations: oracle.evaluations,
        hash,
        class,
        failure_summary: failure_summary(failure_msg),
    }
}

async fn simplify_ops_once<F>(mut ops: Vec<Op>, oracle: &mut MemoOracle<F>) -> Vec<Op>
where
    F: AsyncFnMut(&[Op]) -> bool,
{
    // Binary-search smaller AdvanceTick deltas.
    for idx in 0..ops.len() {
        let Op::AdvanceTick { minutes } = ops[idx] else { continue };
        if minutes == 0 {
            continue;
        }
        let mut cand = ops.clone();
        cand[idx] = Op::AdvanceTick { minutes: 0 };
        if oracle.test(&cand).await {
            ops = cand;
            continue;
        }
        let mut lo = 0u16;
        let mut hi = minutes;
        while lo + 1 < hi {
            let mid = lo + (hi - lo) / 2;
            cand[idx] = Op::AdvanceTick { minutes: mid };
            if oracle.test(&cand).await {
                hi = mid;
            } else {
                lo = mid;
            }
        }
        if hi < minutes {
            ops[idx] = Op::AdvanceTick { minutes: hi };
        }
    }

    // Simplify per-op flags, faults, and card selectors.
    for idx in 0..ops.len() {
        loop {
            let mut improved = false;
            for replacement in simplify_op_fields(&ops[idx]) {
                let mut cand = ops.clone();
                cand[idx] = replacement;
                if oracle.test(&cand).await {
                    ops = cand;
                    improved = true;
                    break;
                }
            }
            if !improved {
                break;
            }
        }
    }

    // Lower pool indices consistently across the whole sequence, then per-op.
    ops = lower_pool_indices_globally(ops, oracle).await;
    for idx in 0..ops.len() {
        loop {
            let mut improved = false;
            for replacement in lower_op_indices(&ops[idx]) {
                let mut cand = ops.clone();
                cand[idx] = replacement;
                if oracle.test(&cand).await {
                    ops = cand;
                    improved = true;
                    break;
                }
            }
            if !improved {
                break;
            }
        }
    }

    ops
}

/// Candidate single-field simplifications for an `Op`, ordered from simplest/most-canonical first.
fn simplify_op_fields(op: &Op) -> Vec<Op> {
    let mut out = Vec::new();
    match *op {
        Op::PrOpened { worker, short_sha, no_ci } => {
            if short_sha {
                out.push(Op::PrOpened { worker, short_sha: false, no_ci });
            }
            if no_ci {
                out.push(Op::PrOpened { worker, short_sha, no_ci: false });
            }
        }
        Op::PrUpdated { worker, short_sha: true } => {
            out.push(Op::PrUpdated { worker, short_sha: false });
        }
        Op::PrReady { worker, short_sha: true } => {
            out.push(Op::PrReady { worker, short_sha: false });
        }
        Op::PrFollowup { worker, pr, event, short_sha, no_ci } => {
            if short_sha {
                out.push(Op::PrFollowup { worker, pr, event, short_sha: false, no_ci });
            }
            if no_ci {
                out.push(Op::PrFollowup { worker, pr, event, short_sha, no_ci: false });
            }
        }
        Op::GhStatus { pr, checks, merge_state, draft } => {
            if draft {
                out.push(Op::GhStatus { pr, checks, merge_state, draft: false });
            }
            if merge_state != GhMergeState::Clean {
                out.push(Op::GhStatus { pr, checks, merge_state: GhMergeState::Clean, draft });
            }
            if checks != GhChecks::Passing {
                out.push(Op::GhStatus { pr, checks: GhChecks::Passing, merge_state, draft });
            }
        }
        Op::GhFinish { pr, state, predates } => {
            if predates {
                out.push(Op::GhFinish { pr, state, predates: false });
            }
            if state != GhEndState::Merged {
                out.push(Op::GhFinish { pr, state: GhEndState::Merged, predates });
            }
        }
        Op::GhReadError { pr, fail: true } => {
            out.push(Op::GhReadError { pr, fail: false });
        }
        Op::MergeWaitPut { pr, ticket, reassigned_id: true } => {
            out.push(Op::MergeWaitPut { pr, ticket, reassigned_id: false });
        }
        Op::GhQueueConfig { pr, enabled, queued } => {
            if queued {
                out.push(Op::GhQueueConfig { pr, enabled, queued: false });
            }
            if enabled {
                out.push(Op::GhQueueConfig { pr, enabled: false, queued: false });
            }
        }
        Op::GhQueueDrop { pr, reason, predates } => {
            if predates {
                out.push(Op::GhQueueDrop { pr, reason, predates: false });
            }
            if reason != QueueDropReason::FailedChecks {
                out.push(Op::GhQueueDrop { pr, reason: QueueDropReason::FailedChecks, predates });
            }
        }
        Op::AnswerCard { card } => {
            let cap = card.nth.min(MAX_CARD_NTH_CANDIDATES);
            for nth in 0..cap {
                out.push(Op::AnswerCard { card: CardSym { kind: card.kind, nth } });
            }
            if card.kind != CardKind::Any {
                out.push(Op::AnswerCard { card: CardSym { kind: CardKind::Any, nth: 0 } });
                if card.nth > 0 && card.nth <= MAX_CARD_NTH_CANDIDATES {
                    out.push(Op::AnswerCard { card: CardSym { kind: CardKind::Any, nth: card.nth } });
                }
            }
        }
        Op::DismissCard { card } => {
            let cap = card.nth.min(MAX_CARD_NTH_CANDIDATES);
            for nth in 0..cap {
                out.push(Op::DismissCard { card: CardSym { kind: card.kind, nth } });
            }
            if card.kind != CardKind::Any {
                out.push(Op::DismissCard { card: CardSym { kind: CardKind::Any, nth: 0 } });
                if card.nth > 0 && card.nth <= MAX_CARD_NTH_CANDIDATES {
                    out.push(Op::DismissCard { card: CardSym { kind: CardKind::Any, nth: card.nth } });
                }
            }
        }
        Op::AnswerReadyAndClearGate { nth } => {
            for lower in 0..nth.min(MAX_CARD_NTH_CANDIDATES) {
                out.push(Op::AnswerReadyAndClearGate { nth: lower });
            }
        }
        Op::WorkerSetBusy { worker, busy: true } => {
            out.push(Op::WorkerSetBusy { worker, busy: false });
        }
        Op::TicketClose { ticket, by_board: true } => {
            out.push(Op::TicketClose { ticket, by_board: false });
        }
        Op::TicketArchive { ticket, tick: true } => {
            out.push(Op::TicketArchive { ticket, tick: false });
        }
        Op::TicketThreadGone { ticket, tick: true } => {
            out.push(Op::TicketThreadGone { ticket, tick: false });
        }
        Op::WfReply { worker, by_label, reopen } => {
            if by_label {
                out.push(Op::WfReply { worker, by_label: false, reopen });
            }
            if reopen {
                out.push(Op::WfReply { worker, by_label, reopen: false });
            }
        }
        Op::Pause { target, paused: true } => {
            out.push(Op::Pause { target, paused: false });
        }
        Op::SetRelay { relay: false } => {
            out.push(Op::SetRelay { relay: true });
        }
        Op::WorkerSetMergeOnly { worker, merge_only: true } => {
            out.push(Op::WorkerSetMergeOnly { worker, merge_only: false });
        }
        Op::GhFilesPaginated { pr, paginated: true } => {
            out.push(Op::GhFilesPaginated { pr, paginated: false });
        }
        _ => {}
    }
    out
}

fn lower_op_indices(op: &Op) -> Vec<Op> {
    let mut out = Vec::new();
    let max_t = TICKETS.len();
    let max_w = WORKERS.len();
    let max_p = PRS.len();
    match *op {
        Op::PrOpened { worker, short_sha, no_ci } => {
            for w in 0..worker.0.min(max_w) {
                out.push(Op::PrOpened { worker: WSym(w), short_sha, no_ci });
            }
        }
        Op::PrUpdated { worker, short_sha } => {
            for w in 0..worker.0.min(max_w) {
                out.push(Op::PrUpdated { worker: WSym(w), short_sha });
            }
        }
        Op::PrReady { worker, short_sha } => {
            for w in 0..worker.0.min(max_w) {
                out.push(Op::PrReady { worker: WSym(w), short_sha });
            }
        }
        Op::GhPush { pr } => {
            for p in 0..pr.0.min(max_p) {
                out.push(Op::GhPush { pr: PrSym(p) });
            }
        }
        Op::GhStatus { pr, checks, merge_state, draft } => {
            for p in 0..pr.0.min(max_p) {
                out.push(Op::GhStatus { pr: PrSym(p), checks, merge_state, draft });
            }
        }
        Op::GhFinish { pr, state, predates } => {
            for p in 0..pr.0.min(max_p) {
                out.push(Op::GhFinish { pr: PrSym(p), state, predates });
            }
        }
        Op::GhReadError { pr, fail } => {
            for p in 0..pr.0.min(max_p) {
                out.push(Op::GhReadError { pr: PrSym(p), fail });
            }
        }
        Op::GhRemovePr { pr } => {
            for p in 0..pr.0.min(max_p) {
                out.push(Op::GhRemovePr { pr: PrSym(p) });
            }
        }
        Op::Adopt { to, from } => {
            for t in 0..to.0.min(max_t) {
                if t != from.0 {
                    out.push(Op::Adopt { to: TSym(t), from });
                }
            }
            for f in 0..from.0.min(max_t) {
                if f != to.0 {
                    out.push(Op::Adopt { to, from: TSym(f) });
                }
            }
        }
        Op::TicketGateClear { ticket } => {
            for t in 0..ticket.0.min(max_t) {
                out.push(Op::TicketGateClear { ticket: TSym(t) });
            }
        }
        Op::TicketSendGate { ticket } => {
            for t in 0..ticket.0.min(max_t) {
                out.push(Op::TicketSendGate { ticket: TSym(t) });
            }
        }
        Op::TicketGateOpen { ticket } => {
            for t in 0..ticket.0.min(max_t) {
                out.push(Op::TicketGateOpen { ticket: TSym(t) });
            }
        }
        Op::WorkerReviewGate { worker } => {
            for w in 0..worker.0.min(max_w) {
                out.push(Op::WorkerReviewGate { worker: WSym(w) });
            }
        }
        Op::WorkerClose { worker } => {
            for w in 0..worker.0.min(max_w) {
                out.push(Op::WorkerClose { worker: WSym(w) });
            }
        }
        Op::WorkerHandoff { worker } => {
            for w in 0..worker.0.min(max_w) {
                out.push(Op::WorkerHandoff { worker: WSym(w) });
            }
        }
        Op::MergeWaitPut { pr, ticket, reassigned_id } => {
            for p in 0..pr.0.min(max_p) {
                out.push(Op::MergeWaitPut { pr: PrSym(p), ticket, reassigned_id });
            }
            for t in 0..ticket.0.min(max_t) {
                out.push(Op::MergeWaitPut { pr, ticket: TSym(t), reassigned_id });
            }
        }
        Op::GhQueueConfig { pr, enabled, queued } => {
            for p in 0..pr.0.min(max_p) {
                out.push(Op::GhQueueConfig { pr: PrSym(p), enabled, queued });
            }
        }
        Op::GhQueueDrop { pr, reason, predates } => {
            for p in 0..pr.0.min(max_p) {
                out.push(Op::GhQueueDrop { pr: PrSym(p), reason, predates });
            }
        }
        Op::FakeSetPrs { worker, pr } => {
            for w in 0..worker.0.min(max_w) {
                out.push(Op::FakeSetPrs { worker: WSym(w), pr });
            }
            for p in 0..pr.0.min(max_p) {
                out.push(Op::FakeSetPrs { worker, pr: PrSym(p) });
            }
        }
        Op::WorkerSetBusy { worker, busy } => {
            for w in 0..worker.0.min(max_w) {
                out.push(Op::WorkerSetBusy { worker: WSym(w), busy });
            }
        }
        Op::ParentReply { worker } => {
            for w in 0..worker.0.min(max_w) {
                out.push(Op::ParentReply { worker: WSym(w) });
            }
        }
        Op::TicketClose { ticket, by_board } => {
            for t in 0..ticket.0.min(max_t) {
                out.push(Op::TicketClose { ticket: TSym(t), by_board });
            }
        }
        Op::TicketArchive { ticket, tick } => {
            for t in 0..ticket.0.min(max_t) {
                out.push(Op::TicketArchive { ticket: TSym(t), tick });
            }
        }
        Op::TicketThreadGone { ticket, tick } => {
            for t in 0..ticket.0.min(max_t) {
                out.push(Op::TicketThreadGone { ticket: TSym(t), tick });
            }
        }
        Op::WfReply { worker, by_label, reopen } => {
            for w in 0..worker.0.min(max_w) {
                out.push(Op::WfReply { worker: WSym(w), by_label, reopen });
            }
        }
        Op::Pause { target, paused } => match target {
            PauseTarget::Ticket { ticket } => {
                for t in 0..ticket.0.min(max_t) {
                    out.push(Op::Pause { target: PauseTarget::Ticket { ticket: TSym(t) }, paused });
                }
            }
            PauseTarget::BoardTicket { ticket } => {
                for t in 0..ticket.0.min(max_t) {
                    out.push(Op::Pause { target: PauseTarget::BoardTicket { ticket: TSym(t) }, paused });
                }
            }
            PauseTarget::WorkerSelf { worker } => {
                for w in 0..worker.0.min(max_w) {
                    out.push(Op::Pause { target: PauseTarget::WorkerSelf { worker: WSym(w) }, paused });
                }
            }
            PauseTarget::WorkerByParent { worker } => {
                for w in 0..worker.0.min(max_w) {
                    out.push(Op::Pause { target: PauseTarget::WorkerByParent { worker: WSym(w) }, paused });
                }
            }
            PauseTarget::BoardWorker { worker } => {
                for w in 0..worker.0.min(max_w) {
                    out.push(Op::Pause { target: PauseTarget::BoardWorker { worker: WSym(w) }, paused });
                }
            }
        },
        Op::Sweep { target } => match target {
            SweepTarget::Ticket { ticket } => {
                for t in 0..ticket.0.min(max_t) {
                    out.push(Op::Sweep { target: SweepTarget::Ticket { ticket: TSym(t) } });
                }
            }
            SweepTarget::Worker { worker } => {
                for w in 0..worker.0.min(max_w) {
                    out.push(Op::Sweep { target: SweepTarget::Worker { worker: WSym(w) } });
                }
            }
        },
        Op::WorkerSetMergeOnly { worker, merge_only } => {
            for w in 0..worker.0.min(max_w) {
                out.push(Op::WorkerSetMergeOnly { worker: WSym(w), merge_only });
            }
        }
        Op::PrFollowup { worker, pr, event, short_sha, no_ci } => {
            for w in 0..worker.0.min(max_w) {
                out.push(Op::PrFollowup { worker: WSym(w), pr, event, short_sha, no_ci });
            }
            for p in 0..pr.0.min(max_p) {
                out.push(Op::PrFollowup { worker, pr: PrSym(p), event, short_sha, no_ci });
            }
        }
        Op::GhFilesPaginated { pr, paginated } => {
            for p in 0..pr.0.min(max_p) {
                out.push(Op::GhFilesPaginated { pr: PrSym(p), paginated });
            }
        }
        Op::GhReopen { pr } => {
            for p in 0..pr.0.min(max_p) {
                out.push(Op::GhReopen { pr: PrSym(p) });
            }
        }
        _ => {}
    }
    out
}

fn remap_index(idx: usize, hi: usize, lo: usize, swap: bool) -> usize {
    if idx == hi {
        lo
    } else if swap && idx == lo {
        hi
    } else {
        idx
    }
}

fn remap_ops(
    ops: &[Op],
    ticket: Option<(usize, usize, bool)>,
    worker: Option<(usize, usize, bool)>,
    pr: Option<(usize, usize, bool)>,
) -> Vec<Op> {
    let rt = |t: TSym| match ticket {
        Some((hi, lo, swap)) => TSym(remap_index(t.0, hi, lo, swap)),
        None => t,
    };
    let rw = |w: WSym| match worker {
        Some((hi, lo, swap)) => WSym(remap_index(w.0, hi, lo, swap)),
        None => w,
    };
    let rp = |p: PrSym| match pr {
        Some((hi, lo, swap)) => PrSym(remap_index(p.0, hi, lo, swap)),
        None => p,
    };
    ops.iter()
        .map(|op| match op {
            &Op::PrOpened { worker, short_sha, no_ci } => Op::PrOpened { worker: rw(worker), short_sha, no_ci },
            &Op::PrUpdated { worker, short_sha } => Op::PrUpdated { worker: rw(worker), short_sha },
            &Op::PrReady { worker, short_sha } => Op::PrReady { worker: rw(worker), short_sha },
            &Op::GhPush { pr } => Op::GhPush { pr: rp(pr) },
            &Op::GhStatus { pr, checks, merge_state, draft } => Op::GhStatus { pr: rp(pr), checks, merge_state, draft },
            &Op::GhFinish { pr, state, predates } => Op::GhFinish { pr: rp(pr), state, predates },
            &Op::GhReadError { pr, fail } => Op::GhReadError { pr: rp(pr), fail },
            &Op::GhRemovePr { pr } => Op::GhRemovePr { pr: rp(pr) },
            &Op::Adopt { to, from } => Op::Adopt { to: rt(to), from: rt(from) },
            &Op::TicketGateClear { ticket } => Op::TicketGateClear { ticket: rt(ticket) },
            &Op::TicketSendGate { ticket } => Op::TicketSendGate { ticket: rt(ticket) },
            &Op::TicketGateOpen { ticket } => Op::TicketGateOpen { ticket: rt(ticket) },
            &Op::WorkerReviewGate { worker } => Op::WorkerReviewGate { worker: rw(worker) },
            &Op::WorkerClose { worker } => Op::WorkerClose { worker: rw(worker) },
            &Op::WorkerHandoff { worker } => Op::WorkerHandoff { worker: rw(worker) },
            &Op::MergeWaitPut { pr, ticket, reassigned_id } => {
                Op::MergeWaitPut { pr: rp(pr), ticket: rt(ticket), reassigned_id }
            }
            &Op::GhQueueConfig { pr, enabled, queued } => Op::GhQueueConfig { pr: rp(pr), enabled, queued },
            &Op::GhQueueDrop { pr, reason, predates } => Op::GhQueueDrop { pr: rp(pr), reason, predates },
            &Op::FakeSetPrs { worker, pr } => Op::FakeSetPrs { worker: rw(worker), pr: rp(pr) },
            &Op::WorkerSetBusy { worker, busy } => Op::WorkerSetBusy { worker: rw(worker), busy },
            &Op::ParentReply { worker } => Op::ParentReply { worker: rw(worker) },
            &Op::TicketClose { ticket, by_board } => Op::TicketClose { ticket: rt(ticket), by_board },
            &Op::TicketArchive { ticket, tick } => Op::TicketArchive { ticket: rt(ticket), tick },
            &Op::TicketThreadGone { ticket, tick } => Op::TicketThreadGone { ticket: rt(ticket), tick },
            &Op::WfReply { worker, by_label, reopen } => Op::WfReply { worker: rw(worker), by_label, reopen },
            &Op::Pause { target, paused } => {
                let target = match target {
                    PauseTarget::Ticket { ticket } => PauseTarget::Ticket { ticket: rt(ticket) },
                    PauseTarget::WorkerSelf { worker } => PauseTarget::WorkerSelf { worker: rw(worker) },
                    PauseTarget::WorkerByParent { worker } => PauseTarget::WorkerByParent { worker: rw(worker) },
                    PauseTarget::BoardTicket { ticket } => PauseTarget::BoardTicket { ticket: rt(ticket) },
                    PauseTarget::BoardWorker { worker } => PauseTarget::BoardWorker { worker: rw(worker) },
                };
                Op::Pause { target, paused }
            }
            &Op::Sweep { target } => {
                let target = match target {
                    SweepTarget::Ticket { ticket } => SweepTarget::Ticket { ticket: rt(ticket) },
                    SweepTarget::Worker { worker } => SweepTarget::Worker { worker: rw(worker) },
                };
                Op::Sweep { target }
            }
            &Op::WorkerSetMergeOnly { worker, merge_only } => Op::WorkerSetMergeOnly { worker: rw(worker), merge_only },
            &Op::PrFollowup { worker, pr, event, short_sha, no_ci } => {
                Op::PrFollowup { worker: rw(worker), pr: rp(pr), event, short_sha, no_ci }
            }
            &Op::GhFilesPaginated { pr, paginated } => Op::GhFilesPaginated { pr: rp(pr), paginated },
            &Op::GhReopen { pr } => Op::GhReopen { pr: rp(pr) },
            other => other.clone(),
        })
        .collect()
}

struct FirstUse {
    tickets: Vec<Option<usize>>,
    workers: Vec<Option<usize>>,
    prs: Vec<Option<usize>>,
}

fn first_positions(ops: &[Op]) -> FirstUse {
    let mut t_pos = vec![None; TICKETS.len()];
    let mut w_pos = vec![None; WORKERS.len()];
    let mut p_pos = vec![None; PRS.len()];
    let mut rec_t = |t: TSym, i: usize| {
        if t.0 < t_pos.len() && t_pos[t.0].is_none() {
            t_pos[t.0] = Some(i);
        }
    };
    let mut rec_w = |w: WSym, i: usize| {
        if w.0 < w_pos.len() && w_pos[w.0].is_none() {
            w_pos[w.0] = Some(i);
        }
    };
    let mut rec_p = |p: PrSym, i: usize| {
        if p.0 < p_pos.len() && p_pos[p.0].is_none() {
            p_pos[p.0] = Some(i);
        }
    };
    for (i, op) in ops.iter().enumerate() {
        match *op {
            Op::PrOpened { worker, .. }
            | Op::PrUpdated { worker, .. }
            | Op::PrReady { worker, .. }
            | Op::WorkerReviewGate { worker }
            | Op::WorkerClose { worker }
            | Op::WorkerHandoff { worker }
            | Op::WorkerSetBusy { worker, .. }
            | Op::ParentReply { worker }
            | Op::WfReply { worker, .. }
            | Op::WorkerSetMergeOnly { worker, .. } => rec_w(worker, i),
            Op::GhPush { pr }
            | Op::GhStatus { pr, .. }
            | Op::GhFinish { pr, .. }
            | Op::GhReadError { pr, .. }
            | Op::GhRemovePr { pr }
            | Op::GhQueueConfig { pr, .. }
            | Op::GhQueueDrop { pr, .. }
            | Op::GhFilesPaginated { pr, .. }
            | Op::GhReopen { pr } => rec_p(pr, i),
            Op::Adopt { to, from } => {
                rec_t(to, i);
                rec_t(from, i);
            }
            Op::TicketGateClear { ticket }
            | Op::TicketSendGate { ticket }
            | Op::TicketGateOpen { ticket }
            | Op::TicketClose { ticket, .. }
            | Op::TicketArchive { ticket, .. }
            | Op::TicketThreadGone { ticket, .. } => rec_t(ticket, i),
            Op::MergeWaitPut { pr, ticket, .. } => {
                rec_p(pr, i);
                rec_t(ticket, i);
            }
            Op::FakeSetPrs { worker, pr } | Op::PrFollowup { worker, pr, .. } => {
                rec_w(worker, i);
                rec_p(pr, i);
            }
            Op::Pause { target, .. } => match target {
                PauseTarget::Ticket { ticket } | PauseTarget::BoardTicket { ticket } => rec_t(ticket, i),
                PauseTarget::WorkerSelf { worker }
                | PauseTarget::WorkerByParent { worker }
                | PauseTarget::BoardWorker { worker } => rec_w(worker, i),
            },
            Op::Sweep { target } => match target {
                SweepTarget::Ticket { ticket } => rec_t(ticket, i),
                SweepTarget::Worker { worker } => rec_w(worker, i),
            },
            _ => {}
        }
    }
    FirstUse { tickets: t_pos, workers: w_pos, prs: p_pos }
}

async fn lower_pool_indices_globally<F>(mut ops: Vec<Op>, oracle: &mut MemoOracle<F>) -> Vec<Op>
where
    F: AsyncFnMut(&[Op]) -> bool,
{
    // Coupled (worker, pr) lowering first because worker `i` defaults to `PRS[i]`.
    for hi in 1..WORKERS.len().min(PRS.len()) {
        for lo in 0..hi {
            let cand = remap_ops(&ops, None, Some((hi, lo, false)), Some((hi, lo, false)));
            if cand != ops && oracle.test(&cand).await {
                ops = cand;
            }
            let pos = first_positions(&ops);
            let hi_before_lo = match (pos.workers[hi].or(pos.prs[hi]), pos.workers[lo].or(pos.prs[lo])) {
                (Some(a), Some(b)) => a < b,
                _ => false,
            };
            if hi_before_lo {
                let cand = remap_ops(&ops, None, Some((hi, lo, true)), Some((hi, lo, true)));
                if cand != ops && oracle.test(&cand).await {
                    ops = cand;
                }
            }
        }
    }

    // Independent ticket, worker, and PR pool lowering.
    for hi in 1..TICKETS.len() {
        for lo in 0..hi {
            let cand = remap_ops(&ops, Some((hi, lo, false)), None, None);
            if cand != ops && oracle.test(&cand).await {
                ops = cand;
            }
            let pos = first_positions(&ops);
            if matches!((pos.tickets[hi], pos.tickets[lo]), (Some(a), Some(b)) if a < b) {
                let cand = remap_ops(&ops, Some((hi, lo, true)), None, None);
                if cand != ops && oracle.test(&cand).await {
                    ops = cand;
                }
            }
        }
    }
    for hi in 1..WORKERS.len() {
        for lo in 0..hi {
            let cand = remap_ops(&ops, None, Some((hi, lo, false)), None);
            if cand != ops && oracle.test(&cand).await {
                ops = cand;
            }
            let pos = first_positions(&ops);
            if matches!((pos.workers[hi], pos.workers[lo]), (Some(a), Some(b)) if a < b) {
                let cand = remap_ops(&ops, None, Some((hi, lo, true)), None);
                if cand != ops && oracle.test(&cand).await {
                    ops = cand;
                }
            }
        }
    }
    for hi in 1..PRS.len() {
        for lo in 0..hi {
            let cand = remap_ops(&ops, None, None, Some((hi, lo, false)));
            if cand != ops && oracle.test(&cand).await {
                ops = cand;
            }
            let pos = first_positions(&ops);
            if matches!((pos.prs[hi], pos.prs[lo]), (Some(a), Some(b)) if a < b) {
                let cand = remap_ops(&ops, None, None, Some((hi, lo, true)));
                if cand != ops && oracle.test(&cand).await {
                    ops = cand;
                }
            }
        }
    }
    ops
}

/// Deterministic 12-hex-char SHA-256 content hash over the canonical NDJSON op body.
pub fn ops_content_hash(ops: &[Op]) -> String {
    let ndjson = serialize_ops(ops);
    Sha256::digest(ndjson.as_bytes()).iter().take(6).map(|b| format!("{b:02x}")).collect()
}

fn sanitize_comment(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Format an `.ops` corpus file with `#` header comments and NDJSON op lines.
pub fn format_ops_file(ops: &[Op], source: &str, failure: &str) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "# source: {}", sanitize_comment(source));
    let _ = writeln!(out, "# failure: {}", sanitize_comment(failure));
    out.push_str(&serialize_ops(ops));
    out
}

/// Write a shrunk op sequence to `<corpus_dir>/<hash>.ops` and return its path.
pub fn write_corpus_file(corpus_dir: &Path, outcome: &ShrinkOutcome, source: &str) -> Result<PathBuf, String> {
    std::fs::create_dir_all(corpus_dir)
        .map_err(|e| format!("failed to create corpus dir {}: {e}", corpus_dir.display()))?;
    let path = corpus_dir.join(format!("{}.ops", outcome.hash));
    let body = format_ops_file(&outcome.ops, source, &outcome.failure_summary);
    std::fs::write(&path, body).map_err(|e| format!("failed to write {}: {e}", path.display()))?;
    Ok(path)
}

/// Resolve a `CHAOS_OPS` path (directly or relative to `CARGO_MANIFEST_DIR`) and parse it.
pub fn load_ops_file(path: &Path) -> Result<(PathBuf, Vec<Op>), String> {
    let resolved = if path.exists() {
        path.to_path_buf()
    } else if path.is_relative() {
        let from_manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join(path);
        if from_manifest.exists() { from_manifest } else { path.to_path_buf() }
    } else {
        path.to_path_buf()
    };
    let raw = std::fs::read_to_string(&resolved)
        .map_err(|e| format!("failed to read ops file {}: {e}", resolved.display()))?;
    let ops = deserialize_ops(&raw).map_err(|e| format!("failed to parse ops file {}: {e}", resolved.display()))?;
    Ok((resolved, ops))
}

/// List all `*.ops` files inside `dir` in deterministic sorted order.
pub fn list_corpus_files(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("failed to read corpus dir {}: {e}", dir.display()))?;
    let mut files = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| format!("failed to read entry in {}: {e}", dir.display()))?;
        let path = entry.path();
        if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("ops") {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}

pub fn default_corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/chaos_corpus")
}

pub fn corpus_dir_from_env() -> PathBuf {
    match std::env::var("CHAOS_CORPUS_DIR").ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) {
        Some(custom) => PathBuf::from(custom),
        None => default_corpus_dir(),
    }
}

pub fn parse_env_flag(raw: Option<&str>) -> bool {
    raw.map(str::trim).is_some_and(|v| !v.is_empty() && v != "0" && !v.eq_ignore_ascii_case("false"))
}

pub fn env_flag_enabled(name: &str) -> bool {
    parse_env_flag(std::env::var(name).ok().as_deref())
}

pub fn parse_env_path(raw: Option<&str>) -> Option<PathBuf> {
    raw.map(str::trim).filter(|s| !s.is_empty()).map(PathBuf::from)
}

pub fn display_replay_path(path: &Path) -> String {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    path.strip_prefix(manifest).unwrap_or(path).display().to_string()
}

pub fn display_source_path(path: &Path) -> String {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    if let Ok(rel) = path.strip_prefix(manifest) {
        return rel.display().to_string();
    }
    path.file_name().and_then(|s| s.to_str()).map(String::from).unwrap_or_else(|| "ops".to_string())
}

/// Render an `Op` as valid Rust expression syntax suitable for pasting into `tests/all/pr_chaos.rs`.
pub fn format_op_rust(op: &Op) -> String {
    match *op {
        Op::PrOpened { worker, short_sha, no_ci } => {
            format!("Op::PrOpened {{ worker: WSym({}), short_sha: {short_sha}, no_ci: {no_ci} }}", worker.0)
        }
        Op::PrUpdated { worker, short_sha } => {
            format!("Op::PrUpdated {{ worker: WSym({}), short_sha: {short_sha} }}", worker.0)
        }
        Op::PrReady { worker, short_sha } => {
            format!("Op::PrReady {{ worker: WSym({}), short_sha: {short_sha} }}", worker.0)
        }
        Op::GhPush { pr } => format!("Op::GhPush {{ pr: PrSym({}) }}", pr.0),
        Op::GhStatus { pr, checks, merge_state, draft } => {
            format!(
                "Op::GhStatus {{ pr: PrSym({}), checks: GhChecks::{checks:?}, merge_state: GhMergeState::{merge_state:?}, draft: {draft} }}",
                pr.0
            )
        }
        Op::GhFinish { pr, state, predates } => {
            format!("Op::GhFinish {{ pr: PrSym({}), state: GhEndState::{state:?}, predates: {predates} }}", pr.0)
        }
        Op::GhReadError { pr, fail } => format!("Op::GhReadError {{ pr: PrSym({}), fail: {fail} }}", pr.0),
        Op::GhRemovePr { pr } => format!("Op::GhRemovePr {{ pr: PrSym({}) }}", pr.0),
        Op::GhClearReadErrors => "Op::GhClearReadErrors".to_string(),
        Op::Reconcile => "Op::Reconcile".to_string(),
        Op::Adopt { to, from } => format!("Op::Adopt {{ to: TSym({}), from: TSym({}) }}", to.0, from.0),
        Op::TicketGateClear { ticket } => format!("Op::TicketGateClear {{ ticket: TSym({}) }}", ticket.0),
        Op::TicketSendGate { ticket } => format!("Op::TicketSendGate {{ ticket: TSym({}) }}", ticket.0),
        Op::TicketGateOpen { ticket } => format!("Op::TicketGateOpen {{ ticket: TSym({}) }}", ticket.0),
        Op::AnswerCard { card } => {
            format!("Op::AnswerCard {{ card: CardSym {{ kind: CardKind::{:?}, nth: {} }} }}", card.kind, card.nth)
        }
        Op::DismissCard { card } => {
            format!("Op::DismissCard {{ card: CardSym {{ kind: CardKind::{:?}, nth: {} }} }}", card.kind, card.nth)
        }
        Op::AnswerReadyAndClearGate { nth } => format!("Op::AnswerReadyAndClearGate {{ nth: {nth} }}"),
        Op::WorkerReviewGate { worker } => format!("Op::WorkerReviewGate {{ worker: WSym({}) }}", worker.0),
        Op::WorkerClose { worker } => format!("Op::WorkerClose {{ worker: WSym({}) }}", worker.0),
        Op::WorkerHandoff { worker } => format!("Op::WorkerHandoff {{ worker: WSym({}) }}", worker.0),
        Op::AdvanceTick { minutes } => format!("Op::AdvanceTick {{ minutes: {minutes} }}"),
        Op::MergeWaitPut { pr, ticket, reassigned_id } => {
            format!(
                "Op::MergeWaitPut {{ pr: PrSym({}), ticket: TSym({}), reassigned_id: {reassigned_id} }}",
                pr.0, ticket.0
            )
        }
        Op::GhQueueConfig { pr, enabled, queued } => {
            format!("Op::GhQueueConfig {{ pr: PrSym({}), enabled: {enabled}, queued: {queued} }}", pr.0)
        }
        Op::GhQueueDrop { pr, reason, predates } => {
            format!(
                "Op::GhQueueDrop {{ pr: PrSym({}), reason: QueueDropReason::{reason:?}, predates: {predates} }}",
                pr.0
            )
        }
        Op::FakeSetPrs { worker, pr } => {
            format!("Op::FakeSetPrs {{ worker: WSym({}), pr: PrSym({}) }}", worker.0, pr.0)
        }
        Op::WorkerSetBusy { worker, busy } => {
            format!("Op::WorkerSetBusy {{ worker: WSym({}), busy: {busy} }}", worker.0)
        }
        Op::ParentReply { worker } => format!("Op::ParentReply {{ worker: WSym({}) }}", worker.0),
        Op::TicketClose { ticket, by_board } => {
            format!("Op::TicketClose {{ ticket: TSym({}), by_board: {by_board} }}", ticket.0)
        }
        Op::TicketArchive { ticket, tick } => {
            format!("Op::TicketArchive {{ ticket: TSym({}), tick: {tick} }}", ticket.0)
        }
        Op::TicketThreadGone { ticket, tick } => {
            format!("Op::TicketThreadGone {{ ticket: TSym({}), tick: {tick} }}", ticket.0)
        }
        Op::WfReply { worker, by_label, reopen } => {
            format!("Op::WfReply {{ worker: WSym({}), by_label: {by_label}, reopen: {reopen} }}", worker.0)
        }
        Op::Pause { target, paused } => {
            let t = match target {
                PauseTarget::Ticket { ticket } => format!("PauseTarget::Ticket {{ ticket: TSym({}) }}", ticket.0),
                PauseTarget::WorkerSelf { worker } => {
                    format!("PauseTarget::WorkerSelf {{ worker: WSym({}) }}", worker.0)
                }
                PauseTarget::WorkerByParent { worker } => {
                    format!("PauseTarget::WorkerByParent {{ worker: WSym({}) }}", worker.0)
                }
                PauseTarget::BoardTicket { ticket } => {
                    format!("PauseTarget::BoardTicket {{ ticket: TSym({}) }}", ticket.0)
                }
                PauseTarget::BoardWorker { worker } => {
                    format!("PauseTarget::BoardWorker {{ worker: WSym({}) }}", worker.0)
                }
            };
            format!("Op::Pause {{ target: {t}, paused: {paused} }}")
        }
        Op::Sweep { target } => {
            let t = match target {
                SweepTarget::Ticket { ticket } => format!("SweepTarget::Ticket {{ ticket: TSym({}) }}", ticket.0),
                SweepTarget::Worker { worker } => format!("SweepTarget::Worker {{ worker: WSym({}) }}", worker.0),
            };
            format!("Op::Sweep {{ target: {t} }}")
        }
        Op::SetRelay { relay } => format!("Op::SetRelay {{ relay: {relay} }}"),
        Op::WorkerSetMergeOnly { worker, merge_only } => {
            format!("Op::WorkerSetMergeOnly {{ worker: WSym({}), merge_only: {merge_only} }}", worker.0)
        }
        Op::PrFollowup { worker, pr, event, short_sha, no_ci } => {
            format!(
                "Op::PrFollowup {{ worker: WSym({}), pr: PrSym({}), event: PrEventKind::{event:?}, short_sha: {short_sha}, no_ci: {no_ci} }}",
                worker.0, pr.0
            )
        }
        Op::GhFilesPaginated { pr, paginated } => {
            format!("Op::GhFilesPaginated {{ pr: PrSym({}), paginated: {paginated} }}", pr.0)
        }
        Op::GhReopen { pr } => format!("Op::GhReopen {{ pr: PrSym({}) }}", pr.0),
    }
}

/// Format a ready-to-paste `#[tokio::test]` regression for `tests/all/pr_chaos.rs`.
pub fn format_regression_test(outcome: &ShrinkOutcome, source: &str) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "#[tokio::test]");
    let _ = writeln!(out, "async fn chaos_regression_{}() {{", outcome.hash);
    let _ = writeln!(out, "    // Source: {}", sanitize_comment(source));
    let _ = writeln!(out, "    // Failure: {}", sanitize_comment(&outcome.failure_summary));
    let _ = writeln!(out, "    let ops = vec![");
    for op in &outcome.ops {
        let _ = writeln!(out, "        {},", format_op_rust(op));
    }
    let _ = writeln!(out, "    ];");
    let _ =
        writeln!(out, "    let _ = run_sequential_ops(&ops, None, true, \"chaos-regression-{}\").await;", outcome.hash);
    let _ = write!(out, "}}");
    out
}

/// Format the full shrink report printed on `CHAOS_SHRINK=1` failure.
pub fn format_shrink_report(
    outcome: &ShrinkOutcome,
    source: &str,
    written_path: &Path,
    sentinel_name: Option<&str>,
) -> String {
    let rel = display_replay_path(written_path);
    let test_snippet = format_regression_test(outcome, source);
    let replay_cmd = match sentinel_name {
        Some(name) => format!(
            "CHAOS_OPS={rel} CHAOS_SENTINEL={name} cargo test --locked --features test-sentinels --test all pr_chaos::seeded_sequential_chaos -- --nocapture"
        ),
        None => {
            format!("CHAOS_OPS={rel} cargo test --locked --test all pr_chaos::seeded_sequential_chaos -- --nocapture")
        }
    };
    let loc_suffix = match &outcome.class.location {
        Some(loc) => format!(" @ {loc}"),
        None => String::new(),
    };
    format!(
        "CHAOS_SHRINK shrunk {source} from {} ops to {} ops ({} oracle evaluations)\n\
         failure class: {:?} / {}{loc_suffix}\n\
         corpus file:   {rel}\n\
         replay:        {replay_cmd}\n\n\
         Ready-to-paste regression test:\n\n{test_snippet}",
        outcome.original_len,
        outcome.ops.len(),
        outcome.evaluations,
        outcome.class.kind,
        outcome.class.signature,
    )
}
