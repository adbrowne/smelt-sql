//! Live-tier tests for the `Backend` trait implementation, driven against a
//! real Trino coordinator + Iceberg REST catalog (`scripts/trino-up.sh`).
//!
//! Gated on `SMELT_TRINO_URL` (`scripts/trino-env.sh`): unset, every test
//! here skips green — the default `cargo test` stays backend-agnostic
//! (mirrors `scripts/spark-env.sh`'s `SPARK_CONNECT_URL` gate).
//!
//! Run with:
//!   bash scripts/trino-up.sh
//!   source scripts/trino-env.sh
//!   cargo test -p smelt-backend-trino --test backend_live
//!   bash scripts/trino-down.sh

use arrow::array::{Array, Int32Array};
use smelt_backend::{Backend, BackendError, Materialization};
use smelt_backend_trino::{TrinoBackend, TrinoClientConfig};

/// One live-run's connection, catalog and isolated schema.
struct LiveEnv {
    backend: TrinoBackend,
    catalog: String,
    schema: String,
}

/// A schema name unique to this run, so two worktrees — or a developer beside
/// an autonomy loop — never collide.
fn unique_schema() -> String {
    let base = std::env::var("SMELT_TRINO_SCHEMA").unwrap_or_else(|_| "smelt_dev".to_string());
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    format!("{base}_{}_{nanos}", std::process::id())
}

/// Quote and join a catalog-qualified name the same way `TrinoBackend`
/// itself does — kept local since the backend's own quoting is crate-private.
fn qualified(catalog: &str, schema: &str, name: &str) -> String {
    format!("\"{catalog}\".\"{schema}\".\"{name}\"")
}

/// Connect, create the run's isolated schema, and return the env — or
/// `None` when `SMELT_TRINO_URL` is unset, meaning the caller should skip.
async fn live_env_or_skip(test_name: &str) -> Option<LiveEnv> {
    let Ok(base_url) = std::env::var("SMELT_TRINO_URL") else {
        eprintln!("Skipping {test_name} — set SMELT_TRINO_URL");
        return None;
    };
    let user = std::env::var("SMELT_TRINO_USER").unwrap_or_else(|_| "smelt".to_string());
    let catalog = std::env::var("SMELT_TRINO_CATALOG").unwrap_or_else(|_| "iceberg".to_string());
    let schema = unique_schema();

    let backend = TrinoBackend::new(TrinoClientConfig {
        base_url,
        user,
        catalog: catalog.clone(),
        schema: schema.clone(),
        password: None,
    });
    backend
        .ensure_schema(&schema)
        .await
        .unwrap_or_else(|e| panic!("ensure_schema must succeed against a live tier: {e}"));

    Some(LiveEnv {
        backend,
        catalog,
        schema,
    })
}

/// Best-effort teardown of the run's isolated schema. Not a hard assertion —
/// a failure here should not fail the test whose assertions already ran.
async fn drop_schema(env: &LiveEnv) {
    let _ = env
        .backend
        .execute_sql(&format!(
            "DROP SCHEMA IF EXISTS \"{}\".\"{}\"",
            env.catalog, env.schema
        ))
        .await;
}

#[tokio::test]
async fn ensure_schema_is_idempotent() {
    let Some(env) = live_env_or_skip("ensure_schema_is_idempotent").await else {
        return;
    };
    env.backend
        .ensure_schema(&env.schema)
        .await
        .expect("second ensure_schema must be a no-op, not an error");

    drop_schema(&env).await;
}

#[tokio::test]
async fn table_exists_is_false_then_true_then_false() {
    let Some(env) = live_env_or_skip("table_exists_is_false_then_true_then_false").await else {
        return;
    };

    assert!(!env
        .backend
        .table_exists(&env.schema, "smelt_w6p6_texists")
        .await
        .unwrap());

    env.backend
        .create_table_as(&env.schema, "smelt_w6p6_texists", "SELECT 1 AS id")
        .await
        .expect("create_table_as must succeed");
    assert!(env
        .backend
        .table_exists(&env.schema, "smelt_w6p6_texists")
        .await
        .unwrap());

    env.backend
        .drop_table_if_exists(&env.schema, "smelt_w6p6_texists")
        .await
        .expect("drop_table_if_exists must succeed");
    assert!(!env
        .backend
        .table_exists(&env.schema, "smelt_w6p6_texists")
        .await
        .unwrap());

    drop_schema(&env).await;
}

#[tokio::test]
async fn create_table_as_then_row_count_and_preview() {
    let Some(env) = live_env_or_skip("create_table_as_then_row_count_and_preview").await else {
        return;
    };

    env.backend
        .create_table_as(
            &env.schema,
            "smelt_w6p6_rowcount",
            "SELECT * FROM (VALUES (1, 'a'), (2, 'b'), (3, 'c')) AS t(id, label)",
        )
        .await
        .expect("create_table_as must succeed");

    let count = env
        .backend
        .get_row_count(&env.schema, "smelt_w6p6_rowcount")
        .await
        .expect("get_row_count must succeed");
    assert_eq!(count, 3);

    let preview = env
        .backend
        .get_preview(&env.schema, "smelt_w6p6_rowcount", 2)
        .await
        .expect("get_preview must succeed");
    let total_rows: usize = preview.iter().map(|b| b.num_rows()).sum();
    assert_eq!(total_rows, 2);
    let column_names: Vec<String> = preview[0]
        .schema()
        .fields()
        .iter()
        .map(|f| f.name().clone())
        .collect();
    assert_eq!(column_names, vec!["id".to_string(), "label".to_string()]);

    drop_schema(&env).await;
}

#[tokio::test]
async fn create_view_as_then_read_back_and_drop() {
    let Some(env) = live_env_or_skip("create_view_as_then_read_back_and_drop").await else {
        return;
    };

    env.backend
        .create_table_as(
            &env.schema,
            "smelt_w6p6_view_base",
            "SELECT * FROM (VALUES (1, 'x'), (2, 'y')) AS t(id, label)",
        )
        .await
        .expect("base table create_table_as must succeed");

    let base_table = qualified(&env.catalog, &env.schema, "smelt_w6p6_view_base");
    env.backend
        .create_view_as(
            &env.schema,
            "smelt_w6p6_view",
            &format!("SELECT id FROM {base_table}"),
        )
        .await
        .expect("create_view_as must succeed");

    let view = qualified(&env.catalog, &env.schema, "smelt_w6p6_view");
    let batches = env
        .backend
        .execute_sql(&format!("SELECT id FROM {view} ORDER BY id"))
        .await
        .expect("querying the view must succeed");
    // `VALUES (1, 'x')` infers the integer column as Trino `integer`
    // (Arrow `Int32`), not `bigint` — unlike `count(*)`, which Trino always
    // types as `bigint` (Arrow `Int64`, decoded via `decode_bigint_count`).
    let ids: Vec<i32> = batches
        .iter()
        .flat_map(|b| {
            b.column(0)
                .as_any()
                .downcast_ref::<Int32Array>()
                .unwrap()
                .values()
                .to_vec()
        })
        .collect();
    assert_eq!(ids, vec![1, 2]);

    env.backend
        .drop_view_if_exists(&env.schema, "smelt_w6p6_view")
        .await
        .expect("drop_view_if_exists must succeed");

    drop_schema(&env).await;
}

#[tokio::test]
async fn execute_model_materializes_a_table_and_a_view() {
    let Some(env) = live_env_or_skip("execute_model_materializes_a_table_and_a_view").await else {
        return;
    };

    let table_result = env
        .backend
        .execute_model(
            &env.schema,
            "smelt_w6p6_exec_table",
            "SELECT * FROM (VALUES (1), (2), (3)) AS t(n)",
            Materialization::Table,
            false,
        )
        .await
        .expect("execute_model (table) must succeed");
    assert_eq!(table_result.row_count, 3);

    let base_table = qualified(&env.catalog, &env.schema, "smelt_w6p6_exec_table");
    let view_result = env
        .backend
        .execute_model(
            &env.schema,
            "smelt_w6p6_exec_view",
            &format!("SELECT n FROM {base_table}"),
            Materialization::View,
            false,
        )
        .await
        .expect("execute_model (view) must succeed");
    assert_eq!(view_result.row_count, 3);

    let preview = env
        .backend
        .get_preview(&env.schema, "smelt_w6p6_exec_view", 10)
        .await
        .expect("reading the materialized view back must succeed");
    let total_rows: usize = preview.iter().map(|b| b.num_rows()).sum();
    assert_eq!(total_rows, 3);

    drop_schema(&env).await;
}

#[tokio::test]
async fn a_bad_statement_maps_to_a_typed_error() {
    let Some(env) = live_env_or_skip("a_bad_statement_maps_to_a_typed_error").await else {
        return;
    };

    let err = env
        .backend
        .execute_sql("SELECT * FROM does_not_exist_smelt_w6p6")
        .await
        .expect_err("querying a nonexistent table must fail");
    assert!(
        matches!(
            err,
            BackendError::ExecutionFailed { .. } | BackendError::NotFound { .. }
        ),
        "expected a typed BackendError, got: {err:?}"
    );

    drop_schema(&env).await;
}

#[tokio::test]
async fn drop_table_if_exists_on_a_missing_table_is_ok() {
    let Some(env) = live_env_or_skip("drop_table_if_exists_on_a_missing_table_is_ok").await else {
        return;
    };

    env.backend
        .drop_table_if_exists(&env.schema, "smelt_w6p6_never_existed")
        .await
        .expect("dropping a table that never existed must be Ok, not an error");

    drop_schema(&env).await;
}
