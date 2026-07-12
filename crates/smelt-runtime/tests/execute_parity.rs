//! Phase 5 parity CI gate: `execute_project` produces reporter-agnostic, deterministic
//! `RunOutcome` values.
//!
//! Both `smelt-cli` and `smelt-ui` call `execute_project` with the same
//! `ExecuteRequest`; this test asserts that two calls with *different* reporters
//! produce identical `RunOutcome` values (same model keys, same row counts, same
//! strategies). Coverage of Mode-B execution classes: full-refresh tables,
//! views, and ephemeral CTE inlining. Additional targeted tests cover
//! test models (filter-out).
//!
//! The test uses a real DuckDB backend against a temporary `.duckdb` file so
//! the parity assertion exercises the actual compile + execute path.
//! Runs under `cargo test --workspace` with no special flags required.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

use smelt_backend::Backend;
use smelt_backend_duckdb::DuckDbBackend;
use smelt_core::config::{Config, Materialization, ModelConfig, Target};
use smelt_core::graph::DependencyGraph;
use smelt_core::ModelDiscovery;
use smelt_runtime::execute::{BackendFactory, BackendFuture};
use smelt_runtime::reporter::RunReporter;
use smelt_runtime::types::{ExecuteRequest, RunOutcome};
use smelt_runtime::{execute_project, NoOpReporter};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

// ── CapturingReporter ─────────────────────────────────────────────────────────

#[derive(Debug, Default)]
struct CapturingReporter {
    events: Mutex<Vec<String>>,
}

impl CapturingReporter {
    fn events_snapshot(&self) -> Vec<String> {
        self.events.lock().unwrap().clone()
    }
}

impl RunReporter for CapturingReporter {
    fn run_started(&self, _run_id: &str, models: &[String], _total_batches: usize) {
        self.events
            .lock()
            .unwrap()
            .push(format!("run_started:{}", models.len()));
    }

    fn model_started(&self, _run_id: &str, model: &str, _idx: usize, _total: usize) {
        self.events
            .lock()
            .unwrap()
            .push(format!("model_started:{}", model));
    }

    fn model_completed(&self, _run_id: &str, model: &str, row_count: usize, _duration: Duration) {
        self.events
            .lock()
            .unwrap()
            .push(format!("model_completed:{}:{}", model, row_count));
    }

    fn run_completed(&self, _run_id: &str, total_rows: usize, _duration: Duration) {
        self.events
            .lock()
            .unwrap()
            .push(format!("run_completed:{}", total_rows));
    }
}

// ── DuckDbBackendFactory ──────────────────────────────────────────────────────

struct DuckDbBackendFactory {
    db_path: std::path::PathBuf,
}

impl BackendFactory for DuckDbBackendFactory {
    fn create<'a>(
        &'a self,
        _target_name: &'a str,
        target_config: &'a smelt_core::config::Target,
        _project_dir: &'a Path,
    ) -> BackendFuture<'a> {
        let path = self.db_path.clone();
        let schema = target_config.schema.clone();
        Box::pin(async move {
            let backend = DuckDbBackend::new(&path, &schema)
                .await
                .map_err(|e| anyhow::anyhow!("DuckDB init failed: {}", e))?;
            Ok(Box::new(backend) as Box<dyn Backend>)
        })
    }
}

// ── Fixture helpers ───────────────────────────────────────────────────────────

/// Write a model file into `project_dir/models/<name>.sql`.
fn write_model(project_dir: &Path, name: &str, content: &str) {
    let path = project_dir.join("models").join(format!("{}.sql", name));
    std::fs::write(path, content).expect("write model file");
}

/// Build the Salsa DB and DependencyGraph by discovering models from the
/// project directory on disk.
fn build_db_and_graph(
    project_dir: &Path,
    config: &Config,
) -> (
    Arc<tokio::sync::Mutex<smelt_db::Database>>,
    Arc<tokio::sync::Mutex<DependencyGraph>>,
) {
    let discovery = ModelDiscovery::new(project_dir.to_path_buf(), config.paths.clone());
    let sql_models = match discovery.discover_models() {
        Ok(models) => models,
        Err(e) => panic!("discover_models failed: {}", e),
    };

    let mut db = smelt_db::Database::default();
    let project = db.set_project_input(project_dir.to_path_buf(), String::new());
    let source_files: Vec<_> = sql_models
        .iter()
        .map(|m| db.set_source_file(m.path.clone(), m.content.clone(), project_dir.to_path_buf()))
        .collect();
    db.set_workspace(source_files, vec![project]);
    if let Ok(cfg) = smelt_core::config::Config::load(project_dir) {
        db.set_active_target(cfg.target.map(|t| std::sync::Arc::from(t.as_str())));
    }

    let graph = DependencyGraph::build(sql_models, None).expect("build graph");

    (
        Arc::new(tokio::sync::Mutex::new(db)),
        Arc::new(tokio::sync::Mutex::new(graph)),
    )
}

fn make_request(target: &str) -> ExecuteRequest {
    ExecuteRequest {
        target: target.to_string(),
        select: vec![],
        exclude: vec![],
        start: None,
        end: None,
        batch_size_days: None,
        per_partition: false,
        full_refresh: false,
        dry_run: false,
        enforce_safety: false,
        allow_column_removal: false,
        allow_full_refresh: false,
        ephemeral_seed_ctes: vec![],
        run_checks: false,
        checks: vec![],
    }
}

/// Assert that two `RunOutcome` values agree on model presence and row counts.
/// Modulo run_id and timestamps (which are per-invocation), the logical content
/// of both runs must be identical: same model names, same row counts per model,
/// same strategy strings.
fn assert_outcomes_equivalent(a: &RunOutcome, b: &RunOutcome, label: &str) {
    let mut a_keys: Vec<&String> = a.models.keys().collect();
    let mut b_keys: Vec<&String> = b.models.keys().collect();
    a_keys.sort();
    b_keys.sort();
    assert_eq!(
        a_keys, b_keys,
        "[{label}] model keys differ: a={a_keys:?} b={b_keys:?}"
    );
    for key in &a_keys {
        let ra = &a.models[*key];
        let rb = &b.models[*key];
        assert_eq!(
            ra.row_count, rb.row_count,
            "[{label}] row_count differs for model '{key}': a={} b={}",
            ra.row_count, rb.row_count
        );
        assert_eq!(
            ra.strategy, rb.strategy,
            "[{label}] strategy differs for model '{key}': a={} b={}",
            ra.strategy, rb.strategy
        );
        assert_eq!(
            ra.partitions_updated, rb.partitions_updated,
            "[{label}] partitions_updated differs for model '{key}'"
        );
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// Main parity test: run the same project with a `CapturingReporter` (CLI-style)
/// and a `NoOpReporter` (UI-style) against two separate fresh DuckDB databases.
/// Asserts the two `RunOutcome` values are identical (modulo run_id/timestamps).
#[tokio::test]
async fn test_cli_ui_manifest_parity() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project_dir = tmp.path();
    std::fs::create_dir_all(project_dir.join("models")).unwrap();

    // Fixture models.
    write_model(project_dir, "base", "SELECT 1 AS id, 'hello' AS label");
    write_model(project_dir, "derived", "SELECT id, label FROM smelt.base");
    write_model(
        project_dir,
        "ephemeral_helper",
        "SELECT id * 2 AS doubled_id, label FROM smelt.base",
    );
    write_model(
        project_dir,
        "with_ephemeral",
        "SELECT doubled_id, label FROM smelt.ephemeral_helper",
    );

    // Two separate DuckDB files — fresh, no pre-existing state.
    let db1_path = project_dir.join("run1.duckdb");
    let db2_path = project_dir.join("run2.duckdb");

    // Write smelt.yml with per-model materializations and pointing at db1.
    let smelt_yml = format!(
        "name: parity_test\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: {db1}\n    schema: main\ndefault_materialization: view\nmodels:\n  derived:\n    materialization: table\n  with_ephemeral:\n    materialization: table\n  ephemeral_helper:\n    materialization: ephemeral\n",
        db1 = db1_path.display()
    );
    std::fs::write(project_dir.join("smelt.yml"), &smelt_yml).unwrap();

    let mut targets = HashMap::new();
    targets.insert(
        "dev".to_string(),
        Target {
            target_type: "duckdb".to_string(),
            database: Some(db1_path.to_string_lossy().into_owned()),
            schema: "main".to_string(),
            connect_url: None,
            catalog: None,
            warehouse: None,
            format: None,
            settings: None,
        },
    );
    let mut models_config = HashMap::new();
    models_config.insert(
        "derived".to_string(),
        ModelConfig {
            materialization: Some(Materialization::Table),
            timeseries: None,
            refresh: None,
            grain: None,
            batched: None,
            tags: vec![],
            target: None,
            format: None,
        },
    );
    models_config.insert(
        "with_ephemeral".to_string(),
        ModelConfig {
            materialization: Some(Materialization::Table),
            timeseries: None,
            refresh: None,
            grain: None,
            batched: None,
            tags: vec![],
            target: None,
            format: None,
        },
    );
    models_config.insert(
        "ephemeral_helper".to_string(),
        ModelConfig {
            materialization: Some(Materialization::Ephemeral),
            timeseries: None,
            refresh: None,
            grain: None,
            batched: None,
            tags: vec![],
            target: None,
            format: None,
        },
    );

    let config = Arc::new(Config {
        name: "parity_test".to_string(),
        version: 1,
        paths: vec!["models".to_string()],
        targets,
        default_materialization: Materialization::View,
        models: models_config,
        python: None,
        target: None,
        state: Default::default(),
        maintenance: None,
    });

    let (db, graph) = build_db_and_graph(project_dir, &config);

    // ── Run 1: CLI-style reporter (CapturingReporter) ─────────────────────────
    let cli_reporter = CapturingReporter::default();
    let outcome_a = execute_project(
        "run-cli".to_string(),
        make_request("dev"),
        Arc::clone(&config),
        Arc::clone(&graph),
        Arc::clone(&db),
        project_dir,
        &DuckDbBackendFactory {
            db_path: db1_path.clone(),
        },
        &cli_reporter,
        CancellationToken::new(),
    )
    .await
    .expect("CLI-style run must succeed");

    // Verify the capturing reporter received expected callbacks.
    let events = cli_reporter.events_snapshot();
    assert!(
        events.iter().any(|e| e.starts_with("run_started:")),
        "run_started must be fired: {events:?}"
    );
    assert!(
        events.iter().any(|e| e.starts_with("run_completed:")),
        "run_completed must be fired: {events:?}"
    );

    // ── Run 2: UI-style reporter (NoOpReporter) ───────────────────────────────
    // Use a fresh config pointing at db2 so both runs start from identical state.
    let mut targets2 = HashMap::new();
    targets2.insert(
        "dev".to_string(),
        Target {
            target_type: "duckdb".to_string(),
            database: Some(db2_path.to_string_lossy().into_owned()),
            schema: "main".to_string(),
            connect_url: None,
            catalog: None,
            warehouse: None,
            format: None,
            settings: None,
        },
    );

    // Rewrite smelt.yml to point at db2 before building the second DB/graph.
    let smelt_yml2 = format!(
        "name: parity_test\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: {db2}\n    schema: main\ndefault_materialization: view\nmodels:\n  derived:\n    materialization: table\n  with_ephemeral:\n    materialization: table\n  ephemeral_helper:\n    materialization: ephemeral\n",
        db2 = db2_path.display()
    );
    std::fs::write(project_dir.join("smelt.yml"), &smelt_yml2).unwrap();

    let config2 = Arc::new(Config {
        name: "parity_test".to_string(),
        version: 1,
        paths: vec!["models".to_string()],
        targets: targets2,
        default_materialization: Materialization::View,
        models: config.models.clone(),
        python: None,
        target: None,
        state: Default::default(),
        maintenance: None,
    });
    let (db2, graph2) = build_db_and_graph(project_dir, &config2);

    let outcome_b = execute_project(
        "run-ui".to_string(),
        make_request("dev"),
        Arc::clone(&config2),
        Arc::clone(&graph2),
        Arc::clone(&db2),
        project_dir,
        &DuckDbBackendFactory {
            db_path: db2_path.clone(),
        },
        &NoOpReporter,
        CancellationToken::new(),
    )
    .await
    .expect("UI-style run must succeed");

    // ── Assert parity ─────────────────────────────────────────────────────────
    assert_outcomes_equivalent(&outcome_a, &outcome_b, "cli-vs-ui");
    assert_eq!(
        outcome_a.total_rows, outcome_b.total_rows,
        "total_rows must match between CLI and UI runs"
    );
}

/// Verify that test models (`smelt.test` declarations) are filtered out of execution
/// and do not appear in the `RunOutcome.models` map.
#[tokio::test]
async fn test_cli_ui_manifest_parity_with_test_models() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project_dir = tmp.path();
    std::fs::create_dir_all(project_dir.join("models")).unwrap();

    write_model(project_dir, "base", "SELECT 42 AS val");

    // Write a test model using `smelt.test` syntax — must be filtered from execution.
    write_model(
        project_dir,
        "test_base",
        "smelt.test check_val AS (\n    SELECT val FROM smelt.base WHERE val = 42\n)\nPASSING base AS ({val: 42})\nEXPECT ({val: 42})\n",
    );

    let db1_path = project_dir.join("run1.duckdb");
    let db2_path = project_dir.join("run2.duckdb");

    // Config with default view materialization (no special model overrides).
    let smelt_yml = format!(
        "name: parity_test_models\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: {db1}\n    schema: main\ndefault_materialization: view\n",
        db1 = db1_path.display()
    );
    std::fs::write(project_dir.join("smelt.yml"), &smelt_yml).unwrap();

    let config = Arc::new(smelt_core::config::Config::load(project_dir).expect("load config"));
    let (db, graph) = build_db_and_graph(project_dir, &config);

    let outcome_a = execute_project(
        "run-a".to_string(),
        make_request("dev"),
        Arc::clone(&config),
        Arc::clone(&graph),
        Arc::clone(&db),
        project_dir,
        &DuckDbBackendFactory {
            db_path: db1_path.clone(),
        },
        &NoOpReporter,
        CancellationToken::new(),
    )
    .await
    .expect("run must succeed");

    // Test model must be absent from the outcome.
    assert!(
        !outcome_a.models.contains_key("test_base"),
        "test model must not appear in RunOutcome; got keys: {:?}",
        outcome_a.models.keys().collect::<Vec<_>>()
    );
    // Regular model must be present.
    assert!(
        outcome_a.models.contains_key("base"),
        "base model must appear in RunOutcome"
    );

    // UI run: same result.
    let smelt_yml2 = format!(
        "name: parity_test_models\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: {db2}\n    schema: main\ndefault_materialization: view\n",
        db2 = db2_path.display()
    );
    std::fs::write(project_dir.join("smelt.yml"), &smelt_yml2).unwrap();
    let config2 = Arc::new(smelt_core::config::Config::load(project_dir).expect("load config 2"));
    let (db2, graph2) = build_db_and_graph(project_dir, &config2);

    let outcome_b = execute_project(
        "run-b".to_string(),
        make_request("dev"),
        Arc::clone(&config2),
        Arc::clone(&graph2),
        Arc::clone(&db2),
        project_dir,
        &DuckDbBackendFactory {
            db_path: db2_path.clone(),
        },
        &NoOpReporter,
        CancellationToken::new(),
    )
    .await
    .expect("run 2 must succeed");

    assert_outcomes_equivalent(&outcome_a, &outcome_b, "test-model-filter");
}

/// P1 / BUG-077 gate: the UI/`execute_project` path now surfaces Python-derived
/// models.  Before this migration, Python model discovery only lived in `smelt-cli`
/// so the UI path omitted Python models entirely.
///
/// This test calls `smelt_runtime::discover_python_models` (the moved entry point)
/// to build the Python `ModelFile` set, includes those models in the
/// `DependencyGraph` passed to `execute_project`, and asserts the Python model
/// appears in the `RunOutcome` — i.e., the shared runtime path handles it.
#[tokio::test]
async fn ui_path_runs_python_models() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project_dir = tmp.path();
    std::fs::create_dir_all(project_dir.join("models")).unwrap();

    // Create Python SDK inside the temp project
    let sdk_dir = project_dir.join("python").join("smelt");
    std::fs::create_dir_all(&sdk_dir).unwrap();
    let repo_sdk = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("python")
        .join("smelt");
    for entry in std::fs::read_dir(&repo_sdk).unwrap() {
        let entry = entry.unwrap();
        if entry.path().is_file() {
            std::fs::copy(entry.path(), sdk_dir.join(entry.file_name())).unwrap();
        }
    }

    // A minimal SQL model (required because `discover_models` errors on empty projects).
    // The Python model will also be in the graph alongside it.
    write_model(project_dir, "sql_base", "SELECT 1 AS anchor_id");

    // Python model that returns a simple SQL string
    std::fs::write(
        project_dir.join("models").join("gen.py"),
        r#"
from smelt import model

@model
def py_model(project):
    return "SELECT 1 as id, 'from_python' as source"
"#,
    )
    .unwrap();

    let db_path = project_dir.join("run.duckdb");
    let smelt_yml = format!(
        "name: ui_python_test\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: {db}\n    schema: main\ndefault_materialization: view\n",
        db = db_path.display()
    );
    std::fs::write(project_dir.join("smelt.yml"), &smelt_yml).unwrap();

    let config = Arc::new(smelt_core::config::Config::load(project_dir).expect("load config"));

    // Discover SQL models
    let discovery = ModelDiscovery::new(project_dir.to_path_buf(), config.paths.clone());
    let sql_models = discovery.discover_models().expect("discover_models");
    let python_files = discovery
        .discover_python_files()
        .expect("discover_python_files");

    // Call discover_python_models via the runtime entry point (not smelt-cli)
    let python_models = smelt_runtime::discover_python_models(
        &python_files,
        &sql_models,
        &config,
        project_dir,
        config.python.as_deref(),
    )
    .expect("discover_python_models must succeed");

    assert_eq!(
        python_models.len(),
        1,
        "runtime discovery must find the Python model; got: {:?}",
        python_models.iter().map(|m| &m.name).collect::<Vec<_>>()
    );
    assert_eq!(python_models[0].name, "py_model");

    // Build the Salsa DB + DependencyGraph including the Python models
    let mut all_models = sql_models;
    all_models.extend(python_models);

    let mut db = smelt_db::Database::default();
    let project = db.set_project_input(project_dir.to_path_buf(), String::new());
    let source_files: Vec<_> = all_models
        .iter()
        .map(|m| db.set_source_file(m.path.clone(), m.content.clone(), project_dir.to_path_buf()))
        .collect();
    db.set_workspace(source_files, vec![project]);

    let graph = DependencyGraph::build(all_models, None).expect("build graph");

    let db_arc = Arc::new(tokio::sync::Mutex::new(db));
    let graph_arc = Arc::new(tokio::sync::Mutex::new(graph));

    // Execute via the shared `execute_project` (the UI-style path)
    let outcome = execute_project(
        "run-ui-python".to_string(),
        make_request("dev"),
        Arc::clone(&config),
        Arc::clone(&graph_arc),
        Arc::clone(&db_arc),
        project_dir,
        &DuckDbBackendFactory {
            db_path: db_path.clone(),
        },
        &NoOpReporter,
        CancellationToken::new(),
    )
    .await
    .expect("execute_project must succeed for Python model");

    // Assert: Python model appears in the RunOutcome (BUG-077 fix)
    assert!(
        outcome.models.contains_key("py_model")
            || outcome
                .models
                .keys()
                .any(|k| k.ends_with("py_model") || k.contains("py_model")),
        "Python model 'py_model' must appear in RunOutcome.models; got: {:?}",
        outcome.models.keys().collect::<Vec<_>>()
    );
}

/// Verify that ephemeral models are inlined as CTEs and do NOT appear as
/// top-level entries in `RunOutcome.models`.
#[tokio::test]
async fn test_cli_ui_manifest_parity_with_ephemerals() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project_dir = tmp.path();
    std::fs::create_dir_all(project_dir.join("models")).unwrap();

    write_model(project_dir, "src", "SELECT 10 AS x, 'foo' AS tag");
    write_model(
        project_dir,
        "eph",
        "---\nmaterialization: ephemeral\n---\nSELECT x * 3 AS tripled, tag FROM smelt.src",
    );
    write_model(
        project_dir,
        "consumer",
        "SELECT tripled, tag FROM smelt.eph",
    );

    let db1_path = project_dir.join("run1.duckdb");
    let db2_path = project_dir.join("run2.duckdb");

    let smelt_yml = format!(
        "name: eph_parity\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: {db1}\n    schema: main\ndefault_materialization: view\nmodels:\n  consumer:\n    materialization: table\n",
        db1 = db1_path.display()
    );
    std::fs::write(project_dir.join("smelt.yml"), &smelt_yml).unwrap();
    let config = Arc::new(smelt_core::config::Config::load(project_dir).expect("load config"));
    let (db, graph) = build_db_and_graph(project_dir, &config);

    let outcome_a = execute_project(
        "run-a".to_string(),
        make_request("dev"),
        Arc::clone(&config),
        Arc::clone(&graph),
        Arc::clone(&db),
        project_dir,
        &DuckDbBackendFactory {
            db_path: db1_path.clone(),
        },
        &CapturingReporter::default(),
        CancellationToken::new(),
    )
    .await
    .expect("run must succeed");

    // Ephemeral model must NOT appear in the outcome — it's inlined as CTE.
    assert!(
        !outcome_a.models.contains_key("eph"),
        "ephemeral model 'eph' must not appear in RunOutcome; got: {:?}",
        outcome_a.models.keys().collect::<Vec<_>>()
    );
    // Consumer model that depends on the ephemeral must be present.
    assert!(
        outcome_a.models.contains_key("consumer"),
        "consumer model must appear in RunOutcome"
    );

    // UI run (db2) must match.
    let smelt_yml2 = format!(
        "name: eph_parity\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: {db2}\n    schema: main\ndefault_materialization: view\nmodels:\n  consumer:\n    materialization: table\n",
        db2 = db2_path.display()
    );
    std::fs::write(project_dir.join("smelt.yml"), &smelt_yml2).unwrap();
    let config2 = Arc::new(smelt_core::config::Config::load(project_dir).expect("load config 2"));
    let (db2, graph2) = build_db_and_graph(project_dir, &config2);

    let outcome_b = execute_project(
        "run-b".to_string(),
        make_request("dev"),
        Arc::clone(&config2),
        Arc::clone(&graph2),
        Arc::clone(&db2),
        project_dir,
        &DuckDbBackendFactory {
            db_path: db2_path.clone(),
        },
        &NoOpReporter,
        CancellationToken::new(),
    )
    .await
    .expect("run 2 must succeed");

    assert_outcomes_equivalent(&outcome_a, &outcome_b, "ephemeral-inlining");
}
