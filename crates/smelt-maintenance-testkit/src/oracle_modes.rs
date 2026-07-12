//! Oracle mode selection for mixed models
//! (`docs/plans/20260712-generative-maintenance-conformance.md` Phase 4;
//! design doc `docs/research/20260711-generative-maintenance-conformance.md`
//! §6 "The equivalence oracle, generalized" — "Mixed models"): the pure
//! `OracleMode` enum a mixed (append-only fact + mutable dimension) case
//! selects between. Selection itself is one named type so the choice isn't
//! duplicated ad hoc at each call site; the bookkeeping of *when* a mutation
//! becomes outstanding and *when* a catch-up run clears it lives on
//! [`crate::s_tracker::STracker`] (it owns per-run/per-window state already,
//! design §6: "The S-tracker owns this bookkeeping").

#![allow(dead_code)]

/// Which equivalence discipline currently applies to a tracked model
/// (design §6 "Mixed models").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OracleMode {
    /// No mutation is outstanding against any region the driving source has
    /// processed: full S-restricted equivalence is assertable right now
    /// (Phase 3's append-only discipline; for a mixed model the dimension
    /// additionally contributes its CURRENT physical state — design §6).
    SRestricted,
    /// A mutable source's mutation is outstanding against a region no
    /// catch-up run has yet re-covered: full equality only holds at the next
    /// **settled point** (the catch-up run covering that region). Between
    /// now and then, only the weaker expected-staleness contract holds, and
    /// it is asserted non-fatally, never as a hard failure (design §6).
    SettledPoint,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two variants compare and print distinctly — a smoke test that the
    /// enum itself is usable as a `match` target the way `STracker::oracle_mode`
    /// and its callers rely on.
    #[test]
    fn oracle_mode_variants_are_distinct() {
        assert_ne!(OracleMode::SRestricted, OracleMode::SettledPoint);
    }
}
