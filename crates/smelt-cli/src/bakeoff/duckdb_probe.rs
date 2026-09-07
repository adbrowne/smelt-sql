//! Raw DuckDB probes a bakeoff run uses directly against the target
//! database: event-time extent discovery, row counts, cross-variant
//! equivalence (`EXCEPT ALL`), and scratch-schema source views.

use anyhow::{Context, Result};
use std::path::Path;

pub(super) fn open(database_path: &Path) -> Result<duckdb::Connection> {
    duckdb::Connection::open(database_path)
        .with_context(|| format!("opening duckdb database at {}", database_path.display()))
}

/// `[start, end]` inclusive date extent (`YYYY-MM-DD`) of
/// `event_time_column` in the driving source's own physical table
/// (`{schema}.{table}`).
pub(super) fn event_time_extent(
    conn: &duckdb::Connection,
    schema: &str,
    table: &str,
    event_time_column: &str,
) -> Result<Option<(String, String)>> {
    let query = format!(
        "SELECT CAST(MIN(CAST({event_time_column} AS DATE)) AS VARCHAR), \
         CAST(MAX(CAST({event_time_column} AS DATE)) AS VARCHAR) \
         FROM {schema}.{table}"
    );
    let row: (Option<String>, Option<String>) = conn
        .query_row(&query, [], |r| Ok((r.get(0)?, r.get(1)?)))
        .with_context(|| format!("computing event-time extent of {schema}.{table}"))?;
    Ok(match row {
        (Some(lo), Some(hi)) => Some((lo, hi)),
        _ => None,
    })
}

pub(super) fn row_count(conn: &duckdb::Connection, schema: &str, table: &str) -> Result<i64> {
    conn.query_row(&format!("SELECT count(*) FROM {schema}.{table}"), [], |r| {
        r.get(0)
    })
    .with_context(|| format!("counting rows in {schema}.{table}"))
}

/// Rows in `left` not present in `right` (multiset semantics) —
/// `EXCEPT ALL`, one direction. Zero in both directions is the
/// equivalence proof; non-zero in either fails the bakeoff loudly.
pub(super) fn except_all_count(
    conn: &duckdb::Connection,
    left_schema: &str,
    right_schema: &str,
    table: &str,
) -> Result<i64> {
    conn.query_row(
        &format!(
            "SELECT count(*) FROM (SELECT * FROM {left_schema}.{table} EXCEPT ALL \
             SELECT * FROM {right_schema}.{table})"
        ),
        [],
        |r| r.get(0),
    )
    .with_context(|| format!("EXCEPT ALL {left_schema}.{table} vs {right_schema}.{table}"))
}

pub(super) fn drop_schema(conn: &duckdb::Connection, schema: &str) -> Result<()> {
    conn.execute_batch(&format!("DROP SCHEMA IF EXISTS {schema} CASCADE"))
        .with_context(|| format!("dropping scratch schema {schema}"))
}

/// Make every declared source the model reads visible in the scratch
/// schema without copying data: a same-database view over the real
/// schema's physical source table. `execute_project` still only ever
/// *writes* the maintained model's own output into the scratch schema —
/// sources are read-only from the run's perspective, so a view is
/// equivalent to a copy for measurement purposes and orders of
/// magnitude cheaper.
pub(super) fn ensure_scratch_source_views(
    conn: &duckdb::Connection,
    scratch_schema: &str,
    real_schema: &str,
    source_tables: &[String],
) -> Result<()> {
    conn.execute_batch(&format!("CREATE SCHEMA IF NOT EXISTS {scratch_schema}"))
        .with_context(|| format!("creating scratch schema {scratch_schema}"))?;
    for table in source_tables {
        conn.execute_batch(&format!(
            "CREATE OR REPLACE VIEW {scratch_schema}.{table} AS SELECT * FROM {real_schema}.{table}"
        ))
        .with_context(|| format!("creating scratch source view {scratch_schema}.{table}"))?;
    }
    Ok(())
}
