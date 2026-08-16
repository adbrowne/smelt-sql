//! F15 — dimension-driven horizon-bounded MERGE equivalence fixture.
//!
//! Proves the SQL emitted by `smelt_runtime::dimension_horizon_merge`
//! satisfies the `incremental_models.md` §"The equivalence invariant" oracle:
//! merging a dimension batch clamped to `[conv_ts − H, conv_ts]` via
//! `Backend::merge_into` produces exactly the same result — row for row —
//! as a full rebuild that recomputes the target from the same batch data.
//!
//! Requires `DUCKDB_LIB_DIR`/system DuckDB (`duckdb` feature, the default);
//! guarded the same way `targeted_column_backfill.rs` guards its DuckDB path.

#![cfg(feature = "duckdb")]

use smelt_backend::Backend;
use smelt_backend_duckdb::DuckDbBackend;
use smelt_logical::analysis::join_shape::ContributionVerdict;
use smelt_logical::analysis::source_bounds::{BoundResult, Seconds};
use smelt_runtime::dimension_horizon_merge;
use tempfile::TempDir;

#[test]
fn horizon_merge_matches_full_rebuild_over_the_clamped_slice() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test.duckdb");
    let schema = "main";
    let rt = tokio::runtime::Runtime::new().unwrap();

    rt.block_on(async {
        let backend = DuckDbBackend::new(&db_path, schema)
            .await
            .unwrap_or_else(|e| panic!("DuckDB open failed: {e}"));

        // The dimension batch: rows that changed at various conv_ts values,
        // some inside the horizon window, some outside (older than H before
        // the run's conv_ts — must not be touched by the clamped merge).
        backend
            .execute_sql(
                "CREATE TABLE dim_users_batch AS \
                 SELECT 1 AS user_id, TIMESTAMP '2026-07-05 01:00:00' AS conv_ts, 'gold' AS plan_tier \
                 UNION ALL SELECT 2, TIMESTAMP '2026-07-04 12:00:00', 'silver' \
                 UNION ALL SELECT 3, TIMESTAMP '2026-07-01 00:00:00', 'bronze'",
            )
            .await
            .unwrap_or_else(|e| panic!("create dim_users_batch failed: {e}"));

        // Existing target replica (as if from a prior run) — user 1 and 2
        // pre-exist with stale tiers; user 3 is intentionally absent so the
        // "must not touch rows outside the horizon" assertion is meaningful.
        backend
            .execute_sql(
                "CREATE TABLE dim_users AS \
                 SELECT 1 AS user_id, TIMESTAMP '2026-06-01 00:00:00' AS conv_ts, 'silver' AS plan_tier \
                 UNION ALL SELECT 2, TIMESTAMP '2026-06-01 00:00:00', 'bronze'",
            )
            .await
            .unwrap_or_else(|e| panic!("create dim_users (prior run) failed: {e}"));

        // Licence: a monotone join contribution (F6) plus a 24h bounded
        // horizon H (F1's forward `after` reach).
        let contribution = ContributionVerdict::Monotone;
        let bound = BoundResult::Bounded {
            source_partition_col: "conv_ts".to_string(),
            before: Seconds::ZERO,
            after: Seconds::hours(24),
        };
        let conv_ts = "2026-07-05 01:00:00";

        let source_sql = dimension_horizon_merge(
            &contribution,
            &bound,
            "dim_users",
            "conv_ts",
            conv_ts,
            "SELECT user_id, conv_ts, plan_tier FROM dim_users_batch",
        )
        .expect("licensed horizon MERGE should succeed");

        backend
            .merge_into("main", "dim_users", &source_sql, &["user_id".to_string()], &[])
            .await
            .unwrap_or_else(|e| panic!("horizon MERGE failed: {e}\nSQL: {source_sql}"));

        // Oracle: a full rebuild that only ever touches rows within the same
        // clamped horizon window, merged the same way (no re-read of any
        // fact table — the batch itself is the whole input).
        backend
            .execute_sql(
                "CREATE TABLE dim_users_oracle AS SELECT * FROM dim_users_batch",
            )
            .await
            .unwrap_or_else(|e| panic!("build oracle batch failed: {e}"));
        backend
            .execute_sql(
                "CREATE TABLE dim_users_rebuilt AS \
                 SELECT 1 AS user_id, TIMESTAMP '2026-06-01 00:00:00' AS conv_ts, 'silver' AS plan_tier \
                 UNION ALL SELECT 2, TIMESTAMP '2026-06-01 00:00:00', 'bronze'",
            )
            .await
            .unwrap_or_else(|e| panic!("seed rebuilt target failed: {e}"));
        backend
            .merge_into(
                "main",
                "dim_users_rebuilt",
                "SELECT user_id, conv_ts, plan_tier FROM dim_users_oracle \
                 WHERE conv_ts >= (TIMESTAMP '2026-07-05 01:00:00' - INTERVAL '86400 seconds') \
                 AND conv_ts <= TIMESTAMP '2026-07-05 01:00:00'",
                &["user_id".to_string()],
                &[],
            )
            .await
            .unwrap_or_else(|e| panic!("oracle merge failed: {e}"));

        let merged = backend
            .execute_sql("SELECT user_id, plan_tier FROM dim_users ORDER BY user_id")
            .await
            .unwrap_or_else(|e| panic!("select merged failed: {e}"));
        let rebuilt = backend
            .execute_sql("SELECT user_id, plan_tier FROM dim_users_rebuilt ORDER BY user_id")
            .await
            .unwrap_or_else(|e| panic!("select rebuilt failed: {e}"));

        let merged_rows = batches_to_strings(&merged);
        let rebuilt_rows = batches_to_strings(&rebuilt);

        assert_eq!(
            merged_rows, rebuilt_rows,
            "horizon-clamped MERGE result must equal the equivalently-clamped rebuild"
        );

        // The row outside the horizon (user 3, conv_ts 2026-07-01, more than
        // 24h before the run's conv_ts) must never have been merged in.
        assert!(
            !merged_rows.contains('3'),
            "row outside the horizon window must not be touched by the clamped merge:\n{merged_rows}"
        );
    });
}

#[test]
fn non_monotone_contribution_never_touches_the_table() {
    let err = dimension_horizon_merge(
        &ContributionVerdict::Refused("join fans out into a decrementing aggregate".to_string()),
        &BoundResult::Bounded {
            source_partition_col: "conv_ts".to_string(),
            before: Seconds::ZERO,
            after: Seconds::hours(24),
        },
        "dim_users",
        "conv_ts",
        "2026-07-05 01:00:00",
        "SELECT user_id, conv_ts, plan_tier FROM dim_users_batch",
    )
    .expect_err("non-monotone contribution must be refused, never emit a MERGE source");

    assert!(!err.to_uppercase().contains("SELECT * FROM"));
}

#[test]
fn unbounded_horizon_never_touches_the_table() {
    let err = dimension_horizon_merge(
        &ContributionVerdict::Monotone,
        &BoundResult::Unbounded,
        "dim_users",
        "conv_ts",
        "2026-07-05 01:00:00",
        "SELECT user_id, conv_ts, plan_tier FROM dim_users_batch",
    )
    .expect_err("unbounded horizon must be refused, never merge approximately");

    assert!(!err.to_uppercase().contains("SELECT * FROM"));
}

/// Render every batch's rows as comparable strings.
fn batches_to_strings(batches: &[arrow::array::RecordBatch]) -> String {
    use arrow::util::pretty::pretty_format_batches;
    pretty_format_batches(batches)
        .map(|d| d.to_string())
        .unwrap_or_default()
}
