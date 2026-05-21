#![cfg(feature = "duckdb")]
//! Incremental model correctness test infrastructure.
//!
//! Provides a harness that runs models both as full refresh and incrementally,
//! then compares results via SQL EXCEPT to detect discrepancies.

mod backfill;
mod intervals;
mod lookback;
mod schema_evolution;
mod strategies;

use anyhow::Result;
use smelt_backend::{Backend, IncrementalStrategy, MaterializationStrategy, PartitionRange};
use smelt_backend_duckdb::DuckDbBackend;
use tempfile::TempDir;

/// Time range for incremental execution.
#[derive(Debug, Clone)]
pub struct TestTimeRange {
    pub start: String,
    pub end: String,
}

/// Create a fresh DuckDB backend in a temp directory.
pub async fn setup_backend() -> Result<(TempDir, DuckDbBackend)> {
    let temp_dir = TempDir::new()?;
    let db_path = temp_dir.path().join("test.duckdb");
    let backend = DuckDbBackend::new(&db_path, "main").await?;
    backend
        .execute_sql("CREATE SCHEMA IF NOT EXISTS raw")
        .await?;
    Ok((temp_dir, backend))
}

/// Seed a source table with transaction data spanning multiple days.
pub async fn seed_transactions(backend: &DuckDbBackend) -> Result<()> {
    backend
        .execute_sql(
            r#"
            CREATE TABLE IF NOT EXISTS raw.transactions AS
            SELECT * FROM (VALUES
                (1,  1, 100.00, '2024-12-25 10:00:00'::TIMESTAMP),
                (2,  2, 200.00, '2024-12-25 14:00:00'::TIMESTAMP),
                (3,  1,  50.00, '2024-12-26 09:00:00'::TIMESTAMP),
                (4,  3, 300.00, '2024-12-26 16:00:00'::TIMESTAMP),
                (5,  2,  75.00, '2024-12-27 11:00:00'::TIMESTAMP),
                (6,  1, 125.00, '2024-12-27 15:00:00'::TIMESTAMP),
                (7,  3,  80.00, '2024-12-28 08:00:00'::TIMESTAMP),
                (8,  2, 150.00, '2024-12-28 12:00:00'::TIMESTAMP),
                (9,  1, 200.00, '2024-12-29 10:00:00'::TIMESTAMP),
                (10, 3,  60.00, '2024-12-29 14:00:00'::TIMESTAMP)
            ) AS t(id, user_id, amount, transaction_timestamp)
        "#,
        )
        .await?;
    Ok(())
}

/// The daily revenue model SQL used across tests.
pub const DAILY_REVENUE_SQL: &str = r#"
    SELECT
        transaction_timestamp::DATE as revenue_date,
        user_id,
        SUM(amount) as total_revenue,
        COUNT(*) as transaction_count
    FROM raw.transactions
    GROUP BY 1, 2
"#;

/// The daily revenue model SQL with a time filter applied for a given range.
pub fn daily_revenue_filtered(start: &str, end: &str) -> String {
    format!(
        r#"
        SELECT
            transaction_timestamp::DATE as revenue_date,
            user_id,
            SUM(amount) as total_revenue,
            COUNT(*) as transaction_count
        FROM raw.transactions
        WHERE transaction_timestamp >= '{start}' AND transaction_timestamp < '{end}'
        GROUP BY 1, 2
    "#
    )
}

/// Run a model as full refresh and return the result table name.
pub async fn run_full_refresh(backend: &DuckDbBackend, table_name: &str, sql: &str) -> Result<()> {
    backend.drop_table_if_exists("main", table_name).await?;
    backend.create_table_as("main", table_name, sql).await?;
    Ok(())
}

/// Run a model incrementally over a sequence of time ranges.
///
/// Creates the table on first run, then applies the incremental strategy
/// for each subsequent range.
pub async fn run_incremental_sequence(
    backend: &DuckDbBackend,
    table_name: &str,
    model_sql_fn: &dyn Fn(&str, &str) -> String,
    ranges: &[TestTimeRange],
    strategy: IncrementalStrategy,
    partition_column: &str,
    unique_key: &[String],
) -> Result<()> {
    for range in ranges {
        let sql = model_sql_fn(&range.start, &range.end);
        let table_exists = backend.table_exists("main", table_name).await?;

        if !table_exists {
            // First run: CREATE TABLE AS
            backend.drop_table_if_exists("main", table_name).await?;
            backend.create_table_as("main", table_name, &sql).await?;
        } else {
            // Subsequent runs: use strategy
            let partition = PartitionRange {
                column: partition_column.to_string(),
                start: range.start.clone(),
                end: range.end.clone(),
            };

            let mat_strategy = MaterializationStrategy::Incremental {
                partition: partition.clone(),
                strategy: strategy.clone(),
                unique_key: unique_key.to_vec(),
            };

            backend
                .execute_model_incremental(
                    "main",
                    table_name,
                    &sql,
                    smelt_backend::Materialization::Table,
                    mat_strategy,
                    false,
                )
                .await?;
        }
    }
    Ok(())
}

/// Compare two tables using EXCEPT (both directions) and assert no differences.
///
/// This is the core correctness assertion: the incremental result must match
/// full refresh exactly.
pub async fn assert_tables_equal(
    backend: &DuckDbBackend,
    baseline_table: &str,
    incremental_table: &str,
) -> Result<()> {
    // Rows in baseline but not in incremental
    let missing_sql = format!(
        "SELECT COUNT(*) as cnt FROM (SELECT * FROM main.{baseline} EXCEPT SELECT * FROM main.{incremental})",
        baseline = baseline_table,
        incremental = incremental_table,
    );
    let result = backend.execute_sql(&missing_sql).await?;
    let missing_count = extract_count(&result[0]);
    assert_eq!(
        missing_count, 0,
        "Found {} rows in {} but not in {} (missing from incremental)",
        missing_count, baseline_table, incremental_table
    );

    // Rows in incremental but not in baseline
    let extra_sql = format!(
        "SELECT COUNT(*) as cnt FROM (SELECT * FROM main.{incremental} EXCEPT SELECT * FROM main.{baseline})",
        baseline = baseline_table,
        incremental = incremental_table,
    );
    let result = backend.execute_sql(&extra_sql).await?;
    let extra_count = extract_count(&result[0]);
    assert_eq!(
        extra_count, 0,
        "Found {} extra rows in {} not in {} (extra in incremental)",
        extra_count, incremental_table, baseline_table
    );

    Ok(())
}

/// Assert that a table has exactly the expected row count.
pub async fn assert_row_count(
    backend: &DuckDbBackend,
    table_name: &str,
    expected: usize,
) -> Result<()> {
    let count = backend.get_row_count("main", table_name).await?;
    assert_eq!(
        count, expected,
        "Expected {} rows in {}, got {}",
        expected, table_name, count
    );
    Ok(())
}

/// Generate date strings for partition values between start (inclusive) and end (exclusive).
#[allow(dead_code)]
fn generate_partition_dates(start: &str, end: &str) -> Vec<String> {
    use chrono::{Duration, NaiveDate};

    let start_date = NaiveDate::parse_from_str(start, "%Y-%m-%d").unwrap();
    let end_date = NaiveDate::parse_from_str(end, "%Y-%m-%d").unwrap();

    let mut dates = Vec::new();
    let mut current = start_date;
    while current < end_date {
        dates.push(current.format("%Y-%m-%d").to_string());
        current += Duration::days(1);
    }
    dates
}

/// Extract a count (i64) from an Arrow RecordBatch.
fn extract_count(batch: &arrow::array::RecordBatch) -> i64 {
    batch
        .column(0)
        .as_any()
        .downcast_ref::<arrow::array::Int64Array>()
        .unwrap()
        .value(0)
}
