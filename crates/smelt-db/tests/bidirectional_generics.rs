//! Phase 27 integration tests — bidirectional generics (§16 #14, Decision 14).
//!
//! Verifies that the `expected_return` context propagates through
//! `check_tier3_return_type` → `try_registry_inference` so that built-in
//! generic calls widen their type-variable binding when the surrounding
//! function body declares a concrete return type.

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
/// `ABS(x)` where `x: Expr<Integer>`, the expected return type should
/// propagate into `try_registry_inference` so that ABS (a `T: Numeric`
/// generic) widens to `Double` and the Tier 3 return-type check passes
/// with zero diagnostics.
///
/// Without bidirectional inference: `ABS(Integer) → Integer`, which
/// mismatches the declared `-> Expr<Double>`.
/// With Phase 27: `expected_return = Some(Double)` is passed to
/// `unify_call_with_expected`, LUB({Integer, Double}) = Double under the
/// Numeric promotion chain — check passes.
///
/// This test is NOT a false positive: `seed_param_context` binds
/// `Expr<Integer>` to `DataType::Integer` (not Double), so the widening
/// can only happen via the bidirectional `expected_return` hint.
#[test]
fn abs_integer_widens_to_declared_double_return() {
    let root = PathBuf::from("/fake/abs_bidir");
    let fn_path = root.join("functions").join("abs_wrapper.sql");
    // Tier 3: `x: Expr<Integer>` → binds to DataType::Integer in body context.
    // ABS<T: Numeric>(T) → T via REGISTRY_MIGRATED.
    // Without bidir: ABS(Integer) = Integer ≠ Double → ReturnTypeMismatch.
    // With bidir: expected_return=Double, LUB(Integer, Double)=Double → ok.
    let fn_src = "smelt.define abs_wrapper(x: Expr<Integer>) \
                  -> Expr<Double> AS (ABS(x))\n";

    let (db, ws, files) = build_db(root, &[(fn_path.clone(), fn_src)]);
    let fn_file = files[0];

    let diags = all_type_diags(&db, ws, fn_file);
    assert!(
        diags.is_empty(),
        "expected no type diagnostics for ABS(Integer) widening to Double, got {diags:?}"
    );
}

/// A Tier 3 function that wraps ABS but declares `-> Expr<Text>` when
/// the arg is `Expr<Integer>` should produce a ReturnTypeMismatch — the
/// Integer→Text coercion is not in the Numeric promotion chain.
#[test]
fn abs_integer_wrong_return_type_emits_diagnostic() {
    let root = PathBuf::from("/fake/abs_wrong_return");
    let fn_path = root.join("functions").join("bad_abs.sql");
    // Declares `-> Expr<Text>` but body is ABS(Integer).
    // ABS(Integer) infers to a numeric type, not Text.
    // The return-type mismatch should surface.
    let fn_src = "smelt.define bad_abs(x: Expr<Integer>) \
                  -> Expr<Text> AS (ABS(x))\n";

    let (db, ws, files) = build_db(root, &[(fn_path.clone(), fn_src)]);
    let fn_file = files[0];

    let diags = all_type_diags(&db, ws, fn_file);
    let return_diags: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::ReturnTypeMismatch))
        .collect();
    assert!(
        !return_diags.is_empty(),
        "expected a ReturnTypeMismatch diagnostic for Integer→Text, got none. All diags: {diags:?}"
    );
}

/// Tier 2 body check: `smelt.define f(revenue: Expr<Decimal>) AS (MIN(revenue))`
///
/// MIN<T: Ordered>(T) → T with `revenue` bound to Decimal must return
/// Decimal — no `FunctionBodyTypeMismatch` diagnostic. This test goes
/// through the full `check_function_body` + `seed_param_context` path,
/// verifying that the registry generic correctly propagates the concrete
/// Decimal type from argument to return without any bidirectional hint.
#[test]
fn generics_within_tier2_body_integration() {
    let root = PathBuf::from("/fake/tier2_generic_bidir");
    let fn_path = root.join("functions").join("min_revenue.sql");
    // Tier 2: no return annotation. Body calls MIN(revenue) where revenue is
    // a Decimal param. MIN is in REGISTRY_MIGRATED with T: Ordered.
    // Registry unification: arg T=Decimal → return T=Decimal.
    // No mismatch, no diagnostic expected.
    let fn_src = "smelt.define min_revenue(revenue: Expr<Decimal>) AS (MIN(revenue))\n";

    let (db, ws, files) = build_db(root, &[(fn_path.clone(), fn_src)]);
    let fn_file = files[0];

    let diags = all_type_diags(&db, ws, fn_file);
    assert!(
        diags.is_empty(),
        "expected no type diagnostics for MIN(Decimal) in Tier 2 body, got {diags:?}"
    );
}
