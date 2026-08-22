//! Empirical live-Spark tests for two provisional capability cells (W6·P2).
//!
//! Determines the real behavior of:
//!   - `supports_struct_field_ddl`  on Spark **Parquet**:
//!     code=true (before P2), matrix=✗
//!   - `supports_nested_array_ddl`  on Spark **Delta**:
//!     code=false (before P2), matrix=✓
//!
//! Each test attempts the actual ALTER against a live server and asserts that
//! the `BackendCapabilities` constructor flag matches the observed result.
//! Both require a live Spark Connect server (`SPARK_CONNECT_URL` set); both
//! skip silently when the variable is absent. The Delta test additionally
//! requires Delta Lake on the server — it records a skip when Delta is absent.

use smelt_backend::Backend;
use smelt_backend_spark::SparkBackend;
use smelt_dialect::BackendCapabilities;

/// Helper: connect to Spark or skip.
async fn spark_or_skip(url: &str) -> Option<SparkBackend> {
    match SparkBackend::new(url, "spark_catalog", "default", None, true).await {
        Ok(b) => Some(b),
        Err(e) => {
            eprintln!("Skipping — could not connect to Spark: {}", e);
            None
        }
    }
}

/// Empirically verify `supports_struct_field_ddl` for Spark Parquet.
///
/// Creates a Parquet table with a struct column, attempts to add a nested
/// struct field via `ALTER TABLE … ADD COLUMNS (struct_col.new_field TYPE)`,
/// and asserts that the constructor flag matches the observed success/failure.
///
/// Observed (2026-06-30, Spark 4.1.x, no Delta): ALTER fails with
/// [UNSUPPORTED_FEATURE.TABLE_OPERATION] — flag must be `false`.
#[tokio::test]
async fn spark_parquet_struct_field_ddl_observed() {
    let Some(url) = std::env::var("SPARK_CONNECT_URL").ok() else {
        eprintln!("Skipping spark_parquet_struct_field_ddl_observed — set SPARK_CONNECT_URL");
        return;
    };
    let Some(backend) = spark_or_skip(&url).await else {
        return;
    };

    // Clean up from any previous run.
    let _ = backend
        .execute_sql("DROP TABLE IF EXISTS spark_catalog.default.smelt_w6p2_parquet_struct")
        .await;

    // Create a Parquet table with a struct column.
    backend
        .execute_sql(
            "CREATE TABLE spark_catalog.default.smelt_w6p2_parquet_struct \
             (id INT, info STRUCT<name: STRING, age: INT>) USING PARQUET",
        )
        .await
        .expect("CREATE TABLE with struct column should succeed");

    // Attempt to add a nested struct field via DDL.
    let result = backend
        .execute_sql(
            "ALTER TABLE spark_catalog.default.smelt_w6p2_parquet_struct \
             ADD COLUMNS (info.score DOUBLE)",
        )
        .await;

    let observed = result.is_ok();
    if observed {
        eprintln!("supports_struct_field_ddl on Parquet: TRUE — ALTER succeeded");
    } else {
        eprintln!(
            "supports_struct_field_ddl on Parquet: FALSE — ALTER failed: {}",
            result.unwrap_err()
        );
    }

    // Clean up.
    let _ = backend
        .execute_sql("DROP TABLE IF EXISTS spark_catalog.default.smelt_w6p2_parquet_struct")
        .await;

    let caps = BackendCapabilities::spark_parquet();
    assert_eq!(
        caps.supports_struct_field_ddl,
        observed,
        "BackendCapabilities::spark_parquet().supports_struct_field_ddl = {} but live Spark \
         {} the struct-field ALTER — update dialect.rs:spark_parquet() to match the observed truth",
        caps.supports_struct_field_ddl,
        if observed { "ACCEPTED" } else { "REJECTED" },
    );
}

/// Empirically verify `supports_nested_array_ddl` for Spark Delta.
///
/// Creates a Delta table with an `ARRAY<STRUCT<…>>` column, attempts to add
/// a nested field via `ALTER TABLE … ADD COLUMNS (arr.element.new_field TYPE)`,
/// and asserts that the constructor flag matches the observed success/failure.
///
/// Requires Delta Lake on the Spark server. Skips gracefully when Delta is
/// absent (`[DATA_SOURCE_NOT_FOUND] DELTA`), recording the gap for human triage.
#[tokio::test]
async fn spark_delta_nested_array_ddl_observed() {
    let Some(url) = std::env::var("SPARK_CONNECT_URL").ok() else {
        eprintln!("Skipping spark_delta_nested_array_ddl_observed — set SPARK_CONNECT_URL");
        return;
    };
    let Some(backend) = spark_or_skip(&url).await else {
        return;
    };

    // Clean up from any previous run.
    let _ = backend
        .execute_sql("DROP TABLE IF EXISTS spark_catalog.default.smelt_w6p2_delta_array")
        .await;

    // Attempt to create a Delta table.  If Delta Lake is absent, skip gracefully.
    let create_result = backend
        .execute_sql(
            "CREATE TABLE spark_catalog.default.smelt_w6p2_delta_array \
             (id INT, items ARRAY<STRUCT<x: INT>>) USING DELTA",
        )
        .await;

    if let Err(ref e) = create_result {
        let msg = e.to_string();
        if msg.contains("DATA_SOURCE_NOT_FOUND") || msg.contains("DELTA") {
            eprintln!(
                "spark_delta_nested_array_ddl_observed: SKIPPED — Delta Lake not available \
                 on this server. Install the Delta connector to verify \
                 supports_nested_array_ddl. (matrix=✓, code=false — unresolved)"
            );
            return;
        }
        panic!("Unexpected error creating Delta table: {}", e);
    }

    // Attempt to add a nested field within the array-of-struct via DDL.
    let result = backend
        .execute_sql(
            "ALTER TABLE spark_catalog.default.smelt_w6p2_delta_array \
             ADD COLUMNS (items.element.y STRING)",
        )
        .await;

    let observed = result.is_ok();
    if observed {
        eprintln!("supports_nested_array_ddl on Delta: TRUE — ALTER succeeded");
    } else {
        eprintln!(
            "supports_nested_array_ddl on Delta: FALSE — ALTER failed: {}",
            result.unwrap_err()
        );
    }

    // Clean up.
    let _ = backend
        .execute_sql("DROP TABLE IF EXISTS spark_catalog.default.smelt_w6p2_delta_array")
        .await;

    let caps = BackendCapabilities::spark_delta();
    assert_eq!(
        caps.supports_nested_array_ddl,
        observed,
        "BackendCapabilities::spark_delta().supports_nested_array_ddl = {} but live Spark \
         {} the nested-array ALTER — update dialect.rs:spark_delta() to match the observed truth",
        caps.supports_nested_array_ddl,
        if observed { "ACCEPTED" } else { "REJECTED" },
    );
}

// ─────────────────────────────────────────────────────────────────────────
// smelt's own migration DDL, executed on the server
//
// The tests above measure the *server* — whether a given ALTER form works at
// all. The tests below take the statements `plan_migration_for_backend` emits
// for a Spark target and run those, so a generator that emits SQL its own
// target rejects fails here rather than in a user's run. It is the only
// assertion that can catch it: the generator is a pure function, and a unit
// test can only pin the string, never that a live server accepts it.
//
// This is the Spark counterpart of the BigQuery legs in
// `crates/smelt-cli/tests/schema_evolution_parity.rs`.

use smelt_state::ddl_spark::SparkTableFormat;
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

/// The DDL generator for a Spark target, as `execute.rs` selects it at run time.
fn spark_backend(format: SparkTableFormat) -> DdlBackend {
    DdlBackend::Spark {
        catalog: "spark_catalog".to_string(),
        format,
        capabilities: match format {
            SparkTableFormat::Delta => BackendCapabilities::spark_delta(),
            SparkTableFormat::Parquet => BackendCapabilities::spark_parquet(),
        },
    }
}

/// Plan the migration and return the statements, failing loudly on a refusal.
fn planned_statements(
    format: SparkTableFormat,
    table: &str,
    deployed: &[DeployedColumn],
    inferred: &[DeployedColumn],
    defaults: &HashMap<String, String>,
) -> Vec<String> {
    let diff = diff_schemas(deployed, inferred);
    assert!(
        !diff.is_empty(),
        "{format:?}: the fixture must produce a diff"
    );
    let action = plan_migration_for_backend(
        "default",
        table,
        &diff,
        false,
        defaults,
        &HashMap::new(),
        &spark_backend(format),
        deployed,
        inferred,
    );
    match action {
        MigrationAction::AlterTable { statements } => statements,
        other => panic!(
            "{format:?}: expected ALTER TABLE DDL for this change, got {other:?}. \
             A change the format cannot express falls back to a full refresh — \
             which is safe, but means this migration is not supported there."
        ),
    }
}

/// Rows a SELECT returns.
async fn count_rows(backend: &SparkBackend, sql: &str) -> usize {
    backend
        .execute_sql(sql)
        .await
        .unwrap_or_else(|e| panic!("query failed: {sql}\n  error: {e}"))
        .iter()
        .map(|b| b.num_rows())
        .sum()
}

/// Create a fresh table, seed one row, then run smelt's generated statements
/// against it. Panics with the statement text if the server refuses one.
async fn migrate_live(
    backend: &SparkBackend,
    qualified: &str,
    create: &str,
    seed: &str,
    statements: &[String],
) {
    let _ = backend
        .execute_sql(&format!("DROP TABLE IF EXISTS {qualified}"))
        .await;
    backend
        .execute_sql(create)
        .await
        .unwrap_or_else(|e| panic!("CREATE TABLE failed: {create}\n  error: {e}"));
    backend
        .execute_sql(seed)
        .await
        .unwrap_or_else(|e| panic!("INSERT failed: {e}"));
    for stmt in statements {
        backend.execute_sql(stmt).await.unwrap_or_else(|e| {
            panic!("Spark rejected smelt's migration DDL.\n  statement: {stmt}\n  error: {e}")
        });
    }
}

/// Whether this server has Delta Lake; the Delta legs skip without it.
async fn has_delta(backend: &SparkBackend) -> bool {
    let probe = "spark_catalog.default.smelt_delta_probe";
    let ok = backend
        .execute_sql(&format!("CREATE TABLE {probe} (id BIGINT) USING DELTA"))
        .await
        .is_ok();
    let _ = backend
        .execute_sql(&format!("DROP TABLE IF EXISTS {probe}"))
        .await;
    ok
}

/// A generated `ADD COLUMN` migration must be SQL its own target executes.
///
/// **Red before the whole-diff routing**: a flat `ADD COLUMN` was emitted in
/// DuckDB's dialect — `ALTER TABLE default.t ADD COLUMN note VARCHAR` — which
/// Spark refuses with `DATATYPE_MISSING_SIZE`, bare `VARCHAR` having no length.
#[tokio::test]
async fn generated_add_column_ddl_executes_on_spark() {
    let Some(url) = std::env::var("SPARK_CONNECT_URL").ok() else {
        eprintln!("Skipping generated_add_column_ddl_executes_on_spark — set SPARK_CONNECT_URL");
        return;
    };
    let Some(backend) = spark_or_skip(&url).await else {
        return;
    };
    let delta_available = has_delta(&backend).await;

    for (format, using) in [
        (SparkTableFormat::Delta, "DELTA"),
        (SparkTableFormat::Parquet, "PARQUET"),
    ] {
        if format == SparkTableFormat::Delta && !delta_available {
            eprintln!(
                "generated_add_column_ddl_executes_on_spark: Delta leg SKIPPED — no Delta Lake"
            );
            continue;
        }
        let table = format!("smelt_gen_add_{}", using.to_lowercase());
        let qualified = format!("spark_catalog.default.{table}");

        // v2 adds a nullable column of each family whose DuckDB type name
        // Spark rejects outright, so a shared generator cannot pass.
        let deployed = vec![
            deployed_col("id", "BIGINT", true),
            deployed_col("label", "VARCHAR", true),
        ];
        let mut inferred = deployed.clone();
        inferred.push(deployed_col("note", "VARCHAR", true));
        inferred.push(deployed_col("ratio", "DOUBLE", true));
        inferred.push(deployed_col("n", "BIGINT", true));

        let statements = planned_statements(format, &table, &deployed, &inferred, &HashMap::new());
        assert_eq!(
            statements.len(),
            3,
            "{format:?}: one ADD COLUMNS per new column; got {statements:#?}"
        );

        migrate_live(
            &backend,
            &qualified,
            &format!("CREATE TABLE {qualified} (id BIGINT, label STRING) USING {using}"),
            &format!("INSERT INTO {qualified} VALUES (1, 'a')"),
            &statements,
        )
        .await;

        let rows = count_rows(&backend, &format!("SELECT * FROM {qualified}")).await;
        assert_eq!(rows, 1, "{format:?}: the row must survive the migration");
        let _ = backend
            .execute_sql(&format!("DROP TABLE IF EXISTS {qualified}"))
            .await;
    }
}

/// A generated `default:` migration must fill the rows already in the table.
///
/// Delta refuses a `DEFAULT` clause on `ADD COLUMNS`, so the generator spells
/// the same outcome as a plain add plus an `UPDATE`. **Red** if it emits the
/// DEFAULT clause (`WRONG_COLUMN_DEFAULTS_FOR_DELTA_…`) or omits the UPDATE —
/// the existing row would keep NULL, diverging from DuckDB.
#[tokio::test]
async fn generated_default_backfill_executes_on_spark_delta() {
    let Some(url) = std::env::var("SPARK_CONNECT_URL").ok() else {
        eprintln!(
            "Skipping generated_default_backfill_executes_on_spark_delta — set SPARK_CONNECT_URL"
        );
        return;
    };
    let Some(backend) = spark_or_skip(&url).await else {
        return;
    };
    if !has_delta(&backend).await {
        eprintln!("generated_default_backfill_executes_on_spark_delta: SKIPPED — no Delta Lake");
        return;
    }

    let table = "smelt_gen_default_delta";
    let qualified = format!("spark_catalog.default.{table}");
    let deployed = vec![deployed_col("id", "BIGINT", true)];
    let inferred = vec![
        deployed_col("id", "BIGINT", true),
        deployed_col("status", "VARCHAR", true),
    ];
    let mut defaults = HashMap::new();
    defaults.insert("status".to_string(), "'pending'".to_string());

    let statements = planned_statements(
        SparkTableFormat::Delta,
        table,
        &deployed,
        &inferred,
        &defaults,
    );

    migrate_live(
        &backend,
        &qualified,
        &format!("CREATE TABLE {qualified} (id BIGINT) USING DELTA"),
        &format!("INSERT INTO {qualified} VALUES (1)"),
        &statements,
    )
    .await;

    let filled = count_rows(
        &backend,
        &format!("SELECT * FROM {qualified} WHERE status = 'pending'"),
    )
    .await;
    let _ = backend
        .execute_sql(&format!("DROP TABLE IF EXISTS {qualified}"))
        .await;
    assert_eq!(
        filled, 1,
        "the default must fill the row already in the table, as it does on DuckDB"
    );
}

/// Relaxing nullability is DDL on Delta; the generated statement must execute.
#[tokio::test]
async fn generated_drop_not_null_executes_on_spark_delta() {
    let Some(url) = std::env::var("SPARK_CONNECT_URL").ok() else {
        eprintln!(
            "Skipping generated_drop_not_null_executes_on_spark_delta — set SPARK_CONNECT_URL"
        );
        return;
    };
    let Some(backend) = spark_or_skip(&url).await else {
        return;
    };
    if !has_delta(&backend).await {
        eprintln!("generated_drop_not_null_executes_on_spark_delta: SKIPPED — no Delta Lake");
        return;
    }

    let table = "smelt_gen_drop_not_null";
    let qualified = format!("spark_catalog.default.{table}");
    let deployed = vec![deployed_col("id", "BIGINT", false)];
    let inferred = vec![deployed_col("id", "BIGINT", true)];
    let statements = planned_statements(
        SparkTableFormat::Delta,
        table,
        &deployed,
        &inferred,
        &HashMap::new(),
    );

    migrate_live(
        &backend,
        &qualified,
        &format!("CREATE TABLE {qualified} (id BIGINT NOT NULL) USING DELTA"),
        &format!("INSERT INTO {qualified} VALUES (1)"),
        &statements,
    )
    .await;

    let rows = count_rows(&backend, &format!("SELECT * FROM {qualified}")).await;
    let _ = backend
        .execute_sql(&format!("DROP TABLE IF EXISTS {qualified}"))
        .await;
    assert_eq!(rows, 1, "the row must survive the migration");
}
