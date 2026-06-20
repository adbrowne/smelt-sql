//! Verify that non-broken example workspaces produce zero LSP diagnostics.
//!
//! This test ensures that `file_diagnostics()` and `check_type_diagnostics()`
//! report no warnings or errors for any model in the example workspaces.
//! Regressions introduced by parser, type-inference, or example changes are
//! caught here.

use line_index::LineIndex;
use smelt_cli::{init_db, Config, ModelDiscovery};
use smelt_db::{DiagnosticAcc, Workspace};
use std::path::Path;

fn check_workspace_no_diagnostics(example_dir: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join(example_dir);

    let config: Config =
        serde_yaml::from_str(&std::fs::read_to_string(path.join("smelt.yml")).unwrap()).unwrap();

    let discovery = ModelDiscovery::new(path.clone(), config.paths.clone());
    let mut models = discovery.discover_models().unwrap();

    // Discover function files under `functions/`. Phase 3 registers them as
    // Salsa `SourceFile` inputs alongside models so the signature index sees
    // them. Workspaces without a `functions/` directory get an empty vec.
    let function_files = discovery.discover_function_files().unwrap();
    models.extend(function_files);

    // Discover Python models (executes Python to get generated SQL)
    let python_files = discovery.discover_python_files().unwrap();
    if !python_files.is_empty() {
        let python_models = smelt_cli::discover_python_models(
            &python_files,
            &models,
            &config,
            &path,
            config.python.as_deref(),
        )
        .unwrap();
        models.extend(python_models);
    }

    let db = init_db(&path, &models);
    let ws = Workspace::try_get(&db).expect("workspace not initialized");

    let mut all_issues = Vec::new();
    for model in &models {
        let file = match db.source_file(&model.path) {
            Some(f) => f,
            None => continue,
        };
        for d in smelt_db::file_diagnostics(&db, ws, file).iter() {
            all_issues.push(format!(
                "[{:?}] {}: {}",
                d.severity,
                model.path.strip_prefix(&path).unwrap().display(),
                d.message
            ));
        }
        for d in smelt_db::check_type_diagnostics::accumulated::<DiagnosticAcc>(&db, ws, file) {
            all_issues.push(format!(
                "[{:?}] {}: {}",
                d.0.severity,
                model.path.strip_prefix(&path).unwrap().display(),
                d.0.message
            ));
        }
    }

    assert!(
        all_issues.is_empty(),
        "Found {} diagnostic(s) in {}:\n  {}",
        all_issues.len(),
        example_dir,
        all_issues.join("\n  ")
    );
}

#[test]
fn timeseries_no_diagnostics() {
    check_workspace_no_diagnostics("examples/timeseries");
}

#[test]
fn retail_analytics_no_diagnostics() {
    check_workspace_no_diagnostics("examples/retail_analytics");
}

#[test]
fn test_workspace_no_diagnostics() {
    check_workspace_no_diagnostics("examples/test_workspace");
}

#[test]
fn ephemeral_demo_no_diagnostics() {
    check_workspace_no_diagnostics("examples/ephemeral_demo");
}

#[test]
fn multi_engine_no_diagnostics() {
    check_workspace_no_diagnostics("examples/multi_engine");
}

#[test]
fn ecommerce_no_diagnostics() {
    check_workspace_no_diagnostics("examples/ecommerce");
}

#[test]
fn functions_demo_no_diagnostics() {
    check_workspace_no_diagnostics("examples/functions_demo");
}

#[test]
fn web_analytics_no_diagnostics() {
    check_workspace_no_diagnostics("examples/web_analytics");
}

#[test]
fn fn_tableexpr_star_no_diagnostics() {
    check_workspace_no_diagnostics("examples/fn_tableexpr_star");
}

/// D1 phase: functions × incremental × timeseries happy-path fixture.
/// Verifies that a workspace mixing smelt.define predicates (called in WHERE)
/// and partition-aligned window-function helpers inside incremental models
/// loads without any diagnostics.
#[test]
fn fn_incremental_ts_no_diagnostics() {
    check_workspace_no_diagnostics("examples/fn_incremental_ts");
}

/// D-01/D-05: domain-grouped layout where models live under `billing/` and
/// `finance/` rather than a top-level `models/` directory. Verifies that
/// project-wide discovery (no scan-root gate) finds both models and that
/// `paths:` acts as a strip-list only, producing addresses `orders` and `revenue`.
#[test]
fn architecture_domain_layout_no_diagnostics() {
    check_workspace_no_diagnostics("examples/architecture_domain_layout");
}

/// Test 4 (TDD): All example SQL files must use the unified `smelt.<path>`
/// syntax.  This test FAILS until the migration tool has been run on all
/// example workspaces.
#[test]
fn all_examples_use_path_syntax() {
    let examples_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("examples");
    let mut legacy_usages: Vec<String> = Vec::new();
    for entry in walkdir::WalkDir::new(&examples_dir) {
        let entry = entry.unwrap();
        if entry.path().extension().and_then(|e| e.to_str()) != Some("sql") {
            continue;
        }
        let content = std::fs::read_to_string(entry.path()).unwrap();
        for (line_no, line) in content.lines().enumerate() {
            // Skip comment lines
            let trimmed = line.trim_start();
            if trimmed.starts_with("--") {
                continue;
            }
            for pattern in &["smelt.ref(", "smelt.source(", "smelt.fn."] {
                if line.contains(pattern) {
                    legacy_usages.push(format!(
                        "{}:{}: {}",
                        entry.path().display(),
                        line_no + 1,
                        line.trim()
                    ));
                }
            }
        }
    }
    assert!(
        legacy_usages.is_empty(),
        "Found legacy smelt syntax in examples (must be migrated to smelt.<path>):\n{}",
        legacy_usages.join("\n")
    );
}

/// Phase 4 TDD: After legacy `smelt.ref()` and `smelt.source()` deletion, all
/// known-good example workspaces must still produce zero diagnostics.  This is
/// the named TDD gate for Phase 4 of the smelt-path migration plan.
///
/// This test will pass only when:
///   1. All example SQL has been migrated from `smelt.ref()`/`smelt.source()` to
///      `smelt.<path>` form (covered by `all_examples_use_path_syntax`), AND
///   2. The parser correctly handles the new path form without introducing
///      spurious parse errors.
#[test]
fn all_examples_clean_after_legacy_removal() {
    for workspace in &[
        "examples/timeseries",
        "examples/retail_analytics",
        "examples/test_workspace",
        "examples/ephemeral_demo",
        "examples/multi_engine",
        "examples/ecommerce",
        "examples/functions_demo",
        "examples/web_analytics",
    ] {
        check_workspace_no_diagnostics(workspace);
    }
}

/// Test 5 (TDD): All known-good example workspaces must produce zero LSP
/// diagnostics after migration.  This re-runs every non-broken workspace in
/// one sweep so a migration regression is caught quickly.
///
/// The per-workspace `*_no_diagnostics` tests above also cover this — this
/// test is a belt-and-suspenders sweep that makes the intent explicit.
#[test]
fn all_examples_have_zero_lsp_diagnostics_after_migration() {
    // This serves as a combined check; the individual per-workspace tests
    // above cover the same workspaces individually for better error messages.
    for workspace in &[
        "examples/timeseries",
        "examples/retail_analytics",
        "examples/test_workspace",
        "examples/ephemeral_demo",
        "examples/multi_engine",
        "examples/ecommerce",
        "examples/functions_demo",
        "examples/web_analytics",
    ] {
        check_workspace_no_diagnostics(workspace);
    }
}

// ===== Phase A (meta-language) TDD tests =====
//
// Layout: two separate workspaces mirror the existing broken/ pattern.
//   - `examples/meta_lists/`        — happy-path models only; paths: [models]
//   - `examples/meta_lists_broken/` — one broken model per diagnostic code;
//     paths: [models]
//
// The clean workspace test uses `check_workspace_no_diagnostics` (same helper
// as every other non-broken workspace).  The broken-workspace tests use
// `check_workspace_emits_exactly_one_diagnostic` which asserts exactly one
// diagnostic of the target code and no other diagnostics.

/// The four Phase A diagnostic codes.  The helper uses this set to enforce
/// that the broken fixture triggers exactly one Phase A code and no others.
/// `ParseError` is NOT in this set because the WHERE-spread model legitimately
/// produces a parse error alongside the meta-language diagnostic.
const PHASE_A_CODES: &[smelt_db::DiagnosticCode] = &[
    smelt_db::DiagnosticCode::MetaListEmptyTypeUnknown,
    smelt_db::DiagnosticCode::MetaListHeterogeneous,
    smelt_db::DiagnosticCode::MetaSpreadInForbiddenPosition,
    smelt_db::DiagnosticCode::MetaSpreadOnNonList,
    smelt_db::DiagnosticCode::MetaListInScalarPosition,
];

/// Helper: loads `example_dir` as a workspace, then checks diagnostics for the
/// one file whose relative path ends with `expected_file`.  Asserts:
///   1. Exactly one Phase A diagnostic fires for that file (codes in
///      `PHASE_A_CODES`).  ParseError is allowed alongside it because the
///      WHERE-spread model necessarily produces a parser error in addition to
///      `MetaSpreadInForbiddenPosition`.
///   2. That Phase A diagnostic has code `expected_code`.
///   3. No Phase A diagnostics fire for any OTHER file in the workspace.
///
/// This keeps each broken fixture surgical: one intentionally broken model
/// triggers one specific Phase A diagnostic, and no other file in the workspace
/// triggers any Phase A code.  The broken workspace may contain multiple broken
/// models that are each tested individually.
fn check_workspace_emits_exactly_one_diagnostic(
    example_dir: &str,
    expected_file: &str,
    expected_code: smelt_db::DiagnosticCode,
) {
    use smelt_cli::{init_db, Config, ModelDiscovery};
    use smelt_db::{DiagnosticAcc, Workspace};
    use std::path::Path;

    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join(example_dir);

    let config: Config =
        serde_yaml::from_str(&std::fs::read_to_string(path.join("smelt.yml")).unwrap()).unwrap();

    let discovery = ModelDiscovery::new(path.clone(), config.paths.clone());
    let mut models = discovery.discover_models().unwrap();
    let function_files = discovery.discover_function_files().unwrap();
    models.extend(function_files);

    let db = init_db(&path, &models);
    let ws = Workspace::try_get(&db).expect("workspace not initialized");

    // Phase A diagnostics from the target file.
    let mut target_phase_a: Vec<smelt_db::Diagnostic> = Vec::new();
    // Phase A diagnostics from all OTHER files (must be empty).
    let mut other_phase_a: Vec<(String, smelt_db::Diagnostic)> = Vec::new();

    let is_phase_a = |code: Option<&smelt_db::DiagnosticCode>| -> bool {
        code.is_some_and(|c| PHASE_A_CODES.contains(c))
    };

    for model in &models {
        let file = match db.source_file(&model.path) {
            Some(f) => f,
            None => continue,
        };
        let rel = model
            .path
            .strip_prefix(&path)
            .unwrap()
            .display()
            .to_string();
        let is_target = rel
            .replace('\\', "/")
            .ends_with(&expected_file.replace('\\', "/"));

        for d in smelt_db::file_diagnostics(&db, ws, file).iter() {
            if !is_phase_a(d.code.as_ref()) {
                continue;
            }
            if is_target {
                target_phase_a.push(d.clone());
            } else {
                other_phase_a.push((rel.clone(), d.clone()));
            }
        }
        for d in smelt_db::check_type_diagnostics::accumulated::<DiagnosticAcc>(&db, ws, file) {
            if !is_phase_a(d.0.code.as_ref()) {
                continue;
            }
            if is_target {
                target_phase_a.push(d.0.clone());
            } else {
                other_phase_a.push((rel.clone(), d.0.clone()));
            }
        }
    }

    // No other file in the workspace may fire Phase A diagnostics.
    assert!(
        other_phase_a.is_empty(),
        "expected zero Phase A diagnostics from files other than '{}' in {}, got {}:\n  {}",
        expected_file,
        example_dir,
        other_phase_a.len(),
        other_phase_a
            .iter()
            .map(|(f, d)| format!("[{:?}] {}: {}", d.code, f, d.message))
            .collect::<Vec<_>>()
            .join("\n  ")
    );

    // The target file must produce exactly one Phase A diagnostic.
    assert_eq!(
        target_phase_a.len(),
        1,
        "expected exactly 1 Phase A diagnostic from '{}' in {}, got {}:\n  {}",
        expected_file,
        example_dir,
        target_phase_a.len(),
        target_phase_a
            .iter()
            .map(|d| format!("[{:?}]: {}", d.code, d.message))
            .collect::<Vec<_>>()
            .join("\n  ")
    );

    // Assert the diagnostic has the expected code.
    assert_eq!(
        target_phase_a[0].code,
        Some(expected_code),
        "expected Phase A diagnostic code {:?} from '{}' in {}, got {:?}: {}",
        expected_code,
        expected_file,
        example_dir,
        target_phase_a[0].code,
        target_phase_a[0].message
    );
}

/// Phase A TDD: `examples/meta_lists/` produces zero diagnostics.
/// This is the acceptance gate for the four happy-path models.
#[test]
fn meta_lists_clean_workspace() {
    check_workspace_no_diagnostics("examples/meta_lists");
}

/// Phase A TDD: `examples/meta_lists_broken_heterogeneous/` produces exactly
/// one `MetaListHeterogeneous` diagnostic anchored at `models/heterogeneous.sql`.
#[test]
fn meta_lists_broken_heterogeneous() {
    check_workspace_emits_exactly_one_diagnostic(
        "examples/meta_lists_broken_heterogeneous",
        "models/heterogeneous.sql",
        smelt_db::DiagnosticCode::MetaListHeterogeneous,
    );
}

/// Phase A TDD: `examples/meta_lists_broken_empty_unknown/` produces exactly
/// one `MetaListEmptyTypeUnknown` diagnostic anchored at `models/empty_unknown.sql`.
#[test]
fn meta_lists_broken_empty_unknown() {
    check_workspace_emits_exactly_one_diagnostic(
        "examples/meta_lists_broken_empty_unknown",
        "models/empty_unknown.sql",
        smelt_db::DiagnosticCode::MetaListEmptyTypeUnknown,
    );
}

/// P6 TDD: `examples/meta_lists_broken_list_in_scalar_position/` produces
/// exactly one `MetaListInScalarPosition` diagnostic anchored at
/// `models/list_in_scalar.sql` — a bare meta `List<T>` in a Data-World scalar
/// position, in a FROM-less model (the select-shape check must still run).
#[test]
fn meta_lists_broken_list_in_scalar_position() {
    check_workspace_emits_exactly_one_diagnostic(
        "examples/meta_lists_broken_list_in_scalar_position",
        "models/list_in_scalar.sql",
        smelt_db::DiagnosticCode::MetaListInScalarPosition,
    );
}

/// Phase A TDD: `examples/meta_lists_broken_spread_forbidden/` produces exactly
/// one `MetaSpreadInForbiddenPosition` diagnostic anchored at
/// `models/spread_forbidden.sql`.  A `ParseError` from the parser's error
/// recovery on the `...` token is allowed alongside the Phase A code (the helper
/// only counts Phase A codes).
#[test]
fn meta_lists_broken_spread_forbidden() {
    check_workspace_emits_exactly_one_diagnostic(
        "examples/meta_lists_broken_spread_forbidden",
        "models/spread_forbidden.sql",
        smelt_db::DiagnosticCode::MetaSpreadInForbiddenPosition,
    );
}

// ===== Phase B (meta-language) TDD tests =====
//
// Layout mirrors Phase A: one clean workspace + one broken workspace per diagnostic code.
//   - `examples/meta_hofs/`                         — happy-path models only
//   - `examples/meta_hofs_broken_*/`                — one broken model per Phase B code
//
// The clean workspace test uses `check_workspace_no_diagnostics`.
// The broken-workspace tests use `check_workspace_emits_exactly_one_phase_b_diagnostic` which
// asserts exactly one Phase B code fires in the named file.

/// The Phase B / F diagnostic codes. Used to filter diagnostics in the broken-workspace helper.
const PHASE_B_CODES: &[smelt_db::DiagnosticCode] = &[
    smelt_db::DiagnosticCode::LambdaInForbiddenPosition,
    smelt_db::DiagnosticCode::LambdaArityMismatch,
    smelt_db::DiagnosticCode::LambdaZeroParameters,
    smelt_db::DiagnosticCode::LambdaDuplicateParameter,
    smelt_db::DiagnosticCode::LambdaResultTypeMismatch,
    smelt_db::DiagnosticCode::HofExpectsLambda,
    smelt_db::DiagnosticCode::HofExpectsReducer,
    smelt_db::DiagnosticCode::HofNameShadowed,
    smelt_db::DiagnosticCode::ReducerNameShadowed,
    smelt_db::DiagnosticCode::PipeRhsNotCall,
    smelt_db::DiagnosticCode::PipeInDataPosition,
    smelt_db::DiagnosticCode::ReducerInputTypeMismatch,
    smelt_db::DiagnosticCode::ReducerEmptyNoIdentity,
    smelt_db::DiagnosticCode::ReducerArityMismatch,
    smelt_db::DiagnosticCode::ReducerArgTypeMismatch,
    smelt_db::DiagnosticCode::ReducerArgNotCompileTime,
    smelt_db::DiagnosticCode::ReducerNamedArgument,
    smelt_db::DiagnosticCode::HofNamedArgument,
    smelt_db::DiagnosticCode::TernaryConditionNotBoolean,
    smelt_db::DiagnosticCode::TernaryBranchTypeMismatch,
    smelt_db::DiagnosticCode::TernaryKeywordShadowed,
    smelt_db::DiagnosticCode::TernaryInDataPosition,
    smelt_db::DiagnosticCode::TernaryDanglingThen,
    smelt_db::DiagnosticCode::TernaryDanglingElse,
    smelt_db::DiagnosticCode::ConfigVarNotFound,
    smelt_db::DiagnosticCode::ConfigVarNameNotLiteral,
    smelt_db::DiagnosticCode::ConfigVarNullCoercion,
];

/// Helper: loads `example_dir` as a workspace, checks that exactly one Phase B diagnostic
/// fires for the file ending in `expected_file`, and that no other file emits Phase B codes.
fn check_workspace_emits_exactly_one_phase_b_diagnostic(
    example_dir: &str,
    expected_file: &str,
    expected_code: smelt_db::DiagnosticCode,
) {
    use smelt_cli::{init_db, Config, ModelDiscovery};
    use smelt_db::{DiagnosticAcc, Workspace};
    use std::path::Path;

    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join(example_dir);

    let config: Config =
        serde_yaml::from_str(&std::fs::read_to_string(path.join("smelt.yml")).unwrap()).unwrap();

    let discovery = ModelDiscovery::new(path.clone(), config.paths.clone());
    let mut models = discovery.discover_models().unwrap();
    let function_files = discovery.discover_function_files().unwrap();
    models.extend(function_files);

    let db = init_db(&path, &models);
    let ws = Workspace::try_get(&db).expect("workspace not initialized");

    let mut target_phase_b: Vec<smelt_db::Diagnostic> = Vec::new();
    let mut other_phase_b: Vec<(String, smelt_db::Diagnostic)> = Vec::new();

    let is_phase_b = |code: Option<&smelt_db::DiagnosticCode>| -> bool {
        code.is_some_and(|c| PHASE_B_CODES.contains(c))
    };

    for model in &models {
        let file = match db.source_file(&model.path) {
            Some(f) => f,
            None => continue,
        };
        let rel = model
            .path
            .strip_prefix(&path)
            .unwrap()
            .display()
            .to_string();
        let is_target = rel
            .replace('\\', "/")
            .ends_with(&expected_file.replace('\\', "/"));

        for d in smelt_db::file_diagnostics(&db, ws, file).iter() {
            if !is_phase_b(d.code.as_ref()) {
                continue;
            }
            if is_target {
                target_phase_b.push(d.clone());
            } else {
                other_phase_b.push((rel.clone(), d.clone()));
            }
        }
        for d in smelt_db::check_type_diagnostics::accumulated::<DiagnosticAcc>(&db, ws, file) {
            if !is_phase_b(d.0.code.as_ref()) {
                continue;
            }
            if is_target {
                target_phase_b.push(d.0.clone());
            } else {
                other_phase_b.push((rel.clone(), d.0.clone()));
            }
        }
    }

    assert!(
        other_phase_b.is_empty(),
        "expected zero Phase B diagnostics from files other than '{}' in {}, got {}:\n  {}",
        expected_file,
        example_dir,
        other_phase_b.len(),
        other_phase_b
            .iter()
            .map(|(f, d)| format!("[{:?}] {}: {}", d.code, f, d.message))
            .collect::<Vec<_>>()
            .join("\n  ")
    );

    assert_eq!(
        target_phase_b.len(),
        1,
        "expected exactly 1 Phase B diagnostic from '{}' in {}, got {}:\n  {}",
        expected_file,
        example_dir,
        target_phase_b.len(),
        target_phase_b
            .iter()
            .map(|d| format!("[{:?}]: {}", d.code, d.message))
            .collect::<Vec<_>>()
            .join("\n  ")
    );

    assert_eq!(
        target_phase_b[0].code,
        Some(expected_code),
        "expected Phase B diagnostic code {:?} from '{}' in {}, got {:?}: {}",
        expected_code,
        expected_file,
        example_dir,
        target_phase_b[0].code,
        target_phase_b[0].message
    );
}

/// Phase B TDD: `examples/meta_hofs/` produces zero diagnostics (excluding intentional
/// `ConfigVarNullCoercion` warnings if a fixture exercises null coercion).
#[test]
fn meta_hofs_clean_workspace() {
    check_workspace_no_diagnostics("examples/meta_hofs");
}

/// Phase B TDD: `examples/meta_hofs_broken_lambda_in_forbidden_position/` produces exactly
/// one `LambdaInForbiddenPosition` diagnostic.
#[test]
fn meta_hofs_broken_lambda_in_forbidden_position() {
    check_workspace_emits_exactly_one_phase_b_diagnostic(
        "examples/meta_hofs_broken_lambda_in_forbidden_position",
        "models/lambda_in_forbidden_position.sql",
        smelt_db::DiagnosticCode::LambdaInForbiddenPosition,
    );
}

/// Phase F TDD: `examples/meta_hofs_broken_lambda_arity_not_supported/` produces exactly
/// one `LambdaArityMismatch` diagnostic (Phase F replaces the old LambdaArityNotSupported
/// code with the arity-aware code now that multi-arg lambdas parse as proper CST nodes).
#[test]
fn meta_hofs_broken_lambda_arity_not_supported() {
    check_workspace_emits_exactly_one_phase_b_diagnostic(
        "examples/meta_hofs_broken_lambda_arity_not_supported",
        "models/lambda_arity_not_supported.sql",
        smelt_db::DiagnosticCode::LambdaArityMismatch,
    );
}

/// Phase B TDD: `examples/meta_hofs_broken_lambda_result_type_mismatch/` produces exactly
/// one `LambdaResultTypeMismatch` diagnostic.
#[test]
fn meta_hofs_broken_lambda_result_type_mismatch() {
    check_workspace_emits_exactly_one_phase_b_diagnostic(
        "examples/meta_hofs_broken_lambda_result_type_mismatch",
        "models/lambda_result_type_mismatch.sql",
        smelt_db::DiagnosticCode::LambdaResultTypeMismatch,
    );
}

/// D-19 TDD: `examples/meta_hofs_broken_hof_named_argument/` produces exactly
/// one `HofNamedArgument` diagnostic.
#[test]
fn meta_hofs_broken_hof_named_argument() {
    check_workspace_emits_exactly_one_phase_b_diagnostic(
        "examples/meta_hofs_broken_hof_named_argument",
        "models/hof_named_argument.sql",
        smelt_db::DiagnosticCode::HofNamedArgument,
    );
}

/// Phase B TDD: `examples/meta_hofs_broken_hof_expects_lambda/` produces exactly
/// one `HofExpectsLambda` diagnostic.
#[test]
fn meta_hofs_broken_hof_expects_lambda() {
    check_workspace_emits_exactly_one_phase_b_diagnostic(
        "examples/meta_hofs_broken_hof_expects_lambda",
        "models/hof_expects_lambda.sql",
        smelt_db::DiagnosticCode::HofExpectsLambda,
    );
}

/// Phase B TDD: `examples/meta_hofs_broken_hof_expects_reducer/` produces exactly
/// one `HofExpectsReducer` diagnostic.
#[test]
fn meta_hofs_broken_hof_expects_reducer() {
    check_workspace_emits_exactly_one_phase_b_diagnostic(
        "examples/meta_hofs_broken_hof_expects_reducer",
        "models/hof_expects_reducer.sql",
        smelt_db::DiagnosticCode::HofExpectsReducer,
    );
}

/// Phase B TDD: `examples/meta_hofs_broken_hof_name_shadowed/` produces exactly
/// one `HofNameShadowed` diagnostic.
#[test]
fn meta_hofs_broken_hof_name_shadowed() {
    check_workspace_emits_exactly_one_phase_b_diagnostic(
        "examples/meta_hofs_broken_hof_name_shadowed",
        "functions/shadowed_hof.sql",
        smelt_db::DiagnosticCode::HofNameShadowed,
    );
}

/// Phase B TDD: `examples/meta_hofs_broken_pipe_rhs_not_call/` produces exactly
/// one `PipeRhsNotCall` diagnostic.
#[test]
fn meta_hofs_broken_pipe_rhs_not_call() {
    check_workspace_emits_exactly_one_phase_b_diagnostic(
        "examples/meta_hofs_broken_pipe_rhs_not_call",
        "models/pipe_rhs_not_call.sql",
        smelt_db::DiagnosticCode::PipeRhsNotCall,
    );
}

/// Phase B TDD: `examples/meta_hofs_broken_reducer_input_type_mismatch/` produces exactly
/// one `ReducerInputTypeMismatch` diagnostic.
#[test]
fn meta_hofs_broken_reducer_input_type_mismatch() {
    check_workspace_emits_exactly_one_phase_b_diagnostic(
        "examples/meta_hofs_broken_reducer_input_type_mismatch",
        "models/reducer_input_type_mismatch.sql",
        smelt_db::DiagnosticCode::ReducerInputTypeMismatch,
    );
}

/// Phase B TDD: `examples/meta_hofs_broken_reducer_empty_no_identity/` produces exactly
/// one `ReducerEmptyNoIdentity` diagnostic.
#[test]
fn meta_hofs_broken_reducer_empty_no_identity() {
    check_workspace_emits_exactly_one_phase_b_diagnostic(
        "examples/meta_hofs_broken_reducer_empty_no_identity",
        "models/reducer_empty_no_identity.sql",
        smelt_db::DiagnosticCode::ReducerEmptyNoIdentity,
    );
}

/// Phase B TDD: `examples/meta_hofs_broken_config_var_not_found/` produces exactly
/// one `ConfigVarNotFound` diagnostic.
#[test]
fn meta_hofs_broken_config_var_not_found() {
    check_workspace_emits_exactly_one_phase_b_diagnostic(
        "examples/meta_hofs_broken_config_var_not_found",
        "models/config_var_not_found.sql",
        smelt_db::DiagnosticCode::ConfigVarNotFound,
    );
}

/// Phase B TDD: `examples/meta_hofs_broken_reducer_name_shadowed/` produces exactly
/// one `ReducerNameShadowed` diagnostic.
#[test]
fn meta_hofs_broken_reducer_name_shadowed() {
    check_workspace_emits_exactly_one_phase_b_diagnostic(
        "examples/meta_hofs_broken_reducer_name_shadowed",
        "functions/shadowed_reducer.sql",
        smelt_db::DiagnosticCode::ReducerNameShadowed,
    );
}

/// Phase B TDD: `examples/meta_hofs_broken_pipe_in_data_position/` produces exactly
/// one `PipeInDataPosition` diagnostic.
#[test]
fn meta_hofs_broken_pipe_in_data_position() {
    check_workspace_emits_exactly_one_phase_b_diagnostic(
        "examples/meta_hofs_broken_pipe_in_data_position",
        "models/pipe_in_data_position.sql",
        smelt_db::DiagnosticCode::PipeInDataPosition,
    );
}

/// Phase B TDD: `examples/meta_hofs_broken_config_var_name_not_literal/` produces exactly
/// one `ConfigVarNameNotLiteral` diagnostic.
#[test]
fn meta_hofs_broken_config_var_name_not_literal() {
    check_workspace_emits_exactly_one_phase_b_diagnostic(
        "examples/meta_hofs_broken_config_var_name_not_literal",
        "models/config_var_name_not_literal.sql",
        smelt_db::DiagnosticCode::ConfigVarNameNotLiteral,
    );
}

/// Phase B TDD: `examples/meta_hofs_broken_config_var_null_coercion/` produces exactly
/// one `ConfigVarNullCoercion` diagnostic (a warning).
#[test]
fn meta_hofs_broken_config_var_null_coercion() {
    check_workspace_emits_exactly_one_phase_b_diagnostic(
        "examples/meta_hofs_broken_config_var_null_coercion",
        "models/config_var_null_coercion.sql",
        smelt_db::DiagnosticCode::ConfigVarNullCoercion,
    );
}

// ===== Phase C (meta-language) TDD tests =====
//
// Layout mirrors Phase B: one clean workspace + one broken workspace per diagnostic code.
//   - `examples/meta_columns/`                            — happy-path: coalesce_numeric function + schema-driven select
//   - `examples/meta_columns_broken_columns_of_requires_table_expr/`   — ColumnsOfRequiresTableExpr
//   - `examples/meta_columns_broken_columns_of_named_argument/`        — ColumnsOfNamedArgument
//   - `examples/meta_columns_broken_columns_of_unresolvable_schema/`   — ColumnsOfUnresolvableSchema
//   - `examples/meta_columns_broken_column_ref_field_unknown/`         — ColumnRefFieldUnknown

/// The Phase C diagnostic codes used to filter diagnostics in the broken-workspace helper.
const PHASE_C_CODES: &[smelt_db::DiagnosticCode] = &[
    smelt_db::DiagnosticCode::ColumnsOfRequiresTableExpr,
    smelt_db::DiagnosticCode::ColumnsOfNamedArgument,
    smelt_db::DiagnosticCode::ColumnsOfUnresolvableSchema,
    smelt_db::DiagnosticCode::ColumnRefFieldUnknown,
];

/// Helper: loads `example_dir` as a workspace, checks that exactly one Phase C diagnostic
/// fires for the file ending in `expected_file`, and that no other file emits Phase C codes.
fn check_workspace_emits_exactly_one_phase_c_diagnostic(
    example_dir: &str,
    expected_file: &str,
    expected_code: smelt_db::DiagnosticCode,
) {
    use smelt_cli::{init_db, Config, ModelDiscovery};
    use smelt_db::{DiagnosticAcc, Workspace};
    use std::path::Path;

    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join(example_dir);

    let config: Config =
        serde_yaml::from_str(&std::fs::read_to_string(path.join("smelt.yml")).unwrap()).unwrap();

    let discovery = ModelDiscovery::new(path.clone(), config.paths.clone());
    let mut models = discovery.discover_models().unwrap();
    let function_files = discovery.discover_function_files().unwrap();
    models.extend(function_files);

    let db = init_db(&path, &models);
    let ws = Workspace::try_get(&db).expect("workspace not initialized");

    let mut target_phase_c: Vec<smelt_db::Diagnostic> = Vec::new();
    let mut other_phase_c: Vec<(String, smelt_db::Diagnostic)> = Vec::new();

    let is_phase_c = |code: Option<&smelt_db::DiagnosticCode>| -> bool {
        code.is_some_and(|c| PHASE_C_CODES.contains(c))
    };

    for model in &models {
        let file = match db.source_file(&model.path) {
            Some(f) => f,
            None => continue,
        };
        let rel = model
            .path
            .strip_prefix(&path)
            .unwrap()
            .display()
            .to_string();
        let is_target = rel
            .replace('\\', "/")
            .ends_with(&expected_file.replace('\\', "/"));

        for d in smelt_db::file_diagnostics(&db, ws, file).iter() {
            if !is_phase_c(d.code.as_ref()) {
                continue;
            }
            if is_target {
                target_phase_c.push(d.clone());
            } else {
                other_phase_c.push((rel.clone(), d.clone()));
            }
        }
        for d in smelt_db::check_type_diagnostics::accumulated::<DiagnosticAcc>(&db, ws, file) {
            if !is_phase_c(d.0.code.as_ref()) {
                continue;
            }
            if is_target {
                target_phase_c.push(d.0.clone());
            } else {
                other_phase_c.push((rel.clone(), d.0.clone()));
            }
        }
    }

    assert!(
        other_phase_c.is_empty(),
        "expected zero Phase C diagnostics from files other than '{}' in {}, got {}:\n  {}",
        expected_file,
        example_dir,
        other_phase_c.len(),
        other_phase_c
            .iter()
            .map(|(f, d)| format!("[{:?}] {}: {}", d.code, f, d.message))
            .collect::<Vec<_>>()
            .join("\n  ")
    );

    assert_eq!(
        target_phase_c.len(),
        1,
        "expected exactly 1 Phase C diagnostic from '{}' in {}, got {}:\n  {}",
        expected_file,
        example_dir,
        target_phase_c.len(),
        target_phase_c
            .iter()
            .map(|d| format!("[{:?}]: {}", d.code, d.message))
            .collect::<Vec<_>>()
            .join("\n  ")
    );

    assert_eq!(
        target_phase_c[0].code,
        Some(expected_code),
        "expected Phase C diagnostic code {:?} from '{}' in {}, got {:?}: {}",
        expected_code,
        expected_file,
        example_dir,
        target_phase_c[0].code,
        target_phase_c[0].message
    );
}

/// Phase C TDD: `examples/meta_columns/` produces zero diagnostics.
/// The clean fixture exercises the coalesce_numeric function end-to-end.
#[test]
fn meta_columns_clean_workspace() {
    check_workspace_no_diagnostics("examples/meta_columns");
}

/// Phase C TDD: `examples/meta_columns_broken_columns_of_requires_table_expr/` produces
/// exactly one `ColumnsOfRequiresTableExpr` diagnostic.
#[test]
fn meta_columns_broken_columns_of_requires_table_expr() {
    check_workspace_emits_exactly_one_phase_c_diagnostic(
        "examples/meta_columns_broken_columns_of_requires_table_expr",
        "models/columns_of_requires_table_expr.sql",
        smelt_db::DiagnosticCode::ColumnsOfRequiresTableExpr,
    );
}

/// Phase C TDD: `examples/meta_columns_broken_columns_of_named_argument/` produces
/// exactly one `ColumnsOfNamedArgument` diagnostic.
#[test]
fn meta_columns_broken_columns_of_named_argument() {
    check_workspace_emits_exactly_one_phase_c_diagnostic(
        "examples/meta_columns_broken_columns_of_named_argument",
        "models/columns_of_named_argument.sql",
        smelt_db::DiagnosticCode::ColumnsOfNamedArgument,
    );
}

/// Phase C TDD: `examples/meta_columns_broken_columns_of_unresolvable_schema/` produces
/// exactly one `ColumnsOfUnresolvableSchema` diagnostic.
#[test]
fn meta_columns_broken_columns_of_unresolvable_schema() {
    check_workspace_emits_exactly_one_phase_c_diagnostic(
        "examples/meta_columns_broken_columns_of_unresolvable_schema",
        "models/columns_of_unresolvable_schema.sql",
        smelt_db::DiagnosticCode::ColumnsOfUnresolvableSchema,
    );
}

/// Phase C TDD: `examples/meta_columns_broken_column_ref_field_unknown/` produces
/// exactly one `ColumnRefFieldUnknown` diagnostic.
///
/// The fixture uses a model-level HOF call `map(smelt.columns_of(orders), fn c => c.invalid)`
/// where `c` is ColumnRef-typed (bound by the HOF source list) and `invalid` is not
/// in the closed field set {name, type, is_numeric}.
#[test]
fn meta_columns_broken_column_ref_field_unknown() {
    check_workspace_emits_exactly_one_phase_c_diagnostic(
        "examples/meta_columns_broken_column_ref_field_unknown",
        "models/bad_field_access.sql",
        smelt_db::DiagnosticCode::ColumnRefFieldUnknown,
    );
}

// ===== Phase 5b TDD tests =====

/// Phase 5b TDD Test 3: After broken/ fixtures are migrated to `smelt.functions.*`,
/// the `all_examples_use_path_syntax` scan must include broken/ with no violations.
/// This test FAILS before migration because broken/ still contains `smelt.fn.*`.
#[test]
fn all_examples_use_path_syntax_including_broken() {
    let examples_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("examples");
    let mut legacy_usages: Vec<String> = Vec::new();
    for entry in walkdir::WalkDir::new(&examples_dir) {
        let entry = entry.unwrap();
        if entry.path().extension().and_then(|e| e.to_str()) != Some("sql") {
            continue;
        }
        // No skip for broken/ — this test covers ALL examples including broken/
        let content = std::fs::read_to_string(entry.path()).unwrap();
        for (line_no, line) in content.lines().enumerate() {
            // Skip comment lines
            let trimmed = line.trim_start();
            if trimmed.starts_with("--") {
                continue;
            }
            for pattern in &["smelt.ref(", "smelt.source(", "smelt.fn."] {
                if line.contains(pattern) {
                    legacy_usages.push(format!(
                        "{}:{}: {}",
                        entry.path().display(),
                        line_no + 1,
                        line.trim()
                    ));
                }
            }
        }
    }
    assert!(
        legacy_usages.is_empty(),
        "Found legacy smelt syntax in examples (including broken/; must be migrated to smelt.<path>):\n{}",
        legacy_usages.join("\n")
    );
}

/// Phase 5b TDD Test 4: Guard that key diagnostic codes still fire after
/// broken/ is migrated to `smelt.functions.*`. This is a regression guard —
/// it should pass both before and after migration.
#[test]
fn broken_workspace_diagnostics_still_fire() {
    use smelt_cli::{init_db, Config, ModelDiscovery};
    use smelt_db::{DiagnosticAcc, Workspace};
    use std::collections::HashSet;

    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("examples/broken");

    let config: Config =
        serde_yaml::from_str(&std::fs::read_to_string(path.join("smelt.yml")).unwrap()).unwrap();

    let discovery = ModelDiscovery::new(path.clone(), config.paths.clone());
    let mut models = discovery.discover_models().unwrap();
    let function_files = discovery.discover_function_files().unwrap();
    models.extend(function_files);

    let db = init_db(&path, &models);
    let ws = Workspace::try_get(&db).expect("workspace not initialized");

    let mut codes: HashSet<String> = HashSet::new();
    for model in &models {
        let file = match db.source_file(&model.path) {
            Some(f) => f,
            None => continue,
        };
        for d in smelt_db::file_diagnostics(&db, ws, file).iter() {
            if let Some(code) = &d.code {
                codes.insert(format!("{:?}", code));
            }
        }
        for d in smelt_db::check_type_diagnostics::accumulated::<DiagnosticAcc>(&db, ws, file) {
            if let Some(code) = &d.0.code {
                codes.insert(format!("{:?}", code));
            }
        }
    }

    // These diagnostic codes must still fire after migration to smelt.functions.*
    for required_code in &[
        "ArgTypeMismatch",
        "MissingArgument",
        "FunctionCallCycle",
        "UnknownSmeltFn",
        "UnknownIdentifier",
    ] {
        assert!(
            codes.contains(*required_code),
            "Expected diagnostic code {:?} to fire in broken workspace, but it was absent.\n\
             Codes present: {:?}",
            required_code,
            codes
        );
    }
}

// ===== Phase D (meta-language) TDD tests =====
//
// Layout mirrors Phase C: one clean workspace + one broken workspace per diagnostic code.
//   - `examples/meta_workspace/`                                        — happy-path models
//   - `examples/meta_workspace_broken_with_tag_requires_text/`         — WithTagRequiresText
//   - `examples/meta_workspace_broken_with_tag_named_argument/`        — WithTagNamedArgument
//   - `examples/meta_workspace_broken_wide_reflection_unknown_accessor/` — WideReflectionUnknownAccessor
//   - `examples/meta_workspace_broken_wide_reflection_unexpected_argument/` — WideReflectionUnexpectedArgument
//   - `examples/meta_workspace_broken_model_ref_field_unknown/`        — ModelRefFieldUnknown
//   - `examples/meta_workspace_broken_source_ref_field_unknown/`       — SourceRefFieldUnknown

/// The Phase D diagnostic codes used to filter diagnostics in the broken-workspace helper.
const PHASE_D_CODES: &[smelt_db::DiagnosticCode] = &[
    smelt_db::DiagnosticCode::WithTagRequiresText,
    smelt_db::DiagnosticCode::WithTagNamedArgument,
    smelt_db::DiagnosticCode::WideReflectionUnknownAccessor,
    smelt_db::DiagnosticCode::WideReflectionUnexpectedArgument,
    smelt_db::DiagnosticCode::ModelRefFieldUnknown,
    smelt_db::DiagnosticCode::SourceRefFieldUnknown,
];

/// Helper: loads `example_dir` as a workspace, checks that exactly one Phase D diagnostic
/// fires for the file ending in `expected_file`, and that no other file emits Phase D codes.
fn check_workspace_emits_exactly_one_phase_d_diagnostic(
    example_dir: &str,
    expected_file: &str,
    expected_code: smelt_db::DiagnosticCode,
) {
    use smelt_cli::{init_db, Config, ModelDiscovery};
    use smelt_db::{DiagnosticAcc, Workspace};
    use std::path::Path;

    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join(example_dir);

    let config: Config =
        serde_yaml::from_str(&std::fs::read_to_string(path.join("smelt.yml")).unwrap()).unwrap();

    let discovery = ModelDiscovery::new(path.clone(), config.paths.clone());
    let mut models = discovery.discover_models().unwrap();
    let function_files = discovery.discover_function_files().unwrap();
    models.extend(function_files);

    let db = init_db(&path, &models);
    let ws = Workspace::try_get(&db).expect("workspace not initialized");

    let mut target_phase_d: Vec<smelt_db::Diagnostic> = Vec::new();
    let mut other_phase_d: Vec<(String, smelt_db::Diagnostic)> = Vec::new();

    let is_phase_d = |code: Option<&smelt_db::DiagnosticCode>| -> bool {
        code.is_some_and(|c| PHASE_D_CODES.contains(c))
    };

    for model in &models {
        let file = match db.source_file(&model.path) {
            Some(f) => f,
            None => continue,
        };
        let rel = model
            .path
            .strip_prefix(&path)
            .unwrap()
            .display()
            .to_string();
        let is_target = rel
            .replace('\\', "/")
            .ends_with(&expected_file.replace('\\', "/"));

        for d in smelt_db::file_diagnostics(&db, ws, file).iter() {
            if !is_phase_d(d.code.as_ref()) {
                continue;
            }
            if is_target {
                target_phase_d.push(d.clone());
            } else {
                other_phase_d.push((rel.clone(), d.clone()));
            }
        }
        for d in smelt_db::check_type_diagnostics::accumulated::<DiagnosticAcc>(&db, ws, file) {
            if !is_phase_d(d.0.code.as_ref()) {
                continue;
            }
            if is_target {
                target_phase_d.push(d.0.clone());
            } else {
                other_phase_d.push((rel.clone(), d.0.clone()));
            }
        }
    }

    assert!(
        other_phase_d.is_empty(),
        "expected zero Phase D diagnostics from files other than '{}' in {}, got {}:\n  {}",
        expected_file,
        example_dir,
        other_phase_d.len(),
        other_phase_d
            .iter()
            .map(|(f, d)| format!("[{:?}] {}: {}", d.code, f, d.message))
            .collect::<Vec<_>>()
            .join("\n  ")
    );

    assert_eq!(
        target_phase_d.len(),
        1,
        "expected exactly 1 Phase D diagnostic from '{}' in {}, got {}:\n  {}",
        expected_file,
        example_dir,
        target_phase_d.len(),
        target_phase_d
            .iter()
            .map(|d| format!("[{:?}]: {}", d.code, d.message))
            .collect::<Vec<_>>()
            .join("\n  ")
    );

    assert_eq!(
        target_phase_d[0].code,
        Some(expected_code),
        "expected Phase D diagnostic code {:?} from '{}' in {}, got {:?}: {}",
        expected_code,
        expected_file,
        example_dir,
        target_phase_d[0].code,
        target_phase_d[0].message
    );
}

/// Phase D TDD: `examples/meta_workspace/` produces zero diagnostics.
/// Exercises smelt.models.with_tag, smelt.models.all, smelt.sources.with_tag,
/// smelt.sources.all, ModelRef field projection, and SourceRef field projection.
#[test]
fn meta_workspace_clean_workspace() {
    check_workspace_no_diagnostics("examples/meta_workspace");
}

/// Phase D TDD: `examples/meta_workspace_broken_with_tag_requires_text/` produces
/// exactly one `WithTagRequiresText` diagnostic.
#[test]
fn meta_workspace_broken_with_tag_requires_text() {
    check_workspace_emits_exactly_one_phase_d_diagnostic(
        "examples/meta_workspace_broken_with_tag_requires_text",
        "models/with_tag_requires_text.sql",
        smelt_db::DiagnosticCode::WithTagRequiresText,
    );
}

/// Phase D TDD: `examples/meta_workspace_broken_with_tag_named_argument/` produces
/// exactly one `WithTagNamedArgument` diagnostic.
#[test]
fn meta_workspace_broken_with_tag_named_argument() {
    check_workspace_emits_exactly_one_phase_d_diagnostic(
        "examples/meta_workspace_broken_with_tag_named_argument",
        "models/with_tag_named_argument.sql",
        smelt_db::DiagnosticCode::WithTagNamedArgument,
    );
}

/// Phase D TDD: `examples/meta_workspace_broken_wide_reflection_unknown_accessor/`
/// produces exactly one `WideReflectionUnknownAccessor` diagnostic.
#[test]
fn meta_workspace_broken_wide_reflection_unknown_accessor() {
    check_workspace_emits_exactly_one_phase_d_diagnostic(
        "examples/meta_workspace_broken_wide_reflection_unknown_accessor",
        "models/wide_reflection_unknown_accessor.sql",
        smelt_db::DiagnosticCode::WideReflectionUnknownAccessor,
    );
}

/// Phase D TDD: `examples/meta_workspace_broken_wide_reflection_unexpected_argument/`
/// produces exactly one `WideReflectionUnexpectedArgument` diagnostic.
#[test]
fn meta_workspace_broken_wide_reflection_unexpected_argument() {
    check_workspace_emits_exactly_one_phase_d_diagnostic(
        "examples/meta_workspace_broken_wide_reflection_unexpected_argument",
        "models/wide_reflection_unexpected_argument.sql",
        smelt_db::DiagnosticCode::WideReflectionUnexpectedArgument,
    );
}

/// Phase D TDD: `examples/meta_workspace_broken_model_ref_field_unknown/` produces
/// exactly one `ModelRefFieldUnknown` diagnostic.
#[test]
fn meta_workspace_broken_model_ref_field_unknown() {
    check_workspace_emits_exactly_one_phase_d_diagnostic(
        "examples/meta_workspace_broken_model_ref_field_unknown",
        "models/model_ref_field_unknown.sql",
        smelt_db::DiagnosticCode::ModelRefFieldUnknown,
    );
}

/// Phase D TDD: `examples/meta_workspace_broken_source_ref_field_unknown/` produces
/// exactly one `SourceRefFieldUnknown` diagnostic.
#[test]
fn meta_workspace_broken_source_ref_field_unknown() {
    check_workspace_emits_exactly_one_phase_d_diagnostic(
        "examples/meta_workspace_broken_source_ref_field_unknown",
        "models/source_ref_field_unknown.sql",
        smelt_db::DiagnosticCode::SourceRefFieldUnknown,
    );
}

// ===== Phase E1 (meta-language) TDD tests =====
//
// Layout mirrors earlier phases: one clean workspace + one broken workspace per
// implemented diagnostic code.
//   - `examples/meta_config/`                           — happy-path: record + YAML loader +
//                                                          Map<Text, Record> + per-target overlay
//   - `examples/meta_config_broken_*/`                  — one per loader diagnostic code
//
// Record codes (SmeltRecordRedefinition, RecordField*, RecordLiteral*, RecordIn*) and
// Map codes (MapKey*, MapApi*, MapGet*) are NOT yet wired into file_diagnostics —
// those 17 codes are deferred (see "Deferred during implementation" in the plan).
// The 13 loader codes ARE wired and have broken sub-fixtures.
//
// The clean workspace test uses `check_workspace_no_diagnostics_with_loaders`.
// The broken-workspace tests use `check_workspace_emits_exactly_one_phase_e1_diagnostic`
// which asserts exactly one Phase E1 loader code fires in the named file.

/// The Phase E1 diagnostic codes.  Used to filter diagnostics in the
/// broken-workspace helper.
const PHASE_E1_CODES: &[smelt_db::DiagnosticCode] = &[
    // Record codes (declared but not yet wired into file_diagnostics)
    smelt_db::DiagnosticCode::SmeltRecordRedefinition,
    smelt_db::DiagnosticCode::RecordFieldUnknown,
    smelt_db::DiagnosticCode::RecordFieldMissing,
    smelt_db::DiagnosticCode::RecordFieldDuplicate,
    smelt_db::DiagnosticCode::RecordFieldTypeMismatch,
    smelt_db::DiagnosticCode::RecordLiteralUnknownTarget,
    smelt_db::DiagnosticCode::RecordFieldNotProjectable,
    smelt_db::DiagnosticCode::RecordFieldTypeForbidden,
    smelt_db::DiagnosticCode::RecordCyclicDeclaration,
    smelt_db::DiagnosticCode::RecordInDataWorld,
    // Map codes (declared but not yet wired into file_diagnostics)
    smelt_db::DiagnosticCode::MapKeyTypeNotText,
    smelt_db::DiagnosticCode::MapApiUnknown,
    smelt_db::DiagnosticCode::MapApiArityMismatch,
    smelt_db::DiagnosticCode::MapApiNamedArgument,
    smelt_db::DiagnosticCode::MapApiUnexpectedArgument,
    smelt_db::DiagnosticCode::MapGetMissingKey,
    smelt_db::DiagnosticCode::MapApiArgTypeMismatch,
    // Loader codes (wired via loader_call_diagnostics_for_file)
    smelt_db::DiagnosticCode::ConfigLoaderPathNotLiteral,
    smelt_db::DiagnosticCode::ConfigLoaderPathEscapesWorkspace,
    smelt_db::DiagnosticCode::ConfigLoaderPathBackslash,
    smelt_db::DiagnosticCode::ConfigLoaderFileNotFound,
    smelt_db::DiagnosticCode::ConfigLoaderSchemaForbidden,
    smelt_db::DiagnosticCode::ConfigLoaderTomlNotYetSupported,
    smelt_db::DiagnosticCode::ConfigLoaderParseError,
    smelt_db::DiagnosticCode::ConfigLoaderRequiredFieldMissing,
    smelt_db::DiagnosticCode::ConfigLoaderUnknownField,
    smelt_db::DiagnosticCode::ConfigLoaderTypeMismatch,
    smelt_db::DiagnosticCode::ConfigLoaderRootShapeMismatch,
    smelt_db::DiagnosticCode::ConfigLoaderDuplicateMapKey,
    smelt_db::DiagnosticCode::ConfigLoaderNullCoercion,
];

/// Initialise a Salsa `Database` for `project_dir` / `models` AND register every
/// `.yaml` / `.yml` / `.json` file under `project_dir` as a `LoaderFileInput`.
///
/// This is needed so that `loader_call_diagnostics_for_file_with_content` (step 2)
/// can perform content-validation and emit diagnostics such as
/// `ConfigLoaderRequiredFieldMissing`, `ConfigLoaderParseError`, etc.  The plain
/// `init_db` helper only registers SQL model files; without the loader-file
/// registration the content-validation path is skipped and those diagnostics never
/// fire.
///
/// `sources.yml` / `sources.yaml` are excluded (they are project config, not
/// loader targets).
fn init_db_with_loaders(project_dir: &Path, models: &[smelt_cli::ModelFile]) -> smelt_db::Database {
    let mut db = smelt_cli::init_db(project_dir, models);

    // Walk the project directory for YAML / JSON files and register each as a
    // loader file input.
    let walker = walkdir::WalkDir::new(project_dir)
        .into_iter()
        .filter_entry(|e| {
            // Skip hidden directories and the `target/` build artefact tree.
            let name = e.file_name().to_string_lossy();
            !name.starts_with('.') && name != "target"
        });

    for entry in walker.flatten() {
        if !entry.file_type().is_file() {
            continue;
        }
        let ext = entry
            .path()
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");
        if !matches!(ext, "yaml" | "yml" | "json") {
            continue;
        }
        // Exclude sources.yml / sources.yaml (project config, not loader targets).
        let file_name = entry
            .path()
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");
        if file_name == "sources.yml" || file_name == "sources.yaml" {
            continue;
        }
        // Exclude smelt.yml (workspace config).
        if file_name == "smelt.yml" || file_name == "smelt.yaml" {
            continue;
        }

        // Compute workspace-relative path (forward slashes).
        let rel = match entry.path().strip_prefix(project_dir) {
            Ok(r) => r.to_string_lossy().replace('\\', "/"),
            Err(_) => continue,
        };

        let content = match std::fs::read_to_string(entry.path()) {
            Ok(c) => c,
            Err(_) => continue,
        };

        db.set_loader_file(
            std::sync::Arc::from(rel.as_str()),
            std::sync::Arc::from(content.as_str()),
            true,
        );
    }

    db
}

/// Variant of `check_workspace_no_diagnostics` that also registers loader files
/// so content-validation diagnostics (e.g. `ConfigLoaderRequiredFieldMissing`) can
/// fire.  Used for the clean `examples/meta_config/` fixture.
fn check_workspace_no_diagnostics_with_loaders(example_dir: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join(example_dir);

    let config: Config =
        serde_yaml::from_str(&std::fs::read_to_string(path.join("smelt.yml")).unwrap()).unwrap();

    let discovery = ModelDiscovery::new(path.clone(), config.paths.clone());
    let mut models = discovery.discover_models().unwrap();
    let function_files = discovery.discover_function_files().unwrap();
    models.extend(function_files);

    let db = init_db_with_loaders(&path, &models);
    let ws = Workspace::try_get(&db).expect("workspace not initialized");

    let mut all_issues = Vec::new();
    for model in &models {
        let file = match db.source_file(&model.path) {
            Some(f) => f,
            None => continue,
        };
        for d in smelt_db::file_diagnostics(&db, ws, file).iter() {
            all_issues.push(format!(
                "[{:?}] {}: {}",
                d.severity,
                model.path.strip_prefix(&path).unwrap().display(),
                d.message
            ));
        }
        for d in smelt_db::check_type_diagnostics::accumulated::<DiagnosticAcc>(&db, ws, file) {
            all_issues.push(format!(
                "[{:?}] {}: {}",
                d.0.severity,
                model.path.strip_prefix(&path).unwrap().display(),
                d.0.message
            ));
        }
    }

    assert!(
        all_issues.is_empty(),
        "Found {} diagnostic(s) in {}:\n  {}",
        all_issues.len(),
        example_dir,
        all_issues.join("\n  ")
    );
}

/// Helper: loads `example_dir` as a workspace (with loader files registered),
/// checks that exactly one Phase E1 diagnostic fires for the file ending in
/// `expected_file`, and that no other file emits Phase E1 codes.
fn check_workspace_emits_exactly_one_phase_e1_diagnostic(
    example_dir: &str,
    expected_file: &str,
    expected_code: smelt_db::DiagnosticCode,
) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join(example_dir);

    let config: Config =
        serde_yaml::from_str(&std::fs::read_to_string(path.join("smelt.yml")).unwrap()).unwrap();

    let discovery = ModelDiscovery::new(path.clone(), config.paths.clone());
    let mut models = discovery.discover_models().unwrap();
    let function_files = discovery.discover_function_files().unwrap();
    models.extend(function_files);

    let db = init_db_with_loaders(&path, &models);
    let ws = Workspace::try_get(&db).expect("workspace not initialized");

    let mut target_phase_e1: Vec<smelt_db::Diagnostic> = Vec::new();
    let mut other_phase_e1: Vec<(String, smelt_db::Diagnostic)> = Vec::new();

    let is_phase_e1 = |code: Option<&smelt_db::DiagnosticCode>| -> bool {
        code.is_some_and(|c| PHASE_E1_CODES.contains(c))
    };

    for model in &models {
        let file = match db.source_file(&model.path) {
            Some(f) => f,
            None => continue,
        };
        let rel = model
            .path
            .strip_prefix(&path)
            .unwrap()
            .display()
            .to_string();
        let is_target = rel
            .replace('\\', "/")
            .ends_with(&expected_file.replace('\\', "/"));

        for d in smelt_db::file_diagnostics(&db, ws, file).iter() {
            if !is_phase_e1(d.code.as_ref()) {
                continue;
            }
            if is_target {
                target_phase_e1.push(d.clone());
            } else {
                other_phase_e1.push((rel.clone(), d.clone()));
            }
        }
        for d in smelt_db::check_type_diagnostics::accumulated::<DiagnosticAcc>(&db, ws, file) {
            if !is_phase_e1(d.0.code.as_ref()) {
                continue;
            }
            if is_target {
                target_phase_e1.push(d.0.clone());
            } else {
                other_phase_e1.push((rel.clone(), d.0.clone()));
            }
        }
    }

    assert!(
        other_phase_e1.is_empty(),
        "expected zero Phase E1 diagnostics from files other than '{}' in {}, got {}:\n  {}",
        expected_file,
        example_dir,
        other_phase_e1.len(),
        other_phase_e1
            .iter()
            .map(|(f, d)| format!("[{:?}] {}: {}", d.code, f, d.message))
            .collect::<Vec<_>>()
            .join("\n  ")
    );

    assert_eq!(
        target_phase_e1.len(),
        1,
        "expected exactly 1 Phase E1 diagnostic from '{}' in {}, got {}:\n  {}",
        expected_file,
        example_dir,
        target_phase_e1.len(),
        target_phase_e1
            .iter()
            .map(|d| format!("[{:?}]: {}", d.code, d.message))
            .collect::<Vec<_>>()
            .join("\n  ")
    );

    assert_eq!(
        target_phase_e1[0].code,
        Some(expected_code),
        "expected Phase E1 diagnostic code {:?} from '{}' in {}, got {:?}: {}",
        expected_code,
        expected_file,
        example_dir,
        target_phase_e1[0].code,
        target_phase_e1[0].message
    );
}

/// Phase E1 TDD: `examples/meta_config/` produces zero diagnostics.
/// Exercises smelt.record declaration + inline record types + YAML loader
/// (`Map<Text, Record>` and `List<Record>` root shapes) + per-target overlay.
/// HOF consumption of loader results (`m.entries() |> map(fn e => …)`) is
/// covered by unit tests in `smelt-db::type_inference`; production wiring of
/// the loader-result type through SELECT expressions is a known follow-up
/// (see `docs/plans/20260509-meta-language-E1.md` Deferred — Phase 5).
#[test]
fn meta_config_clean_workspace() {
    check_workspace_no_diagnostics_with_loaders("examples/meta_config");
}

/// Phase E1 TDD: `examples/meta_config_broken_config_loader_path_not_literal/`
/// produces exactly one `ConfigLoaderPathNotLiteral` diagnostic.
#[test]
fn meta_config_broken_config_loader_path_not_literal() {
    check_workspace_emits_exactly_one_phase_e1_diagnostic(
        "examples/meta_config_broken_config_loader_path_not_literal",
        "models/config_loader_path_not_literal.sql",
        smelt_db::DiagnosticCode::ConfigLoaderPathNotLiteral,
    );
}

/// Phase E1 TDD: `examples/meta_config_broken_config_loader_path_escapes_workspace/`
/// produces exactly one `ConfigLoaderPathEscapesWorkspace` diagnostic.
#[test]
fn meta_config_broken_config_loader_path_escapes_workspace() {
    check_workspace_emits_exactly_one_phase_e1_diagnostic(
        "examples/meta_config_broken_config_loader_path_escapes_workspace",
        "models/config_loader_path_escapes_workspace.sql",
        smelt_db::DiagnosticCode::ConfigLoaderPathEscapesWorkspace,
    );
}

/// Phase E1 TDD: `examples/meta_config_broken_config_loader_path_backslash/`
/// produces exactly one `ConfigLoaderPathBackslash` diagnostic.
#[test]
fn meta_config_broken_config_loader_path_backslash() {
    check_workspace_emits_exactly_one_phase_e1_diagnostic(
        "examples/meta_config_broken_config_loader_path_backslash",
        "models/config_loader_path_backslash.sql",
        smelt_db::DiagnosticCode::ConfigLoaderPathBackslash,
    );
}

/// Phase E1 TDD: `examples/meta_config_broken_config_loader_file_not_found/`
/// produces exactly one `ConfigLoaderFileNotFound` diagnostic.
#[test]
fn meta_config_broken_config_loader_file_not_found() {
    check_workspace_emits_exactly_one_phase_e1_diagnostic(
        "examples/meta_config_broken_config_loader_file_not_found",
        "models/config_loader_file_not_found.sql",
        smelt_db::DiagnosticCode::ConfigLoaderFileNotFound,
    );
}

/// Phase E1 TDD: `examples/meta_config_broken_config_loader_schema_forbidden/`
/// produces exactly one `ConfigLoaderSchemaForbidden` diagnostic.
#[test]
fn meta_config_broken_config_loader_schema_forbidden() {
    check_workspace_emits_exactly_one_phase_e1_diagnostic(
        "examples/meta_config_broken_config_loader_schema_forbidden",
        "models/config_loader_schema_forbidden.sql",
        smelt_db::DiagnosticCode::ConfigLoaderSchemaForbidden,
    );
}

/// Phase E1 TDD: `examples/meta_config_broken_config_loader_toml_not_yet_supported/`
/// produces exactly one `ConfigLoaderTomlNotYetSupported` diagnostic.
#[test]
fn meta_config_broken_config_loader_toml_not_yet_supported() {
    check_workspace_emits_exactly_one_phase_e1_diagnostic(
        "examples/meta_config_broken_config_loader_toml_not_yet_supported",
        "models/config_loader_toml_not_yet_supported.sql",
        smelt_db::DiagnosticCode::ConfigLoaderTomlNotYetSupported,
    );
}

/// Phase E1 TDD: `examples/meta_config_broken_config_loader_parse_error/`
/// produces exactly one `ConfigLoaderParseError` diagnostic.
#[test]
fn meta_config_broken_config_loader_parse_error() {
    check_workspace_emits_exactly_one_phase_e1_diagnostic(
        "examples/meta_config_broken_config_loader_parse_error",
        "models/config_loader_parse_error.sql",
        smelt_db::DiagnosticCode::ConfigLoaderParseError,
    );
}

/// Phase E1 TDD: `examples/meta_config_broken_config_loader_required_field_missing/`
/// produces exactly one `ConfigLoaderRequiredFieldMissing` diagnostic.
#[test]
fn meta_config_broken_config_loader_required_field_missing() {
    check_workspace_emits_exactly_one_phase_e1_diagnostic(
        "examples/meta_config_broken_config_loader_required_field_missing",
        "models/config_loader_required_field_missing.sql",
        smelt_db::DiagnosticCode::ConfigLoaderRequiredFieldMissing,
    );
}

/// Phase E1 TDD: `examples/meta_config_broken_config_loader_unknown_field/`
/// produces exactly one `ConfigLoaderUnknownField` diagnostic.
#[test]
fn meta_config_broken_config_loader_unknown_field() {
    check_workspace_emits_exactly_one_phase_e1_diagnostic(
        "examples/meta_config_broken_config_loader_unknown_field",
        "models/config_loader_unknown_field.sql",
        smelt_db::DiagnosticCode::ConfigLoaderUnknownField,
    );
}

/// Phase E1 TDD: `examples/meta_config_broken_config_loader_type_mismatch/`
/// produces exactly one `ConfigLoaderTypeMismatch` diagnostic.
#[test]
fn meta_config_broken_config_loader_type_mismatch() {
    check_workspace_emits_exactly_one_phase_e1_diagnostic(
        "examples/meta_config_broken_config_loader_type_mismatch",
        "models/config_loader_type_mismatch.sql",
        smelt_db::DiagnosticCode::ConfigLoaderTypeMismatch,
    );
}

/// Phase E1 TDD: `examples/meta_config_broken_config_loader_root_shape_mismatch/`
/// produces exactly one `ConfigLoaderRootShapeMismatch` diagnostic.
#[test]
fn meta_config_broken_config_loader_root_shape_mismatch() {
    check_workspace_emits_exactly_one_phase_e1_diagnostic(
        "examples/meta_config_broken_config_loader_root_shape_mismatch",
        "models/config_loader_root_shape_mismatch.sql",
        smelt_db::DiagnosticCode::ConfigLoaderRootShapeMismatch,
    );
}

/// Phase E1 TDD: `ConfigLoaderDuplicateMapKey` — deferred from E2E fixture coverage.
///
/// `marked_yaml` (and `serde_json`) silently deduplicate keys when
/// `error_on_duplicate_keys` is `false` (the default). The validator's
/// `seen_keys` map never observes a second entry for the same key, so no
/// diagnostic fires via a real YAML/JSON file on disk. This code is covered
/// by the synthetic unit test `loader::tests::yaml_parse_map_root_emits_duplicate_key`
/// which injects a `ParsedNode::Mapping` with repeated keys directly.
///
/// To promote this to a live E2E fixture, the `parse_yaml` loader would need
/// to enable `error_on_duplicate_keys` and map the `DuplicateKey` load error
/// to `ConfigLoaderDuplicateMapKey` instead of `ConfigLoaderParseError`.
/// Deferred to a future loader spec edit.
#[test]
#[ignore = "ConfigLoaderDuplicateMapKey cannot be triggered via real YAML/JSON files \
             because YAML/JSON parsers silently deduplicate keys; covered by \
             loader::tests::yaml_parse_map_root_emits_duplicate_key unit test"]
fn meta_config_broken_config_loader_duplicate_map_key() {
    check_workspace_emits_exactly_one_phase_e1_diagnostic(
        "examples/meta_config_broken_config_loader_duplicate_map_key",
        "models/config_loader_duplicate_map_key.sql",
        smelt_db::DiagnosticCode::ConfigLoaderDuplicateMapKey,
    );
}

/// Phase E1 TDD: `examples/meta_config_broken_config_loader_null_coercion/`
/// produces exactly one `ConfigLoaderNullCoercion` diagnostic (warning severity).
#[test]
fn meta_config_broken_config_loader_null_coercion() {
    check_workspace_emits_exactly_one_phase_e1_diagnostic(
        "examples/meta_config_broken_config_loader_null_coercion",
        "models/config_loader_null_coercion.sql",
        smelt_db::DiagnosticCode::ConfigLoaderNullCoercion,
    );
}

/// BUG-014 P4 TDD: A schema-violating overlay file surfaces
/// `ConfigLoaderUnknownField` anchored at the generator call site.
///
/// `examples/meta_config_overlay_probe_invalid/` has `target: prod` in
/// `smelt.yml` so the overlay `cohorts.prod.yaml` is always active.  That
/// overlay contains `extra_field` (not in the schema) → exactly one
/// `ConfigLoaderUnknownField` must fire for `models/cohorts.gen.sql`.
#[test]
fn overlay_probe_invalid_overlay_emits_unknown_field() {
    check_workspace_emits_exactly_one_phase_e1_diagnostic(
        "examples/meta_config_overlay_probe_invalid",
        "models/cohorts.gen.sql",
        smelt_db::DiagnosticCode::ConfigLoaderUnknownField,
    );
}

// ===== Phase E2 (multi-model production) TDD tests =====
//
// Layout:
//   - `examples/per_cohort_union/`                          — happy-path demo
//   - `examples/staging_from_sources/`                      — sources-driven generator
//   - `examples/per_cohort_union_broken_<code>/`            — one per diagnostic code
//
// Each broken fixture uses the generic E2 helper below, which filters to the
// ten Phase E2 diagnostic codes.

/// The ten Phase E2 diagnostic codes.
const PHASE_E2_CODES: &[smelt_db::DiagnosticCode] = &[
    smelt_db::DiagnosticCode::GeneratesUnknownValue,
    smelt_db::DiagnosticCode::GeneratesMixedWithBareModel,
    smelt_db::DiagnosticCode::GenerateFileBareSelectForbidden,
    smelt_db::DiagnosticCode::GenerateFileBodyTypeError,
    smelt_db::DiagnosticCode::ModelDefOutsideGeneratorFile,
    smelt_db::DiagnosticCode::ModelDefInvalidName,
    smelt_db::DiagnosticCode::ModelDefInvalidMaterialization,
    smelt_db::DiagnosticCode::ModelDefDuplicateName,
    smelt_db::DiagnosticCode::ModelDefHandAuthoredCollision,
    smelt_db::DiagnosticCode::GeneratorBodyForbidsModelReflection,
];

/// Helper: loads `example_dir`, asserts exactly one Phase E2 diagnostic fires
/// for the file ending in `expected_file`, and that no other file emits E2 codes.
fn check_workspace_emits_exactly_one_phase_e2_diagnostic(
    example_dir: &str,
    expected_file: &str,
    expected_code: smelt_db::DiagnosticCode,
) {
    use smelt_cli::{init_db, Config, ModelDiscovery};
    use smelt_db::{DiagnosticAcc, Workspace};
    use std::path::Path;

    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join(example_dir);

    let config: Config =
        serde_yaml::from_str(&std::fs::read_to_string(path.join("smelt.yml")).unwrap()).unwrap();

    let discovery = ModelDiscovery::new(path.clone(), config.paths.clone());
    let mut models = discovery.discover_models().unwrap();
    let function_files = discovery.discover_function_files().unwrap();
    models.extend(function_files);

    let db = init_db(&path, &models);
    let ws = Workspace::try_get(&db).expect("workspace not initialized");

    let mut target_e2: Vec<smelt_db::Diagnostic> = Vec::new();
    let mut other_e2: Vec<(String, smelt_db::Diagnostic)> = Vec::new();

    let is_e2 = |code: Option<&smelt_db::DiagnosticCode>| -> bool {
        code.is_some_and(|c| PHASE_E2_CODES.contains(c))
    };

    for model in &models {
        let file = match db.source_file(&model.path) {
            Some(f) => f,
            None => continue,
        };
        let rel = model
            .path
            .strip_prefix(&path)
            .unwrap()
            .display()
            .to_string();
        let is_target = rel
            .replace('\\', "/")
            .ends_with(&expected_file.replace('\\', "/"));

        for d in smelt_db::file_diagnostics(&db, ws, file).iter() {
            if !is_e2(d.code.as_ref()) {
                continue;
            }
            if is_target {
                target_e2.push(d.clone());
            } else {
                other_e2.push((rel.clone(), d.clone()));
            }
        }
        for d in smelt_db::check_type_diagnostics::accumulated::<DiagnosticAcc>(&db, ws, file) {
            if !is_e2(d.0.code.as_ref()) {
                continue;
            }
            if is_target {
                target_e2.push(d.0.clone());
            } else {
                other_e2.push((rel.clone(), d.0.clone()));
            }
        }
    }

    assert!(
        other_e2.is_empty(),
        "expected zero Phase E2 diagnostics from files other than '{}' in {}, got {}:\n  {}",
        expected_file,
        example_dir,
        other_e2.len(),
        other_e2
            .iter()
            .map(|(f, d)| format!("[{:?}] {}: {}", d.code, f, d.message))
            .collect::<Vec<_>>()
            .join("\n  ")
    );

    assert_eq!(
        target_e2.len(),
        1,
        "expected exactly 1 Phase E2 diagnostic from '{}' in {}, got {}:\n  {}",
        expected_file,
        example_dir,
        target_e2.len(),
        target_e2
            .iter()
            .map(|d| format!("[{:?}]: {}", d.code, d.message))
            .collect::<Vec<_>>()
            .join("\n  ")
    );

    assert_eq!(
        target_e2[0].code,
        Some(expected_code),
        "expected Phase E2 code {:?} from '{}' in {}, got {:?}: {}",
        expected_code,
        expected_file,
        example_dir,
        target_e2[0].code,
        target_e2[0].message
    );
}

/// Phase E2 TDD: `examples/per_cohort_union/` produces zero diagnostics.
#[test]
fn per_cohort_union_example_has_zero_diagnostics() {
    check_workspace_no_diagnostics("examples/per_cohort_union");
}

/// Phase 2 (VALUES-derived-table-typing) TDD item 3: `examples/per_cohort_union/models/orders.sql`
/// exports five concrete-typed columns via a VALUES-derived table.
///
/// `orders.sql` selects from a `(VALUES (…), …) AS t(id, user_id, region, revenue, created_at)`
/// subquery. Phase 2 integrates VALUES-column typing into `typed_model_schema`, so each of the
/// five columns must resolve to a concrete, non-Unknown type.
///
/// Note: `all_cohorts_unioned.sql` and the generator-emitted cohort models are NOT asserted here
/// — their schemas are still Unknown because they depend on generator-emission schema inference,
/// which is out of scope for this phase (tracked in `docs/specs/meta_language.md`).
#[test]
fn per_cohort_union_orders_has_concrete_typed_schema() {
    use smelt_cli::{init_db, Config, ModelDiscovery};
    use smelt_db::Workspace;
    use std::path::Path;

    let example_dir = "examples/per_cohort_union";
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join(example_dir);

    let config: Config =
        serde_yaml::from_str(&std::fs::read_to_string(path.join("smelt.yml")).unwrap()).unwrap();

    let discovery = ModelDiscovery::new(path.clone(), config.paths.clone());
    let mut models = discovery.discover_models().unwrap();
    let function_files = discovery.discover_function_files().unwrap();
    models.extend(function_files);

    let db = init_db(&path, &models);
    let ws = Workspace::try_get(&db).expect("workspace not initialized");

    // Find orders.sql
    let orders_path = path.join("models").join("orders.sql");
    let orders_file = db
        .source_file(&orders_path)
        .expect("orders.sql not found in workspace");

    let schema = smelt_db::typed_model_schema(&db, ws, orders_file);

    assert_eq!(
        schema.columns.len(),
        5,
        "orders.sql should export exactly 5 columns, got {}:\n  {:?}",
        schema.columns.len(),
        schema.columns.iter().map(|c| &c.name).collect::<Vec<_>>()
    );

    for col in &schema.columns {
        let typed = col.data_type.as_ref().unwrap_or_else(|| {
            panic!(
                "orders.sql column '{}' has no TypedColumn (data_type is None)",
                col.name
            )
        });
        assert_ne!(
            typed.data_type,
            smelt_db::DataType::unknown_dynamic(),
            "orders.sql column '{}' should have a concrete type, got Unknown",
            col.name
        );
    }
}

/// Phase E2 TDD: `examples/staging_from_sources/` produces zero diagnostics.
#[test]
fn staging_from_sources_example_has_zero_diagnostics() {
    check_workspace_no_diagnostics("examples/staging_from_sources");
}

/// D5 probe (clean path): `seed_source_type_join` joins a seed (inferred types
/// via CSV + sidecar using aliases INT/BOOL/DECIMAL) with a source (declared
/// aliases INT8/TIMESTAMPTZ/TEXT). All type aliases must resolve to recognised
/// smelt DataTypes; no LSP diagnostics must fire.
#[test]
fn seed_source_type_join_has_zero_diagnostics() {
    check_workspace_no_diagnostics("examples/seed_source_type_join");
}

/// Non-ASCII fixture: `examples/non_ascii_columns/` has Greek-letter column
/// aliases and must produce zero diagnostics.
#[test]
fn non_ascii_columns_example_has_zero_diagnostics() {
    check_workspace_no_diagnostics("examples/non_ascii_columns");
}

/// Phase E2 TDD: `GeneratesUnknownValue` — `generates: views` fires.
#[test]
fn per_cohort_union_broken_generates_unknown_value() {
    check_workspace_emits_exactly_one_phase_e2_diagnostic(
        "examples/per_cohort_union_broken_generates_unknown_value",
        "models/broken.gen.sql",
        smelt_db::DiagnosticCode::GeneratesUnknownValue,
    );
}

/// Phase E2 TDD: `GeneratesMixedWithBareModel` — `generates: models` combined with `name:`.
#[test]
fn per_cohort_union_broken_generates_mixed_with_name_field() {
    check_workspace_emits_exactly_one_phase_e2_diagnostic(
        "examples/per_cohort_union_broken_generates_mixed_with_name_field",
        "models/broken.gen.sql",
        smelt_db::DiagnosticCode::GeneratesMixedWithBareModel,
    );
}

/// Phase E2 TDD: `GeneratesMixedWithBareModel` — `generates: models` combined with section delimiter.
#[test]
fn per_cohort_union_broken_generates_mixed_with_section_delimiter() {
    check_workspace_emits_exactly_one_phase_e2_diagnostic(
        "examples/per_cohort_union_broken_generates_mixed_with_section_delimiter",
        "models/broken.gen.sql",
        smelt_db::DiagnosticCode::GeneratesMixedWithBareModel,
    );
}

/// Phase E2 TDD: `GenerateFileBareSelectForbidden` — bare SELECT in generator body.
#[test]
fn per_cohort_union_broken_generate_file_bare_select_forbidden() {
    check_workspace_emits_exactly_one_phase_e2_diagnostic(
        "examples/per_cohort_union_broken_generate_file_bare_select_forbidden",
        "models/broken.gen.sql",
        smelt_db::DiagnosticCode::GenerateFileBareSelectForbidden,
    );
}

/// Phase E2 TDD: `GenerateFileBodyTypeError` — body type is not `List<ModelDef>`.
#[test]
fn per_cohort_union_broken_generate_file_body_type_error() {
    check_workspace_emits_exactly_one_phase_e2_diagnostic(
        "examples/per_cohort_union_broken_generate_file_body_type_error",
        "models/broken.gen.sql",
        smelt_db::DiagnosticCode::GenerateFileBodyTypeError,
    );
}

/// Phase E2 TDD: `ModelDefOutsideGeneratorFile` — `ModelDef` in a non-generator file.
#[test]
fn per_cohort_union_broken_model_def_outside_generator_file() {
    check_workspace_emits_exactly_one_phase_e2_diagnostic(
        "examples/per_cohort_union_broken_model_def_outside_generator_file",
        "models/broken.sql",
        smelt_db::DiagnosticCode::ModelDefOutsideGeneratorFile,
    );
}

/// Phase E2 TDD: `ModelDefInvalidName` — name with non-path-safe characters.
#[test]
fn per_cohort_union_broken_model_def_invalid_name() {
    check_workspace_emits_exactly_one_phase_e2_diagnostic(
        "examples/per_cohort_union_broken_model_def_invalid_name",
        "models/broken.gen.sql",
        smelt_db::DiagnosticCode::ModelDefInvalidName,
    );
}

/// Phase E2 TDD: `ModelDefInvalidMaterialization` — materialization not in closed set.
#[test]
fn per_cohort_union_broken_model_def_invalid_materialization() {
    check_workspace_emits_exactly_one_phase_e2_diagnostic(
        "examples/per_cohort_union_broken_model_def_invalid_materialization",
        "models/broken.gen.sql",
        smelt_db::DiagnosticCode::ModelDefInvalidMaterialization,
    );
}

/// Phase E2 TDD: `ModelDefDuplicateName` — two ModelDefs with the same name.
#[test]
fn per_cohort_union_broken_model_def_duplicate_name() {
    check_workspace_emits_exactly_one_phase_e2_diagnostic(
        "examples/per_cohort_union_broken_model_def_duplicate_name",
        "models/broken.gen.sql",
        smelt_db::DiagnosticCode::ModelDefDuplicateName,
    );
}

/// Phase E2 TDD: `ModelDefHandAuthoredCollision` — generator emission collides
/// with a hand-authored model `models/collision/my_model.sql`.
#[test]
fn per_cohort_union_broken_model_def_hand_authored_collision() {
    check_workspace_emits_exactly_one_phase_e2_diagnostic(
        "examples/per_cohort_union_broken_model_def_hand_authored_collision",
        "models/collision.gen.sql",
        smelt_db::DiagnosticCode::ModelDefHandAuthoredCollision,
    );
}

/// Phase E2 TDD: `GeneratorBodyForbidsModelReflection` — `smelt.models.with_tag`
/// inside a generator body.
#[test]
fn per_cohort_union_broken_generator_body_forbids_model_reflection() {
    check_workspace_emits_exactly_one_phase_e2_diagnostic(
        "examples/per_cohort_union_broken_generator_body_forbids_model_reflection",
        "models/broken.gen.sql",
        smelt_db::DiagnosticCode::GeneratorBodyForbidsModelReflection,
    );
}

// ===== Phase F (meta-language polish) TDD tests =====
//
// Layout mirrors prior phases: one clean workspace + sibling broken workspaces.
//   - `examples/meta_polish/`                                   — happy-path: concat_with, ternary
//   - `examples/meta_polish_broken_ternary_non_boolean_cond/`  — TernaryConditionNotBoolean
//   - `examples/meta_polish_broken_ternary_branch_mismatch/`   — TernaryBranchTypeMismatch
//   - `examples/meta_polish_broken_reducer_arity/`             — ReducerArityMismatch
//
// Broken-workspace tests reuse `check_workspace_emits_exactly_one_phase_b_diagnostic`
// (Phase B CODES already include all Phase F codes: Ternary*, Reducer*, Lambda*).

/// Phase F TDD: `examples/meta_polish/` produces zero diagnostics.
/// Exercises the `concat_with(sep)` parameterised reducer and the
/// `if cond then a else b` ternary with a Boolean-literal condition.
#[test]
fn meta_polish_clean_workspace() {
    check_workspace_no_diagnostics("examples/meta_polish");
}

/// Phase F TDD: `examples/meta_polish_broken_ternary_non_boolean_cond/` produces
/// exactly one `TernaryConditionNotBoolean` diagnostic.
#[test]
fn meta_polish_broken_ternary_non_boolean_cond() {
    check_workspace_emits_exactly_one_phase_b_diagnostic(
        "examples/meta_polish_broken_ternary_non_boolean_cond",
        "models/ternary_non_boolean_cond.sql",
        smelt_db::DiagnosticCode::TernaryConditionNotBoolean,
    );
}

/// Phase F TDD: `examples/meta_polish_broken_ternary_branch_mismatch/` produces
/// exactly one `TernaryBranchTypeMismatch` diagnostic.
#[test]
fn meta_polish_broken_ternary_branch_mismatch() {
    check_workspace_emits_exactly_one_phase_b_diagnostic(
        "examples/meta_polish_broken_ternary_branch_mismatch",
        "models/ternary_branch_mismatch.sql",
        smelt_db::DiagnosticCode::TernaryBranchTypeMismatch,
    );
}

/// Phase F TDD: `examples/meta_polish_broken_reducer_arity/` produces
/// exactly one `ReducerArityMismatch` diagnostic.
#[test]
fn meta_polish_broken_reducer_arity() {
    check_workspace_emits_exactly_one_phase_b_diagnostic(
        "examples/meta_polish_broken_reducer_arity",
        "models/reducer_arity.sql",
        smelt_db::DiagnosticCode::ReducerArityMismatch,
    );
}

// ===== Timeseries frontmatter validation TDD tests =====
//
// Fixture: `examples/timeseries_broken_incremental_without_timeseries/`
//   — one broken model that declares `incremental:` without `timeseries:`

/// Helper: loads `example_dir` as a workspace and asserts that exactly one
/// `TimeseriesRequiredForIncremental` or `MalformedTimeseries` diagnostic fires
/// for the file ending in `expected_file`, and no such diagnostic fires in any
/// other file in the workspace.
fn check_workspace_emits_timeseries_diagnostic(
    example_dir: &str,
    expected_file: &str,
    expected_code: smelt_db::DiagnosticCode,
) {
    use smelt_cli::{init_db, Config, ModelDiscovery};
    use smelt_db::{DiagnosticAcc, Workspace};
    use std::path::Path;

    const TIMESERIES_CODES: &[smelt_db::DiagnosticCode] = &[
        smelt_db::DiagnosticCode::TimeseriesRequiredForIncremental,
        smelt_db::DiagnosticCode::MalformedTimeseries,
    ];

    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join(example_dir);

    let config: Config =
        serde_yaml::from_str(&std::fs::read_to_string(path.join("smelt.yml")).unwrap()).unwrap();

    let discovery = ModelDiscovery::new(path.clone(), config.paths.clone());
    let mut models = discovery.discover_models().unwrap();
    let function_files = discovery.discover_function_files().unwrap();
    models.extend(function_files);

    let db = init_db(&path, &models);
    let ws = Workspace::try_get(&db).expect("workspace not initialized");

    let mut target_ts: Vec<smelt_db::Diagnostic> = Vec::new();
    let mut other_ts: Vec<(String, smelt_db::Diagnostic)> = Vec::new();

    let is_ts = |code: Option<&smelt_db::DiagnosticCode>| -> bool {
        code.is_some_and(|c| TIMESERIES_CODES.contains(c))
    };

    for model in &models {
        let file = match db.source_file(&model.path) {
            Some(f) => f,
            None => continue,
        };
        let rel = model
            .path
            .strip_prefix(&path)
            .unwrap()
            .display()
            .to_string();
        let is_target = rel
            .replace('\\', "/")
            .ends_with(&expected_file.replace('\\', "/"));

        for d in smelt_db::file_diagnostics(&db, ws, file).iter() {
            if !is_ts(d.code.as_ref()) {
                continue;
            }
            if is_target {
                target_ts.push(d.clone());
            } else {
                other_ts.push((rel.clone(), d.clone()));
            }
        }
        for d in smelt_db::check_type_diagnostics::accumulated::<DiagnosticAcc>(&db, ws, file) {
            if !is_ts(d.0.code.as_ref()) {
                continue;
            }
            if is_target {
                target_ts.push(d.0.clone());
            } else {
                other_ts.push((rel.clone(), d.0.clone()));
            }
        }
    }

    assert!(
        other_ts.is_empty(),
        "expected zero timeseries diagnostics from files other than '{}' in {}, got {}:\n  {}",
        expected_file,
        example_dir,
        other_ts.len(),
        other_ts
            .iter()
            .map(|(f, d)| format!("[{:?}] {}: {}", d.code, f, d.message))
            .collect::<Vec<_>>()
            .join("\n  ")
    );

    assert_eq!(
        target_ts.len(),
        1,
        "expected exactly 1 timeseries diagnostic from '{}' in {}, got {}:\n  {}",
        expected_file,
        example_dir,
        target_ts.len(),
        target_ts
            .iter()
            .map(|d| format!("[{:?}]: {}", d.code, d.message))
            .collect::<Vec<_>>()
            .join("\n  ")
    );

    assert_eq!(
        target_ts[0].code,
        Some(expected_code),
        "expected timeseries diagnostic code {:?} from '{}' in {}, got {:?}: {}",
        expected_code,
        expected_file,
        example_dir,
        target_ts[0].code,
        target_ts[0].message
    );
}

/// Timeseries TDD: `examples/timeseries_broken_incremental_without_timeseries/` produces
/// exactly one `TimeseriesRequiredForIncremental` diagnostic anchored at
/// `models/incremental_without_timeseries.sql`.
///
/// This test verifies that `validate_timeseries` is wired into the production
/// diagnostics pipeline (not just callable in unit tests).
#[test]
fn timeseries_broken_incremental_without_timeseries() {
    check_workspace_emits_timeseries_diagnostic(
        "examples/timeseries_broken_incremental_without_timeseries",
        "models/incremental_without_timeseries.sql",
        smelt_db::DiagnosticCode::TimeseriesRequiredForIncremental,
    );
}

// ===== BUG-006: CumulativeForbidsTimeseries / CumulativeForbidsIncremental regression =====
//
// Before the fix, `validate_timeseries` returned these errors but `file_diagnostics`
// silently dropped them via the `_ => None` catch-all in the match block.
//
// Fixtures:
//   - `examples/timeseries_broken_cumulative_with_timeseries/`   — CumulativeForbidsTimeseries
//   - `examples/timeseries_broken_cumulative_with_incremental/`  — CumulativeForbidsIncremental

/// Helper: loads `example_dir` as a workspace and asserts that exactly one
/// `CumulativeForbidsTimeseries` or `CumulativeForbidsIncremental` diagnostic fires
/// for the file ending in `expected_file`, and no such diagnostic fires in any other
/// file in the workspace.
fn check_workspace_emits_cumulative_frontmatter_diagnostic(
    example_dir: &str,
    expected_file: &str,
    expected_code: smelt_db::DiagnosticCode,
) {
    use smelt_cli::{init_db, Config, ModelDiscovery};
    use smelt_db::{DiagnosticAcc, Workspace};
    use std::path::Path;

    const CUMULATIVE_FRONTMATTER_CODES: &[smelt_db::DiagnosticCode] = &[
        smelt_db::DiagnosticCode::CumulativeForbidsTimeseries,
        smelt_db::DiagnosticCode::CumulativeForbidsIncremental,
    ];

    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join(example_dir);

    let config: Config =
        serde_yaml::from_str(&std::fs::read_to_string(path.join("smelt.yml")).unwrap()).unwrap();

    let discovery = ModelDiscovery::new(path.clone(), config.paths.clone());
    let mut models = discovery.discover_models().unwrap();
    let function_files = discovery.discover_function_files().unwrap();
    models.extend(function_files);

    let db = init_db(&path, &models);
    let ws = Workspace::try_get(&db).expect("workspace not initialized");

    let mut target_diags: Vec<smelt_db::Diagnostic> = Vec::new();
    let mut other_diags: Vec<(String, smelt_db::Diagnostic)> = Vec::new();

    let is_cumulative = |code: Option<&smelt_db::DiagnosticCode>| -> bool {
        code.is_some_and(|c| CUMULATIVE_FRONTMATTER_CODES.contains(c))
    };

    for model in &models {
        let file = match db.source_file(&model.path) {
            Some(f) => f,
            None => continue,
        };
        let rel = model
            .path
            .strip_prefix(&path)
            .unwrap()
            .display()
            .to_string();
        let is_target = rel
            .replace('\\', "/")
            .ends_with(&expected_file.replace('\\', "/"));

        for d in smelt_db::file_diagnostics(&db, ws, file).iter() {
            if !is_cumulative(d.code.as_ref()) {
                continue;
            }
            if is_target {
                target_diags.push(d.clone());
            } else {
                other_diags.push((rel.clone(), d.clone()));
            }
        }
        for d in smelt_db::check_type_diagnostics::accumulated::<DiagnosticAcc>(&db, ws, file) {
            if !is_cumulative(d.0.code.as_ref()) {
                continue;
            }
            if is_target {
                target_diags.push(d.0.clone());
            } else {
                other_diags.push((rel.clone(), d.0.clone()));
            }
        }
    }

    assert!(
        other_diags.is_empty(),
        "expected zero cumulative frontmatter diagnostics from files other than '{}' in {}, got {}:\n  {}",
        expected_file,
        example_dir,
        other_diags.len(),
        other_diags
            .iter()
            .map(|(f, d)| format!("[{:?}] {}: {}", d.code, f, d.message))
            .collect::<Vec<_>>()
            .join("\n  ")
    );

    assert_eq!(
        target_diags.len(),
        1,
        "expected exactly 1 cumulative frontmatter diagnostic from '{}' in {}, got {}:\n  {}",
        expected_file,
        example_dir,
        target_diags.len(),
        target_diags
            .iter()
            .map(|d| format!("[{:?}]: {}", d.code, d.message))
            .collect::<Vec<_>>()
            .join("\n  ")
    );

    assert_eq!(
        target_diags[0].code,
        Some(expected_code),
        "expected cumulative frontmatter diagnostic code {:?} from '{}' in {}, got {:?}: {}",
        expected_code,
        expected_file,
        example_dir,
        target_diags[0].code,
        target_diags[0].message
    );
}

/// BUG-006 regression: `examples/timeseries_broken_cumulative_with_timeseries/` produces
/// exactly one `CumulativeForbidsTimeseries` diagnostic from
/// `models/cumulative_with_timeseries.sql`.
///
/// Before the fix, `validate_timeseries` returned `CumulativeForbidsTimeseries`
/// but `file_diagnostics` silently dropped it (`_ => None` in the match block),
/// so the LSP showed no error even though cumulative models must not declare
/// `timeseries:` (cumulative_aggregate.md §"Output shape").
#[test]
fn timeseries_broken_cumulative_with_timeseries() {
    check_workspace_emits_cumulative_frontmatter_diagnostic(
        "examples/timeseries_broken_cumulative_with_timeseries",
        "models/cumulative_with_timeseries.sql",
        smelt_db::DiagnosticCode::CumulativeForbidsTimeseries,
    );
}

/// BUG-006 regression: `examples/timeseries_broken_cumulative_with_incremental/` produces
/// exactly one `CumulativeForbidsIncremental` diagnostic from
/// `models/cumulative_with_incremental.sql`.
///
/// Before the fix, `validate_timeseries` returned `CumulativeForbidsIncremental`
/// but `file_diagnostics` silently dropped it, so the LSP showed no error even
/// though cumulative models must not declare `incremental:` (cumulative_aggregate.md
/// §"Constraints & Invariants" #2).
#[test]
fn timeseries_broken_cumulative_with_incremental() {
    check_workspace_emits_cumulative_frontmatter_diagnostic(
        "examples/timeseries_broken_cumulative_with_incremental",
        "models/cumulative_with_incremental.sql",
        smelt_db::DiagnosticCode::CumulativeForbidsIncremental,
    );
}

// ===== Struct-field-type-validation TDD tests =====
//
// `examples/functions_broken_struct_field_type/` contains a function whose
// struct return type declares an unrecognized field type (`Bogus`). The
// workspace must emit exactly one `UnknownStructFieldType` anchored at the
// individual struct field's type-ref span; no other files in the workspace
// may emit that code.

/// Verifies that an unrecognized type name nested in a struct field position in
/// a function's return or parameter annotation emits `UnknownStructFieldType`
/// at the individual field's type-ref span (not the whole annotation).
///
/// The broken workspace `examples/functions_broken_struct_field_type/` contains
/// a `smelt.define` with `-> Expr<Struct<{a: Integer, b: Bogus}>>` where `Bogus`
/// is not a known DataType. The declaration must be flagged; no other file in
/// the workspace may produce this code.
#[test]
fn functions_broken_struct_field_type_emits_unknown_struct_field_type() {
    use smelt_cli::{init_db, Config, ModelDiscovery};
    use smelt_db::{DiagnosticAcc, Workspace};
    use std::path::Path;

    let example_dir = "examples/functions_broken_struct_field_type";
    let expected_file = "functions/broken_struct.sql";

    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join(example_dir);

    let config: Config =
        serde_yaml::from_str(&std::fs::read_to_string(path.join("smelt.yml")).unwrap()).unwrap();

    let discovery = ModelDiscovery::new(path.clone(), config.paths.clone());
    let mut models = discovery.discover_models().unwrap();
    let function_files = discovery.discover_function_files().unwrap();
    models.extend(function_files);

    let db = init_db(&path, &models);
    let ws = Workspace::try_get(&db).expect("workspace not initialized");

    let is_target_code = |code: Option<&smelt_db::DiagnosticCode>| -> bool {
        code == Some(&smelt_db::DiagnosticCode::UnknownStructFieldType)
    };

    let mut target_diags: Vec<smelt_db::Diagnostic> = Vec::new();
    let mut other_diags: Vec<(String, smelt_db::Diagnostic)> = Vec::new();

    for model in &models {
        let file = match db.source_file(&model.path) {
            Some(f) => f,
            None => continue,
        };
        let rel = model
            .path
            .strip_prefix(&path)
            .unwrap()
            .display()
            .to_string();
        let is_target = rel
            .replace('\\', "/")
            .ends_with(&expected_file.replace('\\', "/"));

        for d in smelt_db::file_diagnostics(&db, ws, file).iter() {
            if !is_target_code(d.code.as_ref()) {
                continue;
            }
            if is_target {
                target_diags.push(d.clone());
            } else {
                other_diags.push((rel.clone(), d.clone()));
            }
        }
        for d in smelt_db::check_type_diagnostics::accumulated::<DiagnosticAcc>(&db, ws, file) {
            if !is_target_code(d.0.code.as_ref()) {
                continue;
            }
            if is_target {
                target_diags.push(d.0.clone());
            } else {
                other_diags.push((rel.clone(), d.0.clone()));
            }
        }
    }

    assert!(
        other_diags.is_empty(),
        "expected zero UnknownStructFieldType diagnostics from files other than '{}' in {}, \
         got {}:\n  {}",
        expected_file,
        example_dir,
        other_diags.len(),
        other_diags
            .iter()
            .map(|(f, d)| format!("[{:?}] {}: {}", d.code, f, d.message))
            .collect::<Vec<_>>()
            .join("\n  ")
    );

    assert_eq!(
        target_diags.len(),
        1,
        "expected exactly 1 UnknownStructFieldType from '{}' in {}, got {}:\n  {}",
        expected_file,
        example_dir,
        target_diags.len(),
        target_diags
            .iter()
            .map(|d| format!("[{:?}]: {}", d.code, d.message))
            .collect::<Vec<_>>()
            .join("\n  ")
    );

    assert_eq!(
        target_diags[0].code,
        Some(smelt_db::DiagnosticCode::UnknownStructFieldType),
        "expected UnknownStructFieldType from '{}' in {}, got {:?}: {}",
        expected_file,
        example_dir,
        target_diags[0].code,
        target_diags[0].message
    );
}

// ===== AliasColumnArityMismatch broken-fixture TDD tests =====
//
// `examples/values_broken_alias_arity/` and
// `examples/cte_broken_alias_arity/` each contain one model that
// intentionally declares a mismatched alias column list.  Each workspace
// must emit exactly one `AliasColumnArityMismatch` diagnostic at the
// broken model; no other file in the workspace may emit that code.

/// Helper: loads `example_dir` as a workspace and checks that exactly one
/// `AliasColumnArityMismatch` diagnostic fires for the file ending in
/// `expected_file`, and that no other file in the workspace emits that code.
fn check_workspace_emits_exactly_one_alias_arity_mismatch(example_dir: &str, expected_file: &str) {
    use smelt_cli::{init_db, Config, ModelDiscovery};
    use smelt_db::{DiagnosticAcc, Workspace};
    use std::path::Path;

    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join(example_dir);

    let config: Config =
        serde_yaml::from_str(&std::fs::read_to_string(path.join("smelt.yml")).unwrap()).unwrap();

    let discovery = ModelDiscovery::new(path.clone(), config.paths.clone());
    let mut models = discovery.discover_models().unwrap();
    let function_files = discovery.discover_function_files().unwrap();
    models.extend(function_files);

    let db = init_db(&path, &models);
    let ws = Workspace::try_get(&db).expect("workspace not initialized");

    let is_target_code = |code: Option<&smelt_db::DiagnosticCode>| -> bool {
        code == Some(&smelt_db::DiagnosticCode::AliasColumnArityMismatch)
    };

    let mut target_diags: Vec<smelt_db::Diagnostic> = Vec::new();
    let mut other_diags: Vec<(String, smelt_db::Diagnostic)> = Vec::new();

    for model in &models {
        let file = match db.source_file(&model.path) {
            Some(f) => f,
            None => continue,
        };
        let rel = model
            .path
            .strip_prefix(&path)
            .unwrap()
            .display()
            .to_string();
        let is_target = rel
            .replace('\\', "/")
            .ends_with(&expected_file.replace('\\', "/"));

        for d in smelt_db::file_diagnostics(&db, ws, file).iter() {
            if !is_target_code(d.code.as_ref()) {
                continue;
            }
            if is_target {
                target_diags.push(d.clone());
            } else {
                other_diags.push((rel.clone(), d.clone()));
            }
        }
        for d in smelt_db::check_type_diagnostics::accumulated::<DiagnosticAcc>(&db, ws, file) {
            if !is_target_code(d.0.code.as_ref()) {
                continue;
            }
            if is_target {
                target_diags.push(d.0.clone());
            } else {
                other_diags.push((rel.clone(), d.0.clone()));
            }
        }
    }

    assert!(
        other_diags.is_empty(),
        "expected zero AliasColumnArityMismatch diagnostics from files other than '{}' in {}, \
         got {}:\n  {}",
        expected_file,
        example_dir,
        other_diags.len(),
        other_diags
            .iter()
            .map(|(f, d)| format!("[{:?}] {}: {}", d.code, f, d.message))
            .collect::<Vec<_>>()
            .join("\n  ")
    );

    assert_eq!(
        target_diags.len(),
        1,
        "expected exactly 1 AliasColumnArityMismatch from '{}' in {}, got {}:\n  {}",
        expected_file,
        example_dir,
        target_diags.len(),
        target_diags
            .iter()
            .map(|d| format!("[{:?}]: {}", d.code, d.message))
            .collect::<Vec<_>>()
            .join("\n  ")
    );

    assert_eq!(
        target_diags[0].code,
        Some(smelt_db::DiagnosticCode::AliasColumnArityMismatch),
        "expected AliasColumnArityMismatch from '{}' in {}, got {:?}: {}",
        expected_file,
        example_dir,
        target_diags[0].code,
        target_diags[0].message
    );
}

/// Phase 3 TDD: `examples/values_broken_alias_arity/` emits exactly one
/// `AliasColumnArityMismatch` at `models/broken_arity.sql`.
#[test]
fn values_broken_alias_arity_emits_arity_mismatch() {
    check_workspace_emits_exactly_one_alias_arity_mismatch(
        "examples/values_broken_alias_arity",
        "models/broken_arity.sql",
    );
}

/// Phase 3 TDD: `examples/cte_broken_alias_arity/` emits exactly one
/// `AliasColumnArityMismatch` at `models/broken_arity.sql`.
#[test]
fn cte_broken_alias_arity_emits_arity_mismatch() {
    check_workspace_emits_exactly_one_alias_arity_mismatch(
        "examples/cte_broken_alias_arity",
        "models/broken_arity.sql",
    );
}

// ===== Generator-emission typed schema TDD tests =====

/// Phase 2 closure assertion: `examples/per_cohort_union/models/all_cohorts_unioned.sql`
/// is a UNION ALL of three generator-emitted cohort models. Each emitted model's typed
/// schema is propagated to the consumer through the `smelt.<path>` resolver. All five
/// output columns must resolve to a concrete, non-Unknown type.
///
/// Prior to Phase 2 the FROM-clause typing path did not consult the emission registry,
/// so every projected column typed as Unknown.
#[test]
fn per_cohort_union_all_cohorts_unioned_has_concrete_typed_schema() {
    use smelt_cli::{init_db, Config, ModelDiscovery};
    use smelt_db::Workspace;
    use std::path::Path;

    let example_dir = "examples/per_cohort_union";
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join(example_dir);

    let config: Config =
        serde_yaml::from_str(&std::fs::read_to_string(path.join("smelt.yml")).unwrap()).unwrap();

    let discovery = ModelDiscovery::new(path.clone(), config.paths.clone());
    let mut models = discovery.discover_models().unwrap();
    let function_files = discovery.discover_function_files().unwrap();
    models.extend(function_files);

    let db = init_db(&path, &models);
    let ws = Workspace::try_get(&db).expect("workspace not initialized");

    // Find all_cohorts_unioned.sql
    let consumer_path = path.join("models").join("all_cohorts_unioned.sql");
    let consumer_file = db
        .source_file(&consumer_path)
        .expect("all_cohorts_unioned.sql not found in workspace");

    let schema = smelt_db::typed_model_schema(&db, ws, consumer_file);

    assert_eq!(
        schema.columns.len(),
        5,
        "all_cohorts_unioned.sql should export 5 columns, got {}:\n  {:?}",
        schema.columns.len(),
        schema.columns.iter().map(|c| &c.name).collect::<Vec<_>>()
    );

    for col in &schema.columns {
        let dt = col
            .data_type
            .as_ref()
            .map(|tc| tc.data_type.clone())
            .unwrap_or(smelt_db::DataType::unknown_dynamic());
        assert_ne!(
            dt,
            smelt_db::DataType::unknown_dynamic(),
            "all_cohorts_unioned.sql column '{}' should be concrete (from emitted cohort), \
             got Unknown",
            col.name
        );
    }
}

// ===== Emission-body diagnostics TDD tests =====
//
// Layout:
//   - `examples/per_cohort_union/`                                    — regression: still zero diagnostics
//   - `examples/staging_from_sources/`                                — regression: still zero diagnostics
//   - `examples/per_cohort_union_broken_emission_body_undeclared_column/`   — UndeclaredColumn in body
//   - `examples/per_cohort_union_broken_emission_body_parse_error/`         — ParseError in body
//   - `examples/per_cohort_union_broken_emission_body_cte_cycle/`           — CteCycle in body
//   - `examples/per_cohort_union_broken_emission_body_collision_suppression/` — ModelDefDuplicateName, zero body diags

/// Helper: loads `example_dir`, asserts exactly one emission-body diagnostic fires
/// for the file ending in `expected_file`, and that no other file emits the target code.
fn check_workspace_emits_exactly_one_emission_body_diagnostic(
    example_dir: &str,
    expected_file: &str,
    expected_code: smelt_db::DiagnosticCode,
) {
    use smelt_cli::{init_db, Config, ModelDiscovery};
    use smelt_db::{DiagnosticAcc, Workspace};
    use std::path::Path;

    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join(example_dir);

    let config: Config =
        serde_yaml::from_str(&std::fs::read_to_string(path.join("smelt.yml")).unwrap()).unwrap();

    let discovery = ModelDiscovery::new(path.clone(), config.paths.clone());
    let mut models = discovery.discover_models().unwrap();
    let function_files = discovery.discover_function_files().unwrap();
    models.extend(function_files);

    let db = init_db(&path, &models);
    let ws = Workspace::try_get(&db).expect("workspace not initialized");

    let mut target_diags: Vec<smelt_db::Diagnostic> = Vec::new();
    let mut other_diags: Vec<(String, smelt_db::Diagnostic)> = Vec::new();

    let is_target_code =
        |code: Option<&smelt_db::DiagnosticCode>| -> bool { code == Some(&expected_code) };

    for model in &models {
        let file = match db.source_file(&model.path) {
            Some(f) => f,
            None => continue,
        };
        let rel = model
            .path
            .strip_prefix(&path)
            .unwrap()
            .display()
            .to_string();
        let is_target = rel
            .replace('\\', "/")
            .ends_with(&expected_file.replace('\\', "/"));

        for d in smelt_db::file_diagnostics(&db, ws, file).iter() {
            if !is_target_code(d.code.as_ref()) {
                continue;
            }
            if is_target {
                target_diags.push(d.clone());
            } else {
                other_diags.push((rel.clone(), d.clone()));
            }
        }
        for d in smelt_db::check_type_diagnostics::accumulated::<DiagnosticAcc>(&db, ws, file) {
            if !is_target_code(d.0.code.as_ref()) {
                continue;
            }
            if is_target {
                target_diags.push(d.0.clone());
            } else {
                other_diags.push((rel.clone(), d.0.clone()));
            }
        }
    }

    assert!(
        other_diags.is_empty(),
        "expected zero {:?} diagnostics from files other than '{}' in {}, got {}:\n  {}",
        expected_code,
        expected_file,
        example_dir,
        other_diags.len(),
        other_diags
            .iter()
            .map(|(f, d)| format!("[{:?}] {}: {}", d.code, f, d.message))
            .collect::<Vec<_>>()
            .join("\n  ")
    );

    assert_eq!(
        target_diags.len(),
        1,
        "expected exactly 1 {:?} diagnostic from '{}' in {}, got {}:\n  {}",
        expected_code,
        expected_file,
        example_dir,
        target_diags.len(),
        target_diags
            .iter()
            .map(|d| format!("[{:?}]: {}", d.code, d.message))
            .collect::<Vec<_>>()
            .join("\n  ")
    );

    assert_eq!(
        target_diags[0].code,
        Some(expected_code),
        "expected {:?} from '{}' in {}, got {:?}: {}",
        expected_code,
        expected_file,
        example_dir,
        target_diags[0].code,
        target_diags[0].message
    );
}

/// Emission-body TDD: `examples/per_cohort_union_broken_emission_body_undeclared_column/`
/// produces exactly one `UndeclaredColumn` diagnostic anchored in the generator file body.
/// The body references `nonexistent_column` which is not declared in `smelt.orders`.
#[test]
fn per_cohort_union_broken_emission_body_undeclared_column() {
    check_workspace_emits_exactly_one_emission_body_diagnostic(
        "examples/per_cohort_union_broken_emission_body_undeclared_column",
        "models/broken.gen.sql",
        smelt_db::DiagnosticCode::UndeclaredColumn,
    );

    // Position regression: the diagnostic must land inside the body (line >= 3,
    // 0-indexed), not at the YAML frontmatter delimiter (line 0).
    // The fixture body is on line 3 of broken.gen.sql:
    //   line 0: ---
    //   line 1: generates: models
    //   line 2: ---
    //   line 3: [ModelDef { name: 'x', body: SELECT nonexistent_column FROM smelt.orders }]
    use smelt_cli::{init_db, Config, ModelDiscovery};
    use smelt_db::{DiagnosticAcc, Workspace};
    use std::path::Path;

    let example_dir = "examples/per_cohort_union_broken_emission_body_undeclared_column";
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join(example_dir);

    let config: Config =
        serde_yaml::from_str(&std::fs::read_to_string(path.join("smelt.yml")).unwrap()).unwrap();
    let discovery = ModelDiscovery::new(path.clone(), config.paths.clone());
    let mut models = discovery.discover_models().unwrap();
    let function_files = discovery.discover_function_files().unwrap();
    models.extend(function_files);

    let db = init_db(&path, &models);
    let ws = Workspace::try_get(&db).expect("workspace not initialized");

    let mut undeclared_diags: Vec<smelt_db::Diagnostic> = Vec::new();
    for model in &models {
        let file = match db.source_file(&model.path) {
            Some(f) => f,
            None => continue,
        };
        let rel = model
            .path
            .strip_prefix(&path)
            .unwrap()
            .display()
            .to_string();
        if !rel.replace('\\', "/").ends_with("models/broken.gen.sql") {
            continue;
        }
        for d in smelt_db::file_diagnostics(&db, ws, file).iter() {
            if d.code == Some(smelt_db::DiagnosticCode::UndeclaredColumn) {
                undeclared_diags.push(d.clone());
            }
        }
        for d in smelt_db::check_type_diagnostics::accumulated::<DiagnosticAcc>(&db, ws, file) {
            if d.0.code == Some(smelt_db::DiagnosticCode::UndeclaredColumn) {
                undeclared_diags.push(d.0.clone());
            }
        }
    }

    assert_eq!(
        undeclared_diags.len(),
        1,
        "expected exactly one UndeclaredColumn diagnostic"
    );
    // Convert the TextRange byte-offset to a line number for the assertion.
    let broken_model_path = path.join("models/broken.gen.sql");
    let broken_text =
        std::fs::read_to_string(&broken_model_path).expect("could not read broken.gen.sql");
    let li = LineIndex::new(&broken_text);
    let diag_start_line = li.line_col(undeclared_diags[0].range.start()).line;
    assert!(
        diag_start_line >= 3,
        "expected UndeclaredColumn diagnostic anchored inside the body (line >= 3, 0-indexed), \
         got line {} — diagnostic is likely pinned to the frontmatter delimiter (line 0) \
         rather than the body content",
        diag_start_line,
    );
}

/// Emission-body TDD: `examples/per_cohort_union_broken_emission_body_parse_error/`
/// produces exactly one `ParseError` diagnostic anchored at the body span.
/// The body contains a SQL syntax error (`SELEKT` keyword typo).
#[test]
fn per_cohort_union_broken_emission_body_parse_error() {
    check_workspace_emits_exactly_one_emission_body_diagnostic(
        "examples/per_cohort_union_broken_emission_body_parse_error",
        "models/broken.gen.sql",
        smelt_db::DiagnosticCode::ParseError,
    );
}

/// Emission-body TDD: `examples/per_cohort_union_broken_emission_body_cte_cycle/`
/// produces exactly one `CteCycle` diagnostic anchored in the generator body's WITH clause.
#[test]
fn per_cohort_union_broken_emission_body_cte_cycle() {
    check_workspace_emits_exactly_one_emission_body_diagnostic(
        "examples/per_cohort_union_broken_emission_body_cte_cycle",
        "models/broken.gen.sql",
        smelt_db::DiagnosticCode::CteCycle,
    );
}

/// Emission-body TDD: discarded-emission suppression.
///
/// `examples/per_cohort_union_broken_emission_body_collision_suppression/` declares a
/// generator that emits two `ModelDef`s with the same name `dup`, where the second body
/// also contains an `UndeclaredColumn` reference. Exactly one `ModelDefDuplicateName`
/// diagnostic fires (the W3 collision diagnostic); zero `UndeclaredColumn` diagnostics
/// fire because the discarded emission's body is not analysed.
#[test]
fn per_cohort_union_broken_emission_body_collision_suppression() {
    use smelt_cli::{init_db, Config, ModelDiscovery};
    use smelt_db::{DiagnosticAcc, Workspace};
    use std::path::Path;

    let example_dir = "examples/per_cohort_union_broken_emission_body_collision_suppression";
    let expected_gen = "models/broken.gen.sql";

    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join(example_dir);

    let config: Config =
        serde_yaml::from_str(&std::fs::read_to_string(path.join("smelt.yml")).unwrap()).unwrap();

    let discovery = ModelDiscovery::new(path.clone(), config.paths.clone());
    let mut models = discovery.discover_models().unwrap();
    let function_files = discovery.discover_function_files().unwrap();
    models.extend(function_files);

    let db = init_db(&path, &models);
    let ws = Workspace::try_get(&db).expect("workspace not initialized");

    let mut duplicate_name_diags: Vec<smelt_db::Diagnostic> = Vec::new();
    let mut undeclared_column_diags: Vec<smelt_db::Diagnostic> = Vec::new();

    for model in &models {
        let file = match db.source_file(&model.path) {
            Some(f) => f,
            None => continue,
        };
        for d in smelt_db::file_diagnostics(&db, ws, file).iter() {
            match d.code {
                Some(smelt_db::DiagnosticCode::ModelDefDuplicateName) => {
                    duplicate_name_diags.push(d.clone());
                }
                Some(smelt_db::DiagnosticCode::UndeclaredColumn) => {
                    undeclared_column_diags.push(d.clone());
                }
                _ => {}
            }
        }
        for d in smelt_db::check_type_diagnostics::accumulated::<DiagnosticAcc>(&db, ws, file) {
            match d.0.code {
                Some(smelt_db::DiagnosticCode::ModelDefDuplicateName) => {
                    duplicate_name_diags.push(d.0.clone());
                }
                Some(smelt_db::DiagnosticCode::UndeclaredColumn) => {
                    undeclared_column_diags.push(d.0.clone());
                }
                _ => {}
            }
        }
    }

    // Must have exactly one ModelDefDuplicateName.
    assert_eq!(
        duplicate_name_diags.len(),
        1,
        "expected exactly 1 ModelDefDuplicateName in {}/{}, got {}:\n  {}",
        example_dir,
        expected_gen,
        duplicate_name_diags.len(),
        duplicate_name_diags
            .iter()
            .map(|d| format!("[{:?}]: {}", d.code, d.message))
            .collect::<Vec<_>>()
            .join("\n  ")
    );

    // Must have zero UndeclaredColumn (discarded emission body not analysed).
    assert!(
        undeclared_column_diags.is_empty(),
        "expected zero UndeclaredColumn diagnostics (discarded emission body must not be analysed) \
         in {}, got {}:\n  {}",
        example_dir,
        undeclared_column_diags.len(),
        undeclared_column_diags
            .iter()
            .map(|d| format!("[{:?}]: {}", d.code, d.message))
            .collect::<Vec<_>>()
            .join("\n  ")
    );
}

/// Regression: each of `cohorts.us_west`, `cohorts.us_east`, and `cohorts.eu`
/// emitted by `examples/per_cohort_union/models/cohorts.gen.sql` must have
/// five typed (non-Unknown) columns after Phase 1 lands.
///
/// Prior to Phase 1 the emitted-model typed-schema query did not exist and
/// emitted models carried no column type information.
#[test]
fn per_cohort_union_emitted_cohorts_have_typed_schemas() {
    use smelt_cli::{init_db, Config, ModelDiscovery};
    use smelt_db::Workspace;
    use std::path::Path;

    let example_dir = "examples/per_cohort_union";
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join(example_dir);

    let config: Config =
        serde_yaml::from_str(&std::fs::read_to_string(path.join("smelt.yml")).unwrap()).unwrap();

    let discovery = ModelDiscovery::new(path.clone(), config.paths.clone());
    let mut models = discovery.discover_models().unwrap();
    let function_files = discovery.discover_function_files().unwrap();
    models.extend(function_files);

    let db = init_db(&path, &models);
    let ws = Workspace::try_get(&db).expect("workspace not initialized");

    let gen_path = path.join("models").join("cohorts.gen.sql");
    let gen_file = db
        .source_file(&gen_path)
        .expect("cohorts.gen.sql registered in workspace");

    // Each cohort emission must have 5 typed (non-Unknown) columns.
    let emission_names = ["us_west", "us_east", "eu"];
    for name in &emission_names {
        let schema = smelt_db::emitted_model_typed_schema(&db, ws, gen_file, name.to_string());
        assert_eq!(
            schema.columns.len(),
            5,
            "emission '{}' should have 5 columns, got {}: {:?}",
            name,
            schema.columns.len(),
            schema.columns.iter().map(|c| &c.name).collect::<Vec<_>>()
        );
        for col in &schema.columns {
            let dt = col
                .data_type
                .as_ref()
                .map(|tc| tc.data_type.clone())
                .unwrap_or(smelt_db::DataType::unknown_dynamic());
            assert_ne!(
                dt,
                smelt_db::DataType::unknown_dynamic(),
                "emission '{}' column '{}' should be concrete, got Unknown",
                name,
                col.name
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Non-ASCII broken fixture: E2E non-ASCII diagnostic anchoring gate
// ---------------------------------------------------------------------------

/// CLI gate: `examples/non_ascii_broken/` produces exactly one `UndeclaredColumn`
/// diagnostic anchored in `models/broken.gen.sql`.
///
/// The generator body `SELECT 1 AS α, nonexistent_column FROM smelt.upstream`
/// uses a 2-byte Greek letter (α, U+03B1) before the undeclared column, so the
/// byte-column position of the diagnostic differs from the UTF-16 position.
/// This fixture is used by the E2E position-encoding test to verify that
/// `publishDiagnostics` emits the correct `character` field under both encodings.
#[test]
fn non_ascii_broken_undeclared_column() {
    check_workspace_emits_exactly_one_emission_body_diagnostic(
        "examples/non_ascii_broken",
        "models/broken.gen.sql",
        smelt_db::DiagnosticCode::UndeclaredColumn,
    );
}

// ===== U5 Frontmatter-parity end-to-end TDD tests =====
//
// Four fixtures cover the three BUGs end-to-end:
//   - `examples/frontmatter_function_key_on_model/`        — positive: warns but builds as TABLE
//   - `examples/timeseries_broken_invalid_granularity/`    — MalformedTimeseries (BUG-023 e2e)
//   - `examples/timeseries_broken_unknown_key/`            — MalformedTimeseries (BUG-025 e2e)
//   - `examples/frontmatter_broken_unknown_key/`           — FrontmatterParseError (BUG-016 e2e)

/// U5 TDD: `examples/frontmatter_function_key_on_model/` emits exactly one
/// Warning-severity `FrontmatterParseError` (inapplicable `deterministic` key)
/// and zero Error-severity diagnostics.  The model's `materialization: table`
/// must be retained — the block is not dropped.
#[test]
fn frontmatter_function_key_on_model_emits_warning_not_error() {
    use smelt_cli::{init_db, Config, ModelDiscovery};
    use smelt_db::{DiagnosticAcc, DiagnosticSeverity, Workspace};
    use std::path::Path;

    let example_dir = "examples/frontmatter_function_key_on_model";
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join(example_dir);

    let config: Config =
        serde_yaml::from_str(&std::fs::read_to_string(path.join("smelt.yml")).unwrap()).unwrap();

    let discovery = ModelDiscovery::new(path.clone(), config.paths.clone());
    let mut models = discovery.discover_models().unwrap();
    let function_files = discovery.discover_function_files().unwrap();
    models.extend(function_files);

    let db = init_db(&path, &models);
    let ws = Workspace::try_get(&db).expect("workspace not initialized");

    let mut all_errors: Vec<smelt_db::Diagnostic> = Vec::new();
    let mut fm_warnings: Vec<smelt_db::Diagnostic> = Vec::new();

    for model in &models {
        let file = match db.source_file(&model.path) {
            Some(f) => f,
            None => continue,
        };
        for d in smelt_db::file_diagnostics(&db, ws, file).iter() {
            match d.severity {
                DiagnosticSeverity::Error => all_errors.push(d.clone()),
                DiagnosticSeverity::Warning
                    if d.code == Some(smelt_db::DiagnosticCode::FrontmatterParseError) =>
                {
                    fm_warnings.push(d.clone())
                }
                _ => {}
            }
        }
        for d in smelt_db::check_type_diagnostics::accumulated::<DiagnosticAcc>(&db, ws, file) {
            match d.0.severity {
                DiagnosticSeverity::Error => all_errors.push(d.0.clone()),
                DiagnosticSeverity::Warning
                    if d.0.code == Some(smelt_db::DiagnosticCode::FrontmatterParseError) =>
                {
                    fm_warnings.push(d.0.clone())
                }
                _ => {}
            }
        }
    }

    assert!(
        all_errors.is_empty(),
        "expected zero Error-severity diagnostics in {}, got {}:\n  {}",
        example_dir,
        all_errors.len(),
        all_errors
            .iter()
            .map(|d| format!("[{:?}] {:?}: {}", d.severity, d.code, d.message))
            .collect::<Vec<_>>()
            .join("\n  ")
    );

    assert_eq!(
        fm_warnings.len(),
        1,
        "expected exactly 1 FrontmatterParseError Warning in {}, got {}:\n  {}",
        example_dir,
        fm_warnings.len(),
        fm_warnings
            .iter()
            .map(|d| format!("[{:?}]: {}", d.code, d.message))
            .collect::<Vec<_>>()
            .join("\n  ")
    );

    assert!(
        fm_warnings[0].message.contains("deterministic"),
        "warning message must name the inapplicable key 'deterministic'; got: {}",
        fm_warnings[0].message
    );
}

/// U5 TDD: `examples/timeseries_broken_invalid_granularity/` emits exactly one
/// `MalformedTimeseries` Error — `granularity: fortnight` is not a valid value.
/// BUG-023 end-to-end regression.
#[test]
fn timeseries_broken_invalid_granularity_emits_malformed_timeseries() {
    check_workspace_emits_timeseries_diagnostic(
        "examples/timeseries_broken_invalid_granularity",
        "models/invalid_granularity.sql",
        smelt_db::DiagnosticCode::MalformedTimeseries,
    );
}

/// U5 TDD: `examples/timeseries_broken_unknown_key/` emits exactly one
/// `MalformedTimeseries` Error — unknown sub-key `partition_columm` (typo).
/// BUG-025 end-to-end regression.
#[test]
fn timeseries_broken_unknown_key_emits_malformed_timeseries() {
    check_workspace_emits_timeseries_diagnostic(
        "examples/timeseries_broken_unknown_key",
        "models/unknown_subkey.sql",
        smelt_db::DiagnosticCode::MalformedTimeseries,
    );
}

/// U5 TDD: `examples/frontmatter_broken_unknown_key/` emits exactly one
/// `FrontmatterParseError` Error — `mateializaton` is an unknown top-level key.
/// BUG-016 end-to-end regression.
#[test]
fn frontmatter_broken_unknown_key_emits_frontmatter_parse_error() {
    check_workspace_emits_exactly_one_emission_body_diagnostic(
        "examples/frontmatter_broken_unknown_key",
        "models/unknown_key.sql",
        smelt_db::DiagnosticCode::FrontmatterParseError,
    );
}

// ===== BUG-007: CTE-collision diagnostic (CteShadowsCallerCte) TDD test =====

/// BUG-007 TDD: `examples/expansion_broken_cte_caller_collision/` emits exactly
/// one `CteShadowsCallerCte` Error on `models/collision_model.sql`.
///
/// The model declares a CTE `helper` and calls `smelt.functions.with_helper`
/// whose body also declares a CTE `helper`. At codegen time the two CTEs
/// would collide; the analysis-time check refuses with `CteShadowsCallerCte`.
#[test]
fn expansion_broken_cte_caller_collision_emits_cte_shadows_caller_cte() {
    use smelt_cli::{init_db, Config, ModelDiscovery};
    use smelt_db::{DiagnosticAcc, Workspace};
    use std::path::Path;

    let example_dir = "examples/expansion_broken_cte_caller_collision";
    let expected_file = "models/collision_model.sql";

    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join(example_dir);

    let config: Config =
        serde_yaml::from_str(&std::fs::read_to_string(path.join("smelt.yml")).unwrap()).unwrap();

    let discovery = ModelDiscovery::new(path.clone(), config.paths.clone());
    let mut models = discovery.discover_models().unwrap();
    let function_files = discovery.discover_function_files().unwrap();
    models.extend(function_files);

    let db = init_db(&path, &models);
    let ws = Workspace::try_get(&db).expect("workspace not initialized");

    let mut target_diags: Vec<smelt_db::Diagnostic> = Vec::new();
    let mut other_diags: Vec<(String, smelt_db::Diagnostic)> = Vec::new();

    let is_cte_shadow = |code: Option<&smelt_db::DiagnosticCode>| -> bool {
        code.is_some_and(|c| *c == smelt_db::DiagnosticCode::CteShadowsCallerCte)
    };

    for model in &models {
        let file = match db.source_file(&model.path) {
            Some(f) => f,
            None => continue,
        };
        let rel = model
            .path
            .strip_prefix(&path)
            .unwrap()
            .display()
            .to_string();
        let is_target = rel
            .replace('\\', "/")
            .ends_with(&expected_file.replace('\\', "/"));

        for d in smelt_db::file_diagnostics(&db, ws, file).iter() {
            if !is_cte_shadow(d.code.as_ref()) {
                continue;
            }
            if is_target {
                target_diags.push(d.clone());
            } else {
                other_diags.push((rel.clone(), d.clone()));
            }
        }
        for d in smelt_db::check_type_diagnostics::accumulated::<DiagnosticAcc>(&db, ws, file) {
            if !is_cte_shadow(d.0.code.as_ref()) {
                continue;
            }
            if is_target {
                target_diags.push(d.0.clone());
            } else {
                other_diags.push((rel.clone(), d.0.clone()));
            }
        }
    }

    assert!(
        other_diags.is_empty(),
        "expected zero CteShadowsCallerCte diagnostics from files other than '{}', got {}:\n  {}",
        expected_file,
        other_diags.len(),
        other_diags
            .iter()
            .map(|(f, d)| format!("[{:?}] {}: {}", d.code, f, d.message))
            .collect::<Vec<_>>()
            .join("\n  ")
    );

    assert_eq!(
        target_diags.len(),
        1,
        "expected exactly 1 CteShadowsCallerCte diagnostic from '{}', got {}:\n  {}",
        expected_file,
        target_diags.len(),
        target_diags
            .iter()
            .map(|d| format!("[{:?}]: {}", d.code, d.message))
            .collect::<Vec<_>>()
            .join("\n  ")
    );

    assert_eq!(
        target_diags[0].code,
        Some(smelt_db::DiagnosticCode::CteShadowsCallerCte),
        "expected CteShadowsCallerCte, got {:?}: {}",
        target_diags[0].code,
        target_diags[0].message
    );
}

// ===== BUG-017: cross-family arithmetic TypeMismatch fixture =====

/// BUG-017: `examples/types_broken_crossfamily_add/` must emit exactly one
/// `TypeMismatch` Error diagnostic (cross-family `42 + '3'` arithmetic).
#[test]
fn types_broken_crossfamily_add_emits_type_mismatch() {
    use smelt_cli::{init_db, Config, ModelDiscovery};
    use smelt_db::Workspace;
    use std::path::Path;

    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("examples/types_broken_crossfamily_add");

    let config: Config =
        serde_yaml::from_str(&std::fs::read_to_string(path.join("smelt.yml")).unwrap()).unwrap();
    let discovery = ModelDiscovery::new(path.clone(), config.paths.clone());
    let models = discovery.discover_models().unwrap();
    let db = init_db(&path, &models);
    let ws = Workspace::try_get(&db).expect("workspace not initialized");

    let mut all_diags = Vec::new();
    for model in &models {
        let file = match db.source_file(&model.path) {
            Some(f) => f,
            None => continue,
        };
        for d in smelt_db::file_diagnostics(&db, ws, file).iter() {
            all_diags.push(d.clone());
        }
    }

    let type_mismatches: Vec<_> = all_diags
        .iter()
        .filter(|d| d.code == Some(smelt_db::DiagnosticCode::TypeMismatch))
        .collect();

    assert_eq!(
        type_mismatches.len(),
        1,
        "examples/types_broken_crossfamily_add must emit exactly 1 TypeMismatch; got {}:\n  {}",
        all_diags.len(),
        all_diags
            .iter()
            .map(|d| format!("[{:?}] {:?}: {}", d.severity, d.code, d.message))
            .collect::<Vec<_>>()
            .join("\n  ")
    );
    assert_eq!(
        type_mismatches[0].severity,
        smelt_db::DiagnosticSeverity::Error,
        "TypeMismatch for cross-family arithmetic must be Error severity"
    );
}

/// BUG-066 regression: a generator file whose filename does NOT end with
/// `.gen.sql` must be discovered as a generator (no parse errors) and must
/// emit models on `smelt build`.
///
/// Spec: meta_language.md §"Multi-model production" — "The `.gen.sql` extension
/// is a recommended convention; it is **not load-bearing**. The compiler
/// determines a file's status from the frontmatter alone."
#[test]
fn d3_meta_fn_config_generator_without_gen_suffix_no_diagnostics() {
    check_workspace_no_diagnostics("examples/d3_meta_fn_config");
}

// ===== §17 Collation TDD tests =====

/// §17 TDD: `examples/collation_clean/` produces zero diagnostics.
///
/// The model uses `COLLATE "C"` — a binary (portable) collation — so no
/// `NonPortableCollation` diagnostic should fire.
#[test]
fn collation_clean_workspace() {
    check_workspace_no_diagnostics("examples/collation_clean");
}

/// §17 TDD: `examples/collation_broken/` — `models/non_binary_collation.sql`
/// uses `COLLATE NOCASE` and must produce exactly one `NonPortableCollation`
/// Error diagnostic and no other diagnostics.
#[test]
fn collation_broken_non_binary() {
    use smelt_cli::{init_db, Config, ModelDiscovery};
    use smelt_db::{DiagnosticAcc, Workspace};

    let example_dir = "examples/collation_broken";
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join(example_dir);

    let config: Config =
        serde_yaml::from_str(&std::fs::read_to_string(path.join("smelt.yml")).unwrap()).unwrap();

    let discovery = ModelDiscovery::new(path.clone(), config.paths.clone());
    let models = discovery.discover_models().unwrap();

    let db = init_db(&path, &models);
    let ws = Workspace::try_get(&db).expect("workspace not initialized");

    let mut all_diags: Vec<smelt_db::Diagnostic> = Vec::new();
    for model in &models {
        let file = match db.source_file(&model.path) {
            Some(f) => f,
            None => continue,
        };
        for d in smelt_db::file_diagnostics(&db, ws, file).iter() {
            all_diags.push(d.clone());
        }
        for d in smelt_db::check_type_diagnostics::accumulated::<DiagnosticAcc>(&db, ws, file) {
            all_diags.push(d.0.clone());
        }
    }

    let collation_diags: Vec<_> = all_diags
        .iter()
        .filter(|d| d.code == Some(smelt_db::DiagnosticCode::NonPortableCollation))
        .collect();

    assert_eq!(
        collation_diags.len(),
        1,
        "expected exactly 1 NonPortableCollation diagnostic in {example_dir}; got {}:\n  {}",
        all_diags.len(),
        all_diags
            .iter()
            .map(|d| format!("[{:?}] {:?}: {}", d.severity, d.code, d.message))
            .collect::<Vec<_>>()
            .join("\n  ")
    );
    assert_eq!(
        collation_diags[0].severity,
        smelt_db::DiagnosticSeverity::Error,
        "NonPortableCollation must be Error severity"
    );
    assert_eq!(
        all_diags.len(),
        1,
        "collation_broken must emit exactly 1 diagnostic total (no extra diagnostics); got {}:\n  {}",
        all_diags.len(),
        all_diags
            .iter()
            .map(|d| format!("[{:?}] {:?}: {}", d.severity, d.code, d.message))
            .collect::<Vec<_>>()
            .join("\n  ")
    );
}

/// §17 standing collation gate: `examples/collation_clean/` — including the
/// binary-string grouping and ordering model — produces zero diagnostics.
///
/// This test is the positive companion to `collation_broken_non_binary`.  It
/// covers `models/binary_groupby_orderby.sql` which uses `GROUP BY` and
/// `ORDER BY` on a `Text` column without any COLLATE clause (implicit binary)
/// alongside `models/binary_collation.sql` which uses an explicit `COLLATE "C"`.
/// Neither model should emit a `NonPortableCollation` diagnostic.
#[test]
fn collation_clean_binary_groupby_orderby() {
    check_workspace_no_diagnostics("examples/collation_clean");
}

// ===== EventTimeColumnNotVisibleAtOuterSelect TDD tests =====
//
// Fixture: `examples/incremental_broken_union_event_time/`
//   — one broken incremental model that declares `timeseries:` with
//     `event_time_column: event_date` but uses a UNION ALL query, so the
//     time filter cannot be injected at the outermost SELECT.

/// Helper: loads `example_dir` as a workspace and asserts that exactly one
/// `EventTimeColumnNotVisibleAtOuterSelect` diagnostic fires for the file
/// ending in `expected_file`, and no such diagnostic fires in any other file
/// in the workspace.
fn check_workspace_emits_event_time_not_visible_diagnostic(example_dir: &str, expected_file: &str) {
    use smelt_cli::{init_db, Config, ModelDiscovery};
    use smelt_db::{DiagnosticAcc, Workspace};
    use std::path::Path;

    let target_code = smelt_db::DiagnosticCode::EventTimeColumnNotVisibleAtOuterSelect;

    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join(example_dir);

    let config: Config =
        serde_yaml::from_str(&std::fs::read_to_string(path.join("smelt.yml")).unwrap()).unwrap();

    let discovery = ModelDiscovery::new(path.clone(), config.paths.clone());
    let mut models = discovery.discover_models().unwrap();
    let function_files = discovery.discover_function_files().unwrap();
    models.extend(function_files);

    let db = init_db(&path, &models);
    let ws = Workspace::try_get(&db).expect("workspace not initialized");

    let mut target_diags: Vec<smelt_db::Diagnostic> = Vec::new();
    let mut other_diags: Vec<(String, smelt_db::Diagnostic)> = Vec::new();

    let is_target_code = |code: Option<&smelt_db::DiagnosticCode>| -> bool {
        code.is_some_and(|c| *c == target_code)
    };

    for model in &models {
        let file = match db.source_file(&model.path) {
            Some(f) => f,
            None => continue,
        };
        let rel = model
            .path
            .strip_prefix(&path)
            .unwrap()
            .display()
            .to_string();
        let is_target_file = rel
            .replace('\\', "/")
            .ends_with(&expected_file.replace('\\', "/"));

        for d in smelt_db::file_diagnostics(&db, ws, file).iter() {
            if !is_target_code(d.code.as_ref()) {
                continue;
            }
            if is_target_file {
                target_diags.push(d.clone());
            } else {
                other_diags.push((rel.clone(), d.clone()));
            }
        }
        for d in smelt_db::check_type_diagnostics::accumulated::<DiagnosticAcc>(&db, ws, file) {
            if !is_target_code(d.0.code.as_ref()) {
                continue;
            }
            if is_target_file {
                target_diags.push(d.0.clone());
            } else {
                other_diags.push((rel.clone(), d.0.clone()));
            }
        }
    }

    assert!(
        other_diags.is_empty(),
        "expected zero EventTimeColumnNotVisibleAtOuterSelect diagnostics from files other \
         than '{}' in {}, got {}:\n  {}",
        expected_file,
        example_dir,
        other_diags.len(),
        other_diags
            .iter()
            .map(|(f, d)| format!("[{:?}] {}: {}", d.code, f, d.message))
            .collect::<Vec<_>>()
            .join("\n  ")
    );

    assert_eq!(
        target_diags.len(),
        1,
        "expected exactly 1 EventTimeColumnNotVisibleAtOuterSelect diagnostic from '{}' in \
         {}, got {}:\n  {}",
        expected_file,
        example_dir,
        target_diags.len(),
        target_diags
            .iter()
            .map(|d| format!("[{:?}]: {}", d.code, d.message))
            .collect::<Vec<_>>()
            .join("\n  ")
    );

    assert_eq!(
        target_diags[0].code,
        Some(target_code),
        "unexpected diagnostic code from '{}' in {}: {:?}: {}",
        expected_file,
        example_dir,
        target_diags[0].code,
        target_diags[0].message
    );
}

/// Phase 6 TDD: `examples/incremental_broken_union_event_time/` produces
/// exactly one `EventTimeColumnNotVisibleAtOuterSelect` Error diagnostic
/// anchored at `models/union_mart.sql`.
///
/// The model declares `event_time_column: event_date` but uses a UNION ALL
/// query: injecting a WHERE clause would only filter the first branch and
/// produce incorrect results.
#[test]
fn incremental_broken_union_event_time() {
    check_workspace_emits_event_time_not_visible_diagnostic(
        "examples/incremental_broken_union_event_time",
        "models/union_mart.sql",
    );
}
