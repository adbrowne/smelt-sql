//! Phase 30 — Logical plan data model tests.
//!
//! These tests verify that `smelt-db` builds a `smelt_planner::logical::Plan`
//! from a parsed model and that the plan contains the expected structure.
//!
//! The Salsa query `logical_plan(db, ws, file)` lives in `smelt-db`; these tests
//! drive it by constructing an in-memory `Database`, setting up source files, and
//! asserting on the plan tree.

use std::path::PathBuf;
use std::sync::Arc;

use smelt_db::Database;
use smelt_planner::logical::{FunctionProperties, LogicalNode, Provenance};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a database, register a single file, and set up the workspace.
fn setup_single_file(
    path: &str,
    text: &str,
) -> (Database, smelt_db::SourceFile, smelt_db::Workspace) {
    let mut db = Database::default();
    let file = db.set_source_file(PathBuf::from(path), text.to_string(), PathBuf::from("."));
    db.set_workspace(vec![file], vec![]);
    let ws = db.workspace();
    (db, file, ws)
}

// ---------------------------------------------------------------------------
// Test 1 — plan contains a FunctionCall node for smelt.fn.* calls
// ---------------------------------------------------------------------------

#[test]
fn plan_builds_function_call_node() {
    let (db, file, ws) = setup_single_file("models/m.sql", "SELECT smelt.fn.some_fn(a, b) FROM t");

    let plan =
        smelt_db::logical_plan(&db, ws, file).expect("plan should be Some for a valid model");

    let call = first_function_call(&plan).expect("expected a FunctionCall node in plan");
    assert_eq!(
        call.fn_id, "some_fn",
        "FunctionCall fn_id should be the segments after smelt.fn."
    );
}

// ---------------------------------------------------------------------------
// Test 2 — transparent flag matches function origin
// ---------------------------------------------------------------------------

#[test]
fn transparent_flag_matches_function_transparency() {
    // --- smelt.define → transparent = true ---
    {
        let define_text = "smelt.define some_fn(x) AS (x + 1)\nSELECT smelt.fn.some_fn(col) FROM t";
        let (db, file, ws) = setup_single_file("models/define_model.sql", define_text);
        let plan = smelt_db::logical_plan(&db, ws, file).expect("plan should be Some");
        let call = first_function_call(&plan).expect("expected a FunctionCall in plan");
        assert!(
            call.transparent,
            "smelt.define function should be transparent=true, got: {call:?}"
        );
    }

    // --- smelt.extern → transparent = false ---
    {
        let extern_text = "smelt.extern ext_fn(x)\nSELECT smelt.fn.ext_fn(col) FROM t";
        let (db, file, ws) = setup_single_file("models/extern_model.sql", extern_text);
        let plan = smelt_db::logical_plan(&db, ws, file).expect("plan should be Some");
        let call = first_function_call(&plan).expect("expected a FunctionCall in plan");
        assert!(
            !call.transparent,
            "smelt.extern function should be transparent=false, got: {call:?}"
        );
    }
}

/// Traverse and return the first `FunctionCall` variant's fields.
fn first_function_call(node: &Arc<LogicalNode>) -> Option<FunctionCallFields> {
    match node.as_ref() {
        LogicalNode::FunctionCall {
            fn_id,
            args,
            transparent,
            provenance,
            properties,
        } => Some(FunctionCallFields {
            fn_id: fn_id.clone(),
            transparent: *transparent,
            provenance: provenance.clone(),
            properties: properties.clone(),
            args: args.clone(),
        }),
        LogicalNode::Select { from, filter, .. } => from
            .as_ref()
            .and_then(first_function_call)
            .or_else(|| filter.as_ref().and_then(first_function_call)),
        LogicalNode::TableRef { .. } | LogicalNode::Literal(_) => None,
    }
}

#[derive(Debug)]
#[allow(dead_code)]
struct FunctionCallFields {
    fn_id: String,
    transparent: bool,
    provenance: Provenance,
    properties: FunctionProperties,
    args: Vec<Arc<LogicalNode>>,
}

// ---------------------------------------------------------------------------
// Test 3 — properties propagate from frontmatter
// ---------------------------------------------------------------------------

#[test]
fn properties_propagate_from_frontmatter() {
    // A file containing:
    //   1. A smelt.define with `deterministic: true` in its per-declaration frontmatter
    //   2. A SELECT that calls the function
    //
    // Per-declaration frontmatter uses the `---…---` fence immediately before
    // the `smelt.define` keyword.
    let text = "---\ndeterministic: true\n---\nsmelt.define det_fn(x) AS (x * 2)\nSELECT smelt.fn.det_fn(col) FROM t";

    let (db, file, ws) = setup_single_file("models/det_model.sql", text);
    let plan = smelt_db::logical_plan(&db, ws, file).expect("plan should be Some");
    let call = first_function_call(&plan).expect("expected a FunctionCall in plan");
    assert!(
        call.properties.deterministic,
        "expected deterministic=true from frontmatter, got: {:?}",
        call.properties
    );
}

// ---------------------------------------------------------------------------
// Test 4 — logical_plan is a Salsa query (stable results across calls)
// ---------------------------------------------------------------------------
//
// Full Salsa event-counter infrastructure is complex to set up, so this test
// verifies the weaker property: calling `logical_plan` twice on an unchanged
// database produces identical results. The caching guarantee is implicit in
// Salsa's tracked-function semantics.
//
// This is documented as partial coverage — it proves result stability but not
// that Salsa avoids re-execution.

#[test]
fn plan_is_salsa_query_stable_results() {
    let (db, file, ws) = setup_single_file("models/m.sql", "SELECT smelt.fn.some_fn(a, b) FROM t");

    let plan1 = smelt_db::logical_plan(&db, ws, file);
    let plan2 = smelt_db::logical_plan(&db, ws, file);

    assert_eq!(
        plan1, plan2,
        "calling logical_plan twice should return equal plans"
    );
}
