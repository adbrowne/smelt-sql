//! The one dialect dispatch point for the warehouse-resident ledger
//! (`docs/specs/incremental_models.md` §"The frontier record (reconciliation
//! ledger)", `docs/specs/incremental_shapes.md` §"The transactional frontier
//! write (merge ledger)").
//!
//! Every caller asks this module, never `ddl_duckdb::generate_ledger_*` or
//! `ddl_bigquery::generate_ledger_*` directly, so a dialect's spelling is
//! chosen in exactly one place instead of at each of the driver's call sites.
//! The `match` is exhaustive over [`SqlDialect`]: a new dialect is a compile
//! error here, never a silent default (`CLAUDE.md` §"Fail-loud discipline").
//!
//! **Spark is refused, not deferred.** Delta gives per-table atomicity and no
//! cross-table transaction, so a ledger write and its data write cannot be
//! made atomic there and the never-fold-twice refusal has no sound
//! realisation — `docs/specs/state.md` §"Which dialects realise which
//! structure" records that as a permanent absence. Asking for Spark ledger
//! text is a caller bug (it should have consulted the availability layer and
//! degraded), so it returns an error naming the dialect rather than
//! DuckDB-flavoured SQL Spark cannot run.
//!
//! Realisability itself is **not** decided here — `realisable_state_structures`
//! in `smelt-logical` owns it, and a run-layer caller consults that (via
//! `smelt_runtime::maintenance_driver::realises_merge_ledger`) before building
//! any statement. This module only answers "how is it spelled".

use smelt_dialect::SqlDialect;

use crate::{ddl_bigquery, ddl_duckdb};

/// A dialect with no ledger spelling was asked for one.
///
/// Reaching this means a caller skipped the availability check: the
/// availability layer would have recorded a downgrade and never got here.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "no warehouse ledger SQL exists for dialect '{dialect}': the ledger substrate is not \
     realisable there, so the plan layer should have downgraded this cell before the run \
     reached a ledger statement (docs/specs/state.md §\"Which dialects realise which \
     structure\")"
)]
pub struct UnsupportedLedgerDialect {
    /// The dialect's own name, as `SqlDialect::name` spells it.
    pub dialect: &'static str,
}

impl UnsupportedLedgerDialect {
    fn new(dialect: SqlDialect) -> Self {
        Self {
            dialect: dialect.name(),
        }
    }
}

type LedgerResult<T> = Result<T, UnsupportedLedgerDialect>;

/// Idempotent DDL creating the ledger table if it does not already exist.
pub fn ledger_table_ddl(dialect: SqlDialect, schema: &str) -> LedgerResult<String> {
    match dialect {
        SqlDialect::DuckDB => Ok(ddl_duckdb::generate_ledger_table_ddl(schema)),
        SqlDialect::BigQuery => Ok(ddl_bigquery::generate_ledger_table_ddl(schema)),
        SqlDialect::SparkSQL => Err(UnsupportedLedgerDialect::new(dialect)),
    }
}

/// Record one delta identity as folded for `(model, group, input)`.
///
/// Whether a repeat is *refused* is a dialect property, not this function's:
/// DuckDB's enforced `PRIMARY KEY` violates, BigQuery's `NOT ENFORCED` one
/// does not. A caller relying on the refusal must first check that the dialect
/// realises `StateStructure::ReconciliationLedger` in `smelt-logical`'s
/// availability layer.
#[allow(clippy::too_many_arguments)]
pub fn ledger_insert_sql(
    dialect: SqlDialect,
    schema: &str,
    model: &str,
    group: &str,
    input: &str,
    delta_id: &str,
    region_start: &str,
    region_end: &str,
) -> LedgerResult<String> {
    match dialect {
        SqlDialect::DuckDB => Ok(ddl_duckdb::generate_ledger_insert_sql(
            schema,
            model,
            group,
            input,
            delta_id,
            region_start,
            region_end,
        )),
        SqlDialect::BigQuery => Ok(ddl_bigquery::generate_ledger_insert_sql(
            schema,
            model,
            group,
            input,
            delta_id,
            region_start,
            region_end,
        )),
        SqlDialect::SparkSQL => Err(UnsupportedLedgerDialect::new(dialect)),
    }
}

/// Record one merged window as bookkeeping, no-op on repeat — the
/// re-run-tolerant (`Grade::Idempotent`) merge-ledger write.
#[allow(clippy::too_many_arguments)]
pub fn ledger_upsert_sql(
    dialect: SqlDialect,
    schema: &str,
    model: &str,
    group: &str,
    input: &str,
    delta_id: &str,
    region_start: &str,
    region_end: &str,
) -> LedgerResult<String> {
    match dialect {
        SqlDialect::DuckDB => Ok(ddl_duckdb::generate_ledger_upsert_sql(
            schema,
            model,
            group,
            input,
            delta_id,
            region_start,
            region_end,
        )),
        SqlDialect::BigQuery => Ok(ddl_bigquery::generate_ledger_upsert_sql(
            schema,
            model,
            group,
            input,
            delta_id,
            region_start,
            region_end,
        )),
        SqlDialect::SparkSQL => Err(UnsupportedLedgerDialect::new(dialect)),
    }
}

/// Best-effort existence check for `(model, group, input, delta_id)`.
pub fn ledger_exists_sql(
    dialect: SqlDialect,
    schema: &str,
    model: &str,
    group: &str,
    input: &str,
    delta_id: &str,
) -> LedgerResult<String> {
    match dialect {
        SqlDialect::DuckDB => Ok(ddl_duckdb::generate_ledger_exists_sql(
            schema, model, group, input, delta_id,
        )),
        SqlDialect::BigQuery => Ok(ddl_bigquery::generate_ledger_exists_sql(
            schema, model, group, input, delta_id,
        )),
        SqlDialect::SparkSQL => Err(UnsupportedLedgerDialect::new(dialect)),
    }
}

/// The ledger's region-recompute reset: `DELETE` every intersecting entry,
/// then `INSERT` exactly the input this recompute read.
#[allow(clippy::too_many_arguments)]
pub fn ledger_recompute_reset_sqls(
    dialect: SqlDialect,
    schema: &str,
    model: &str,
    group: &str,
    region_start: &str,
    region_end: &str,
    input: &str,
    delta_id: &str,
) -> LedgerResult<Vec<String>> {
    match dialect {
        SqlDialect::DuckDB => Ok(ddl_duckdb::generate_ledger_recompute_reset_sqls(
            schema,
            model,
            group,
            region_start,
            region_end,
            input,
            delta_id,
        )),
        SqlDialect::BigQuery => Ok(ddl_bigquery::generate_ledger_recompute_reset_sqls(
            schema,
            model,
            group,
            region_start,
            region_end,
            input,
            delta_id,
        )),
        SqlDialect::SparkSQL => Err(UnsupportedLedgerDialect::new(dialect)),
    }
}
