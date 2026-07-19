//! TDD tests for Phase 2: smelt.check diagnostic detection.
//! - CheckHasTestClause: a smelt.check with PASSING/EXPECT yields a diagnostic
//! - CteRefOutsideTest: a `#` ref inside a smelt.check body yields CteRefOutsideTest
//!
//! Also pins the diagnostic *pairing* (fail-loud discipline, architecture.md
//! §"Fail-loud discipline") at the two `error`-classified `DataType::Unknown`
//! census sites: a construction site that degrades to `Unknown` must never
//! fire without its diagnostic. See `.claude/unknown-census.toml`.

use smelt_db::{
    file_diagnostics, typed_model_schema, Database, DiagnosticCode, SourceFile, Workspace,
};
use smelt_types::DataType;
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

/// Mixed-tz timestamp subtraction (naive `TIMESTAMP` minus tz-aware
/// `TIMESTAMPTZ`) must degrade the column's inferred type to `Unknown` *and*
/// emit a `TypeMismatch` diagnostic at the operator — the pairing, not just
/// each half in isolation. Pins `.claude/unknown-census.toml`'s
/// `binary.rs` mixed-tz-arithmetic site.
#[test]
fn mixed_tz_subtraction_unknown_is_paired_with_type_mismatch() {
    let model_path = PathBuf::from("/fake/project/models/mixed_tz.sql");
    let sql = "SELECT (TIMESTAMP '2020-01-01 00:00:00' - TIMESTAMPTZ '2020-01-01 00:00:00+00') AS ts_diff";

    let (db, ws, sf) = build_db_single(model_path, sql);

    let schema = typed_model_schema(&db, ws, sf);
    let ts_diff = schema
        .columns
        .iter()
        .find(|c| c.name == "ts_diff")
        .expect("expected a ts_diff column in the inferred schema");
    let data_type = ts_diff
        .data_type
        .as_ref()
        .map(|tc| tc.data_type.clone())
        .expect("ts_diff column should have an inferred type");
    assert!(
        matches!(data_type, DataType::Unknown(_)),
        "expected mixed-tz timestamp subtraction to infer as Unknown, got: {data_type:?}"
    );

    let diags = file_diagnostics(&db, ws, sf);
    let type_mismatch = diags
        .iter()
        .find(|d| d.code == Some(DiagnosticCode::TypeMismatch));
    assert!(
        type_mismatch.is_some(),
        "expected a TypeMismatch diagnostic paired with the Unknown inference for mixed-tz \
         timestamp subtraction; got: {diags:#?}"
    );

    // The diagnostic must land on the offending `-` operator, not just
    // anywhere in the file — the range is the single-character operator
    // token span (`check_mixed_tz_arithmetic_diagnostics` uses
    // `BinaryExpr::operator_token_range`).
    let op_offset = sql
        .find(" - ")
        .expect("fixture SQL should contain the mixed-tz subtraction operator")
        + 1;
    let range = type_mismatch.expect("checked above").range;
    let expected_start: u32 = op_offset as u32;
    assert_eq!(
        u32::from(range.start()),
        expected_start,
        "expected TypeMismatch diagnostic range to start at the `-` operator (offset \
         {expected_start}), got range {range:?} in SQL: {sql}"
    );
    assert_eq!(
        u32::from(range.end()),
        expected_start + 1,
        "expected TypeMismatch diagnostic range to span exactly the `-` operator token, got \
         range {range:?} in SQL: {sql}"
    );
}

/// A non-binary `COLLATE` (e.g. `COLLATE NOCASE`) must degrade the operand's
/// inferred type to `Unknown` *and* emit a `NonPortableCollation` diagnostic
/// on the `COLLATE` clause — the pairing, not just each half in isolation.
/// Pins `.claude/unknown-census.toml`'s `collation.rs` non-binary-collate site.
#[test]
fn non_binary_collate_unknown_is_paired_with_non_portable_collation() {
    let model_path = PathBuf::from("/fake/project/models/bad_collation.sql");
    let sql = "SELECT 'foo' COLLATE NOCASE AS bad_collation";

    let (db, ws, sf) = build_db_single(model_path, sql);

    let schema = typed_model_schema(&db, ws, sf);
    let col = schema
        .columns
        .iter()
        .find(|c| c.name == "bad_collation")
        .expect("expected a bad_collation column in the inferred schema");
    let data_type = col
        .data_type
        .as_ref()
        .map(|tc| tc.data_type.clone())
        .expect("bad_collation column should have an inferred type");
    assert!(
        matches!(data_type, DataType::Unknown(_)),
        "expected non-binary COLLATE to infer as Unknown, got: {data_type:?}"
    );

    let diags = file_diagnostics(&db, ws, sf);
    let non_portable = diags
        .iter()
        .find(|d| d.code == Some(DiagnosticCode::NonPortableCollation));
    assert!(
        non_portable.is_some(),
        "expected a NonPortableCollation diagnostic paired with the Unknown inference for \
         non-binary COLLATE; got: {diags:#?}"
    );

    // The diagnostic must land on the `COLLATE` clause span
    // (`check_collation_diagnostics` uses `CollateExpr::syntax().text_range()`,
    // covering the whole `<expr> COLLATE <name>` clause), not just anywhere
    // in the file.
    let clause = "'foo' COLLATE NOCASE";
    let clause_start = sql
        .find(clause)
        .expect("fixture SQL should contain the COLLATE clause") as u32;
    let clause_end = clause_start + clause.len() as u32;
    let range = non_portable.expect("checked above").range;
    assert_eq!(
        u32::from(range.start()),
        clause_start,
        "expected NonPortableCollation diagnostic range to start at the COLLATE clause \
         (offset {clause_start}), got range {range:?} in SQL: {sql}"
    );
    assert_eq!(
        u32::from(range.end()),
        clause_end,
        "expected NonPortableCollation diagnostic range to end at the COLLATE clause \
         (offset {clause_end}), got range {range:?} in SQL: {sql}"
    );
}
