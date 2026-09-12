//! The region-recompute reconciliation-ledger reset, and the `CREATE TABLE
//! IF NOT EXISTS` that must precede it.
//!
//! Extracted from `mod.rs` so one derivation serves all three DeleteInsert
//! dispatch arms (model-edge restricted, external-sidecar restricted, plain)
//! and so `mod.rs` — already the workspace's largest file — does not carry
//! it.

use anyhow::Result;
use smelt_dialect::SqlDialect;
use smelt_logical::maintenance::availability::{StateAvailability, StateStructure};

/// The reset statements for one region recompute, plus the ledger DDL that
/// must exist first.
#[derive(Debug, Default, Clone)]
pub(super) struct LedgerReset {
    /// `CREATE TABLE IF NOT EXISTS <ledger>`, or empty when there is no
    /// reset to protect.
    pub ensure_sqls: Vec<String>,
    /// The reset itself: `DELETE` every intersecting entry, then `INSERT`
    /// exactly the input this recompute read. Empty when the reconciliation
    /// ledger is unrealisable on this target.
    pub pre_write_sqls: Vec<String>,
    /// Whether a reset was built at all — the caller reports the skip.
    pub built: bool,
}

/// Build the reset for a region `[start, end)` of `model`.
///
/// The record is the whole-row group `{*}` under the nominal input `self`,
/// watermarked to the region's own end, and runs in the SAME backend
/// transaction as the write it protects (via
/// `Backend::execute_write_with_bookkeeping` — state residency: the ledger
/// table *is* the state, `.smelt/reconciliation.json` no longer exists).
///
/// Two conditions suppress it, and neither is a dialect comparison:
///
/// - `column_merge_dispatch` — a live `ColumnScopedMerge` cell is not a
///   region DeleteInsert, and its own ledger interaction is unrelated.
/// - the run's resolved [`StateAvailability`] not carrying
///   `ReconciliationLedger`. On a ledger-less target this is not a silent
///   skip: the per-cell technique already carries a recorded
///   `state_downgrade` (`smelt-logical`'s `resolve_availability`), which
///   `smelt explain` and the warning diagnostic surface.
///
/// The statements come from `smelt_state::ledger`, the one dialect dispatch
/// point, never a named per-dialect builder — this path became reachable on
/// BigQuery the moment the reconciliation ledger was declared realisable
/// there, and DuckDB's spelling in a BigQuery job fails at the warehouse
/// (`Type not found: VARCHAR`).
pub(super) fn build(
    dialect: SqlDialect,
    availability: &StateAvailability,
    column_merge_dispatch_is_some: bool,
    schema: &str,
    model: &str,
    region_start: &str,
    region_end: &str,
) -> Result<LedgerReset> {
    if column_merge_dispatch_is_some || !availability.contains(StateStructure::ReconciliationLedger)
    {
        return Ok(LedgerReset::default());
    }
    let pre_write_sqls = smelt_state::ledger::ledger_recompute_reset_sqls(
        dialect,
        schema,
        model,
        "{*}",
        region_start,
        region_end,
        "self",
        region_end,
    )?;
    Ok(LedgerReset {
        ensure_sqls: vec![smelt_state::ledger::ledger_table_ddl(dialect, schema)?],
        pre_write_sqls,
        built: true,
    })
}
