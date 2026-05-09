//! Verify that non-broken example workspaces produce zero LSP diagnostics.
//!
//! This test ensures that `file_diagnostics()` and `check_type_diagnostics()`
//! report no warnings or errors for any model in the example workspaces.
//! Regressions introduced by parser, type-inference, or example changes are
//! caught here.

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
