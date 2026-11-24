use std::fmt;

use arbitrary::Unstructured;
use serde::{Deserialize, Serialize};
use t3_codemode::board::Card;

use super::{PRS, Rng, TICKETS, WORKERS};

/// Default maximum number of ops decoded per sequential chaos seed.
pub const DEFAULT_MAX_OPS: usize = 70;

/// Deterministic byte buffer size fed into `arbitrary::Unstructured` per seed.
///
/// Each op consumes at most ~6 bytes (1 for kind + up to 5 for operands), so
/// 768 bytes comfortably decodes `DEFAULT_MAX_OPS` ops with room to spare.
pub const SEED_BYTES: usize = 768;

/// Salt separating Swarm profile derivation from the `Unstructured` op byte stream.
const SWARM_SALT: u64 = 0x51a7_c0de_2440_cafe;

/// Pool-index symbolic handle for a ticket thread (`TICKETS`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TSym(pub usize);

/// Pool-index symbolic handle for a worker thread (`WORKERS`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WSym(pub usize);

/// Pool-index symbolic handle for a pull request (`PRS`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PrSym(pub usize);

/// Filter classifying live board cards for symbolic card selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CardKind {
    Any,
    ReadyMerge,
    Relayed,
    TicketGate,
    Report,
}

impl CardKind {
    pub fn matches(self, c: &Card) -> bool {
        match self {
            Self::Any => true,
            Self::ReadyMerge => {
                c.kind == "gate" && c.worker.is_none() && c.ticket.is_some() && c.pr.as_ref().is_some_and(|p| p.ready)
            }
            Self::Relayed => c.worker.is_some(),
            Self::TicketGate => c.kind == "gate" && c.worker.is_none() && !c.pr.as_ref().is_some_and(|p| p.ready),
            Self::Report => c.kind == "report",
        }
    }
}

/// Symbolic handle for an open board card: the `nth` live card matching `kind`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CardSym {
    pub kind: CardKind,
    pub nth: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GhChecks {
    Pending,
    Passing,
    Failing,
    Cancelled,
    Unread,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GhMergeState {
    Clean,
    Unknown,
    Blocked,
    Behind,
    Dirty,
}

impl GhMergeState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Clean => "CLEAN",
            Self::Unknown => "UNKNOWN",
            Self::Blocked => "BLOCKED",
            Self::Behind => "BEHIND",
            Self::Dirty => "DIRTY",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GhEndState {
    Merged,
    Closed,
}

impl GhEndState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Merged => "MERGED",
            Self::Closed => "CLOSED",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueueDropReason {
    FailedChecks,
    MergeConflict,
}

impl QueueDropReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::FailedChecks => "failed_checks",
            Self::MergeConflict => "merge_conflict",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PauseTarget {
    Ticket { ticket: TSym },
    WorkerSelf { worker: WSym },
    WorkerByParent { worker: WSym },
    BoardTicket { ticket: TSym },
    BoardWorker { worker: WSym },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SweepTarget {
    Ticket { ticket: TSym },
    Worker { worker: WSym },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrEventKind {
    Opened,
    Updated,
    Ready,
}

/// Declarative sequential chaos operation with symbolic pool-index targets.
///
/// Every subsequence of a `Vec<Op>` is a valid replayable program: any out-of-range
/// handle, closed worker, non-open PR, or missing card becomes a recorded no-op.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Op {
    PrOpened { worker: WSym, short_sha: bool, no_ci: bool },
    PrUpdated { worker: WSym, short_sha: bool },
    PrReady { worker: WSym, short_sha: bool },
    GhPush { pr: PrSym },
    GhStatus { pr: PrSym, checks: GhChecks, merge_state: GhMergeState, draft: bool },
    GhFinish { pr: PrSym, state: GhEndState, predates: bool },
    GhReadError { pr: PrSym, fail: bool },
    GhRemovePr { pr: PrSym },
    GhClearReadErrors,
    Reconcile,
    Adopt { to: TSym, from: TSym },
    TicketGateClear { ticket: TSym },
    TicketSendGate { ticket: TSym },
    TicketGateOpen { ticket: TSym },
    AnswerCard { card: CardSym },
    DismissCard { card: CardSym },
    AnswerReadyAndClearGate { nth: usize },
    WorkerReviewGate { worker: WSym },
    WorkerClose { worker: WSym },
    WorkerHandoff { worker: WSym },
    AdvanceTick { minutes: u16 },
    MergeWaitPut { pr: PrSym, ticket: TSym, reassigned_id: bool },
    GhQueueConfig { pr: PrSym, enabled: bool, queued: bool },
    GhQueueDrop { pr: PrSym, reason: QueueDropReason, predates: bool },
    FakeSetPrs { worker: WSym, pr: PrSym },
    WorkerSetBusy { worker: WSym, busy: bool },
    ParentReply { worker: WSym },
    TicketClose { ticket: TSym, by_board: bool },
    TicketArchive { ticket: TSym, tick: bool },
    TicketThreadGone { ticket: TSym, tick: bool },
    WfReply { worker: WSym, by_label: bool, reopen: bool },
    Pause { target: PauseTarget, paused: bool },
    Sweep { target: SweepTarget },
    SetRelay { relay: bool },
    WorkerSetMergeOnly { worker: WSym, merge_only: bool },
    PrFollowup { worker: WSym, pr: PrSym, event: PrEventKind, short_sha: bool, no_ci: bool },
    GhFilesPaginated { pr: PrSym, paginated: bool },
    GhReopen { pr: PrSym },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(usize)]
pub enum OpKind {
    PrOpened = 0,
    PrUpdated,
    PrReady,
    GhPush,
    GhStatus,
    GhFinish,
    GhReadError,
    GhRemovePr,
    GhClearReadErrors,
    Reconcile,
    Adopt,
    TicketGateClear,
    TicketSendGate,
    TicketGateOpen,
    AnswerCard,
    DismissCard,
    AnswerReadyAndClearGate,
    WorkerReviewGate,
    WorkerClose,
    WorkerHandoff,
    AdvanceTick,
    MergeWaitPut,
    GhQueueConfig,
    GhQueueDrop,
    FakeSetPrs,
    WorkerSetBusy,
    ParentReply,
    TicketClose,
    TicketArchive,
    TicketThreadGone,
    WfReply,
    Pause,
    Sweep,
    SetRelay,
    WorkerSetMergeOnly,
    PrFollowup,
    GhFilesPaginated,
    GhReopen,
}

impl OpKind {
    pub const ALL: &'static [OpKind] = &[
        Self::PrOpened,
        Self::PrUpdated,
        Self::PrReady,
        Self::GhPush,
        Self::GhStatus,
        Self::GhFinish,
        Self::GhReadError,
        Self::GhRemovePr,
        Self::GhClearReadErrors,
        Self::Reconcile,
        Self::Adopt,
        Self::TicketGateClear,
        Self::TicketSendGate,
        Self::TicketGateOpen,
        Self::AnswerCard,
        Self::DismissCard,
        Self::AnswerReadyAndClearGate,
        Self::WorkerReviewGate,
        Self::WorkerClose,
        Self::WorkerHandoff,
        Self::AdvanceTick,
        Self::MergeWaitPut,
        Self::GhQueueConfig,
        Self::GhQueueDrop,
        Self::FakeSetPrs,
        Self::WorkerSetBusy,
        Self::ParentReply,
        Self::TicketClose,
        Self::TicketArchive,
        Self::TicketThreadGone,
        Self::WfReply,
        Self::Pause,
        Self::Sweep,
        Self::SetRelay,
        Self::WorkerSetMergeOnly,
        Self::PrFollowup,
        Self::GhFilesPaginated,
        Self::GhReopen,
    ];

    pub const COUNT: usize = Self::ALL.len();

    pub const fn name(self) -> &'static str {
        match self {
            Self::PrOpened => "pr_opened",
            Self::PrUpdated => "pr_updated",
            Self::PrReady => "pr_ready",
            Self::GhPush => "gh_push",
            Self::GhStatus => "gh_status",
            Self::GhFinish => "gh_finish",
            Self::GhReadError => "gh_read_error",
            Self::GhRemovePr => "gh_remove_pr",
            Self::GhClearReadErrors => "gh_clear_read_errors",
            Self::Reconcile => "reconcile",
            Self::Adopt => "adopt",
            Self::TicketGateClear => "ticket_gate_clear",
            Self::TicketSendGate => "ticket_send_gate",
            Self::TicketGateOpen => "ticket_gate_open",
            Self::AnswerCard => "answer_card",
            Self::DismissCard => "dismiss_card",
            Self::AnswerReadyAndClearGate => "answer_ready_and_clear_gate",
            Self::WorkerReviewGate => "worker_review_gate",
            Self::WorkerClose => "worker_close",
            Self::WorkerHandoff => "worker_handoff",
            Self::AdvanceTick => "advance_tick",
            Self::MergeWaitPut => "merge_wait_put",
            Self::GhQueueConfig => "gh_queue_config",
            Self::GhQueueDrop => "gh_queue_drop",
            Self::FakeSetPrs => "fake_set_prs",
            Self::WorkerSetBusy => "worker_set_busy",
            Self::ParentReply => "parent_reply",
            Self::TicketClose => "ticket_close",
            Self::TicketArchive => "ticket_archive",
            Self::TicketThreadGone => "ticket_thread_gone",
            Self::WfReply => "wf_reply",
            Self::Pause => "pause",
            Self::Sweep => "sweep",
            Self::SetRelay => "set_relay",
            Self::WorkerSetMergeOnly => "worker_set_merge_only",
            Self::PrFollowup => "pr_followup",
            Self::GhFilesPaginated => "gh_files_paginated",
            Self::GhReopen => "gh_reopen",
        }
    }

    pub const fn is_creating(self) -> bool {
        matches!(
            self,
            Self::PrOpened
                | Self::PrReady
                | Self::TicketSendGate
                | Self::WorkerReviewGate
                | Self::MergeWaitPut
                | Self::FakeSetPrs
                | Self::ParentReply
                | Self::WfReply
                | Self::PrFollowup
        )
    }

    pub const fn is_fault(self) -> bool {
        matches!(self, Self::GhReadError | Self::GhRemovePr)
    }

    const fn base_weight(self) -> u16 {
        match self {
            Self::Reconcile => 5,
            Self::PrReady => 4,
            Self::GhStatus | Self::WorkerSetBusy | Self::ParentReply | Self::WfReply => 3,
            Self::PrOpened
            | Self::PrUpdated
            | Self::GhPush
            | Self::GhFinish
            | Self::Adopt
            | Self::TicketGateClear
            | Self::TicketSendGate
            | Self::TicketGateOpen
            | Self::AnswerCard
            | Self::DismissCard
            | Self::AnswerReadyAndClearGate
            | Self::WorkerReviewGate
            | Self::WorkerClose
            | Self::WorkerHandoff
            | Self::AdvanceTick
            | Self::MergeWaitPut
            | Self::GhQueueConfig
            | Self::GhQueueDrop
            | Self::FakeSetPrs
            | Self::Pause
            | Self::Sweep
            | Self::PrFollowup
            | Self::GhFilesPaginated
            | Self::GhReopen => 2,
            Self::GhReadError
            | Self::GhRemovePr
            | Self::GhClearReadErrors
            | Self::TicketClose
            | Self::TicketArchive
            | Self::TicketThreadGone
            | Self::SetRelay
            | Self::WorkerSetMergeOnly => 1,
        }
    }
}

impl Op {
    pub const fn kind(&self) -> OpKind {
        match self {
            Self::PrOpened { .. } => OpKind::PrOpened,
            Self::PrUpdated { .. } => OpKind::PrUpdated,
            Self::PrReady { .. } => OpKind::PrReady,
            Self::GhPush { .. } => OpKind::GhPush,
            Self::GhStatus { .. } => OpKind::GhStatus,
            Self::GhFinish { .. } => OpKind::GhFinish,
            Self::GhReadError { .. } => OpKind::GhReadError,
            Self::GhRemovePr { .. } => OpKind::GhRemovePr,
            Self::GhClearReadErrors => OpKind::GhClearReadErrors,
            Self::Reconcile => OpKind::Reconcile,
            Self::Adopt { .. } => OpKind::Adopt,
            Self::TicketGateClear { .. } => OpKind::TicketGateClear,
            Self::TicketSendGate { .. } => OpKind::TicketSendGate,
            Self::TicketGateOpen { .. } => OpKind::TicketGateOpen,
            Self::AnswerCard { .. } => OpKind::AnswerCard,
            Self::DismissCard { .. } => OpKind::DismissCard,
            Self::AnswerReadyAndClearGate { .. } => OpKind::AnswerReadyAndClearGate,
            Self::WorkerReviewGate { .. } => OpKind::WorkerReviewGate,
            Self::WorkerClose { .. } => OpKind::WorkerClose,
            Self::WorkerHandoff { .. } => OpKind::WorkerHandoff,
            Self::AdvanceTick { .. } => OpKind::AdvanceTick,
            Self::MergeWaitPut { .. } => OpKind::MergeWaitPut,
            Self::GhQueueConfig { .. } => OpKind::GhQueueConfig,
            Self::GhQueueDrop { .. } => OpKind::GhQueueDrop,
            Self::FakeSetPrs { .. } => OpKind::FakeSetPrs,
            Self::WorkerSetBusy { .. } => OpKind::WorkerSetBusy,
            Self::ParentReply { .. } => OpKind::ParentReply,
            Self::TicketClose { .. } => OpKind::TicketClose,
            Self::TicketArchive { .. } => OpKind::TicketArchive,
            Self::TicketThreadGone { .. } => OpKind::TicketThreadGone,
            Self::WfReply { .. } => OpKind::WfReply,
            Self::Pause { .. } => OpKind::Pause,
            Self::Sweep { .. } => OpKind::Sweep,
            Self::SetRelay { .. } => OpKind::SetRelay,
            Self::WorkerSetMergeOnly { .. } => OpKind::WorkerSetMergeOnly,
            Self::PrFollowup { .. } => OpKind::PrFollowup,
            Self::GhFilesPaginated { .. } => OpKind::GhFilesPaginated,
            Self::GhReopen { .. } => OpKind::GhReopen,
        }
    }
}

/// Per-`OpKind` decoded and non-noop execution counters, mergeable across seeds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpStats {
    decoded: [u64; OpKind::COUNT],
    executed: [u64; OpKind::COUNT],
}

impl Default for OpStats {
    fn default() -> Self {
        Self { decoded: [0; OpKind::COUNT], executed: [0; OpKind::COUNT] }
    }
}

impl OpStats {
    pub fn record(&mut self, kind: OpKind, executed: bool) {
        self.decoded[kind as usize] = self.decoded[kind as usize].saturating_add(1);
        if executed {
            self.executed[kind as usize] = self.executed[kind as usize].saturating_add(1);
        }
    }

    pub fn merge(&mut self, other: &OpStats) {
        for i in 0..OpKind::COUNT {
            self.decoded[i] = self.decoded[i].saturating_add(other.decoded[i]);
            self.executed[i] = self.executed[i].saturating_add(other.executed[i]);
        }
    }

    pub fn executed(&self, kind: OpKind) -> u64 {
        self.executed[kind as usize]
    }

    pub fn assert_all_executed(&self, context: &str) {
        let missing: Vec<&str> =
            OpKind::ALL.iter().copied().filter(|&k| self.executed(k) == 0).map(OpKind::name).collect();
        assert!(missing.is_empty(), "op execution coverage failed in {context}: unexecuted ops: {missing:?}\n{self}");
    }
}

impl fmt::Display for OpStats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Op execution summary (executed / decoded):")?;
        for &k in OpKind::ALL {
            writeln!(f, "  {:<32} {:>4} / {:<4}", k.name(), self.executed[k as usize], self.decoded[k as usize])?;
        }
        Ok(())
    }
}

/// Per-op-kind weight table and fault density passed to [`decode_ops`].
///
/// Kept outside the `arbitrary::Unstructured` byte stream so seeded Swarm runs
/// and coverage-guided fuzzers share the exact same decoder without byte-cascade
/// mutations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpWeights {
    pub weights: [u16; OpKind::COUNT],
    /// Probability (0..=100) for inline fault variants such as `MergeWaitPut { reassigned_id: true }`.
    pub fault_pct: u8,
    /// Upper bound on ops decoded from one `Unstructured` stream.
    pub max_ops: usize,
}

impl OpWeights {
    /// Equal weight across every op kind, for coverage-guided fuzzers and unbiased testing.
    pub const UNIFORM: Self = Self { weights: [1; OpKind::COUNT], fault_pct: 33, max_ops: DEFAULT_MAX_OPS };

    pub fn get(&self, kind: OpKind) -> u16 {
        self.weights[kind as usize]
    }

    pub fn set(&mut self, kind: OpKind, weight: u16) {
        self.weights[kind as usize] = weight;
    }

    pub fn total_weight(&self) -> u32 {
        self.weights.iter().map(|&w| u32::from(w)).sum()
    }

    pub fn enabled_count(&self) -> usize {
        self.weights.iter().filter(|&&w| w > 0).count()
    }

    pub fn zeroed_count(&self) -> usize {
        self.weights.iter().filter(|&&w| w == 0).count()
    }

    /// Derive a per-seed Swarm Testing profile outside the `Unstructured` byte stream.
    ///
    /// Zeroes ~40% of op kinds per seed, always keeps `Reconcile` and at least one
    /// creating op enabled, and varies fault density across four regimes (off, low,
    /// medium, high).
    pub fn swarm(seed: u64) -> Self {
        let mut rng = Rng::new(seed ^ SWARM_SALT);
        let (fault_scale, fault_pct): (u16, u8) = match rng.pick(4) {
            0 => (0, 0),
            1 => (1, 15),
            2 => (2, 35),
            _ => (3, 65),
        };

        let mut weights = [0u16; OpKind::COUNT];
        for &kind in OpKind::ALL {
            if kind == OpKind::Reconcile {
                weights[kind as usize] = kind.base_weight();
                continue;
            }
            if kind.is_fault() {
                if fault_scale == 0 || rng.pick(5) < 2 {
                    weights[kind as usize] = 0;
                } else {
                    weights[kind as usize] = kind.base_weight().saturating_mul(fault_scale);
                }
                continue;
            }
            if rng.pick(5) < 2 {
                weights[kind as usize] = 0;
            } else {
                weights[kind as usize] = kind.base_weight();
            }
        }

        let creating: Vec<OpKind> = OpKind::ALL.iter().copied().filter(|k| k.is_creating()).collect();
        if creating.iter().all(|&k| weights[k as usize] == 0) {
            let chosen = creating[rng.pick(creating.len())];
            weights[chosen as usize] = chosen.base_weight();
        }

        Self { weights, fault_pct, max_ops: DEFAULT_MAX_OPS }
    }
}

/// Generate a deterministic byte stream for `Unstructured::new` from a seed.
pub fn seed_bytes(seed: u64) -> [u8; SEED_BYTES] {
    let mut rng = Rng::new(seed);
    let mut buf = [0u8; SEED_BYTES];
    for chunk in buf.as_chunks_mut::<8>().0 {
        *chunk = rng.next().to_le_bytes();
    }
    buf
}

fn pick_kind(u: &mut Unstructured<'_>, weights: &OpWeights, total: u32) -> arbitrary::Result<OpKind> {
    let mut target = if total == 1 {
        let _ = u.arbitrary::<u8>()?;
        0
    } else {
        u.int_in_range(0..=(total - 1))?
    };
    for &kind in OpKind::ALL {
        let w = u32::from(weights.get(kind));
        if w == 0 {
            continue;
        }
        if target < w {
            return Ok(kind);
        }
        target -= w;
    }
    unreachable!("total_weight matches sum of OpWeights::weights")
}

fn decode_worker(u: &mut Unstructured<'_>) -> arbitrary::Result<WSym> {
    Ok(WSym(u.int_in_range(0..=(WORKERS.len() - 1))?))
}

fn decode_ticket(u: &mut Unstructured<'_>) -> arbitrary::Result<TSym> {
    Ok(TSym(u.int_in_range(0..=(TICKETS.len() - 1))?))
}

fn decode_pr(u: &mut Unstructured<'_>) -> arbitrary::Result<PrSym> {
    Ok(PrSym(u.int_in_range(0..=(PRS.len() - 1))?))
}

fn decode_card(u: &mut Unstructured<'_>) -> arbitrary::Result<CardSym> {
    const KINDS: &[CardKind] = &[
        CardKind::Any,
        CardKind::Any,
        CardKind::ReadyMerge,
        CardKind::Relayed,
        CardKind::TicketGate,
        CardKind::Report,
    ];
    let kind = *u.choose(KINDS)?;
    let nth = match u.int_in_range(0..=7)? {
        0..=4 => 0,
        5..=6 => 1,
        _ => 2,
    };
    Ok(CardSym { kind, nth })
}

fn decode_one(u: &mut Unstructured<'_>, weights: &OpWeights, total: u32) -> arbitrary::Result<Op> {
    let kind = pick_kind(u, weights, total)?;
    let op = match kind {
        OpKind::PrOpened => {
            Op::PrOpened { worker: decode_worker(u)?, short_sha: u.arbitrary()?, no_ci: u.int_in_range(0..=7)? == 0 }
        }
        OpKind::PrUpdated => Op::PrUpdated { worker: decode_worker(u)?, short_sha: u.arbitrary()? },
        OpKind::PrReady => Op::PrReady { worker: decode_worker(u)?, short_sha: u.arbitrary()? },
        OpKind::GhPush => Op::GhPush { pr: decode_pr(u)? },
        OpKind::GhStatus => {
            const CHECKS: &[GhChecks] = &[
                GhChecks::Pending,
                GhChecks::Pending,
                GhChecks::Passing,
                GhChecks::Passing,
                GhChecks::Failing,
                GhChecks::Cancelled,
                GhChecks::Unread,
            ];
            const STATES: &[GhMergeState] = &[
                GhMergeState::Clean,
                GhMergeState::Clean,
                GhMergeState::Unknown,
                GhMergeState::Blocked,
                GhMergeState::Behind,
                GhMergeState::Dirty,
            ];
            Op::GhStatus {
                pr: decode_pr(u)?,
                checks: *u.choose(CHECKS)?,
                merge_state: *u.choose(STATES)?,
                draft: u.int_in_range(0..=5)? == 0,
            }
        }
        OpKind::GhFinish => Op::GhFinish {
            pr: decode_pr(u)?,
            state: if u.arbitrary()? { GhEndState::Merged } else { GhEndState::Closed },
            predates: u.arbitrary()?,
        },
        OpKind::GhReadError => Op::GhReadError { pr: decode_pr(u)?, fail: u.int_in_range(0..=2)? != 0 },
        OpKind::GhRemovePr => Op::GhRemovePr { pr: decode_pr(u)? },
        OpKind::GhClearReadErrors => Op::GhClearReadErrors,
        OpKind::Reconcile => Op::Reconcile,
        OpKind::Adopt => {
            let from = u.int_in_range(0..=(TICKETS.len() - 1))?;
            let offset = u.int_in_range(1..=(TICKETS.len() - 1))?;
            let to = (from + offset) % TICKETS.len();
            Op::Adopt { to: TSym(to), from: TSym(from) }
        }
        OpKind::TicketGateClear => Op::TicketGateClear { ticket: decode_ticket(u)? },
        OpKind::TicketSendGate => Op::TicketSendGate { ticket: decode_ticket(u)? },
        OpKind::TicketGateOpen => Op::TicketGateOpen { ticket: decode_ticket(u)? },
        OpKind::AnswerCard => Op::AnswerCard { card: decode_card(u)? },
        OpKind::DismissCard => Op::DismissCard { card: decode_card(u)? },
        OpKind::AnswerReadyAndClearGate => {
            let nth = usize::from(u.int_in_range(0..=3)? == 3);
            Op::AnswerReadyAndClearGate { nth }
        }
        OpKind::WorkerReviewGate => Op::WorkerReviewGate { worker: decode_worker(u)? },
        OpKind::WorkerClose => Op::WorkerClose { worker: decode_worker(u)? },
        OpKind::WorkerHandoff => Op::WorkerHandoff { worker: decode_worker(u)? },
        OpKind::AdvanceTick => {
            const DELTAS: &[u16] = &[15, 35, 60, 200];
            Op::AdvanceTick { minutes: *u.choose(DELTAS)? }
        }
        OpKind::MergeWaitPut => {
            let pr = decode_pr(u)?;
            let ticket = decode_ticket(u)?;
            let reassigned_id = weights.fault_pct > 0 && u.int_in_range(0..=99)? < weights.fault_pct.min(100);
            Op::MergeWaitPut { pr, ticket, reassigned_id }
        }
        OpKind::GhQueueConfig => {
            let pr = decode_pr(u)?;
            let enabled = u.int_in_range(0..=3)? != 0;
            let queued = enabled && u.arbitrary()?;
            Op::GhQueueConfig { pr, enabled, queued }
        }
        OpKind::GhQueueDrop => Op::GhQueueDrop {
            pr: decode_pr(u)?,
            reason: if u.arbitrary()? { QueueDropReason::FailedChecks } else { QueueDropReason::MergeConflict },
            predates: u.int_in_range(0..=2)? == 0,
        },
        OpKind::FakeSetPrs => Op::FakeSetPrs { worker: decode_worker(u)?, pr: decode_pr(u)? },
        OpKind::WorkerSetBusy => Op::WorkerSetBusy { worker: decode_worker(u)?, busy: u.int_in_range(0..=2)? != 0 },
        OpKind::ParentReply => Op::ParentReply { worker: decode_worker(u)? },
        OpKind::TicketClose => Op::TicketClose { ticket: decode_ticket(u)?, by_board: u.arbitrary()? },
        OpKind::TicketArchive => Op::TicketArchive { ticket: decode_ticket(u)?, tick: u.arbitrary()? },
        OpKind::TicketThreadGone => Op::TicketThreadGone { ticket: decode_ticket(u)?, tick: u.arbitrary()? },
        OpKind::WfReply => Op::WfReply { worker: decode_worker(u)?, by_label: u.arbitrary()?, reopen: u.arbitrary()? },
        OpKind::Pause => {
            let target = match u.int_in_range(0..=4)? {
                0 => PauseTarget::Ticket { ticket: decode_ticket(u)? },
                1 => PauseTarget::WorkerSelf { worker: decode_worker(u)? },
                2 => PauseTarget::WorkerByParent { worker: decode_worker(u)? },
                3 => PauseTarget::BoardTicket { ticket: decode_ticket(u)? },
                _ => PauseTarget::BoardWorker { worker: decode_worker(u)? },
            };
            Op::Pause { target, paused: u.int_in_range(0..=2)? != 0 }
        }
        OpKind::Sweep => {
            let target = if u.arbitrary()? {
                SweepTarget::Ticket { ticket: decode_ticket(u)? }
            } else {
                SweepTarget::Worker { worker: decode_worker(u)? }
            };
            Op::Sweep { target }
        }
        OpKind::SetRelay => Op::SetRelay { relay: u.int_in_range(0..=2)? == 0 },
        OpKind::WorkerSetMergeOnly => {
            Op::WorkerSetMergeOnly { worker: decode_worker(u)?, merge_only: u.int_in_range(0..=2)? != 0 }
        }
        OpKind::PrFollowup => {
            let worker = decode_worker(u)?;
            let offset = u.int_in_range(1..=(PRS.len() - 1))?;
            let pr = PrSym((worker.0 + offset) % PRS.len());
            const EVENTS: &[PrEventKind] = &[PrEventKind::Opened, PrEventKind::Updated, PrEventKind::Ready];
            Op::PrFollowup {
                worker,
                pr,
                event: *u.choose(EVENTS)?,
                short_sha: u.arbitrary()?,
                no_ci: u.int_in_range(0..=7)? == 0,
            }
        }
        OpKind::GhFilesPaginated => Op::GhFilesPaginated { pr: decode_pr(u)?, paginated: u.int_in_range(0..=2)? != 0 },
        OpKind::GhReopen => Op::GhReopen { pr: decode_pr(u)? },
    };
    Ok(op)
}

/// Decode a sequence of symbolic chaos operations from `u` according to `weights`.
///
/// Stops when `u` is exhausted or `weights.max_ops` ops have been decoded.
pub fn decode_ops(u: &mut Unstructured<'_>, weights: &OpWeights) -> Vec<Op> {
    let total = weights.total_weight();
    if total == 0 || weights.max_ops == 0 {
        return Vec::new();
    }
    let mut ops = Vec::with_capacity(weights.max_ops.min(u.len()));
    while !u.is_empty() && ops.len() < weights.max_ops {
        match decode_one(u, weights, total) {
            Ok(op) => ops.push(op),
            Err(_) => break,
        }
    }
    ops
}

/// Serialize ops as one JSON object per line (NDJSON) for stable `.ops` corpus files.
pub fn serialize_ops(ops: &[Op]) -> String {
    let mut out = String::new();
    for op in ops {
        out.push_str(&serde_json::to_string(op).expect("Op serialization cannot fail"));
        out.push('\n');
    }
    out
}

/// Deserialize an NDJSON `.ops` corpus string, ignoring blank lines and `#` comments.
pub fn deserialize_ops(raw: &str) -> Result<Vec<Op>, String> {
    let mut ops = Vec::new();
    for (idx, line) in raw.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let op: Op =
            serde_json::from_str(trimmed).map_err(|e| format!("invalid op on line {}: {e} ({trimmed:?})", idx + 1))?;
        ops.push(op);
    }
    Ok(ops)
}
