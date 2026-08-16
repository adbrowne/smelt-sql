//! Real-fixture equivalence for the B4 group-aligned `HAVING`/`DISTINCT`
//! admission (`docs/plans/20260704-model-updates-group-b.md` Phase B4,
//! `docs/specs/incremental_shapes.md` §"Safety checks").
//!
//! A `HAVING` (or `DISTINCT`) whose own scope's `GROUP BY` (or dedup key) is
//! a superset of the model's `partition_column` is safe to batch: the
//! DELETE+INSERT contract deletes and re-inserts whole partitions, so a
//! `HAVING`/`DISTINCT` that only ever sees rows already scoped to one
//! partition cannot behave differently on a partial (batched) run than on a
//! full refresh restricted to that same partition.
//!
//! This harness proves the per-partition equivalence contract for exactly
//! that case: a daily-revenue model with `GROUP BY revenue_date, user_id
//! HAVING SUM(amount) > 100` run day-by-day must match a full refresh of the
//! same model filtered to each day.

use super::*;

/// Daily revenue, grouped by `(revenue_date, user_id)` — a superset of the
/// model's `partition_column` (`revenue_date`) — with a `HAVING` clause that
/// keeps only users whose daily revenue exceeds 100. Group-aligned per
/// `incremental_shapes.md` §"Safety checks".
const DAILY_REVENUE_HAVING_SQL: &str = r#"
    SELECT
        transaction_timestamp::DATE as revenue_date,
        user_id,
        SUM(amount) as total_revenue,
        COUNT(*) as transaction_count
    FROM raw.transactions
    GROUP BY 1, 2
    HAVING SUM(amount) > 100
"#;

fn daily_revenue_having_filtered(start: &str, end: &str) -> String {
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
        HAVING SUM(amount) > 100
    "#
    )
}

/// A group-aligned `HAVING` model, run day-by-day, matches a full refresh of
/// the same model — the per-partition equivalence oracle every Group B phase
/// is pinned to (`incremental_shapes.md` §"Per-partition equivalence").
#[tokio::test]
async fn test_group_aligned_having_matches_full_refresh_per_partition() -> Result<()> {
    let (_dir, backend) = setup_backend().await?;
    seed_transactions(&backend).await?;

    // Full-refresh baseline.
    run_full_refresh(&backend, "baseline_having", DAILY_REVENUE_HAVING_SQL).await?;

    // Day-by-day incremental replay across the full seeded range
    // (2024-12-25 .. 2024-12-30).
    run_incremental_sequence(
        &backend,
        "inc_having",
        &daily_revenue_having_filtered,
        &[
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
        ],
        IncrementalStrategy::DeleteInsert,
        "revenue_date",
        &[],
    )
    .await?;

    assert_tables_equal(&backend, "baseline_having", "inc_having").await?;

    // Sanity: the HAVING clause actually filtered something out relative to
    // the ungated aggregate — otherwise this fixture doesn't exercise HAVING
    // at all.
    let ungated = backend
        .execute_sql(
            "SELECT COUNT(*) as cnt FROM (\
             SELECT transaction_timestamp::DATE as revenue_date, user_id, SUM(amount) as total \
             FROM raw.transactions GROUP BY 1, 2)",
        )
        .await?;
    let gated = backend
        .execute_sql("SELECT COUNT(*) as cnt FROM main.baseline_having")
        .await?;
    let ungated_count = ungated[0]
        .column(0)
        .as_any()
        .downcast_ref::<arrow::array::Int64Array>()
        .unwrap()
        .value(0);
    let gated_count = gated[0]
        .column(0)
        .as_any()
        .downcast_ref::<arrow::array::Int64Array>()
        .unwrap()
        .value(0);
    assert!(
        gated_count < ungated_count,
        "expected HAVING to filter out at least one group (ungated={ungated_count}, gated={gated_count})"
    );

    Ok(())
}
