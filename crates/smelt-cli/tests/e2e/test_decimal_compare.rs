#![cfg(feature = "duckdb")]
//! Integration and unit tests for D-44: DECIMAL columns are compared exactly
//! (no 1e-6 tolerance); decimal-shaped strings in `inputs` are cast to DECIMAL.

use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

fn smelt_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_smelt"))
}

fn run_smelt_test(project_dir: &Path) -> std::process::Output {
    Command::new(smelt_bin())
        .arg("test")
        .args(["--project-dir", project_dir.to_str().unwrap()])
        .env_remove("RUST_LOG")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt test`: {e}"))
}

fn smelt_yml(name: &str) -> String {
    format!(
        "name: {name}\nversion: 1\npaths:\n  - models\n  - tests\n\
         targets:\n  dev:\n    type: duckdb\n    database: target/dev.duckdb\n    schema: main\n\
         default_materialization: view\n"
    )
}

/// A decimal-string input `'300.00'` must be cast to DECIMAL in the generated SQL
/// (not left as VARCHAR). The SUM of a DECIMAL column must compare exactly to the
/// decimal-string expected value `'300.00'`.
#[test]
fn decimal_string_coerces_to_decimal_cte() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("decimal_coerce_ws");
    std::fs::create_dir_all(root.join("models")).unwrap();
    std::fs::create_dir_all(root.join("tests")).unwrap();
    std::fs::create_dir_all(root.join("target")).unwrap();
    std::fs::write(root.join("smelt.yml"), smelt_yml("decimal_coerce_ws")).unwrap();

    // Model sums a DECIMAL column from smelt.orders.
    std::fs::write(
        root.join("models/total.sql"),
        "SELECT SUM(amount) AS total FROM smelt.orders\n",
    )
    .unwrap();

    // smelt.test: PASSING uses decimal-string values; EXPECT uses the exact DECIMAL sum.
    // If the values were cast to VARCHAR (old behaviour), DuckDB SUM would fail.
    // If cast to DECIMAL, SUM = 100.50 + 200.50 = 301.00.
    let test_sql = "smelt.test test_decimal_coerce AS (\n\
        SELECT SUM(amount) AS total FROM smelt.orders\n\
    )\n\
    PASSING orders AS (\n\
        {amount: '100.50'},\n\
        {amount: '200.50'}\n\
    )\n\
    EXPECT (\n\
        {total: '301.00'}\n\
    )\n";
    std::fs::write(root.join("tests/test_decimal_coerce.sql"), test_sql).unwrap();

    let output = run_smelt_test(&root);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "decimal-string coercion test must pass (SUM = 301.00);\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stdout.contains("PASS") || stdout.contains("pass") || stdout.contains("1 passed"),
        "decimal test must report PASS;\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
}

/// A DECIMAL actual value that differs from the expected value must FAIL,
/// even when the numeric difference is within the 1e-6 float tolerance.
/// This confirms that DECIMAL columns use exact (not float-tolerance) comparison.
#[test]
fn decimal_no_float_tolerance() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("decimal_exact_ws");
    std::fs::create_dir_all(root.join("models")).unwrap();
    std::fs::create_dir_all(root.join("tests")).unwrap();
    std::fs::create_dir_all(root.join("target")).unwrap();
    std::fs::write(root.join("smelt.yml"), smelt_yml("decimal_exact_ws")).unwrap();

    // Model: cast to DECIMAL(10,7) so Arrow returns a Decimal128 column.
    // The tiny delta (1e-7) is within float tolerance but must fail for DECIMAL.
    std::fs::write(
        root.join("models/val.sql"),
        "SELECT CAST('1.0000001' AS DECIMAL(10,7)) AS val\n",
    )
    .unwrap();

    // Expected: exactly '1.0000000' — differs from actual 1.0000001 by 1e-7.
    // Float tolerance (1e-6 relative) would accept this diff (~1e-7 / 1.0 < 1e-6).
    // DECIMAL exact comparison must reject it.
    // No PASSING needed (model has no external smelt deps).
    let test_sql = "smelt.test test_decimal_exact AS (\n\
        SELECT CAST('1.0000001' AS DECIMAL(10,7)) AS val\n\
    )\n\
    EXPECT (\n\
        {val: '1.0000000'}\n\
    )\n";
    std::fs::write(root.join("tests/test_decimal_exact.sql"), test_sql).unwrap();

    let output = run_smelt_test(&root);
    let stdout = String::from_utf8_lossy(&output.stdout);

    // The test must FAIL: 1.0000001 != 1.0000000 (exact DECIMAL comparison).
    // NOTE: use `!output.status.success()` only — "fail" (lowercase) also appears in
    // "0 failed" on passing runs, causing a false positive if we check the string.
    assert!(
        !output.status.success(),
        "decimal mismatch with tiny delta must fail with exact comparison (DECIMAL exact, not float-tolerance);\nstdout:\n{stdout}"
    );
}

/// FLOAT/DOUBLE columns must still use the 1e-6 relative-epsilon tolerance
/// (unchanged from pre-D44 behaviour). A tiny floating-point difference that
/// is within tolerance must still PASS.
#[test]
fn float_tolerance_unchanged() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("float_tol_ws");
    std::fs::create_dir_all(root.join("models")).unwrap();
    std::fs::create_dir_all(root.join("tests")).unwrap();
    std::fs::create_dir_all(root.join("target")).unwrap();
    std::fs::write(root.join("smelt.yml"), smelt_yml("float_tol_ws")).unwrap();

    // Model: a DOUBLE that differs from 1.0 by ~1e-8 (well within 1e-6 tolerance).
    std::fs::write(
        root.join("models/val.sql"),
        "SELECT (1.0::DOUBLE + 1e-8) AS val\n",
    )
    .unwrap();

    // Expected: 1.0 — diff ≈ 1e-8, tolerance 1e-6, must PASS for DOUBLE.
    // No PASSING needed (model has no external smelt deps).
    let test_sql = "smelt.test test_float_tol AS (\n\
        SELECT (1.0::DOUBLE + 1e-8) AS val\n\
    )\n\
    EXPECT (\n\
        {val: 1.0}\n\
    )\n";
    std::fs::write(root.join("tests/test_float_tol.sql"), test_sql).unwrap();

    let output = run_smelt_test(&root);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "float tolerance must still apply for DOUBLE columns;\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stdout.contains("PASS") || stdout.contains("pass") || stdout.contains("1 passed"),
        "float tolerance test must report PASS;\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
}
