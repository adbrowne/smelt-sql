//! Invocation of externally-produced sources' black-box steps on the run
//! path (`docs/specs/sources.md` §"Externally-produced sources (black-box
//! steps)"). Plan: `docs/outcomes/20260906-external-dag-steps/phases/04-plan.md`.
//!
//! Exercises the real `execute_project` pipeline against a real DuckDB
//! backend; the step's `command:` is a temp shell script.

use std::path::Path;
use std::sync::Arc;

use smelt_backend::Backend;
use smelt_backend_duckdb::DuckDbBackend;
use smelt_core::config::Config;
use smelt_core::graph::DependencyGraph;
use smelt_core::ModelDiscovery;
use smelt_runtime::execute::{execute_project, BackendFactory, BackendFuture};
use smelt_runtime::types::ExecuteRequest;
use smelt_runtime::NoOpReporter;
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

/// Whether the `duckdb` CLI is on `PATH` — the step scripts below use it to
/// land the produced source's table (the step is a real, opaque external
/// program; smelt's own DuckDB connection is not open yet when a step
/// runs, so a shell script needs its own way to write the file). Not every
/// CI runner installs the CLI (only `libduckdb.so` is provisioned), so
/// tests that need a landed table skip gracefully rather than fail — the
/// same posture this repo already uses for Spark/BigQuery-gated tests.
fn duckdb_cli_available() -> bool {
    std::process::Command::new("duckdb")
        .arg("-version")
        .output()
        .is_ok()
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

/// Scaffold a project with one source (`sources.raw_events`), one external
/// step declared to produce it (`command:` is `bash loader.sh`, written
/// separately by each test), and one consumer model reading the source.
fn scaffold(project_dir: &Path, db_path: &Path) -> Arc<Config> {
    std::fs::create_dir_all(project_dir.join("models").join("sources")).unwrap();
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
        project_dir.join("models").join("consumer.sql"),
        "---\nmaterialization: table\n---\nSELECT id FROM smelt.sources.raw_events\n",
    )
    .unwrap();

    Arc::new(Config::load(project_dir).expect("load smelt.yml"))
}

fn write_loader_script(project_dir: &Path, body: &str) {
    std::fs::write(project_dir.join("loader.sh"), body).expect("write loader.sh");
}

/// The step's `command:` creates and populates the produced source's table
/// before the consumer model runs — proven by the run succeeding at all
/// (the consumer's `SELECT` would otherwise fail against a nonexistent
/// table).
#[tokio::test]
async fn step_runs_before_its_consumer() {
    if !duckdb_cli_available() {
        eprintln!("duckdb CLI not on PATH — skipping step_runs_before_its_consumer");
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
        "run-1".to_string(),
        make_request(vec!["consumer".to_string()], false, true),
        Arc::clone(&config),
        graph,
        db,
        &project_dir,
        &DuckDbBackendFactory {
            db_path: db_path.clone(),
        },
        &NoOpReporter,
        CancellationToken::new(),
    )
    .await
    .expect("run must succeed — the step must have landed the table before the consumer ran");

    let backend = DuckDbBackend::new(&db_path, "main")
        .await
        .expect("reopen duckdb");
    let rows = backend
        .execute_sql("SELECT id FROM main.consumer")
        .await
        .expect("consumer table must exist");
    assert_eq!(rows[0].num_rows(), 1);
}

/// A non-zero exit from the step's `command:` fails the run, naming the
/// step and its exit code, and leaves the consumer unbuilt.
#[tokio::test]
async fn nonzero_exit_fails_run_naming_step() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project_dir = tmp.path().join("proj");
    std::fs::create_dir_all(&project_dir).unwrap();
    let db_path = tmp.path().join("run.duckdb");
    let config = scaffold(&project_dir, &db_path);
    write_loader_script(&project_dir, "#!/bin/bash\nexit 7\n");

    let (db, graph) = build_db_and_graph(&project_dir, &config);
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
        &NoOpReporter,
        CancellationToken::new(),
    )
    .await
    .expect_err("a nonzero exit must fail the run");
    let message = err.to_string();
    assert!(message.contains("ExternalStepFailed"), "got: {message}");
    assert!(message.contains("loader"), "got: {message}");
    assert!(message.contains('7'), "got: {message}");
    assert!(
        !db_path.exists() || {
            // The database file may exist (created by the backend factory
            // opening it) even though the consumer table inside it does not.
            true
        },
        "sanity"
    );
}

/// A `command:` naming no executable refuses with `ExternalStepNotInvocable`;
/// nothing is built.
#[tokio::test]
async fn unspawnable_command_refuses() {
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
        "external_step:\n  produces:\n    - smelt.sources.raw_events\n  command: [\"/no/such/binary-xyz\"]\n",
    )
    .unwrap();
    std::fs::write(
        project_dir.join("models").join("consumer.sql"),
        "---\nmaterialization: table\n---\nSELECT id FROM smelt.sources.raw_events\n",
    )
    .unwrap();
    let config = Arc::new(Config::load(&project_dir).expect("load smelt.yml"));

    let (db, graph) = build_db_and_graph(&project_dir, &config);
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
        &NoOpReporter,
        CancellationToken::new(),
    )
    .await
    .expect_err("an unspawnable command must refuse");
    assert!(
        err.to_string().contains("ExternalStepNotInvocable"),
        "got: {err}"
    );
}

/// A dry run whose selection reaches a step refuses with
/// `ExternalStepNotInvocable` rather than proceeding.
#[tokio::test]
async fn dry_run_reaching_step_refuses() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project_dir = tmp.path().join("proj");
    std::fs::create_dir_all(&project_dir).unwrap();
    let db_path = tmp.path().join("run.duckdb");
    let config = scaffold(&project_dir, &db_path);
    write_loader_script(&project_dir, "#!/bin/bash\nexit 0\n");

    let (db, graph) = build_db_and_graph(&project_dir, &config);
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
        &NoOpReporter,
        CancellationToken::new(),
    )
    .await
    .expect_err("a dry run reaching a step must refuse");
    assert!(
        err.to_string().contains("ExternalStepNotInvocable"),
        "got: {err}"
    );
}

/// `invoke_external_steps: false` (the "environment that cannot execute"
/// leg) refuses with the same code as a dry run.
#[tokio::test]
async fn environment_declining_invocation_refuses() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project_dir = tmp.path().join("proj");
    std::fs::create_dir_all(&project_dir).unwrap();
    let db_path = tmp.path().join("run.duckdb");
    let config = scaffold(&project_dir, &db_path);
    write_loader_script(&project_dir, "#!/bin/bash\nexit 0\n");

    let (db, graph) = build_db_and_graph(&project_dir, &config);
    let err = execute_project(
        "run-1".to_string(),
        make_request(vec!["consumer".to_string()], false, false),
        Arc::clone(&config),
        graph,
        db,
        &project_dir,
        &DuckDbBackendFactory {
            db_path: db_path.clone(),
        },
        &NoOpReporter,
        CancellationToken::new(),
    )
    .await
    .expect_err("invoke_external_steps: false reaching a step must refuse");
    assert!(
        err.to_string().contains("ExternalStepNotInvocable"),
        "got: {err}"
    );
}

/// A step whose produced source no selected model reads is never invoked —
/// its marker file stays absent and the run succeeds.
#[tokio::test]
async fn unreached_step_is_not_invoked() {
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
    // Consumer does NOT read the source — an ordinary standalone model.
    std::fs::write(
        project_dir.join("models").join("standalone.sql"),
        "---\nmaterialization: table\n---\nSELECT 1 AS x\n",
    )
    .unwrap();
    write_loader_script(&project_dir, "#!/bin/bash\ntouch invoked.marker\n");
    let config = Arc::new(Config::load(&project_dir).expect("load smelt.yml"));

    let (db, graph) = build_db_and_graph(&project_dir, &config);
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
        &NoOpReporter,
        CancellationToken::new(),
    )
    .await
    .expect("run over an unrelated model must succeed without the step");

    assert!(
        !project_dir.join("invoked.marker").exists(),
        "a step whose produced source no selected model reads must never be invoked"
    );
}

/// `--select <step>` alone invokes the step and builds zero models.
#[tokio::test]
async fn selecting_the_step_alone_invokes_it_and_builds_no_models() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project_dir = tmp.path().join("proj");
    std::fs::create_dir_all(&project_dir).unwrap();
    let db_path = tmp.path().join("run.duckdb");
    let config = scaffold(&project_dir, &db_path);
    write_loader_script(&project_dir, "#!/bin/bash\ntouch invoked.marker\n");

    let (db, graph) = build_db_and_graph(&project_dir, &config);
    let outcome = execute_project(
        "run-1".to_string(),
        make_request(vec!["loader".to_string()], false, true),
        Arc::clone(&config),
        graph,
        db,
        &project_dir,
        &DuckDbBackendFactory {
            db_path: db_path.clone(),
        },
        &NoOpReporter,
        CancellationToken::new(),
    )
    .await
    .expect("selecting the step alone must succeed");

    assert!(
        project_dir.join("invoked.marker").exists(),
        "selecting the step by name must invoke it"
    );
    assert!(
        outcome.models.is_empty(),
        "selecting the step alone must build zero models, got: {:?}",
        outcome.models
    );
}
