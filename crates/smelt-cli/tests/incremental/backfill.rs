//! Tests for backfill execution: batch-chunked incremental runs produce the
//! same result as a single-query full-table run.
//!
//! Batch-safety classification and batch-count arithmetic are now internal to
//! `smelt-runtime::execute_project`. These tests validate the execution
//! invariant (batched == full-table) directly against the DuckDB backend.

use super::*;
use smelt_cli::TimeRange;
use smelt_core::config::TimeseriesConfig;
use smelt_core::{BatchedConfig, Granularity};

fn batch_safe_filtered(start: &str, end: &str) -> String {
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

#[allow(dead_code)]
fn _unused_range_ref() -> TimeRange {
    TimeRange {
        start: String::new(),
        end: String::new(),
    }
}

#[allow(dead_code)]
fn _unused_config_ref() -> (BatchedConfig, TimeseriesConfig) {
    (
        BatchedConfig {
            unique_key: vec![],
            nondeterministic_columns: vec![],
            safety_overrides: Default::default(),
        },
        TimeseriesConfig {
            event_time_column: "transaction_timestamp".into(),
            partition_column: "revenue_date".into(),
            granularity: Granularity::Day,
            week_start: None,
            assert_monotonic: false,
        },
    )
}

/// Fully batch-safe model: single-query (whole range at once) matches
/// per-partition (one day at a time).
#[tokio::test]
async fn test_batch_safe_single_query_matches_per_partition() -> Result<()> {
    let (_dir, backend) = setup_backend().await?;
    seed_transactions(&backend).await?;

    run_full_refresh(
        &backend,
        "single_query",
        &batch_safe_filtered("2024-12-25", "2024-12-30"),
    )
    .await?;

    let ranges: Vec<TestTimeRange> = (25..30)
        .map(|d| TestTimeRange {
            start: format!("2024-12-{:02}", d),
            end: format!("2024-12-{:02}", d + 1),
        })
        .collect();

    run_incremental_sequence(
        &backend,
        "per_partition",
        &batch_safe_filtered,
        &ranges,
        IncrementalStrategy::DeleteInsert,
        "revenue_date",
        &[],
    )
    .await?;

    assert_tables_equal(&backend, "single_query", "per_partition").await?;
    Ok(())
}

/// 2-day chunks of a batch-safe model produce the same result as a full-table
/// baseline.
#[tokio::test]
async fn test_batch_safe_multi_day_chunks_match() -> Result<()> {
    let (_dir, backend) = setup_backend().await?;
    seed_transactions(&backend).await?;

    run_full_refresh(&backend, "baseline_chunks", DAILY_REVENUE_SQL).await?;

    let ranges = vec![
        TestTimeRange {
            start: "2024-12-25".into(),
            end: "2024-12-27".into(),
        },
        TestTimeRange {
            start: "2024-12-27".into(),
            end: "2024-12-29".into(),
        },
        TestTimeRange {
            start: "2024-12-29".into(),
            end: "2024-12-30".into(),
        },
    ];

    run_incremental_sequence(
        &backend,
        "chunked",
        &batch_safe_filtered,
        &ranges,
        IncrementalStrategy::DeleteInsert,
        "revenue_date",
        &[],
    )
    .await?;

    assert_tables_equal(&backend, "baseline_chunks", "chunked").await?;
    Ok(())
}

/// 2-day batches of a fully-batch-safe model equal the full-table result.
/// Range 2024-12-25 to 2024-12-30 (5 days) → 3 batches: [25,27), [27,29), [29,30).
#[tokio::test]
async fn test_backfill_batches_produce_same_result_as_full() -> Result<()> {
    let (_dir, backend) = setup_backend().await?;
    seed_transactions(&backend).await?;

    run_full_refresh(&backend, "baseline_backfill", DAILY_REVENUE_SQL).await?;

    let test_ranges = vec![
        TestTimeRange {
            start: "2024-12-25".into(),
            end: "2024-12-27".into(),
        },
        TestTimeRange {
            start: "2024-12-27".into(),
            end: "2024-12-29".into(),
        },
        TestTimeRange {
            start: "2024-12-29".into(),
            end: "2024-12-30".into(),
        },
    ];

    run_incremental_sequence(
        &backend,
        "backfill_batched",
        &batch_safe_filtered,
        &test_ranges,
        IncrementalStrategy::DeleteInsert,
        "revenue_date",
        &[],
    )
    .await?;

    assert_tables_equal(&backend, "baseline_backfill", "backfill_batched").await?;
    Ok(())
}
