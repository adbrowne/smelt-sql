//! Integration tests for source name resolution.
//!
//! Test 1 (`aggregate_sources_yml_stops_build`): Verifies that
//! `UpstreamSchemas::from_database` returns a hard error when a legacy
//! aggregate `sources.yml` is present at the project root.
//!
//! Test 2 (`name_override_appears_in_compiled_sql`): Verifies that a per-entity
//! source YAML with `name: warehouse.users_v2` causes the compiler to emit
//! `warehouse.users_v2` in the FROM clause of a model that references
//! `smelt.sources.raw.users`.
//!
//! Test 3 (`no_name_override_uses_unified_default_mapping`): BUG-033 regression
//! test. Verifies that a per-entity source with NO `name:` override follows the
//! same unified default mapping as models — `<target_schema>.<path-joined-by-_>`
//! — rather than the old special-cased `<source_name>.<table_name>` legacy mapping.
//! For `smelt.sources.raw.users` this means `main.sources_raw_users`, not `raw.users`.

use smelt_cli::config::{Config, Materialization, Target};
use smelt_cli::discovery::ModelDiscovery;
use smelt_cli::init_db;
use smelt_runtime::{CompilerRegistry, UpstreamSchemas};
use std::collections::HashMap;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Shared test helpers
// ---------------------------------------------------------------------------

/// Stage a workspace with the given file tree under a tempdir.
/// Each entry is `(relative_path_from_project_root, contents)`.
/// Parent directories are created automatically.
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
    let yml = "name: test_proj\nversion: 1\npaths:\n  - models\n\
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
        name: "test_proj".to_string(),
        version: 1,
        paths: vec!["models".to_string()],
        targets,
        default_materialization: Materialization::View,
        models: HashMap::new(),
        python: None,
        target: None,
    }
}

// ---------------------------------------------------------------------------
// Test 1: aggregate sources.yml stops the build
// ---------------------------------------------------------------------------

/// A project with a legacy aggregate `sources.yml` at the project root must
/// produce an error from `UpstreamSchemas::from_database`.
///
/// This is the runtime integration counterpart to the isolation test in
/// `smelt_core::tests::source_yaml::aggregate_sources_yml_errors`.  It proves
/// the guard fires on the actual CLI workspace-load path (not just through
/// the pure function directly).
#[test]
fn aggregate_sources_yml_stops_build() {
    let tmp = stage_workspace(&[
        // A regular model so discovery finds at least one file.
        ("models/users.sql", "SELECT 1 AS id"),
    ]);
    let project_dir = tmp.path().to_path_buf();

    // Drop a legacy aggregate sources.yml at the project root.
    let legacy_sources = r#"
sources:
  raw:
    tables:
      users:
        columns:
          - name: id
            type: INTEGER
"#;
    std::fs::write(project_dir.join("sources.yml"), legacy_sources).unwrap();

    let discovery = ModelDiscovery::new(project_dir.clone(), vec!["models".to_string()]);
    let models = discovery.discover_models().unwrap();
    let db = init_db(&project_dir, &models);

    let result = UpstreamSchemas::from_database(&db, &project_dir, &models);
    assert!(
        result.is_err(),
        "expected an error when aggregate sources.yml is present, got Ok"
    );

    let msg = match result {
        Err(e) => e.to_string(),
        Ok(_) => unreachable!(),
    };
    assert!(
        msg.contains("Aggregate sources file not supported")
            || msg.contains("sources.yml")
            || msg.contains("per-entity"),
        "error message should mention the aggregate sources.yml migration, got: {msg}"
    );
}

// ---------------------------------------------------------------------------
// Test 2: name_override appears in compiled SQL
// ---------------------------------------------------------------------------

/// A per-entity source YAML with `name: warehouse.users_v2` must cause the
/// compiled SQL to use `warehouse.users_v2` in the FROM clause instead of
/// the default `main.sources_raw_users` mapping.
///
/// workspace:
///   models/sources/raw/users.yml  → source with name: warehouse.users_v2
///   models/use_source.sql         → SELECT * FROM smelt.sources.raw.users
///
/// After compilation the emitted SQL must contain `warehouse.users_v2`.
#[test]
fn name_override_appears_in_compiled_sql() {
    let source_yaml = r#"
columns:
  - name: id
    type: INTEGER
  - name: email
    type: TEXT
name: warehouse.users_v2
"#;

    let model_sql = "SELECT id, email FROM smelt.sources.raw.users\n";

    let tmp = stage_workspace(&[
        ("models/sources/raw/users.yml", source_yaml),
        ("models/use_source.sql", model_sql),
    ]);
    let project_dir = tmp.path().to_path_buf();

    let discovery = ModelDiscovery::new(project_dir.clone(), vec!["models".to_string()]);
    let models = discovery.discover_models().unwrap();

    // Only the SQL model is a model (the YAML is a source).
    let use_source = models
        .iter()
        .find(|m| m.name == "use_source")
        .expect("use_source model not found");

    let db = init_db(&project_dir, &models);

    let mut targets = HashMap::new();
    targets.insert("default".to_string(), duckdb_target("main"));
    let config = config_with_targets(targets.clone());
    let mut compilers = CompilerRegistry::new(&config, &targets);

    let upstream_schemas = UpstreamSchemas::from_database(&db, &project_dir, &models)
        .expect("from_database should succeed — no aggregate sources.yml present");
    compilers.set_upstream_schemas_all(std::sync::Arc::new(upstream_schemas));

    let compiled = compilers
        .get("default")
        .compile(use_source, "main")
        .expect("compile should succeed");

    assert!(
        compiled.sql.contains("warehouse.users_v2"),
        "compiled SQL must contain the name: override `warehouse.users_v2`, got:\n{}",
        compiled.sql
    );
    assert!(
        !compiled.sql.contains("smelt.sources.raw.users"),
        "smelt.sources.raw.users should be resolved, got:\n{}",
        compiled.sql
    );
    assert!(
        !compiled.sql.contains("main.sources_raw_users"),
        "default mapping `main.sources_raw_users` should NOT appear when name: override is set, got:\n{}",
        compiled.sql
    );
}

// ---------------------------------------------------------------------------
// Test 3: no name: override → unified default mapping (BUG-033)
// ---------------------------------------------------------------------------

/// BUG-033 regression test: a per-entity source with NO `name:` override must
/// resolve via the same unified default mapping as models:
///   `<target_schema>.<address-path-joined-by-_>`
///
/// For `smelt.sources.raw.users` with `schema: main` → `main.sources_raw_users`
/// NOT the old legacy `raw.users`.
///
/// workspace:
///   models/sources/raw/users.yml  → source WITHOUT name: override
///   models/use_source.sql         → SELECT * FROM smelt.sources.raw.users
///
/// After compilation the emitted SQL must contain `main.sources_raw_users`,
/// not `raw.users`.
#[test]
fn no_name_override_uses_unified_default_mapping() {
    let source_yaml = r#"
columns:
  - name: id
    type: INTEGER
  - name: email
    type: TEXT
"#;

    let model_sql = "SELECT id, email FROM smelt.sources.raw.users\n";

    let tmp = stage_workspace(&[
        ("models/sources/raw/users.yml", source_yaml),
        ("models/use_source.sql", model_sql),
    ]);
    let project_dir = tmp.path().to_path_buf();

    let discovery = ModelDiscovery::new(project_dir.clone(), vec!["models".to_string()]);
    let models = discovery.discover_models().unwrap();

    let use_source = models
        .iter()
        .find(|m| m.name == "use_source")
        .expect("use_source model not found");

    let db = init_db(&project_dir, &models);

    let mut targets = HashMap::new();
    targets.insert("default".to_string(), duckdb_target("main"));
    let config = config_with_targets(targets.clone());
    let mut compilers = CompilerRegistry::new(&config, &targets);

    let upstream_schemas = UpstreamSchemas::from_database(&db, &project_dir, &models)
        .expect("from_database should succeed — no aggregate sources.yml present");
    compilers.set_upstream_schemas_all(std::sync::Arc::new(upstream_schemas));

    let compiled = compilers
        .get("default")
        .compile(use_source, "main")
        .expect("compile should succeed");

    assert!(
        compiled.sql.contains("main.sources_raw_users"),
        "compiled SQL must use unified default mapping `main.sources_raw_users` \
         (not legacy `raw.users`) when no name: override is set, got:\n{}",
        compiled.sql
    );
    assert!(
        !compiled.sql.contains("raw.users"),
        "legacy mapping `raw.users` must NOT appear when no name: override is set, got:\n{}",
        compiled.sql
    );
    assert!(
        !compiled.sql.contains("smelt.sources.raw.users"),
        "smelt.sources.raw.users should be resolved, got:\n{}",
        compiled.sql
    );
}
