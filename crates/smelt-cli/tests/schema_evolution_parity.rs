//! W5·P5 — schema-evolution (add-column) parity across backends.
//!
//! Exercises `ALTER TABLE ... ADD COLUMN` (DuckDB) and `ADD COLUMNS` (Spark)
//! on both backends.  Materialises a v1 model, evolves the schema by adding a
//! nullable column, inserts new rows with the column populated, and asserts
//! the migrated table has the same logical rows on both DuckDB and Spark.
//!
//! The old rows (pre-migration) have NULL for the new column; the new rows
//! have it populated.  Both backends are compared row-for-row via
//! `assert_table_parity`.
//!
//! With `SPARK_CONNECT_URL` unset: DuckDB only (Spark path skips green).
//! With `SPARK_CONNECT_URL` set AND `--features spark`: also covers Spark.
//! With `SMELT_BQ_PROJECT`/`SMELT_BQ_ACCESS_TOKEN` set AND `--features
//! bigquery`: also covers BigQuery. This exercises the warehouse's own
//! `ADD COLUMN`, which is a separate question from smelt emitting evolution
//! DDL for BigQuery — that path resolves to a full refresh instead.

mod common;
#[cfg(feature = "spark")]
use common::spark_connect_url;
use common::{drop_bq_dataset, fetch_rows, targets_to_run, TargetKind};
use tempfile::TempDir;

const SPARK_SCHEMA: &str = "smelt_evo_p5";

/// Scopes this suite's BigQuery dataset, as `SPARK_SCHEMA` does for Spark.
const BQ_LABEL: &str = "evo_p5";

/// Create a small source table (3 rows, 2 days).
fn create_source_sql(schema: &str) -> String {
    format!(
        "CREATE TABLE IF NOT EXISTS {schema}.evt_source AS \
         SELECT '2024-01-01' AS event_date, 'A' AS user_id, CAST(100 AS BIGINT) AS amount \
         UNION ALL SELECT '2024-01-01', 'B', CAST(200 AS BIGINT) \
         UNION ALL SELECT '2024-01-02', 'A', CAST(150 AS BIGINT)"
    )
}

/// v1 model: aggregate day-1 rows only; no `event_count` column yet.
fn v1_model_sql(schema: &str) -> String {
    format!(
        "SELECT event_date, user_id, CAST(SUM(amount) AS BIGINT) AS total_amount \
         FROM {schema}.evt_source \
         WHERE event_date = '2024-01-01' \
         GROUP BY event_date, user_id"
    )
}

/// Dialect-specific `ALTER TABLE … ADD COLUMN` DDL.
///
/// DuckDB:   `ALTER TABLE schema.table ADD COLUMN col TYPE`
/// Spark:    `ALTER TABLE catalog.schema.table ADD COLUMNS (col TYPE)`
/// BigQuery: `ALTER TABLE dataset.table ADD COLUMN col INT64` — GoogleSQL
///           spells the 64-bit integer `INT64`, and takes one column per
///           `ADD COLUMN` rather than Spark's parenthesised list.
fn add_column_ddl(kind: &TargetKind, schema: &str) -> String {
    match kind {
        TargetKind::DuckDb => {
            format!("ALTER TABLE {schema}.event_totals ADD COLUMN event_count BIGINT")
        }
        TargetKind::Spark => {
            format!(
                "ALTER TABLE spark_catalog.{schema}.event_totals ADD COLUMNS (event_count BIGINT)"
            )
        }
        TargetKind::BigQuery { dataset } => {
            format!("ALTER TABLE {dataset}.event_totals ADD COLUMN event_count INT64")
        }
    }
}

/// v2 incremental insert: day-2 rows with `event_count` populated.
fn v2_insert_sql(schema: &str) -> String {
    format!(
        "SELECT event_date, user_id, \
                CAST(SUM(amount) AS BIGINT) AS total_amount, \
                CAST(COUNT(*) AS BIGINT) AS event_count \
         FROM {schema}.evt_source \
         WHERE event_date = '2024-01-02' \
         GROUP BY event_date, user_id"
    )
}

/// Materialise a v1 model, add a nullable column, insert new rows with the
/// column populated, and assert the final state matches across backends.
///
/// Expected final table (3 rows):
///   event_date='2024-01-01', user_id='A', total_amount=100, event_count=NULL
///   event_date='2024-01-01', user_id='B', total_amount=200, event_count=NULL
///   event_date='2024-01-02', user_id='A', total_amount=150, event_count=1
///
/// **Red on Spark** if the `ADD COLUMNS` DDL or the subsequent INSERT is
/// rejected by the live server; fix `ddl_spark.rs` or the backend → green.
/// **DuckDB always green; Spark skips when `SPARK_CONNECT_URL` is unset.**
#[cfg(feature = "duckdb")]
#[test]
fn add_column_migration_matches_across_backends() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test.duckdb");
    let warehouse = tmp.path().join("warehouse");
    std::fs::create_dir_all(&warehouse).unwrap();
    let rt = tokio::runtime::Runtime::new().unwrap();

    // Reference rows collected from DuckDB; Spark result is compared against them.
    let mut reference_rows: Vec<Vec<String>> = Vec::new();

    for kind in targets_to_run(BQ_LABEL) {
        let schema = match &kind {
            TargetKind::DuckDb => "main",
            TargetKind::Spark => SPARK_SCHEMA,
            TargetKind::BigQuery { dataset } => dataset.as_str(),
        };

        match &kind {
            TargetKind::DuckDb => {
                #[cfg(feature = "duckdb")]
                rt.block_on(async {
                    use smelt_backend::Backend;
                    use smelt_backend_duckdb::DuckDbBackend;

                    let backend = DuckDbBackend::new(&db_path, schema)
                        .await
                        .unwrap_or_else(|e| panic!("DuckDB open failed: {e}"));

                    // Seed source table.
                    backend
                        .drop_table_if_exists(schema, "evt_source")
                        .await
                        .unwrap();
                    backend
                        .execute_sql(&create_source_sql(schema))
                        .await
                        .unwrap_or_else(|e| panic!("DuckDB create source failed: {e}"));

                    // Create v1 event_totals (day-1 rows only, no event_count).
                    backend
                        .drop_table_if_exists(schema, "event_totals")
                        .await
                        .unwrap();
                    backend
                        .create_table_as(schema, "event_totals", &v1_model_sql(schema))
                        .await
                        .unwrap_or_else(|e| panic!("DuckDB create_table_as failed: {e}"));

                    // Evolve schema: add nullable event_count column.
                    backend
                        .execute_sql(&add_column_ddl(&kind, schema))
                        .await
                        .unwrap_or_else(|e| panic!("DuckDB ADD COLUMN failed: {e}"));

                    // Incremental insert: day-2 rows WITH event_count.
                    backend
                        .insert_into_from_query(schema, "event_totals", &v2_insert_sql(schema))
                        .await
                        .unwrap_or_else(|e| panic!("DuckDB insert v2 rows failed: {e}"));
                });
            }
            TargetKind::Spark => {
                // Only reachable when compiled with --features spark AND SPARK_CONNECT_URL is set.
                #[cfg(not(feature = "spark"))]
                panic!("Spark path should only be reached when --features spark is enabled");
                #[cfg(feature = "spark")]
                {
                    use smelt_backend::Backend;
                    use smelt_backend_spark::SparkBackend;

                    let url = spark_connect_url()
                        .expect("SPARK_CONNECT_URL must be set to reach Spark target");
                    let wh = warehouse
                        .to_str()
                        .expect("warehouse path must be valid UTF-8");

                    rt.block_on(async {
                        let backend =
                            SparkBackend::new(&url, "spark_catalog", schema, Some(wh), true)
                                .await
                                .unwrap_or_else(|e| panic!("Spark connect failed: {e}"));

                        // Seed source table.
                        backend
                            .drop_table_if_exists(schema, "evt_source")
                            .await
                            .unwrap();
                        backend
                            .execute_sql(&create_source_sql(schema))
                            .await
                            .unwrap_or_else(|e| panic!("Spark create source failed: {e}"));

                        // Create v1 event_totals.
                        backend
                            .drop_table_if_exists(schema, "event_totals")
                            .await
                            .unwrap();
                        backend
                            .create_table_as(schema, "event_totals", &v1_model_sql(schema))
                            .await
                            .unwrap_or_else(|e| panic!("Spark create_table_as failed: {e}"));

                        // Evolve schema: ADD COLUMNS (Spark DDL from ddl_spark.rs).
                        // RED on Spark if the ADD COLUMNS statement is rejected.
                        backend
                            .execute_sql(&add_column_ddl(&kind, schema))
                            .await
                            .unwrap_or_else(|e| panic!("Spark ADD COLUMNS failed: {e}"));

                        // Incremental insert: day-2 rows WITH event_count.
                        backend
                            .insert_into_from_query(schema, "event_totals", &v2_insert_sql(schema))
                            .await
                            .unwrap_or_else(|e| panic!("Spark insert v2 rows failed: {e}"));
                    });
                }
            }
            TargetKind::BigQuery { dataset } => {
                let _ = dataset;
                #[cfg(not(feature = "bigquery"))]
                panic!("BigQuery path should only be reached when --features bigquery is enabled");
                #[cfg(feature = "bigquery")]
                rt.block_on(async {
                    use smelt_backend::Backend;

                    let backend = common::bq_backend(dataset).await;

                    // Seed source table.
                    backend
                        .drop_table_if_exists(schema, "evt_source")
                        .await
                        .unwrap();
                    backend
                        .execute_sql(&create_source_sql(schema))
                        .await
                        .unwrap_or_else(|e| panic!("BigQuery create source failed: {e}"));

                    // Create v1 event_totals.
                    backend
                        .drop_table_if_exists(schema, "event_totals")
                        .await
                        .unwrap();
                    backend
                        .create_table_as(schema, "event_totals", &v1_model_sql(schema))
                        .await
                        .unwrap_or_else(|e| panic!("BigQuery create_table_as failed: {e}"));

                    // Evolve schema: ADD COLUMN (GoogleSQL spelling).
                    backend
                        .execute_sql(&add_column_ddl(&kind, schema))
                        .await
                        .unwrap_or_else(|e| panic!("BigQuery ADD COLUMN failed: {e}"));

                    // Incremental insert: day-2 rows WITH event_count.
                    backend
                        .insert_into_from_query(schema, "event_totals", &v2_insert_sql(schema))
                        .await
                        .unwrap_or_else(|e| panic!("BigQuery insert v2 rows failed: {e}"));
                });
            }
        }

        // Fetch final rows and compare.  DuckDB establishes the reference;
        // each subsequent target (Spark, BigQuery) must match it.
        let actual = fetch_rows(&kind, &db_path, &warehouse, schema, "event_totals");
        drop_bq_dataset(&kind);
        if reference_rows.is_empty() {
            // First target sets the reference.
            assert_eq!(actual.len(), 3, "{kind:?}: expected 3 rows after migration");
            reference_rows = actual;
        } else {
            assert_eq!(
                actual, reference_rows,
                "{kind:?} rows differ from DuckDB reference.\n  expected: {reference_rows:#?}\n  actual: {actual:#?}"
            );
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────
// smelt's own migration DDL, executed on the warehouse
//
// The test above evolves the schema with hand-written DDL, so it measures the
// *warehouse* — that `ALTER TABLE … ADD COLUMN` behaves the same everywhere.
// The tests below instead take the statements `plan_migration_for_backend`
// emits and run those, so a generator that emits SQL its own target rejects
// fails here rather than in a user's run. That is the only assertion that can
// catch it: the generators are pure functions, and a unit test can only pin
// the string, never that GoogleSQL accepts it.
//
// DuckDB and BigQuery only. Spark's generator answers with strategies beyond
// statements (`MergeSchemaWrite`, `TableRewrite`) that need a different
// assertion shape; its equivalent lives in
// `crates/smelt-backend-spark/tests/ddl_observed.rs`.

use smelt_state::schema_tracking::{
    diff_schemas, plan_migration_for_backend, DdlBackend, DeployedColumn, MigrationAction,
};
use std::collections::HashMap;

fn deployed_col(name: &str, data_type: &str, nullable: bool) -> DeployedColumn {
    DeployedColumn {
        name: name.to_string(),
        data_type: data_type.to_string(),
        nullable,
    }
}

/// The DDL generator for a target, as `execute.rs` selects it at run time.
fn ddl_backend_for(kind: &TargetKind) -> DdlBackend {
    match kind {
        TargetKind::DuckDb => DdlBackend::DuckDb,
        TargetKind::BigQuery { .. } => DdlBackend::BigQuery,
        TargetKind::Spark => unreachable!("Spark is filtered out of these tests"),
    }
}

/// Plan the migration and return the statements, failing loudly on a refusal.
fn planned_statements(
    kind: &TargetKind,
    schema: &str,
    table: &str,
    deployed: &[DeployedColumn],
    inferred: &[DeployedColumn],
) -> Vec<String> {
    let diff = diff_schemas(deployed, inferred);
    assert!(
        !diff.is_empty(),
        "{kind:?}: the fixture must produce a diff"
    );
    let action = plan_migration_for_backend(
        schema,
        table,
        &diff,
        false,
        &HashMap::new(),
        &HashMap::new(),
        &ddl_backend_for(kind),
        deployed,
        inferred,
    );
    match action {
        MigrationAction::AlterTable { statements } => statements,
        other => panic!(
            "{kind:?}: expected ALTER TABLE DDL for this change, got {other:?}. \
             A backend that cannot express it falls back to a full refresh — \
             which is safe, but means this migration is not supported there."
        ),
    }
}

/// Targets whose DDL generator emits plain statement lists.
fn statement_ddl_targets(label: &str) -> Vec<TargetKind> {
    targets_to_run(label)
        .into_iter()
        .filter(|k| !matches!(k, TargetKind::Spark))
        .collect()
}

/// Run `statements` against the target, failing with the statement text.
#[allow(unused_variables)]
fn execute_all(
    rt: &tokio::runtime::Runtime,
    kind: &TargetKind,
    db_path: &std::path::Path,
    schema: &str,
    statements: &[String],
) {
    match kind {
        TargetKind::DuckDb => {
            #[cfg(feature = "duckdb")]
            rt.block_on(async {
                use smelt_backend::Backend;
                use smelt_backend_duckdb::DuckDbBackend;
                let backend = DuckDbBackend::new(db_path, schema).await.unwrap();
                for stmt in statements {
                    backend
                        .execute_sql(stmt)
                        .await
                        .unwrap_or_else(|e| panic!("DuckDB rejected its own migration DDL.\n  statement: {stmt}\n  error: {e}"));
                }
            });
        }
        TargetKind::BigQuery { dataset } => {
            #[cfg(not(feature = "bigquery"))]
            panic!("BigQuery path should only be reached when --features bigquery is enabled");
            #[cfg(feature = "bigquery")]
            rt.block_on(async {
                use smelt_backend::Backend;
                let backend = common::bq_backend(dataset).await;
                for stmt in statements {
                    backend
                        .execute_sql(stmt)
                        .await
                        .unwrap_or_else(|e| panic!("BigQuery rejected smelt's migration DDL.\n  statement: {stmt}\n  error: {e}"));
                }
            });
        }
        TargetKind::Spark => unreachable!("Spark is filtered out of these tests"),
    }
}

/// Create a table from a SELECT on the target.
#[allow(unused_variables)]
fn create_table(
    rt: &tokio::runtime::Runtime,
    kind: &TargetKind,
    db_path: &std::path::Path,
    schema: &str,
    table: &str,
    query: &str,
) {
    match kind {
        TargetKind::DuckDb => {
            #[cfg(feature = "duckdb")]
            rt.block_on(async {
                use smelt_backend::Backend;
                use smelt_backend_duckdb::DuckDbBackend;
                let backend = DuckDbBackend::new(db_path, schema).await.unwrap();
                backend.drop_table_if_exists(schema, table).await.unwrap();
                backend
                    .create_table_as(schema, table, query)
                    .await
                    .unwrap_or_else(|e| panic!("DuckDB create_table_as failed: {e}"));
            });
        }
        TargetKind::BigQuery { dataset } => {
            #[cfg(not(feature = "bigquery"))]
            panic!("BigQuery path should only be reached when --features bigquery is enabled");
            #[cfg(feature = "bigquery")]
            rt.block_on(async {
                use smelt_backend::Backend;
                let backend = common::bq_backend(dataset).await;
                backend.drop_table_if_exists(schema, table).await.unwrap();
                backend
                    .create_table_as(schema, table, query)
                    .await
                    .unwrap_or_else(|e| panic!("BigQuery create_table_as failed: {e}"));
            });
        }
        TargetKind::Spark => unreachable!("Spark is filtered out of these tests"),
    }
}

/// A generated `ADD COLUMN` migration must be SQL its own target executes.
///
/// **Red on BigQuery** if `ddl_bigquery` emits a DuckDB type name (`BIGINT`
/// is an accepted alias, but `VARCHAR`/`DOUBLE`/`TEXT` are `Type not found`)
/// or a form GoogleSQL spells differently.
#[cfg(feature = "duckdb")]
#[test]
fn generated_add_column_ddl_executes_on_every_backend() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test.duckdb");
    let warehouse = tmp.path().join("warehouse");
    std::fs::create_dir_all(&warehouse).unwrap();
    let rt = tokio::runtime::Runtime::new().unwrap();

    const TABLE: &str = "evo_gen_add";
    let mut reference_rows: Vec<Vec<String>> = Vec::new();

    for kind in statement_ddl_targets("evo_gen_add") {
        let schema = match &kind {
            TargetKind::DuckDb => "main",
            TargetKind::BigQuery { dataset } => dataset.as_str(),
            TargetKind::Spark => unreachable!(),
        };

        // v1: two columns.
        create_table(
            &rt,
            &kind,
            &db_path,
            schema,
            TABLE,
            "SELECT CAST(1 AS BIGINT) AS id, 'a' AS label",
        );

        // v2 adds a nullable column of each family whose DuckDB type name
        // GoogleSQL rejects outright, so a shared generator cannot pass.
        let deployed = vec![
            deployed_col("id", "BIGINT", true),
            deployed_col("label", "VARCHAR", true),
        ];
        let mut inferred = deployed.clone();
        inferred.push(deployed_col("note", "VARCHAR", true));
        inferred.push(deployed_col("ratio", "DOUBLE", true));
        inferred.push(deployed_col("n", "BIGINT", true));

        let statements = planned_statements(&kind, schema, TABLE, &deployed, &inferred);
        assert_eq!(
            statements.len(),
            3,
            "{kind:?}: one ADD COLUMN per new column; got {statements:#?}"
        );
        execute_all(&rt, &kind, &db_path, schema, &statements);

        let actual = fetch_rows(&kind, &db_path, &warehouse, schema, TABLE);
        drop_bq_dataset(&kind);
        assert_eq!(
            actual.len(),
            1,
            "{kind:?}: the row must survive the migration"
        );
        if reference_rows.is_empty() {
            reference_rows = actual;
        } else {
            assert_eq!(
                actual, reference_rows,
                "{kind:?} rows differ from the DuckDB reference"
            );
        }
    }
}

/// A generated type-widening migration must be SQL its own target executes.
///
/// **Red on BigQuery** if `ddl_bigquery` emits DuckDB's
/// `ALTER COLUMN c TYPE t` (a syntax error in GoogleSQL, which spells it
/// `SET DATA TYPE`) or a `DECIMAL(p,s)` name instead of `NUMERIC(p,s)`.
#[cfg(feature = "duckdb")]
#[test]
fn generated_widen_type_ddl_executes_on_every_backend() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test.duckdb");
    let warehouse = tmp.path().join("warehouse");
    std::fs::create_dir_all(&warehouse).unwrap();
    let rt = tokio::runtime::Runtime::new().unwrap();

    const TABLE: &str = "evo_gen_widen";

    for kind in statement_ddl_targets("evo_gen_widen") {
        let schema = match &kind {
            TargetKind::DuckDb => "main",
            TargetKind::BigQuery { dataset } => dataset.as_str(),
            TargetKind::Spark => unreachable!(),
        };

        // Seeded with an explicit column type rather than a CAST: GoogleSQL
        // refuses a parameterized type inside CAST (`Parameterized types are
        // not allowed in CAST expressions`), and the fixture needs the
        // deployed column to really be DECIMAL(5,2) for the widening to be
        // the change under test.
        let (decl, qualified) = match &kind {
            TargetKind::DuckDb => ("DECIMAL(5,2)", format!("{schema}.{TABLE}")),
            TargetKind::BigQuery { dataset } => ("NUMERIC(5,2)", format!("`{dataset}.{TABLE}`")),
            TargetKind::Spark => unreachable!(),
        };
        execute_all(
            &rt,
            &kind,
            &db_path,
            schema,
            &[
                format!("DROP TABLE IF EXISTS {qualified}"),
                format!("CREATE TABLE {qualified} (amount {decl})"),
                format!("INSERT INTO {qualified} (amount) VALUES (1.25)"),
            ],
        );

        // A safe widening under `schema_evolution.md`: the scale grows and the
        // integer-digit capacity does not shrink.
        let deployed = vec![deployed_col("amount", "DECIMAL(5,2)", true)];
        let inferred = vec![deployed_col("amount", "DECIMAL(10,4)", true)];

        let statements = planned_statements(&kind, schema, TABLE, &deployed, &inferred);
        execute_all(&rt, &kind, &db_path, schema, &statements);

        let actual = fetch_rows(&kind, &db_path, &warehouse, schema, TABLE);
        drop_bq_dataset(&kind);
        assert_eq!(
            actual.len(),
            1,
            "{kind:?}: the row must survive the widening"
        );
        // The value is preserved; only its declared width changed. Backends
        // print the widened decimal differently (trailing zeros), so compare
        // numerically rather than as text.
        let printed = &actual[0][0];
        let parsed: f64 = printed
            .parse()
            .unwrap_or_else(|e| panic!("{kind:?}: could not parse widened value {printed:?}: {e}"));
        assert!(
            (parsed - 1.25).abs() < 1e-9,
            "{kind:?}: expected 1.25 after widening, got {printed}"
        );
    }
}
