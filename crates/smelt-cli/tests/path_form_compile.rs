//! Phase 2b — verify that `smelt.<path>` path-form references are resolved
//! through `SqlCompiler` so that `smelt.models.*` / `smelt.sources.*` /
//! `smelt.seeds.*` calls are correctly rewritten to backend SQL.
//!
//! These tests cover the new `smelt_path_ref` and `smelt_path_call` closures
//! wired into `PrintContext`. No SQL execution — assertions are on emitted text.

use smelt_cli::compiler::CompilerRegistry;
use smelt_cli::config::{Config, Materialization, Target};
use smelt_cli::discovery::ModelDiscovery;
use smelt_cli::init_db;
use std::collections::HashMap;
use tempfile::TempDir;

/// Stage a workspace with the given file tree under a tempdir. Each entry is
/// `(relative_path_from_project_root, contents)`. Parent directories are
/// created automatically.
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
    let yml = "name: test_proj\n\
               version: 1\n\
               model_paths:\n  - models\n\
               targets:\n  default:\n    type: duckdb\n    schema: main\n\
               default_materialization: view\n";
    std::fs::write(tmp.path().join("smelt.yml"), yml).unwrap();
    tmp
}

/// Build a duckdb target with the given schema name.
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

/// Build a `Config` with the supplied targets.
fn config_with_targets(targets: HashMap<String, Target>) -> Config {
    Config {
        name: "test_proj".to_string(),
        version: 1,
        model_paths: vec!["models".to_string()],
        seed_paths: vec!["seeds".to_string()],
        targets,
        default_materialization: Materialization::View,
        models: HashMap::new(),
        python: None,
    }
}

// ─── Test 7: compiles_path_form_workspace_to_duckdb ──────────────────────────

/// CLI compile of a small path-form workspace produces backend SQL
/// that correctly resolves `smelt.models.users` to the schema-qualified name.
///
/// workspace:
///   models/users.sql       → SELECT 1 AS id
///   models/downstream.sql  → SELECT * FROM smelt.models.users
///
/// After compilation, `downstream` should contain `main.users` (schema-qualified).
#[test]
fn compiles_path_form_workspace_to_duckdb() {
    let users_sql = "SELECT 1 AS id\n";
    let downstream_sql = "SELECT * FROM smelt.models.users\n";

    let tmp = stage_workspace(&[
        ("models/users.sql", users_sql),
        ("models/downstream.sql", downstream_sql),
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

    let downstream = models
        .iter()
        .find(|m| m.name == "downstream")
        .expect("downstream model not found");

    let compiled = compilers
        .get("default")
        .compile(downstream, "main")
        .expect("compile should succeed");

    assert!(
        compiled.sql.contains("main.users"),
        "expected schema-qualified `main.users` in output, got: {}",
        compiled.sql
    );
    assert!(
        !compiled.sql.contains("smelt.models.users"),
        "smelt.models.users should be rewritten, got: {}",
        compiled.sql
    );
    let _ = db; // keep db alive
}

// ─── Test 8: compiles_path_form_seed_ref_to_duckdb ───────────────────────────

/// CLI compile resolves `smelt.seeds.raw.users` to a schema-qualified seed
/// table name (`main.raw_users`).
///
/// workspace:
///   models/uses_seed.sql  → SELECT * FROM smelt.seeds.raw.users
///
/// `make_path_ref_resolver` joins the non-namespace segments with '_' and
/// qualifies with the schema, so ["seeds", "raw", "users"] → `main.raw_users`.
#[test]
fn compiles_path_form_seed_ref_to_duckdb() {
    let seed_model_sql = "SELECT * FROM smelt.seeds.raw.users\n";

    let tmp = stage_workspace(&[("models/uses_seed.sql", seed_model_sql)]);
    let project_dir = tmp.path().to_path_buf();

    let discovery = ModelDiscovery::new(project_dir.clone(), vec!["models".to_string()]);
    let models = discovery.discover_models().unwrap();
    assert_eq!(
        models.len(),
        1,
        "expected 1 model, got: {:?}",
        models.iter().map(|m| &m.name).collect::<Vec<_>>()
    );

    let db = init_db(&project_dir, &models);

    let mut targets = HashMap::new();
    targets.insert("default".to_string(), duckdb_target("main"));
    let config = config_with_targets(targets.clone());
    let compilers = CompilerRegistry::new(&config, &targets);

    let uses_seed = models
        .iter()
        .find(|m| m.name == "uses_seed")
        .expect("uses_seed model not found");

    let compiled = compilers
        .get("default")
        .compile(uses_seed, "main")
        .expect("compile should succeed");

    assert!(
        compiled.sql.contains("main.raw_users"),
        "expected schema-qualified seed `main.raw_users` in output, got: {}",
        compiled.sql
    );
    assert!(
        !compiled.sql.contains("smelt.seeds.raw.users"),
        "smelt.seeds.raw.users should be rewritten, got: {}",
        compiled.sql
    );
    let _ = db; // keep db alive
}
