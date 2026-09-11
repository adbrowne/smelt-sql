use smelt_dialect::SqlDialect;
use smelt_logical::maintenance::availability::{realisable_state_structures, StateStructure};

/// Can `dialect` write the re-run-tolerant merge-ledger bookkeeping record
/// (`docs/specs/incremental_shapes.md` §"The transactional frontier write
/// (merge ledger)")?
///
/// **Derived from the availability layer, never hardcoded** — the same posture
/// as [`super::records_observed_deltas`], and for the same reason: a dialect
/// gaining the structure in `realisable_state_structures` retires its run-layer
/// guards in the commit that lands its emitters, so the plan layer and the run
/// layer cannot drift apart again (`docs/specs/state.md` §"The state-structure
/// inventory"; `docs/outcomes/20260906-bigquery-correctness` decision log,
/// 2026-09-10). A comparison against `SqlDialect::DuckDB` here would instead be
/// a guard the structural census (`tests/state_guard_census.rs`) has to be told
/// about.
///
/// Where this is `false`, the write site **skips the bookkeeping record and
/// proceeds with the write**: the merge-ledger record for an idempotent cell is
/// bookkeeping, not a correctness gate, and the affected cell's own recorded
/// `state_downgrade` is the user-visible channel.
///
/// This answers only the *idempotent* record. The additive fold's
/// never-fold-twice refusal is a different structure
/// (`StateStructure::ReconciliationLedger`), because on a dialect whose
/// `PRIMARY KEY` is unenforced the same table cannot refuse a repeat.
pub fn realises_merge_ledger(dialect: SqlDialect) -> bool {
    realisable_state_structures(dialect).contains(&StateStructure::MergeLedger)
}
