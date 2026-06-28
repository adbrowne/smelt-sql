#![cfg(feature = "duckdb")]
//! Integration tests for Phase 5: smelt.test AST-driven test declarations.
//!
//! These verify that the new `smelt.test` grammar works end-to-end through
//! `smelt test`: PASSING mocks, EXPECT comparison, #cte targeting,
//! UnknownTestInput/UnknownTestCte diagnostics, and check_order from frontmatter.

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

/// Minimal smelt.yml for a temp workspace.
fn smelt_yml(name: &str) -> String {
    format!(
        "name: {name}\nversion: 1\npaths:\n  - models\n\
         targets:\n  dev:\n    type: duckdb\n    database: target/dev.duckdb\n    schema: main\n\
         default_materialization: view\n"
    )
}

/// A `smelt.test` full-query test passes when EXPECT matches the query result,
/// and fails when EXPECT does not match.
#[test]
fn smelt_test_full_query_passes() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("new_syntax_fq");
    std::fs::create_dir_all(root.join("models")).unwrap();
    std::fs::create_dir_all(root.join("target")).unwrap();
    std::fs::write(root.join("smelt.yml"), smelt_yml("new_syntax_fq")).unwrap();

    // Placeholder raw_orders model so the resolver can find it.
    std::fs::write(
        root.join("models/raw_orders.sql"),
        "SELECT order_id, amount, status, order_date FROM (VALUES (0,0,'x','2024-01-01')) AS t(order_id,amount,status,order_date)\n",
    )
    .unwrap();

    // New-syntax test file: full-query test using smelt.test.
    // PASSING mocks smelt.raw_orders; EXPECT must match the filtered aggregate.
    std::fs::write(
        root.join("models/test_fq.sql"),
        "smelt.test check_completed AS (\n\
             SELECT order_date, COUNT(*) AS cnt, SUM(amount) AS total\n\
             FROM smelt.raw_orders\n\
             WHERE status = 'completed'\n\
             GROUP BY order_date\n\
         )\n\
         PASSING raw_orders AS (\n\
             {order_id: 1, amount: 100, status: 'completed', order_date: '2024-01-15'},\n\
             {order_id: 2, amount: 200, status: 'completed', order_date: '2024-01-15'},\n\
             {order_id: 3, amount: 75,  status: 'cancelled', order_date: '2024-01-16'}\n\
         )\n\
         EXPECT (\n\
             {order_date: '2024-01-15', cnt: 2, total: 300}\n\
         )\n",
    )
    .unwrap();

    let output = run_smelt_test(&root);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "new-syntax full-query smelt.test with correct EXPECT must pass;\n\
         stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stdout.contains("PASS") || stdout.contains("1 passed"),
        "must report PASS;\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
}

/// A `smelt.test` full-query test FAILS when EXPECT contains wrong values.
#[test]
fn smelt_test_full_query_wrong_expect_fails() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("new_syntax_fq_fail");
    std::fs::create_dir_all(root.join("models")).unwrap();
    std::fs::create_dir_all(root.join("target")).unwrap();
    std::fs::write(root.join("smelt.yml"), smelt_yml("new_syntax_fq_fail")).unwrap();

    std::fs::write(
        root.join("models/raw_orders.sql"),
        "SELECT order_id, amount, status FROM (VALUES (0,0,'x')) AS t(order_id,amount,status)\n",
    )
    .unwrap();

    // Same test but EXPECT says total=999 (wrong — actual is 300).
    std::fs::write(
        root.join("models/test_fq_fail.sql"),
        "smelt.test check_wrong AS (\n\
             SELECT SUM(amount) AS total FROM smelt.raw_orders WHERE status = 'completed'\n\
         )\n\
         PASSING raw_orders AS (\n\
             {order_id: 1, amount: 100, status: 'completed'},\n\
             {order_id: 2, amount: 200, status: 'completed'}\n\
         )\n\
         EXPECT (\n\
             {total: 999}\n\
         )\n",
    )
    .unwrap();

    let output = run_smelt_test(&root);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        !output.status.success(),
        "wrong EXPECT must cause smelt test to exit non-zero;\n\
         stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stdout.contains("FAIL") || stdout.contains("0 passed"),
        "must report FAIL;\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
}

/// A `smelt.test` CTE-level test (`#cte` suffix) runs the internal CTE chain
/// as-written, mocking only external `smelt.<path>` dependencies.
///
/// Model has two CTEs: `cleaned` reads from `smelt.raw_orders`; `agg` sums cleaned.
/// Test targets `agg` via `smelt.model_with_ctes#agg`. The internal `cleaned` runs
/// as-written; only `raw_orders` is mocked via PASSING.
#[test]
fn smelt_test_cte_isolation() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("new_syntax_cte");
    std::fs::create_dir_all(root.join("models")).unwrap();
    std::fs::create_dir_all(root.join("target")).unwrap();
    std::fs::write(root.join("smelt.yml"), smelt_yml("new_syntax_cte")).unwrap();

    // Model with two CTEs: cleaned filters; agg sums.
    std::fs::write(
        root.join("models/order_summary.sql"),
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
        "SELECT amount FROM (VALUES (0)) AS t(amount)\n",
    )
    .unwrap();

    // New-syntax CTE-level test: target `agg` inside `order_summary`.
    // PASSING mocks the external dep `raw_orders`; internal `cleaned` runs as-written.
    std::fs::write(
        root.join("models/test_cte.sql"),
        "smelt.test check_agg AS (\n\
             SELECT total FROM smelt.order_summary#agg\n\
         )\n\
         PASSING raw_orders AS (\n\
             {amount: 100},\n\
             {amount: 200},\n\
             {amount: -50}\n\
         )\n\
         EXPECT (\n\
             {total: 300}\n\
         )\n",
    )
    .unwrap();

    let output = run_smelt_test(&root);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "CTE-level smelt.test must pass (internal cleaned runs as-written, total=300);\n\
         stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stdout.contains("PASS") || stdout.contains("1 passed"),
        "must report PASS;\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
}

/// A PASSING clause name that does not match any `smelt.<path>` ref in the body
/// produces a compilation failure (UnknownTestInput).
#[test]
fn passing_unknown_dep_diagnoses() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("new_syntax_unknown_dep");
    std::fs::create_dir_all(root.join("models")).unwrap();
    std::fs::create_dir_all(root.join("target")).unwrap();
    std::fs::write(root.join("smelt.yml"), smelt_yml("new_syntax_unknown_dep")).unwrap();

    std::fs::write(
        root.join("models/raw_orders.sql"),
        "SELECT amount FROM (VALUES (0)) AS t(amount)\n",
    )
    .unwrap();

    // PASSING uses 'nonexistent_table' but body refs smelt.raw_orders.
    // 'nonexistent_table' is not a dep of the body → UnknownTestInput.
    std::fs::write(
        root.join("models/test_unknown_dep.sql"),
        "smelt.test check_unknown AS (\n\
             SELECT SUM(amount) AS total FROM smelt.raw_orders\n\
         )\n\
         PASSING nonexistent_table AS (\n\
             {amount: 100}\n\
         )\n\
         EXPECT (\n\
             {total: 100}\n\
         )\n",
    )
    .unwrap();

    let output = run_smelt_test(&root);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        !output.status.success(),
        "PASSING with unknown dep must fail;\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stdout.contains("nonexistent_table") || stderr.contains("nonexistent_table"),
        "failure must mention the unknown PASSING name;\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
}

/// A `#cte` reference to a CTE that does not exist in the target model
/// produces a compilation failure (UnknownTestCte).
#[test]
fn hash_unknown_cte_diagnoses() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("new_syntax_unknown_cte");
    std::fs::create_dir_all(root.join("models")).unwrap();
    std::fs::create_dir_all(root.join("target")).unwrap();
    std::fs::write(root.join("smelt.yml"), smelt_yml("new_syntax_unknown_cte")).unwrap();

    // Model has only a 'real_cte'; test body references 'nonexistent_cte'.
    std::fs::write(
        root.join("models/simple_model.sql"),
        "WITH real_cte AS (\n\
             SELECT 1 AS x\n\
         )\n\
         SELECT * FROM real_cte\n",
    )
    .unwrap();

    std::fs::write(
        root.join("models/test_bad_cte.sql"),
        "smelt.test check_missing_cte AS (\n\
             SELECT x FROM smelt.simple_model#nonexistent_cte\n\
         )\n\
         EXPECT (\n\
             {x: 1}\n\
         )\n",
    )
    .unwrap();

    let output = run_smelt_test(&root);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        !output.status.success(),
        "#nonexistent_cte must cause smelt test to fail;\n\
         stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    // Error must mention the missing CTE name or 'not found'.
    assert!(
        stdout.contains("nonexistent_cte")
            || stderr.contains("nonexistent_cte")
            || stdout.contains("not found")
            || stderr.contains("not found"),
        "failure must mention 'nonexistent_cte' or 'not found';\n\
         stdout:\n{stdout}\nstderr:\n{stderr}"
    );
}

/// `smelt test --select <model>` includes new-syntax tests whose body references that
/// model, and excludes them when a different model is selected.
#[test]
fn smelt_test_select_by_subject_model() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("new_syntax_select");
    std::fs::create_dir_all(root.join("models")).unwrap();
    std::fs::create_dir_all(root.join("target")).unwrap();
    std::fs::write(root.join("smelt.yml"), smelt_yml("new_syntax_select")).unwrap();

    // Subject model: the one the smelt.test targets.
    std::fs::write(
        root.join("models/subject_model.sql"),
        "SELECT x FROM (VALUES (42)) AS t(x)\n",
    )
    .unwrap();

    // Unrelated model: NOT referenced by the test.
    std::fs::write(
        root.join("models/other_model.sql"),
        "SELECT y FROM (VALUES (99)) AS t(y)\n",
    )
    .unwrap();

    // New-syntax test file that references smelt.subject_model.
    std::fs::write(
        root.join("models/test_subject.sql"),
        "smelt.test check_subject AS (\n\
             SELECT x FROM smelt.subject_model\n\
         )\n\
         PASSING subject_model AS (\n\
             {x: 42}\n\
         )\n\
         EXPECT (\n\
             {x: 42}\n\
         )\n",
    )
    .unwrap();

    // Case 1: --select subject_model → test should run and pass.
    let output_match = Command::new(smelt_bin())
        .arg("test")
        .args(["--project-dir", root.to_str().unwrap()])
        .args(["--select", "subject_model"])
        .env_remove("RUST_LOG")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt test`: {e}"));
    let stdout_match = String::from_utf8_lossy(&output_match.stdout);
    let stderr_match = String::from_utf8_lossy(&output_match.stderr);

    assert!(
        output_match.status.success(),
        "--select subject_model must run the test and pass;\n\
         stdout:\n{stdout_match}\nstderr:\n{stderr_match}"
    );
    assert!(
        stdout_match.contains("PASS") || stdout_match.contains("1 passed"),
        "--select subject_model must report PASS;\n\
         stdout:\n{stdout_match}\nstderr:\n{stderr_match}"
    );

    // Case 2: --select other_model → test should be excluded.
    let output_no_match = Command::new(smelt_bin())
        .arg("test")
        .args(["--project-dir", root.to_str().unwrap()])
        .args(["--select", "other_model"])
        .env_remove("RUST_LOG")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt test`: {e}"));
    let stdout_no = String::from_utf8_lossy(&output_no_match.stdout);
    let stderr_no = String::from_utf8_lossy(&output_no_match.stderr);

    assert!(
        output_no_match.status.success(),
        "--select other_model must exit 0 (no tests matched);\n\
         stdout:\n{stdout_no}\nstderr:\n{stderr_no}"
    );
    // Either "No tests matched" or "No tests found" is acceptable — the important
    // thing is that no PASS/FAIL was reported (the test was excluded).
    assert!(
        !stdout_no.contains("PASS") && !stdout_no.contains("FAIL"),
        "--select other_model must NOT run the test;\n\
         stdout:\n{stdout_no}\nstderr:\n{stderr_no}"
    );
}

/// A `smelt.test #cte` declaration with omitted PASSING columns triggers the
/// property-based test loop (`cases` iterations) instead of a one-shot test.
///
/// The model CTE reads both `amount` and `flag` from the external dep.  The PASS
/// clause provides only `amount`; `flag` is omitted.  Because SUM(amount) does not
/// depend on `flag`, all random iterations should produce `total = 100` and pass.
#[test]
fn smelt_test_cte_property_dispatch_with_omitted_column() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("new_syntax_prop");
    std::fs::create_dir_all(root.join("models")).unwrap();
    std::fs::create_dir_all(root.join("target")).unwrap();
    std::fs::write(root.join("smelt.yml"), smelt_yml("new_syntax_prop")).unwrap();

    // Placeholder model so the resolver can find source_data.
    std::fs::write(
        root.join("models/source_data.sql"),
        "SELECT amount, flag FROM (VALUES (0, 'x')) AS t(amount, flag)\n",
    )
    .unwrap();

    // Model with a CTE that reads both `amount` and `flag` from external dep.
    std::fs::write(
        root.join("models/model_with_cte.sql"),
        "WITH result AS (\n\
             SELECT SUM(amount) AS total, MAX(flag) AS max_flag\n\
             FROM smelt.source_data\n\
         )\n\
         SELECT * FROM result\n",
    )
    .unwrap();

    // Test file: cases: 3, PASSING provides only `amount` (not `flag`).
    // This triggers the property loop with 3 iterations.
    // SUM(amount) = 100 regardless of what random value flag gets.
    std::fs::write(
        root.join("models/test_prop.sql"),
        "---\ntest:\n  cases: 3\n---\n\
         smelt.test check_total_invariant AS (\n\
             SELECT total FROM smelt.model_with_cte#result\n\
         )\n\
         PASSING source_data AS (\n\
             {amount: 100}\n\
         )\n\
         EXPECT (\n\
             {total: 100}\n\
         )\n",
    )
    .unwrap();

    let output = run_smelt_test(&root);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "property-dispatch #cte test with omitted column must pass \
         (SUM(amount) is invariant to `flag`);\n\
         stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    // Either PASS (show-all) or the summary "1 passed" must appear.
    assert!(
        stdout.contains("PASS") || stdout.contains("passed"),
        "must report PASS or 'N passed';\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
}

/// `check_order: true` in the file frontmatter `test:` block enforces positional
/// row comparison.  When EXPECT rows are in a different order than actual, the
/// test fails.
///
/// The test body produces two rows in a deterministic order
/// (ORDER BY x ASC → x=1 first, x=2 second). EXPECT lists x=2 first — this
/// mismatch is caught only when check_order: true is in effect.
#[test]
fn check_order_and_cases_from_frontmatter() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("new_syntax_check_order");
    std::fs::create_dir_all(root.join("models")).unwrap();
    std::fs::create_dir_all(root.join("target")).unwrap();
    std::fs::write(root.join("smelt.yml"), smelt_yml("new_syntax_check_order")).unwrap();

    // Test file with check_order: true in frontmatter.
    // The body SELECT produces rows [x=1, x=2] via ORDER BY x ASC.
    // EXPECT lists [x=2, x=1] — wrong order — should fail under check_order: true.
    std::fs::write(
        root.join("models/test_order.sql"),
        "---\ntest:\n  check_order: true\n---\n\
         smelt.test check_row_order AS (\n\
             SELECT x FROM (VALUES (1), (2)) AS t(x) ORDER BY x ASC\n\
         )\n\
         EXPECT (\n\
             {x: 2},\n\
             {x: 1}\n\
         )\n",
    )
    .unwrap();

    let output = run_smelt_test(&root);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    // With check_order: true and wrong order, test must fail.
    assert!(
        !output.status.success(),
        "check_order: true with wrong row order must fail;\n\
         stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stdout.contains("FAIL") || stdout.contains("0 passed"),
        "must report FAIL;\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );

    // ── Positive case: correct order passes ──────────────────────────────────
    let root2 = tmp.path().join("new_syntax_check_order_pass");
    std::fs::create_dir_all(root2.join("models")).unwrap();
    std::fs::create_dir_all(root2.join("target")).unwrap();
    std::fs::write(
        root2.join("smelt.yml"),
        smelt_yml("new_syntax_check_order_pass"),
    )
    .unwrap();

    std::fs::write(
        root2.join("models/test_order_pass.sql"),
        "---\ntest:\n  check_order: true\n---\n\
         smelt.test check_row_order_correct AS (\n\
             SELECT x FROM (VALUES (1), (2)) AS t(x) ORDER BY x ASC\n\
         )\n\
         EXPECT (\n\
             {x: 1},\n\
             {x: 2}\n\
         )\n",
    )
    .unwrap();

    let output2 = run_smelt_test(&root2);
    let stdout2 = String::from_utf8_lossy(&output2.stdout);
    let stderr2 = String::from_utf8_lossy(&output2.stderr);

    assert!(
        output2.status.success(),
        "check_order: true with correct order must pass;\n\
         stdout:\n{stdout2}\nstderr:\n{stderr2}"
    );
    assert!(
        stdout2.contains("PASS") || stdout2.contains("1 passed"),
        "must report PASS;\nstdout:\n{stdout2}\nstderr:\n{stderr2}"
    );
}
