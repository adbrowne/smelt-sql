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
        usize::from(diag.range.start()) > 0,
        "ReturnTypeMismatch must carry a valid body range (start > 0), got {:?}",
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

// ============================================================================
// Phase 25 — Skip body expansion for Tier 2/3 call sites
// ============================================================================

/// Build a two-file workspace: a function-definition file and a caller file.
/// The caller file contains a SELECT statement so `smelt.functions.*` calls are
/// dispatched with a resolvable expression context.
fn build_two_file_db(
    fn_src: &str,
    caller_src: &str,
) -> (Database, Workspace, SourceFile, SourceFile) {
    let root = PathBuf::from("/fake/project");
    let fn_path = root.join("functions").join("fn_def.sql");
    let caller_path = root.join("models").join("caller_model.sql");

    let files = [(fn_path.clone(), fn_src), (caller_path.clone(), caller_src)];
    let (db, ws, handles) = build_db(root, &files);
    let fn_file = handles[0];
    let caller_file = handles[1];
    (db, ws, fn_file, caller_file)
}

#[test]
fn tier2_call_arg_checked_against_declared_param() {
    // Tier 2 function: `mul_typed(x: Expr<Integer>, y: Expr<Integer>)`.
    // Caller passes a Text literal `'hello'` for `x` — type mismatch.
    // Expect: ArgTypeMismatch on the caller file, mentioning "x" and/or "Integer".
    let fn_src = "smelt.define mul_typed(x: Expr<Integer>, y: Expr<Integer>) AS (x * y)\n";
    // Use a literal so the arg type is concrete (Text), not Unknown.
    let caller_src = "SELECT smelt.functions.mul_typed('hello', 1) AS r\n";

    let (db, ws, _fn_file, caller_file) = build_two_file_db(fn_src, caller_src);
    let diags = file_diagnostics(&db, ws, caller_file);

    let arg_errors: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::ArgTypeMismatch))
        .collect();

    assert!(
        !arg_errors.is_empty(),
        "Tier 2 call with Text literal for Integer param must emit ArgTypeMismatch, got {diags:?}"
    );
    // Must name the param and expected type.
    let msg = &arg_errors[0].message;
    assert!(
        msg.contains('x') || msg.contains("Integer"),
        "ArgTypeMismatch message must mention param or type, got: {msg}"
    );
}

#[test]
fn tier3_call_arg_checked_same_as_tier2() {
    // Tier 3 function: `clamp_tier3(x: Expr<Integer>) -> Expr<Integer>`.
    // Caller passes a Text literal — type mismatch.
    // Expect: ArgTypeMismatch on the caller file.
    let fn_src = "smelt.define clamp_tier3(x: Expr<Integer>) -> Expr<Integer> AS (x)\n";
    let caller_src = "SELECT smelt.functions.clamp_tier3('hello') AS r\n";

    let (db, ws, _fn_file, caller_file) = build_two_file_db(fn_src, caller_src);
    let diags = file_diagnostics(&db, ws, caller_file);

    let arg_errors: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::ArgTypeMismatch))
        .collect();

    assert!(
        !arg_errors.is_empty(),
        "Tier 3 call with Text literal for Integer param must emit ArgTypeMismatch, got {diags:?}"
    );
}

#[test]
fn tier1_call_still_uses_expansion() {
    // Tier 1 (unannotated) function: `broken_raw(x)` with body referencing
    // an undefined identifier `undefined_var`. Expansion cascades the
    // UnknownIdentifier error to the call site with an ExpansionFrames payload.
    let fn_src = "smelt.define broken_raw(x) AS (x + undefined_var)\n";
    let caller_src = "SELECT smelt.functions.broken_raw(42) AS r\n";

    let (db, ws, _fn_file, caller_file) = build_two_file_db(fn_src, caller_src);
    let diags = file_diagnostics(&db, ws, caller_file);

    // Expansion should cascade the UnknownIdentifier from the body to the call site,
    // producing a diagnostic with an ExpansionFrames payload at the caller.
    let expanded_errors: Vec<_> = diags
        .iter()
        .filter(|d| {
            d.code == Some(DiagnosticCode::UnknownIdentifier)
                && matches!(&d.data, Some(smelt_db::DiagnosticData::ExpansionFrames(_)))
        })
        .collect();

    assert!(
        !expanded_errors.is_empty(),
        "Tier 1 call must cascade body expansion errors (with ExpansionFrames) to caller, got {diags:?}"
    );
}

#[test]
fn checking_mode_no_expansion_performed() {
    // Tier 2 function with a broken body: `broken_tier2(x: Expr<Integer>)` body
    // has `x + 'text'` — a type error caught at definition time (Phase 23).
    // Caller passes a correct Integer literal 42.
    // Observable proof that expansion was skipped: if body_lookup were called, the
    // FunctionBodyTypeMismatch from `x + 'text'` would cascade to the caller file.
    // Asserting it is absent confirms the Tier 2 call-site skips expansion entirely.
    let fn_src = "smelt.define broken_tier2(x: Expr<Integer>) AS (x + 'text')\n";
    let caller_src = "SELECT smelt.functions.broken_tier2(42) AS r\n";

    let (db, ws, _fn_file, caller_file) = build_two_file_db(fn_src, caller_src);
    let diags = file_diagnostics(&db, ws, caller_file);

    let body_errors: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::FunctionBodyTypeMismatch))
        .collect();

    assert!(
        body_errors.is_empty(),
        "Tier 2 call with correct arg must NOT cascade body errors to caller (expansion skipped \
         for Tier 2 means body_lookup is never called), got {diags:?}"
    );
}
