//! Integration tests for the combined SQL-generator + Python fixed-point loop.
//!
//! These tests drive `run_combined_discovery_loop` directly without needing a
//! real DuckDB connection — no backend target is required because discovery
//! only runs the Salsa analysis layer and the Python runner.
//!
//! TDD gate: these tests were written BEFORE `combined_loop.rs` was implemented;
//! they initially fail with "cannot find function `run_combined_discovery_loop`"
//! and turn green once the implementation is in place.

use smelt_core::config::{Config, Materialization};
use smelt_core::ModelDiscovery;
use smelt_runtime::run_combined_discovery_loop;
use std::collections::HashMap;
use std::path::Path;

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Copy the smelt Python SDK from the repo into `<project_dir>/python/smelt/`.
fn setup_sdk(project_dir: &Path) {
    let sdk_dir = project_dir.join("python").join("smelt");
    std::fs::create_dir_all(&sdk_dir).unwrap();
    let repo_sdk = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap() // crates/
        .parent()
        .unwrap() // repo root
        .join("python")
        .join("smelt");
    for entry in std::fs::read_dir(&repo_sdk).unwrap() {
        let entry = entry.unwrap();
        if entry.path().is_file() {
            std::fs::copy(entry.path(), sdk_dir.join(entry.file_name())).unwrap();
        }
    }
}

/// Write a minimal `smelt.yml` referencing only paths (no backend target needed
/// for discovery).
fn write_smelt_yml(project_dir: &Path, paths: &[&str]) {
    let paths_yaml = paths
        .iter()
        .map(|p| format!("  - {p}"))
        .collect::<Vec<_>>()
        .join("\n");
    let yml = format!(
        "name: combined_loop_test\nversion: 1\npaths:\n{paths_yaml}\ndefault_materialization: view\n"
    );
    std::fs::write(project_dir.join("smelt.yml"), yml).unwrap();
}

/// Build a minimal `Config` for a project dir with the given scan paths.
fn minimal_config(_project_dir: &Path, paths: Vec<String>) -> Config {
    Config {
        name: "combined_loop_test".into(),
        version: 1,
        paths,
        targets: HashMap::new(),
        target: None,
        default_materialization: Materialization::View,
        models: HashMap::new(),
        python: None,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// Test 1: a workspace with one plain SQL model and one Python @model file
/// that doesn't reference any other model.  The loop should converge in one
/// round and return both models.  No `.gen.sql` generator files should appear
/// in the output.
#[test]
fn combined_loop_converges_when_independent() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project_dir = tmp.path();
    std::fs::create_dir_all(project_dir.join("models")).unwrap();

    setup_sdk(project_dir);
    write_smelt_yml(project_dir, &["models"]);

    // Plain SQL model.
    std::fs::write(
        project_dir.join("models").join("base.sql"),
        "SELECT 1 AS id",
    )
    .unwrap();

    // Python model that doesn't reference any other model.
    std::fs::write(
        project_dir.join("models").join("gen.py"),
        r#"
from smelt import model

@model
def py_generated(project):
    return "SELECT 42 AS result"
"#,
    )
    .unwrap();

    let config = minimal_config(project_dir, vec!["models".to_string()]);
    let discovery = ModelDiscovery::new(project_dir.to_path_buf(), config.paths.clone());

    let raw_sql = discovery.discover_models().expect("discover_models");
    let python_files = discovery
        .discover_python_files()
        .expect("discover_python_files");

    let result = run_combined_discovery_loop(
        raw_sql,
        python_files,
        project_dir,
        &config,
        config.python.as_deref(),
        None,
    )
    .expect("combined loop must succeed");

    // Should contain the base SQL model.
    let names: Vec<&str> = result.iter().map(|m| m.name.as_str()).collect();
    assert!(
        names.contains(&"base"),
        "result must contain 'base' SQL model; got: {names:?}"
    );

    // Should contain the Python-generated model.
    assert!(
        names.contains(&"py_generated"),
        "result must contain 'py_generated' Python model; got: {names:?}"
    );

    // Must NOT contain any .gen.sql generator file model.
    for m in &result {
        assert!(
            !m.name.ends_with(".gen"),
            "result must not contain generator files, but found: {}",
            m.name
        );
        assert!(
            !m.path.to_string_lossy().contains(".gen."),
            "result must not contain .gen. paths, but found: {}",
            m.path.display()
        );
    }
}

/// Test 2: within-round evaluation order is ascending by path.
/// Two Python files in different directories should always appear in the same
/// (path-ascending) order regardless of how many times the loop is called.
#[test]
fn combined_loop_within_round_order_is_path_then_name() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project_dir = tmp.path();
    std::fs::create_dir_all(project_dir.join("models").join("aa")).unwrap();
    std::fs::create_dir_all(project_dir.join("models").join("zz")).unwrap();

    setup_sdk(project_dir);
    write_smelt_yml(project_dir, &["models"]);

    // A SQL model is required so that discover_models doesn't fail on empty project.
    std::fs::write(
        project_dir.join("models").join("base.sql"),
        "SELECT 1 AS id",
    )
    .unwrap();

    // Python file at aa/alpha.py — emits alpha_model.
    std::fs::write(
        project_dir.join("models").join("aa").join("alpha.py"),
        r#"
from smelt import model

@model
def alpha_model(project):
    return "SELECT 1 AS alpha_id"
"#,
    )
    .unwrap();

    // Python file at zz/zeta.py — emits zeta_model.
    std::fs::write(
        project_dir.join("models").join("zz").join("zeta.py"),
        r#"
from smelt import model

@model
def zeta_model(project):
    return "SELECT 2 AS zeta_id"
"#,
    )
    .unwrap();

    let config = minimal_config(project_dir, vec!["models".to_string()]);
    let discovery = ModelDiscovery::new(project_dir.to_path_buf(), config.paths.clone());

    // First call.
    let raw_sql1 = discovery.discover_models().expect("discover_models 1");
    let python_files1 = discovery
        .discover_python_files()
        .expect("discover_python_files 1");
    let result1 = run_combined_discovery_loop(
        raw_sql1,
        python_files1,
        project_dir,
        &config,
        config.python.as_deref(),
        None,
    )
    .expect("combined loop call 1 must succeed");

    // Second call.
    let raw_sql2 = discovery.discover_models().expect("discover_models 2");
    let python_files2 = discovery
        .discover_python_files()
        .expect("discover_python_files 2");
    let result2 = run_combined_discovery_loop(
        raw_sql2,
        python_files2,
        project_dir,
        &config,
        config.python.as_deref(),
        None,
    )
    .expect("combined loop call 2 must succeed");

    // Both results must be byte-equal.
    let names1: Vec<&str> = result1.iter().map(|m| m.name.as_str()).collect();
    let names2: Vec<&str> = result2.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(
        names1, names2,
        "two calls must return models in the same order"
    );

    // alpha_model (from aa/alpha.py) must appear before zeta_model (from zz/zeta.py)
    // because "aa/..." < "zz/..." in path order.
    let alpha_pos = result1
        .iter()
        .position(|m| m.name == "alpha_model")
        .expect("alpha_model must be present");
    let zeta_pos = result1
        .iter()
        .position(|m| m.name == "zeta_model")
        .expect("zeta_model must be present");
    assert!(
        alpha_pos < zeta_pos,
        "alpha_model (from aa/) must appear before zeta_model (from zz/) in path order"
    );
}

/// Test 3: calling the loop twice on the same stable workspace should produce
/// byte-identical results (same names, same content, same order).
#[test]
fn combined_loop_byte_equal_stabilisation() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project_dir = tmp.path();
    std::fs::create_dir_all(project_dir.join("models")).unwrap();

    setup_sdk(project_dir);
    write_smelt_yml(project_dir, &["models"]);

    // A SQL model.
    std::fs::write(
        project_dir.join("models").join("orders.sql"),
        "SELECT 1 AS id",
    )
    .unwrap();

    // A stable Python @model.
    std::fs::write(
        project_dir.join("models").join("gen.py"),
        r#"
from smelt import model

@model
def derived_orders(project):
    return "SELECT id * 2 AS doubled_id FROM smelt.ref('orders')"
"#,
    )
    .unwrap();

    let config = minimal_config(project_dir, vec!["models".to_string()]);
    let discovery = ModelDiscovery::new(project_dir.to_path_buf(), config.paths.clone());

    // First invocation.
    let raw1 = discovery.discover_models().expect("discover_models 1");
    let py1 = discovery
        .discover_python_files()
        .expect("discover_python_files 1");
    let result1 = run_combined_discovery_loop(
        raw1,
        py1,
        project_dir,
        &config,
        config.python.as_deref(),
        None,
    )
    .expect("first loop must succeed");

    // Second invocation.
    let raw2 = discovery.discover_models().expect("discover_models 2");
    let py2 = discovery
        .discover_python_files()
        .expect("discover_python_files 2");
    let result2 = run_combined_discovery_loop(
        raw2,
        py2,
        project_dir,
        &config,
        config.python.as_deref(),
        None,
    )
    .expect("second loop must succeed");

    // Collect (name, content) pairs for both results.
    let key1: Vec<(String, String)> = result1
        .iter()
        .map(|m| (m.name.clone(), m.content.clone()))
        .collect();
    let key2: Vec<(String, String)> = result2
        .iter()
        .map(|m| (m.name.clone(), m.content.clone()))
        .collect();

    assert_eq!(
        key1, key2,
        "two invocations of the combined loop must produce byte-equal results"
    );
}
