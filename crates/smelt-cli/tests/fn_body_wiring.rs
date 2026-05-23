//! Phase 56 — verify that `smelt.fn.*` function bodies are wired through
//! `CompilerRegistry` so that `smelt build` / `smelt run` actually expands
//! call sites instead of emitting verbatim `smelt.fn.*` text.
//!
//! These tests cover the orchestration helper [`smelt_cli::build_fn_body_map`]
//! and the registry-level setter [`CompilerRegistry::set_function_bodies_all`]
//! that the production codepaths now invoke. They do not execute SQL — they
//! only assert on emitted text, so no `bundled-duckdb` feature is required.

use smelt_cli::config::{Config, Materialization, Target};
use smelt_cli::discovery::ModelDiscovery;
use smelt_cli::init_db;
use smelt_runtime::{build_fn_body_map, CompilerRegistry, SqlCompiler};
use std::collections::HashMap;
use tempfile::TempDir;

const SAFE_DIVIDE_BODY: &str = "---\nbackends: [duckdb]\n---\n\
smelt.define safe_divide(numerator: Expr<Numeric>, denominator: Expr<Numeric>) -> Expr<Double>\n\
    AS (CASE WHEN denominator = 0 OR denominator IS NULL THEN NULL ELSE CAST(numerator AS DOUBLE) / CAST(denominator AS DOUBLE) END)\n";

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
               paths:\n  - models\n\
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
        paths: vec!["models".to_string()],
        targets,
        default_materialization: Materialization::View,
        models: HashMap::new(),
        python: None,
    }
}

// ─── Test 1: build_fn_body_map extracts safe_divide ──────────────────────────

#[test]
fn build_fn_body_map_extracts_safe_divide_body() {
    let tmp = stage_workspace(&[("functions/safe_divide.sql", SAFE_DIVIDE_BODY)]);
    let project_dir = tmp.path().to_path_buf();

    let discovery = ModelDiscovery::new(project_dir.clone(), vec!["models".to_string()]);
    let function_files = discovery.discover_function_files().unwrap();
    assert_eq!(
        function_files.len(),
        1,
        "expected one function file, got: {:?}",
        function_files
    );

    let db = init_db(&project_dir, &function_files);
    let workspace = smelt_db::Workspace::try_get(&db).expect("workspace not initialised");
    let map = build_fn_body_map(&db, workspace);

    let entry = map
        .get("safe_divide")
        .expect("safe_divide entry missing from fn body map");
    let param_names: Vec<&str> = entry.0.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        param_names,
        vec!["numerator", "denominator"],
        "param names should be in source order"
    );
    assert!(
        entry.1.contains("CASE WHEN denominator = 0"),
        "extracted body should contain the canonical CASE WHEN; got: {:?}",
        entry.1
    );
}

// ─── Test 2: end-to-end registry wiring expands a smelt.functions.* call ────────────

#[test]
fn smelt_fn_call_expanded_via_registry_wiring() {
    let model_sql = "SELECT smelt.functions.safe_divide(revenue, cost) AS ratio FROM main.orders";
    let tmp = stage_workspace(&[
        ("functions/safe_divide.sql", SAFE_DIVIDE_BODY),
        ("models/uses_safe_divide.sql", model_sql),
    ]);
    let project_dir = tmp.path().to_path_buf();

    let discovery = ModelDiscovery::new(project_dir.clone(), vec!["models".to_string()]);
    let models = discovery.discover_models().unwrap();
    let function_files = discovery.discover_function_files().unwrap();

    let mut db_files = models.clone();
    db_files.extend(function_files.iter().cloned());
    let db = init_db(&project_dir, &db_files);

    let workspace = smelt_db::Workspace::try_get(&db).expect("workspace not initialised");
    let fn_bodies = build_fn_body_map(&db, workspace);
    assert!(
        fn_bodies.contains_key("safe_divide"),
        "fn body map should contain safe_divide; got keys: {:?}",
        fn_bodies.keys().collect::<Vec<_>>()
    );

    let mut targets = HashMap::new();
    targets.insert("default".to_string(), duckdb_target("main"));
    let config = config_with_targets(targets.clone());
    let mut compilers = CompilerRegistry::new(&config, &targets);
    compilers.set_function_bodies_all(fn_bodies);

    let model = models
        .iter()
        .find(|m| m.name == "uses_safe_divide")
        .expect("expected uses_safe_divide model");
    let compiled = compilers
        .get("default")
        .compile(model, "main")
        .expect("compile should succeed");

    assert!(
        compiled.sql.contains("CASE WHEN cost = 0"),
        "expanded body should substitute denominator → cost; got: {}",
        compiled.sql
    );
    assert!(
        !compiled.sql.contains("smelt.functions.safe_divide"),
        "smelt.functions.safe_divide call should be replaced; got: {}",
        compiled.sql
    );
}

// ─── Test 3: set_function_bodies_all propagates to every target ─────────────

#[test]
fn compiler_registry_set_function_bodies_propagates_to_all_targets() {
    let model_sql = "SELECT smelt.functions.safe_divide(a, b) AS r FROM main.t";
    let tmp = stage_workspace(&[
        ("functions/safe_divide.sql", SAFE_DIVIDE_BODY),
        ("models/uses.sql", model_sql),
    ]);
    let project_dir = tmp.path().to_path_buf();

    let discovery = ModelDiscovery::new(project_dir.clone(), vec!["models".to_string()]);
    let models = discovery.discover_models().unwrap();
    let function_files = discovery.discover_function_files().unwrap();

    let mut db_files = models.clone();
    db_files.extend(function_files.iter().cloned());
    let db = init_db(&project_dir, &db_files);
    let workspace = smelt_db::Workspace::try_get(&db).expect("workspace not initialised");
    let fn_bodies = build_fn_body_map(&db, workspace);
    assert!(!fn_bodies.is_empty(), "fn body map should be populated");

    // Two distinct targets, each with its own schema, both DuckDB.
    let mut targets: HashMap<String, Target> = HashMap::new();
    targets.insert("dev".to_string(), duckdb_target("dev_schema"));
    targets.insert("prod".to_string(), duckdb_target("prod_schema"));

    let config = config_with_targets(targets.clone());
    let mut compilers = CompilerRegistry::new(&config, &targets);
    compilers.set_function_bodies_all(fn_bodies);

    let model = models.iter().find(|m| m.name == "uses").unwrap();

    let dev_sql = compilers
        .get("dev")
        .compile(model, "dev_schema")
        .unwrap()
        .sql;
    let prod_sql = compilers
        .get("prod")
        .compile(model, "prod_schema")
        .unwrap()
        .sql;

    for (label, sql) in [("dev", &dev_sql), ("prod", &prod_sql)] {
        assert!(
            sql.contains("CASE WHEN b = 0"),
            "{} target should have expanded body; got: {}",
            label,
            sql
        );
        assert!(
            !sql.contains("smelt.functions.safe_divide"),
            "{} target should not still contain smelt.functions.safe_divide; got: {}",
            label,
            sql
        );
    }
}

// ─── Test 4: model files without defines are skipped, no panic on no body ───

#[test]
fn build_fn_body_map_skips_files_without_defines_and_handles_no_body_gracefully() {
    // One regular model file (no smelt.define), one function file with a
    // valid smelt.define body. Map must contain only the smelt.define entry.
    let trivial_define = "smelt.define identity(x: Expr<Int>) -> Expr<Int> AS (x)\n";
    let tmp = stage_workspace(&[
        ("models/plain.sql", "SELECT 1 AS x\n"),
        ("functions/identity.sql", trivial_define),
    ]);
    let project_dir = tmp.path().to_path_buf();

    let discovery = ModelDiscovery::new(project_dir.clone(), vec!["models".to_string()]);
    let models = discovery.discover_models().unwrap();
    let function_files = discovery.discover_function_files().unwrap();

    let mut db_files = models.clone();
    db_files.extend(function_files.iter().cloned());
    let db = init_db(&project_dir, &db_files);
    let workspace = smelt_db::Workspace::try_get(&db).expect("workspace not initialised");
    let map = build_fn_body_map(&db, workspace);

    assert_eq!(
        map.len(),
        1,
        "only the smelt.define should produce an entry; got {} entries: {:?}",
        map.len(),
        map.keys().collect::<Vec<_>>()
    );
    let entry = map.get("identity").expect("identity entry missing");
    let param_names: Vec<&str> = entry.0.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(param_names, vec!["x"]);
    assert!(
        entry.1.contains('x'),
        "identity body should contain x; got: {:?}",
        entry.1
    );
}

// ─── Test 5: legacy project (no functions/ dir) compiles unchanged ──────────

#[test]
fn legacy_project_without_functions_directory_compiles_unchanged() {
    // No functions/ directory at all. discover_function_files() should return
    // an empty Vec, build_fn_body_map should return an empty map, and a
    // model that uses zero smelt.fn.* calls must compile to identical SQL
    // both with and without the fn-body wiring step.
    let model_sql = "SELECT id, name FROM main.users\n";
    let tmp = stage_workspace(&[("models/users_view.sql", model_sql)]);
    let project_dir = tmp.path().to_path_buf();

    let discovery = ModelDiscovery::new(project_dir.clone(), vec!["models".to_string()]);
    let models = discovery.discover_models().unwrap();
    let function_files = discovery.discover_function_files().unwrap();
    assert!(
        function_files.is_empty(),
        "legacy project should have no function files"
    );

    let mut db_files = models.clone();
    db_files.extend(function_files.iter().cloned());
    let db = init_db(&project_dir, &db_files);
    let workspace = smelt_db::Workspace::try_get(&db).expect("workspace not initialised");
    let fn_bodies = build_fn_body_map(&db, workspace);
    assert!(
        fn_bodies.is_empty(),
        "legacy project should yield empty fn body map; got: {:?}",
        fn_bodies.keys().collect::<Vec<_>>()
    );

    // Compile path A: through the wiring (set_function_bodies_all called
    // even with empty map would change behaviour, so the production code
    // guards on is_empty — mirror that here by skipping the call).
    let mut targets = HashMap::new();
    targets.insert("default".to_string(), duckdb_target("main"));
    let config = config_with_targets(targets.clone());
    let mut compilers_wired = CompilerRegistry::new(&config, &targets);
    if !fn_bodies.is_empty() {
        compilers_wired.set_function_bodies_all(fn_bodies);
    }

    let model = models.iter().find(|m| m.name == "users_view").unwrap();
    let wired_sql = compilers_wired
        .get("default")
        .compile(model, "main")
        .unwrap()
        .sql;

    // Compile path B: a fresh SqlCompiler that never had any fn-body
    // configuration touched.
    let plain_compiler = SqlCompiler::new(config, &duckdb_target("main"));
    let plain_sql = plain_compiler.compile(model, "main").unwrap().sql;

    assert_eq!(
        wired_sql, plain_sql,
        "wiring with empty fn-body map must not change emitted SQL"
    );
}
