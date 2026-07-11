//! TDD tests for Phase 2: smelt.check diagnostic detection.
//! - CheckHasTestClause: a smelt.check with PASSING/EXPECT yields a diagnostic
//! - CteRefOutsideTest: a `#` ref inside a smelt.check body yields CteRefOutsideTest

use smelt_db::{file_diagnostics, Database, DiagnosticCode, SourceFile, Workspace};
use std::path::PathBuf;

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

/// A `smelt.check` with a PASSING clause must yield `CheckHasTestClause`.
#[test]
fn check_with_passing_diagnoses() {
    let check_path = PathBuf::from("/fake/project/checks/bad_check.sql");
    let sql = r#"smelt.check bad_check AS (
    SELECT id FROM smelt.orders WHERE id IS NULL
)
PASSING orders AS ({id: 1})"#;

    let (db, ws, sf) = build_db_single(check_path, sql);
    let diags = file_diagnostics(&db, ws, sf);

    let check_diag = diags
        .iter()
        .find(|d| d.code == Some(DiagnosticCode::CheckHasTestClause));

    assert!(
        check_diag.is_some(),
        "expected CheckHasTestClause diagnostic for check with PASSING clause; got: {diags:#?}"
    );
}

/// A `smelt.check` with an EXPECT clause must also yield `CheckHasTestClause`.
#[test]
fn check_with_expect_diagnoses() {
    let check_path = PathBuf::from("/fake/project/checks/bad_expect_check.sql");
    let sql = r#"smelt.check bad_expect_check AS (
    SELECT id FROM smelt.orders WHERE id IS NULL
)
EXPECT ({id: 1})"#;

    let (db, ws, sf) = build_db_single(check_path, sql);
    let diags = file_diagnostics(&db, ws, sf);

    let check_diag = diags
        .iter()
        .find(|d| d.code == Some(DiagnosticCode::CheckHasTestClause));

    assert!(
        check_diag.is_some(),
        "expected CheckHasTestClause diagnostic for check with EXPECT clause; got: {diags:#?}"
    );
}

/// A `smelt.check` body with a `#` ref must yield `CteRefOutsideTest`.
/// A check body is NOT a test body, so the `#` operator is invalid there.
#[test]
fn hash_cte_ref_in_check_is_outside_test() {
    let check_path = PathBuf::from("/fake/project/checks/cte_check.sql");
    let sql = "smelt.check bad_cte_ref AS (SELECT x FROM smelt.daily_revenue#daily_agg)";

    let (db, ws, sf) = build_db_single(check_path, sql);
    let diags = file_diagnostics(&db, ws, sf);

    let cte_diag = diags
        .iter()
        .find(|d| d.code == Some(DiagnosticCode::CteRefOutsideTest));

    assert!(
        cte_diag.is_some(),
        "expected CteRefOutsideTest for `#` ref inside smelt.check body; got: {diags:#?}"
    );
}

/// A `smelt.check` with an invalid `severity` value must yield a diagnostic
/// (fail-loud discipline: unknown user input never silently defaults to Error).
#[test]
fn check_with_invalid_severity_diagnoses() {
    let check_path = PathBuf::from("/fake/project/checks/bad_severity.sql");
    let sql = r#"---
severity: bogus
---
smelt.check bad_severity AS (
    SELECT id FROM smelt.orders WHERE id IS NULL
)"#;

    let (db, ws, sf) = build_db_single(check_path, sql);
    let diags = file_diagnostics(&db, ws, sf);

    assert!(
        !diags.is_empty(),
        "expected a diagnostic for smelt.check with invalid severity value; got none"
    );
    let sev_diag = diags
        .iter()
        .find(|d| d.code == Some(DiagnosticCode::YamlParseError));
    assert!(
        sev_diag.is_some(),
        "expected a YamlParseError diagnostic for severity: bogus; got: {diags:#?}"
    );
}

/// A `smelt.check` with a valid `severity: warn` value produces no
/// severity-related diagnostic.
#[test]
fn check_with_valid_severity_warn_ok() {
    let check_path = PathBuf::from("/fake/project/checks/warn_check.sql");
    let sql = r#"---
severity: warn
---
smelt.check warn_check AS (
    SELECT id FROM smelt.orders WHERE id IS NULL
)"#;

    let (db, ws, sf) = build_db_single(check_path, sql);
    let diags = file_diagnostics(&db, ws, sf);

    let sev_diag = diags
        .iter()
        .find(|d| d.code == Some(DiagnosticCode::YamlParseError));
    assert!(
        sev_diag.is_none(),
        "severity: warn is valid and should produce no YamlParseError; got: {diags:#?}"
    );
}

/// A well-formed `smelt.check` (no PASSING, no EXPECT, no `#`) produces no
/// CheckHasTestClause or CteRefOutsideTest diagnostics.
#[test]
fn well_formed_check_no_diagnostics() {
    let check_path = PathBuf::from("/fake/project/checks/good_check.sql");
    let sql = "smelt.check no_nulls AS (SELECT id FROM smelt.orders WHERE id IS NULL)";

    let (db, ws, sf) = build_db_single(check_path, sql);
    let diags = file_diagnostics(&db, ws, sf);

    let bad_diags: Vec<_> = diags
        .iter()
        .filter(|d| {
            d.code == Some(DiagnosticCode::CheckHasTestClause)
                || d.code == Some(DiagnosticCode::CteRefOutsideTest)
        })
        .collect();

    assert!(
        bad_diags.is_empty(),
        "well-formed smelt.check should produce no CheckHasTestClause or CteRefOutsideTest; got: {bad_diags:#?}"
    );
}
