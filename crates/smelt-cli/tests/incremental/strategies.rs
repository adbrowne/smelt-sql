//! Tests for each incremental strategy: DELETE+INSERT, MERGE, APPEND, INSERT_OVERWRITE.
//!
//! Each test runs the model incrementally over multiple time ranges and compares
//! the result to a full refresh baseline.

use super::*;

// ---------------------------------------------------------------------------
// DELETE+INSERT
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_delete_insert_matches_full_refresh() -> Result<()> {
    let (_dir, backend) = setup_backend().await?;
    seed_transactions(&backend).await?;

    // Full refresh baseline
    run_full_refresh(&backend, "baseline", DAILY_REVENUE_SQL).await?;

    // Incremental: process day-by-day
    let ranges = vec![
        TestTimeRange {
            start: "2024-12-25".into(),
            end: "2024-12-26".into(),
        },
        TestTimeRange {
            start: "2024-12-26".into(),
            end: "2024-12-27".into(),
        },
        TestTimeRange {
            start: "2024-12-27".into(),
            end: "2024-12-28".into(),
        },
        TestTimeRange {
            start: "2024-12-28".into(),
            end: "2024-12-29".into(),
        },
        TestTimeRange {
            start: "2024-12-29".into(),
            end: "2024-12-30".into(),
        },
    ];

    run_incremental_sequence(
        &backend,
        "inc_delete_insert",
        &daily_revenue_filtered,
        &ranges,
        IncrementalStrategy::DeleteInsert,
        "revenue_date",
        &[],
    )
    .await?;

    assert_tables_equal(&backend, "baseline", "inc_delete_insert").await?;
    Ok(())
}

#[tokio::test]
async fn test_delete_insert_first_run_creates_table() -> Result<()> {
    let (_dir, backend) = setup_backend().await?;
    seed_transactions(&backend).await?;

    // Table doesn't exist → first run should CREATE
    let exists_before = backend.table_exists("main", "first_run_test").await?;
    assert!(!exists_before);

    run_incremental_sequence(
        &backend,
        "first_run_test",
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

    let exists_after = backend.table_exists("main", "first_run_test").await?;
    assert!(exists_after);
    Ok(())
}

#[tokio::test]
async fn test_delete_insert_overlapping_ranges_idempotent() -> Result<()> {
    let (_dir, backend) = setup_backend().await?;
    seed_transactions(&backend).await?;

    // Full refresh baseline
    run_full_refresh(&backend, "baseline_overlap", DAILY_REVENUE_SQL).await?;

    // Run with overlapping ranges — re-process Dec 26 twice
    let ranges = vec![
        TestTimeRange {
            start: "2024-12-25".into(),
            end: "2024-12-27".into(),
        },
        TestTimeRange {
            start: "2024-12-26".into(),
            end: "2024-12-28".into(),
        },
        TestTimeRange {
            start: "2024-12-28".into(),
            end: "2024-12-30".into(),
        },
    ];

    run_incremental_sequence(
        &backend,
        "inc_overlap",
        &daily_revenue_filtered,
        &ranges,
        IncrementalStrategy::DeleteInsert,
        "revenue_date",
        &[],
    )
    .await?;

    assert_tables_equal(&backend, "baseline_overlap", "inc_overlap").await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// MERGE
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_merge_matches_full_refresh() -> Result<()> {
    let (_dir, backend) = setup_backend().await?;
    seed_transactions(&backend).await?;

    run_full_refresh(&backend, "baseline_merge", DAILY_REVENUE_SQL).await?;

    let ranges = vec![
        TestTimeRange {
            start: "2024-12-25".into(),
            end: "2024-12-27".into(),
        },
        TestTimeRange {
            start: "2024-12-27".into(),
            end: "2024-12-30".into(),
        },
    ];

    run_incremental_sequence(
        &backend,
        "inc_merge",
        &daily_revenue_filtered,
        &ranges,
        IncrementalStrategy::Merge,
        "revenue_date",
        &["revenue_date".to_string(), "user_id".to_string()],
    )
    .await?;

    assert_tables_equal(&backend, "baseline_merge", "inc_merge").await?;
    Ok(())
}

#[tokio::test]
async fn test_merge_overlapping_ranges_idempotent() -> Result<()> {
    let (_dir, backend) = setup_backend().await?;
    seed_transactions(&backend).await?;

    run_full_refresh(&backend, "baseline_merge_overlap", DAILY_REVENUE_SQL).await?;

    // Re-process same range twice — MERGE should be idempotent
    let ranges = vec![
        TestTimeRange {
            start: "2024-12-25".into(),
            end: "2024-12-30".into(),
        },
        TestTimeRange {
            start: "2024-12-25".into(),
            end: "2024-12-30".into(),
        },
    ];

    run_incremental_sequence(
        &backend,
        "inc_merge_overlap",
        &daily_revenue_filtered,
        &ranges,
        IncrementalStrategy::Merge,
        "revenue_date",
        &["revenue_date".to_string(), "user_id".to_string()],
    )
    .await?;

    assert_tables_equal(&backend, "baseline_merge_overlap", "inc_merge_overlap").await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// APPEND
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_append_accumulates_rows() -> Result<()> {
    let (_dir, backend) = setup_backend().await?;
    seed_transactions(&backend).await?;

    // APPEND doesn't delete — it just adds rows, so with non-overlapping ranges
    // it should match a full refresh over the union of those ranges
    let ranges = vec![
        TestTimeRange {
            start: "2024-12-25".into(),
            end: "2024-12-27".into(),
        },
        TestTimeRange {
            start: "2024-12-27".into(),
            end: "2024-12-30".into(),
        },
    ];

    run_incremental_sequence(
        &backend,
        "inc_append",
        &daily_revenue_filtered,
        &ranges,
        IncrementalStrategy::Append,
        "revenue_date",
        &[],
    )
    .await?;

    // Baseline is the same full range
    run_full_refresh(&backend, "baseline_append", DAILY_REVENUE_SQL).await?;

    assert_tables_equal(&backend, "baseline_append", "inc_append").await?;
    Ok(())
}

#[tokio::test]
async fn test_append_overlapping_creates_duplicates() -> Result<()> {
    let (_dir, backend) = setup_backend().await?;
    seed_transactions(&backend).await?;

    // APPEND with overlapping ranges WILL create duplicates — that's expected behavior
    let ranges = vec![
        TestTimeRange {
            start: "2024-12-25".into(),
            end: "2024-12-27".into(),
        },
        TestTimeRange {
            start: "2024-12-25".into(),
            end: "2024-12-27".into(),
        },
    ];

    run_incremental_sequence(
        &backend,
        "inc_append_dup",
        &daily_revenue_filtered,
        &ranges,
        IncrementalStrategy::Append,
        "revenue_date",
        &[],
    )
    .await?;

    // Single full refresh for Dec 25-27 gives N rows; APPEND twice gives 2N
    run_full_refresh(
        &backend,
        "baseline_append_dup",
        &daily_revenue_filtered("2024-12-25", "2024-12-27"),
    )
    .await?;

    let baseline_count = backend.get_row_count("main", "baseline_append_dup").await?;
    let append_count = backend.get_row_count("main", "inc_append_dup").await?;
    assert_eq!(
        append_count,
        baseline_count * 2,
        "APPEND with overlapping ranges should double the rows"
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// INSERT OVERWRITE
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_insert_overwrite_matches_full_refresh() -> Result<()> {
    let (_dir, backend) = setup_backend().await?;
    seed_transactions(&backend).await?;

    run_full_refresh(&backend, "baseline_io", DAILY_REVENUE_SQL).await?;

    let ranges = vec![
        TestTimeRange {
            start: "2024-12-25".into(),
            end: "2024-12-27".into(),
        },
        TestTimeRange {
            start: "2024-12-27".into(),
            end: "2024-12-30".into(),
        },
    ];

    run_incremental_sequence(
        &backend,
        "inc_insert_overwrite",
        &daily_revenue_filtered,
        &ranges,
        IncrementalStrategy::InsertOverwrite,
        "revenue_date",
        &[],
    )
    .await?;

    assert_tables_equal(&backend, "baseline_io", "inc_insert_overwrite").await?;
    Ok(())
}

#[tokio::test]
async fn test_insert_overwrite_overlapping_is_idempotent() -> Result<()> {
    let (_dir, backend) = setup_backend().await?;
    seed_transactions(&backend).await?;

    run_full_refresh(&backend, "baseline_io_overlap", DAILY_REVENUE_SQL).await?;

    // Re-process same partition — should overwrite cleanly
    let ranges = vec![
        TestTimeRange {
            start: "2024-12-25".into(),
            end: "2024-12-30".into(),
        },
        TestTimeRange {
            start: "2024-12-25".into(),
            end: "2024-12-28".into(),
        },
        TestTimeRange {
            start: "2024-12-28".into(),
            end: "2024-12-30".into(),
        },
    ];

    run_incremental_sequence(
        &backend,
        "inc_io_overlap",
        &daily_revenue_filtered,
        &ranges,
        IncrementalStrategy::InsertOverwrite,
        "revenue_date",
        &[],
    )
    .await?;

    assert_tables_equal(&backend, "baseline_io_overlap", "inc_io_overlap").await?;
    Ok(())
}
