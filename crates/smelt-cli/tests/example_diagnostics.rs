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

    let discovery = ModelDiscovery::new(path.clone(), config.model_paths.clone());
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
        // `examples/broken/` is excluded: those fixtures intentionally use
        // legacy `smelt.fn.*` syntax to trigger specific type diagnostics.
        // Migrating them requires extending the function-call diagnostic
        // system to handle `SmeltPathCall` nodes (deferred).
        if entry.path().components().any(|c| c.as_os_str() == "broken") {
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
