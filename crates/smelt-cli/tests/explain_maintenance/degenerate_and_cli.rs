use std::path::Path;
use std::process::Command;

use crate::support::build_report_for;

/// A model whose SQL has an unqualified column ambiguous between two joined
/// sources cannot be resolved to a single provenance — the derivation falls
/// back to one column group spanning the whole model
/// (`crates/smelt-logical/tests/maintenance_grouping.rs::degenerate_collapse_is_surfaced`
/// is the pure-derivation-level test for this same shape). The CLI report
/// must call this out in plain language, not silently print an
/// indistinguishable single-group plan.
#[test]
fn degenerate_plan_visibly_reported() {
    let tmp = tempfile::TempDir::new().expect("create tempdir");

    let yml = "name: test_proj\n\
               version: 1\n\
               paths:\n  - models\n\
               targets:\n  dev:\n    type: duckdb\n    schema: main\n\
               default_materialization: view\n";
    std::fs::write(tmp.path().join("smelt.yml"), yml).unwrap();

    std::fs::create_dir_all(tmp.path().join("models/sources")).unwrap();
    std::fs::write(
        tmp.path().join("models/sources/orders.yml"),
        "description: orders\ncolumns:\n- name: order_id\n  type: INTEGER\n- name: order_date\n  type: DATE\n- name: amount\n  type: DECIMAL(10,2)\n- name: user_id\n  type: INTEGER\nmutation_profile:\n  kind: append_only\n",
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("models/sources/customers.yml"),
        "description: customers\ncolumns:\n- name: user_id\n  type: INTEGER\n- name: tier\n  type: VARCHAR\n",
    )
    .unwrap();

    let model_sql = "---\n\
                      materialization: table\n\
                      refresh: incremental\n\
                      grain: key\n\
                      ---\n\
                      SELECT o.order_id, MIN(o.order_date) AS order_date, MIN(amount) AS amount \
                      FROM smelt.sources.orders o \
                      JOIN smelt.sources.customers c ON c.user_id = o.user_id \
                      GROUP BY o.order_id\n";
    std::fs::write(tmp.path().join("models/ambiguous_join.sql"), model_sql).unwrap();

    let report = build_report_for(tmp.path(), "ambiguous_join")
        .expect("ambiguous_join is an incremental model with a maintenance plan");

    assert!(
        report.contains("single column group") || report.contains("could not distinguish"),
        "expected the report to call out the whole-model collapse in plain language: {report}"
    );
}

/// One full CLI-argument-parsing path: spawn the real `smelt` binary with a
/// positional model name and assert the maintenance-plan report (not the
/// whole-project graph JSON) is what prints.
#[test]
fn explain_model_arg_wired_through_real_cli() {
    let project_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/timeseries")
        .canonicalize()
        .expect("examples/timeseries exists");

    let output = Command::new(env!("CARGO_BIN_EXE_smelt"))
        .arg("explain")
        .arg("daily_events")
        .arg("--project-dir")
        .arg(&project_dir)
        .output()
        .expect("spawn smelt explain daily_events");

    assert!(
        output.status.success(),
        "smelt explain daily_events failed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Maintenance plan: daily_events"),
        "expected the maintenance-plan report, got: {stdout}"
    );
    assert!(
        !stdout.contains("Logical Graph:"),
        "explain <model> must not fall through to the whole-project graph output: {stdout}"
    );
}

/// `smelt explain` with no model argument keeps printing the whole-project
/// graph — unchanged by this phase.
#[test]
fn explain_without_model_arg_prints_whole_project_graph() {
    let project_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/timeseries")
        .canonicalize()
        .expect("examples/timeseries exists");

    let output = Command::new(env!("CARGO_BIN_EXE_smelt"))
        .arg("explain")
        .arg("--project-dir")
        .arg(&project_dir)
        .output()
        .expect("spawn smelt explain");

    assert!(
        output.status.success(),
        "smelt explain failed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Logical Graph:"),
        "expected the whole-project graph output, got: {stdout}"
    );
}
