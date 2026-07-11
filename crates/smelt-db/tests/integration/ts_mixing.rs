//! Phase 2 (timezone axis) — Strict tz mixing rule.
//!
//! Verifies that:
//!   1. `SELECT ts_col UNION SELECT tstz_col` produces a TypeMismatch diagnostic.
//!   2. `SELECT ts1 UNION SELECT ts2` (two naive columns) produces no diagnostic.
//!   3. `SELECT tstz1 UNION SELECT tstz2` (two tz-aware columns) produces no diagnostic.
//!   4. `SELECT tstz_col - ts_col` emits TypeMismatch.
//!   5. A CASE with naive THEN and tz-aware ELSE emits TypeMismatch.

use std::path::PathBuf;

use smelt_db::{file_diagnostics, Database, DiagnosticCode, SourceFile, Workspace};

// ---------------------------------------------------------------------------
// Shared harness (mirrors ts_function_returns.rs)
// ---------------------------------------------------------------------------

fn build_db(
    project_root: PathBuf,
    files: &[(PathBuf, &str)],
) -> (Database, Workspace, Vec<SourceFile>) {
    let mut db = Database::default();
    let project = db.set_project_input(project_root.clone(), String::new());

    let mut handles = Vec::with_capacity(files.len());
    for (path, content) in files {
        let sf = db.set_source_file(path.clone(), (*content).to_string(), project_root.clone());
        handles.push(sf);
    }
    db.set_workspace(handles.clone(), vec![project]);
    let ws = db.workspace();
    (db, ws, handles)
}

/// Build a two-model workspace:
/// - `upstream.sql` emits named typed columns (used as the FROM source)
/// - `model.sql` contains the SQL under test referencing the upstream columns
///
/// Returns the diagnostics for `model.sql`.
fn diags_for_model_over_upstream(
    upstream_sql: &str,
    model_sql: &str,
    test_name: &str,
) -> Vec<smelt_db::Diagnostic> {
    let root = PathBuf::from(format!("/fake/project/{}", test_name));
    let upstream_path = root.join("models").join("upstream.sql");
    let model_path = root.join("models").join("model.sql");

    let (db, ws, files) = build_db(
        root,
        &[(upstream_path, upstream_sql), (model_path, model_sql)],
    );
    file_diagnostics(&db, ws, files[1])
}

/// Check whether any diagnostic in `diags` has code TypeMismatch.
fn has_type_mismatch(diags: &[smelt_db::Diagnostic]) -> bool {
    diags
        .iter()
        .any(|d| matches!(d.code, Some(DiagnosticCode::TypeMismatch)))
}

// ---------------------------------------------------------------------------
// Upstream SQL fragments that expose typed columns
// ---------------------------------------------------------------------------

// Exposes both naive and tz-aware columns (used for arithmetic and CASE tests).
const UPSTREAM_MIXED_COLS: &str = "\
SELECT
    MAKE_TIMESTAMP(2024, 1, 1, 0, 0, 0) AS ts_col,
    NOW() AS tstz_col
";

// ---------------------------------------------------------------------------
// Test 1 — UNION of naive and tz-aware → TypeMismatch
// ---------------------------------------------------------------------------

#[test]
fn union_mixed_tz_is_type_mismatch() {
    // Use function-call expressions whose return types are statically known
    // (MAKE_TIMESTAMP → naive Timestamp, NOW() → Timestamp WITH TIME ZONE)
    // so that type inference works without needing an upstream column schema.
    let root = PathBuf::from("/fake/project/union_mixed_tz");
    let model_path = root.join("models").join("model.sql");

    // MAKE_TIMESTAMP returns naive Timestamp; NOW() returns Timestamp WITH TIME ZONE.
    let model_sql = "\
SELECT MAKE_TIMESTAMP(2024, 1, 1, 0, 0, 0) AS ts
UNION ALL
SELECT NOW() AS ts
";

    let (db, ws, files) = build_db(root, &[(model_path, model_sql)]);
    let diags = file_diagnostics(&db, ws, files[0]);

    assert!(
        has_type_mismatch(&diags),
        "Expected TypeMismatch when UNIONing naive Timestamp with Timestamp WITH TIME ZONE, \
         got diagnostics: {:?}",
        diags
    );
}

// ---------------------------------------------------------------------------
// Test 1b — EXCEPT / INTERSECT mixed tz → TypeMismatch (the spec names all
// three set operators, not just UNION)
// ---------------------------------------------------------------------------

#[test]
fn except_mixed_tz_is_type_mismatch() {
    let root = PathBuf::from("/fake/project/except_mixed_tz");
    let model_path = root.join("models").join("model.sql");

    let model_sql = "\
SELECT MAKE_TIMESTAMP(2024, 1, 1, 0, 0, 0) AS ts
EXCEPT
SELECT NOW() AS ts
";

    let (db, ws, files) = build_db(root, &[(model_path, model_sql)]);
    let diags = file_diagnostics(&db, ws, files[0]);

    assert!(
        has_type_mismatch(&diags),
        "Expected TypeMismatch when EXCEPTing naive Timestamp with Timestamp WITH TIME ZONE, \
         got diagnostics: {:?}",
        diags
    );
}

#[test]
fn intersect_mixed_tz_is_type_mismatch() {
    let root = PathBuf::from("/fake/project/intersect_mixed_tz");
    let model_path = root.join("models").join("model.sql");

    let model_sql = "\
SELECT MAKE_TIMESTAMP(2024, 1, 1, 0, 0, 0) AS ts
INTERSECT
SELECT NOW() AS ts
";

    let (db, ws, files) = build_db(root, &[(model_path, model_sql)]);
    let diags = file_diagnostics(&db, ws, files[0]);

    assert!(
        has_type_mismatch(&diags),
        "Expected TypeMismatch when INTERSECTing naive Timestamp with Timestamp WITH TIME ZONE, \
         got diagnostics: {:?}",
        diags
    );
}

// ---------------------------------------------------------------------------
// Test 2 — UNION of two naive Timestamps → no TypeMismatch (no regression)
// ---------------------------------------------------------------------------

#[test]
fn union_same_tz_naive_ok() {
    let root = PathBuf::from("/fake/project/union_same_naive");
    let model_path = root.join("models").join("model.sql");

    // Both branches produce naive Timestamp via MAKE_TIMESTAMP — no mismatch.
    let model_sql = "\
SELECT MAKE_TIMESTAMP(2024, 1, 1, 0, 0, 0) AS ts
UNION ALL
SELECT MAKE_TIMESTAMP(2024, 6, 1, 0, 0, 0) AS ts
";

    let (db, ws, files) = build_db(root, &[(model_path, model_sql)]);
    let diags = file_diagnostics(&db, ws, files[0]);

    assert!(
        !has_type_mismatch(&diags),
        "Expected no TypeMismatch when UNIONing two naive Timestamps, \
         got TypeMismatch in diagnostics: {:?}",
        diags
    );
}

// ---------------------------------------------------------------------------
// Test 3 — UNION of two tz-aware Timestamps → no TypeMismatch
// ---------------------------------------------------------------------------

#[test]
fn union_same_tz_aware_ok() {
    let root = PathBuf::from("/fake/project/union_same_tz");
    let model_path = root.join("models").join("model.sql");

    // Both branches produce Timestamp WITH TIME ZONE via NOW() — no mismatch.
    let model_sql = "\
SELECT NOW() AS ts
UNION ALL
SELECT CURRENT_TIMESTAMP AS ts
";

    let (db, ws, files) = build_db(root, &[(model_path, model_sql)]);
    let diags = file_diagnostics(&db, ws, files[0]);

    assert!(
        !has_type_mismatch(&diags),
        "Expected no TypeMismatch when UNIONing two Timestamp WITH TIME ZONE values, \
         got TypeMismatch in diagnostics: {:?}",
        diags
    );
}

// ---------------------------------------------------------------------------
// Test 4 — Arithmetic: tstz_col - ts_col → TypeMismatch
// ---------------------------------------------------------------------------

#[test]
fn arithmetic_mixed_tz_is_type_mismatch() {
    let model_sql = "\
SELECT tstz_col - ts_col AS diff FROM smelt.models.upstream
";

    let diags = diags_for_model_over_upstream(UPSTREAM_MIXED_COLS, model_sql, "arith_mixed_tz");

    assert!(
        has_type_mismatch(&diags),
        "Expected TypeMismatch for tstz_col - ts_col (mixing tz variants), \
         got diagnostics: {:?}",
        diags
    );
}

// ---------------------------------------------------------------------------
// Test 5 — CASE with mixed tz THEN/ELSE → TypeMismatch
// ---------------------------------------------------------------------------

#[test]
fn case_mixed_tz_is_type_mismatch() {
    let model_sql = "\
SELECT CASE WHEN TRUE THEN ts_col ELSE tstz_col END AS result
FROM smelt.models.upstream
";

    let diags = diags_for_model_over_upstream(UPSTREAM_MIXED_COLS, model_sql, "case_mixed_tz");

    assert!(
        has_type_mismatch(&diags),
        "Expected TypeMismatch for CASE with naive THEN and tz-aware ELSE, \
         got diagnostics: {:?}",
        diags
    );
}
