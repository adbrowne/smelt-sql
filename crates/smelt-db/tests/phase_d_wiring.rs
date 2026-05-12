//! Phase D (meta-language) — orchestration-layer wiring tests.
//!
//! These integration tests verify that the Phase D diagnostic checks are
//! plumbed through the production `check_file_diagnostics` / `file_diagnostics`
//! pipeline so that a user writing `smelt.models.with_tag(42)` in a real model
//! file receives a diagnostic rather than silence.
//!
//! Covered cases:
//!
//!  1. `with_tag_integer_arg_in_model_body_emits_diagnostic` — a model SELECT
//!     containing `smelt.models.with_tag(42)` emits exactly one
//!     `WithTagRequiresText` diagnostic via the orchestration pipeline (not just
//!     the unit-test harness).
//!  2. `with_tag_string_literal_in_model_body_no_diagnostic` — a model SELECT
//!     containing `smelt.models.with_tag('core')` emits zero Phase D diagnostics.
//!  3. `wide_reflection_unknown_accessor_in_model_body` — `smelt.models.bogus()`
//!     emits one `WideReflectionUnknownAccessor` in a model file.
//!  4. `model_ref_unknown_field_in_hof_lambda_in_model_body` — a model SELECT
//!     with `map(smelt.models.with_tag('x'), fn m => m.bogus)` emits one
//!     `ModelRefFieldUnknown` via the HOF dispatcher.
//!  5. `source_ref_unknown_field_in_hof_lambda_in_model_body` — analogous for
//!     `smelt.sources.with_tag('x')` → `SourceRefFieldUnknown`.
//!  6. `model_ref_known_field_in_hof_lambda_no_diagnostic` — `m.name` is a
//!     valid `ModelRef` field; no Phase D diagnostic emitted.

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

fn diags_with_code(
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

/// Acceptance criterion: calling `check_file_diagnostics` on a file containing
/// `smelt.models.with_tag(42)` produces exactly one `WithTagRequiresText`
/// diagnostic.
#[test]
fn with_tag_integer_arg_in_model_body_emits_diagnostic() {
    let root = PathBuf::from("/fake/project");
    let model_path = root.join("models").join("reflect.sql");
    // `42` is an integer literal — not compile-time Text → WithTagRequiresText.
    let model_src = "SELECT smelt.models.with_tag(42) AS models FROM t\n";

    let (db, ws, files) = build_db(root, &[(model_path, model_src)]);
    let model_file = files[0];

    let diags = diags_with_code(&db, ws, model_file, DiagnosticCode::WithTagRequiresText);
    assert_eq!(
        diags.len(),
        1,
        "expected exactly one WithTagRequiresText diagnostic for smelt.models.with_tag(42), \
         got {diags:?}"
    );
}

/// `smelt.models.with_tag('core')` (a bare string literal) must produce zero
/// Phase D diagnostics.
#[test]
fn with_tag_string_literal_in_model_body_no_diagnostic() {
    let root = PathBuf::from("/fake/project");
    let model_path = root.join("models").join("reflect.sql");
    let model_src = "SELECT smelt.models.with_tag('core') AS models FROM t\n";

    let (db, ws, files) = build_db(root, &[(model_path, model_src)]);
    let model_file = files[0];

    let all_diags = file_diagnostics(&db, ws, model_file);
    let phase_d_diags: Vec<_> = all_diags
        .iter()
        .filter(|d| {
            matches!(
                d.code,
                Some(DiagnosticCode::WithTagRequiresText)
                    | Some(DiagnosticCode::WithTagNamedArgument)
                    | Some(DiagnosticCode::WideReflectionUnknownAccessor)
                    | Some(DiagnosticCode::WideReflectionUnexpectedArgument)
                    | Some(DiagnosticCode::ModelRefFieldUnknown)
                    | Some(DiagnosticCode::SourceRefFieldUnknown)
            )
        })
        .collect();
    assert!(
        phase_d_diags.is_empty(),
        "smelt.models.with_tag('core') must emit zero Phase D diagnostics, got {phase_d_diags:?}"
    );
}

/// `smelt.models.bogus()` emits exactly one `WideReflectionUnknownAccessor`
/// in a model body.
#[test]
fn wide_reflection_unknown_accessor_in_model_body() {
    let root = PathBuf::from("/fake/project");
    let model_path = root.join("models").join("reflect.sql");
    let model_src = "SELECT smelt.models.bogus() AS models FROM t\n";

    let (db, ws, files) = build_db(root, &[(model_path, model_src)]);
    let model_file = files[0];

    let diags = diags_with_code(
        &db,
        ws,
        model_file,
        DiagnosticCode::WideReflectionUnknownAccessor,
    );
    assert_eq!(
        diags.len(),
        1,
        "expected exactly one WideReflectionUnknownAccessor diagnostic for smelt.models.bogus(), \
         got {diags:?}"
    );
}

/// `map(smelt.models.with_tag('x'), fn m => m.bogus)` inside a model body
/// emits exactly one `ModelRefFieldUnknown` via the HOF body dispatcher.
#[test]
fn model_ref_unknown_field_in_hof_lambda_in_model_body() {
    let root = PathBuf::from("/fake/project");
    let model_path = root.join("models").join("reflect.sql");
    let model_src = "SELECT map(smelt.models.with_tag('core'), fn m => m.bogus) AS result FROM t\n";

    let (db, ws, files) = build_db(root, &[(model_path, model_src)]);
    let model_file = files[0];

    let diags = diags_with_code(&db, ws, model_file, DiagnosticCode::ModelRefFieldUnknown);
    assert_eq!(
        diags.len(),
        1,
        "expected exactly one ModelRefFieldUnknown diagnostic for m.bogus in HOF lambda, \
         got {diags:?}"
    );
}

/// `map(smelt.sources.with_tag('x'), fn s => s.bogus)` inside a model body
/// emits exactly one `SourceRefFieldUnknown` via the HOF body dispatcher.
#[test]
fn source_ref_unknown_field_in_hof_lambda_in_model_body() {
    let root = PathBuf::from("/fake/project");
    let model_path = root.join("models").join("reflect.sql");
    let model_src =
        "SELECT map(smelt.sources.with_tag('core'), fn s => s.bogus) AS result FROM t\n";

    let (db, ws, files) = build_db(root, &[(model_path, model_src)]);
    let model_file = files[0];

    let diags = diags_with_code(&db, ws, model_file, DiagnosticCode::SourceRefFieldUnknown);
    assert_eq!(
        diags.len(),
        1,
        "expected exactly one SourceRefFieldUnknown diagnostic for s.bogus in HOF lambda, \
         got {diags:?}"
    );
}

/// `map(smelt.models.with_tag('core'), fn m => m.name)` must emit zero Phase D
/// diagnostics — `name` is a valid `ModelRef` field.
#[test]
fn model_ref_known_field_in_hof_lambda_no_diagnostic() {
    let root = PathBuf::from("/fake/project");
    let model_path = root.join("models").join("reflect.sql");
    let model_src = "SELECT map(smelt.models.with_tag('core'), fn m => m.name) AS result FROM t\n";

    let (db, ws, files) = build_db(root, &[(model_path, model_src)]);
    let model_file = files[0];

    let all_diags = file_diagnostics(&db, ws, model_file);
    let phase_d_diags: Vec<_> = all_diags
        .iter()
        .filter(|d| {
            matches!(
                d.code,
                Some(DiagnosticCode::WithTagRequiresText)
                    | Some(DiagnosticCode::WithTagNamedArgument)
                    | Some(DiagnosticCode::WideReflectionUnknownAccessor)
                    | Some(DiagnosticCode::WideReflectionUnexpectedArgument)
                    | Some(DiagnosticCode::ModelRefFieldUnknown)
                    | Some(DiagnosticCode::SourceRefFieldUnknown)
            )
        })
        .collect();
    assert!(
        phase_d_diags.is_empty(),
        "m.name is a valid ModelRef field — must emit zero Phase D diagnostics, \
         got {phase_d_diags:?}"
    );
}
