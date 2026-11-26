use std::collections::BTreeSet;
use std::fmt;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

const MAX_ADOPTED_TRACKED: usize = 256;

/// Reachability tier for a tracked workflow state-machine branch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReachTier {
    /// Required to be hit at least once across the default chaos seed range.
    Tier1,
    /// Tracked and verified by directed tests; promoted to Tier 1 once chaos exercises its preconditions.
    Tier2,
}

/// Critical workflow state-machine branches tracked by [`Reach`].
///
/// Modeled on Antithesis "sometimes" assertions and FoundationDB `TEST()` probes:
/// a chaos suite that silently stops reaching one of these branches fails fast
/// instead of reporting a vacuous green run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(usize)]
pub enum ReachPoint {
    /// `reconcile::advance` completed a `mergePullRequest` mutation.
    AdvanceMerged = 0,
    /// `reconcile::advance` completed an `enqueuePullRequest` mutation.
    AdvanceEnqueued,
    /// `reconcile::open` rejected and retired a `MergeWait` whose PR node ID no longer matched GitHub.
    ReassignedIdRejected,
    /// `reconcile::finished` quietly ignored a `MERGED` event whose timestamp predated tracking.
    MergeEventPredatesTracking,
    /// `reconcile::finished` quietly ignored a `CLOSED` event whose timestamp predated tracking.
    CloseEventPredatesTracking,
    /// `reconcile::finished` acted on an after-tracking `MERGED` or `CLOSED` event.
    FinishedEventReported,
    /// `reconcile::dropped` ignored a merge-queue removal event whose timestamp predated tracking.
    QueueDropPredatesTracking,
    /// `reconcile::dropped` reported an after-tracking merge-queue removal event.
    QueueDropReported,
    /// A single ticket simultaneously held two or more open ready merge cards.
    TwoReadyCardsOneTicket,
    /// `board_card` preserved an open ready merge card when a newer ticket-owned gate opened.
    DeliveryGateOverReadyCard,
    /// `wf.gate({ clear: true })` kept a ticket's `merge` gate active for another unanswered ready card.
    GateClearPreservedReadyCard,
    /// `reconcile::fix` reconciled a PR whose tracking had been transferred by `wf.adopt`.
    ReconcileAdoptedPr,
    /// `Wf::dismiss_at` resolved an open board card with an explicit `CardRef` identity check.
    DismissAtResolved,
    /// `reconcile` re-checked a worker's waiting `pr.ready` state via `retry_ready`.
    ReadyWaitingRetried,
    /// `reconcile::open` dropped a ready merge card and released its worker because the PR head moved.
    ReadyCardDroppedHeadMoved,
    /// `Wf::answer_relayed` routed a board answer directly to a worker past its ticket.
    RelayedGateAnswered,
    /// Closing a link (or flushing a closed link's outbox) purged at least one held outbox wake.
    OutboxHeldThenClosed,
    /// A board ticket link closed via board close or ticket self-close.
    TicketClosed,
    /// Watcher closed a board ticket link after T3 reported its thread archived.
    TicketArchived,
    /// Watcher closed a board ticket link after T3 reported its thread gone.
    TicketThreadGone,
    /// `wf.reply` delivered or queued a wake with `reopen: true` on a closed child link.
    ReplyReopenedClosed,
    /// Watcher skipped nudging/reporting a link because `paused_at` was set.
    PausedLinkSkipped,
    /// `wf.sweep` inspected a thread's open children, pending launches, and parent link state.
    SweepExecuted,
    /// A worker's PR event or relayable gate fell back to its parent ticket because `relay: false`.
    RelayOffFallback,
    /// `wf.pr ready` fell back to its parent ticket because the worker brief set `mergeOnly: true`.
    MergeOnlyFallback,
    /// `wf.pr` recorded a follow-up PR number on a worker that already tracked a primary PR.
    PrFollowupRecorded,
    /// `wf.pr ready` fell back to its parent ticket because the PR changed more than 100 files.
    PrFilesPaginatedFallback,
    /// `reconcile::fix` reconciled a previously `CLOSED` PR after GitHub showed it reopened (`OPEN`).
    PrReopenedReconciled,
    /// The outbox held a recipient's wakes because its thread was running.
    OutboxHeld,
    /// Watcher reported a link whose thread stayed busy past `stall` minutes with nothing sent.
    StallReported,
    /// Watcher escalated a link to its parent after `miss` unanswered nudges.
    IdleEscalated,
    /// Watcher nudged a link because its latest run ended failed, interrupted or cancelled.
    BadEndNudged,
    /// Watcher closed a worker link after T3 reported its thread gone.
    WorkerThreadGone,
    /// Watcher closed a worker link after T3 reported its thread archived.
    WorkerArchived,
    /// `reconcile::fix` skipped a pull request that stopped being tracked while the batched read was in flight.
    ReconcileDroppedUntracked,
    /// `Store::link_save` found the row changed by another process since it was read and merged field by field.
    LinkSaveMerged,
}

impl ReachPoint {
    pub const ALL: &'static [ReachPoint] = &[
        Self::AdvanceMerged,
        Self::AdvanceEnqueued,
        Self::ReassignedIdRejected,
        Self::MergeEventPredatesTracking,
        Self::CloseEventPredatesTracking,
        Self::FinishedEventReported,
        Self::QueueDropPredatesTracking,
        Self::QueueDropReported,
        Self::TwoReadyCardsOneTicket,
        Self::DeliveryGateOverReadyCard,
        Self::GateClearPreservedReadyCard,
        Self::ReconcileAdoptedPr,
        Self::DismissAtResolved,
        Self::ReadyWaitingRetried,
        Self::ReadyCardDroppedHeadMoved,
        Self::RelayedGateAnswered,
        Self::OutboxHeldThenClosed,
        Self::TicketClosed,
        Self::TicketArchived,
        Self::TicketThreadGone,
        Self::ReplyReopenedClosed,
        Self::PausedLinkSkipped,
        Self::SweepExecuted,
        Self::RelayOffFallback,
        Self::MergeOnlyFallback,
        Self::PrFollowupRecorded,
        Self::PrFilesPaginatedFallback,
        Self::PrReopenedReconciled,
        Self::OutboxHeld,
        Self::StallReported,
        Self::IdleEscalated,
        Self::BadEndNudged,
        Self::WorkerThreadGone,
        Self::WorkerArchived,
        Self::ReconcileDroppedUntracked,
        Self::LinkSaveMerged,
    ];

    pub const COUNT: usize = Self::ALL.len();

    pub const fn name(self) -> &'static str {
        match self {
            Self::AdvanceMerged => "advance_merged",
            Self::AdvanceEnqueued => "advance_enqueued",
            Self::ReassignedIdRejected => "reassigned_id_rejected",
            Self::MergeEventPredatesTracking => "merge_event_predates_tracking",
            Self::CloseEventPredatesTracking => "close_event_predates_tracking",
            Self::FinishedEventReported => "finished_event_reported",
            Self::QueueDropPredatesTracking => "queue_drop_predates_tracking",
            Self::QueueDropReported => "queue_drop_reported",
            Self::TwoReadyCardsOneTicket => "two_ready_cards_one_ticket",
            Self::DeliveryGateOverReadyCard => "delivery_gate_over_ready_card",
            Self::GateClearPreservedReadyCard => "gate_clear_preserved_ready_card",
            Self::ReconcileAdoptedPr => "reconcile_adopted_pr",
            Self::DismissAtResolved => "dismiss_at_resolved",
            Self::ReadyWaitingRetried => "ready_waiting_retried",
            Self::ReadyCardDroppedHeadMoved => "ready_card_dropped_head_moved",
            Self::RelayedGateAnswered => "relayed_gate_answered",
            Self::OutboxHeldThenClosed => "outbox_held_then_closed",
            Self::TicketClosed => "ticket_closed",
            Self::TicketArchived => "ticket_archived",
            Self::TicketThreadGone => "ticket_thread_gone",
            Self::ReplyReopenedClosed => "reply_reopened_closed",
            Self::PausedLinkSkipped => "paused_link_skipped",
            Self::SweepExecuted => "sweep_executed",
            Self::RelayOffFallback => "relay_off_fallback",
            Self::MergeOnlyFallback => "merge_only_fallback",
            Self::PrFollowupRecorded => "pr_followup_recorded",
            Self::PrFilesPaginatedFallback => "pr_files_paginated_fallback",
            Self::PrReopenedReconciled => "pr_reopened_reconciled",
            Self::OutboxHeld => "outbox_held",
            Self::StallReported => "stall_reported",
            Self::IdleEscalated => "idle_escalated",
            Self::BadEndNudged => "bad_end_nudged",
            Self::WorkerThreadGone => "worker_thread_gone",
            Self::WorkerArchived => "worker_archived",
            Self::ReconcileDroppedUntracked => "reconcile_dropped_untracked",
            Self::LinkSaveMerged => "link_save_merged",
        }
    }

    pub const fn tier(self) -> ReachTier {
        match self {
            Self::OutboxHeldThenClosed
            | Self::PrReopenedReconciled
            | Self::BadEndNudged
            | Self::WorkerThreadGone
            | Self::WorkerArchived
            | Self::ReconcileDroppedUntracked
            | Self::LinkSaveMerged => ReachTier::Tier2,
            _ => ReachTier::Tier1,
        }
    }

    pub fn tier1() -> impl Iterator<Item = ReachPoint> {
        Self::ALL.iter().copied().filter(|p| p.tier() == ReachTier::Tier1)
    }
}

/// Per-`Wf` reachability counter registry.
///
/// Uses a fixed-size array of relaxed atomics owned by each `Wf` instance so parallel tests
/// never pollute one another, release builds keep identical control flow with negligible overhead,
/// and release-mode chaos soaks (`cargo test --release`) measure reachability without extra flags.
pub struct Reach {
    counts: [AtomicU64; ReachPoint::COUNT],
    has_adopted: AtomicBool,
    adopted_prs: Mutex<BTreeSet<(String, u64, String)>>,
}

impl Default for Reach {
    fn default() -> Self {
        Self::new()
    }
}

impl Reach {
    pub fn new() -> Self {
        Self {
            counts: std::array::from_fn(|_| AtomicU64::new(0)),
            has_adopted: AtomicBool::new(false),
            adopted_prs: Mutex::new(BTreeSet::new()),
        }
    }

    #[inline]
    pub(crate) fn hit(&self, point: ReachPoint) {
        self.counts[point as usize].fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn note_adopted(&self, repo: &str, pr: u64, dest: &str) {
        let mut set = self.adopted_prs.lock().unwrap_or_else(|e| e.into_inner());
        if set.len() >= MAX_ADOPTED_TRACKED {
            set.pop_first();
        }
        set.insert((repo.to_string(), pr, dest.to_string()));
        self.has_adopted.store(true, Ordering::Release);
    }

    #[inline]
    pub(crate) fn take_adopted(&self, repo: &str, pr: u64, live_tickets: &[(String, String)]) -> bool {
        if !self.has_adopted.load(Ordering::Acquire) {
            return false;
        }
        let mut set = self.adopted_prs.lock().unwrap_or_else(|e| e.into_inner());
        let before = set.len();
        set.retain(|(r, p, dest)| !(r == repo && *p == pr && live_tickets.iter().any(|(t, _)| t == dest)));
        let matched = set.len() < before;
        if set.is_empty() {
            self.has_adopted.store(false, Ordering::Release);
        }
        matched
    }

    pub fn count(&self, point: ReachPoint) -> u64 {
        self.counts[point as usize].load(Ordering::Relaxed)
    }

    pub fn snapshot(&self) -> ReachSnapshot {
        ReachSnapshot { counts: std::array::from_fn(|i| self.counts[i].load(Ordering::Relaxed)) }
    }
}

/// Immutable point-in-time counts read from [`Reach`], mergeable across seeds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReachSnapshot {
    counts: [u64; ReachPoint::COUNT],
}

impl Default for ReachSnapshot {
    fn default() -> Self {
        Self { counts: [0; ReachPoint::COUNT] }
    }
}

impl ReachSnapshot {
    pub fn get(&self, point: ReachPoint) -> u64 {
        self.counts[point as usize]
    }

    pub fn merge(&mut self, other: &ReachSnapshot) {
        for (dst, src) in self.counts.iter_mut().zip(other.counts.iter()) {
            *dst = dst.saturating_add(*src);
        }
    }

    pub fn missing(&self, required: &[ReachPoint]) -> Vec<ReachPoint> {
        required.iter().copied().filter(|&p| self.get(p) == 0).collect()
    }

    pub fn missing_tier1(&self) -> Vec<ReachPoint> {
        ReachPoint::tier1().filter(|&p| self.get(p) == 0).collect()
    }

    pub fn assert_all(&self, required: &[ReachPoint], context: &str) {
        let missing = self.missing(required);
        if !missing.is_empty() {
            let names: Vec<&str> = missing.iter().map(|p| p.name()).collect();
            panic!(
                "reachability check failed in {context}: unreached required branches: [{}]\n{self}",
                names.join(", ")
            );
        }
    }

    pub fn assert_tier1(&self, context: &str) {
        let tier1: Vec<ReachPoint> = ReachPoint::tier1().collect();
        self.assert_all(&tier1, context);
    }
}

impl fmt::Display for ReachSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Reach summary:")?;
        for &p in ReachPoint::ALL {
            let tier = match p.tier() {
                ReachTier::Tier1 => "T1",
                ReachTier::Tier2 => "T2",
            };
            writeln!(f, "  [{tier}] {:<32} {}", p.name(), self.get(p))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_variants_match_their_index_and_have_unique_names() {
        let mut names = BTreeSet::new();
        for (idx, &point) in ReachPoint::ALL.iter().enumerate() {
            assert_eq!(point as usize, idx, "ReachPoint::ALL order drifted at {:?}", point);
            assert!(names.insert(point.name()), "duplicate ReachPoint name {}", point.name());
        }
    }

    #[test]
    fn assert_tier1_fails_when_any_single_tier1_branch_is_unreached() {
        for omitted in ReachPoint::tier1() {
            let reach = Reach::new();
            for p in ReachPoint::tier1() {
                if p != omitted {
                    reach.hit(p);
                }
            }
            let snap = reach.snapshot();
            assert_eq!(snap.missing_tier1(), vec![omitted]);
            let err = std::panic::catch_unwind(|| snap.assert_tier1("unit-test")).unwrap_err();
            let msg = err
                .downcast_ref::<String>()
                .map(String::as_str)
                .or_else(|| err.downcast_ref::<&str>().copied())
                .unwrap_or_default();
            assert!(
                msg.contains(omitted.name()),
                "panic message did not name missing branch {}: {msg}",
                omitted.name()
            );
        }
    }

    #[test]
    fn assert_tier1_passes_when_all_tier1_hit_even_if_tier2_is_zero() {
        let reach = Reach::new();
        for p in ReachPoint::tier1() {
            reach.hit(p);
        }
        let snap = reach.snapshot();
        assert!(snap.missing_tier1().is_empty());
        assert_eq!(snap.get(ReachPoint::OutboxHeldThenClosed), 0);
        assert_eq!(snap.get(ReachPoint::PrReopenedReconciled), 0);
        snap.assert_tier1("tier1-only");
    }

    #[test]
    fn instances_are_isolated_and_snapshots_merge() {
        let a = Reach::new();
        let b = Reach::new();
        a.hit(ReachPoint::AdvanceMerged);
        b.hit(ReachPoint::AdvanceEnqueued);
        b.hit(ReachPoint::AdvanceEnqueued);
        assert_eq!(a.count(ReachPoint::AdvanceEnqueued), 0);
        assert_eq!(b.count(ReachPoint::AdvanceMerged), 0);

        let mut total = ReachSnapshot::default();
        total.merge(&a.snapshot());
        total.merge(&b.snapshot());
        assert_eq!(total.get(ReachPoint::AdvanceMerged), 1);
        assert_eq!(total.get(ReachPoint::AdvanceEnqueued), 2);
    }

    #[test]
    fn adopted_pr_tracking_matches_destination_and_is_bounded() {
        let r = Reach::new();
        let t1 = vec![("mcp:t1".to_string(), "2026-11-17T12:00:00Z".to_string())];
        let t2 = vec![("mcp:t2".to_string(), "2026-11-17T12:00:00Z".to_string())];
        assert!(!r.take_adopted("acme/widgets", 7, &t2));
        r.note_adopted("acme/widgets", 7, "mcp:t2");
        assert!(!r.take_adopted("acme/widgets", 7, &t1));
        assert!(r.take_adopted("acme/widgets", 7, &t2));
        assert!(!r.take_adopted("acme/widgets", 7, &t2));
        for n in 0..(MAX_ADOPTED_TRACKED as u64 + 16) {
            r.note_adopted("acme/widgets", n, "mcp:t2");
        }
        assert!(r.adopted_prs.lock().unwrap().len() <= MAX_ADOPTED_TRACKED);
    }
}
