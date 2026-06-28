//! Phase 51 — TDD tests for the provenance/joins validator.
//!
//! These tests validate the pure `provenance_validator` module in `smelt-db`
//! against the five scenarios described in the Phase 51 plan.

use std::path::PathBuf;

use smelt_db::{file_diagnostics, Database, DiagnosticCode, SourceFile, Workspace};

/// Build a test database with `unstable_schema: true` in `smelt.yml`.
/// Prepends frontmatter to `fn_sql` if `frontmatter` is non-empty.
fn make_db_with_unstable(fn_sql: &str, frontmatter: &str) -> (Database, Workspace, SourceFile) {
    let smelt_yml = "unstable_schema: true\n";
    let full_sql = if frontmatter.is_empty() {
        fn_sql.to_string()
    } else {
        format!("---\n{frontmatter}\n---\n{fn_sql}")
    };
    let path = PathBuf::from("/tmp/test_provenance_fn.sql");
    let project_root = PathBuf::from("/tmp");
    let mut db = Database::default();
    // set_project_input reads smelt.yml from disk; use set_project_smelt_yml to
    // inject the in-memory content so unstable_schema is true in tests.
    let project = db.set_project_input(project_root.clone(), String::new());
    db.set_project_smelt_yml(&project_root, smelt_yml.to_string());
    let sf = db.set_source_file(path.clone(), full_sql, project_root.clone());
    db.set_workspace(vec![sf], vec![project]);
    let ws = Workspace::try_get(&db).expect("workspace");
    (db, ws, sf)
}

/// Test 1 — provenance matches the body projection. Expect ZERO ProvenanceMismatch.
#[test]
fn provenance_matches_body_projection() {
    let fn_sql = "smelt.define add_margin(source: TableExpr<{revenue: Numeric, cost: Numeric}>) \
                  -> TableExpr AS (\n  SELECT revenue - cost AS margin FROM source\n)";
    let frontmatter = "provenance: { margin: [source.revenue, source.cost] }";

    let (db, ws, sf) = make_db_with_unstable(fn_sql, frontmatter);
    let diags = file_diagnostics(&db, ws, sf);
    let mismatches: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::ProvenanceMismatch))
        .collect();
    assert!(
        mismatches.is_empty(),
        "expected zero ProvenanceMismatch diagnostics for matching provenance, got: {mismatches:#?}"
    );
}

/// Test 2 — provenance declares a source column not read by the body.
/// Expect AT LEAST ONE ProvenanceMismatch.
#[test]
fn provenance_extra_column_errors() {
    let fn_sql = "smelt.define add_margin(source: TableExpr<{revenue: Numeric, cost: Numeric}>) \
                  -> TableExpr AS (\n  SELECT revenue - cost AS margin FROM source\n)";
    // dim.x is declared but not read
    let frontmatter = "provenance: { margin: [source.revenue, source.cost, dim.x] }";

    let (db, ws, sf) = make_db_with_unstable(fn_sql, frontmatter);
    let diags = file_diagnostics(&db, ws, sf);
    let mismatches: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::ProvenanceMismatch))
        .collect();
    assert!(
        !mismatches.is_empty(),
        "expected at least one ProvenanceMismatch for extra declared column dim.x, got: {diags:#?}"
    );
}

/// Test 3 — provenance omits a column that IS read by the body.
/// Expect AT LEAST ONE ProvenanceMismatch.
#[test]
fn provenance_missing_column_errors() {
    let fn_sql = "smelt.define add_margin(source: TableExpr<{revenue: Numeric, cost: Numeric}>) \
                  -> TableExpr AS (\n  SELECT revenue - cost AS margin FROM source\n)";
    // source.revenue is read but not declared
    let frontmatter = "provenance: { margin: [source.cost] }";

    let (db, ws, sf) = make_db_with_unstable(fn_sql, frontmatter);
    let diags = file_diagnostics(&db, ws, sf);
    let mismatches: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::ProvenanceMismatch))
        .collect();
    assert!(
        !mismatches.is_empty(),
        "expected at least one ProvenanceMismatch for missing declared column source.revenue, got: {diags:#?}"
    );
}

/// Test 4 — joins: declares dim_a but body joins dim_b. Expect AT LEAST ONE JoinsMismatch.
#[test]
fn joins_declared_but_body_has_different_join_set() {
    let fn_sql = "smelt.define fn_joins_check(src: TableExpr) -> TableExpr AS (\n  \
                  SELECT a.x FROM src LEFT JOIN dim_b AS dim_b ON src.id = dim_b.id\n)";
    // Declares dim_a but body has dim_b
    let frontmatter =
        "joins:\n  - table: dim_a\n    on: \"src.id = dim_a.id\"\n    cardinality: \"1:1\"";

    let (db, ws, sf) = make_db_with_unstable(fn_sql, frontmatter);
    let diags = file_diagnostics(&db, ws, sf);
    let mismatches: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::JoinsMismatch))
        .collect();
    assert!(
        !mismatches.is_empty(),
        "expected at least one JoinsMismatch for declared dim_a not found in body joins, got: {diags:#?}"
    );
}

/// Test 5 — join matches but has cardinality declared. Expect AT LEAST ONE DeclaredCardinalityUnverifiable warning.
#[test]
fn joins_cardinality_unverifiable_warning() {
    let fn_sql = "smelt.define fn_joins_check(src: TableExpr) -> TableExpr AS (\n  \
                  SELECT a.x FROM src LEFT JOIN dim_b AS dim_b ON src.id = dim_b.id\n)";
    // dim_b matches, but cardinality is declared (unverifiable)
    let frontmatter =
        "joins:\n  - table: dim_b\n    on: \"src.id = dim_b.id\"\n    cardinality: \"1:1\"";

    let (db, ws, sf) = make_db_with_unstable(fn_sql, frontmatter);
    let diags = file_diagnostics(&db, ws, sf);
    let warnings: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::DeclaredCardinalityUnverifiable))
        .collect();
    assert!(
        !warnings.is_empty(),
        "expected at least one DeclaredCardinalityUnverifiable warning, got: {diags:#?}"
    );
}
