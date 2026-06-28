//! Tests for Phase 4: `#` CTE-reference operator — `CteRefOutsideTest` diagnostic.
//!
//! A `smelt.<model>#<cte>` reference is only valid inside a `smelt.test` body.
//! Using the `#` suffix anywhere else (model body, smelt.define body, top-level
//! expression) is a hard error (`CteRefOutsideTest`, anchored at the `#`).

use std::path::PathBuf;

use smelt_db::{file_diagnostics, Database, DiagnosticCode, SourceFile, Workspace};

// ── helpers ──────────────────────────────────────────────────────────────────

fn build_db_single(model_path: PathBuf, sql: &str) -> (Database, Workspace, SourceFile) {
    let root = PathBuf::from("/fake/project");
    let mut db = Database::default();
    let project = db.set_project_input(root.clone(), String::new());
    db.set_project_smelt_yml(&root, "name: test\nversion: 1\n".to_string());
    let sf = db.set_source_file(model_path.clone(), sql.to_string(), root.clone());
    db.set_workspace(vec![sf], vec![project]);
    let ws = db.workspace();
    (db, ws, sf)
}

// ── cte_ref_outside_test_diagnostic ──────────────────────────────────────────

/// A `#` CTE ref in a model body (not a smelt.test) must yield `CteRefOutsideTest`,
/// anchored at the `#`.
#[test]
fn cte_ref_outside_test_in_model_body_emits_diagnostic() {
    let model_path = PathBuf::from("/fake/project/models/my_model.sql");
    // The `#` is used inside a plain SELECT — not inside smelt.test.
    let sql = "SELECT day FROM smelt.daily_revenue#daily_agg";

    let (db, ws, sf) = build_db_single(model_path, sql);
    let diags = file_diagnostics(&db, ws, sf);

    let cte_diag = diags
        .iter()
        .find(|d| d.code == Some(DiagnosticCode::CteRefOutsideTest));

    assert!(
        cte_diag.is_some(),
        "expected CteRefOutsideTest diagnostic; got: {diags:#?}"
    );

    // Diagnostic must be anchored at the `#` character (byte offset 35
    // in "SELECT day FROM smelt.daily_revenue#daily_agg").
    let diag = cte_diag.unwrap();
    let hash_start: u32 = diag.range.start().into();
    assert_eq!(
        hash_start, 35,
        "CteRefOutsideTest should be anchored at the `#` (byte 35); got byte {hash_start}"
    );
    assert_eq!(
        u32::from(diag.range.len()),
        1,
        "CteRefOutsideTest range should span exactly 1 byte (the `#`)"
    );
}

// ── cte_ref_inside_test_ok ────────────────────────────────────────────────────

/// The same `#` CTE ref inside a `smelt.test` body must NOT produce a
/// `CteRefOutsideTest` diagnostic.
#[test]
fn cte_ref_inside_test_body_produces_no_diagnostic() {
    let test_path = PathBuf::from("/fake/project/tests/my_test.sql");
    // The `#` ref appears inside a smelt.test body — this is the valid usage.
    let sql = r#"smelt.test daily_agg_rollup AS (
    SELECT day, revenue FROM smelt.daily_revenue#daily_agg
)
PASSING orders AS ({order_id: 1, amount: 100.0, order_date: '2024-01-01'})
EXPECT ({day: '2024-01-01', revenue: 100.0})"#;

    let (db, ws, sf) = build_db_single(test_path, sql);
    let diags = file_diagnostics(&db, ws, sf);

    let cte_diags: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::CteRefOutsideTest))
        .collect();

    assert!(
        cte_diags.is_empty(),
        "expected no CteRefOutsideTest inside smelt.test body; got: {cte_diags:#?}"
    );
}

// ── no false positives on plain refs ─────────────────────────────────────────

/// Plain `smelt.<path>` refs (without `#`) must never trigger
/// `CteRefOutsideTest`.
#[test]
fn plain_smelt_ref_no_false_positive() {
    let model_path = PathBuf::from("/fake/project/models/my_model.sql");
    let sql = "SELECT * FROM smelt.daily_revenue";

    let (db, ws, sf) = build_db_single(model_path, sql);
    let diags = file_diagnostics(&db, ws, sf);

    assert!(
        !diags
            .iter()
            .any(|d| d.code == Some(DiagnosticCode::CteRefOutsideTest)),
        "plain smelt ref should not produce CteRefOutsideTest; got: {diags:#?}"
    );
}

// ── cte_ref_inside_define_body ────────────────────────────────────────────────

/// A `#` CTE ref inside a `smelt.define` body (not a smelt.test) must yield
/// `CteRefOutsideTest`, anchored at the `#`.  `smelt.define` bodies are
/// explicitly listed in the spec as an invalid position for `#`-refs.
#[test]
fn cte_ref_in_define_body_emits_diagnostic() {
    let file_path = PathBuf::from("/fake/project/models/helpers.sql");
    // The `#` ref is used inside a smelt.define body — not inside a smelt.test.
    // "smelt.define helper() AS (SELECT x FROM smelt.daily_revenue#daily_agg)"
    //  ^0                                                          ^59
    let sql = "smelt.define helper() AS (SELECT x FROM smelt.daily_revenue#daily_agg)";

    let (db, ws, sf) = build_db_single(file_path, sql);
    let diags = file_diagnostics(&db, ws, sf);

    let cte_diags: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::CteRefOutsideTest))
        .collect();

    assert_eq!(
        cte_diags.len(),
        1,
        "expected exactly one CteRefOutsideTest inside smelt.define body; got: {cte_diags:#?}"
    );

    // Diagnostic must be anchored at the `#` character (byte offset 59).
    let diag = cte_diags[0];
    let hash_start: u32 = diag.range.start().into();
    assert_eq!(
        hash_start, 59,
        "CteRefOutsideTest should be anchored at the `#` (byte 59); got byte {hash_start}"
    );
    assert_eq!(
        u32::from(diag.range.len()),
        1,
        "CteRefOutsideTest range should span exactly 1 byte (the `#`)"
    );
}

// ── multiple # refs in mixed contexts ─────────────────────────────────────────

/// Multiple `#` refs in the same file — one inside a smelt.test (OK) and one
/// outside (Error).  Only the outside one must trigger the diagnostic.
#[test]
fn mixed_cte_refs_only_outside_test_emits_diagnostic() {
    let file_path = PathBuf::from("/fake/project/tests/mixed.sql");
    // Two refs: one in a bare SELECT (outside test) and one inside a smelt.test.
    let sql = r#"smelt.test valid_test AS (
    SELECT day FROM smelt.daily_revenue#daily_agg
)
EXPECT ({day: '2024-01-01'})

-- This bare SELECT is outside any smelt.test:
SELECT day FROM smelt.daily_revenue#daily_agg"#;

    let (db, ws, sf) = build_db_single(file_path, sql);
    let diags = file_diagnostics(&db, ws, sf);

    let cte_diags: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::CteRefOutsideTest))
        .collect();

    assert_eq!(
        cte_diags.len(),
        1,
        "expected exactly one CteRefOutsideTest (the outside-test ref); got: {cte_diags:#?}"
    );
}
