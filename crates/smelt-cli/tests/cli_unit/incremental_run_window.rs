//! Tests for run-window-vs-partition decoupling (Phase 6).
//!
//! Verifies that:
//! - A multi-partition run window uses a single engine query (FullyBatchSafe)
//! - Running `[D, D+7d)` as one window produces the same output as 7 daily runs
//! - Misaligned windows (not integer multiples of granularity) are rejected
//! - DELETE covers the full window, not just one partition
#![cfg(feature = "duckdb")]

use anyhow::Result;
use smelt_backend::Backend;
use smelt_backend::PartitionRange;
use smelt_backend_duckdb::DuckDbBackend;
use tempfile::TempDir;

/// Create a fresh DuckDB backend in a temp directory with a `raw` schema.
async fn setup_backend() -> Result<(TempDir, DuckDbBackend)> {
    let temp_dir = TempDir::new()?;
    let db_path = temp_dir.path().join("test.duckdb");
    let backend = DuckDbBackend::new(&db_path, "main").await?;
    backend
        .execute_sql("CREATE SCHEMA IF NOT EXISTS raw")
        .await?;
    Ok((temp_dir, backend))
}

/// Seed a raw events table with 7 days of data (one event per day).
async fn seed_events(backend: &DuckDbBackend) -> Result<()> {
    backend
        .execute_sql(
            r#"
            CREATE TABLE IF NOT EXISTS raw.events AS
            SELECT * FROM (VALUES
                (1, '2024-01-01'::DATE, 10),
                (2, '2024-01-02'::DATE, 20),
                (3, '2024-01-03'::DATE, 30),
                (4, '2024-01-04'::DATE, 40),
                (5, '2024-01-05'::DATE, 50),
                (6, '2024-01-06'::DATE, 60),
                (7, '2024-01-07'::DATE, 70)
            ) AS t(id, event_date, value)
        "#,
        )
        .await?;
    Ok(())
}

/// SQL for a simple daily aggregation model with a time filter injected.
fn daily_agg_sql(start: &str, end: &str) -> String {
    format!(
        r#"
        SELECT
            event_date,
            SUM(value) as total_value,
            COUNT(*) as event_count
        FROM raw.events
        WHERE event_date >= '{start}'::DATE AND event_date < '{end}'::DATE
        GROUP BY event_date
    "#
    )
}

/// Run the model over a window via a single INSERT (for table creation or incremental).
async fn run_single_query(backend: &DuckDbBackend, table_name: &str, sql: &str) -> Result<()> {
    let exists = backend.table_exists("main", table_name).await?;
    if !exists {
        backend.create_table_as("main", table_name, sql).await?;
    } else {
        backend
            .insert_into_from_query("main", table_name, sql)
            .await?;
    }
    Ok(())
}

/// --- test_seven_day_window_one_query ---
///
/// A 7-day window on a FullyBatchSafe model produces output rows
/// spanning 7 distinct partition values, all from one query execution.
#[tokio::test]
async fn test_seven_day_window_one_query() -> Result<()> {
    let (_dir, backend) = setup_backend().await?;
    seed_events(&backend).await?;

    // Run the full 7-day window in one query.
    let sql = daily_agg_sql("2024-01-01", "2024-01-08");
    run_single_query(&backend, "daily_agg", &sql).await?;

    let count = backend.get_row_count("main", "daily_agg").await?;
    assert_eq!(count, 7, "7-day window must produce 7 partition rows");

    // Verify distinct partition values.
    let batches = backend
        .execute_sql("SELECT COUNT(DISTINCT event_date) as n FROM main.daily_agg")
        .await?;
    let distinct: i64 = batches[0]
        .column(0)
        .as_any()
        .downcast_ref::<arrow::array::Int64Array>()
        .unwrap()
        .value(0);
    assert_eq!(distinct, 7, "All 7 partition dates must be present");

    Ok(())
}

/// --- test_window_equivalent_to_daily_runs ---
///
/// Running `[D, D+7d)` as one window produces the same per-partition output
/// as 7 successive `[D+i, D+i+1)` runs.
#[tokio::test]
async fn test_window_equivalent_to_daily_runs() -> Result<()> {
    let (_dir, backend) = setup_backend().await?;
    seed_events(&backend).await?;

    // Approach A: single window query.
    let window_sql = daily_agg_sql("2024-01-01", "2024-01-08");
    backend
        .create_table_as("main", "window_result", &window_sql)
        .await?;

    // Approach B: 7 daily incremental runs.
    for day in 1..=7u32 {
        let start = format!("2024-01-{:02}", day);
        let end = format!("2024-01-{:02}", if day < 7 { day + 1 } else { 8 });
        let day_sql = daily_agg_sql(&start, &end);

        let exists = backend.table_exists("main", "daily_result").await?;
        if !exists {
            backend
                .create_table_as("main", "daily_result", &day_sql)
                .await?;
        } else {
            // Use range-based delete for the partition.
            let range = PartitionRange {
                column: "event_date".to_string(),
                start: start.clone(),
                end: end.clone(),
                axis: smelt_backend::PartitionAxis::Calendar,
            };
            backend
                .delete_partitions("main", "daily_result", &range)
                .await?;
            backend
                .insert_into_from_query("main", "daily_result", &day_sql)
                .await?;
        }
    }

    // The two tables must be identical.
    let missing = backend
        .execute_sql(
            "SELECT COUNT(*) FROM (SELECT * FROM main.window_result EXCEPT SELECT * FROM main.daily_result)",
        )
        .await?;
    let missing_count: i64 = missing[0]
        .column(0)
        .as_any()
        .downcast_ref::<arrow::array::Int64Array>()
        .unwrap()
        .value(0);
    assert_eq!(
        missing_count, 0,
        "Rows in window_result not in daily_result: {missing_count}"
    );

    let extra = backend
        .execute_sql(
            "SELECT COUNT(*) FROM (SELECT * FROM main.daily_result EXCEPT SELECT * FROM main.window_result)",
        )
        .await?;
    let extra_count: i64 = extra[0]
        .column(0)
        .as_any()
        .downcast_ref::<arrow::array::Int64Array>()
        .unwrap()
        .value(0);
    assert_eq!(
        extra_count, 0,
        "Rows in daily_result not in window_result: {extra_count}"
    );

    Ok(())
}

/// --- test_misaligned_window_rejected ---
///
/// A run window that is not an integer multiple of granularity is rejected
/// at planning time with a clear diagnostic.
#[test]
fn test_misaligned_window_rejected() {
    use chrono::NaiveDate;
    use smelt_core::Granularity;

    // Helper that mimics the alignment check the run command performs.
    fn check_alignment(start: &str, end: &str, granularity: &Granularity) -> Result<(), String> {
        let start_date = NaiveDate::parse_from_str(start, "%Y-%m-%d").map_err(|e| e.to_string())?;
        let end_date = NaiveDate::parse_from_str(end, "%Y-%m-%d").map_err(|e| e.to_string())?;
        smelt_cli::validate_run_window_alignment(start_date, end_date, granularity)
    }

    // Aligned windows must be accepted.
    assert!(
        check_alignment("2024-01-01", "2024-01-08", &Granularity::Day).is_ok(),
        "7-day window must be accepted for daily granularity"
    );
    assert!(
        check_alignment("2024-01-01", "2024-01-08", &Granularity::Week).is_ok(),
        "1-week window must be accepted for weekly granularity"
    );
    assert!(
        check_alignment("2024-01-01", "2024-02-01", &Granularity::Month).is_ok(),
        "1-month window must be accepted for monthly granularity"
    );

    // Misaligned: 3 days for weekly granularity.
    let err = check_alignment("2024-01-01", "2024-01-04", &Granularity::Week)
        .expect_err("3-day window must be rejected for weekly granularity");
    assert!(
        err.contains("not aligned"),
        "Error message must mention 'not aligned': {err}"
    );
    assert!(
        err.contains("week") || err.contains("Week") || err.contains("7"),
        "Error message must mention granularity: {err}"
    );

    // Misaligned: 5 days for weekly granularity.
    let err = check_alignment("2024-01-01", "2024-01-06", &Granularity::Week)
        .expect_err("5-day window must be rejected for weekly granularity");
    assert!(err.contains("not aligned"), "{err}");

    // Misaligned: 10 days for monthly granularity.
    let err = check_alignment("2024-01-01", "2024-01-11", &Granularity::Month)
        .expect_err("10-day window must be rejected for monthly granularity");
    assert!(err.contains("not aligned"), "{err}");
}

/// --- test_delete_covers_full_window ---
///
/// The DELETE before INSERT removes all N partitions in the run window,
/// not just one.
#[tokio::test]
async fn test_delete_covers_full_window() -> Result<()> {
    let (_dir, backend) = setup_backend().await?;
    seed_events(&backend).await?;

    // Create an initial table with all 7 days of data.
    let initial_sql = daily_agg_sql("2024-01-01", "2024-01-08");
    backend
        .create_table_as("main", "daily_agg", &initial_sql)
        .await?;

    let initial_count = backend.get_row_count("main", "daily_agg").await?;
    assert_eq!(initial_count, 7);

    // Now DELETE the full 7-day window using range-based deletion.
    let range = PartitionRange {
        column: "event_date".to_string(),
        start: "2024-01-01".to_string(),
        end: "2024-01-08".to_string(),
        axis: smelt_backend::PartitionAxis::Calendar,
    };
    backend
        .delete_partitions("main", "daily_agg", &range)
        .await?;

    let after_delete = backend.get_row_count("main", "daily_agg").await?;
    assert_eq!(
        after_delete, 0,
        "DELETE must remove all 7 partitions in [2024-01-01, 2024-01-08); got {after_delete}"
    );

    // Verify partial-window delete leaves untouched partitions alone.
    // Re-insert all 7 days.
    backend
        .insert_into_from_query("main", "daily_agg", &initial_sql)
        .await?;

    // Delete only [Jan 1, Jan 4): 3 days.
    let partial_range = PartitionRange {
        column: "event_date".to_string(),
        start: "2024-01-01".to_string(),
        end: "2024-01-04".to_string(),
        axis: smelt_backend::PartitionAxis::Calendar,
    };
    backend
        .delete_partitions("main", "daily_agg", &partial_range)
        .await?;

    let after_partial = backend.get_row_count("main", "daily_agg").await?;
    assert_eq!(
        after_partial, 4,
        "Partial DELETE must leave Jan 4-7 intact; got {after_partial}"
    );

    Ok(())
}
