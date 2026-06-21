#![cfg(feature = "duckdb")]
//! Integration tests for D-45: CTE-level tests mock external `smelt.<path>` deps,
//! not internal CTEs. The internal CTE chain executes as-written.

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

/// A CTE-level test targeting `agg` must mock the EXTERNAL dep `smelt.raw_orders`,
/// not the internal CTE `cleaned`. The internal CTE `cleaned` runs as-written,
/// reading from the mocked `raw_orders` CTE. Result must equal SUM(100 + 200) = 300.
#[test]
fn cte_test_mocks_external_deps_not_internal_ctes() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("cte_ext_deps_ws");
    std::fs::create_dir_all(root.join("models")).unwrap();
    std::fs::create_dir_all(root.join("tests")).unwrap();
    std::fs::create_dir_all(root.join("target")).unwrap();
    std::fs::write(root.join("smelt.yml"), smelt_yml("cte_ext_deps_ws")).unwrap();

    // Model with two-CTE chain: cleaned → agg; cleaned reads from smelt.raw_orders.
    std::fs::write(
        root.join("models/orders_agg.sql"),
        "WITH cleaned AS (\n\
             SELECT amount FROM smelt.raw_orders WHERE amount > 0\n\
         ),\n\
         agg AS (\n\
             SELECT SUM(amount) AS total FROM cleaned\n\
         )\n\
         SELECT * FROM agg\n",
    )
    .unwrap();

    // Placeholder so smelt.raw_orders resolves in discovery.
    std::fs::write(
        root.join("models/raw_orders.sql"),
        "SELECT amount FROM smelt.source\n",
    )
    .unwrap();

    // Placeholder so smelt.source resolves in discovery.
    std::fs::write(
        root.join("models/source.sql"),
        "SELECT amount FROM (VALUES (0)) AS t(amount)\n",
    )
    .unwrap();

    // CTE-level test targeting 'agg'; inputs provides the EXTERNAL dep 'raw_orders'.
    // cleaned must run as-written (reading from the mocked raw_orders CTE, not mocked itself).
    let test_sql = "--- name: test_agg_cte ---\n\
        materialization: test\n\
        test:\n  \
          model: orders_agg\n  \
          target_cte: agg\n  \
          inputs:\n    \
            raw_orders:\n      \
              - {amount: 100}\n      \
              - {amount: 200}\n  \
          expect:\n    \
            - {total: 300}\n\
        ---\n";
    std::fs::write(root.join("tests/test_agg_cte.sql"), test_sql).unwrap();

    let output = run_smelt_test(&root);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "CTE-level test must pass (total=300, cleaned runs as-written);\
        \nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stdout.contains("PASS") || stdout.contains("pass") || stdout.contains("1 passed"),
        "CTE test must report PASS;\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
}

/// An `inputs` key that is an internal CTE name (not a `smelt.<path>` dep) must
/// be silently ignored. The test still runs against the actual external dep when
/// provided via the correct dot-key and must produce the correct result.
#[test]
fn cte_test_inputs_keys_are_model_deps_not_cte_names() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("cte_internal_key_ws");
    std::fs::create_dir_all(root.join("models")).unwrap();
    std::fs::create_dir_all(root.join("tests")).unwrap();
    std::fs::create_dir_all(root.join("target")).unwrap();
    std::fs::write(root.join("smelt.yml"), smelt_yml("cte_internal_key_ws")).unwrap();

    std::fs::write(
        root.join("models/orders_agg.sql"),
        "WITH cleaned AS (\n\
             SELECT amount FROM smelt.raw_orders WHERE amount > 0\n\
         ),\n\
         agg AS (\n\
             SELECT SUM(amount) AS total FROM cleaned\n\
         )\n\
         SELECT * FROM agg\n",
    )
    .unwrap();
    std::fs::write(
        root.join("models/raw_orders.sql"),
        "SELECT amount FROM smelt.source\n",
    )
    .unwrap();
    std::fs::write(
        root.join("models/source.sql"),
        "SELECT amount FROM (VALUES (0)) AS t(amount)\n",
    )
    .unwrap();

    // inputs includes "cleaned" (an internal CTE name, must be ignored) AND "raw_orders"
    // (the actual external dep). Test must pass with total=300 — "cleaned" key is ignored.
    let test_sql = "--- name: test_internal_key_ignored ---\n\
        materialization: test\n\
        test:\n  \
          model: orders_agg\n  \
          target_cte: agg\n  \
          inputs:\n    \
            cleaned:\n      \
              - {amount: 999}\n    \
            raw_orders:\n      \
              - {amount: 100}\n      \
              - {amount: 200}\n  \
          expect:\n    \
            - {total: 300}\n\
        ---\n";
    std::fs::write(root.join("tests/test_internal_key_ignored.sql"), test_sql).unwrap();

    let output = run_smelt_test(&root);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    // cleaned runs as-written (using raw_orders mock = 100+200 = 300), total=300.
    // The "cleaned" input key is ignored (internal CTE, not a smelt dep).
    assert!(
        output.status.success(),
        "CTE test with internal CTE name in inputs must pass (total=300);\
        \nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stdout.contains("PASS") || stdout.contains("pass") || stdout.contains("1 passed"),
        "test must report PASS;\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
}

/// A CTE-level test targeting a CTE whose chain has a transitive external dep
/// must correctly resolve that dep through the chain.
///
/// Model: a reads from `smelt.source_data`; b sums a; c doubles b's total.
/// Targeting `c` with `inputs: {source_data: [{amount: 50}, {amount: 100}]}`
/// must resolve `source_data` through the a→b→c chain and yield doubled = 300.
#[test]
fn cte_test_transitive_external_deps_reachable() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("cte_transitive_ws");
    std::fs::create_dir_all(root.join("models")).unwrap();
    std::fs::create_dir_all(root.join("tests")).unwrap();
    std::fs::create_dir_all(root.join("target")).unwrap();
    std::fs::write(root.join("smelt.yml"), smelt_yml("cte_transitive_ws")).unwrap();

    // Chain: a reads smelt.source_data (external); b sums a; c doubles b.
    std::fs::write(
        root.join("models/chain.sql"),
        "WITH a AS (\n\
             SELECT amount FROM smelt.source_data\n\
         ),\n\
         b AS (\n\
             SELECT SUM(amount) AS total FROM a\n\
         ),\n\
         c AS (\n\
             SELECT total * 2 AS doubled FROM b\n\
         )\n\
         SELECT * FROM c\n",
    )
    .unwrap();
    std::fs::write(
        root.join("models/source_data.sql"),
        "SELECT amount FROM smelt.raw\n",
    )
    .unwrap();
    std::fs::write(
        root.join("models/raw.sql"),
        "SELECT amount FROM (VALUES (0)) AS t(amount)\n",
    )
    .unwrap();

    // target_cte: c; inputs provides the transitive external dep 'source_data'.
    // Chain c→b→a→source_data(external): source_data mock yields SUM=150, doubled=300.
    let test_sql = "--- name: test_chain_cte ---\n\
        materialization: test\n\
        test:\n  \
          model: chain\n  \
          target_cte: c\n  \
          inputs:\n    \
            source_data:\n      \
              - {amount: 50}\n      \
              - {amount: 100}\n  \
          expect:\n    \
            - {doubled: 300}\n\
        ---\n";
    std::fs::write(root.join("tests/test_chain_cte.sql"), test_sql).unwrap();

    let output = run_smelt_test(&root);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "transitive CTE chain test must pass (doubled=300);\
        \nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stdout.contains("PASS") || stdout.contains("pass") || stdout.contains("1 passed"),
        "CTE transitive test must report PASS;\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
}
