//! Phase 39 — `smelt build --show-plan` integration tests.
//!
//! Tests 1, 2, 5 invoke the compiled `smelt` binary against the
//! `examples/functions_demo` workspace. Tests 3 and 4 exercise the rule
//! list and formatter directly through the public planner API; the
//! existing `logical_plan` Salsa query does not yet construct WHERE
//! filters or LeftJoin nodes from SQL, so end-to-end binary tests for
//! pushdown and join elimination are deferred to the Phase 40+ plan that
//! enriches the logical-plan builder.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use smelt_planner::logical::{Cardinality, FunctionProperties, LogicalNode, Provenance};
use smelt_planner::logical_plan_rules::{apply_rules_to_fixed_point, show_plan_rules};
use smelt_planner::plan_printer::format_plan;

fn smelt_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_smelt"))
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn run_show_plan(model_rel_path: &str) -> std::process::Output {
    let model = workspace_root().join(model_rel_path);
    Command::new(smelt_bin())
        .args(["build", model.to_str().unwrap(), "--show-plan"])
        .env_remove("RUST_LOG")
        .output()
        .expect("failed to spawn smelt build --show-plan")
}

#[test]
fn cli_show_plan_prints_logical_plan() {
    let output = run_show_plan("examples/functions_demo/models/uses_safe_divide.sql");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "smelt build --show-plan must exit 0; stderr:\n{}\nstdout:\n{}",
        String::from_utf8_lossy(&output.stderr),
        stdout
    );
    assert!(
        stdout.contains("safe_divide"),
        "expected output to mention safe_divide; got:\n{stdout}"
    );
    // The plan root is a Select. The transparent safe_divide call gets
    // expanded; the surviving fn-id reference therefore lands on an
    // ExpandedCall, but the printed output still names it.
    assert!(
        stdout.contains("ExpandedCall") || stdout.contains("FunctionCall"),
        "expected a FunctionCall or ExpandedCall node; got:\n{stdout}"
    );
}

#[test]
fn cli_show_plan_runs_expand_rule() {
    let output = run_show_plan("examples/functions_demo/models/uses_safe_divide.sql");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success());
    assert!(
        stdout.contains("ExpandedCall"),
        "expected ExpandTransparentFunctionCalls to fire (safe_divide is transparent); got:\n{stdout}"
    );
}

// Test 3: pushdown wiring. Constructed plan rather than driven through
// `logical_plan`, which does not yet build filters from SQL WHERE clauses.
#[test]
fn cli_show_plan_runs_pushdown_when_eligible() {
    let call = Arc::new(LogicalNode::FunctionCall {
        fn_id: "demo".to_string(),
        args: vec![],
        transparent: true,
        provenance: Provenance::Declared(vec![("y".to_string(), vec!["x.y".to_string()])]),
        properties: FunctionProperties {
            deterministic: true,
            ..FunctionProperties::default()
        },
        pushed_filter: None,
        body: None,
    });
    let pred = Arc::new(LogicalNode::Literal(smelt_types::DataType::Boolean));
    let plan = Arc::new(LogicalNode::Select {
        projections: vec!["y".to_string()],
        from: Some(call),
        filter: Some(pred),
    });

    let optimised = apply_rules_to_fixed_point(plan, &show_plan_rules());
    let printed = format_plan(&optimised);

    assert!(
        printed.contains("pushed_filter=Some(_)"),
        "expected pushdown to fire when conditions hold; got:\n{printed}"
    );
}

// Test 4: join-elimination wiring. As with test 3, constructed plan —
// `logical_plan` does not yet emit LeftJoin nodes from SQL.
#[test]
fn cli_show_plan_eliminates_unused_join() {
    let lhs = Arc::new(LogicalNode::TableRef {
        name: "orders".to_string(),
    });
    let rhs = Arc::new(LogicalNode::TableRef {
        name: "dim_customer".to_string(),
    });
    let join = Arc::new(LogicalNode::LeftJoin {
        lhs,
        rhs,
        join_columns: vec!["customer_id".to_string()],
        cardinality: Cardinality::OneToOne,
        output_columns: vec!["customer_name".to_string(), "customer_tier".to_string()],
    });
    let plan = Arc::new(LogicalNode::Select {
        projections: vec!["order_id".to_string(), "total".to_string()],
        from: Some(join),
        filter: None,
    });

    let optimised = apply_rules_to_fixed_point(plan, &show_plan_rules());
    let printed = format_plan(&optimised);

    assert!(
        !printed.contains("LeftJoin"),
        "expected LeftJoin to be elided when no RHS column is consumed; got:\n{printed}"
    );
    assert!(
        printed.contains("TableRef name=\"orders\""),
        "expected lhs TableRef to remain; got:\n{printed}"
    );
}

// Test 5: default build path is unchanged. Verifying byte-for-byte
// equality of an entire build run is brittle; instead pin the contract
// the user observes — the show-plan banner must NOT appear unless
// --show-plan is passed.
#[cfg(feature = "duckdb")]
#[test]
fn default_compile_unchanged() {
    use std::process::Stdio;
    use tempfile::TempDir;

    let tmp = TempDir::new().unwrap();
    let project = tmp.path().join("default_unchanged_ws");
    std::fs::create_dir_all(project.join("models")).unwrap();
    std::fs::create_dir_all(project.join("seeds")).unwrap();
    std::fs::create_dir_all(project.join("target")).unwrap();
    std::fs::write(
        project.join("smelt.yml"),
        "name: default_unchanged_ws\n\
         version: 1\n\
         model_paths:\n  - models\n\
         seed_paths:\n  - seeds\n\
         targets:\n  dev:\n    type: duckdb\n    database: target/dev.duckdb\n    schema: main\n\
         default_materialization: view\n",
    )
    .unwrap();
    std::fs::write(project.join("models/m.sql"), "SELECT 1 AS x\n").unwrap();

    let output = Command::new(smelt_bin())
        .args(["build", "--project-dir", project.to_str().unwrap()])
        .env_remove("RUST_LOG")
        .stdin(Stdio::null())
        .output()
        .expect("failed to spawn smelt build");

    assert!(
        output.status.success(),
        "default build must still succeed; stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stdout.contains("Select projections=") && !stderr.contains("Select projections="),
        "default build must not emit show-plan output;\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        !stdout.contains("ExpandedCall") && !stderr.contains("ExpandedCall"),
        "default build must not emit show-plan output;\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    // Existing one-line summary contract from build_summary_visibility.rs.
    assert!(
        stderr.contains("smelt: built 1 model(s)"),
        "default build must still emit success summary; stderr:\n{stderr}"
    );
}
