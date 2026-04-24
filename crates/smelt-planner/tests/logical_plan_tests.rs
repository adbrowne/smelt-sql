//! Logical plan data model tests (Phase 30 + Phase 31).
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
            ..
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

// ===========================================================================
// Phase 31 — Provenance and unstable-schema gate
// ===========================================================================

/// Write a minimal `smelt.yml` into `dir` with the given `unstable_schema` value.
fn write_smelt_yml(dir: &std::path::Path, unstable_schema: bool) {
    let yml = format!(
        "name: test_project\nversion: 1\ntargets: {{}}\nunstable_schema: {unstable_schema}\n"
    );
    std::fs::write(dir.join("smelt.yml"), yml).unwrap();
}

/// Build a database rooted at `project_root`, register a single file, and set
/// up the workspace. The `smelt_yml_text` is fed into the Salsa-tracked
/// `ProjectInput` (not read from disk) so Salsa can detect changes.
fn setup_with_project_root(
    project_root: &std::path::Path,
    path: &str,
    text: &str,
) -> (Database, smelt_db::SourceFile, smelt_db::Workspace) {
    let smelt_yml_text =
        std::fs::read_to_string(project_root.join("smelt.yml")).unwrap_or_default();
    let mut db = Database::default();
    let file = db.set_source_file(
        project_root.join(path),
        text.to_string(),
        project_root.to_path_buf(),
    );
    let project = db.set_project_input(project_root.to_path_buf(), String::new());
    db.set_project_smelt_yml(project_root, smelt_yml_text);
    db.set_workspace(vec![file], vec![project]);
    let ws = db.workspace();
    (db, file, ws)
}

// ---------------------------------------------------------------------------
// Phase 31 Test 1 — provenance_parsed_from_frontmatter
// ---------------------------------------------------------------------------
//
// A smelt.define with `provenance: { margin: [source.revenue, source.cost] }`
// in its frontmatter and `unstable_schema: true` in smelt.yml should produce
// a FunctionCall plan node with provenance = Declared([("margin", [...])]).

#[test]
fn provenance_parsed_from_frontmatter() {
    let dir = tempfile::tempdir().unwrap();
    write_smelt_yml(dir.path(), true);

    // File declares the function with provenance AND calls it.
    let text = concat!(
        "---\n",
        "provenance: { margin: [source.revenue, source.cost] }\n",
        "deterministic: true\n",
        "---\n",
        "smelt.define add_margin_with_provenance(source: TableExpr<{revenue: Numeric, cost: Numeric}>) -> TableExpr AS (\n",
        "  SELECT source.*, revenue - cost AS margin FROM source\n",
        ")\n",
        "SELECT smelt.fn.add_margin_with_provenance(t) FROM t\n",
    );

    let (db, file, ws) = setup_with_project_root(dir.path(), "models/m.sql", text);
    let plan = smelt_db::logical_plan(&db, ws, file).expect("plan should be Some");
    let call = first_function_call(&plan).expect("expected a FunctionCall in plan");

    assert_eq!(
        call.provenance,
        Provenance::Declared(vec![(
            "margin".to_string(),
            vec!["source.revenue".to_string(), "source.cost".to_string()],
        )]),
        "expected Declared provenance on FunctionCall; got: {:?}",
        call.provenance
    );
}

// ---------------------------------------------------------------------------
// Phase 31 Test 2 — undeclared_provenance_is_opaque
// ---------------------------------------------------------------------------
//
// A function with no `provenance:` key in its frontmatter gets
// provenance = Provenance::Unknown on the resulting FunctionCall node.

#[test]
fn undeclared_provenance_is_opaque() {
    let text = concat!(
        "---\n",
        "deterministic: true\n",
        "---\n",
        "smelt.define no_prov_fn(x) AS (x + 1)\n",
        "SELECT smelt.fn.no_prov_fn(col) FROM t\n",
    );

    let (db, file, ws) = setup_single_file("models/no_prov.sql", text);
    let plan = smelt_db::logical_plan(&db, ws, file).expect("plan should be Some");
    let call = first_function_call(&plan).expect("expected a FunctionCall in plan");

    assert_eq!(
        call.provenance,
        Provenance::Unknown,
        "expected Unknown provenance when no provenance: key declared; got: {:?}",
        call.provenance
    );
}

// ---------------------------------------------------------------------------
// Phase 31 Test 3 — deterministic_idempotent_append_only_propagate
// ---------------------------------------------------------------------------
//
// A function declared with deterministic: true, idempotent: true, append_only:
// true in frontmatter has all three properties set on the FunctionCall node.
// This ensures Phase 30 properties still work correctly with Phase 31 additions.

#[test]
fn deterministic_idempotent_append_only_propagate() {
    let text = concat!(
        "---\n",
        "deterministic: true\n",
        "idempotent: true\n",
        "append_only: true\n",
        "---\n",
        "smelt.define triple_flag_fn(x) AS (x * 2)\n",
        "SELECT smelt.fn.triple_flag_fn(col) FROM t\n",
    );

    let (db, file, ws) = setup_single_file("models/triple_flag.sql", text);
    let plan = smelt_db::logical_plan(&db, ws, file).expect("plan should be Some");
    let call = first_function_call(&plan).expect("expected a FunctionCall in plan");

    assert!(
        call.properties.deterministic,
        "expected deterministic=true; got: {:?}",
        call.properties
    );
    assert!(
        call.properties.idempotent,
        "expected idempotent=true; got: {:?}",
        call.properties
    );
    assert!(
        call.properties.append_only,
        "expected append_only=true; got: {:?}",
        call.properties
    );
}

// ---------------------------------------------------------------------------
// Phase 31 Test 4 — provenance_schema_frozen_under_unstable_flag
// ---------------------------------------------------------------------------
//
// If a function's frontmatter contains `provenance:` but smelt.yml does NOT
// have `unstable_schema: true`, the system emits a
// DiagnosticCode::UnstableSchemaRequired diagnostic, and the plan node's
// provenance stays Unknown.

#[test]
fn provenance_schema_frozen_under_unstable_flag() {
    let dir = tempfile::tempdir().unwrap();
    // Write a smelt.yml WITHOUT unstable_schema: true (omitting it defaults to false).
    std::fs::write(
        dir.path().join("smelt.yml"),
        "name: test_project\nversion: 1\ntargets: {}\n",
    )
    .unwrap();

    let text = concat!(
        "---\n",
        "provenance: { margin: [source.revenue, source.cost] }\n",
        "---\n",
        "smelt.define fn_with_prov(source: TableExpr<{revenue: Numeric, cost: Numeric}>) -> TableExpr AS (\n",
        "  SELECT source.*, revenue - cost AS margin FROM source\n",
        ")\n",
        "SELECT smelt.fn.fn_with_prov(t) FROM t\n",
    );

    let (db, file, ws) = setup_with_project_root(dir.path(), "models/m.sql", text);

    // The plan node's provenance must stay Unknown.
    let plan = smelt_db::logical_plan(&db, ws, file).expect("plan should be Some");
    let call = first_function_call(&plan).expect("expected a FunctionCall in plan");
    assert_eq!(
        call.provenance,
        Provenance::Unknown,
        "expected provenance=Unknown when unstable_schema is false; got: {:?}",
        call.provenance
    );

    // The diagnostic UnstableSchemaRequired must be emitted through file_diagnostics.
    let diags = smelt_db::file_diagnostics(&db, ws, file);
    let matching: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(smelt_db::DiagnosticCode::UnstableSchemaRequired))
        .collect();
    assert!(
        !matching.is_empty(),
        "expected UnstableSchemaRequired diagnostic when provenance: used without unstable_schema flag; \
         got diagnostics: {diags:#?}"
    );
}
