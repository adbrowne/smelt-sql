//! T5 — observed output delta recording (`docs/plans/
//! 20260715-composed-axes-conditional-maintenance.md` Phase D2;
//! `docs/specs/incremental_models.md` §"The graph layer" — "Observed
//! deltas on model edges").
//!
//! A change-suppressed column-scoped MERGE (C4,
//! `maintenance_driver::execute_column_scoped_merge_full`) already computes
//! the changed-row set it actually touches (the `IS DISTINCT FROM` guard's
//! matched rows, plus every unmatched/inserted row). This suite proves the
//! recording those writes now perform, against a real DuckDB backend:
//! - the recorded `changed_keys` holds exactly the rows that differed;
//! - a fully-suppressed run records an EMPTY delta (present, not absent);
//! - an Incomparable column's own flutter (excluded from `compared_columns`)
//!   never dirties the recorded delta, even though the row differs;
//! - the record survives a re-run of the same window as a REPLACE, never a
//!   duplicate (`PRIMARY KEY (model_name, window_start, window_end)`).

use smelt_backend::{Backend, PartitionRange};
use smelt_backend_duckdb::DuckDbBackend;
use smelt_logical::analysis::walk::{ColumnComparability, Comparability};
use smelt_logical::maintenance::choice::{resolve_write_suppression, WriteSuppression};
use smelt_logical::maintenance::{RowIdentity, RowIdentityVerdict};
use smelt_runtime::maintenance_driver::execute_column_scoped_merge_full;

/// A retry policy that never retries — these tests exercise the
/// column-scoped MERGE write directly against a real DuckDB backend,
/// outside `execute_project`, so there is no `ExecuteRequest`/run reporter
/// to derive one from (`docs/plans/20260719-prod-w2-operability.md` Phase
/// 6).
const NO_OP_REPORTER: smelt_runtime::NoOpReporter = smelt_runtime::NoOpReporter;
fn no_retry_policy() -> smelt_runtime::RetryPolicy<'static> {
    smelt_runtime::RetryPolicy {
        retry_max: 0,
        base_backoff_ms: 0,
        run_id: "observed-delta-test",
        model_name: "observed-delta-test",
        reporter: &NO_OP_REPORTER,
    }
}

fn key_suppression(compared: &[&str]) -> WriteSuppression {
    let row_identity = RowIdentityVerdict {
        identity: RowIdentity::Key(vec!["user_id".to_string()]),
        proven_mismatch: None,
    };
    let comparability: Vec<ColumnComparability> = compared
        .iter()
        .map(|c| ColumnComparability {
            output: c.to_string(),
            comparability: Comparability::Comparable,
        })
        .collect();
    let compared_columns: Vec<String> = compared.iter().map(|c| c.to_string()).collect();
    resolve_write_suppression(&compared_columns, &comparability, &row_identity)
}

async fn recorded_delta(
    backend: &DuckDbBackend,
    model: &str,
    window: &PartitionRange,
) -> Option<(Vec<String>, Vec<String>)> {
    let sql = format!(
        "SELECT changed_keys, partitions FROM main._smelt_observed_delta \
         WHERE model_name = '{model}' AND window_start = '{}' AND window_end = '{}'",
        window.start, window.end
    );
    let batches = backend.execute_sql(&sql).await.expect("query delta table");
    let rows = batches_to_string_lists(&batches);
    rows.into_iter().next()
}

/// Pull `(changed_keys, partitions)` as `Vec<String>` out of DuckDB's
/// `LIST`-typed columns via a debug-format round trip — good enough for
/// this test's assertions (exact membership, not ordering).
fn batches_to_string_lists(
    batches: &[arrow::array::RecordBatch],
) -> Vec<(Vec<String>, Vec<String>)> {
    use arrow::array::{Array, ListArray, StringArray};
    let mut out = Vec::new();
    for batch in batches {
        let changed = batch
            .column(0)
            .as_any()
            .downcast_ref::<ListArray>()
            .expect("changed_keys is a LIST column");
        let partitions = batch
            .column(1)
            .as_any()
            .downcast_ref::<ListArray>()
            .expect("partitions is a LIST column");
        for i in 0..batch.num_rows() {
            let changed_vals = changed.value(i);
            let changed_strs: Vec<String> = changed_vals
                .as_any()
                .downcast_ref::<StringArray>()
                .expect("changed_keys elements are VARCHAR")
                .iter()
                .map(|v| v.unwrap_or_default().to_string())
                .collect();
            let partition_vals = partitions.value(i);
            let partition_strs: Vec<String> = partition_vals
                .as_any()
                .downcast_ref::<StringArray>()
                .expect("partitions elements are VARCHAR")
                .iter()
                .map(|v| v.unwrap_or_default().to_string())
                .collect();
            out.push((changed_strs, partition_strs));
        }
    }
    out
}

fn window() -> PartitionRange {
    PartitionRange {
        column: String::new(),
        start: "2026-01-01".to_string(),
        end: "2026-01-02".to_string(),
    }
}

/// A window whose `column` names the model's declared partition column —
/// the shape `crates/smelt-runtime/src/execute.rs` actually constructs for
/// any timeseries-partitioned composed model
/// (`window.column: inc_plan.timeseries.partition_column.clone()`). This
/// exercises `changed_keys_select`'s `Some(partition_column)` branch (the
/// `CAST(source.<col> AS VARCHAR) AS delta_partition` projection), which the
/// other tests in this suite — all using the always-empty `window()` helper
/// above — never reach.
fn partitioned_window() -> PartitionRange {
    PartitionRange {
        column: "region".to_string(),
        start: "2026-01-01".to_string(),
        end: "2026-01-02".to_string(),
    }
}

/// 100 candidate rows, exactly 3 of which differ from the target — the
/// recorded delta must hold exactly those 3 keys, no more, no fewer.
#[tokio::test]
async fn suppressed_merge_records_exactly_the_changed_keys() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("test.duckdb");
    let backend = DuckDbBackend::new(&db_path, "main")
        .await
        .expect("open duckdb");

    backend
        .execute_sql("CREATE TABLE main.dim_users (user_id BIGINT, tier VARCHAR)")
        .await
        .unwrap();
    backend
        .execute_sql("CREATE TABLE main.sources_users (user_id BIGINT, tier VARCHAR)")
        .await
        .unwrap();

    // Seed 100 identical rows in both target and source.
    let mut target_values = Vec::new();
    let mut source_values = Vec::new();
    for i in 0..100i64 {
        target_values.push(format!("({i}, 'bronze')"));
        source_values.push(format!("({i}, 'bronze')"));
    }
    backend
        .execute_sql(&format!(
            "INSERT INTO main.dim_users VALUES {}",
            target_values.join(", ")
        ))
        .await
        .unwrap();
    // Mutate exactly 3 rows (ids 1, 42, 99) in the source.
    for id in [1i64, 42, 99] {
        source_values[id as usize] = format!("({id}, 'gold')");
    }
    backend
        .execute_sql(&format!(
            "INSERT INTO main.sources_users VALUES {}",
            source_values.join(", ")
        ))
        .await
        .unwrap();

    let suppression = key_suppression(&["tier"]);
    let dimension_batch_sql = "SELECT u.user_id, u.tier FROM main.sources_users u";
    let w = window();

    execute_column_scoped_merge_full(
        &backend,
        "main",
        "dim_users",
        &["user_id".to_string()],
        dimension_batch_sql,
        &[],
        &suppression,
        &w,
        &no_retry_policy(),
    )
    .await
    .expect("suppressed merge succeeds");

    let (changed_keys, _partitions) = recorded_delta(&backend, "dim_users", &w)
        .await
        .expect("a delta row is recorded (present, not absent)");
    let mut sorted = changed_keys.clone();
    sorted.sort();
    assert_eq!(
        sorted,
        vec!["1".to_string(), "42".to_string(), "99".to_string()],
        "recorded delta must hold exactly the 3 changed keys, got: {changed_keys:?}"
    );
}

/// A fully-suppressed run (nothing changed) must record a PRESENT-AND-EMPTY
/// delta — distinct from no row at all (absent).
#[tokio::test]
async fn fully_suppressed_run_records_present_and_empty_delta() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("test.duckdb");
    let backend = DuckDbBackend::new(&db_path, "main")
        .await
        .expect("open duckdb");

    backend
        .execute_sql("CREATE TABLE main.dim_users (user_id BIGINT, tier VARCHAR)")
        .await
        .unwrap();
    backend
        .execute_sql("INSERT INTO main.dim_users VALUES (1, 'bronze'), (2, 'silver')")
        .await
        .unwrap();
    backend
        .execute_sql("CREATE TABLE main.sources_users (user_id BIGINT, tier VARCHAR)")
        .await
        .unwrap();
    backend
        .execute_sql("INSERT INTO main.sources_users VALUES (1, 'bronze'), (2, 'silver')")
        .await
        .unwrap();

    let suppression = key_suppression(&["tier"]);
    let dimension_batch_sql = "SELECT u.user_id, u.tier FROM main.sources_users u";
    let w = window();

    execute_column_scoped_merge_full(
        &backend,
        "main",
        "dim_users",
        &["user_id".to_string()],
        dimension_batch_sql,
        &[],
        &suppression,
        &w,
        &no_retry_policy(),
    )
    .await
    .expect("suppressed merge succeeds");

    let (changed_keys, partitions) = recorded_delta(&backend, "dim_users", &w)
        .await
        .expect("a delta row is recorded even when nothing changed (present-and-empty)");
    assert!(
        changed_keys.is_empty(),
        "an unchanged-input run must record an empty changed-key set, got: {changed_keys:?}"
    );
    assert!(partitions.is_empty(), "partitions must also be empty");
}

/// An Incomparable column's own change (excluded from `compared_columns`,
/// e.g. a `plausible` audit stamp) must never appear in — or dirty — the
/// recorded delta. `compared_columns` here names only `tier`; `notes`
/// changes on every row but is not in the compared set, so the recorded
/// delta must stay empty even though `notes` visibly differs.
#[tokio::test]
async fn incomparable_column_change_alone_records_nothing() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("test.duckdb");
    let backend = DuckDbBackend::new(&db_path, "main")
        .await
        .expect("open duckdb");

    backend
        .execute_sql("CREATE TABLE main.dim_users (user_id BIGINT, tier VARCHAR, notes VARCHAR)")
        .await
        .unwrap();
    backend
        .execute_sql(
            "INSERT INTO main.dim_users VALUES (1, 'bronze', 'stamp_a'), (2, 'silver', 'stamp_a')",
        )
        .await
        .unwrap();
    backend
        .execute_sql(
            "CREATE TABLE main.sources_users (user_id BIGINT, tier VARCHAR, notes VARCHAR)",
        )
        .await
        .unwrap();
    // `tier` unchanged; `notes` (an Incomparable audit stamp) changed on
    // every row.
    backend
        .execute_sql(
            "INSERT INTO main.sources_users VALUES (1, 'bronze', 'stamp_b'), (2, 'silver', 'stamp_b')",
        )
        .await
        .unwrap();

    // Only `tier` is admitted comparable — `notes` is excluded from the
    // compared set entirely (the P3 walk's own job in production; here the
    // test asserts the emitted admission this record consumes).
    let suppression = key_suppression(&["tier"]);
    let dimension_batch_sql = "SELECT u.user_id, u.tier, u.notes FROM main.sources_users u";
    let w = window();

    execute_column_scoped_merge_full(
        &backend,
        "main",
        "dim_users",
        &["user_id".to_string()],
        dimension_batch_sql,
        &[],
        &suppression,
        &w,
        &no_retry_policy(),
    )
    .await
    .expect("suppressed merge succeeds");

    let (changed_keys, _partitions) = recorded_delta(&backend, "dim_users", &w)
        .await
        .expect("a delta row is recorded");
    assert!(
        changed_keys.is_empty(),
        "an Incomparable column's own flutter must never dirty the recorded delta, got: \
         {changed_keys:?}"
    );
}

/// Re-running the same window is an idempotent REPLACE — never a second
/// row for the same `(model, window_start, window_end)`.
#[tokio::test]
async fn rerunning_the_same_window_replaces_never_duplicates() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("test.duckdb");
    let backend = DuckDbBackend::new(&db_path, "main")
        .await
        .expect("open duckdb");

    backend
        .execute_sql("CREATE TABLE main.dim_users (user_id BIGINT, tier VARCHAR)")
        .await
        .unwrap();
    backend
        .execute_sql("INSERT INTO main.dim_users VALUES (1, 'bronze'), (2, 'silver')")
        .await
        .unwrap();
    backend
        .execute_sql("CREATE TABLE main.sources_users (user_id BIGINT, tier VARCHAR)")
        .await
        .unwrap();
    backend
        .execute_sql("INSERT INTO main.sources_users VALUES (1, 'bronze'), (2, 'silver')")
        .await
        .unwrap();

    let suppression = key_suppression(&["tier"]);
    let dimension_batch_sql = "SELECT u.user_id, u.tier FROM main.sources_users u";
    let w = window();

    // Run 1: no changes.
    execute_column_scoped_merge_full(
        &backend,
        "main",
        "dim_users",
        &["user_id".to_string()],
        dimension_batch_sql,
        &[],
        &suppression,
        &w,
        &no_retry_policy(),
    )
    .await
    .unwrap();

    // Mutate, then re-run over the SAME window.
    backend
        .execute_sql("UPDATE main.sources_users SET tier = 'gold' WHERE user_id = 1")
        .await
        .unwrap();
    execute_column_scoped_merge_full(
        &backend,
        "main",
        "dim_users",
        &["user_id".to_string()],
        dimension_batch_sql,
        &[],
        &suppression,
        &w,
        &no_retry_policy(),
    )
    .await
    .unwrap();

    let count_sql = "SELECT COUNT(*) AS n FROM main._smelt_observed_delta \
                      WHERE model_name = 'dim_users' AND window_start = '2026-01-01' \
                      AND window_end = '2026-01-02'";
    let batches = backend.execute_sql(count_sql).await.unwrap();
    let rows = smelt_runtime::check_runner::batches_to_rows(&batches);
    let n: u64 = rows[0].get("n").unwrap().parse().unwrap();
    assert_eq!(n, 1, "the same window must replace, never duplicate");

    let (changed_keys, _) = recorded_delta(&backend, "dim_users", &w).await.unwrap();
    assert_eq!(
        changed_keys,
        vec!["1".to_string()],
        "the replaced record must reflect the LATEST run's delta"
    );
}

/// When `window.column` names the model's declared partition column (the
/// shape `execute.rs` actually builds for a timeseries-partitioned composed
/// model), `changed_keys_select`'s `Some(partition_column)` branch projects
/// `CAST(source.<col> AS VARCHAR) AS delta_partition` — the recorded
/// `partitions` array must hold exactly the touched rows' partition values,
/// not an empty array.
#[tokio::test]
async fn partitioned_window_records_touched_partitions() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("test.duckdb");
    let backend = DuckDbBackend::new(&db_path, "main")
        .await
        .expect("open duckdb");

    backend
        .execute_sql("CREATE TABLE main.dim_users (user_id BIGINT, region VARCHAR, tier VARCHAR)")
        .await
        .unwrap();
    backend
        .execute_sql(
            "CREATE TABLE main.sources_users (user_id BIGINT, region VARCHAR, tier VARCHAR)",
        )
        .await
        .unwrap();

    // Seed 4 rows across two partitions ('east', 'west'), all identical
    // between target and source.
    backend
        .execute_sql(
            "INSERT INTO main.dim_users VALUES \
             (1, 'east', 'bronze'), (2, 'east', 'bronze'), \
             (3, 'west', 'bronze'), (4, 'west', 'bronze')",
        )
        .await
        .unwrap();
    // Mutate one row in 'east' (id 1) and one row in 'west' (id 3); leave
    // the rest unchanged.
    backend
        .execute_sql(
            "INSERT INTO main.sources_users VALUES \
             (1, 'east', 'gold'), (2, 'east', 'bronze'), \
             (3, 'west', 'gold'), (4, 'west', 'bronze')",
        )
        .await
        .unwrap();

    let suppression = key_suppression(&["tier"]);
    let dimension_batch_sql = "SELECT u.user_id, u.region, u.tier FROM main.sources_users u";
    let w = partitioned_window();

    execute_column_scoped_merge_full(
        &backend,
        "main",
        "dim_users",
        &["user_id".to_string()],
        dimension_batch_sql,
        &[],
        &suppression,
        &w,
        &no_retry_policy(),
    )
    .await
    .expect("suppressed merge succeeds");

    let (changed_keys, partitions) = recorded_delta(&backend, "dim_users", &w)
        .await
        .expect("a delta row is recorded");
    let mut sorted_keys = changed_keys.clone();
    sorted_keys.sort();
    assert_eq!(
        sorted_keys,
        vec!["1".to_string(), "3".to_string()],
        "recorded delta must hold exactly the 2 changed keys, got: {changed_keys:?}"
    );
    let mut sorted_partitions = partitions.clone();
    sorted_partitions.sort();
    assert_eq!(
        sorted_partitions,
        vec!["east".to_string(), "west".to_string()],
        "recorded partitions must hold exactly the touched partitions, got: {partitions:?}"
    );
}
