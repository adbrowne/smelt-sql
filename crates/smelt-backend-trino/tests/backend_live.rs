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

use std::sync::Arc;

use arrow::array::{
    Array, BooleanArray, Date32Array, Decimal128Array, Float64Array, Int32Array, Int64Array,
    RecordBatch, StringArray, TimestampMicrosecondArray,
};
use arrow::datatypes::{DataType, Field, Schema, SchemaRef, TimeUnit};
use smelt_backend::{
    Backend, BackendError, Materialization, PartitionAxis, PartitionColumnType, PartitionRange,
};
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

/// The whole seed type set of `seeds.md` §"Type inference", every column
/// nullable — matches the schema `rows_to_record_batch` always reads back
/// with (`Field::new(&c.name, dt.clone(), true)`), so a round trip through
/// `load_table` + `execute_sql` can assert field-for-field schema equality.
fn seed_type_set_schema() -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new("c_bool", DataType::Boolean, true),
        Field::new("c_i32", DataType::Int32, true),
        Field::new("c_i64", DataType::Int64, true),
        Field::new("c_decimal", DataType::Decimal128(18, 4), true),
        Field::new("c_double", DataType::Float64, true),
        Field::new("c_date", DataType::Date32, true),
        Field::new(
            "c_timestamp",
            DataType::Timestamp(TimeUnit::Microsecond, None),
            true,
        ),
        Field::new("c_string", DataType::Utf8, true),
    ]))
}

/// Three rows over [`seed_type_set_schema`]: row 1 (index 1) is NULL in
/// every column, rows 0 and 2 carry values — one NULL row per column, as
/// phase 7's test list specifies.
fn seed_type_set_batch(schema: SchemaRef) -> RecordBatch {
    RecordBatch::try_new(
        schema,
        vec![
            Arc::new(BooleanArray::from(vec![Some(true), None, Some(false)])) as _,
            Arc::new(Int32Array::from(vec![Some(1), None, Some(3)])) as _,
            Arc::new(Int64Array::from(vec![
                Some(10_000_000_000_i64),
                None,
                Some(3),
            ])) as _,
            Arc::new(
                Decimal128Array::from(vec![Some(12_345_600_i128), None, Some(-500_i128)])
                    .with_precision_and_scale(18, 4)
                    .unwrap(),
            ) as _,
            Arc::new(Float64Array::from(vec![Some(1.5), None, Some(-2.25)])) as _,
            // 2024-01-15 and 2024-06-01, days since Unix epoch.
            Arc::new(Date32Array::from(vec![Some(19737), None, Some(19875)])) as _,
            Arc::new(TimestampMicrosecondArray::from(vec![
                Some(1_705_318_861_123_456_i64),
                None,
                Some(1_717_200_000_000_000_i64),
            ])) as _,
            Arc::new(StringArray::from(vec![
                Some("hello"),
                None,
                Some("it's ok"),
            ])) as _,
        ],
    )
    .expect("seed_type_set_batch: schema/data mismatch")
}

#[tokio::test]
async fn load_table_round_trips_the_whole_seed_type_set() {
    let Some(env) = live_env_or_skip("load_table_round_trips_the_whole_seed_type_set").await else {
        return;
    };

    let schema = seed_type_set_schema();
    let batch = seed_type_set_batch(schema.clone());
    env.backend
        .load_table(
            &env.schema,
            "smelt_w7p7_roundtrip",
            schema.clone(),
            vec![batch],
        )
        .await
        .expect("load_table must succeed over the whole seed type set");

    let table = qualified(&env.catalog, &env.schema, "smelt_w7p7_roundtrip");
    let batches = env
        .backend
        .execute_sql(&format!("SELECT * FROM {table} ORDER BY c_i32 NULLS FIRST"))
        .await
        .expect("reading the loaded table back must succeed");

    assert_eq!(batches.len(), 1, "expected the whole result in one page");
    let out = &batches[0];
    assert_eq!(
        out.schema().as_ref(),
        schema.as_ref(),
        "round-tripped schema must be field-for-field equal to the input schema"
    );
    assert_eq!(out.num_rows(), 3);

    // Row order: NULL sorts first, so index 0 is the all-NULL row, then the
    // two value rows in ascending c_i32 order (1, 3).
    let bools = out
        .column(0)
        .as_any()
        .downcast_ref::<BooleanArray>()
        .unwrap();
    assert!(bools.is_null(0));
    assert!(bools.value(1));
    assert!(!bools.value(2));

    let i32s = out.column(1).as_any().downcast_ref::<Int32Array>().unwrap();
    assert!(i32s.is_null(0));
    assert_eq!(i32s.value(1), 1);
    assert_eq!(i32s.value(2), 3);

    let i64s = out.column(2).as_any().downcast_ref::<Int64Array>().unwrap();
    assert!(i64s.is_null(0));
    assert_eq!(i64s.value(1), 10_000_000_000_i64);
    assert_eq!(i64s.value(2), 3_i64);

    let decimals = out
        .column(3)
        .as_any()
        .downcast_ref::<Decimal128Array>()
        .unwrap();
    assert!(decimals.is_null(0));
    assert_eq!(decimals.value(1), 12_345_600_i128);
    assert_eq!(decimals.value(2), -500_i128);

    let doubles = out
        .column(4)
        .as_any()
        .downcast_ref::<Float64Array>()
        .unwrap();
    assert!(doubles.is_null(0));
    assert_eq!(doubles.value(1), 1.5);
    assert_eq!(doubles.value(2), -2.25);

    let dates = out
        .column(5)
        .as_any()
        .downcast_ref::<Date32Array>()
        .unwrap();
    assert!(dates.is_null(0));
    assert_eq!(dates.value(1), 19737);
    assert_eq!(dates.value(2), 19875);

    let timestamps = out
        .column(6)
        .as_any()
        .downcast_ref::<TimestampMicrosecondArray>()
        .unwrap();
    assert!(timestamps.is_null(0));
    assert_eq!(timestamps.value(1), 1_705_318_861_123_456_i64);
    assert_eq!(timestamps.value(2), 1_717_200_000_000_000_i64);

    let strings = out
        .column(7)
        .as_any()
        .downcast_ref::<StringArray>()
        .unwrap();
    assert!(strings.is_null(0));
    assert_eq!(strings.value(1), "hello");
    assert_eq!(strings.value(2), "it's ok");

    drop_schema(&env).await;
}

#[tokio::test]
async fn load_table_replaces_an_existing_table() {
    let Some(env) = live_env_or_skip("load_table_replaces_an_existing_table").await else {
        return;
    };

    let schema: SchemaRef = Arc::new(Schema::new(vec![Field::new("n", DataType::Int32, true)]));
    let first_batch = RecordBatch::try_new(
        schema.clone(),
        vec![Arc::new(Int32Array::from(vec![1, 2, 3])) as _],
    )
    .unwrap();
    env.backend
        .load_table(
            &env.schema,
            "smelt_w7p7_replace",
            schema.clone(),
            vec![first_batch],
        )
        .await
        .expect("first load_table must succeed");
    assert_eq!(
        env.backend
            .get_row_count(&env.schema, "smelt_w7p7_replace")
            .await
            .unwrap(),
        3
    );

    let second_batch = RecordBatch::try_new(
        schema.clone(),
        vec![Arc::new(Int32Array::from(vec![10, 20])) as _],
    )
    .unwrap();
    env.backend
        .load_table(
            &env.schema,
            "smelt_w7p7_replace",
            schema,
            vec![second_batch],
        )
        .await
        .expect("second load_table must succeed and replace the first");

    // Only the second load's rows survive (the trait's drop-then-create
    // contract), not 3 + 2 = 5.
    assert_eq!(
        env.backend
            .get_row_count(&env.schema, "smelt_w7p7_replace")
            .await
            .unwrap(),
        2
    );

    drop_schema(&env).await;
}

#[tokio::test]
async fn load_table_rejects_null_in_non_nullable_against_the_live_tier() {
    let Some(env) =
        live_env_or_skip("load_table_rejects_null_in_non_nullable_against_the_live_tier").await
    else {
        return;
    };

    let schema: SchemaRef = Arc::new(Schema::new(vec![Field::new("n", DataType::Int32, false)]));
    let batch = RecordBatch::try_new(
        // The batch's own field stays nullable so it can carry the NULL that
        // violates the stricter `schema` passed to `load_table` below.
        Arc::new(Schema::new(vec![Field::new("n", DataType::Int32, true)])),
        vec![Arc::new(Int32Array::from(vec![Some(1), None])) as _],
    )
    .unwrap();

    let err = env
        .backend
        .load_table(&env.schema, "smelt_w7p7_null_reject", schema, vec![batch])
        .await
        .expect_err("a NULL in a non-nullable column must be refused");
    assert!(matches!(err, BackendError::NullInNonNullableColumn { .. }));

    assert!(
        !env.backend
            .table_exists(&env.schema, "smelt_w7p7_null_reject")
            .await
            .unwrap(),
        "no table must be left behind after a rejected load"
    );

    drop_schema(&env).await;
}

#[tokio::test]
async fn load_table_loads_a_multi_chunk_batch() {
    let Some(env) = live_env_or_skip("load_table_loads_a_multi_chunk_batch").await else {
        return;
    };

    // Above the 1000-row chunk bound `INSERT_CHUNK_SIZE` sets, so this also
    // measures multi-statement bulk-load timing (recorded in the phase 7
    // summary and the outcome's decision log).
    let row_count = 12_000_i32;
    let schema: SchemaRef = Arc::new(Schema::new(vec![Field::new("n", DataType::Int32, true)]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![Arc::new(Int32Array::from((0..row_count).collect::<Vec<_>>())) as _],
    )
    .unwrap();

    let start = std::time::Instant::now();
    env.backend
        .load_table(&env.schema, "smelt_w7p7_multichunk", schema, vec![batch])
        .await
        .expect("multi-chunk load_table must succeed");
    let elapsed = start.elapsed();
    eprintln!(
        "load_table_loads_a_multi_chunk_batch: {row_count} rows in {:?} ({:.1} rows/sec)",
        elapsed,
        row_count as f64 / elapsed.as_secs_f64()
    );

    let count = env
        .backend
        .get_row_count(&env.schema, "smelt_w7p7_multichunk")
        .await
        .expect("get_row_count must succeed");
    assert_eq!(count, row_count as usize);

    drop_schema(&env).await;
}

/// Phase 10 of `20260913-trino-emission`: decides whether the "array
/// doesn't decode to Arrow" divergence entry in
/// `docs/specs/multi_backend.md` §Known Divergences is closed or must be
/// narrowed. `trino_type_to_arrow` (phase 9) already maps the `array(...)`
/// type *signature* to `DataType::List` (`build_column`'s `data_types` pass
/// succeeds), but `build_column` itself has no `DataType::List` builder arm,
/// so a projected `ARRAY[...]` column fails at the *cell* decode step. This
/// asserts that specific failure mode rather than the broader "unrecognised
/// type signature" error, so the divergence entry can name the cell decoder
/// precisely, not the type map phase 9 already fixed. If this test starts
/// failing because `build_column` gained a `List` arm, delete it and close
/// the divergence entry instead of updating the assertion.
#[tokio::test]
async fn array_result_column_decodes_to_arrow() {
    let Some(env) = live_env_or_skip("array_result_column_decodes_to_arrow").await else {
        return;
    };

    let err = env
        .backend
        .execute_sql("SELECT ARRAY[1, 2, 3] AS xs")
        .await
        .expect_err(
            "array result columns do not decode yet (narrowed divergence, \
             docs/specs/multi_backend.md §Known Divergences) — remove this \
             test and close the entry if this starts succeeding",
        );
    let message = err.to_string();
    assert!(
        message.contains("no column builder for Arrow type") && message.contains("List"),
        "expected the cell-decoder gap for List, got: {message}"
    );

    drop_schema(&env).await;
}

/// `20260913-trino-incremental` phase 3, test 6: `insert_into_from_query`
/// appends the query's rows to an existing Iceberg table and leaves prior
/// rows intact — the insert-only append family's write primitive.
#[tokio::test]
async fn insert_into_from_query_appends_and_leaves_prior_rows_intact() {
    let Some(env) =
        live_env_or_skip("insert_into_from_query_appends_and_leaves_prior_rows_intact").await
    else {
        return;
    };

    env.backend
        .create_table_as(
            &env.schema,
            "smelt_p3_append",
            "SELECT * FROM (VALUES (1, 'a'), (2, 'b')) AS t(id, label)",
        )
        .await
        .expect("create_table_as must succeed");

    env.backend
        .insert_into_from_query(
            &env.schema,
            "smelt_p3_append",
            "SELECT * FROM (VALUES (3, 'c'), (4, 'd')) AS t(id, label)",
        )
        .await
        .expect("insert_into_from_query must succeed");

    let count = env
        .backend
        .get_row_count(&env.schema, "smelt_p3_append")
        .await
        .expect("get_row_count must succeed");
    assert_eq!(
        count, 4,
        "insert_into_from_query must append, leaving the original 2 rows plus the 2 new ones"
    );

    drop_schema(&env).await;
}

/// `20260913-trino-incremental` phase 3: the whole-row `MERGE` upsert family
/// — `Backend::merge_into`'s default implementation, routed through
/// `require_merge_columns` + `emit_column_scoped_merge`'s Trino spelling
/// (column-by-column `UPDATE SET`/`INSERT`, never `SET *`/`INSERT *`/`INSERT
/// ROW`) — matches a matched row, inserts an unmatched one, and is
/// idempotent by key across two runs, the second mutating one key's value.
#[tokio::test]
async fn merge_into_upserts_matched_and_unmatched_rows_across_two_runs() {
    let Some(env) =
        live_env_or_skip("merge_into_upserts_matched_and_unmatched_rows_across_two_runs").await
    else {
        return;
    };

    env.backend
        .create_table_as(
            &env.schema,
            "smelt_p3_merge",
            "SELECT * FROM (VALUES (1, 10), (2, 20)) AS t(id, attr)",
        )
        .await
        .expect("create_table_as must succeed");

    let columns = vec!["id".to_string(), "attr".to_string()];
    let unique_key = vec!["id".to_string()];

    // First MERGE: id=2 matches and updates, id=3 is unmatched and inserts.
    env.backend
        .merge_into(
            &env.schema,
            "smelt_p3_merge",
            "SELECT * FROM (VALUES (2, 99), (3, 30)) AS t(id, attr)",
            &unique_key,
            &columns,
        )
        .await
        .expect("first merge_into must succeed");

    let mut rows = fetch_id_attr_rows(&env, "smelt_p3_merge").await;
    rows.sort();
    assert_eq!(rows, vec![(1, 10), (2, 99), (3, 30)]);

    // Second MERGE: id=1 matches and updates, id=2/3 are untouched.
    env.backend
        .merge_into(
            &env.schema,
            "smelt_p3_merge",
            "SELECT * FROM (VALUES (1, 111)) AS t(id, attr)",
            &unique_key,
            &columns,
        )
        .await
        .expect("second merge_into must succeed");

    let mut rows = fetch_id_attr_rows(&env, "smelt_p3_merge").await;
    rows.sort();
    assert_eq!(
        rows,
        vec![(1, 111), (2, 99), (3, 30)],
        "the whole-row MERGE upsert must be idempotent by key across separate runs"
    );

    drop_schema(&env).await;
}

/// Reads an integer column back as `i64` regardless of whether Trino's
/// literal-type inference decoded it as Arrow `Int32` or `Int64`.
fn int_column_as_i64(batch: &RecordBatch, idx: usize) -> Vec<i64> {
    let column = batch.column(idx);
    if let Some(a) = column.as_any().downcast_ref::<Int64Array>() {
        return (0..a.len()).map(|i| a.value(i)).collect();
    }
    column
        .as_any()
        .downcast_ref::<Int32Array>()
        .unwrap_or_else(|| panic!("expected an Int32 or Int64 column at index {idx}"))
        .iter()
        .map(|v| v.unwrap_or_default() as i64)
        .collect()
}

/// `20260913-trino-incremental` phase 3: the insert-only append family —
/// `Backend::delete_and_insert_transactional`'s default implementation,
/// routed through `emit_delete_insert`'s dialect-invariant `Region`
/// text — run over two disjoint integer-axis windows, ending
/// multiset-equal to inserting the union directly. The integer axis is
/// deliberate: it needs no per-dialect literal typing, so it isolates this
/// family's own emitter/backend correctness from the separate, pre-existing
/// calendar-literal gap this phase's `trino_incremental_families.rs` test
/// documents (a bare quoted `'2026-01-01'` compared against a `DATE`/
/// `TIMESTAMP` column, implicitly coerced by DuckDB/Spark/BigQuery but
/// refused outright by Trino).
#[tokio::test]
async fn delete_and_insert_transactional_covers_two_disjoint_windows() {
    let Some(env) =
        live_env_or_skip("delete_and_insert_transactional_covers_two_disjoint_windows").await
    else {
        return;
    };

    env.backend
        .create_table_as(
            &env.schema,
            "smelt_p3_append",
            "SELECT * FROM (VALUES (CAST(0 AS INTEGER), CAST(0 AS INTEGER))) AS t(batch_id, val) WHERE 1=0",
        )
        .await
        .expect("create_table_as must succeed");

    env.backend
        .delete_and_insert_transactional(
            &env.schema,
            "smelt_p3_append",
            &PartitionRange {
                column: "batch_id".to_string(),
                start: "1".to_string(),
                end: "2".to_string(),
                axis: PartitionAxis::Integer,
                column_type: PartitionColumnType::Undeclared,
            },
            "SELECT * FROM (VALUES (1, 1), (1, 2)) AS t(batch_id, val)",
        )
        .await
        .expect("window 1's delete_and_insert_transactional must succeed");

    env.backend
        .delete_and_insert_transactional(
            &env.schema,
            "smelt_p3_append",
            &PartitionRange {
                column: "batch_id".to_string(),
                start: "2".to_string(),
                end: "3".to_string(),
                axis: PartitionAxis::Integer,
                column_type: PartitionColumnType::Undeclared,
            },
            "SELECT * FROM (VALUES (2, 3), (2, 4), (2, 5)) AS t(batch_id, val)",
        )
        .await
        .expect("window 2's delete_and_insert_transactional must succeed");

    let mut rows = fetch_two_int_columns(&env, "smelt_p3_append", "batch_id", "val").await;
    rows.sort();
    assert_eq!(
        rows,
        vec![(1, 1), (1, 2), (2, 3), (2, 4), (2, 5)],
        "two disjoint windows' DELETE+INSERT must together equal the union, with window 1's \
         rows untouched by window 2's write"
    );

    drop_schema(&env).await;
}

async fn fetch_two_int_columns(
    env: &LiveEnv,
    table: &str,
    col_a: &str,
    col_b: &str,
) -> Vec<(i64, i64)> {
    let batches = env
        .backend
        .execute_sql(&format!(
            "SELECT {col_a}, {col_b} FROM {}",
            qualified(&env.catalog, &env.schema, table)
        ))
        .await
        .expect("execute_sql must succeed");
    let mut rows = Vec::new();
    for batch in &batches {
        let a = int_column_as_i64(batch, 0);
        let b = int_column_as_i64(batch, 1);
        rows.extend(a.into_iter().zip(b));
    }
    rows
}

async fn fetch_id_attr_rows(env: &LiveEnv, table: &str) -> Vec<(i64, i64)> {
    fetch_two_int_columns(env, table, "id", "attr").await
}
