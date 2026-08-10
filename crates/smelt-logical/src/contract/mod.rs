//! The contract lattice (`docs/specs/incremental_models.md` §"The contract
//! lattice"): a declared relaxation of the equivalence invariant is
//! admissible only as a complete single-owner triple — declaration schema,
//! pure oracle transform, and probe emitter — defined here, in
//! `smelt-logical`, never ad hoc by a caller
//! (`docs/outcomes/20260809-contract-lattice-v1/outcome.md`).
//!
//! The declaration *schema* (`smelt_core::config::ContractConfig`) lives one
//! layer down, in `smelt-core`, because `ModelMetadata` must deserialize it
//! and `smelt-core` sits below `smelt-logical` — the single-owner rule binds
//! the *semantics* (validation, oracle transform, probe emitter), not the
//! struct's crate.
//!
//! Landing status per lattice point:
//! - `frozen_horizon`: the complete triple has landed — grain-admissibility
//!   validation, the write-range clamp, and the late-arrival probe emitter
//!   all live in the `frozen_horizon` module.
//! - `deferral`: the complete triple has landed — clock-admissibility
//!   validation, the lag oracle, and the deferral-exceeded probe comparison
//!   all live in the `deferral` module. The two capabilities the point
//!   licenses, run skipping and work subsumption, have also landed —
//!   `deferral::run_license`, `deferral::pending_window`, and
//!   `deferral::subsumption` single-own the licensing decisions;
//!   `smelt-runtime`'s scheduler is a thin builder over them
//!   (`docs/outcomes/20260809-contract-lattice-v1/outcome.md` phase 5).

pub mod deferral;
pub mod frozen_horizon;

/// A point in the contract lattice, restated as the value the conformance
/// gate (and any other oracle-consuming caller) dispatches on
/// (`docs/outcomes/20260809-contract-lattice-v1/phases/06-plan.md`). Day-
/// valued, matching `frozen_horizon`/`deferral`'s own unit-agnostic
/// functions (days, at the current `smelt-runtime`/testkit call sites).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContractPoint {
    /// The equivalence invariant — no relaxation.
    Default,
    /// `contract.frozen_horizon: h` — partitions older than `h` are never
    /// revisited.
    FrozenHorizon { h: i64 },
    /// `contract.deferral: d` — the maintained state may lag its inputs by
    /// up to `d`.
    Deferral { d: i64 },
}

/// What a lattice point's oracle demands of a comparison against the
/// maintained output (`docs/specs/incremental_models.md` §"The contract
/// lattice"): the default point and `frozen_horizon` both compare against a
/// single restricted `S` in both directions ([`restrict_run_window`] is the
/// identity for the default point); `deferral` compares against a bracket of
/// two `S`s instead, one direction each.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OracleObligation {
    /// Both-directions equal to the default point's own (unrestricted) `S`.
    Exact,
    /// Both-directions equal to `S` restricted via [`restrict_run_window`].
    ExactOverRestrictedS,
    /// `full_refresh(S_settled) ⊆ maintained ⊆ full_refresh(S)` — one
    /// `EXCEPT ALL` direction per leg.
    Bracketed,
}

/// The oracle obligation [`point`] licenses. Pure dispatch, no I/O.
pub fn oracle_obligation(point: &ContractPoint) -> OracleObligation {
    match point {
        ContractPoint::Default => OracleObligation::Exact,
        ContractPoint::FrozenHorizon { .. } => OracleObligation::ExactOverRestrictedS,
        ContractPoint::Deferral { .. } => OracleObligation::Bracketed,
    }
}

/// Narrows a run's `[start, end)` window per `point`'s restriction, never
/// widening: `frozen_horizon` delegates to
/// [`frozen_horizon::clamp_write_range`] (the SAME transform the runtime's
/// own write-eligibility clamp calls — this is the "shares one derivation"
/// leg the oracle and the real write clamp cannot drift apart on); every
/// other point (including `deferral`, which restricts by settled cutoff,
/// not by per-run window) is the identity.
pub fn restrict_run_window(point: &ContractPoint, start: i64, end: i64) -> (i64, i64) {
    match point {
        ContractPoint::FrozenHorizon { h } => {
            (frozen_horizon::clamp_write_range(start, end, *h), end)
        }
        ContractPoint::Default | ContractPoint::Deferral { .. } => (start, end),
    }
}

/// The settled cutoff `deferral` licenses being asserted exactly: event time
/// strictly before this point must be fully reflected by the maintained
/// state (`docs/outcomes/20260809-contract-lattice-v1/outcome.md`'s
/// 2026-08-10 "asserted as a bracket" decision). `None` for every
/// non-deferral point — there is nothing to settle.
pub fn settled_cutoff(point: &ContractPoint, input_frontier: i64) -> Option<i64> {
    match point {
        ContractPoint::Deferral { d } => Some(deferral::settled_cutoff(input_frontier, *d)),
        ContractPoint::Default | ContractPoint::FrozenHorizon { .. } => None,
    }
}

#[cfg(test)]
mod point_tests {
    use super::*;

    #[test]
    fn default_point_obligation_is_exact() {
        assert_eq!(
            oracle_obligation(&ContractPoint::Default),
            OracleObligation::Exact
        );
        assert_eq!(
            restrict_run_window(&ContractPoint::Default, 100, 400),
            (100, 400)
        );
        assert_eq!(settled_cutoff(&ContractPoint::Default, 400), None);
    }

    #[test]
    fn frozen_horizon_point_restricts_each_run_window() {
        let point = ContractPoint::FrozenHorizon { h: 90 };
        assert_eq!(
            oracle_obligation(&point),
            OracleObligation::ExactOverRestrictedS
        );
        // Shares its narrowing with `frozen_horizon::clamp_write_range`
        // directly — never a second, independently-written formula.
        assert_eq!(
            restrict_run_window(&point, 0, 400),
            (frozen_horizon::clamp_write_range(0, 400, 90), 400),
        );
        assert_eq!(restrict_run_window(&point, 0, 400), (310, 400));
        // Never widens: a run range shorter than H is unchanged.
        assert_eq!(restrict_run_window(&point, 350, 400), (350, 400));
        assert_eq!(settled_cutoff(&point, 400), None);
    }

    #[test]
    fn deferral_point_is_bracketed_and_does_not_restrict_the_window() {
        let point = ContractPoint::Deferral { d: 6 };
        assert_eq!(oracle_obligation(&point), OracleObligation::Bracketed);
        assert_eq!(restrict_run_window(&point, 100, 400), (100, 400));
        assert_eq!(
            settled_cutoff(&point, 110),
            Some(deferral::settled_cutoff(110, 6))
        );
        assert_eq!(settled_cutoff(&point, 110), Some(104));
    }
}
