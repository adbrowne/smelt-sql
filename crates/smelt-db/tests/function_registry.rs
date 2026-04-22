//! Phase 3 (smelt-functions) — Salsa function signature index.
//!
//! These integration tests verify:
//!   1. `functions_in_file` / `resolve_function` index `smelt.define`
//!      signatures correctly.
//!   2. Duplicate function names across files emit a single
//!      `DuplicateFunctionDefinition` diagnostic, anchored at the second
//!      (sorted-by-path) declaration's name span.
//!   3. §20H: body edits do not invalidate `function_signature`'s output
//!      (value-equality check), while `function_body` *does* change.
//!
//! The tests build a `Database` directly (no `smelt-cli`) to keep the test
//! scoped to `smelt-db`.

use std::path::PathBuf;

use salsa::Setter;
use smelt_db::{
    file_diagnostics, function_body, function_signature, functions_in_file, resolve_function,
    workspace_function_diagnostics, Database, DiagnosticCode, SourceFile, Workspace,
};

/// Build a fresh `Database` seeded with the given `(path, contents)` files
/// and an empty `ProjectInput`. Returns the DB, workspace, and the
/// `SourceFile` handles in the same order as the input.
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
fn function_declarations_indexed() {
    // TDD test 1 from plan §Phase 3.
    let root = PathBuf::from("/fake/project");
    let path = root.join("functions").join("safe_divide.sql");
    let source = "smelt.define safe_divide(numerator: Expr<Numeric>, denominator: Expr<Numeric>) \
                  -> Expr<Double> AS (CASE WHEN denominator = 0 THEN NULL ELSE numerator / denominator END)\n";

    let (db, ws, files) = build_db(root.clone(), &[(path.clone(), source)]);
    let file = files[0];

    let sigs = functions_in_file(&db, file);
    assert_eq!(
        sigs.len(),
        1,
        "expected one indexed signature, got {sigs:?}"
    );

    let sig = &sigs[0];
    assert_eq!(sig.name, "safe_divide");
    assert_eq!(sig.params.len(), 2);
    assert_eq!(sig.params[0].name, "numerator");
    assert_eq!(sig.params[1].name, "denominator");

    let return_text = sig.return_type_text.as_deref().expect("annotated return");
    assert!(
        return_text.contains("Expr<Double>"),
        "expected Expr<Double> in return text, got {return_text:?}"
    );

    // resolve_function (workspace-wide)
    let resolved = resolve_function(&db, ws, "safe_divide".to_string());
    assert!(
        resolved.is_some(),
        "resolve_function should find safe_divide"
    );
    assert_eq!(resolved.unwrap().name, "safe_divide");

    // Missing name returns None.
    assert!(resolve_function(&db, ws, "does_not_exist".to_string()).is_none());
}

#[test]
fn duplicate_function_name_across_files_diagnostic() {
    // TDD test 2 from plan §Phase 3.
    let root = PathBuf::from("/fake/project");
    let a_path = root.join("functions").join("a.sql");
    let b_path = root.join("functions").join("b.sql");

    let (db, ws, files) = build_db(
        root.clone(),
        &[
            (a_path.clone(), "smelt.define foo(x) AS (x)\n"),
            (b_path.clone(), "smelt.define foo(x) AS (x)\n"),
        ],
    );
    let a_file = files[0];
    let b_file = files[1];

    // Workspace-level: exactly one diagnostic, anchored at b.sql.
    let ws_diags = workspace_function_diagnostics(&db, ws);
    assert_eq!(
        ws_diags.len(),
        1,
        "expected exactly one duplicate diagnostic, got {ws_diags:?}"
    );
    let (diag_path, diag) = &ws_diags[0];
    assert_eq!(diag_path, &b_path, "diagnostic should attach to b.sql");
    assert_eq!(diag.code, Some(DiagnosticCode::DuplicateFunctionDefinition));
    assert!(
        diag.message.contains("already defined"),
        "unexpected message: {}",
        diag.message
    );
    assert!(
        diag.message.contains(a_path.to_str().unwrap()),
        "diagnostic should reference a.sql's path, got: {}",
        diag.message
    );

    // The span should point at "foo" in b.sql (line 0, column 13..16).
    let sigs_b = functions_in_file(&db, b_file);
    assert_eq!(sigs_b.len(), 1);
    assert_eq!(
        diag.range, sigs_b[0].name_range,
        "diagnostic range must equal DEFINE_NAME span of the second declaration"
    );

    // Per-file: a.sql sees no duplicate diagnostic.
    let a_diags = file_diagnostics(&db, ws, a_file);
    assert!(
        a_diags
            .iter()
            .all(|d| d.code != Some(DiagnosticCode::DuplicateFunctionDefinition)),
        "a.sql should have no DuplicateFunctionDefinition diagnostic, got {a_diags:?}"
    );

    // Per-file: b.sql sees the duplicate diagnostic.
    let b_diags = file_diagnostics(&db, ws, b_file);
    let b_dup: Vec<_> = b_diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::DuplicateFunctionDefinition))
        .collect();
    assert_eq!(
        b_dup.len(),
        1,
        "b.sql should have exactly one dup diagnostic"
    );
    assert_eq!(b_dup[0].range, sigs_b[0].name_range);
}

#[test]
fn function_body_invalidation_separate_from_signature() {
    // TDD test 3 from plan §Phase 3 (§20H invariant).
    //
    // After editing only the body of a function (not its signature), the
    // signature-query output must be content-equal to the pre-edit output.
    // The body-query output may (and should) differ.
    //
    // We cannot rely on `Arc::ptr_eq` — Salsa 0.26 re-wraps the return into
    // a fresh `Arc` on each call, even when the underlying computation is
    // memoized. Value equality on `FunctionSig` is the observable §20H
    // invariant this test pins.
    let root = PathBuf::from("/fake/project");
    let path = root.join("functions").join("foo.sql");

    let mut db = Database::default();
    let project = db.set_project_input(root.clone(), String::new());
    let file = db.set_source_file(
        path.clone(),
        "smelt.define foo(x: Expr<Integer>) -> Expr<Integer> AS (x + 1)\n".to_string(),
        root.clone(),
    );
    db.set_workspace(vec![file], vec![project]);

    // Pre-edit: capture the signature and the body range.
    let sig_before = function_signature(&db, file, "foo".to_string()).expect("foo indexed");
    let body_before = function_body(&db, file, "foo".to_string()).expect("body present");

    // Edit only the body: change `x + 1` -> `x + 2`.
    file.set_text(&mut db)
        .to("smelt.define foo(x: Expr<Integer>) -> Expr<Integer> AS (x + 2)\n".to_string());

    let sig_after = function_signature(&db, file, "foo".to_string()).expect("foo still indexed");
    let body_after = function_body(&db, file, "foo".to_string()).expect("body still present");

    // §20H invariant: the signature value is unchanged despite the body edit.
    assert_eq!(
        *sig_before, *sig_after,
        "body-only edit must not change the signature value (sig_before={:?}, sig_after={:?})",
        sig_before, sig_after
    );

    // Sanity: the body range is the same (both bodies are the same length),
    // but a longer/shorter edit would shift it. We mainly care that the body
    // query re-ran — it cannot pointer-equal the old result because Salsa
    // re-wraps on each call. The semantically important claim is the
    // asymmetric one asserted above.
    //
    // For a stronger observation, edit to a body that changes the text length
    // so the returned `BodyRange` end differs.
    file.set_text(&mut db)
        .to("smelt.define foo(x: Expr<Integer>) -> Expr<Integer> AS (x + 100)\n".to_string());
    let body_after_len = function_body(&db, file, "foo".to_string()).expect("body present");
    assert_ne!(
        body_before, body_after_len,
        "body query must reflect the edited body text (ranges differ when body length changes)"
    );

    // Signature still equal.
    let sig_final = function_signature(&db, file, "foo".to_string()).expect("foo still indexed");
    assert_eq!(
        *sig_before, *sig_final,
        "signature still unchanged after longer body edit"
    );

    // Suppress unused-warning noise on the intermediate `body_after` binding.
    let _ = body_after;
}

/// Phase 4 fixture-backed test: `examples/broken/models/fn_bad_type_ref.sql`
/// declares a function with an unsupported sort (`TableExpr<T>`) and must
/// surface exactly one `InvalidFunctionTypeRef` diagnostic, anchored at the
/// `TypeRef` span. The plan explicitly flags this as "asserted via a targeted
/// unit test in Phase 4 until the Phase 6 harness arrives" — Phase 6 migrates
/// the assertion into `crates/smelt-cli/tests/broken_function_diagnostics.rs`.
#[test]
fn broken_fixture_bad_type_ref_emits_diagnostic() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let broken_dir = manifest_dir
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("examples")
        .join("broken")
        .join("models");
    let path = broken_dir.join("fn_bad_type_ref.sql");
    let content = std::fs::read_to_string(&path).expect("fn_bad_type_ref.sql exists");

    let root = broken_dir.parent().unwrap().to_path_buf();
    let (db, ws, files) = build_db(root, &[(path.clone(), &content)]);
    let file = files[0];

    let diags = file_diagnostics(&db, ws, file);
    let invalid: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::InvalidFunctionTypeRef))
        .collect();
    assert_eq!(
        invalid.len(),
        1,
        "fn_bad_type_ref.sql should emit exactly one InvalidFunctionTypeRef diagnostic, got {diags:?}"
    );
    assert!(
        invalid[0].message.contains("TableExpr"),
        "diagnostic should name the unsupported sort, got: {}",
        invalid[0].message
    );
}

/// Fixture-backed test: the `examples/broken/` duplicate-define pair produces
/// the expected `DuplicateFunctionDefinition` diagnostic end-to-end. Phase 6
/// will migrate this assertion into the unified
/// `crates/smelt-cli/tests/broken_function_diagnostics.rs` harness.
#[test]
fn broken_fixture_duplicate_defines_emit_diagnostic() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let broken_dir = manifest_dir
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("examples")
        .join("broken")
        .join("models");

    let a_path = broken_dir.join("fn_duplicate_define.sql");
    let b_path = broken_dir.join("fn_duplicate_define_other.sql");

    // a.sql sorts before b.sql alphabetically — b.sql is the colliding
    // declaration. We load both directly from disk to pin the fixture
    // content as the assertion target.
    let a_content = std::fs::read_to_string(&a_path).expect("fn_duplicate_define.sql exists");
    let b_content = std::fs::read_to_string(&b_path).expect("fn_duplicate_define_other.sql exists");

    let root = broken_dir.parent().unwrap().to_path_buf();
    let (db, ws, files) = build_db(
        root,
        &[(a_path.clone(), &a_content), (b_path.clone(), &b_content)],
    );
    let b_file = files[1];

    let diags = file_diagnostics(&db, ws, b_file);
    let dup: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::DuplicateFunctionDefinition))
        .collect();
    assert_eq!(
        dup.len(),
        1,
        "fn_duplicate_define_other.sql should emit exactly one duplicate diagnostic, got {diags:?}"
    );
    assert!(
        dup[0].message.contains("shared_name"),
        "diagnostic should name the duplicated function, got: {}",
        dup[0].message
    );
}
