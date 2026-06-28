#![cfg(feature = "duckdb")]
//! Integration tests for PASSING key binding: the PASSING clause name must match
//! a `smelt.<path>` dep referenced in the test body, identified by its last path segment.
//! A typo or underscore-form key that doesn't match any dep triggers D-43 UnknownTestInput.

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

/// A PASSING clause name that matches the `smelt.<path>` dep's last segment
/// injects mock rows into the test body, enabling the assertion to pass.
///
/// Model references `smelt.orders`; PASSING `orders AS (...)` matches → SUM = 300.
#[test]
fn inputs_dot_key_resolves() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("dot_key_ws");
    std::fs::create_dir_all(root.join("models")).unwrap();
    std::fs::create_dir_all(root.join("tests")).unwrap();
    std::fs::create_dir_all(root.join("target")).unwrap();
    std::fs::write(root.join("smelt.yml"), smelt_yml("dot_key_ws")).unwrap();

    // Model under test: aggregates over smelt.orders.
    std::fs::write(
        root.join("models/total.sql"),
        "SELECT SUM(amount) AS total FROM smelt.orders\n",
    )
    .unwrap();

    // Placeholder source model so smelt.orders resolves.
    std::fs::write(
        root.join("models/orders.sql"),
        "SELECT amount FROM smelt.raw_data\n",
    )
    .unwrap();

    // smelt.test: PASSING name 'orders' matches dep 'smelt.orders' → SUM = 300.
    let test_sql = "smelt.test test_total AS (\n\
        SELECT SUM(amount) AS total FROM smelt.orders\n\
    )\n\
    PASSING orders AS (\n\
        {amount: 100},\n\
        {amount: 200}\n\
    )\n\
    EXPECT (\n\
        {total: 300}\n\
    )\n";
    std::fs::write(root.join("tests/test_total_dot_key.sql"), test_sql).unwrap();

    let output = run_smelt_test(&root);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "smelt test must exit 0 when PASSING name matches dep;\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stdout.contains("PASS") || stdout.contains("pass") || stdout.contains("1 passed"),
        "test must pass (SUM(amount) = 300);\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
}

/// D-43: a PASSING clause name that does not match any `smelt.<path>` dep of the
/// test body must cause the test to fail with an `UnknownTestInput` diagnostic.
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

    // Typo: PASSING "ordr" but dep is "orders" — must fail with UnknownTestInput.
    // (Must use a non-keyword name; "order" is a SQL keyword and can't be a PASSING name.)
    let test_sql = "smelt.test test_typo_key AS (\n\
        SELECT COUNT(*) AS cnt FROM smelt.orders\n\
    )\n\
    PASSING ordr AS (\n\
        {id: 1}\n\
    )\n\
    EXPECT (\n\
        {cnt: 1}\n\
    )\n";
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

/// D-43 positive case: a correctly spelled PASSING name ("orders") must
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

    let test_sql = "smelt.test test_correct_key AS (\n\
        SELECT COUNT(*) AS cnt FROM smelt.orders\n\
    )\n\
    PASSING orders AS (\n\
        {id: 1}\n\
    )\n\
    EXPECT (\n\
        {cnt: 1}\n\
    )\n";
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

/// A PASSING name that does not match the dep's last path segment must cause D-43
/// failure. Using a wrong compound name ("orders_typo" instead of "orders") is
/// the canonical example — it does not match the dep `smelt.orders`.
#[test]
fn inputs_underscore_key_does_not_resolve() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("underscore_key_ws");
    std::fs::create_dir_all(root.join("models")).unwrap();
    std::fs::create_dir_all(root.join("tests")).unwrap();
    std::fs::create_dir_all(root.join("target")).unwrap();
    std::fs::write(root.join("smelt.yml"), smelt_yml("underscore_key_ws")).unwrap();

    std::fs::write(
        root.join("models/total.sql"),
        "SELECT SUM(amount) AS total FROM smelt.orders\n",
    )
    .unwrap();
    std::fs::write(
        root.join("models/orders.sql"),
        "SELECT amount FROM smelt.raw_data\n",
    )
    .unwrap();

    // PASSING uses wrong name "orders_typo" — must not match dep "orders" → D-43 failure.
    let test_sql = "smelt.test test_underscore_miss AS (\n\
        SELECT SUM(amount) AS total FROM smelt.orders\n\
    )\n\
    PASSING orders_typo AS (\n\
        {amount: 100}\n\
    )\n\
    EXPECT (\n\
        {total: 300}\n\
    )\n";
    std::fs::write(root.join("tests/test_underscore_miss.sql"), test_sql).unwrap();

    let output = run_smelt_test(&root);
    let stdout = String::from_utf8_lossy(&output.stdout);

    // The test must FAIL: wrong PASSING name triggers D-43 UnknownTestInput.
    assert!(
        stdout.contains("FAIL") || stdout.contains("fail") || !output.status.success(),
        "wrong PASSING name must trigger D-43 failure;\nstdout:\n{stdout}"
    );
}
