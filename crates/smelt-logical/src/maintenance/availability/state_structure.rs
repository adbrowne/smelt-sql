use serde::Serialize;

use smelt_dialect::SqlDialect;

use crate::maintenance::Technique;

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
            StateStructure::TombstoneLedger,
        ],
        SqlDialect::SparkSQL | SqlDialect::BigQuery => vec![
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
    /// The succession grain's tombstone ledger — a per-model sibling table
    /// holding `k ∪ {t}` for every recorded delete event
    /// (`docs/specs/state.md`'s tombstone ledger row,
    /// `docs/specs/incremental_shapes.md` §"The tombstone ledger (hidden
    /// state)"). DuckDB-only today (`realisable_state_structures`): a
    /// `TombstonePatch` cell with no realisable ledger downgrades to
    /// `DeleteInsert` (full refresh), never a ledger-less patch.
    TombstoneLedger,
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
            StateStructure::TombstoneLedger => "tombstone ledger",
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
        Technique::SuccessionPatch => Some(StateStructure::TombstoneLedger),
        Technique::DeleteInsert | Technique::PerGroupRecompute => None,
    }
}
