#![cfg(feature = "duckdb")]
//! Integration tests for `columns.<c>.tests` — declarative column tests
//! (`docs/specs/data_tests.md`).
//!
//! TDD: tests were written before the implementation to drive the feature.
//! Each test copies a real `examples/` workspace into a temp dir, injects
//! frontmatter declaring `columns.<c>.tests`, and runs the real `smelt`
//! binary against it — the same harness style as `tests/check_command.rs`.

use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

fn smelt_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_smelt"))
}

fn examples_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("examples")
}

/// Copy a directory tree, skipping `target/` subdirectories.
fn copy_dir(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap();
    for entry in std::fs::read_dir(src).unwrap().flatten() {
        let path = entry.path();
        let name = entry.file_name();
        if name == "target" {
            continue;
        }
        let dest = dst.join(&name);
        if path.is_dir() {
            copy_dir(&path, &dest);
        } else {
            std::fs::copy(&path, &dest).unwrap();
        }
    }
}

/// Copy `examples/data_checks` into a fresh temp dir, without building it.
fn copy_data_checks() -> (TempDir, PathBuf) {
    let tmp = TempDir::new().unwrap();
    let src = examples_root().join("data_checks");
    let dest = tmp.path().join("data_checks");
    copy_dir(&src, &dest);
    (tmp, dest)
}

fn run_smelt(project_dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new(smelt_bin())
        .args(args)
        .args(["--project-dir", project_dir.to_str().unwrap()])
        .env_remove("RUST_LOG")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt {}`: {e}", args.join(" ")))
}

/// Build the project and assert it succeeds — the unproven-scan tests need
/// real materialized data to scan (contrast: the proven-verdict tests above
/// need no built target at all).
fn build_project(project_dir: &Path) {
    let out = run_smelt(project_dir, &["build"]);
    assert!(
        out.status.success(),
        "smelt build failed.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A `columns.<c>.tests` entry naming a column absent from the model's
/// inferred output schema is a hard diagnostic — contrast with the
/// silent-drop rule for other `columns:` keys
/// (`docs/specs/data_tests.md` §"Fail-loud validation").
#[test]
fn test_on_unknown_column_is_diagnostic() {
    let (_tmp, project_dir) = copy_data_checks();

    // `revenue.sql` originally has no frontmatter and outputs `order_id`,
    // `amount`. Declare a test on a column that does not exist.
    std::fs::write(
        project_dir.join("models/revenue.sql"),
        r#"---
name: revenue
columns:
  order_id:
    tests:
      - not_null
  bogus_column:
    tests:
      - not_null
---
SELECT
    1 AS order_id,
    100.0 AS amount
"#,
    )
    .unwrap();

    let out = run_smelt(&project_dir, &["run", "--dry-run"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let combined = format!("{stdout}{stderr}");

    assert!(
        !out.status.success(),
        "smelt run --dry-run should fail loudly on a test naming an unmodeled column.\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        combined.contains("bogus_column"),
        "diagnostic should name the offending column.\ncombined: {combined}"
    );
}

/// A `not_null` test on a column the type checker already proves
/// non-nullable reports `proven` — no scan is emitted for it
/// (`docs/specs/data_tests.md` §Semantics "Resolution order").
#[test]
fn proven_not_null_emits_no_scan() {
    let (_tmp, project_dir) = copy_data_checks();

    // Both `order_id` and `amount` are literal constants (Computed,
    // non-nullable) — the type checker proves `order_id IS NOT NULL`.
    std::fs::write(
        project_dir.join("models/revenue.sql"),
        r#"---
name: revenue
columns:
  order_id:
    tests:
      - not_null
---
SELECT
    1 AS order_id,
    100.0 AS amount
"#,
    )
    .unwrap();

    // The declarative-tests proof pass needs no built target — run `smelt
    // check` directly (no `smelt build` first).
    let out = run_smelt(&project_dir, &["check"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);

    assert!(
        stdout.contains("PROVEN") && stdout.contains("revenue.order_id.not_null"),
        "output should report the not_null test as proven, with no scan.\nstdout: {stdout}\nstderr: {stderr}"
    );
}

/// A `unique` test on the model's own declared `unique_key:` reports
/// `proven` — the grain key is the proof, no scan needed
/// (`docs/specs/data_tests.md` §Semantics "Resolution order").
#[test]
fn proven_unique_via_grain_key() {
    let (_tmp, project_dir) = copy_data_checks();

    std::fs::write(
        project_dir.join("models/revenue.sql"),
        r#"---
name: revenue
materialization: table
refresh: incremental
unique_key: [order_id]
columns:
  order_id:
    tests:
      - unique
---
SELECT
    1 AS order_id,
    100.0 AS amount
"#,
    )
    .unwrap();

    let out = run_smelt(&project_dir, &["check"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);

    assert!(
        stdout.contains("PROVEN") && stdout.contains("revenue.order_id.unique"),
        "output should report the unique test as proven via the declared unique_key.\nstdout: {stdout}\nstderr: {stderr}"
    );
}

/// A `not_null` test on a column the type checker cannot prove non-nullable
/// (a real NULL value is present) lowers to a failing-rows scan: exit 1,
/// output names `<model>.<column> not_null` and the failing-row count
/// (`docs/specs/data_tests.md` §Semantics step 2).
#[test]
fn unproven_not_null_scan_fails_on_nulls() {
    let (_tmp, project_dir) = copy_data_checks();

    std::fs::write(
        project_dir.join("models/revenue.sql"),
        r#"---
name: revenue
columns:
  amount:
    tests:
      - not_null
---
SELECT 1 AS order_id, 100.0 AS amount
UNION ALL
SELECT 2 AS order_id, CAST(NULL AS DOUBLE) AS amount
"#,
    )
    .unwrap();

    build_project(&project_dir);

    let out = run_smelt(&project_dir, &["check"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);

    assert!(
        !out.status.success(),
        "smelt check should exit nonzero when the not_null scan finds a NULL.\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        stdout.contains("FAIL") && stdout.contains("revenue.amount.not_null"),
        "output should name the failing model.column.test.\nstdout: {stdout}"
    );
    assert!(
        stdout.contains("1 violating row"),
        "output should report the failing-row count.\nstdout: {stdout}"
    );
}

/// `accepted_values` always lowers to a scan (no proof path exists —
/// `docs/specs/data_tests.md` §"Known Divergences"). Both directions:
/// a column whose values are all in the accepted list PASSes; a column with
/// an out-of-list value FAILs.
#[test]
fn accepted_values_pass_and_fail() {
    let (_tmp, project_dir) = copy_data_checks();

    std::fs::write(
        project_dir.join("models/revenue.sql"),
        r#"---
name: revenue
columns:
  status_ok:
    tests:
      - accepted_values: ['a', 'b']
  status_bad:
    tests:
      - accepted_values: ['a', 'b']
---
SELECT 1 AS order_id, 100.0 AS amount, 'a' AS status_ok, 'a' AS status_bad
UNION ALL
SELECT 2 AS order_id, 50.0 AS amount, 'b' AS status_ok, 'z' AS status_bad
"#,
    )
    .unwrap();

    build_project(&project_dir);

    let out = run_smelt(&project_dir, &["check"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);

    assert!(
        !out.status.success(),
        "smelt check should exit nonzero — status_bad has a violation.\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        stdout.contains("PASS") && stdout.contains("revenue.status_ok.accepted_values"),
        "status_ok (all values accepted) should PASS.\nstdout: {stdout}"
    );
    assert!(
        stdout.contains("FAIL") && stdout.contains("revenue.status_bad.accepted_values"),
        "status_bad (one out-of-list value) should FAIL.\nstdout: {stdout}"
    );
}

/// `relationships` always lowers to a NOT-EXISTS anti-join scan (no proof
/// path exists today). A child row whose foreign key has no matching parent
/// row is an orphan → FAIL, exit 1; intact referential integrity → PASS,
/// exit 0 (`docs/specs/data_tests.md` §Semantics step 2).
#[test]
fn relationships_orphan_fails() {
    // Orphan case: customer_id=99 has no matching row in `customers`.
    let (_tmp, project_dir) = copy_data_checks();
    std::fs::write(
        project_dir.join("models/customers.sql"),
        "SELECT 1 AS id\nUNION ALL\nSELECT 2 AS id\n",
    )
    .unwrap();
    std::fs::write(
        project_dir.join("models/revenue.sql"),
        r#"---
name: revenue
columns:
  customer_id:
    tests:
      - relationships:
          to: customers
          field: id
---
SELECT 1 AS order_id, 100.0 AS amount, 1 AS customer_id
UNION ALL
SELECT 2 AS order_id, 50.0 AS amount, 99 AS customer_id
"#,
    )
    .unwrap();

    build_project(&project_dir);

    let out = run_smelt(&project_dir, &["check"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);

    assert!(
        !out.status.success(),
        "smelt check should exit nonzero — customer_id=99 is an orphan.\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        stdout.contains("FAIL") && stdout.contains("revenue.customer_id.relationships"),
        "output should name the failing relationships test.\nstdout: {stdout}"
    );
    assert!(
        stdout.contains("1 violating row"),
        "output should report exactly one orphaned row.\nstdout: {stdout}"
    );

    // Intact case: every customer_id matches a row in `customers`.
    let (_tmp2, project_dir2) = copy_data_checks();
    std::fs::write(
        project_dir2.join("models/customers.sql"),
        "SELECT 1 AS id\nUNION ALL\nSELECT 2 AS id\n",
    )
    .unwrap();
    std::fs::write(
        project_dir2.join("models/revenue.sql"),
        r#"---
name: revenue
columns:
  customer_id:
    tests:
      - relationships:
          to: customers
          field: id
---
SELECT 1 AS order_id, 100.0 AS amount, 1 AS customer_id
UNION ALL
SELECT 2 AS order_id, 50.0 AS amount, 2 AS customer_id
"#,
    )
    .unwrap();

    build_project(&project_dir2);

    let out2 = run_smelt(&project_dir2, &["check"]);
    let stdout2 = String::from_utf8_lossy(&out2.stdout);
    let stderr2 = String::from_utf8_lossy(&out2.stderr);

    assert!(
        out2.status.success(),
        "smelt check should exit 0 — referential integrity is intact.\nstdout: {stdout2}\nstderr: {stderr2}"
    );
    assert!(
        stdout2.contains("PASS") && stdout2.contains("revenue.customer_id.relationships"),
        "output should report the relationships test as PASS.\nstdout: {stdout2}"
    );
}

/// The lowered failing-rows SQL for each test kind comes from the pure
/// `smelt_logical::lower_column_test` emitter — not string concatenation in
/// the CLI command layer. Exercised directly as a unit-level check of the
/// emitter's output shape for all four kinds.
#[test]
fn generated_sql_is_emitter_authored() {
    use smelt_core::metadata::ColumnTest;
    use smelt_logical::lower_column_test;

    let not_null = lower_column_test(
        "revenue",
        "amount",
        &ColumnTest::Simple("not_null".to_string()),
    )
    .unwrap();
    assert_eq!(not_null.kind, "not_null");
    assert!(not_null.failing_rows_sql.contains("amount IS NULL"));
    assert!(not_null.failing_rows_sql.contains("smelt.revenue"));

    let unique = lower_column_test(
        "revenue",
        "order_id",
        &ColumnTest::Simple("unique".to_string()),
    )
    .unwrap();
    assert_eq!(unique.kind, "unique");
    assert!(unique.failing_rows_sql.contains("GROUP BY order_id"));
    assert!(unique.failing_rows_sql.contains("HAVING COUNT(*) > 1"));

    let mut accepted_values_params = std::collections::BTreeMap::new();
    accepted_values_params.insert(
        "accepted_values".to_string(),
        serde_yaml::Value::Sequence(vec![serde_yaml::Value::String("a".to_string())]),
    );
    let accepted_values = lower_column_test(
        "revenue",
        "status",
        &ColumnTest::Parameterized(accepted_values_params),
    )
    .unwrap();
    assert_eq!(accepted_values.kind, "accepted_values");
    assert!(accepted_values.failing_rows_sql.contains("NOT IN ('a')"));

    let mut relationships_inner = serde_yaml::Mapping::new();
    relationships_inner.insert(
        serde_yaml::Value::String("to".to_string()),
        serde_yaml::Value::String("customers".to_string()),
    );
    relationships_inner.insert(
        serde_yaml::Value::String("field".to_string()),
        serde_yaml::Value::String("id".to_string()),
    );
    let mut relationships_params = std::collections::BTreeMap::new();
    relationships_params.insert(
        "relationships".to_string(),
        serde_yaml::Value::Mapping(relationships_inner),
    );
    let relationships = lower_column_test(
        "revenue",
        "customer_id",
        &ColumnTest::Parameterized(relationships_params),
    )
    .unwrap();
    assert_eq!(relationships.kind, "relationships");
    assert!(relationships.failing_rows_sql.contains("NOT EXISTS"));
    assert!(relationships.failing_rows_sql.contains("smelt.customers"));
}
