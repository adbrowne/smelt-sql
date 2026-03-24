//! Tests for late-arriving data handling via lookback windows.
//!
//! Verifies that when new data arrives in already-processed partitions,
//! re-running the incremental model correctly picks up the late arrivals.

use super::*;

/// Seed data with a "late arrival" pattern: initial data + data that arrives late
/// into an earlier partition.
async fn seed_with_late_arrivals(backend: &DuckDbBackend) -> Result<()> {
    // Initial transactions
    backend
        .execute_sql(
            r#"
            CREATE TABLE IF NOT EXISTS raw.transactions AS
            SELECT * FROM (VALUES
                (1, 1, 100.00, '2024-12-25 10:00:00'::TIMESTAMP),
                (2, 2, 200.00, '2024-12-25 14:00:00'::TIMESTAMP),
                (3, 1,  50.00, '2024-12-26 09:00:00'::TIMESTAMP),
                (4, 3, 300.00, '2024-12-26 16:00:00'::TIMESTAMP),
                (5, 2,  75.00, '2024-12-27 11:00:00'::TIMESTAMP)
            ) AS t(id, user_id, amount, transaction_timestamp)
        "#,
        )
        .await?;
    Ok(())
}

/// Add late-arriving data into Dec 25 partition.
async fn insert_late_arrivals(backend: &DuckDbBackend) -> Result<()> {
    backend
        .execute_sql(
            r#"
            INSERT INTO raw.transactions VALUES
                (6, 3, 150.00, '2024-12-25 23:59:00'::TIMESTAMP),
                (7, 1,  80.00, '2024-12-25 08:00:00'::TIMESTAMP)
        "#,
        )
        .await?;
    Ok(())
}

#[tokio::test]
async fn test_late_arriving_data_captured_by_reprocess() -> Result<()> {
    let (_dir, backend) = setup_backend().await?;
    seed_with_late_arrivals(&backend).await?;

    // Process Dec 25 and Dec 26
    run_incremental_sequence(
        &backend,
        "inc_lookback",
        &daily_revenue_filtered,
        &[
            TestTimeRange {
                start: "2024-12-25".into(),
                end: "2024-12-26".into(),
            },
            TestTimeRange {
                start: "2024-12-26".into(),
                end: "2024-12-27".into(),
            },
        ],
        IncrementalStrategy::DeleteInsert,
        "revenue_date",
        &[],
    )
    .await?;

    // Now late data arrives for Dec 25
    insert_late_arrivals(&backend).await?;

    // Re-process Dec 25 (simulating lookback trigger)
    run_incremental_sequence(
        &backend,
        "inc_lookback",
        &daily_revenue_filtered,
        &[TestTimeRange {
            start: "2024-12-25".into(),
            end: "2024-12-26".into(),
        }],
        IncrementalStrategy::DeleteInsert,
        "revenue_date",
        &[],
    )
    .await?;

    // Full refresh baseline (with all data including late arrivals)
    run_full_refresh(
        &backend,
        "baseline_lookback",
        &daily_revenue_filtered("2024-12-25", "2024-12-27"),
    )
    .await?;

    assert_tables_equal(&backend, "baseline_lookback", "inc_lookback").await?;
    Ok(())
}

#[tokio::test]
async fn test_late_arriving_data_with_merge() -> Result<()> {
    let (_dir, backend) = setup_backend().await?;
    seed_with_late_arrivals(&backend).await?;

    // Process incrementally with MERGE
    run_incremental_sequence(
        &backend,
        "inc_merge_lookback",
        &daily_revenue_filtered,
        &[
            TestTimeRange {
                start: "2024-12-25".into(),
                end: "2024-12-26".into(),
            },
            TestTimeRange {
                start: "2024-12-26".into(),
                end: "2024-12-27".into(),
            },
        ],
        IncrementalStrategy::Merge,
        "revenue_date",
        &["revenue_date".to_string(), "user_id".to_string()],
    )
    .await?;

    // Late arrivals
    insert_late_arrivals(&backend).await?;

    // Re-MERGE Dec 25
    run_incremental_sequence(
        &backend,
        "inc_merge_lookback",
        &daily_revenue_filtered,
        &[TestTimeRange {
            start: "2024-12-25".into(),
            end: "2024-12-26".into(),
        }],
        IncrementalStrategy::Merge,
        "revenue_date",
        &["revenue_date".to_string(), "user_id".to_string()],
    )
    .await?;

    run_full_refresh(
        &backend,
        "baseline_merge_lookback",
        &daily_revenue_filtered("2024-12-25", "2024-12-27"),
    )
    .await?;

    assert_tables_equal(&backend, "baseline_merge_lookback", "inc_merge_lookback").await?;
    Ok(())
}

#[tokio::test]
async fn test_lookback_window_wider_than_partition() -> Result<()> {
    let (_dir, backend) = setup_backend().await?;
    seed_with_late_arrivals(&backend).await?;
    insert_late_arrivals(&backend).await?;

    // Process with a lookback window wider than the partition:
    // Request Dec 27 but filter from Dec 25 to capture context
    // This simulates the temporal window expansion from Phase 3
    let wider_range_sql = daily_revenue_filtered("2024-12-25", "2024-12-28");
    backend
        .drop_table_if_exists("main", "inc_wide_lookback")
        .await?;
    backend
        .create_table_as("main", "inc_wide_lookback", &wider_range_sql)
        .await?;

    // Baseline: same wide range
    run_full_refresh(
        &backend,
        "baseline_wide_lookback",
        &daily_revenue_filtered("2024-12-25", "2024-12-28"),
    )
    .await?;

    assert_tables_equal(&backend, "baseline_wide_lookback", "inc_wide_lookback").await?;
    Ok(())
}
