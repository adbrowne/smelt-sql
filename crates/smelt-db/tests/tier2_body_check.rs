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
    file_diagnostics, file_signature_inputs, function_body_check::is_tier2_function, Database,
    DiagnosticCode, SourceFile, Workspace,
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
