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
