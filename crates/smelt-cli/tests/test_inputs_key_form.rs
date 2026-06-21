#![cfg(feature = "duckdb")]
//! Integration tests for D-42: `inputs` keys use dot-separated bare address paths.
//!
//! Before D-42, the lookup used the underscore-joined CTE name (e.g. "silver_orders").
//! After D-42, the lookup uses the dot-separated public key (e.g. "silver.orders").

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

/// `inputs` keys that use dot-separation ("silver.orders") must inject mock
/// rows into the model under test, not silently produce an empty CTE.
///
/// This is the D-42 acceptance test: a model referencing `smelt.silver.orders`
/// and a test with `inputs: {"silver.orders": [{amount: 100}, {amount: 200}]}`
/// must pass (SUM = 300), proving the dot-key was resolved to the mock CTE.
#[test]
fn inputs_dot_key_resolves() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("dot_key_ws");
    std::fs::create_dir_all(root.join("models/silver")).unwrap();
    std::fs::create_dir_all(root.join("tests")).unwrap();
    std::fs::create_dir_all(root.join("target")).unwrap();
    std::fs::write(root.join("smelt.yml"), smelt_yml("dot_key_ws")).unwrap();

    // Model under test: aggregates over smelt.silver.orders (multi-segment path).
    std::fs::write(
        root.join("models/total.sql"),
        "SELECT SUM(amount) AS total FROM smelt.silver.orders\n",
    )
    .unwrap();

    // Placeholder source model so smelt.silver.orders resolves.
    std::fs::write(
        root.join("models/silver/orders.sql"),
        "SELECT amount FROM smelt.raw_data\n",
    )
    .unwrap();

    // Test: mock smelt.silver.orders with dot-separated key, expect SUM = 300.
    let test_sql = "--- name: test_total_dot_key ---\n\
        materialization: test\n\
        test:\n  \
          model: total\n  \
          inputs:\n    \
            silver.orders:\n      \
              - {amount: 100}\n      \
              - {amount: 200}\n  \
          expect:\n    \
            - {total: 300}\n\
        ---\n";
    std::fs::write(root.join("tests/test_total_dot_key.sql"), test_sql).unwrap();

    let output = run_smelt_test(&root);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "smelt test must exit 0 when the dot-key test passes;\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stdout.contains("PASS") || stdout.contains("pass") || stdout.contains("1 passed"),
        "dot-key test must pass (SUM(amount) = 300);\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
}

/// D-43: an `inputs` key that does not match any `smelt.<path>` dep of the
/// model must cause the test to fail with an `UnknownTestInput` diagnostic.
/// A typo ("order" instead of "orders") is the canonical example.
#[test]
fn unknown_inputs_key_fails_loudly() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("unknown_key_ws");
    std::fs::create_dir_all(root.join("models")).unwrap();
    std::fs::create_dir_all(root.join("tests")).unwrap();
    std::fs::create_dir_all(root.join("target")).unwrap();
    std::fs::write(root.join("smelt.yml"), smelt_yml("unknown_key_ws")).unwrap();

    std::fs::write(
        root.join("models/order_count.sql"),
        "SELECT COUNT(*) AS cnt FROM smelt.orders\n",
    )
    .unwrap();
    std::fs::write(root.join("models/orders.sql"), "SELECT id FROM smelt.raw\n").unwrap();

    // Typo: "order" instead of "orders" — must fail with UnknownTestInput.
    let test_sql = "--- name: test_typo_key ---\n\
        materialization: test\n\
        test:\n  \
          model: order_count\n  \
          inputs:\n    \
            order:\n      \
              - {id: 1}\n  \
          expect:\n    \
            - {cnt: 1}\n\
        ---\n";
    std::fs::write(root.join("tests/test_typo_key.sql"), test_sql).unwrap();

    let output = run_smelt_test(&root);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}{stderr}");

    // Must fail: 'order' is not a dep of order_count (dep is 'orders').
    assert!(
        !output.status.success() || stdout.contains("FAIL") || stdout.contains("fail"),
        "unknown inputs key must cause test failure;\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    // Error must be an UnknownTestInput diagnostic naming the bad key.
    assert!(
        combined.contains("UnknownTestInput") || combined.contains("not a dependency"),
        "failure must name 'order' as UnknownTestInput;\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
}

/// D-43 positive case: a correctly spelled `inputs` key ("orders") must
/// pass without any `UnknownTestInput` diagnostic.
#[test]
fn matched_inputs_key_passes() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("matched_key_ws");
    std::fs::create_dir_all(root.join("models")).unwrap();
    std::fs::create_dir_all(root.join("tests")).unwrap();
    std::fs::create_dir_all(root.join("target")).unwrap();
    std::fs::write(root.join("smelt.yml"), smelt_yml("matched_key_ws")).unwrap();

    std::fs::write(
        root.join("models/order_count.sql"),
        "SELECT COUNT(*) AS cnt FROM smelt.orders\n",
    )
    .unwrap();
    std::fs::write(root.join("models/orders.sql"), "SELECT id FROM smelt.raw\n").unwrap();

    let test_sql = "--- name: test_correct_key ---\n\
        materialization: test\n\
        test:\n  \
          model: order_count\n  \
          inputs:\n    \
            orders:\n      \
              - {id: 1}\n  \
          expect:\n    \
            - {cnt: 1}\n\
        ---\n";
    std::fs::write(root.join("tests/test_correct_key.sql"), test_sql).unwrap();

    let output = run_smelt_test(&root);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "correctly spelled inputs key must not fail;\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stdout.contains("PASS") || stdout.contains("pass") || stdout.contains("1 passed"),
        "test must PASS;\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
}

/// An `inputs` key using underscore-join ("silver_orders") must NOT resolve
/// the mock data — it should fall back to an empty CTE, causing the test to
/// fail.  This ensures we test the corrected direction (dot-key is canonical).
#[test]
fn inputs_underscore_key_does_not_resolve() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("underscore_key_ws");
    std::fs::create_dir_all(root.join("models/silver")).unwrap();
    std::fs::create_dir_all(root.join("tests")).unwrap();
    std::fs::create_dir_all(root.join("target")).unwrap();
    std::fs::write(root.join("smelt.yml"), smelt_yml("underscore_key_ws")).unwrap();

    std::fs::write(
        root.join("models/total.sql"),
        "SELECT SUM(amount) AS total FROM smelt.silver.orders\n",
    )
    .unwrap();
    std::fs::write(
        root.join("models/silver/orders.sql"),
        "SELECT amount FROM smelt.raw_data\n",
    )
    .unwrap();

    // Key uses underscore ("silver_orders") — incorrect form; must not find rows.
    let test_sql = "--- name: test_underscore_miss ---\n\
        materialization: test\n\
        test:\n  \
          model: total\n  \
          inputs:\n    \
            silver_orders:\n      \
              - {amount: 100}\n  \
          expect:\n    \
            - {total: 300}\n\
        ---\n";
    std::fs::write(root.join("tests/test_underscore_miss.sql"), test_sql).unwrap();

    let output = run_smelt_test(&root);
    let stdout = String::from_utf8_lossy(&output.stdout);

    // The test must FAIL: underscore key is not resolved, SUM returns NULL ≠ 300.
    assert!(
        stdout.contains("FAIL") || stdout.contains("fail") || !output.status.success(),
        "underscore key must not be resolved; test must fail;\nstdout:\n{stdout}"
    );
}
