//! Tests for the DELETE+INSERT incremental strategy, plus direct coverage of
//! the `Backend::insert_into_from_query`/`insert_overwrite` trait methods.
//!
//! `IncrementalStrategy` has one dispatchable variant (`DeleteInsert`) —
//! `Append`/`InsertOverwrite` are gone (`docs/specs/incremental_models.md`
//! §"Strategy enum (backend-internal)"). `insert_into_from_query` and
//! `insert_overwrite` remain on the `Backend` trait as the capability that
//! would admit those strategies once plan derivation selects them; these
//! tests call the methods directly (not through `IncrementalStrategy`
//! dispatch) so DuckDB coverage doesn't lapse.
//!
//! Each `DeleteInsert` test runs the model incrementally over multiple time
//! ranges and compares the result to a full refresh baseline.

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

// MERGE is no longer an incremental strategy — UPSERT is the physical
// primitive of the `cumulative_aggregate` materialization (see
// `docs/specs/cumulative_aggregate.md`). The `Backend::merge_into` trait
// method is still exercised directly in
// `crates/smelt-backend-duckdb/src/lib.rs::test_merge_into_upsert` and the
// cross-partition equivalence harness in
// `crates/smelt-cli/tests/cumulative_equivalence/`.

// ---------------------------------------------------------------------------
// Backend::insert_into_from_query (direct call — capability coverage, not
// reachable via IncrementalStrategy dispatch)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_insert_into_from_query_accumulates_rows() -> Result<()> {
    let (_dir, backend) = setup_backend().await?;
    seed_transactions(&backend).await?;

    // insert_into_from_query doesn't delete — it just adds rows, so with
    // non-overlapping ranges it should match a full refresh over the union
    // of those ranges.
    backend
        .create_table_as(
            "main",
            "inc_append",
            &daily_revenue_filtered("2024-12-25", "2024-12-27"),
        )
        .await?;
    backend
        .insert_into_from_query(
            "main",
            "inc_append",
            &daily_revenue_filtered("2024-12-27", "2024-12-30"),
        )
        .await?;

    // Baseline is the same full range
    run_full_refresh(&backend, "baseline_append", DAILY_REVENUE_SQL).await?;

    assert_tables_equal(&backend, "baseline_append", "inc_append").await?;
    Ok(())
}

#[tokio::test]
async fn test_insert_into_from_query_overlapping_creates_duplicates() -> Result<()> {
    let (_dir, backend) = setup_backend().await?;
    seed_transactions(&backend).await?;

    // Calling insert_into_from_query twice over the same range WILL create
    // duplicates — that's expected behavior for a pure INSERT.
    backend
        .create_table_as(
            "main",
            "inc_append_dup",
            &daily_revenue_filtered("2024-12-25", "2024-12-27"),
        )
        .await?;
    backend
        .insert_into_from_query(
            "main",
            "inc_append_dup",
            &daily_revenue_filtered("2024-12-25", "2024-12-27"),
        )
        .await?;

    // Single full refresh for Dec 25-27 gives N rows; two inserts give 2N
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
        "insert_into_from_query called twice over the same range should double the rows"
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Backend::insert_overwrite (direct call — capability coverage, not
// reachable via IncrementalStrategy dispatch)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_insert_overwrite_matches_full_refresh() -> Result<()> {
    let (_dir, backend) = setup_backend().await?;
    seed_transactions(&backend).await?;

    run_full_refresh(&backend, "baseline_io", DAILY_REVENUE_SQL).await?;

    backend
        .create_table_as(
            "main",
            "inc_insert_overwrite",
            &daily_revenue_filtered("2024-12-25", "2024-12-27"),
        )
        .await?;
    backend
        .insert_overwrite(
            "main",
            "inc_insert_overwrite",
            &daily_revenue_filtered("2024-12-27", "2024-12-30"),
            &PartitionRange {
                column: "revenue_date".to_string(),
                start: "2024-12-27".to_string(),
                end: "2024-12-30".to_string(),
                axis: smelt_backend::PartitionAxis::Calendar,
            },
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

    // Re-process same partition — should overwrite cleanly.
    backend
        .create_table_as(
            "main",
            "inc_io_overlap",
            &daily_revenue_filtered("2024-12-25", "2024-12-30"),
        )
        .await?;
    for (start, end) in [("2024-12-25", "2024-12-28"), ("2024-12-28", "2024-12-30")] {
        backend
            .insert_overwrite(
                "main",
                "inc_io_overlap",
                &daily_revenue_filtered(start, end),
                &PartitionRange {
                    column: "revenue_date".to_string(),
                    start: start.to_string(),
                    end: end.to_string(),
                    axis: smelt_backend::PartitionAxis::Calendar,
                },
            )
            .await?;
    }

    assert_tables_equal(&backend, "baseline_io_overlap", "inc_io_overlap").await?;
    Ok(())
}
