//! Real-fixture equivalence for the B2 bounded-`RANGE` cross-partition
//! window admission (`docs/plans/20260704-model-updates-group-b.md` Phase B2,
//! `docs/specs/incremental_shapes.md` §"Safety checks" — the window-functions admission row).
//!
//! A `LAG(...) OVER (PARTITION BY user_id ORDER BY transaction_timestamp
//! RANGE BETWEEN INTERVAL '1 day' PRECEDING AND CURRENT ROW)` window is
//! partitioned by `user_id` — not the model's own `revenue_date` partition
//! column — so it is only batch-safe because the bounded 1-day `RANGE`
//! frame licenses a widened *source* read: the planner widens the scan by
//! the frame's margin (the "two-layer move" — widened scan, exact output
//! clamp) so the `LAG` at a partition boundary sees the correct prior row
//! from the previous day even though the *output* is clamped to one day.
//!
//! This test hand-derives the same two-layer SQL the planner would inject
//! (widened scan `WHERE` + outer clamp `WHERE`) and proves it matches a
//! full-refresh build of the same model filtered to the same output window
//! — the per-partition equivalence oracle every Group B phase is pinned to.

use super::*;

/// The bounded-RANGE LAG model SQL, unfiltered — the "full refresh" form.
fn lag_bounded_range_sql_unfiltered() -> String {
    r#"
        SELECT
            transaction_timestamp::DATE as revenue_date,
            user_id,
            transaction_timestamp,
            LAG(transaction_timestamp) OVER (
                PARTITION BY user_id ORDER BY transaction_timestamp
                RANGE BETWEEN INTERVAL '1 day' PRECEDING AND CURRENT ROW
            ) AS prev_transaction_ts
        FROM raw.transactions
    "#
    .to_string()
}

/// The bounded-RANGE LAG model, incrementally compiled for a `[start, end)`
/// run window: the source scan is widened by the frame's 1-day margin
/// (`start - 1 day .. end`, mirroring `inject_source_filters`), and the
/// output is clamped to the exact run window (mirroring `inject_time_filter`)
/// — the two-layer move `derive_model_source_bounds` / `transformer.rs` wire.
fn lag_bounded_range_sql_incremental(start: &str, end: &str) -> String {
    format!(
        r#"
        WITH widened_scan AS (
            SELECT
                transaction_timestamp::DATE as revenue_date,
                user_id,
                transaction_timestamp,
                LAG(transaction_timestamp) OVER (
                    PARTITION BY user_id ORDER BY transaction_timestamp
                    RANGE BETWEEN INTERVAL '1 day' PRECEDING AND CURRENT ROW
                ) AS prev_transaction_ts
            FROM raw.transactions
            WHERE transaction_timestamp >= '{start}'::TIMESTAMP - INTERVAL '1 day'
              AND transaction_timestamp <  '{end}'::TIMESTAMP
        )
        SELECT revenue_date, user_id, transaction_timestamp, prev_transaction_ts
        FROM widened_scan
        WHERE revenue_date >= '{start}' AND revenue_date < '{end}'
        "#
    )
}

/// A narrow scan (no widening) — the shape that would result *without* the
/// B2 admission, i.e. reading only the exact run window. Included as a
/// negative control: it must **diverge** from full refresh at the partition
/// boundary, which is exactly why the widened scan is required.
fn lag_bounded_range_sql_narrow(start: &str, end: &str) -> String {
    format!(
        r#"
        SELECT
            transaction_timestamp::DATE as revenue_date,
            user_id,
            transaction_timestamp,
            LAG(transaction_timestamp) OVER (
                PARTITION BY user_id ORDER BY transaction_timestamp
                RANGE BETWEEN INTERVAL '1 day' PRECEDING AND CURRENT ROW
            ) AS prev_transaction_ts
        FROM raw.transactions
        WHERE transaction_timestamp >= '{start}' AND transaction_timestamp < '{end}'
        "#
    )
}

/// The widened-scan / exact-clamp two-layer incremental build matches full
/// refresh filtered to the same output partition — including at a partition
/// boundary where the `LAG` value legitimately reaches into the prior day
/// (user 1's `2024-12-26 09:00:00` row, whose prior transaction was the day
/// before at `2024-12-25 10:00:00`, 23 hours earlier — within the 1-day
/// frame but across the `revenue_date` partition boundary).
#[tokio::test]
async fn test_bounded_range_lag_widened_scan_matches_full_refresh_at_partition_boundary(
) -> Result<()> {
    let (_dir, backend) = setup_backend().await?;
    seed_transactions(&backend).await?;

    // Full-refresh baseline: compute over all history, then keep only the
    // 2024-12-26 partition — the always-correct oracle.
    backend
        .drop_table_if_exists("main", "baseline_full")
        .await?;
    backend
        .create_table_as("main", "baseline_full", &lag_bounded_range_sql_unfiltered())
        .await?;
    backend
        .execute_sql(
            "CREATE TABLE main.baseline_lag AS \
             SELECT revenue_date, user_id, transaction_timestamp, prev_transaction_ts \
             FROM main.baseline_full \
             WHERE revenue_date >= '2024-12-26' AND revenue_date < '2024-12-27'",
        )
        .await?;

    // Incremental two-layer build for the same [start, end) window.
    backend
        .drop_table_if_exists("main", "inc_lag_widened")
        .await?;
    backend
        .create_table_as(
            "main",
            "inc_lag_widened",
            &lag_bounded_range_sql_incremental("2024-12-26", "2024-12-27"),
        )
        .await?;

    assert_tables_equal(&backend, "baseline_lag", "inc_lag_widened").await?;

    // Sanity: user 1's 2024-12-26 row must have picked up the prior-day value.
    let rows = backend
        .execute_sql(
            "SELECT prev_transaction_ts FROM main.inc_lag_widened \
             WHERE user_id = 1 AND revenue_date = '2024-12-26'",
        )
        .await?;
    let prev_ts = arrow::util::pretty::pretty_format_batches(&rows)
        .map(|f| f.to_string())
        .unwrap_or_default();
    assert!(
        prev_ts.contains("2024-12-25T10:00:00") || prev_ts.contains("2024-12-25 10:00:00"),
        "user 1's 2024-12-26 row must see the 2024-12-25 prior transaction \
         via the widened scan; got: {prev_ts}"
    );

    // Negative control: the narrow (unwidened) scan diverges from full
    // refresh at this exact boundary — proving the widening is load-bearing.
    backend
        .drop_table_if_exists("main", "inc_lag_narrow")
        .await?;
    backend
        .create_table_as(
            "main",
            "inc_lag_narrow",
            &lag_bounded_range_sql_narrow("2024-12-26", "2024-12-27"),
        )
        .await?;
    let mismatch_sql = "SELECT COUNT(*) as cnt FROM (\
         SELECT * FROM main.baseline_lag EXCEPT SELECT * FROM main.inc_lag_narrow)";
    let result = backend.execute_sql(mismatch_sql).await?;
    let mismatch_count = result[0]
        .column(0)
        .as_any()
        .downcast_ref::<arrow::array::Int64Array>()
        .unwrap()
        .value(0);
    assert!(
        mismatch_count > 0,
        "the narrow (unwidened) scan was expected to diverge from full refresh \
         at the partition boundary — if it didn't, the fixture no longer exercises \
         the case the widened scan exists for"
    );

    Ok(())
}
