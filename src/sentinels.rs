#[cfg(feature = "test-sentinels")]
use std::sync::atomic::{AtomicU64, Ordering};

/// Runtime-togglable historical regressions planted for chaos oracle grading.
///
/// Gated behind the `test-sentinels` Cargo feature: with the feature off,
/// [`Sentinels`] is zero-sized and [`Sentinels::is_set`] compiles to a constant
/// `false` so production binaries pay zero cost.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum Sentinel {
    /// `Wf::board_card` supersedes open ready merge cards when another ticket-owned gate opens.
    BoardCardWipesReady = 0,
    /// `Wf::close` leaves `gate`, `gate_since`, and `gate_pr` set on the closed link.
    CloseKeepsGate,
    /// `Wf::gate({ clear: true })` wipes every ticket-owned gate card, including other unanswered ready merge cards.
    GateClearWipesOtherReady,
    /// `reconcile::live_tickets` ignores tickets previously recorded on `pr_seen`.
    ReconcileSkipsIndependentPrSeen,
    /// Link closures skip outbox purges and `flush_outbox` skips its closed-link drop guard.
    OutboxNoPurge,
    /// Retiring a `MergeWait` deletes unconditionally by `(repo, pr)` instead of CAS-matching `approval`.
    MergeWaitRemoveNotCas,
    /// `reconcile::advance` ignores a `false` CAS result from `merge_wait_swap`.
    AdvanceIgnoresSwapResult,
}

impl Sentinel {
    pub const ALL: &'static [Sentinel] = &[
        Self::BoardCardWipesReady,
        Self::CloseKeepsGate,
        Self::GateClearWipesOtherReady,
        Self::ReconcileSkipsIndependentPrSeen,
        Self::OutboxNoPurge,
        Self::MergeWaitRemoveNotCas,
        Self::AdvanceIgnoresSwapResult,
    ];

    pub const fn bit(self) -> u64 {
        1u64 << (self as u8)
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::BoardCardWipesReady => "board_card_wipes_ready",
            Self::CloseKeepsGate => "close_keeps_gate",
            Self::GateClearWipesOtherReady => "gate_clear_wipes_other_ready",
            Self::ReconcileSkipsIndependentPrSeen => "reconcile_skips_independent_pr_seen",
            Self::OutboxNoPurge => "outbox_no_purge",
            Self::MergeWaitRemoveNotCas => "merge_wait_remove_not_cas",
            Self::AdvanceIgnoresSwapResult => "advance_ignores_swap_result",
        }
    }
}

/// Per-`Wf` bitmask of active runtime sentinels.
#[cfg(feature = "test-sentinels")]
pub struct Sentinels {
    bits: AtomicU64,
}

#[cfg(feature = "test-sentinels")]
impl Default for Sentinels {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "test-sentinels")]
impl Sentinels {
    pub const fn new() -> Self {
        Self { bits: AtomicU64::new(0) }
    }

    #[inline(always)]
    pub fn is_set(&self, s: Sentinel) -> bool {
        (self.bits.load(Ordering::Relaxed) & s.bit()) != 0
    }

    pub fn set(&self, s: Sentinel) {
        self.bits.fetch_or(s.bit(), Ordering::Relaxed);
    }
}

/// Zero-sized no-op sentinel registry compiled when `test-sentinels` is off.
#[cfg(not(feature = "test-sentinels"))]
#[derive(Default)]
pub struct Sentinels;

#[cfg(not(feature = "test-sentinels"))]
impl Sentinels {
    pub const fn new() -> Self {
        Self
    }

    #[inline(always)]
    pub fn is_set(&self, _: Sentinel) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn all_variants_have_distinct_bits_and_names() {
        let mut names = BTreeSet::new();
        let mut mask = 0u64;
        for (idx, &s) in Sentinel::ALL.iter().enumerate() {
            assert_eq!(s as usize, idx, "Sentinel::ALL order drifted at {s:?}");
            assert!((s as u8) < 64);
            assert_eq!(mask & s.bit(), 0, "duplicate bit for {s:?}");
            mask |= s.bit();
            assert!(names.insert(s.name()), "duplicate name {}", s.name());
        }
    }

    #[test]
    fn default_sentinels_are_all_off() {
        let sentinels = Sentinels::new();
        for &s in Sentinel::ALL {
            assert!(!sentinels.is_set(s), "{s:?} should default to off");
        }
    }

    #[cfg(not(feature = "test-sentinels"))]
    #[test]
    fn sentinels_is_zero_sized_when_feature_is_off() {
        assert_eq!(std::mem::size_of::<Sentinels>(), 0);
    }

    #[cfg(feature = "test-sentinels")]
    #[test]
    fn sentinels_toggle_independently_and_isolate_instances() {
        for &s in Sentinel::ALL {
            let a = Sentinels::new();
            let b = Sentinels::new();
            a.set(s);
            assert!(a.is_set(s));
            assert!(!b.is_set(s));
            for &other in Sentinel::ALL {
                if other != s {
                    assert!(!a.is_set(other), "{other:?} unexpectedly set alongside {s:?}");
                }
            }
        }
        let multi = Sentinels::new();
        let first = Sentinel::BoardCardWipesReady;
        let second = Sentinel::OutboxNoPurge;
        multi.set(first);
        multi.set(second);
        assert!(multi.is_set(first));
        assert!(multi.is_set(second));
        for &other in Sentinel::ALL {
            if other != first && other != second {
                assert!(!multi.is_set(other), "{other:?} unexpectedly set alongside {first:?} and {second:?}");
            }
        }
    }
}
