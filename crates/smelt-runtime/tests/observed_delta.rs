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
use smelt_logical::maintenance::emit::{
    emit_keyed_fold, emit_keyed_fold_suppressed, MaintenanceDialect, TargetSlicePredicate,
};
use smelt_logical::maintenance::{RowIdentity, RowIdentityVerdict};
use smelt_runtime::maintenance_driver::{
    execute_column_scoped_merge_full, execute_staged_membership_recompute,
    keyed_fold_changed_keys_select, read_observed_delta, read_observed_delta_changed_keys,
    run_windowed_keyed_maintenance, MaintenanceStep, WindowedKeyedRule,
};
use smelt_runtime::probes::ProbePolicy;
use smelt_runtime::transformer::TimeRange;

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

/// Phase 15 (`docs/outcomes/20260815-definition-delta-migrate/phases/
/// 15-plan.md`): [`read_observed_delta`] decodes BOTH `VARCHAR[]` columns
/// (`changed_keys` and `partitions`) from a real recorded row, not just the
/// `changed_keys` half `read_observed_delta_changed_keys` already covered.
#[tokio::test]
async fn read_observed_delta_decodes_both_columns() {
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
    backend
        .execute_sql(
            "INSERT INTO main.dim_users VALUES (1, 'east', 'bronze'), (2, 'west', 'bronze')",
        )
        .await
        .unwrap();
    backend
        .execute_sql(
            "INSERT INTO main.sources_users VALUES (1, 'east', 'gold'), (2, 'west', 'bronze')",
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

    let decoded = read_observed_delta(&backend, "main", "dim_users", &w.start, &w.end)
        .await
        .expect("read succeeds")
        .expect("a row was recorded");
    assert_eq!(
        decoded.changed_keys,
        vec!["1".to_string()],
        "changed_keys must decode to exactly the one changed row"
    );
    assert_eq!(
        decoded.partitions,
        vec!["east".to_string()],
        "partitions must decode to exactly the touched partition"
    );

    // No row for a window that was never recorded.
    assert!(
        read_observed_delta(&backend, "main", "dim_users", "1999-01-01", "1999-01-02")
            .await
            .expect("read succeeds")
            .is_none(),
        "an unrecorded window must decode to None, not a default-empty row"
    );
}

/// Regression on the [`read_observed_delta`] refactor: the pre-existing
/// `read_observed_delta_changed_keys` reader, now re-expressed over the
/// shared decoder, must still return `Some(&[])` (present-and-empty) vs
/// `None` (absent) distinctly.
#[tokio::test]
async fn read_observed_delta_changed_keys_shares_the_decoder() {
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
    .expect("suppressed merge succeeds (fully suppressed — nothing changed)");

    let keys = read_observed_delta_changed_keys(&backend, "main", "dim_users", &w.start, &w.end)
        .await
        .expect("read succeeds");
    assert_eq!(
        keys,
        Some(vec![]),
        "a fully-suppressed run must read back Some(&[]) — present, not absent"
    );

    let absent =
        read_observed_delta_changed_keys(&backend, "main", "dim_users", "1999-01-01", "1999-01-02")
            .await
            .expect("read succeeds");
    assert_eq!(
        absent, None,
        "a never-recorded window must read back None, not present-and-empty"
    );
}

// ── Phase 16: write-side recording for the keyed fold and staged-candidate
// families (`docs/outcomes/20260815-definition-delta-migrate/phases/
// 16-plan.md`) ──

/// A minimal [`WindowedKeyedRule`] for these tests: a single `MAX`-combiner
/// aggregator column (`GREATEST(target.score, delta.score)`), mirroring
/// `keyed`'s own shape (`crate::cumulative::CumulativeClassification`)
/// closely enough to exercise `run_windowed_keyed_maintenance`'s
/// observed-delta recording without pulling in the full `smelt-planner`
/// classification machinery.
struct TestKeyedRule {
    unique_key: Vec<String>,
    folds: Vec<(String, String)>,
}

#[async_trait::async_trait]
impl WindowedKeyedRule for TestKeyedRule {
    fn refuse(&self) -> Option<String> {
        None
    }

    fn merge_sql(
        &self,
        schema: &str,
        table: &str,
        delta_sql: &str,
        slice: Option<&TargetSlicePredicate>,
        suppression: &WriteSuppression,
        dialect: MaintenanceDialect,
    ) -> String {
        let schema_table = format!("{schema}.{table}");
        let group = match suppression {
            WriteSuppression::Suppressed { compared_columns } => emit_keyed_fold_suppressed(
                &schema_table,
                &self.unique_key,
                &self.folds,
                delta_sql,
                slice,
                compared_columns,
                dialect,
            ),
            WriteSuppression::Unconditional { .. } => emit_keyed_fold(
                &schema_table,
                &self.unique_key,
                &self.folds,
                delta_sql,
                slice,
                dialect,
            ),
        };
        group.statements[0].sql.clone()
    }

    fn observed_delta_changed_keys_sql(
        &self,
        schema: &str,
        table: &str,
        delta_sql: &str,
        compared_columns: &[String],
        partition_column: Option<&str>,
    ) -> Option<String> {
        let schema_table = format!("{schema}.{table}");
        Some(keyed_fold_changed_keys_select(
            &schema_table,
            &self.unique_key,
            delta_sql,
            compared_columns,
            &self.folds,
            partition_column,
        ))
    }
}

fn max_score_rule() -> TestKeyedRule {
    TestKeyedRule {
        unique_key: vec!["user_id".to_string()],
        folds: vec![(
            "score".to_string(),
            "GREATEST(target.score, delta.score)".to_string(),
        )],
    }
}

/// Same shape as [`TestKeyedRule`], but `merge_sql` returns intentionally
/// broken SQL (a `MERGE` referencing a column the target table does not
/// have) — used to prove the recorded delta and the write share one
/// commit point (test 8: a failed write leaves no delta row behind).
struct FailingMergeKeyedRule {
    inner: TestKeyedRule,
}

#[async_trait::async_trait]
impl WindowedKeyedRule for FailingMergeKeyedRule {
    fn refuse(&self) -> Option<String> {
        None
    }

    fn merge_sql(
        &self,
        schema: &str,
        table: &str,
        _delta_sql: &str,
        _slice: Option<&TargetSlicePredicate>,
        _suppression: &WriteSuppression,
        _dialect: MaintenanceDialect,
    ) -> String {
        format!(
            "MERGE INTO {schema}.{table} AS target USING (SELECT 1 AS user_id) AS delta ON \
             target.user_id = delta.user_id WHEN MATCHED THEN UPDATE SET \
             does_not_exist_column = 1"
        )
    }

    fn observed_delta_changed_keys_sql(
        &self,
        schema: &str,
        table: &str,
        delta_sql: &str,
        compared_columns: &[String],
        partition_column: Option<&str>,
    ) -> Option<String> {
        self.inner.observed_delta_changed_keys_sql(
            schema,
            table,
            delta_sql,
            compared_columns,
            partition_column,
        )
    }
}

fn one_step(start: &str, end: &str) -> Vec<MaintenanceStep> {
    vec![MaintenanceStep {
        partition_value: start.to_string(),
        range: TimeRange {
            start: start.to_string(),
            end: end.to_string(),
        },
    }]
}

/// A suppressed keyed-fold step over 100 rows, 3 of which change (a higher
/// `score` for a `GREATEST` fold), must record exactly those 3 keys under
/// `(model, step.range.start, step.range.end)`.
#[tokio::test]
async fn keyed_fold_suppressed_records_changed_keys() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("test.duckdb");
    let backend = DuckDbBackend::new(&db_path, "main")
        .await
        .expect("open duckdb");

    backend
        .execute_sql("CREATE TABLE main.dim_scores (user_id BIGINT, score BIGINT)")
        .await
        .unwrap();
    backend
        .execute_sql("CREATE TABLE main.src_scores (user_id BIGINT, score BIGINT)")
        .await
        .unwrap();

    let mut target_values = Vec::new();
    let mut source_values = Vec::new();
    for i in 0..100i64 {
        target_values.push(format!("({i}, 10)"));
        source_values.push(format!("({i}, 10)"));
    }
    backend
        .execute_sql(&format!(
            "INSERT INTO main.dim_scores VALUES {}",
            target_values.join(", ")
        ))
        .await
        .unwrap();
    for id in [1i64, 42, 99] {
        source_values[id as usize] = format!("({id}, 99)");
    }
    backend
        .execute_sql(&format!(
            "INSERT INTO main.src_scores VALUES {}",
            source_values.join(", ")
        ))
        .await
        .unwrap();

    let suppression = key_suppression(&["score"]);
    let steps = one_step("2026-01-01", "2026-01-02");

    run_windowed_keyed_maintenance(
        &backend,
        "dim_scores",
        "main",
        "dim_scores",
        &steps,
        &max_score_rule(),
        None,
        &suppression,
        |_step| Ok("SELECT user_id, score FROM main.src_scores".to_string()),
        &no_retry_policy(),
        &ProbePolicy::per_run(),
    )
    .await
    .expect("suppressed keyed fold succeeds");

    let w = PartitionRange {
        column: String::new(),
        start: "2026-01-01".to_string(),
        end: "2026-01-02".to_string(),
    };
    let (changed_keys, _partitions) = recorded_delta(&backend, "dim_scores", &w)
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

/// A fully-suppressed keyed-fold step (delta never raises any stored
/// `score`) must record a PRESENT-AND-EMPTY delta.
#[tokio::test]
async fn keyed_fold_fully_suppressed_records_an_empty_delta() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("test.duckdb");
    let backend = DuckDbBackend::new(&db_path, "main")
        .await
        .expect("open duckdb");

    backend
        .execute_sql("CREATE TABLE main.dim_scores (user_id BIGINT, score BIGINT)")
        .await
        .unwrap();
    backend
        .execute_sql("INSERT INTO main.dim_scores VALUES (1, 10), (2, 20)")
        .await
        .unwrap();
    backend
        .execute_sql("CREATE TABLE main.src_scores (user_id BIGINT, score BIGINT)")
        .await
        .unwrap();
    // Every delta score is <= the stored score, so GREATEST never changes
    // anything.
    backend
        .execute_sql("INSERT INTO main.src_scores VALUES (1, 5), (2, 20)")
        .await
        .unwrap();

    let suppression = key_suppression(&["score"]);
    let steps = one_step("2026-01-01", "2026-01-02");

    run_windowed_keyed_maintenance(
        &backend,
        "dim_scores",
        "main",
        "dim_scores",
        &steps,
        &max_score_rule(),
        None,
        &suppression,
        |_step| Ok("SELECT user_id, score FROM main.src_scores".to_string()),
        &no_retry_policy(),
        &ProbePolicy::per_run(),
    )
    .await
    .expect("suppressed keyed fold succeeds");

    let w = PartitionRange {
        column: String::new(),
        start: "2026-01-01".to_string(),
        end: "2026-01-02".to_string(),
    };
    let (changed_keys, partitions) = recorded_delta(&backend, "dim_scores", &w)
        .await
        .expect("a delta row is recorded even when nothing changed (present-and-empty)");
    assert!(
        changed_keys.is_empty(),
        "a fully-suppressed run must record an empty changed-key set, got: {changed_keys:?}"
    );
    assert!(partitions.is_empty(), "partitions must also be empty");
}

/// An `Unconditional` verdict's keyed-fold merge must leave the observed-
/// delta table untouched — the record is a byproduct of the SUPPRESSED
/// write's already-computed changed-row set, never derived after the fact
/// for an unconditional one.
#[tokio::test]
async fn keyed_fold_unconditional_records_no_delta() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("test.duckdb");
    let backend = DuckDbBackend::new(&db_path, "main")
        .await
        .expect("open duckdb");

    backend
        .execute_sql("CREATE TABLE main.dim_scores (user_id BIGINT, score BIGINT)")
        .await
        .unwrap();
    backend
        .execute_sql("INSERT INTO main.dim_scores VALUES (1, 10), (2, 20)")
        .await
        .unwrap();
    backend
        .execute_sql("CREATE TABLE main.src_scores (user_id BIGINT, score BIGINT)")
        .await
        .unwrap();
    backend
        .execute_sql("INSERT INTO main.src_scores VALUES (1, 99), (2, 99)")
        .await
        .unwrap();

    let suppression = WriteSuppression::Unconditional {
        why: "test: unconditional keyed fold must never record a delta".to_string(),
    };
    let steps = one_step("2026-01-01", "2026-01-02");

    run_windowed_keyed_maintenance(
        &backend,
        "dim_scores",
        "main",
        "dim_scores",
        &steps,
        &max_score_rule(),
        None,
        &suppression,
        |_step| Ok("SELECT user_id, score FROM main.src_scores".to_string()),
        &no_retry_policy(),
        &ProbePolicy::per_run(),
    )
    .await
    .expect("unconditional keyed fold succeeds");

    // An `Unconditional` write never even ensures the observed-delta table
    // exists — it is a pure byproduct of a SUPPRESSED write's own change
    // detection, so the table is entirely absent here, not merely empty.
    assert!(
        !backend
            .table_exists("main", "_smelt_observed_delta")
            .await
            .unwrap(),
        "an Unconditional keyed-fold write must never create the observed-delta table at all"
    );
}

/// The changed-keys query is derived and executed BEFORE the merge, inside
/// the SAME transaction: when the merge statement itself fails, the whole
/// transaction rolls back, leaving no delta row behind — proof the record
/// and the write share one commit point (mirrors
/// `smelt-backend-duckdb/src/lib.rs::
/// test_record_observed_delta_rolls_back_record_on_write_failure`).
#[tokio::test]
async fn keyed_fold_delta_rolls_back_with_a_failed_write() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("test.duckdb");
    let backend = DuckDbBackend::new(&db_path, "main")
        .await
        .expect("open duckdb");

    backend
        .execute_sql("CREATE TABLE main.dim_scores (user_id BIGINT, score BIGINT)")
        .await
        .unwrap();
    backend
        .execute_sql("INSERT INTO main.dim_scores VALUES (1, 10), (2, 20)")
        .await
        .unwrap();
    backend
        .execute_sql("CREATE TABLE main.src_scores (user_id BIGINT, score BIGINT)")
        .await
        .unwrap();
    backend
        .execute_sql("INSERT INTO main.src_scores VALUES (1, 99), (2, 99)")
        .await
        .unwrap();

    let suppression = key_suppression(&["score"]);
    let steps = one_step("2026-01-01", "2026-01-02");
    let rule = FailingMergeKeyedRule {
        inner: max_score_rule(),
    };

    let err = run_windowed_keyed_maintenance(
        &backend,
        "dim_scores",
        "main",
        "dim_scores",
        &steps,
        &rule,
        None,
        &suppression,
        |_step| Ok("SELECT user_id, score FROM main.src_scores".to_string()),
        &no_retry_policy(),
        &ProbePolicy::per_run(),
    )
    .await
    .expect_err("a broken MERGE statement must fail the run");
    assert!(
        err.to_string().contains("does_not_exist_column")
            || err.to_string().to_lowercase().contains("column"),
        "the error should name the SQL failure, got: {err}"
    );

    let w = PartitionRange {
        column: String::new(),
        start: "2026-01-01".to_string(),
        end: "2026-01-02".to_string(),
    };
    assert!(
        recorded_delta(&backend, "dim_scores", &w).await.is_none(),
        "a failed write must roll back the delta record too — no row at all"
    );
}

/// A non-DuckDB target refuses fail-loud rather than silently skipping the
/// observed-delta record — the same posture
/// `execute_column_scoped_write_with_observed_delta` already takes for its
/// own `Suppressed` arm.
struct KeyedNonDuckDbBackend;

#[async_trait::async_trait]
impl Backend for KeyedNonDuckDbBackend {
    async fn execute_sql(
        &self,
        _sql: &str,
    ) -> Result<Vec<arrow::array::RecordBatch>, smelt_backend::BackendError> {
        unimplemented!("must not be called — the driver refuses before any write")
    }
    async fn create_table_as(
        &self,
        _: &str,
        _: &str,
        _: &str,
    ) -> Result<(), smelt_backend::BackendError> {
        unimplemented!()
    }
    async fn create_view_as(
        &self,
        _: &str,
        _: &str,
        _: &str,
    ) -> Result<(), smelt_backend::BackendError> {
        unimplemented!()
    }
    async fn drop_table_if_exists(
        &self,
        _: &str,
        _: &str,
    ) -> Result<(), smelt_backend::BackendError> {
        unimplemented!()
    }
    async fn drop_view_if_exists(
        &self,
        _: &str,
        _: &str,
    ) -> Result<(), smelt_backend::BackendError> {
        unimplemented!()
    }
    async fn get_row_count(&self, _: &str, _: &str) -> Result<usize, smelt_backend::BackendError> {
        unimplemented!()
    }
    async fn get_preview(
        &self,
        _: &str,
        _: &str,
        _: usize,
    ) -> Result<Vec<arrow::array::RecordBatch>, smelt_backend::BackendError> {
        unimplemented!()
    }
    async fn table_exists(&self, _: &str, _: &str) -> Result<bool, smelt_backend::BackendError> {
        // The target already exists — so the driver reaches the merge
        // (not the first-run `CREATE TABLE ... AS`) branch, where the
        // dialect refusal lives.
        Ok(true)
    }
    async fn ensure_schema(&self, _: &str) -> Result<(), smelt_backend::BackendError> {
        unimplemented!()
    }
    fn dialect(&self) -> smelt_backend::SqlDialect {
        smelt_backend::SqlDialect::SparkSQL
    }
    fn capabilities(&self) -> smelt_backend::BackendCapabilities {
        unimplemented!()
    }
    async fn load_table(
        &self,
        _: &str,
        _: &str,
        _: arrow::datatypes::SchemaRef,
        _: Vec<arrow::array::RecordBatch>,
    ) -> Result<(), smelt_backend::BackendError> {
        unimplemented!()
    }
    async fn delete_partitions(
        &self,
        _: &str,
        _: &str,
        _: &PartitionRange,
    ) -> Result<(), smelt_backend::BackendError> {
        unimplemented!()
    }
    async fn insert_into_from_query(
        &self,
        _: &str,
        _: &str,
        _: &str,
    ) -> Result<(), smelt_backend::BackendError> {
        unimplemented!()
    }
    async fn insert_overwrite(
        &self,
        _: &str,
        _: &str,
        _: &str,
        _: &PartitionRange,
    ) -> Result<(), smelt_backend::BackendError> {
        unimplemented!()
    }
}

#[tokio::test]
async fn keyed_fold_suppressed_recording_refuses_a_non_duckdb_backend() {
    let backend = KeyedNonDuckDbBackend;
    let suppression = key_suppression(&["score"]);
    let steps = one_step("2026-01-01", "2026-01-02");

    let err = run_windowed_keyed_maintenance(
        &backend,
        "dim_scores",
        "main",
        "dim_scores",
        &steps,
        &max_score_rule(),
        None,
        &suppression,
        |_step| Ok("SELECT user_id, score FROM main.src_scores".to_string()),
        &no_retry_policy(),
        &ProbePolicy::per_run(),
    )
    .await
    .expect_err("a non-DuckDB backend must refuse observed-delta recording");
    assert!(
        err.to_string().contains("observed-delta"),
        "the refusal should name the observed-delta recording capability, got: {err}"
    );
}

/// The staged-candidate conditional recompute records the keys whose
/// applied effect was not the identity — new, changed, or departed.
#[tokio::test]
async fn staged_membership_recompute_records_changed_keys() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("test.duckdb");
    let backend = DuckDbBackend::new(&db_path, "main")
        .await
        .expect("open duckdb");

    backend
        .execute_sql("CREATE TABLE main.dim_members (user_id BIGINT, tier VARCHAR)")
        .await
        .unwrap();
    // 1: unchanged. 2: changed. 3: departed (absent from the candidate).
    backend
        .execute_sql(
            "INSERT INTO main.dim_members VALUES (1, 'bronze'), (2, 'bronze'), (3, 'bronze')",
        )
        .await
        .unwrap();

    // Candidate: 1 unchanged, 2 changed to 'gold', 4 brand new. 3 is absent
    // (departed).
    let candidate_select =
        "SELECT * FROM (VALUES (1, 'bronze'), (2, 'gold'), (4, 'bronze')) AS t(user_id, tier)";

    let w = PartitionRange {
        column: String::new(),
        start: "2026-01-01".to_string(),
        end: "2026-01-02".to_string(),
    };

    execute_staged_membership_recompute(
        &backend,
        "main",
        "dim_members",
        &["user_id".to_string()],
        candidate_select,
        &["tier".to_string()],
        &w,
        &no_retry_policy(),
    )
    .await
    .expect("staged-candidate recompute succeeds");

    let (changed_keys, _partitions) = recorded_delta(&backend, "dim_members", &w)
        .await
        .expect("a delta row is recorded");
    let mut sorted = changed_keys.clone();
    sorted.sort();
    assert_eq!(
        sorted,
        vec!["2".to_string(), "3".to_string(), "4".to_string()],
        "recorded delta must hold exactly the changed (2), departed (3), and new (4) keys, got: \
         {changed_keys:?}"
    );
}

/// When the candidate is identical to the stored state (nothing changed,
/// nothing departed, nothing new), the recorded delta must be
/// present-and-empty.
#[tokio::test]
async fn staged_membership_recompute_records_an_empty_delta_when_nothing_changed() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("test.duckdb");
    let backend = DuckDbBackend::new(&db_path, "main")
        .await
        .expect("open duckdb");

    backend
        .execute_sql("CREATE TABLE main.dim_members (user_id BIGINT, tier VARCHAR)")
        .await
        .unwrap();
    backend
        .execute_sql("INSERT INTO main.dim_members VALUES (1, 'bronze'), (2, 'silver')")
        .await
        .unwrap();

    let candidate_select =
        "SELECT * FROM (VALUES (1, 'bronze'), (2, 'silver')) AS t(user_id, tier)";

    let w = PartitionRange {
        column: String::new(),
        start: "2026-01-01".to_string(),
        end: "2026-01-02".to_string(),
    };

    execute_staged_membership_recompute(
        &backend,
        "main",
        "dim_members",
        &["user_id".to_string()],
        candidate_select,
        &["tier".to_string()],
        &w,
        &no_retry_policy(),
    )
    .await
    .expect("staged-candidate recompute succeeds");

    let (changed_keys, partitions) = recorded_delta(&backend, "dim_members", &w)
        .await
        .expect("a delta row is recorded even when nothing changed (present-and-empty)");
    assert!(
        changed_keys.is_empty(),
        "an unchanged staged-candidate run must record an empty changed-key set, got: \
         {changed_keys:?}"
    );
    assert!(partitions.is_empty(), "partitions must also be empty");
}
