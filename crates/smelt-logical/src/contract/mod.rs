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

use smelt_core::config::{ContractCellConfig, ContractConfig, DataLatency};

/// Which declaration a cell's effective `deferral` window came from — the
/// narrower-wins ladder [`effective_contract`] resolves, mirroring
/// `maintenance::choice`'s own model-vs-cell override reporting
/// (`docs/outcomes/20260809-contract-lattice-v1/phases/07-plan.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeferralOrigin {
    /// The model-level `contract.deferral` default.
    Model,
    /// A `contract.cells[]` entry narrowing the model default for this cell.
    Cell,
}

/// A cell's effective `deferral` window plus which declaration it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveDeferral {
    pub window: DataLatency,
    pub origin: DeferralOrigin,
}

/// The effective contract lattice point(s) that apply to one maintenance
/// cell — default (both fields `None`) or the applicable relaxations with
/// their declared parameters (`docs/specs/incremental_models.md` §"The
/// contract lattice"). `frozen_horizon` is always model-level (it clamps the
/// model's write eligibility, never a single cell's); `deferral` may be
/// narrowed per cell.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EffectiveContract {
    pub frozen_horizon: Option<DataLatency>,
    pub deferral: Option<EffectiveDeferral>,
}

impl EffectiveContract {
    /// True when neither relaxation applies — the default point.
    pub fn is_default(&self) -> bool {
        self.frozen_horizon.is_none() && self.deferral.is_none()
    }

    /// A one-line description used by both the text and JSON renderings
    /// (`smelt-cli::explain`): `"default"`, or the applicable relaxations
    /// joined by `, `, a cell-level `deferral` labelled `(cell)`.
    pub fn render_label(&self) -> String {
        if self.is_default() {
            return "default".to_string();
        }
        let mut parts = Vec::new();
        if let Some(h) = &self.frozen_horizon {
            parts.push(format!("frozen_horizon {}", h.display));
        }
        if let Some(d) = &self.deferral {
            match d.origin {
                DeferralOrigin::Model => parts.push(format!("deferral {}", d.window.display)),
                DeferralOrigin::Cell => parts.push(format!("deferral {} (cell)", d.window.display)),
            }
        }
        parts.join(", ")
    }
}

/// Match one `contract.cells[]` entry against `trigger_address` and
/// `group_columns`, mirroring `maintenance::choice`'s own `matching_cell`
/// addressing semantics ("names any member", not "equals exactly").
fn matching_contract_cell<'a>(
    cells: &'a [ContractCellConfig],
    trigger_address: &str,
    group_columns: &[String],
) -> Option<&'a ContractCellConfig> {
    cells.iter().find(|c| {
        c.on == trigger_address
            && c.columns
                .iter()
                .any(|col| group_columns.iter().any(|g| g == col))
    })
}

/// Resolve the effective contract for one cell: `cfg`'s model-level
/// `frozen_horizon` applies unconditionally (there is no per-cell
/// refinement); `deferral` applies a `contract.cells[]` match if present,
/// else the model-level default, else `None`. `cfg` absent (no `contract:`
/// block) resolves to the default point.
pub fn effective_contract(
    cfg: Option<&ContractConfig>,
    trigger_address: &str,
    group_columns: &[String],
) -> EffectiveContract {
    let Some(cfg) = cfg else {
        return EffectiveContract::default();
    };
    let cell_deferral = matching_contract_cell(&cfg.cells, trigger_address, group_columns)
        .and_then(|c| c.deferral.as_ref());
    let deferral = match cell_deferral {
        Some(window) => Some(EffectiveDeferral {
            window: window.clone(),
            origin: DeferralOrigin::Cell,
        }),
        None => cfg.deferral.as_ref().map(|window| EffectiveDeferral {
            window: window.clone(),
            origin: DeferralOrigin::Model,
        }),
    };
    EffectiveContract {
        frozen_horizon: cfg.frozen_horizon.clone(),
        deferral,
    }
}

#[cfg(test)]
mod effective_contract_tests {
    use super::*;

    #[test]
    fn effective_contract_defaults_to_the_default_point() {
        let effective = effective_contract(None, "sources.raw.events", &["amount".to_string()]);
        assert_eq!(effective, EffectiveContract::default());
        assert!(effective.is_default());
        assert_eq!(effective.render_label(), "default");
    }

    #[test]
    fn effective_contract_applies_model_level_frozen_horizon_to_every_cell() {
        let cfg = ContractConfig {
            frozen_horizon: DataLatency::parse("90 days"),
            deferral: None,
            cells: vec![],
        };
        let effective = effective_contract(Some(&cfg), "backfill", &[]);
        assert_eq!(effective.frozen_horizon, DataLatency::parse("90 days"));
        assert_eq!(effective.deferral, None);
        assert_eq!(effective.render_label(), "frozen_horizon 90 days");

        // Reaches a cell regardless of its trigger/columns.
        let other = effective_contract(Some(&cfg), "sources.raw.events", &["revenue".to_string()]);
        assert_eq!(other.frozen_horizon, DataLatency::parse("90 days"));
    }

    #[test]
    fn effective_contract_cell_deferral_overrides_the_model_default() {
        let cfg = ContractConfig {
            frozen_horizon: None,
            deferral: DataLatency::parse("6 hours"),
            cells: vec![ContractCellConfig {
                columns: vec!["amount".to_string()],
                on: "sources.raw.events".to_string(),
                deferral: DataLatency::parse("1 day"),
            }],
        };
        let effective = effective_contract(
            Some(&cfg),
            "sources.raw.events",
            &["amount".to_string(), "user_id".to_string()],
        );
        assert_eq!(effective.render_label(), "deferral 1 day (cell)");
        let deferral = effective.deferral.expect("deferral applies");
        assert_eq!(deferral.window, DataLatency::parse("1 day").unwrap());
        assert_eq!(deferral.origin, DeferralOrigin::Cell);
    }

    #[test]
    fn effective_contract_non_matching_cell_entry_keeps_the_model_default() {
        let cfg = ContractConfig {
            frozen_horizon: None,
            deferral: DataLatency::parse("6 hours"),
            cells: vec![ContractCellConfig {
                columns: vec!["other_column".to_string()],
                on: "sources.raw.events".to_string(),
                deferral: DataLatency::parse("1 day"),
            }],
        };
        let effective =
            effective_contract(Some(&cfg), "sources.raw.events", &["amount".to_string()]);
        let deferral = effective.deferral.expect("model-level deferral applies");
        assert_eq!(deferral.window, DataLatency::parse("6 hours").unwrap());
        assert_eq!(deferral.origin, DeferralOrigin::Model);

        // A different `on:` trigger also keeps the model default.
        let different_trigger =
            effective_contract(Some(&cfg), "backfill", &["other_column".to_string()]);
        assert_eq!(
            different_trigger.deferral.map(|d| d.origin),
            Some(DeferralOrigin::Model)
        );
    }
}

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
