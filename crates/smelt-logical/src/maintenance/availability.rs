//! Availability resolution — step 2 of `docs/specs/state.md` §"The
//! degradation contract".
//!
//! Ideal derivation (step 1, everywhere else in [`super`]) assumes every
//! classified state structure is available and picks each cell's best
//! technique. This module is the pure, later step that checks each cell's
//! technique against what is actually realisable for a project — the
//! backend has a builder for the structure, `state.warehouse_tables` has not
//! declared it unavailable — and downgrades a cell whose technique needs a
//! structure that is not realisable to the cheapest recompute-family
//! technique that preserves the equivalence invariant, recording the
//! downgrade on the cell rather than silently substituting it.
//!
//! `smelt-runtime`'s maintenance driver, `smelt explain`, and `smelt-db`'s
//! own `maintenance_plan_diagnostics` (the `MaintenanceStateDowngraded` /
//! `DeclaredContractRequiresState` diagnostics) each call
//! [`resolve_availability`] at their own seam, against their own target
//! dialect(s) — never against the ideal-derivation plan itself. Ideal
//! derivation itself never consults availability: early resolution would
//! violate the degradation contract's two-step shape.

use std::collections::BTreeSet;

use serde::Serialize;

use smelt_core::config::WarehouseTables;
use smelt_dialect::SqlDialect;

use super::{Corner, PlanCell, Technique};

/// The [`StateStructure`]s `dialect` has a builder for, independent of
/// `state.warehouse_tables`. Exhaustive over [`SqlDialect`]: a new dialect
/// is a compile error here, not a silent default. Today only DuckDB has a
/// ledger builder (`smelt-state/src/ddl_duckdb.rs`); every dialect realises
/// the sidecar/output-delta structures, which have no per-dialect builder
/// gate (`sources.md` §"The fingerprint sidecar", `incremental_models.md`
/// §"The graph layer").
pub fn realisable_state_structures(dialect: SqlDialect) -> Vec<StateStructure> {
    match dialect {
        SqlDialect::DuckDB => vec![
            StateStructure::MergeLedger,
            StateStructure::ReconciliationLedger,
            StateStructure::ObservedOutputDeltas,
            StateStructure::FingerprintSidecar,
        ],
        SqlDialect::SparkSQL | SqlDialect::PostgreSQL | SqlDialect::BigQuery => vec![
            StateStructure::ObservedOutputDeltas,
            StateStructure::FingerprintSidecar,
        ],
    }
}

/// A persistent structure the maintenance plan may depend on, classified as
/// "correctness" and engine-resident by `docs/specs/state.md` §"The
/// state-structure inventory". Spellings match that table's rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum StateStructure {
    /// The transactional merge ledger (`incremental_shapes.md` §"The
    /// transactional frontier write (merge ledger)").
    MergeLedger,
    /// The reconciliation ledger / frontier record (`incremental_models.md`
    /// §"The frontier record (reconciliation ledger)").
    ReconciliationLedger,
    /// Observed output deltas (`incremental_models.md` §"The graph layer").
    ObservedOutputDeltas,
    /// The fingerprint sidecar (`sources.md` §"The fingerprint sidecar").
    FingerprintSidecar,
}

impl StateStructure {
    /// Spelling used in `MaintenanceStateDowngraded`'s rendered reason and
    /// `smelt explain --json` (`state.md` §"The state-structure inventory").
    pub fn as_str(&self) -> &'static str {
        match self {
            StateStructure::MergeLedger => "transactional merge ledger",
            StateStructure::ReconciliationLedger => "reconciliation ledger (frontier record)",
            StateStructure::ObservedOutputDeltas => "observed output deltas",
            StateStructure::FingerprintSidecar => "fingerprint sidecar",
        }
    }
}

/// The state structure a technique needs to be correct, or `None` for the
/// recompute family (`DeleteInsert`/`PerGroupRecompute`), which needs no
/// bookkeeping to be correct. Exhaustive over [`Technique`]: a new variant is
/// a compile error here, not a silently-unclassified technique.
pub fn required_state_structure(technique: Technique) -> Option<StateStructure> {
    match technique {
        Technique::KeyedFold => Some(StateStructure::ReconciliationLedger),
        Technique::ColumnScopedMerge | Technique::InPlaceUpdate => {
            Some(StateStructure::MergeLedger)
        }
        Technique::DeleteInsert | Technique::PerGroupRecompute => None,
    }
}

/// The set of [`StateStructure`]s available to a project's availability
/// resolution — the intersection of what the target backend can realise and
/// what `state.warehouse_tables` permits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateAvailability {
    available: BTreeSet<StateStructure>,
}

impl StateAvailability {
    /// Every structure available (a backend with every builder, under
    /// `warehouse_tables: allowed`).
    pub fn all() -> Self {
        StateAvailability {
            available: [
                StateStructure::MergeLedger,
                StateStructure::ReconciliationLedger,
                StateStructure::ObservedOutputDeltas,
                StateStructure::FingerprintSidecar,
            ]
            .into_iter()
            .collect(),
        }
    }

    /// No structure available (`warehouse_tables: none`, or a backend with no
    /// builders at all).
    pub fn none() -> Self {
        StateAvailability {
            available: BTreeSet::new(),
        }
    }

    /// The availability a project actually has: `realisable` is the set the
    /// target backend has a builder for; `warehouse_tables: none` overrides
    /// that to the empty set regardless of what the backend could realise
    /// (`state.md` §"Opting out of warehouse bookkeeping").
    pub fn resolve(warehouse_tables: WarehouseTables, realisable: &[StateStructure]) -> Self {
        match warehouse_tables {
            WarehouseTables::None => StateAvailability::none(),
            WarehouseTables::Allowed => StateAvailability {
                available: realisable.iter().copied().collect(),
            },
        }
    }

    pub fn contains(&self, structure: StateStructure) -> bool {
        self.available.contains(&structure)
    }
}

/// The recorded downgrade on a [`super::PlanCell`] (`state.md`
/// §"Diagnostics" `MaintenanceStateDowngraded`): the technique ideal
/// derivation chose, the structure that was missing, and a rendered reason
/// `smelt explain` prints alongside the technique that actually ran.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StateDowngrade {
    pub original: Technique,
    pub missing: StateStructure,
    pub reason: String,
}

/// The cheapest recompute-family technique that preserves the equivalence
/// invariant for `cell` (`state.md` §"The degradation contract"): a
/// targeted-write cell (`Corner::FoldDelta`, `Corner::ColumnMerge`, or one
/// carrying a `key_scope`) downgrades to `PerGroupRecompute`; a region-write
/// cell (`Corner::RmwRegion`, `Corner::RecomputeRegion`) downgrades to
/// `DeleteInsert`.
pub fn recompute_equivalent(cell: &PlanCell) -> Technique {
    if cell.key_scope.is_some() {
        return Technique::PerGroupRecompute;
    }
    match cell.corner {
        Corner::FoldDelta | Corner::ColumnMerge => Technique::PerGroupRecompute,
        Corner::RmwRegion | Corner::RecomputeRegion => Technique::DeleteInsert,
    }
}

/// Availability resolution (`state.md` §"The degradation contract" step 2):
/// for each cell of `cells` whose ideal technique needs a structure not in
/// `available`, swap the technique for [`recompute_equivalent`] and record
/// the downgrade. Idempotent: a cell that already carries a downgrade is
/// left untouched, so the recorded `original` always names the technique
/// ideal derivation actually chose, never an already-downgraded one.
pub fn resolve_availability(cells: &mut [PlanCell], available: &StateAvailability) {
    for cell in cells.iter_mut() {
        if cell.state_downgrade.is_some() {
            continue;
        }
        let Some(required) = required_state_structure(cell.technique) else {
            continue;
        };
        if available.contains(required) {
            continue;
        }
        let original = cell.technique;
        let replacement = recompute_equivalent(cell);
        cell.state_downgrade = Some(StateDowngrade {
            original,
            missing: required,
            reason: format!(
                "{:?} requires the {}, which is unavailable for this project; downgraded to \
                 {:?}, the cheapest recompute-family technique that preserves the equivalence \
                 invariant",
                original,
                required.as_str(),
                replacement
            ),
        });
        cell.technique = replacement;
    }
}
