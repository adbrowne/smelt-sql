//! Phase 5 (smelt-functions) — Tier 1 body type-check with parameter binding.
//!
//! These integration tests verify that a `smelt.define` body is type-checked
//! against its declared parameters:
//!
//!   1. `safe_divide_body_checks_ok` — a correctly-typed body produces zero
//!      body-diagnostics.
//!   2. `body_type_error_reported` — `numerator + "text"` emits one
//!      `FunctionBodyTypeMismatch` diagnostic whose range covers the *inner*
//!      bad subexpression, not the whole body.
//!   3. `body_references_unknown_param` — referencing an undeclared identifier
//!      emits one `UnknownIdentifier` diagnostic at the bad identifier's span.
//!   4. `duplicate_param_name_is_error` — two parameters sharing a name emit
//!      one `DuplicateParameterName` diagnostic anchored at the second
//!      occurrence's name span.
//!
//! The harness also covers the Phase 5 fixture files under `examples/broken/`
//! — these are asserted directly here until Phase 6 migrates them into the
//! unified `crates/smelt-cli/tests/broken_function_diagnostics.rs` harness.

use std::path::PathBuf;

use smelt_db::{file_diagnostics, Database, DiagnosticCode, SourceFile, Workspace};

/// Build a fresh `Database` seeded with the given `(path, contents)` files.
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

fn body_diags(
    db: &Database,
    ws: Workspace,
    file: SourceFile,
    code: DiagnosticCode,
) -> Vec<smelt_db::Diagnostic> {
    file_diagnostics(db, ws, file)
        .into_iter()
        .filter(|d| d.code == Some(code))
        .collect()
}

#[test]
fn safe_divide_body_checks_ok() {
    // TDD test 1: a canonical `safe_divide` body should produce zero
    // function-body diagnostics. The body uses CAST + `/` on two numeric
    // params, which is a type-clean expression.
    let root = PathBuf::from("/fake/project");
    let path = root.join("functions").join("safe_divide.sql");
    let src = "smelt.define safe_divide(numerator: Expr<Numeric>, denominator: Expr<Numeric>) \
               -> Expr<Double> AS (CAST(numerator AS DOUBLE) / denominator)\n";

    let (db, ws, files) = build_db(root, &[(path, src)]);
    let file = files[0];

    let mismatches = body_diags(&db, ws, file, DiagnosticCode::FunctionBodyTypeMismatch);
    let unknowns = body_diags(&db, ws, file, DiagnosticCode::UnknownIdentifier);
    let dup_params = body_diags(&db, ws, file, DiagnosticCode::DuplicateParameterName);

    assert!(
        mismatches.is_empty(),
        "safe_divide body should have no type-mismatch diagnostics, got {mismatches:?}"
    );
    assert!(
        unknowns.is_empty(),
        "safe_divide body should have no unknown-identifier diagnostics, got {unknowns:?}"
    );
    assert!(
        dup_params.is_empty(),
        "safe_divide body should have no duplicate-param diagnostics, got {dup_params:?}"
    );
}

#[test]
fn body_type_error_reported() {
    // TDD test 2: `numerator + 'text'` — Integer + Text should emit exactly
    // one `FunctionBodyTypeMismatch` whose range is the inner bad
    // subexpression (not the whole body). Review-checklist item.
    let root = PathBuf::from("/fake/project");
    let path = root.join("functions").join("bad.sql");
    let src = "smelt.define bad(numerator: Expr<Integer>) AS (numerator + 'text')\n";

    let (db, ws, files) = build_db(root, &[(path, src)]);
    let file = files[0];

    let diags = body_diags(&db, ws, file, DiagnosticCode::FunctionBodyTypeMismatch);
    assert_eq!(
        diags.len(),
        1,
        "expected exactly one FunctionBodyTypeMismatch, got {diags:?}"
    );

    let diag = &diags[0];

    // Data-less for Phase 5 — frame stack lands in Phase 6. Review item:
    // "zero-error bodies don't allocate an empty frame stack" — and we
    // carry no `DiagnosticData` here either.
    assert!(
        diag.data.is_none(),
        "Phase 5 body diagnostics must carry no DiagnosticData (Phase 6 adds ExpansionFrames)"
    );

    // The range must be the inner bad subexpression — the `numerator +
    // 'text'` binary node — NOT the whole function body. The body includes
    // the opening `(`; the binary expression starts after that.
    //
    // Body starts at byte offset of `(numerator + 'text')`; the binary
    // expression starts at `numerator`. The diagnostic span should be a
    // proper subset of the body span.
    //
    // Pin the exact span so the test fails loudly if the anchor moves:
    // `numerator + 'text'` lives on line 0 of the stripped source.
    let text = "smelt.define bad(numerator: Expr<Integer>) AS (numerator + 'text')\n";
    let inner_start = text.find("numerator + 'text'").unwrap();
    let inner_end = inner_start + "numerator + 'text'".len();

    // Convert expected byte offsets to column positions (line 0 throughout).
    assert_eq!(
        diag.range.start.line, 0,
        "expected inner-subexpression span on line 0, got {:?}",
        diag.range
    );
    assert_eq!(
        diag.range.end.line, 0,
        "expected inner-subexpression span on line 0, got {:?}",
        diag.range
    );
    assert_eq!(
        diag.range.start.column as usize, inner_start,
        "expected diagnostic start column at byte offset of inner subexpression, got {:?}",
        diag.range
    );
    assert_eq!(
        diag.range.end.column as usize, inner_end,
        "expected diagnostic end column at end of inner subexpression, got {:?}",
        diag.range
    );
}

#[test]
fn body_references_unknown_param() {
    // TDD test 3: referencing `z` when only `x: Expr<Integer>` is declared
    // emits one `UnknownIdentifier` whose span covers the bad identifier.
    let root = PathBuf::from("/fake/project");
    let path = root.join("functions").join("bad.sql");
    let src = "smelt.define bad(x: Expr<Integer>) AS (x + z)\n";

    let (db, ws, files) = build_db(root, &[(path, src)]);
    let file = files[0];

    let diags = body_diags(&db, ws, file, DiagnosticCode::UnknownIdentifier);
    assert_eq!(
        diags.len(),
        1,
        "expected exactly one UnknownIdentifier diagnostic, got {diags:?}"
    );
    let diag = &diags[0];
    assert!(
        diag.message.contains('z'),
        "diagnostic message should mention the unknown identifier `z`, got: {}",
        diag.message
    );

    // Span should cover the `z` identifier exactly.
    let z_start = src.find(" z)").unwrap() + 1;
    let z_end = z_start + 1;
    assert_eq!(
        diag.range.start.line, 0,
        "expected identifier span on line 0, got {:?}",
        diag.range
    );
    assert_eq!(
        diag.range.start.column as usize, z_start,
        "expected span start at the `z` identifier, got {:?}",
        diag.range
    );
    assert_eq!(
        diag.range.end.column as usize, z_end,
        "expected span end just after `z`, got {:?}",
        diag.range
    );
}

#[test]
fn duplicate_param_name_is_error() {
    // TDD test 4: two params named `x` — emit one `DuplicateParameterName`
    // anchored at the second occurrence's name span.
    let root = PathBuf::from("/fake/project");
    let path = root.join("functions").join("dup.sql");
    let src = "smelt.define f(x: Expr<Integer>, x: Expr<Integer>) AS (x)\n";

    let (db, ws, files) = build_db(root, &[(path, src)]);
    let file = files[0];

    let diags = body_diags(&db, ws, file, DiagnosticCode::DuplicateParameterName);
    assert_eq!(
        diags.len(),
        1,
        "expected exactly one DuplicateParameterName diagnostic, got {diags:?}"
    );

    let diag = &diags[0];
    assert!(
        diag.message.contains('x'),
        "diagnostic message should mention the duplicate param name `x`, got: {}",
        diag.message
    );

    // The second `x` sits after the `,` + space + (then the 8-char offset
    // from the `,`). Just verify the span starts at the second `x`, not the
    // first.
    let first_x = src.find("(x:").unwrap() + 1;
    let second_x = src.rfind("x: Expr<Integer>)").unwrap();
    assert_ne!(first_x, second_x, "test sanity: the two positions differ");
    assert_eq!(
        diag.range.start.line, 0,
        "expected diagnostic on line 0, got {:?}",
        diag.range
    );
    assert_eq!(
        diag.range.start.column as usize, second_x,
        "diagnostic should be anchored at the second `x`, got {:?}",
        diag.range
    );
    assert_eq!(
        diag.range.end.column as usize,
        second_x + 1,
        "diagnostic span should be exactly one char wide, got {:?}",
        diag.range
    );
}

/// Fixture-backed assertion: the Phase 5 broken fixtures emit the expected
/// diagnostics. This test is temporary — Phase 6 migrates these rows into the
/// unified `crates/smelt-cli/tests/broken_function_diagnostics.rs` harness.
#[test]
fn broken_fixtures_emit_expected_diagnostics() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let broken_dir = manifest_dir
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("examples")
        .join("broken")
        .join("models");

    // 1. fn_body_type_mismatch.sql → FunctionBodyTypeMismatch.
    {
        let path = broken_dir.join("fn_body_type_mismatch.sql");
        let content = std::fs::read_to_string(&path).expect("fn_body_type_mismatch.sql must exist");
        let root = broken_dir.parent().unwrap().to_path_buf();
        let (db, ws, files) = build_db(root, &[(path, &content)]);
        let file = files[0];

        let mismatches = body_diags(&db, ws, file, DiagnosticCode::FunctionBodyTypeMismatch);
        assert_eq!(
            mismatches.len(),
            1,
            "fn_body_type_mismatch.sql should emit one FunctionBodyTypeMismatch, got {mismatches:?}"
        );
    }

    // 2. fn_unknown_param.sql → UnknownIdentifier for `z`.
    {
        let path = broken_dir.join("fn_unknown_param.sql");
        let content = std::fs::read_to_string(&path).expect("fn_unknown_param.sql must exist");
        let root = broken_dir.parent().unwrap().to_path_buf();
        let (db, ws, files) = build_db(root, &[(path, &content)]);
        let file = files[0];

        let unknowns = body_diags(&db, ws, file, DiagnosticCode::UnknownIdentifier);
        assert_eq!(
            unknowns.len(),
            1,
            "fn_unknown_param.sql should emit one UnknownIdentifier, got {unknowns:?}"
        );
        assert!(
            unknowns[0].message.contains('z'),
            "fn_unknown_param.sql diagnostic should name `z`, got: {}",
            unknowns[0].message
        );
    }
}
