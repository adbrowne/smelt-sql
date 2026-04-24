//! Phase 27 integration tests — bidirectional generics (§16 #14, Decision 14).
//!
//! Verifies that the `expected_return` context propagates through
//! `check_tier3_return_type` → `try_registry_inference` so that built-in
//! generic calls (e.g. `COALESCE`) widen their type-variable binding when the
//! surrounding function body declares a concrete return type.

use std::path::PathBuf;

use smelt_db::{file_diagnostics, Database, DiagnosticCode, SourceFile, Workspace};

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

fn all_type_diags(db: &Database, ws: Workspace, file: SourceFile) -> Vec<smelt_db::Diagnostic> {
    file_diagnostics(db, ws, file)
        .into_iter()
        .filter(|d| {
            matches!(
                d.code,
                Some(DiagnosticCode::ArgTypeMismatch)
                    | Some(DiagnosticCode::ReturnTypeMismatch)
                    | Some(DiagnosticCode::FunctionBodyTypeMismatch)
            )
        })
        .collect()
}

/// When a Tier 3 function declares `-> Expr<Double>` and the body is
/// `COALESCE(integer_col, integer_col)`, the expected return type should
/// propagate into `try_registry_inference` so that COALESCE widens to
/// `Double` and the Tier 3 return-type check passes with zero diagnostics.
#[test]
fn coalesce_widens_to_declared_double_return() {
    let root = PathBuf::from("/fake/coalesce_bidir");
    let fn_path = root.join("functions").join("coalesce_wrapper.sql");
    // Tier 3: params and return all annotated. Body is COALESCE of two
    // Integer params. Without bidirectional inference, COALESCE → Integer
    // which would mismatch the declared `-> Expr<Double>`. With Phase 27,
    // the Double context widens COALESCE to Double — no diagnostic.
    let fn_src = "smelt.define coalesce_wrapper(a: Expr<Numeric>, b: Expr<Numeric>) \
                  -> Expr<Double> AS (COALESCE(a, b))\n";

    let (db, ws, files) = build_db(root, &[(fn_path.clone(), fn_src)]);
    let fn_file = files[0];

    let diags = all_type_diags(&db, ws, fn_file);
    assert!(
        diags.is_empty(),
        "expected no type diagnostics for COALESCE widening to Double, got {diags:?}"
    );
}

/// A Tier 3 function that wraps COALESCE but declares `-> Expr<Text>` when
/// the args are `Expr<Numeric>` should produce a ReturnTypeMismatch — the
/// expected Double context can't coerce to Text.
#[test]
fn coalesce_wrong_return_type_emits_diagnostic() {
    let root = PathBuf::from("/fake/coalesce_wrong_return");
    let fn_path = root.join("functions").join("bad_coalesce.sql");
    // Declares `-> Expr<Text>` but body is COALESCE of Numeric params.
    // COALESCE(Numeric, Numeric) infers to a numeric type, not Text.
    // The return-type mismatch should surface.
    let fn_src = "smelt.define bad_coalesce(a: Expr<Numeric>, b: Expr<Numeric>) \
                  -> Expr<Text> AS (COALESCE(a, b))\n";

    let (db, ws, files) = build_db(root, &[(fn_path.clone(), fn_src)]);
    let fn_file = files[0];

    let diags = all_type_diags(&db, ws, fn_file);
    let return_diags: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::ReturnTypeMismatch))
        .collect();
    assert!(
        !return_diags.is_empty(),
        "expected a ReturnTypeMismatch diagnostic for Numeric→Text, got none. All diags: {diags:?}"
    );
}
