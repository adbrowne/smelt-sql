//! Run-path reporting for external steps (`docs/specs/run_state.md` §"Run
//! manifest"; `docs/specs/sources.md` §"Externally-produced sources
//! (black-box steps)"). Plan:
//! `docs/outcomes/20260906-external-dag-steps/phases/05-plan.md`.
//!
//! Exercises the real `execute_project` pipeline against a real DuckDB
//! backend with a recording `RunReporter`, over the same fixture shape as
//! `tests/external_step_invocation.rs`.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use smelt_backend::Backend;
use smelt_backend_duckdb::DuckDbBackend;
use smelt_core::config::Config;
use smelt_core::graph::DependencyGraph;
use smelt_core::ModelDiscovery;
use smelt_runtime::execute::{execute_project, BackendFactory, BackendFuture};
use smelt_runtime::reporter::RunReporter;
use smelt_runtime::types::ExecuteRequest;
use tokio_util::sync::CancellationToken;

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

/// Recording `RunReporter`: captures every external-step event fired, in
/// order, for assertion.
#[derive(Debug, Clone, PartialEq)]
enum StepEvent {
    Started { step: String, argv: Vec<String> },
    Completed { step: String },
    Failed { step: String, exit_code: i32 },
}

#[derive(Default)]
struct RecordingReporter {
    events: Mutex<Vec<StepEvent>>,
}

impl RecordingReporter {
    fn events(&self) -> Vec<StepEvent> {
        self.events.lock().unwrap().clone()
    }
}

impl RunReporter for RecordingReporter {
    fn external_step_started(&self, _run_id: &str, step: &str, argv: &[String]) {
        self.events.lock().unwrap().push(StepEvent::Started {
            step: step.to_string(),
            argv: argv.to_vec(),
        });
    }

    fn external_step_completed(&self, _run_id: &str, step: &str, _duration: Duration) {
        self.events.lock().unwrap().push(StepEvent::Completed {
            step: step.to_string(),
        });
    }

    fn external_step_failed(&self, _run_id: &str, step: &str, exit_code: i32, _error: &str) {
        self.events.lock().unwrap().push(StepEvent::Failed {
            step: step.to_string(),
            exit_code,
        });
    }
}

fn build_db_and_graph(
    project_dir: &Path,
    config: &Config,
) -> (
    Arc<tokio::sync::Mutex<smelt_db::Database>>,
    Arc<tokio::sync::Mutex<DependencyGraph>>,
) {
    let discovery = ModelDiscovery::new(project_dir.to_path_buf(), config.paths.clone());
    let sql_models = discovery.discover_models().expect("discover_models");

    let mut db = smelt_db::Database::default();
    let project = db.set_project_input(project_dir.to_path_buf(), String::new());
    let source_files: Vec<_> = sql_models
        .iter()
        .map(|m| db.set_source_file(m.path.clone(), m.content.clone(), project_dir.to_path_buf()))
        .collect();
    db.set_workspace(source_files, vec![project]);
    db.set_active_target(Some(std::sync::Arc::from("dev")));

    let graph = DependencyGraph::build(sql_models, None).expect("build graph");

    (
        Arc::new(tokio::sync::Mutex::new(db)),
        Arc::new(tokio::sync::Mutex::new(graph)),
    )
}

fn make_request(select: Vec<String>, dry_run: bool, invoke_external_steps: bool) -> ExecuteRequest {
    ExecuteRequest {
        target: "dev".to_string(),
        select,
        exclude: vec![],
        start: None,
        end: None,
        batch_size_days: None,
        per_partition: false,
        full_refresh: false,
        rebuild: false,
        dry_run,
        enforce_safety: false,
        allow_column_removal: false,
        allow_full_refresh: false,
        ephemeral_seed_ctes: vec![],
        run_checks: false,
        checks: vec![],
        jobs: None,
        retry_max: None,
        retry_backoff_ms: None,
        resume: false,
        technique_overrides: vec![],
        invoke_external_steps,
    }
}

fn duckdb_cli_available() -> bool {
    std::process::Command::new("duckdb")
        .arg("-version")
        .output()
        .is_ok()
}

/// Scaffold a project with one source (`sources.raw_events`), one external
/// step declared to produce it (`command:` is `bash loader.sh`, written
/// separately by each test), and one consumer model reading the source.
fn scaffold(project_dir: &Path, db_path: &Path) -> Arc<Config> {
    std::fs::create_dir_all(project_dir.join("models").join("sources")).unwrap();
    std::fs::write(
        project_dir.join("smelt.yml"),
        format!(
            "name: test\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: {}\n    schema: main\nstate:\n  mode: intervals\n",
            db_path.display()
        ),
    )
    .unwrap();
    std::fs::write(
        project_dir
            .join("models")
            .join("sources")
            .join("raw_events.yml"),
        "columns:\n  - name: id\n    type: INTEGER\n",
    )
    .unwrap();
    std::fs::write(
        project_dir.join("models").join("loader.yml"),
        "external_step:\n  produces:\n    - smelt.sources.raw_events\n  command: [\"bash\", \"loader.sh\"]\n",
    )
    .unwrap();
    std::fs::write(
        project_dir.join("models").join("consumer.sql"),
        "---\nmaterialization: table\n---\nSELECT id FROM smelt.sources.raw_events\n",
    )
    .unwrap();

    Arc::new(Config::load(project_dir).expect("load smelt.yml"))
}

fn write_loader_script(project_dir: &Path, body: &str) {
    std::fs::write(project_dir.join("loader.sh"), body).expect("write loader.sh");
}

/// A succeeding step fires `external_step_started` (with the resolved argv)
/// then `external_step_completed`.
#[tokio::test]
async fn reporter_sees_started_then_completed() {
    if !duckdb_cli_available() {
        eprintln!("duckdb CLI not on PATH — skipping reporter_sees_started_then_completed");
        return;
    }
    let tmp = tempfile::tempdir().expect("tempdir");
    let project_dir = tmp.path().join("proj");
    std::fs::create_dir_all(&project_dir).unwrap();
    let db_path = tmp.path().join("run.duckdb");
    let config = scaffold(&project_dir, &db_path);
    write_loader_script(
        &project_dir,
        &format!(
            "#!/bin/bash\nset -e\nduckdb '{}' \"CREATE TABLE main.sources_raw_events AS SELECT 1 AS id\"\n",
            db_path.display()
        ),
    );

    let (db, graph) = build_db_and_graph(&project_dir, &config);
    let reporter = RecordingReporter::default();
    execute_project(
        "run-1".to_string(),
        make_request(vec!["consumer".to_string()], false, true),
        Arc::clone(&config),
        graph,
        db,
        &project_dir,
        &DuckDbBackendFactory {
            db_path: db_path.clone(),
        },
        &reporter,
        CancellationToken::new(),
    )
    .await
    .expect("run must succeed");

    let events = reporter.events();
    assert_eq!(events.len(), 2, "got: {:?}", events);
    match &events[0] {
        StepEvent::Started { step, argv } => {
            assert_eq!(step, "loader");
            assert_eq!(argv, &vec!["bash".to_string(), "loader.sh".to_string()]);
        }
        other => panic!("expected Started first, got: {:?}", other),
    }
    match &events[1] {
        StepEvent::Completed { step } => assert_eq!(step, "loader"),
        other => panic!("expected Completed second, got: {:?}", other),
    }
}

/// A step exiting `3` fires `external_step_failed` with `exit_code == 3` and
/// no `external_step_completed`.
#[tokio::test]
async fn reporter_sees_failed_with_exit_code() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project_dir = tmp.path().join("proj");
    std::fs::create_dir_all(&project_dir).unwrap();
    let db_path = tmp.path().join("run.duckdb");
    let config = scaffold(&project_dir, &db_path);
    write_loader_script(&project_dir, "#!/bin/bash\nexit 3\n");

    let (db, graph) = build_db_and_graph(&project_dir, &config);
    let reporter = RecordingReporter::default();
    let err = execute_project(
        "run-1".to_string(),
        make_request(vec!["consumer".to_string()], false, true),
        Arc::clone(&config),
        graph,
        db,
        &project_dir,
        &DuckDbBackendFactory {
            db_path: db_path.clone(),
        },
        &reporter,
        CancellationToken::new(),
    )
    .await
    .expect_err("a nonzero exit must fail the run");
    assert!(err.to_string().contains("ExternalStepFailed"));

    let events = reporter.events();
    assert_eq!(events.len(), 2, "got: {:?}", events);
    assert!(matches!(&events[0], StepEvent::Started { step, .. } if step == "loader"));
    match &events[1] {
        StepEvent::Failed { step, exit_code } => {
            assert_eq!(step, "loader");
            assert_eq!(*exit_code, 3);
        }
        other => panic!("expected Failed second, got: {:?}", other),
    }
}

/// A dry run refuses (`ExternalStepNotInvocable`) with zero step events —
/// nothing was spawned, so nothing may be reported as started.
#[tokio::test]
async fn refusal_fires_no_step_events() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project_dir = tmp.path().join("proj");
    std::fs::create_dir_all(&project_dir).unwrap();
    let db_path = tmp.path().join("run.duckdb");
    let config = scaffold(&project_dir, &db_path);
    write_loader_script(&project_dir, "#!/bin/bash\nexit 0\n");

    let (db, graph) = build_db_and_graph(&project_dir, &config);
    let reporter = RecordingReporter::default();
    let err = execute_project(
        "run-1".to_string(),
        make_request(vec!["consumer".to_string()], true, true),
        Arc::clone(&config),
        graph,
        db,
        &project_dir,
        &DuckDbBackendFactory {
            db_path: db_path.clone(),
        },
        &reporter,
        CancellationToken::new(),
    )
    .await
    .expect_err("a dry run reaching a step must refuse");
    assert!(err.to_string().contains("ExternalStepNotInvocable"));
    assert!(reporter.events().is_empty(), "got: {:?}", reporter.events());
}

/// A run selecting a step-free model fires no step events.
#[tokio::test]
async fn no_step_events_when_selection_requires_none() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project_dir = tmp.path().join("proj");
    std::fs::create_dir_all(project_dir.join("models").join("sources")).unwrap();
    let db_path = tmp.path().join("run.duckdb");
    std::fs::write(
        project_dir.join("smelt.yml"),
        format!(
            "name: test\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: {}\n    schema: main\n",
            db_path.display()
        ),
    )
    .unwrap();
    std::fs::write(
        project_dir
            .join("models")
            .join("sources")
            .join("raw_events.yml"),
        "columns:\n  - name: id\n    type: INTEGER\n",
    )
    .unwrap();
    std::fs::write(
        project_dir.join("models").join("loader.yml"),
        "external_step:\n  produces:\n    - smelt.sources.raw_events\n  command: [\"bash\", \"loader.sh\"]\n",
    )
    .unwrap();
    std::fs::write(
        project_dir.join("models").join("standalone.sql"),
        "---\nmaterialization: table\n---\nSELECT 1 AS x\n",
    )
    .unwrap();
    write_loader_script(&project_dir, "#!/bin/bash\ntouch invoked.marker\n");
    let config = Arc::new(Config::load(&project_dir).expect("load smelt.yml"));

    let (db, graph) = build_db_and_graph(&project_dir, &config);
    let reporter = RecordingReporter::default();
    execute_project(
        "run-1".to_string(),
        make_request(vec!["standalone".to_string()], false, true),
        Arc::clone(&config),
        graph,
        db,
        &project_dir,
        &DuckDbBackendFactory {
            db_path: db_path.clone(),
        },
        &reporter,
        CancellationToken::new(),
    )
    .await
    .expect("run over an unrelated model must succeed");

    assert!(reporter.events().is_empty(), "got: {:?}", reporter.events());
}

/// The run manifest's `external_steps` carries the address, the resolved
/// argv, `produces`, and `outcome: success` after a successful run whose
/// selection reaches a step.
#[tokio::test]
async fn manifest_records_invoked_step() {
    if !duckdb_cli_available() {
        eprintln!("duckdb CLI not on PATH — skipping manifest_records_invoked_step");
        return;
    }
    let tmp = tempfile::tempdir().expect("tempdir");
    let project_dir = tmp.path().join("proj");
    std::fs::create_dir_all(&project_dir).unwrap();
    let db_path = tmp.path().join("run.duckdb");
    let config = scaffold(&project_dir, &db_path);
    write_loader_script(
        &project_dir,
        &format!(
            "#!/bin/bash\nset -e\nduckdb '{}' \"CREATE TABLE main.sources_raw_events AS SELECT 1 AS id\"\n",
            db_path.display()
        ),
    );

    let (db, graph) = build_db_and_graph(&project_dir, &config);
    execute_project(
        "run-manifest-1".to_string(),
        make_request(vec!["consumer".to_string()], false, true),
        Arc::clone(&config),
        graph,
        db,
        &project_dir,
        &DuckDbBackendFactory {
            db_path: db_path.clone(),
        },
        &smelt_runtime::NoOpReporter,
        CancellationToken::new(),
    )
    .await
    .expect("run must succeed");

    let file_store = smelt_state::file_store::FileStore::new(&project_dir, "dev");
    let manifest = file_store
        .load_run("run-manifest-1")
        .expect("load manifest")
        .expect("manifest must exist");

    assert_eq!(
        manifest.external_steps.len(),
        1,
        "got: {:?}",
        manifest.external_steps
    );
    let record = manifest
        .external_steps
        .get("loader")
        .expect("loader step recorded");
    assert_eq!(
        record.command,
        vec!["bash".to_string(), "loader.sh".to_string()]
    );
    assert_eq!(
        record.produces,
        vec!["smelt.sources.raw_events".to_string()]
    );
    assert_eq!(record.outcome, smelt_state::RunOutcomeKind::Success);
}

/// The `external_steps` field is omitted (not an empty object) for a
/// step-free run.
#[tokio::test]
async fn manifest_has_no_external_steps_key_when_none_ran() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project_dir = tmp.path().join("proj");
    std::fs::create_dir_all(project_dir.join("models").join("sources")).unwrap();
    let db_path = tmp.path().join("run.duckdb");
    std::fs::write(
        project_dir.join("smelt.yml"),
        format!(
            "name: test\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: {}\n    schema: main\nstate:\n  mode: intervals\n",
            db_path.display()
        ),
    )
    .unwrap();
    std::fs::write(
        project_dir.join("models").join("standalone.sql"),
        "---\nmaterialization: table\n---\nSELECT 1 AS x\n",
    )
    .unwrap();
    let config = Arc::new(Config::load(&project_dir).expect("load smelt.yml"));

    let (db, graph) = build_db_and_graph(&project_dir, &config);
    execute_project(
        "run-no-steps-1".to_string(),
        make_request(vec!["standalone".to_string()], false, true),
        Arc::clone(&config),
        graph,
        db,
        &project_dir,
        &DuckDbBackendFactory {
            db_path: db_path.clone(),
        },
        &smelt_runtime::NoOpReporter,
        CancellationToken::new(),
    )
    .await
    .expect("run must succeed");

    let file_store = smelt_state::file_store::FileStore::new(&project_dir, "dev");
    let raw = std::fs::read_to_string(
        project_dir
            .join(".smelt")
            .join("targets")
            .join("dev")
            .join("runs")
            .join("run-no-steps-1.json"),
    )
    .expect("read manifest file");
    assert!(
        !raw.contains("external_steps"),
        "manifest JSON must omit external_steps when no step ran, got: {raw}"
    );

    let manifest = file_store
        .load_run("run-no-steps-1")
        .expect("load manifest")
        .expect("manifest must exist");
    assert!(manifest.external_steps.is_empty());
}
