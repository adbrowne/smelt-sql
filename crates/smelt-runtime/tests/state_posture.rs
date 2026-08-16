//! `docs/outcomes/20260816-state-residency/phases/02-plan.md`: `state.mode`
//! is a runtime input, consulted by `FileStore` (`crates/smelt-state/src/
//! file_store.rs`) via `execute_project`'s single `FileStore::new`
//! construction site. This is the run-pipeline-level counterpart of
//! `crates/smelt-state/src/file_store.rs`'s unit tests — it exercises the
//! real `execute_project` entry point against a real DuckDB backend, so a
//! regression in *wiring* `config.state.mode` through to `FileStore` (as
//! opposed to a regression in the gating logic itself) is caught too.
//!
//! Consequence table under test (`docs/specs/state.md` §"`state.mode` and
//! what each posture provides"):
//!
//! | Posture | Observability structures written |
//! |---|---|
//! | `stateless` | none — `.smelt/` need not exist |
//! | `intervals` | manifests, reports, interval ledger, landed deltas, schema snapshots, source postures, probe baselines |
//! | `environments` | everything in `intervals` plus the snapshot/environment store |

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use smelt_backend::Backend;
use smelt_backend_duckdb::DuckDbBackend;
use smelt_core::config::{Config, Materialization, ModelConfig, StateConfig, StateMode, Target};
use smelt_core::graph::DependencyGraph;
use smelt_core::ModelDiscovery;
use smelt_maintenance_testkit::link_c_harness::{base_request, LinkCProject};
use smelt_runtime::execute::{BackendFactory, BackendFuture};
use smelt_runtime::reporter::RunReporter;
use smelt_runtime::types::ExecuteRequest;
use smelt_runtime::{execute_project, NoOpReporter};
use smelt_state::file_store::FileStore;
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

fn write_model(project_dir: &Path, name: &str, content: &str) {
    let path = project_dir.join("models").join(format!("{}.sql", name));
    std::fs::write(path, content).expect("write model file");
}

fn build_db_and_graph(
    project_dir: &Path,
    config: &Config,
) -> (
    Arc<tokio::sync::Mutex<smelt_db::Database>>,
    Arc<tokio::sync::Mutex<DependencyGraph>>,
) {
    let discovery = ModelDiscovery::new(project_dir.to_path_buf(), config.paths.clone());
    let sql_models = discovery.discover_models().expect("discover_models failed");

    let mut db = smelt_db::Database::default();
    let project = db.set_project_input(project_dir.to_path_buf(), String::new());
    let source_files: Vec<_> = sql_models
        .iter()
        .map(|m| db.set_source_file(m.path.clone(), m.content.clone(), project_dir.to_path_buf()))
        .collect();
    db.set_workspace(source_files, vec![project]);
    if let Ok(cfg) = Config::load(project_dir) {
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
        jobs: None,
        retry_max: None,
        retry_backoff_ms: None,
        resume: false,
        technique_overrides: vec![],
        keyed_restrictions: std::collections::BTreeMap::new(),
    }
}

/// Set up a single-model (`base`, materialized as a table) project under the
/// given `state.mode` posture, pointed at a fresh DuckDB file. Returns
/// everything needed to call `execute_project`.
struct Fixture {
    _tmp: tempfile::TempDir,
    project_dir: std::path::PathBuf,
    db_path: std::path::PathBuf,
    config: Arc<Config>,
    graph: Arc<tokio::sync::Mutex<DependencyGraph>>,
    db: Arc<tokio::sync::Mutex<smelt_db::Database>>,
}

fn setup(mode: StateMode) -> Fixture {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project_dir = tmp.path().to_path_buf();
    std::fs::create_dir_all(project_dir.join("models")).unwrap();
    write_model(&project_dir, "base", "SELECT 1 AS id, 'hello' AS label");

    let db_path = project_dir.join("run.duckdb");
    let smelt_yml = format!(
        "name: posture_test\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: {db}\n    schema: main\ndefault_materialization: table\nstate:\n  mode: {mode}\n",
        db = db_path.display(),
        mode = mode.as_str(),
    );
    std::fs::write(project_dir.join("smelt.yml"), &smelt_yml).unwrap();

    let mut targets = HashMap::new();
    targets.insert(
        "dev".to_string(),
        Target {
            target_type: "duckdb".to_string(),
            database: Some(db_path.to_string_lossy().into_owned()),
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
        "base".to_string(),
        ModelConfig {
            materialization: Some(Materialization::Table),
            timeseries: None,
            refresh: None,
            grain: None,
            unique_key: None,
            safety_overrides: None,
            batched_retired: (),
            merge_key: None,
            tags: vec![],
            target: None,
            format: None,
        },
    );

    let config = Arc::new(Config {
        name: "posture_test".to_string(),
        version: 1,
        paths: vec!["models".to_string()],
        targets,
        default_materialization: Materialization::Table,
        models: models_config,
        python: None,
        target: None,
        state: StateConfig { mode },
        maintenance: None,
        probes: Default::default(),
    });

    let (db, graph) = build_db_and_graph(&project_dir, &config);

    Fixture {
        _tmp: tmp,
        project_dir,
        db_path,
        config,
        graph,
        db,
    }
}

async fn run(fixture: &Fixture) -> smelt_runtime::types::RunOutcome {
    execute_project(
        "posture-run".to_string(),
        make_request("dev"),
        Arc::clone(&fixture.config),
        Arc::clone(&fixture.graph),
        Arc::clone(&fixture.db),
        &fixture.project_dir,
        &DuckDbBackendFactory {
            db_path: fixture.db_path.clone(),
        },
        &NoOpReporter,
        CancellationToken::new(),
    )
    .await
    .expect("run must succeed")
}

#[tokio::test]
async fn stateless_run_creates_no_smelt_dir() {
    let fixture = setup(StateMode::Stateless);
    let outcome = run(&fixture).await;
    assert!(outcome.models.contains_key("base"));

    assert!(
        !fixture.project_dir.join(".smelt").exists(),
        "a stateless project must never create .smelt/"
    );
}

#[tokio::test]
async fn stateless_run_writes_no_manifest_or_report() {
    let fixture = setup(StateMode::Stateless);
    run(&fixture).await;

    let file_store = FileStore::new(&fixture.project_dir, "dev", StateMode::Environments);
    assert!(
        file_store.load_runs(None).unwrap().is_empty(),
        "stateless run must leave no manifest behind, even read back under a \
         different posture"
    );
}

/// The reconciliation ledger's frontier grading is a correctness structure,
/// not an observability one (`docs/specs/state.md` §"`state.mode` and what
/// each posture provides") — it is engine-resident and posture-ungated, so
/// even a `stateless` run still records it in `_smelt_frontier`. Needs its
/// own incremental fixture, same shape as
/// `intervals_run_writes_manifest_intervals_and_schemas` below, since the
/// shared `Fixture`'s `base` model is a plain full-refresh table.
#[tokio::test]
async fn stateless_run_still_records_the_frontier_in_the_engine() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project_dir = tmp.path().to_path_buf();
    std::fs::create_dir_all(project_dir.join("models")).unwrap();

    let model_sql = r#"---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
---
SELECT event_date, amount FROM main.raw_events
"#;
    std::fs::write(project_dir.join("models/base.sql"), model_sql).unwrap();

    let db_path = project_dir.join("run.duckdb");
    {
        let conn = duckdb::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            r#"
            CREATE SCHEMA IF NOT EXISTS main;
            CREATE OR REPLACE TABLE main.raw_events AS
            SELECT * FROM (VALUES
                (DATE '2026-01-01', 10.0),
                (DATE '2026-01-02', 5.0)
            ) AS t(event_date, amount);
            "#,
        )
        .unwrap();
    }

    let smelt_yml = format!(
        "name: stateless_frontier_test\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: {db}\n    schema: main\ndefault_materialization: table\nstate:\n  mode: stateless\n",
        db = db_path.display(),
    );
    std::fs::write(project_dir.join("smelt.yml"), &smelt_yml).unwrap();

    let mut targets = HashMap::new();
    targets.insert(
        "dev".to_string(),
        Target {
            target_type: "duckdb".to_string(),
            database: Some(db_path.to_string_lossy().into_owned()),
            schema: "main".to_string(),
            connect_url: None,
            catalog: None,
            warehouse: None,
            format: None,
            settings: None,
        },
    );
    let config = Arc::new(Config {
        name: "stateless_frontier_test".to_string(),
        version: 1,
        paths: vec!["models".to_string()],
        targets,
        default_materialization: Materialization::Table,
        models: HashMap::new(),
        python: None,
        target: None,
        state: StateConfig {
            mode: StateMode::Stateless,
        },
        maintenance: None,
        probes: Default::default(),
    });

    let (db, graph) = build_db_and_graph(&project_dir, &config);

    let mut request = make_request("dev");
    request.start = Some("2026-01-01".to_string());
    request.end = Some("2026-01-03".to_string());

    execute_project(
        "stateless-frontier-run".to_string(),
        request,
        Arc::clone(&config),
        Arc::clone(&graph),
        Arc::clone(&db),
        &project_dir,
        &DuckDbBackendFactory {
            db_path: db_path.clone(),
        },
        &NoOpReporter,
        CancellationToken::new(),
    )
    .await
    .expect("stateless incremental run must succeed");

    assert!(
        !project_dir.join(".smelt").exists(),
        "a stateless project must never create .smelt/"
    );

    let backend = DuckDbBackend::new(&db_path, "main")
        .await
        .expect("reopen duckdb");
    let rows = backend
        .execute_sql("SELECT model_name FROM main._smelt_frontier WHERE model_name = 'base'")
        .await
        .expect("query frontier table");
    let total_rows: usize = rows.iter().map(|b| b.num_rows()).sum();
    assert_eq!(
        total_rows, 1,
        "the frontier record is correctness-class and must be written even under stateless"
    );
}

/// The interval ledger is only written on the incremental-materialization
/// execute path (`execute.rs`'s single `save_intervals` call site), so this
/// needs its own fixture: a `refresh: incremental` model with a `timeseries`
/// declaration, run over an explicit `[start, end)` window — mirrors
/// `tests/contract_deferral_skip_e2e.rs`'s harness shape.
#[tokio::test]
async fn intervals_run_writes_manifest_intervals_and_schemas() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project_dir = tmp.path().to_path_buf();
    std::fs::create_dir_all(project_dir.join("models")).unwrap();

    let model_sql = r#"---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
---
SELECT event_date, amount FROM main.raw_events
"#;
    std::fs::write(project_dir.join("models/base.sql"), model_sql).unwrap();

    let db_path = project_dir.join("run.duckdb");
    {
        let conn = duckdb::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            r#"
            CREATE SCHEMA IF NOT EXISTS main;
            CREATE OR REPLACE TABLE main.raw_events AS
            SELECT * FROM (VALUES
                (DATE '2026-01-01', 10.0),
                (DATE '2026-01-02', 5.0)
            ) AS t(event_date, amount);
            "#,
        )
        .unwrap();
    }

    let smelt_yml = format!(
        "name: intervals_posture_test\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: {db}\n    schema: main\ndefault_materialization: table\nstate:\n  mode: intervals\n",
        db = db_path.display(),
    );
    std::fs::write(project_dir.join("smelt.yml"), &smelt_yml).unwrap();

    let mut targets = HashMap::new();
    targets.insert(
        "dev".to_string(),
        Target {
            target_type: "duckdb".to_string(),
            database: Some(db_path.to_string_lossy().into_owned()),
            schema: "main".to_string(),
            connect_url: None,
            catalog: None,
            warehouse: None,
            format: None,
            settings: None,
        },
    );
    let config = Arc::new(Config {
        name: "intervals_posture_test".to_string(),
        version: 1,
        paths: vec!["models".to_string()],
        targets,
        default_materialization: Materialization::Table,
        models: HashMap::new(),
        python: None,
        target: None,
        state: StateConfig {
            mode: StateMode::Intervals,
        },
        maintenance: None,
        probes: Default::default(),
    });

    let (db, graph) = build_db_and_graph(&project_dir, &config);

    let mut request = make_request("dev");
    request.start = Some("2026-01-01".to_string());
    request.end = Some("2026-01-03".to_string());

    execute_project(
        "intervals-posture-run".to_string(),
        request,
        Arc::clone(&config),
        Arc::clone(&graph),
        Arc::clone(&db),
        &project_dir,
        &DuckDbBackendFactory {
            db_path: db_path.clone(),
        },
        &NoOpReporter,
        CancellationToken::new(),
    )
    .await
    .expect("intervals run must succeed");

    let file_store = FileStore::new(&project_dir, "dev", StateMode::Intervals);
    assert!(
        !file_store.load_runs(None).unwrap().is_empty(),
        "intervals posture must write the run manifest"
    );
    assert!(
        file_store.load_intervals().unwrap().get("base").is_some(),
        "intervals posture must write the interval ledger"
    );
    assert!(
        file_store.load_schema("base").unwrap().is_some(),
        "intervals posture must write the deployed schema snapshot"
    );
}

#[tokio::test]
async fn intervals_run_writes_no_snapshot_store() {
    let fixture = setup(StateMode::Intervals);
    run(&fixture).await;

    assert!(
        !fixture
            .project_dir
            .join(".smelt/targets/dev/snapshots.json")
            .exists(),
        "intervals posture must not write the snapshot/environment store"
    );
}

/// The snapshot/environment store is not yet wired into any
/// `execute_project` write path (`smelt-fingerprint`'s environment-reuse
/// machinery is out of scope for this phase — see the outcome's phase 2
/// plan). This asserts the posture-gating leg directly: a `FileStore` built
/// with the same target/mode `execute_project` would use for an
/// `environments` run does write the snapshot store, so the day that write
/// path lands, it is unblocked by `FileStore::writes` rather than gated out.
#[tokio::test]
async fn environments_run_writes_snapshot_store() {
    let fixture = setup(StateMode::Environments);
    run(&fixture).await;

    let file_store = FileStore::new(&fixture.project_dir, "dev", StateMode::Environments);
    file_store
        .save_snapshot_store(&smelt_state::snapshot_store::SnapshotStore::default())
        .expect("environments posture must accept a snapshot-store write");
    assert!(
        fixture
            .project_dir
            .join(".smelt/targets/dev/snapshots.json")
            .exists(),
        "environments posture must write the snapshot/environment store"
    );
}

#[tokio::test]
async fn resume_under_stateless_refuses_naming_the_posture() {
    let fixture = setup(StateMode::Stateless);
    run(&fixture).await;

    let mut request = make_request("dev");
    request.resume = true;
    let err = execute_project(
        "posture-run-resume".to_string(),
        request,
        Arc::clone(&fixture.config),
        Arc::clone(&fixture.graph),
        Arc::clone(&fixture.db),
        &fixture.project_dir,
        &DuckDbBackendFactory {
            db_path: fixture.db_path.clone(),
        },
        &NoOpReporter,
        CancellationToken::new(),
    )
    .await
    .expect_err("--resume under stateless must refuse");

    let msg = err.to_string();
    assert!(
        msg.contains("state.mode"),
        "error must name state.mode: {msg}"
    );
    assert!(
        msg.contains("stateless"),
        "error must name the stateless posture: {msg}"
    );
    assert!(
        !msg.contains("no partially-failed run"),
        "must not be the generic no-history error, which reads as \"your last \
         run succeeded\" rather than \"this posture keeps no history\": {msg}"
    );
}

/// Captures `probe_advisory` calls for assertion.
#[derive(Default)]
struct RecordingReporter {
    advisories: Mutex<Vec<String>>,
}

impl RunReporter for RecordingReporter {
    fn probe_advisory(&self, _run_id: &str, model: &str, code: &str, message: &str) {
        self.advisories
            .lock()
            .unwrap()
            .push(format!("{code}: model '{model}': {message}"));
    }
}

fn write_file(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, content).unwrap();
}

fn stage_stateless_append_only_project(tmp: &tempfile::TempDir) -> anyhow::Result<LinkCProject> {
    let root = tmp.path().join("stateless_probe_project");
    let db_path = root.join("target/dev.duckdb");
    write_file(
        &root.join("smelt.yml"),
        "name: stateless_probe_project\n\
         version: 1\n\
         paths:\n  - models\n\
         targets:\n  dev:\n    type: duckdb\n    database: target/dev.duckdb\n    schema: main\n\
         default_materialization: table\n\
         probes:\n  cadence: per_run\n\
         state:\n  mode: stateless\n",
    );
    write_file(
        &root.join("models/sources/raw/clicks.yml"),
        "description: Raw append-only clickstream rows; pre-loaded by the test\n\
         name: raw.clicks\n\
         mutation_profile:\n  kind: append_only\n\
         timeseries:\n\
         \x20 event_time_column: click_ts\n  partition_column: click_date\n  granularity: day\n\
         columns:\n\
         \x20 - name: click_ts\n    type: TIMESTAMP\n\
         \x20 - name: click_date\n    type: DATE\n\
         \x20 - name: payload\n    type: VARCHAR\n",
    );
    write_file(
        &root.join("models/clicks_summary.sql"),
        "---\n\
         materialization: table\n\
         ---\n\
         SELECT click_date, payload FROM smelt.sources.raw.clicks\n",
    );
    std::fs::create_dir_all(db_path.parent().unwrap())?;
    LinkCProject::load(root, db_path)
}

fn seed_clicks(db_path: &Path) -> anyhow::Result<()> {
    let conn = duckdb::Connection::open(db_path)?;
    conn.execute_batch(
        "CREATE SCHEMA IF NOT EXISTS raw; \
         CREATE TABLE raw.clicks (click_ts TIMESTAMP, click_date DATE, payload VARCHAR); \
         INSERT INTO raw.clicks VALUES \
           (TIMESTAMP '2026-01-01 00:00:00', DATE '2026-01-01', 'a'), \
           (TIMESTAMP '2026-01-02 00:00:00', DATE '2026-01-02', 'b');",
    )?;
    Ok(())
}

/// Under `state.mode: stateless` no `.smelt/` ever exists, so no posture
/// baseline is ever recorded — every run is an establishing run, and the
/// optionality rule requires that be *reported*, not silent
/// (`docs/specs/sources.md` §Semantics 4). This asserts the "reported"
/// half by running twice: a stateful project's second run would verify
/// against a persisted baseline and report nothing, but stateless can never
/// persist one, so both runs must report the advisory.
#[tokio::test]
async fn stateless_posture_reports_baseline_unavailable_every_run() -> anyhow::Result<()> {
    let tmp = tempfile::TempDir::new()?;
    let project = stage_stateless_append_only_project(&tmp)?;
    seed_clicks(&project.db_path)?;

    let reporter = RecordingReporter::default();
    project
        .run("stateless-probe-1", base_request("dev"), &reporter)
        .await?;
    assert_eq!(
        reporter.advisories.lock().unwrap().len(),
        1,
        "the first run must report the advisory"
    );

    project
        .run("stateless-probe-2", base_request("dev"), &reporter)
        .await?;
    let advisories = reporter.advisories.lock().unwrap();
    assert_eq!(
        advisories.len(),
        2,
        "a second run under `stateless` must still report the advisory — there \
         is no persisted baseline for it to verify against: {advisories:?}"
    );
    for advisory in advisories.iter() {
        assert!(advisory.contains("ProbeBaselineUnavailable"), "{advisory}");
    }
    Ok(())
}
