//! Tests for backfill intelligence: batch subdivision and single-query backfill.
//!
//! Verifies that:
//! - Batch-safe models produce the same result as single-query
//! - Non-batch-safe models with per-partition execution are correct
//! - Backfill batch generation respects safety analysis

use super::*;
use smelt_cli::{compute_batches_for_model, BackfillOptions, TimeRange};
use smelt_core::{Granularity, IncrementalConfig};
use smelt_planner::BatchSafety;

/// Simple GROUP BY aggregation — fully batch safe.
const BATCH_SAFE_SQL: &str = r#"
    SELECT
        transaction_timestamp::DATE as revenue_date,
        user_id,
        SUM(amount) as total_revenue,
        COUNT(*) as transaction_count
    FROM raw.transactions
    GROUP BY 1, 2
"#;

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

fn inc_config() -> IncrementalConfig {
    IncrementalConfig {
        enabled: true,
        event_time_column: "transaction_timestamp".into(),
        partition_column: "revenue_date".into(),
        granularity: Granularity::Day,
        unique_key: vec![],
        safety_overrides: Default::default(),
    }
}

#[tokio::test]
async fn test_batch_safe_single_query_matches_per_partition() -> Result<()> {
    let (_dir, backend) = setup_backend().await?;
    seed_transactions(&backend).await?;

    // Single-query: process entire range at once (batch-safe)
    run_full_refresh(
        &backend,
        "single_query",
        &batch_safe_filtered("2024-12-25", "2024-12-30"),
    )
    .await?;

    // Per-partition: process one day at a time
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

#[tokio::test]
async fn test_batch_safe_multi_day_chunks_match() -> Result<()> {
    let (_dir, backend) = setup_backend().await?;
    seed_transactions(&backend).await?;

    // Full refresh baseline
    run_full_refresh(&backend, "baseline_chunks", DAILY_REVENUE_SQL).await?;

    // 2-day chunks
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
        &daily_revenue_filtered,
        &ranges,
        IncrementalStrategy::DeleteInsert,
        "revenue_date",
        &[],
    )
    .await?;

    assert_tables_equal(&backend, "baseline_chunks", "chunked").await?;
    Ok(())
}

#[test]
fn test_compute_batches_fully_safe_single_batch() {
    let config = inc_config();
    let range = TimeRange {
        start: "2024-12-25".into(),
        end: "2024-12-30".into(),
    };

    let (safety, batches) =
        compute_batches_for_model(BATCH_SAFE_SQL, &config, &range, &range, &Default::default())
            .unwrap();

    assert!(matches!(safety, BatchSafety::FullyBatchSafe));
    // Fully batch safe → single batch for entire range
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].partition_range.start, "2024-12-25");
    assert_eq!(batches[0].partition_range.end, "2024-12-30");
}

#[test]
fn test_compute_batches_per_partition_override() {
    let config = inc_config();
    let range = TimeRange {
        start: "2024-12-25".into(),
        end: "2024-12-28".into(),
    };
    let options = BackfillOptions {
        per_partition: true,
        batch_size_days: None,
    };

    let (_safety, batches) =
        compute_batches_for_model(BATCH_SAFE_SQL, &config, &range, &range, &options).unwrap();

    // Per-partition forced → one batch per day
    assert_eq!(batches.len(), 3);
    assert_eq!(batches[0].partition_range.start, "2024-12-25");
    assert_eq!(batches[0].partition_range.end, "2024-12-26");
    assert_eq!(batches[2].partition_range.start, "2024-12-27");
    assert_eq!(batches[2].partition_range.end, "2024-12-28");
}

#[test]
fn test_compute_batches_custom_batch_size() {
    let config = inc_config();
    let range = TimeRange {
        start: "2024-12-01".into(),
        end: "2025-01-01".into(),
    };
    let options = BackfillOptions {
        batch_size_days: Some(7),
        per_partition: false,
    };

    let (_safety, batches) =
        compute_batches_for_model(BATCH_SAFE_SQL, &config, &range, &range, &options).unwrap();

    // 31 days / 7 = 4 full + 1 partial = 5 batches
    assert_eq!(batches.len(), 5);
    assert_eq!(batches[0].partition_range.start, "2024-12-01");
    assert_eq!(batches[0].partition_range.end, "2024-12-08");
}

#[tokio::test]
async fn test_backfill_batches_produce_same_result_as_full() -> Result<()> {
    let (_dir, backend) = setup_backend().await?;
    seed_transactions(&backend).await?;

    // Full refresh baseline
    run_full_refresh(&backend, "baseline_backfill", DAILY_REVENUE_SQL).await?;

    // Compute batches and execute them sequentially
    let config = inc_config();
    let range = TimeRange {
        start: "2024-12-25".into(),
        end: "2024-12-30".into(),
    };
    let options = BackfillOptions {
        batch_size_days: Some(2),
        per_partition: false,
    };

    let (_safety, batches) =
        compute_batches_for_model(BATCH_SAFE_SQL, &config, &range, &range, &options)?;

    let test_ranges: Vec<TestTimeRange> = batches
        .iter()
        .map(|b| TestTimeRange {
            start: b.partition_range.start.clone(),
            end: b.partition_range.end.clone(),
        })
        .collect();

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
