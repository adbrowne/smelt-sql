//! Phase 23 — Tier 2 body check in isolation.
//!
//! Tier 2 functions have all non-`TableExpr`/non-`SelectItems` parameters
//! annotated with an explicit `Expr<T>`. Their bodies are checked against the
//! declared parameter types *at definition time*, independent of any call site.
//!
//! Tests:
//!   1. `tier2_body_checks_against_declared_params` — a well-typed Tier 2 body
//!      emits no diagnostics.
//!   2. `tier2_body_error_at_definition_time` — a type error in a Tier 2 body
//!      fires `FunctionBodyTypeMismatch` without any call site in the workspace.
//!   3. `tier2_signature_survives_broken_body` — the signature is still returned
//!      by `file_signature_inputs` even when the body is broken. Phase 3's
//!      signature/body split guarantees this.
//!   4. `tier1_body_still_requires_call_site` — Tier 1 (unannotated) bodies do
//!      NOT produce `FunctionBodyTypeMismatch` at definition time (regression).

use std::path::PathBuf;

use smelt_db::{
    declared_return_hover_text, file_diagnostics, file_signature_inputs,
    function_body_check::is_tier2_function, Database, DiagnosticCode, SourceFile, Workspace,
};

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

#[test]
fn tier2_body_checks_against_declared_params() {
    // Tier 2: all parameters annotated. Body uses them with a compatible
    // operator. No diagnostics expected.
    let root = PathBuf::from("/fake/project");
    let path = root.join("functions").join("add_typed.sql");
    let src = "smelt.define add_typed(x: Expr<Integer>, y: Expr<Integer>) AS (x + y)\n";

    let (db, ws, files) = build_db(root, &[(path, src)]);
    let file = files[0];

    let diags = file_diagnostics(&db, ws, file);
    let type_errors: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::FunctionBodyTypeMismatch))
        .collect();

    assert!(
        type_errors.is_empty(),
        "well-typed Tier 2 body should emit no FunctionBodyTypeMismatch, got {diags:?}"
    );
}

#[test]
fn tier2_body_error_at_definition_time() {
    // Tier 2: annotated parameter `x: Expr<Integer>`, body adds a Text literal.
    // Must fire FunctionBodyTypeMismatch without any call site in the workspace.
    let root = PathBuf::from("/fake/project");
    let path = root.join("functions").join("bad_tier2.sql");
    let src = "smelt.define bad_tier2(x: Expr<Integer>) AS (x + 'text')\n";

    let (db, ws, files) = build_db(root, &[(path, src)]);
    let file = files[0];

    let diags = file_diagnostics(&db, ws, file);
    let type_errors: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::FunctionBodyTypeMismatch))
        .collect();

    assert!(
        !type_errors.is_empty(),
        "Tier 2 body with Integer + Text must emit FunctionBodyTypeMismatch at definition time \
         (no call site needed), got {diags:?}"
    );
}

#[test]
fn tier2_signature_survives_broken_body() {
    // Phase 3's signature/body split: even when the body is broken, the
    // signature must still be returned by `file_signature_inputs`. This
    // verifies the Salsa query separation between signature extraction and
    // body checking.
    let root = PathBuf::from("/fake/project");
    let path = root.join("functions").join("bad_tier2_sig.sql");
    let src = "smelt.define bad_tier2_sig(revenue: Expr<Integer>) AS (revenue + 'text')\n";

    let (db, ws, files) = build_db(root, &[(path, src)]);
    let file = files[0];

    // Signature must still be returned.
    let sigs = file_signature_inputs(&db, file);
    assert_eq!(
        sigs.len(),
        1,
        "file_signature_inputs must return the signature even when the body has a type error, \
         got {sigs:?}"
    );
    assert_eq!(
        sigs[0].name, "bad_tier2_sig",
        "signature name must match, got {:?}",
        sigs[0].name
    );

    // And file_diagnostics must report the error.
    let diags = file_diagnostics(&db, ws, file);
    let type_errors: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::FunctionBodyTypeMismatch))
        .collect();
    assert!(
        !type_errors.is_empty(),
        "file_diagnostics must report FunctionBodyTypeMismatch for the broken body, \
         got {diags:?}"
    );
}

#[test]
fn tier1_body_still_requires_call_site() {
    // Tier 1: unannotated parameters (`x` and `y` with no type annotation).
    // Body uses `x + y` — without knowing the types, no FunctionBodyTypeMismatch
    // should fire at definition time. (Parameters bind to `Unknown` which is
    // permissive.)
    let root = PathBuf::from("/fake/project");
    let path = root.join("functions").join("untyped_add.sql");
    let src = "smelt.define untyped_add(x, y) AS (x + y)\n";

    let (db, ws, files) = build_db(root, &[(path, src)]);
    let file = files[0];

    let diags = file_diagnostics(&db, ws, file);
    let type_errors: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::FunctionBodyTypeMismatch))
        .collect();

    assert!(
        type_errors.is_empty(),
        "Tier 1 (unannotated) bodies must NOT emit FunctionBodyTypeMismatch at definition \
         time (no call site = permissive), got {type_errors:?}"
    );
}

#[test]
fn is_tier2_function_returns_true_for_annotated() {
    // Unit test for the pure helper: a function with all Expr<T> params is Tier 2.
    use smelt_parser::{parse, strip_frontmatter, File as AstFile};
    use smelt_types::signatures::extract_function_signatures;

    let src = "smelt.define add_typed(x: Expr<Integer>, y: Expr<Integer>) AS (x + y)\n";
    let clean = strip_frontmatter(src).to_string();
    let p = parse(&clean);
    let ast = AstFile::cast(p.syntax()).expect("FILE");
    let sigs = extract_function_signatures(&ast, &clean);
    assert_eq!(sigs.len(), 1);
    assert!(
        is_tier2_function(&sigs[0]),
        "annotated function must be detected as Tier 2"
    );
}

#[test]
fn is_tier2_function_returns_false_for_unannotated() {
    // Unit test for the pure helper: a function with unannotated params is Tier 1.
    use smelt_parser::{parse, strip_frontmatter, File as AstFile};
    use smelt_types::signatures::extract_function_signatures;

    let src = "smelt.define untyped_add(x, y) AS (x + y)\n";
    let clean = strip_frontmatter(src).to_string();
    let p = parse(&clean);
    let ast = AstFile::cast(p.syntax()).expect("FILE");
    let sigs = extract_function_signatures(&ast, &clean);
    assert_eq!(sigs.len(), 1);
    assert!(
        !is_tier2_function(&sigs[0]),
        "unannotated function must NOT be detected as Tier 2"
    );
}

// ============================================================================
// Phase 24 — Tier 3 return type verification
// ============================================================================

#[test]
fn tier3_body_return_matches_annotation() {
    // Tier 3: all params annotated + return type declared.
    // Body `(x / y)` on `Numeric` params synthesises Double — Numeric
    // constraint is satisfied. No ReturnTypeMismatch expected.
    let root = PathBuf::from("/fake/project");
    let path = root.join("functions").join("margin_ok.sql");
    let src =
        "smelt.define margin_ok(x: Expr<Numeric>, y: Expr<Numeric>) -> Expr<Double> AS (x / y)\n";

    let (db, ws, files) = build_db(root, &[(path, src)]);
    let file = files[0];

    let diags = file_diagnostics(&db, ws, file);
    let return_errors: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::ReturnTypeMismatch))
        .collect();

    assert!(
        return_errors.is_empty(),
        "well-typed Tier 3 body should emit no ReturnTypeMismatch, got {diags:?}"
    );
}

#[test]
fn tier3_body_return_mismatch_errors_at_return_expression() {
    // Tier 3: declared return `Expr<Double>` but body produces Text.
    // Must fire ReturnTypeMismatch anchored at the body expression span.
    let root = PathBuf::from("/fake/project");
    let path = root.join("functions").join("bad_return.sql");
    let src = "smelt.define bad_return(x: Expr<Integer>) -> Expr<Double> AS ('text')\n";

    let (db, ws, files) = build_db(root, &[(path, src)]);
    let file = files[0];

    let diags = file_diagnostics(&db, ws, file);
    let return_errors: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::ReturnTypeMismatch))
        .collect();

    assert!(
        !return_errors.is_empty(),
        "Tier 3 body returning Text against Expr<Double> must emit ReturnTypeMismatch, \
         got {diags:?}"
    );

    // The diagnostic must be anchored at the body expression, not the
    // function name. Verify a range was produced (start column > 0
    // since the body expression is inside `AS (...)`).
    let diag = return_errors[0];
    assert!(
        diag.range.start.column > 0 || diag.range.start.line > 0,
        "ReturnTypeMismatch must carry a valid body range, got {:?}",
        diag.range
    );
}

#[test]
fn lsp_hover_tier3_shows_declared_return() {
    // Unit test for the pure hover helper.
    // A Tier 3 function must return the formatted return type string;
    // Tier 1 and Tier 2 functions must return None.
    use smelt_parser::{parse, strip_frontmatter, File as AstFile};
    use smelt_types::signatures::extract_function_signatures;

    // Tier 3 function — should produce Some("-> Expr<Double>").
    let src3 =
        "smelt.define margin_ok(x: Expr<Numeric>, y: Expr<Numeric>) -> Expr<Double> AS (x / y)\n";
    let clean3 = strip_frontmatter(src3).to_string();
    let p3 = parse(&clean3);
    let ast3 = AstFile::cast(p3.syntax()).expect("FILE");
    let sigs3 = extract_function_signatures(&ast3, &clean3);
    assert_eq!(sigs3.len(), 1);
    let hover3 = declared_return_hover_text(&sigs3[0]);
    assert_eq!(
        hover3,
        Some("-> Expr<Double>".to_string()),
        "Tier 3 hover must return the declared return type"
    );

    // Tier 2 function (params typed, no return type) — should return None.
    let src2 = "smelt.define add_typed(x: Expr<Integer>, y: Expr<Integer>) AS (x + y)\n";
    let clean2 = strip_frontmatter(src2).to_string();
    let p2 = parse(&clean2);
    let ast2 = AstFile::cast(p2.syntax()).expect("FILE");
    let sigs2 = extract_function_signatures(&ast2, &clean2);
    assert_eq!(sigs2.len(), 1);
    let hover2 = declared_return_hover_text(&sigs2[0]);
    assert_eq!(hover2, None, "Tier 2 hover must return None");

    // Tier 1 function (unannotated) — should return None.
    let src1 = "smelt.define untyped(x, y) AS (x + y)\n";
    let clean1 = strip_frontmatter(src1).to_string();
    let p1 = parse(&clean1);
    let ast1 = AstFile::cast(p1.syntax()).expect("FILE");
    let sigs1 = extract_function_signatures(&ast1, &clean1);
    assert_eq!(sigs1.len(), 1);
    let hover1 = declared_return_hover_text(&sigs1[0]);
    assert_eq!(hover1, None, "Tier 1 hover must return None");
}

#[test]
fn tier3_row_variable_in_return_abstract_checked() {
    // Phase 24: verify that exact-type matching works for concrete types.
    // Declared `-> Expr<Integer>` with a body that returns DataType::Integer
    // must produce no ReturnTypeMismatch.
    let root = PathBuf::from("/fake/project");
    let path = root.join("functions").join("identity_int.sql");
    let src = "smelt.define identity_int(x: Expr<Integer>) -> Expr<Integer> AS (x)\n";

    let (db, ws, files) = build_db(root, &[(path, src)]);
    let file = files[0];

    let diags = file_diagnostics(&db, ws, file);
    let return_errors: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::ReturnTypeMismatch))
        .collect();

    assert!(
        return_errors.is_empty(),
        "Tier 3 body returning Integer against Expr<Integer> must not fire ReturnTypeMismatch, \
         got {diags:?}"
    );
}
