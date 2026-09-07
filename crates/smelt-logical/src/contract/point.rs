use super::{deferral, frozen_horizon};

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
/// identity for the default point); `deferral` restates to strict equality
/// over the processed set `S` PLUS a lag bound over what has landed but not
/// yet been processed (`deferral::settled_lag_bound`) — the spec's own two
/// obligations, not the superseded bracket
/// (`full_refresh(S_settled) ⊆ maintained ⊆ full_refresh(S)`), which held
/// vacuously whenever the settled cutoff preceded all recorded event time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OracleObligation {
    /// Both-directions equal to the default point's own (unrestricted) `S`.
    Exact,
    /// Both-directions equal to `S` restricted via [`restrict_run_window`].
    ExactOverRestrictedS,
    /// Both-directions equal to `S` (identical to [`OracleObligation::Exact`]
    /// on the equality leg — `deferral` does not restrict the run window),
    /// PLUS every landed-but-unprocessed event time must be at or after the
    /// settled cutoff (`deferral::settled_lag_bound`).
    ExactOverProcessedSWithLagBound,
}

/// The oracle obligation [`point`] licenses. Pure dispatch, no I/O.
pub fn oracle_obligation(point: &ContractPoint) -> OracleObligation {
    match point {
        ContractPoint::Default => OracleObligation::Exact,
        ContractPoint::FrozenHorizon { .. } => OracleObligation::ExactOverRestrictedS,
        ContractPoint::Deferral { .. } => OracleObligation::ExactOverProcessedSWithLagBound,
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

/// The settled cutoff below which `deferral::settled_lag_bound` demands every
/// landed event time be processed (`incremental_models.md` §"The contract
/// lattice"). Also retained as the superseded bracket's lower-leg boundary
/// (`s_at_settled` in `smelt-maintenance-testkit`) purely as the vacuity
/// witness the 2026-09-05 restatement acts on
/// (`docs/outcomes/20260809-contract-lattice-v1/outcome.md`'s 2026-08-10
/// "asserted as a bracket" decision is superseded —
/// `docs/outcomes/20260904-decided-gap-residue/phases/02-plan.md`). `None`
/// for every non-deferral point — there is nothing to settle.
pub fn settled_cutoff(point: &ContractPoint, input_frontier: i64) -> Option<i64> {
    match point {
        ContractPoint::Deferral { d } => Some(deferral::settled_cutoff(input_frontier, *d)),
        ContractPoint::Default | ContractPoint::FrozenHorizon { .. } => None,
    }
}

/// The state structure `point`'s semantics require to be correct
/// (`state.md` §Diagnostics `DeclaredContractRequiresState`) — part of the
/// point's own definition, per the contract-lattice point single-ownership
/// invariant (`CLAUDE.md` §"Contract-lattice point single ownership"): a
/// caller must never decide this ad hoc. `deferral`'s settled cutoff is
/// measured against the reconciliation ledger's frontier record
/// (`run_state.md` §"Relationship to the reconciliation ledger"); the
/// default point and `frozen_horizon` need no state structure at all.
/// Exhaustive over [`ContractPoint`]: a new point is a compile error here,
/// not a silently-unclassified one.
pub fn required_state_structure(
    point: &ContractPoint,
) -> Option<crate::maintenance::availability::StateStructure> {
    match point {
        ContractPoint::Deferral { .. } => {
            Some(crate::maintenance::availability::StateStructure::ReconciliationLedger)
        }
        ContractPoint::Default | ContractPoint::FrozenHorizon { .. } => None,
    }
}

#[cfg(test)]
mod point_tests {
    use super::*;
    use crate::maintenance::availability::StateStructure;

    #[test]
    fn deferral_requires_the_reconciliation_ledger() {
        assert_eq!(
            required_state_structure(&ContractPoint::Deferral { d: 6 }),
            Some(StateStructure::ReconciliationLedger)
        );
        assert_eq!(required_state_structure(&ContractPoint::Default), None);
        assert_eq!(
            required_state_structure(&ContractPoint::FrozenHorizon { h: 90 }),
            None
        );
    }

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
    fn deferral_obligation_is_exact_over_processed_s_with_lag_bound() {
        let point = ContractPoint::Deferral { d: 6 };
        assert_eq!(
            oracle_obligation(&point),
            OracleObligation::ExactOverProcessedSWithLagBound
        );
        // The equality leg does not restrict the run window either.
        assert_eq!(restrict_run_window(&point, 100, 400), (100, 400));
        assert_eq!(
            settled_cutoff(&point, 110),
            Some(deferral::settled_cutoff(110, 6))
        );
        assert_eq!(settled_cutoff(&point, 110), Some(104));
    }
}
