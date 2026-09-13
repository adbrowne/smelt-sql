use serde::Serialize;

use smelt_dialect::SqlDialect;

use crate::maintenance::{KeyDiscovery, PlanCell, Technique};

/// The [`StateStructure`]s `dialect` has a builder for, independent of
/// `state.warehouse_tables`. Exhaustive over [`SqlDialect`]: a new dialect
/// is a compile error here, not a silent default.
///
/// **Every structure named here must have emitters and a backend seam behind
/// it** (`docs/specs/state.md` §"The state-structure inventory"). Naming one
/// that does not is worse than omitting it: `resolve_availability` records no
/// downgrade for a structure it believes available, so the run reaches a
/// DuckDB-only guard and refuses instead of degrading. That is exactly the
/// defect this list carried until 2026-09-10, when it claimed
/// `ObservedOutputDeltas` and `FingerprintSidecar` for BigQuery and Spark on
/// the reasoning that they "have no per-dialect builder gate" — they have no
/// per-dialect *builder*, which is the opposite conclusion.
///
/// DuckDB has a builder for all five: every ledger, sidecar and observed-delta
/// emitter lives in `smelt-state/src/ddl_duckdb.rs`, and it is the only backend
/// overriding all of the transactional seams those structures need
/// (`Backend::fold_ledger_delta`, `execute_write_with_bookkeeping`,
/// `execute_conditional_write_and_record_observed_delta`). BigQuery has both
/// ledgers and the observed-delta record: `smelt-state/src/ddl_bigquery/`
/// carries their GoogleSQL spellings (a `MERGE … WHEN NOT MATCHED` upsert,
/// since GoogleSQL has no `ON CONFLICT`; `ARRAY_AGG(DISTINCT CAST(… AS STRING)
/// IGNORE NULLS)` for the delta's key set, since GoogleSQL has no `FILTER`
/// clause, raises on a NULL array element, and coerces no element type on
/// write) beside its schema-evolution DDL, dispatched by
/// `smelt_state::ledger` and `smelt_state::observed_delta`, and
/// `smelt-backend-bigquery` overrides `execute_write_with_bookkeeping` and
/// `fold_ledger_delta` with real multi-statement transactions. Its
/// `ReconciliationLedger` row is on for a reason that took re-deriving rather
/// than porting: the never-fold-twice refusal is a `PRIMARY KEY` *violation*
/// on DuckDB, and BigQuery's `PRIMARY KEY` is declared `NOT ENFORCED`, so
/// there the guarantee is a zero-row abort — the record statement is a
/// `MERGE … WHEN NOT MATCHED`, and `@@row_count = 0` raises inside the same
/// transaction that holds the fold action
/// (`smelt_backend_bigquery::sql::fold_ledger_delta_script`). The guarantee is
/// one guarantee with two realisations, not two guarantees.
/// `ddl_spark.rs` carries schema-evolution DDL only.
/// The fingerprint sidecar has a second, independent source of truth agreeing
/// with this: every consumer in `maintenance_driver/sidecar.rs` gates on
/// `BackendCapabilities::supports_fingerprint_sidecar`, `true` for DuckDB
/// alone.
///
/// BigQuery's remaining two rows are pending work: the fingerprint sidecar
/// (whose capability flag above is the second source of truth that must flip
/// with it) and the tombstone ledger
/// (`docs/outcomes/20260906-bigquery-correctness` phase 15). They flip on as
/// each realisation lands. **Spark's are not**:
/// Delta gives per-table atomicity and no cross-table transaction, so a ledger
/// write and its data write cannot be made atomic there and the additive
/// never-fold-twice refusal has no sound Delta realisation — an honest
/// permanent absence, not a deferral.
pub fn realisable_state_structures(dialect: SqlDialect) -> Vec<StateStructure> {
    match dialect {
        SqlDialect::DuckDB => vec![
            StateStructure::MergeLedger,
            StateStructure::ReconciliationLedger,
            StateStructure::ObservedOutputDeltas,
            StateStructure::FingerprintSidecar,
            StateStructure::TombstoneLedger,
        ],
        SqlDialect::BigQuery => vec![
            StateStructure::MergeLedger,
            StateStructure::ReconciliationLedger,
            StateStructure::ObservedOutputDeltas,
            StateStructure::TombstoneLedger,
        ],
        SqlDialect::SparkSQL => vec![],
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
    /// state)"). Realised on DuckDB and BigQuery; refused on Spark, where
    /// Delta's lack of a cross-table transaction leaves the tombstone record
    /// and the presented `MERGE` unable to commit together. A
    /// `SuccessionPatch` cell with no realisable ledger downgrades to
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

/// The state structure `cell` needs to be correct, or `None` for a cell
/// needing no bookkeeping. The requirement is a function of the **cell**, not
/// of its technique alone (`docs/specs/state.md` §"The degradation
/// contract" step 2): every technique other than the recompute family
/// (`DeleteInsert`/`PerGroupRecompute`) requires its own structure
/// unconditionally, but a `PerGroupRecompute` cell requires the
/// **fingerprint sidecar** exactly when it is addressed by a key-addressed
/// model edge — its affected-key discovery is a group-grain sidecar diff
/// (`crate::maintenance::repair::admit_key_addressed_recompute`,
/// `incremental_models.md` §"Upstream model edges"). A `PerGroupRecompute`
/// cell with no `key_scope` (a plain clamp-bounded repair) needs nothing.
///
/// The two `KeyDiscovery` routes that admit a `PerGroupRecompute` cell
/// (`UpstreamKeyed`, `DownstreamGrainOverUpstream`) both need the sidecar;
/// `EnrichmentKeyed` never reaches this arm, since it only ever addresses a
/// `ColumnScopedMerge` cell (already covered by the merge-ledger arm below).
/// Exhaustive over both [`Technique`] and, for the `PerGroupRecompute` arm,
/// [`KeyDiscovery`] — a new technique or discovery route is a compile error
/// here, not a silently-unclassified one.
///
/// This is the single source of truth for the requirement:
/// [`realisable_state_structures`] and
/// `BackendCapabilities::supports_fingerprint_sidecar` are the two
/// realisation-side facts it must agree with, and no consumer may
/// re-derive the requirement at run time (`maintenance-plan purity`) — a
/// run-time check for the same fact is a defensive guard against
/// inconsistent inputs, never a second source of truth.
pub fn required_state_structure(cell: &PlanCell) -> Option<StateStructure> {
    match cell.technique {
        Technique::KeyedFold => Some(StateStructure::ReconciliationLedger),
        Technique::ColumnScopedMerge | Technique::InPlaceUpdate => {
            Some(StateStructure::MergeLedger)
        }
        Technique::SuccessionPatch => Some(StateStructure::TombstoneLedger),
        Technique::DeleteInsert => None,
        Technique::PerGroupRecompute => match &cell.key_scope {
            None => None,
            Some(key_scope) => match key_scope.discovery {
                KeyDiscovery::UpstreamKeyed | KeyDiscovery::DownstreamGrainOverUpstream => {
                    Some(StateStructure::FingerprintSidecar)
                }
                KeyDiscovery::EnrichmentKeyed => None,
            },
        },
    }
}
