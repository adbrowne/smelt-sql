//! Shared multi-target test utilities for W1+ harness tests.
//!
//! DuckDB always runs; Spark runs only when `SPARK_CONNECT_URL` is set, and
//! BigQuery only when `SMELT_BQ_PROJECT` and `SMELT_BQ_ACCESS_TOKEN` are both
//! set. Tests that call `targets_to_run()` are automatically skipped on the
//! remote paths when no warehouse is provisioned — they still pass by covering
//! DuckDB only.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use tempfile::TempDir;

use arrow::array::{BooleanArray, Int32Array, StringArray};
use arrow::datatypes::{DataType, Field, Schema, SchemaRef};
use arrow::record_batch::RecordBatch;

/// Which execution target this harness is running against.
#[derive(Debug)]
pub enum TargetKind {
    DuckDb,
    Spark,
    /// BigQuery, carrying the suite-scoped dataset this run isolates in.
    ///
    /// The dataset doubles as the schema name, so a suite's match arm reads
    /// `TargetKind::BigQuery { dataset } => ("bq", dataset.as_str())`.
    BigQuery {
        dataset: String,
    },
}

/// Returns `SPARK_CONNECT_URL` from the environment, or `None` when absent.
pub fn spark_connect_url() -> Option<String> {
    std::env::var("SPARK_CONNECT_URL").ok()
}

/// The live BigQuery environment a suite runs against.
pub struct BqEnv {
    pub project: String,
    pub token: String,
    pub dataset_base: String,
    pub location: Option<String>,
}

/// The BigQuery environment, or `None` when the BigQuery leg should be skipped.
///
/// Both the project and a live token are required. `SMELT_BQ_PROJECT` alone
/// would let a suite *fail* with an auth error rather than skip, which is
/// exactly the failure mode the skip gate exists to prevent.
pub fn bq_env() -> Option<BqEnv> {
    Some(BqEnv {
        project: std::env::var("SMELT_BQ_PROJECT").ok()?,
        token: std::env::var("SMELT_BQ_ACCESS_TOKEN").ok()?,
        dataset_base: std::env::var("SMELT_BQ_DATASET")
            .unwrap_or_else(|_| "smelt_test".to_string()),
        location: std::env::var("SMELT_BQ_LOCATION").ok(),
    })
}

/// The dataset a suite labelled `label` isolates in.
///
/// Derived rather than minted, so staging (which writes the `bq:` target block)
/// and the assertion loop (which reads back through `TargetKind::BigQuery`)
/// agree without threading state between them. The pid keeps two concurrent
/// runs — two worktrees, or a developer beside an autonomy loop — apart; the
/// label keeps two suites in the same binary apart.
pub fn bq_dataset(label: &str) -> String {
    let base = bq_env()
        .map(|e| e.dataset_base)
        .unwrap_or_else(|| "smelt_test".to_string());
    let label: String = label
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    format!("{base}_{label}_{}", std::process::id())
}

/// True when this binary can actually drive BigQuery: compiled with the feature
/// AND pointed at a live project with a live token.
pub fn bigquery_enabled() -> bool {
    cfg!(feature = "bigquery") && bq_env().is_some()
}

/// Returns the targets to run this iteration.
///
/// Always includes `DuckDb`. Appends `Spark` only when BOTH hold:
/// - `SPARK_CONNECT_URL` is set (a live Connect server is reachable), AND
/// - the test binary was compiled with `--features spark` (so the `smelt`
///   binary also has Spark support).
///
/// Appends `BigQuery` under the same rule: `--features bigquery` plus a live
/// `SMELT_BQ_PROJECT`/`SMELT_BQ_ACCESS_TOKEN` pair.
///
/// This ensures `cargo test --quiet` (the default, no remote features) is always
/// green even when those variables are present in the environment.
///
/// `label` names the suite; it scopes the BigQuery dataset so two suites in one
/// binary never write the same table. Pass the same label to
/// [`bq_target_block`] when staging the workspace.
pub fn targets_to_run(label: &str) -> Vec<TargetKind> {
    let mut targets = vec![TargetKind::DuckDb];
    if cfg!(feature = "spark") && spark_connect_url().is_some() {
        targets.push(TargetKind::Spark);
    }
    if bigquery_enabled() {
        targets.push(TargetKind::BigQuery {
            dataset: bq_dataset(label),
        });
    }
    targets
}

/// The body of a `bigquery` target block (4-space indented, no leading key).
fn bq_target_body(dataset: &str) -> String {
    let env = bq_env().expect("bq_target_body called without a BigQuery environment");
    let location = env
        .location
        .map(|l| format!("\n    location: {l}"))
        .unwrap_or_default();
    format!(
        "type: bigquery\n    project: {}\n    dataset: {dataset}\n    schema: {dataset}{location}",
        env.project
    )
}

/// The whole `bq:` target entry for `label`, ready to concatenate under
/// `targets:` in a hand-written `smelt.yml`.
///
/// Empty when BigQuery is not enabled, so a suite can append it unconditionally
/// — an empty string leaves the default DuckDB-only workspace untouched, and a
/// target the CLI could not parse is never written.
pub fn bq_target_block(label: &str) -> String {
    if !bigquery_enabled() {
        return String::new();
    }
    format!("  bq:\n    {}\n", bq_target_body(&bq_dataset(label)))
}

/// Drops the suite's BigQuery dataset, if this kind is the BigQuery one.
///
/// A no-op for every other target. Datasets also carry the default table
/// expiration `scripts/bigquery-env.sh` exports, so an interrupted run still
/// sheds its tables — this makes the clean path immediate rather than delayed.
pub fn drop_bq_dataset(target: &TargetKind) {
    let TargetKind::BigQuery { dataset } = target else {
        return;
    };
    let _ = dataset;
    #[cfg(feature = "bigquery")]
    {
        use smelt_backend::Backend;
        let env = bq_env().expect("BigQuery env must be set to drop a dataset");
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime for drop_bq_dataset");
        rt.block_on(async {
            let backend = bq_backend(dataset).await;
            let _ = backend
                .execute_sql(&format!(
                    "DROP SCHEMA IF EXISTS `{}.{dataset}` CASCADE",
                    env.project
                ))
                .await;
        });
    }
}

/// Connects a `BigQueryBackend` to `dataset` from the ambient test environment.
///
/// Public so a suite that drives the `Backend` trait directly — rather than
/// going through `smelt run` — can reach the same connection the harness uses.
#[cfg(feature = "bigquery")]
pub async fn bq_backend(dataset: &str) -> smelt_backend_bigquery::BigQueryBackend {
    let env = bq_env().expect("BigQuery env must be set to connect");
    smelt_backend_bigquery::BigQueryBackend::new(
        &env.project,
        dataset,
        env.location.as_deref(),
        &env.token,
    )
    .await
    .unwrap_or_else(|e| panic!("BigQuery connect failed: {e}"))
}

/// Returns `(target_name, yaml_block)` for a single target kind.
///
/// `yaml_block` is the indented YAML content that goes under the target name
/// key (4-space indent — ready to embed inside `targets:` in smelt.yml).
pub fn targets_yaml(kind: &TargetKind, warehouse_dir: &Path) -> (String, String) {
    match kind {
        TargetKind::DuckDb => (
            "dev".to_string(),
            "type: duckdb\n    database: target/dev.duckdb\n    schema: main".to_string(),
        ),
        TargetKind::Spark => {
            let url = spark_connect_url().unwrap_or_else(|| "sc://localhost:15002".to_string());
            let warehouse = warehouse_dir
                .to_str()
                .expect("warehouse path must be valid UTF-8");
            (
                "spark".to_string(),
                format!(
                    "type: spark\n    connect_url: {url}\n    \
                     catalog: spark_catalog\n    schema: smelt_w1\n    \
                     warehouse: {warehouse}\n    format: delta"
                ),
            )
        }
        TargetKind::BigQuery { dataset } => ("bq".to_string(), bq_target_body(dataset)),
    }
}

/// Stages a smelt workspace with a `dev` (DuckDB), a `spark`, and — when the
/// BigQuery environment is live — a `bq` target.
///
/// The `smelt.yml` always includes the DuckDB and Spark blocks so a test can run
/// `--target spark` when Spark is up. `targets_to_run(name)` controls which
/// targets are actually exercised in the loop; pass the same `name` there, since
/// it scopes the BigQuery dataset.
pub fn stage_dual_workspace(
    tmp: &TempDir,
    name: &str,
    models: &[(&str, &str)],
    warehouse_dir: &Path,
) -> PathBuf {
    let root = tmp.path().join(name);
    std::fs::create_dir_all(root.join("models")).unwrap();
    std::fs::create_dir_all(root.join("target")).unwrap();
    std::fs::create_dir_all(warehouse_dir).unwrap();

    let (dev_name, dev_yaml) = targets_yaml(&TargetKind::DuckDb, warehouse_dir);
    let (spark_name, spark_yaml) = targets_yaml(&TargetKind::Spark, warehouse_dir);
    let bq_block = bq_target_block(name);

    let yml = format!(
        "name: {name}\n\
         version: 1\n\
         paths:\n  - models\n\
         targets:\n  {dev_name}:\n    {dev_yaml}\n  {spark_name}:\n    {spark_yaml}\n{bq_block}\
         default_materialization: view\n"
    );
    std::fs::write(root.join("smelt.yml"), yml).unwrap();

    for (file, content) in models {
        std::fs::write(root.join("models").join(file), content).unwrap();
    }
    root
}

/// Invokes `smelt run --project-dir <dir> --target <target_name>` and returns
/// the raw output. The caller asserts `out.status.success()` or inspects stderr.
pub fn run_smelt_on(
    project_dir: &Path,
    target_name: &str,
    extra_args: &[&str],
) -> std::process::Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_smelt"));
    cmd.args([
        "run",
        "--project-dir",
        project_dir.to_str().unwrap(),
        "--target",
        target_name,
    ])
    .args(extra_args)
    .env_remove("RUST_LOG");
    cmd.output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt run`: {e}"))
}

// ─── W3·P1: Source-seeding helpers ──────────────────────────────────────────

/// Arrow schema matching `examples/multi_engine/models/sources/raw/sessions.yml`.
///
/// Column order and types must match what `staging/stg_sessions.sql` reads.
pub fn sessions_arrow_schema() -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new("visitor_id", DataType::Utf8, true),
        Field::new("session_id", DataType::Utf8, true),
        Field::new("session_date", DataType::Utf8, true),
        Field::new("page_views", DataType::Int32, true),
        Field::new("revenue_cents", DataType::Int32, true),
        Field::new("country", DataType::Utf8, true),
        Field::new("traffic_source", DataType::Utf8, true),
        Field::new("device_type", DataType::Utf8, true),
        Field::new("is_converted", DataType::Boolean, true),
    ]))
}

/// A small deterministic `RecordBatch` of sessions data (3 rows).
///
/// Every test that calls `seed_source_table` uses this batch, so assertions
/// on row count always expect exactly 3.
pub fn sessions_record_batch() -> RecordBatch {
    let schema = sessions_arrow_schema();
    RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(vec!["v1", "v2", "v3"])),
            Arc::new(StringArray::from(vec!["s1", "s2", "s3"])),
            Arc::new(StringArray::from(vec![
                "2024-01-01",
                "2024-01-01",
                "2024-01-02",
            ])),
            Arc::new(Int32Array::from(vec![3, 5, 2])),
            Arc::new(Int32Array::from(vec![100, 0, 250])),
            Arc::new(StringArray::from(vec!["US", "GB", "AU"])),
            Arc::new(StringArray::from(vec!["organic", "paid", "email"])),
            Arc::new(StringArray::from(vec!["mobile", "desktop", "tablet"])),
            Arc::new(BooleanArray::from(vec![true, false, true])),
        ],
    )
    .expect("sessions_record_batch: schema/data mismatch")
}

/// Splits `schema.table` into `(schema, table)`.
fn split_fqn(fqn: &str) -> (&str, &str) {
    let dot = fqn.rfind('.').expect("table_fqn must contain '.'");
    (&fqn[..dot], &fqn[dot + 1..])
}

/// Materializes `table_fqn` (e.g. `analytics.sources_raw_sessions`) into the
/// given target from the supplied Arrow data.
///
/// - **DuckDb**: opens `db_path` and calls `load_table`. `_warehouse` is unused.
/// - **Spark**: reads `SPARK_CONNECT_URL` from the environment; uses `_warehouse`
///   as the Delta warehouse root. Requires `--features spark`; panics otherwise.
///
/// Panics if the backend returns an error.
pub fn seed_source_table(
    target: &TargetKind,
    db_path: &Path,
    _warehouse: &Path,
    table_fqn: &str,
    arrow_schema: SchemaRef,
    batch: RecordBatch,
) {
    let (schema, table) = split_fqn(table_fqn);
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime for seed_source_table");

    match target {
        TargetKind::DuckDb => {
            #[cfg(not(feature = "duckdb"))]
            panic!("DuckDB seeding requires --features duckdb");
            #[cfg(feature = "duckdb")]
            {
                use smelt_backend::Backend;
                use smelt_backend_duckdb::DuckDbBackend;
                rt.block_on(async {
                    let backend = DuckDbBackend::new(db_path, schema)
                        .await
                        .unwrap_or_else(|e| panic!("DuckDB open failed for seed: {e}"));
                    backend
                        .load_table(schema, table, arrow_schema, vec![batch])
                        .await
                        .unwrap_or_else(|e| panic!("DuckDB load_table failed: {e}"));
                });
            }
        }
        TargetKind::Spark => {
            #[cfg(not(feature = "spark"))]
            panic!("Spark seeding requires --features spark");
            #[cfg(feature = "spark")]
            {
                use smelt_backend::Backend;
                use smelt_backend_spark::SparkBackend;
                let url =
                    spark_connect_url().expect("SPARK_CONNECT_URL must be set for Spark seeding");
                let wh = _warehouse
                    .to_str()
                    .expect("warehouse path must be valid UTF-8");
                rt.block_on(async {
                    let backend = SparkBackend::new(&url, "spark_catalog", schema, Some(wh), true)
                        .await
                        .unwrap_or_else(|e| panic!("Spark connect failed for seed: {e}"));
                    backend
                        .load_table(schema, table, arrow_schema, vec![batch])
                        .await
                        .unwrap_or_else(|e| panic!("Spark load_table failed: {e}"));
                });
            }
        }
        TargetKind::BigQuery { dataset } => {
            let _ = dataset;
            #[cfg(not(feature = "bigquery"))]
            panic!("BigQuery seeding requires --features bigquery");
            #[cfg(feature = "bigquery")]
            {
                use smelt_backend::Backend;
                rt.block_on(async {
                    let backend = bq_backend(dataset).await;
                    backend
                        .load_table(schema, table, arrow_schema, vec![batch])
                        .await
                        .unwrap_or_else(|e| panic!("BigQuery load_table failed: {e}"));
                });
            }
        }
    }
}

/// Returns the row count of `schema.table` in the given target.
///
/// Uses the same connection parameters as `seed_source_table`.
pub fn count_table_rows(
    target: &TargetKind,
    db_path: &Path,
    _warehouse: &Path,
    schema: &str,
    table: &str,
) -> usize {
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime for count_table_rows");

    match target {
        TargetKind::DuckDb => {
            #[cfg(not(feature = "duckdb"))]
            panic!("DuckDB count requires --features duckdb");
            #[cfg(feature = "duckdb")]
            {
                use smelt_backend::Backend;
                use smelt_backend_duckdb::DuckDbBackend;
                rt.block_on(async {
                    let backend = DuckDbBackend::new(db_path, schema)
                        .await
                        .unwrap_or_else(|e| panic!("DuckDB open failed for count: {e}"));
                    backend
                        .get_row_count(schema, table)
                        .await
                        .unwrap_or_else(|e| panic!("DuckDB get_row_count failed: {e}"))
                })
            }
        }
        TargetKind::Spark => {
            #[cfg(not(feature = "spark"))]
            panic!("Spark count requires --features spark");
            #[cfg(feature = "spark")]
            {
                use smelt_backend::Backend;
                use smelt_backend_spark::SparkBackend;
                let url =
                    spark_connect_url().expect("SPARK_CONNECT_URL must be set for Spark count");
                let wh = _warehouse
                    .to_str()
                    .expect("warehouse path must be valid UTF-8");
                rt.block_on(async {
                    let backend = SparkBackend::new(&url, "spark_catalog", schema, Some(wh), true)
                        .await
                        .unwrap_or_else(|e| panic!("Spark connect failed for count: {e}"));
                    backend
                        .get_row_count(schema, table)
                        .await
                        .unwrap_or_else(|e| panic!("Spark get_row_count failed: {e}"))
                })
            }
        }
        TargetKind::BigQuery { dataset } => {
            let _ = dataset;
            #[cfg(not(feature = "bigquery"))]
            panic!("BigQuery count requires --features bigquery");
            #[cfg(feature = "bigquery")]
            {
                use smelt_backend::Backend;
                rt.block_on(async {
                    let backend = bq_backend(dataset).await;
                    backend
                        .get_row_count(schema, table)
                        .await
                        .unwrap_or_else(|e| panic!("BigQuery get_row_count failed: {e}"))
                })
            }
        }
    }
}

// ─── W5·P1: Result-parity helpers ───────────────────────────────────────────

/// Execute a SQL query on the given target and return the raw Arrow batches.
fn execute_sql_on(
    target: &TargetKind,
    db_path: &Path,
    _warehouse: &Path,
    schema: &str,
    sql: &str,
) -> Vec<RecordBatch> {
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime for execute_sql_on");
    match target {
        TargetKind::DuckDb => {
            #[cfg(not(feature = "duckdb"))]
            panic!("DuckDB SQL requires --features duckdb");
            #[cfg(feature = "duckdb")]
            {
                use smelt_backend::Backend;
                use smelt_backend_duckdb::DuckDbBackend;
                rt.block_on(async {
                    let backend = DuckDbBackend::new(db_path, schema)
                        .await
                        .unwrap_or_else(|e| panic!("DuckDB open for execute_sql_on: {e}"));
                    backend
                        .execute_sql(sql)
                        .await
                        .unwrap_or_else(|e| panic!("DuckDB execute_sql failed: {e}"))
                })
            }
        }
        TargetKind::Spark => {
            #[cfg(not(feature = "spark"))]
            panic!("Spark SQL requires --features spark");
            #[cfg(feature = "spark")]
            {
                use smelt_backend::Backend;
                use smelt_backend_spark::SparkBackend;
                let url = spark_connect_url().expect("SPARK_CONNECT_URL must be set for Spark SQL");
                let wh = _warehouse
                    .to_str()
                    .expect("warehouse path must be valid UTF-8");
                rt.block_on(async {
                    let backend = SparkBackend::new(&url, "spark_catalog", schema, Some(wh), true)
                        .await
                        .unwrap_or_else(|e| panic!("Spark connect for execute_sql_on: {e}"));
                    backend
                        .execute_sql(sql)
                        .await
                        .unwrap_or_else(|e| panic!("Spark execute_sql failed: {e}"))
                })
            }
        }
        TargetKind::BigQuery { dataset } => {
            let _ = dataset;
            #[cfg(not(feature = "bigquery"))]
            panic!("BigQuery SQL requires --features bigquery");
            #[cfg(feature = "bigquery")]
            {
                use smelt_backend::Backend;
                rt.block_on(async {
                    let backend = bq_backend(dataset).await;
                    backend
                        .execute_sql(sql)
                        .await
                        .unwrap_or_else(|e| panic!("BigQuery execute_sql failed: {e}"))
                })
            }
        }
    }
}

/// Convert `RecordBatch` results to a sorted list of string rows.
///
/// Each cell is stringified via `arrow::util::display::array_value_to_string`.
/// Rows are sorted lexicographically so cross-backend comparisons are order-independent.
fn batches_to_sorted_rows(batches: &[RecordBatch]) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    for batch in batches {
        for row_idx in 0..batch.num_rows() {
            let mut row = Vec::new();
            for col_idx in 0..batch.num_columns() {
                let col = batch.column(col_idx);
                let val = arrow::util::display::array_value_to_string(col, row_idx)
                    .unwrap_or_else(|_| "NULL".to_string());
                row.push(val);
            }
            rows.push(row);
        }
    }
    rows.sort();
    rows
}

/// Fetch all rows from `schema.table` on the given target, normalized and sorted.
///
/// This is the reusable result-parity helper that W5 phases use to compare
/// query results across backends.  DuckDb opens `db_path`; Spark connects via
/// `SPARK_CONNECT_URL` with `warehouse` as the Delta root.
pub fn fetch_rows(
    target: &TargetKind,
    db_path: &Path,
    warehouse: &Path,
    schema: &str,
    table: &str,
) -> Vec<Vec<String>> {
    let sql = format!("SELECT * FROM {schema}.{table}");
    let batches = execute_sql_on(target, db_path, warehouse, schema, &sql);
    batches_to_sorted_rows(&batches)
}

/// Assert that `actual` rows match `expected` rows (both sorted, all values as strings).
///
/// `label` names the target in the failure message.
pub fn assert_table_parity(actual: &[Vec<String>], expected: &[Vec<String>], label: &str) {
    let mut act = actual.to_vec();
    let mut exp = expected.to_vec();
    act.sort();
    exp.sort();
    assert_eq!(
        act, exp,
        "{label}: table rows mismatch.\n  expected: {exp:#?}\n  actual:   {act:#?}"
    );
}
