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

// `test_late_arriving_data_with_merge` was deleted along with the
// `IncrementalStrategy::Merge` variant. MERGE is now the physical primitive of
// `materialization: cumulative_aggregate`; the late-arriving-data semantics
// for cumulative tables are covered by the cross-partition equivalence
// harness in `crates/smelt-cli/tests/cumulative_equivalence/` and by the
// Reprocessing semantics section of `docs/specs/cumulative_aggregate.md`.

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
