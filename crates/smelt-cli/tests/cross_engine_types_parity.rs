//! W6·P4 — Cross-engine Parquet type conformance (decimal precision + timestamp NTZ).
//!
//! Validates that DECIMAL(p,s) and TIMESTAMP_NTZ values round-trip correctly
//! across the Spark→DuckDB Parquet boundary via the wired cross-engine
//! `read_parquet` substitution (landed in W6·P3).
//!
//! With `SPARK_CONNECT_URL` unset or `--features spark` absent: skips green.
//! With both set: runs the full cross-engine exchange path on a live Spark server.
//!
//! Invariant under test (from `multi_backend.md` §"Cross-engine data exchange"):
//!   A DuckDB model that references a Spark model via `smelt.ref()` receives
//!   its data through a Parquet file written by Spark and read by DuckDB's
//!   `read_parquet(...)` function.  The Parquet boundary must preserve column
//!   types faithfully — no truncation, rescaling, or timezone shift.

// This file requires both spark and duckdb features; gate at the top so the
// compiler does not emit dead-code warnings in other builds.
#![cfg(all(feature = "spark", feature = "duckdb"))]

mod common;
use common::spark_connect_url;
use std::path::PathBuf;
use tempfile::TempDir;

/// Returns the shared Spark warehouse path (bind-mounted to the host).
fn get_warehouse_path() -> PathBuf {
    let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let default_wh = repo_root.join(".smelt-spark-warehouse");
    std::env::var("SMELT_SPARK_WAREHOUSE")
        .map(PathBuf::from)
        .unwrap_or(default_wh)
}

/// Stage a two-model workspace for a specific Spark schema.
/// Each test uses its own schema name to avoid parallel-execution conflicts.
///
/// - `spark_types` on the Spark target: a DECIMAL(10,2) + TIMESTAMP_NTZ row
/// - `duckdb_types` on the default DuckDB target: reads from `smelt.spark_types`
fn stage_types_workspace(tmp: &TempDir, spark_schema: &str) -> PathBuf {
    let root = tmp.path().join("types_proj");
    std::fs::create_dir_all(root.join("models")).unwrap();
    std::fs::create_dir_all(root.join("target")).unwrap();

    let url = spark_connect_url().unwrap_or_else(|| "sc://localhost:15002".to_string());
    let wh_path = get_warehouse_path();
    std::fs::create_dir_all(&wh_path).unwrap();
    let wh = wh_path
        .to_str()
        .expect("warehouse path is valid UTF-8")
        .to_string();

    let yml = format!(
        "name: types_proj\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: target/dev.duckdb\n    schema: main\n  spark:\n    type: spark\n    connect_url: {url}\n    catalog: spark_catalog\n    schema: {spark_schema}\n    warehouse: {wh}\n    format: parquet\ndefault_materialization: table\n"
    );
    std::fs::write(root.join("smelt.yml"), yml).unwrap();

    // Spark model: DECIMAL(10,2) and TIMESTAMP_NTZ columns.
    // TIMESTAMP_NTZ (without-timezone) stores as isAdjustedToUTC=false in Parquet,
    // which DuckDB reads back as a plain TIMESTAMP — no timezone conversion, no shift.
    std::fs::write(
        root.join("models").join("spark_types.sql"),
        "---\ntarget: spark\nmaterialization: table\n---\n\
         SELECT\n  \
         CAST(12345.67 AS DECIMAL(10, 2)) AS amount,\n  \
         CAST('2026-06-30 12:00:00' AS TIMESTAMP_NTZ) AS ts\n",
    )
    .unwrap();

    // DuckDB model: references spark_types; resolved to read_parquet(...) via P3 wiring.
    std::fs::write(
        root.join("models").join("duckdb_types.sql"),
        "SELECT amount, ts FROM smelt.spark_types\n",
    )
    .unwrap();

    root
}

/// Run `smelt run --target dev` on the project and return (stdout, stderr, success).
fn smelt_run_dev(project_dir: &std::path::Path) -> (String, String, bool) {
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_smelt"));
    cmd.args([
        "run",
        "--project-dir",
        project_dir.to_str().unwrap(),
        "--target",
        "dev",
    ]);
    if let Some(url) = spark_connect_url() {
        cmd.env("SPARK_CONNECT_URL", url);
    }
    let out = cmd.output().expect("failed to spawn `smelt run`");
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
        out.status.success(),
    )
}

/// DECIMAL(10,2) round-trips through the Spark→DuckDB Parquet boundary unchanged.
///
/// **Expected green** if Parquet preserves the DECIMAL logical type: DuckDB reads
/// Spark's `DECIMAL(10,2)` Parquet column as Arrow `Decimal128(10,2)` with the
/// exact value `12345.67` — no truncation or rescale.
///
/// **Red** if the precision/scale metadata is lost in transit or DuckDB widens/
/// narrows the type (e.g. reads as Float64 dropping trailing zeros).
#[test]
fn decimal_precision_roundtrips_spark_to_duckdb() {
    if !cfg!(feature = "spark") {
        eprintln!(
            "spark feature not enabled — skipping decimal_precision_roundtrips_spark_to_duckdb"
        );
        return;
    }
    let Some(_url) = spark_connect_url() else {
        eprintln!(
            "SPARK_CONNECT_URL unset — skipping decimal_precision_roundtrips_spark_to_duckdb"
        );
        return;
    };

    let tmp = TempDir::new().unwrap();
    let root = stage_types_workspace(&tmp, "smelt_types_p4_dec");
    let db_path = root.join("target/dev.duckdb");

    let (stdout, stderr, success) = smelt_run_dev(&root);
    assert!(
        success,
        "smelt run failed for types workspace.\nstdout: {stdout}\nstderr: {stderr}"
    );

    let rt = tokio::runtime::Runtime::new().unwrap();
    let (val_str, col_type) = rt.block_on(async {
        use smelt_backend::Backend;
        use smelt_backend_duckdb::DuckDbBackend;

        let backend = DuckDbBackend::new(&db_path, "main")
            .await
            .expect("DuckDB open failed");
        let batches = backend
            .execute_sql("SELECT amount FROM main.duckdb_types")
            .await
            .expect("query on duckdb_types failed");

        let batch = batches.first().expect("no batches returned");
        let col = batch.column(0);
        let val = arrow::util::display::array_value_to_string(col, 0)
            .unwrap_or_else(|_| "<display error>".to_string());
        let dt = batch.schema().field(0).data_type().clone();
        (val, dt)
    });

    // Value must round-trip exactly (no float approximation).
    assert_eq!(
        val_str, "12345.67",
        "decimal value did not round-trip: expected '12345.67' but got '{val_str}'"
    );

    // Precision and scale must be preserved through Parquet.
    assert!(
        matches!(col_type, arrow::datatypes::DataType::Decimal128(10, 2)),
        "expected Decimal128(10, 2) but the materialized column has type {col_type:?} — \
         Parquet precision/scale metadata may have been dropped or widened"
    );
}

/// TIMESTAMP_NTZ round-trips through the Spark→DuckDB Parquet boundary as the same
/// calendar instant with no timezone shift.
///
/// TIMESTAMP_NTZ in Spark is stored as `TIMESTAMP_MICROS` with `isAdjustedToUTC=false`
/// in Parquet.  DuckDB reads this as a plain TIMESTAMP (no TZ component), so the
/// displayed value matches the original string exactly regardless of the host timezone.
///
/// **Expected green** if DuckDB correctly interprets the `isAdjustedToUTC=false` flag.
/// **Red** if DuckDB applies a UTC→local conversion (e.g. shifts the hour), indicating
/// a TZ-semantics mismatch that requires a fix in `type_conformance.rs` or the
/// `read_parquet` substitution.
#[test]
fn timestamp_tz_roundtrips_spark_to_duckdb() {
    if !cfg!(feature = "spark") {
        eprintln!("spark feature not enabled — skipping timestamp_tz_roundtrips_spark_to_duckdb");
        return;
    }
    let Some(_url) = spark_connect_url() else {
        eprintln!("SPARK_CONNECT_URL unset — skipping timestamp_tz_roundtrips_spark_to_duckdb");
        return;
    };

    let tmp = TempDir::new().unwrap();
    let root = stage_types_workspace(&tmp, "smelt_types_p4_ts");
    let db_path = root.join("target/dev.duckdb");

    let (stdout, stderr, success) = smelt_run_dev(&root);
    assert!(
        success,
        "smelt run failed for types workspace.\nstdout: {stdout}\nstderr: {stderr}"
    );

    let rt = tokio::runtime::Runtime::new().unwrap();
    let val_str = rt.block_on(async {
        use smelt_backend::Backend;
        use smelt_backend_duckdb::DuckDbBackend;

        let backend = DuckDbBackend::new(&db_path, "main")
            .await
            .expect("DuckDB open failed");
        let batches = backend
            .execute_sql("SELECT CAST(ts AS VARCHAR) AS ts FROM main.duckdb_types")
            .await
            .expect("query on duckdb_types failed");

        let batch = batches.first().expect("no batches returned");
        let col = batch.column(0);
        arrow::util::display::array_value_to_string(col, 0)
            .unwrap_or_else(|_| "<display error>".to_string())
    });

    // The timestamp must survive the Parquet round-trip with date and time intact.
    // '2026-06-30 12:00:00' was written as TIMESTAMP_NTZ (isAdjustedToUTC=false);
    // DuckDB should read it back as the same calendar value, no TZ shift.
    assert!(
        val_str.contains("2026-06-30") && val_str.contains("12:00:00"),
        "timestamp did not round-trip correctly: \
         expected a value containing '2026-06-30' and '12:00:00' but got {val_str:?}"
    );
}

/// Stages a workspace with a TZ-aware TIMESTAMP model for the cross-engine round-trip test.
fn stage_tz_aware_workspace(tmp: &TempDir, spark_schema: &str) -> PathBuf {
    let root = tmp.path().join("tz_proj");
    std::fs::create_dir_all(root.join("models")).unwrap();
    std::fs::create_dir_all(root.join("target")).unwrap();

    let url = spark_connect_url().unwrap_or_else(|| "sc://localhost:15002".to_string());
    let wh_path = get_warehouse_path();
    std::fs::create_dir_all(&wh_path).unwrap();
    let wh = wh_path
        .to_str()
        .expect("warehouse path is valid UTF-8")
        .to_string();

    let yml = format!(
        "name: tz_proj\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: target/dev.duckdb\n    schema: main\n  spark:\n    type: spark\n    connect_url: {url}\n    catalog: spark_catalog\n    schema: {spark_schema}\n    warehouse: {wh}\n    format: parquet\ndefault_materialization: table\n"
    );
    std::fs::write(root.join("smelt.yml"), yml).unwrap();

    // Spark model: TZ-aware TIMESTAMP (stored UTC, isAdjustedToUTC=true in Parquet).
    // Use a string literal with explicit UTC zone so Spark records it as a UTC instant.
    // 2025-06-30 12:00:00 UTC = epoch 1751284800 s = 1751284800000000 µs.
    std::fs::write(
        root.join("models").join("spark_tz.sql"),
        "---\ntarget: spark\nmaterialization: table\n---\n\
         SELECT CAST('2025-06-30 12:00:00+00:00' AS TIMESTAMP) AS ts_tz\n",
    )
    .unwrap();

    // DuckDB model: cross-engine read via read_parquet substitution.
    std::fs::write(
        root.join("models").join("duckdb_tz.sql"),
        "SELECT ts_tz FROM smelt.spark_tz\n",
    )
    .unwrap();

    root
}

/// TZ-aware TIMESTAMP round-trips through the Spark→DuckDB Parquet boundary as the same UTC instant.
///
/// Spark's `TIMESTAMP` (with timezone) stores as `TIMESTAMP_MICROS` with
/// `isAdjustedToUTC=true` in Parquet.  DuckDB reads this as `TIMESTAMPTZ` preserving
/// the UTC epoch.  The test uses a microsecond epoch value (1751284800000000 µs =
/// 2025-06-30 12:00:00 UTC) to avoid any ambiguity from local-timezone conversion.
///
/// **Expected green** when the UTC epoch survives the Parquet boundary intact.
/// **Red** if DuckDB applies a spurious TZ shift (e.g. reads it as local time).
#[test]
fn timestamp_tz_aware_roundtrips_spark_to_duckdb() {
    if !cfg!(feature = "spark") {
        eprintln!(
            "spark feature not enabled — skipping timestamp_tz_aware_roundtrips_spark_to_duckdb"
        );
        return;
    }
    let Some(_url) = spark_connect_url() else {
        eprintln!(
            "SPARK_CONNECT_URL unset — skipping timestamp_tz_aware_roundtrips_spark_to_duckdb"
        );
        return;
    };

    let tmp = TempDir::new().unwrap();
    let root = stage_tz_aware_workspace(&tmp, "smelt_types_tz_aware");
    let db_path = root.join("target/dev.duckdb");

    let (stdout, stderr, success) = smelt_run_dev(&root);
    assert!(
        success,
        "smelt run failed for TZ-aware types workspace.\nstdout: {stdout}\nstderr: {stderr}"
    );

    let rt = tokio::runtime::Runtime::new().unwrap();
    let val_str = rt.block_on(async {
        use smelt_backend::Backend;
        use smelt_backend_duckdb::DuckDbBackend;

        let backend = DuckDbBackend::new(&db_path, "main")
            .await
            .expect("DuckDB open failed");
        // Cast to epoch microseconds to compare the UTC instant directly,
        // avoiding display differences from DuckDB's local timezone setting.
        let batches = backend
            .execute_sql("SELECT EPOCH_US(ts_tz) AS epoch_us FROM main.duckdb_tz")
            .await
            .expect("query on duckdb_tz failed");

        let batch = batches.first().expect("no batches returned");
        let col = batch.column(0);
        arrow::util::display::array_value_to_string(col, 0)
            .unwrap_or_else(|_| "<display error>".to_string())
    });

    // 1751284800000000 µs = 2025-06-30 12:00:00 UTC — the exact epoch we encoded.
    assert_eq!(
        val_str, "1751284800000000",
        "TZ-aware timestamp did not round-trip correctly: \
         expected epoch_us=1751284800000000 but got {val_str:?}"
    );
}
