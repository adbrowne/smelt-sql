//! Phase 2c — end-to-end compile test: path form models compile correctly.
//!
//! Test 3: `compile_and_plan_path_workspace` — staging a tempdir workspace
//! with `smelt.models.base` in a FROM clause and asserting that the compiled
//! SQL contains the schema-qualified name `main.base` with no residual
//! `smelt.` prefix.

use smelt_cli::compiler::CompilerRegistry;
use smelt_cli::config::{Config, Materialization, Target};
use smelt_cli::discovery::ModelDiscovery;
use smelt_cli::init_db;
use std::collections::HashMap;
use tempfile::TempDir;

/// Stage a workspace under a tempdir and return it.
fn stage_workspace(files: &[(&str, &str)]) -> TempDir {
    let tmp = TempDir::new().expect("create tempdir");
    for (rel, contents) in files {
        let path = tmp.path().join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&path, contents).unwrap();
    }
    // Minimal smelt.yml so `find_project_root` recognises this as a project.
    let yml = "name: test_path_e2e\n\
               version: 1\n\
               model_paths:\n  - models\n\
               targets:\n  default:\n    type: duckdb\n    schema: main\n\
               default_materialization: view\n";
    std::fs::write(tmp.path().join("smelt.yml"), yml).unwrap();
    tmp
}

fn duckdb_target(schema: &str) -> Target {
    Target {
        target_type: "duckdb".to_string(),
        database: None,
        schema: schema.to_string(),
        connect_url: None,
        catalog: None,
        warehouse: None,
        format: None,
    }
}

fn config_with_targets(targets: HashMap<String, Target>) -> Config {
    Config {
        name: "test_path_e2e".to_string(),
        version: 1,
        model_paths: vec!["models".to_string()],
        seed_paths: vec!["seeds".to_string()],
        targets,
        default_materialization: Materialization::View,
        models: HashMap::new(),
        python: None,
    }
}

// ---------------------------------------------------------------------------
// Test 3: compile_and_plan_path_workspace
// ---------------------------------------------------------------------------

/// Compile a simple two-model workspace where `derived.sql` uses
/// `smelt.models.base` (path form) in its FROM clause. Assert that the
/// compiled SQL for `derived` contains `main.base` (schema-qualified) and
/// contains no residual `smelt.` prefix.
///
/// This test does NOT run DuckDB — it only checks SQL text.
#[test]
fn compile_and_plan_path_workspace() {
    let tmp = stage_workspace(&[
        ("models/base.sql", "SELECT 1 AS id, 'alice' AS name\n"),
        (
            "models/derived.sql",
            "SELECT id, name FROM smelt.models.base\n",
        ),
    ]);
    let project_dir = tmp.path().to_path_buf();

    let discovery = ModelDiscovery::new(project_dir.clone(), vec!["models".to_string()]);
    let models = discovery.discover_models().unwrap();
    assert_eq!(
        models.len(),
        2,
        "expected 2 models, got: {:?}",
        models.iter().map(|m| &m.name).collect::<Vec<_>>()
    );

    let db = init_db(&project_dir, &models);

    let mut targets = HashMap::new();
    targets.insert("default".to_string(), duckdb_target("main"));
    let config = config_with_targets(targets.clone());
    let compilers = CompilerRegistry::new(&config, &targets);

    let derived = models
        .iter()
        .find(|m| m.name == "derived")
        .expect("derived model not found");

    let compiled = compilers
        .get("default")
        .compile(derived, "main")
        .expect("compile should succeed");

    assert!(
        compiled.sql.contains("main.base"),
        "expected schema-qualified `main.base` in output, got:\n{}",
        compiled.sql
    );
    assert!(
        !compiled.sql.contains("smelt."),
        "no residual `smelt.` prefix should remain in output, got:\n{}",
        compiled.sql
    );

    let _ = db; // keep db alive
}
