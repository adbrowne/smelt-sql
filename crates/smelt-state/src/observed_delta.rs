//! The one dialect dispatch point for the warehouse-resident observed-output
//! delta record (`docs/specs/incremental_models.md` §"The graph layer" —
//! "Observed deltas on model edges").
//!
//! Same shape, and the same reasons, as [`crate::ledger`]: every caller asks
//! this module rather than `ddl_duckdb::generate_observed_delta_*` or
//! `ddl_bigquery::generate_observed_delta_*` directly, so a dialect's spelling
//! is chosen in exactly one place instead of at each of the run layer's four
//! call sites. The `match` is exhaustive over [`SqlDialect`]: a new dialect is
//! a compile error here, never a silent default (`CLAUDE.md` §"Fail-loud
//! discipline").
//!
//! **Spark is refused, not deferred** — Delta gives per-table atomicity and no
//! cross-table transaction, so the record and the write it describes cannot be
//! made atomic there, and a delta visible without its write breaks propagation
//! soundness. `docs/specs/state.md` §"Which dialects realise which structure"
//! records that as a permanent absence.
//!
//! Realisability itself is **not** decided here —
//! `realisable_state_structures` in `smelt-logical` owns it, and a run-layer
//! caller consults that (via
//! `smelt_runtime::maintenance_driver::records_observed_deltas`) before
//! building any statement. This module only answers "how is it spelled".

use smelt_dialect::SqlDialect;

use crate::{ddl_bigquery, ddl_duckdb};

/// A dialect with no observed-delta spelling was asked for one.
///
/// Reaching this means a caller skipped the availability check. That check is
/// a *skip*, not a refusal (an absent delta is a legal widen-never-narrow
/// fallback trigger), so this error is a caller bug rather than a user-facing
/// degradation.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "no observed-output-delta SQL exists for dialect '{dialect}': the structure is not \
     realisable there, so the call site should have skipped the record and proceeded with \
     the write (docs/specs/state.md §\"Which dialects realise which structure\")"
)]
pub struct UnsupportedObservedDeltaDialect {
    /// The dialect's own name, as `SqlDialect::name` spells it.
    pub dialect: &'static str,
}

impl UnsupportedObservedDeltaDialect {
    fn new(dialect: SqlDialect) -> Self {
        Self {
            dialect: dialect.name(),
        }
    }
}

type ObservedDeltaResult<T> = Result<T, UnsupportedObservedDeltaDialect>;

/// Idempotent DDL creating the observed-delta table if it does not already
/// exist.
pub fn observed_delta_table_ddl(dialect: SqlDialect, schema: &str) -> ObservedDeltaResult<String> {
    match dialect {
        SqlDialect::DuckDB => Ok(ddl_duckdb::generate_observed_delta_table_ddl(schema)),
        SqlDialect::BigQuery => Ok(ddl_bigquery::generate_observed_delta_table_ddl(schema)),
        SqlDialect::SparkSQL => Err(UnsupportedObservedDeltaDialect::new(dialect)),
    }
}

/// Record one `(model, run window)`'s observed delta, replacing any row
/// already recorded for that window.
///
/// Idempotent-replace in both realisations, spelled differently: DuckDB's
/// `INSERT … ON CONFLICT … DO UPDATE`, BigQuery's `MERGE … WHEN MATCHED THEN
/// UPDATE … WHEN NOT MATCHED THEN INSERT`. Both always write exactly one row
/// for the window, which is what carries "empty and absent are distinct".
pub fn observed_delta_upsert_sql(
    dialect: SqlDialect,
    schema: &str,
    model: &str,
    window_start: &str,
    window_end: &str,
    changed_keys_query: &str,
) -> ObservedDeltaResult<String> {
    match dialect {
        SqlDialect::DuckDB => Ok(ddl_duckdb::generate_observed_delta_upsert_sql(
            schema,
            model,
            window_start,
            window_end,
            changed_keys_query,
        )),
        SqlDialect::BigQuery => Ok(ddl_bigquery::generate_observed_delta_upsert_sql(
            schema,
            model,
            window_start,
            window_end,
            changed_keys_query,
        )),
        SqlDialect::SparkSQL => Err(UnsupportedObservedDeltaDialect::new(dialect)),
    }
}

/// Read one `(model, window)`'s recorded row back. The caller's **row count**
/// is what distinguishes "never recorded" from "recorded and empty" in every
/// dialect — no dialect encodes absence in a column value.
pub fn observed_delta_select_sql(
    dialect: SqlDialect,
    schema: &str,
    model: &str,
    window_start: &str,
    window_end: &str,
) -> ObservedDeltaResult<String> {
    match dialect {
        SqlDialect::DuckDB => Ok(ddl_duckdb::generate_observed_delta_select_sql(
            schema,
            model,
            window_start,
            window_end,
        )),
        SqlDialect::BigQuery => Ok(ddl_bigquery::generate_observed_delta_select_sql(
            schema,
            model,
            window_start,
            window_end,
        )),
        SqlDialect::SparkSQL => Err(UnsupportedObservedDeltaDialect::new(dialect)),
    }
}
