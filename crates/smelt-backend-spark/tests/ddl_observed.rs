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
