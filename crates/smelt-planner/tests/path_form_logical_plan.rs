//! Phase 2c — path-form consumers: logical plan tests.
//!
//! Tests 1 & 2 verify that `logical_plan` understands the unified
//! `SmeltPathCall` surface in addition to the legacy `SmeltFnCall` nodes.
//!
//! Test 1: `smelt.functions.session_rollup(...)` (path form) produces a
//!   `LogicalNode::FunctionCall { transparent: true, body: Some(_) }` node
//!   when the callee is a `smelt.define`.
//!
//! Test 2: `smelt.functions.my_agg(...)` (path form) calling a `smelt.extern`
//!   produces a `FunctionCall { transparent: false, body: None }` node.

use std::sync::Arc;

use smelt_db::Database;
use smelt_planner::logical::{LogicalNode, Provenance};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn setup_multi_file(
    project_root: &std::path::Path,
    files: &[(&str, &str)],
) -> (Database, smelt_db::Workspace) {
    let mut db = Database::default();
    let mut file_handles = Vec::new();
    for (rel_path, text) in files {
        let full_path = project_root.join(rel_path);
        if let Some(parent) = full_path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&full_path, text).unwrap();
        let fh = db.set_source_file(full_path, text.to_string(), project_root.to_path_buf());
        file_handles.push(fh);
    }
    let project = db.set_project_input(project_root.to_path_buf(), String::new());
    db.set_workspace(file_handles, vec![project]);
    let ws = db.workspace();
    (db, ws)
}

// ---------------------------------------------------------------------------
// Traverse helpers
// ---------------------------------------------------------------------------

fn first_function_call(node: &Arc<LogicalNode>) -> Option<FcFields> {
    match node.as_ref() {
        LogicalNode::FunctionCall {
            fn_id,
            transparent,
            body,
            ..
        } => Some(FcFields {
            fn_id: fn_id.clone(),
            transparent: *transparent,
            has_body: body.is_some(),
        }),
        LogicalNode::Tagged { inner, .. } => first_function_call(inner),
        LogicalNode::SpliceList(items) => items.iter().find_map(first_function_call),
        LogicalNode::Raw { .. } => None,
        LogicalNode::Select { from, filter, .. } => from
            .as_ref()
            .and_then(first_function_call)
            .or_else(|| filter.as_ref().and_then(first_function_call)),
        LogicalNode::Cast { inner, .. } => first_function_call(inner),
        LogicalNode::TableRef { .. }
        | LogicalNode::Literal(_)
        | LogicalNode::ExpandedCall { .. } => None,
        LogicalNode::LeftJoin { lhs, rhs, .. } => {
            first_function_call(lhs).or_else(|| first_function_call(rhs))
        }
    }
}

#[derive(Debug)]
#[allow(dead_code)]
struct FcFields {
    fn_id: String,
    transparent: bool,
    has_body: bool,
}

// ---------------------------------------------------------------------------
// Test 1: path-form call → FunctionCall node (transparent = true for defines)
// ---------------------------------------------------------------------------

#[test]
fn function_call_path_resolves_to_logical_function_node() {
    let dir = tempfile::tempdir().unwrap();
    let project_root = dir.path();

    let (db, ws) = setup_multi_file(
        project_root,
        &[
            (
                "functions/session_rollup.sql",
                "smelt.define session_rollup(events: TableExpr) -> TableExpr AS (\n  SELECT * FROM events\n)\n",
            ),
            (
                "models/report.sql",
                "SELECT * FROM smelt.functions.session_rollup(smelt.models.events)\n",
            ),
            (
                "models/events.sql",
                "SELECT 1 AS user_id, CURRENT_TIMESTAMP AS ts\n",
            ),
        ],
    );

    let report_path = project_root.join("models").join("report.sql");
    let report_file = db.source_file(&report_path).expect("report.sql registered");

    let plan = smelt_db::logical_plan(&db, ws, report_file)
        .expect("plan should be Some for a valid model");

    let call = first_function_call(&plan)
        .expect("expected a FunctionCall node in plan for path-form call");
    assert!(
        call.transparent,
        "smelt.define session_rollup via path form should be transparent=true; got: {call:?}"
    );
    assert_eq!(
        call.fn_id, "session_rollup",
        "fn_id should be the last segment; got: {}",
        call.fn_id
    );

    // Also verify: a plain path-ref model (not a call) should produce a
    // Select with no FunctionCall from-node.
    let (db2, ws2) = setup_multi_file(
        project_root,
        &[
            ("models/users.sql", "SELECT 1 AS id, 'alice' AS name\n"),
            ("models/report2.sql", "SELECT * FROM smelt.models.users\n"),
        ],
    );
    let report2_path = project_root.join("models").join("report2.sql");
    let report2_file = db2
        .source_file(&report2_path)
        .expect("report2.sql registered");
    let plan2 = smelt_db::logical_plan(&db2, ws2, report2_file).expect("plan should be Some");
    let call2 = first_function_call(&plan2);
    assert!(
        call2.is_none(),
        "a plain smelt.models.users path-ref (not a call) must NOT produce a FunctionCall node; got: {call2:?}"
    );
}

// ---------------------------------------------------------------------------
// Test 2: legacy extern call → blackbox FunctionCall (parity regression)
// ---------------------------------------------------------------------------

#[test]
fn extern_call_constructs_blackbox_function_call() {
    let dir = tempfile::tempdir().unwrap();
    let project_root = dir.path();

    let (db, ws) = setup_multi_file(
        project_root,
        &[(
            "models/uses_extern.sql",
            "smelt.extern my_agg(col)\nSELECT smelt.functions.my_agg(col) FROM t\n",
        )],
    );

    let model_path = project_root.join("models").join("uses_extern.sql");
    let model_file = db
        .source_file(&model_path)
        .expect("uses_extern.sql registered");

    let plan = smelt_db::logical_plan(&db, ws, model_file).expect("plan should be Some");

    let call =
        first_function_call(&plan).expect("expected a FunctionCall for smelt.functions.my_agg");
    assert!(
        !call.transparent,
        "smelt.extern my_agg should produce transparent=false; got: {call:?}"
    );
    assert!(
        !call.has_body,
        "opaque extern should produce body=None; got: {call:?}"
    );
    let _ = Provenance::Unknown; // ensure import used
}
