//! P4 of docs/plans/20260613-w5b-combined-eval.md — bidirectional cross-type
//! integration tests for the combined SQL-generator + Python fixed-point loop.
//!
//! Three tests:
//!   1. `python_consumes_sql_generator_emission_e2e` — full `smelt run` of a
//!      workspace where a Python `@model` generates SQL referencing a SQL
//!      generator's emitted model. Both must execute successfully on DuckDB.
//!   2. `sql_generator_consumes_python_emission_e2e` — full `smelt run` of a
//!      workspace where a SQL generator's emitted model body references a
//!      Python-emitted model. Both must execute successfully on DuckDB.
//!   3. `combined_loop_non_convergence_errors` — `run_combined_discovery_loop`
//!      must return `Err` with a convergence-failure message when a Python model
//!      produces different SQL on every round (filesystem counter trick).

use smelt_cli::{run_combined_discovery_loop, Config, ModelDiscovery};
use smelt_core::config::Materialization;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

// ── SDK helper ────────────────────────────────────────────────────────────────

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

/// Write a minimal `smelt.yml` for a project at `project_dir` with a DuckDB target.
fn write_smelt_yml_duckdb(project_dir: &Path, name: &str) {
    let db_path = project_dir.join("target").join("dev.duckdb");
    let yml = format!(
        "name: {name}\n\
         version: 1\n\
         paths:\n  - models\n\
         targets:\n  dev:\n    type: duckdb\n    database: {db}\n    schema: main\n\
         default_materialization: view\n",
        name = name,
        db = db_path.to_string_lossy()
    );
    std::fs::write(project_dir.join("smelt.yml"), yml).unwrap();
}

/// Build a minimal `Config` for discovery tests (no DuckDB target needed).
fn minimal_config(paths: Vec<String>) -> Config {
    Config {
        name: "test".into(),
        version: 1,
        paths,
        targets: HashMap::new(),
        target: None,
        default_materialization: Materialization::View,
        models: HashMap::new(),
        python: None,
        state: Default::default(),
        maintenance: None,
        probes: Default::default(),
    }
}

fn smelt_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_smelt"))
}

// ── Test 1: Python → SQL generator (full build) ───────────────────────────────

/// Full `smelt run` where a Python `@model` generates SQL that references a
/// SQL generator's emitted model.  Both families contribute a model; the
/// combined loop converges and the build succeeds on DuckDB.
///
/// Setup:
///   models/gen.gen.sql  → emits `gen.gen_product` with `SELECT 1 AS product_id`
///   models/consume.py   → emits `py_consumer` whose SQL is
///                         `SELECT product_id FROM smelt.gen.gen_product`
#[cfg(feature = "duckdb")]
#[test]
fn python_consumes_sql_generator_emission_e2e() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project_dir = tmp.path();
    std::fs::create_dir_all(project_dir.join("models")).unwrap();
    std::fs::create_dir_all(project_dir.join("target")).unwrap();

    setup_sdk(project_dir);
    write_smelt_yml_duckdb(project_dir, "py_consumes_gen");

    // SQL generator at models/gen.gen.sql emits gen.gen_product.
    std::fs::write(
        project_dir.join("models").join("gen.gen.sql"),
        "---\ngenerates: models\n---\n[ModelDef { name: 'gen_product', body: SELECT 1 AS product_id }]",
    )
    .unwrap();

    // Python model: generates SQL referencing the SQL generator's emission.
    std::fs::write(
        project_dir.join("models").join("consume.py"),
        r#"
from smelt import model

@model
def py_consumer(project):
    return "SELECT product_id FROM smelt.gen.gen_product"
"#,
    )
    .unwrap();

    let output = std::process::Command::new(smelt_bin())
        .args(["run", "--project-dir", project_dir.to_str().unwrap()])
        .env_remove("RUST_LOG")
        .output()
        .expect("smelt run failed to spawn");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "smelt run must succeed for Python-consumes-SQL-generator workspace.\n\
         stdout: {stdout}\nstderr: {stderr}"
    );
}

// ── Test 2: SQL generator → Python (full build) ───────────────────────────────

/// Full `smelt run` where a SQL generator's emitted model body references a
/// Python `@model` emission.  The combined loop converges (Python emits first
/// in round 0; the SQL generator's Salsa DB sees it in round 1 when prev_python
/// is fed back) and the build succeeds on DuckDB.
///
/// Setup:
///   models/source.py    → emits `py_source` with `SELECT 42 AS src_val`
///   models/gen.gen.sql  → emits `gen.derived` with
///                         `SELECT src_val * 2 AS doubled FROM smelt.py_source`
#[cfg(feature = "duckdb")]
#[test]
fn sql_generator_consumes_python_emission_e2e() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project_dir = tmp.path();
    std::fs::create_dir_all(project_dir.join("models")).unwrap();
    std::fs::create_dir_all(project_dir.join("target")).unwrap();

    setup_sdk(project_dir);
    write_smelt_yml_duckdb(project_dir, "gen_consumes_py");

    // Python model: emits py_source.  The file is named py_source.py so that
    // compute_address_segments produces a single-segment address ("py_source"),
    // matching the smelt.py_source ref in the SQL generator body below.
    std::fs::write(
        project_dir.join("models").join("py_source.py"),
        r#"
from smelt import model

@model
def py_source(project):
    return "SELECT 42 AS src_val"
"#,
    )
    .unwrap();

    // SQL generator: emits gen.derived whose body references smelt.py_source.
    std::fs::write(
        project_dir.join("models").join("gen.gen.sql"),
        "---\ngenerates: models\n---\n[ModelDef { name: 'derived', body: SELECT src_val * 2 AS doubled FROM smelt.py_source }]",
    )
    .unwrap();

    let output = std::process::Command::new(smelt_bin())
        .args(["run", "--project-dir", project_dir.to_str().unwrap()])
        .env_remove("RUST_LOG")
        .output()
        .expect("smelt run failed to spawn");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "smelt run must succeed for SQL-generator-consumes-Python workspace.\n\
         stdout: {stdout}\nstderr: {stderr}"
    );
}

// ── Test 3: non-convergence → error ───────────────────────────────────────────

/// `run_combined_discovery_loop` must return `Err` when a Python model produces
/// different SQL on every round and the loop cannot reach byte-equal
/// stabilisation within the 5-round bound.
///
/// The "oscillating" Python model uses a filesystem counter: each time the
/// subprocess is invoked it increments the counter file and embeds the new value
/// in its SQL output. Because the SQL content changes every round the combined
/// loop never sees two identical snapshots and exhausts MAX_ROUNDS.
#[test]
fn combined_loop_non_convergence_errors() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project_dir = tmp.path();
    std::fs::create_dir_all(project_dir.join("models")).unwrap();

    setup_sdk(project_dir);

    // Minimal smelt.yml (no DuckDB target needed for discovery).
    std::fs::write(
        project_dir.join("smelt.yml"),
        "name: nonconv_test\nversion: 1\npaths:\n  - models\ndefault_materialization: view\n",
    )
    .unwrap();

    // Anchor SQL model so discover_models() finds a non-empty workspace.
    std::fs::write(
        project_dir.join("models").join("anchor.sql"),
        "SELECT 1 AS id",
    )
    .unwrap();

    // Counter file lives inside the project_dir temp tree so it's cleaned up
    // with the tempdir.
    let counter_path = project_dir.join("round_counter.txt");
    let counter_path_str = counter_path.to_string_lossy().replace('\\', "/");

    // Python model that increments the counter each invocation and embeds the
    // count in its SQL body — guarantees different content on every round.
    let python_content = format!(
        r#"
from smelt import model

COUNTER_FILE = "{counter}"

@model
def unstable(project):
    try:
        with open(COUNTER_FILE) as f:
            count = int(f.read().strip()) + 1
    except Exception:
        count = 1
    with open(COUNTER_FILE, 'w') as f:
        f.write(str(count))
    return 'SELECT ' + str(count) + ' AS iteration_count'
"#,
        counter = counter_path_str
    );
    std::fs::write(
        project_dir.join("models").join("oscillate.py"),
        &python_content,
    )
    .unwrap();

    let config = minimal_config(vec!["models".to_string()]);
    let discovery = ModelDiscovery::new(project_dir.to_path_buf(), config.paths.clone());

    let raw_sql = discovery.discover_models().expect("discover_models");
    let python_files = discovery
        .discover_python_files()
        .expect("discover_python_files");

    // The combined loop must fail after MAX_ROUNDS (5) since the model set never
    // stabilises (SQL content changes every round).
    let result = run_combined_discovery_loop(
        raw_sql,
        python_files,
        project_dir,
        &config,
        config.python.as_deref(),
        None,
    );

    assert!(
        result.is_err(),
        "combined loop must return Err when model set never stabilises; got Ok"
    );
    let err = result.unwrap_err().to_string();
    assert!(
        err.to_string().to_lowercase().contains("converge")
            || err.to_string().to_lowercase().contains("circular"),
        "error message must mention convergence or circular dependency; got: {err}"
    );
}
