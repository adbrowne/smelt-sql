//! Real-DuckDB, `execute_project`-driven coverage for the whole-table-
//! recompute retention gate (`docs/outcomes/20260906-trimmed-history-sources/
//! outcome.md` criterion 4, `docs/outcomes/20260906-trimmed-history-sources/
//! phases/06-plan.md` tests 7-10). Mirrors `retention_admission.rs`'s
//! harness: a plain `DuckDbBackend` driven through the real `execute_project`
//! pipeline (Run Pipeline Parity).
//!
//! Fixture: one append-only, clocked source (`sources.events`) declares
//! `retention: '45 days'`. `agg` reads a bounded 7-day lookback window over
//! it — always admissible under the reach-versus-retention rolling
//! re-evaluation (`retention_admission.rs`'s own coverage), so every
//! `maintenance_warning` this file observes comes from the whole-table gate
//! under test, never that other check.

use std::path::Path;
use std::sync::{Arc, Mutex};

use smelt_backend::Backend;
use smelt_backend_duckdb::DuckDbBackend;
use smelt_core::config::{Config, Target};
use smelt_core::graph::DependencyGraph;
use smelt_core::ModelDiscovery;
use smelt_runtime::execute::{execute_project, BackendFactory, BackendFuture};
use smelt_runtime::reporter::RunReporter;
use smelt_runtime::types::ExecuteRequest;
use tokio_util::sync::CancellationToken;

struct PlainDuckDbFactory {
    db_path: std::path::PathBuf,
}

impl BackendFactory for PlainDuckDbFactory {
    fn create<'a>(
        &'a self,
        _target_name: &'a str,
        target_config: &'a Target,
        _project_dir: &'a Path,
    ) -> BackendFuture<'a> {
        let path = self.db_path.clone();
        let schema = target_config.schema.clone();
        Box::pin(async move {
            let inner = DuckDbBackend::new(&path, &schema)
                .await
                .map_err(|e| anyhow::anyhow!("DuckDB init failed: {}", e))?;
            Ok(Box::new(inner) as Box<dyn Backend>)
        })
    }
}

#[derive(Default)]
struct CapturingReporter {
    events: Mutex<Vec<String>>,
}

impl CapturingReporter {
    fn events_snapshot(&self) -> Vec<String> {
        self.events.lock().unwrap().clone()
    }
}

impl RunReporter for CapturingReporter {
    fn maintenance_warning(&self, _run_id: &str, model: &str, message: &str) {
        self.events
            .lock()
            .unwrap()
            .push(format!("maintenance_warning:{model}:{message}"));
    }
}

fn stage_project(project_dir: &Path, db_path: &Path) {
    std::fs::create_dir_all(project_dir.join("models/sources")).unwrap();

    let source_yml = r#"description: Raw events.
columns:
  - name: event_date
    type: DATE
  - name: amount
    type: DOUBLE
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
mutation_profile:
  kind: append_only
retention: '45 days'
"#;
    std::fs::write(project_dir.join("models/sources/events.yml"), source_yml).unwrap();

    let agg_sql = r#"---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
---
SELECT event_date,
       SUM(amount) OVER (PARTITION BY event_date ORDER BY event_date
           RANGE BETWEEN INTERVAL '7 days' PRECEDING AND CURRENT ROW) AS total_amount
FROM smelt.sources.events
"#;
    std::fs::write(project_dir.join("models/agg.sql"), agg_sql).unwrap();

    let smelt_yml = format!(
        "name: retention_full_refresh_test\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: {db}\n    schema: main\ndefault_materialization: table\n",
        db = db_path.display()
    );
    std::fs::write(project_dir.join("smelt.yml"), smelt_yml).unwrap();
}

fn seed_events(db_path: &Path) -> anyhow::Result<()> {
    let conn = duckdb::Connection::open(db_path)?;
    conn.execute_batch(
        r#"
        CREATE SCHEMA IF NOT EXISTS main;
        CREATE OR REPLACE TABLE main.sources_events AS
        SELECT * FROM (VALUES
            (DATE '2026-01-01', 10.0),
            (DATE '2026-01-02', 5.0),
            (DATE '2026-01-03', 7.0)
        ) AS t(event_date, amount);
        "#,
    )?;
    Ok(())
}

fn table_exists(db_path: &Path, table: &str) -> bool {
    let conn = duckdb::Connection::open(db_path).unwrap();
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM information_schema.tables WHERE table_schema = 'main' AND table_name = ?",
            [table],
            |r| r.get(0),
        )
        .unwrap();
    count > 0
}

fn row_count(db_path: &Path, table: &str) -> i64 {
    let conn = duckdb::Connection::open(db_path).unwrap();
    conn.query_row(&format!("SELECT COUNT(*) FROM main.{table}"), [], |r| {
        r.get(0)
    })
    .unwrap()
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

    let graph = DependencyGraph::build(sql_models, None).expect("build graph");

    (
        Arc::new(tokio::sync::Mutex::new(db)),
        Arc::new(tokio::sync::Mutex::new(graph)),
    )
}

fn make_request(full_refresh: bool, allow_full_refresh: bool) -> ExecuteRequest {
    ExecuteRequest {
        target: "dev".to_string(),
        select: vec!["agg".to_string()],
        exclude: vec![],
        start: None,
        end: None,
        batch_size_days: None,
        per_partition: false,
        full_refresh,
        rebuild: false,
        dry_run: false,
        enforce_safety: false,
        allow_column_removal: false,
        allow_full_refresh,
        ephemeral_seed_ctes: vec![],
        run_checks: false,
        checks: vec![],
        jobs: None,
        retry_max: None,
        retry_backoff_ms: None,
        resume: false,
        technique_overrides: vec![],
        invoke_external_steps: true,
    }
}

#[allow(clippy::too_many_arguments)]
async fn run(
    label: &str,
    config: &Arc<Config>,
    db: &Arc<tokio::sync::Mutex<smelt_db::Database>>,
    graph: &Arc<tokio::sync::Mutex<DependencyGraph>>,
    project_dir: &Path,
    db_path: &Path,
    reporter: &dyn RunReporter,
    full_refresh: bool,
    allow_full_refresh: bool,
) -> anyhow::Result<smelt_runtime::types::RunOutcome> {
    execute_project(
        label.to_string(),
        make_request(full_refresh, allow_full_refresh),
        Arc::clone(config),
        Arc::clone(graph),
        Arc::clone(db),
        project_dir,
        &PlainDuckDbFactory {
            db_path: db_path.to_path_buf(),
        },
        reporter,
        CancellationToken::new(),
    )
    .await
}

/// Test 7: a `--full-refresh` over a model with stored output, reading a
/// declared-`retention:` source, refuses with `SourceRetentionExceeded`
/// naming the source, and leaves the target table's rows untouched.
#[tokio::test]
async fn a_full_refresh_over_a_trimmed_source_refuses_and_leaves_the_table_intact() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().to_path_buf();
    let db_path = project_dir.join("dev.duckdb");

    stage_project(&project_dir, &db_path);
    seed_events(&db_path).expect("seed events");

    let config = Arc::new(Config::load(&project_dir).expect("load config"));
    let (db, graph) = build_db_and_graph(&project_dir, &config);

    run(
        "run-forward",
        &config,
        &db,
        &graph,
        &project_dir,
        &db_path,
        &smelt_runtime::NoOpReporter,
        false,
        false,
    )
    .await
    .expect("initial forward build must succeed");
    assert!(table_exists(&db_path, "agg"));
    let rows_before = row_count(&db_path, "agg");

    let result = run(
        "run-full-refresh",
        &config,
        &db,
        &graph,
        &project_dir,
        &db_path,
        &smelt_runtime::NoOpReporter,
        true,
        false,
    )
    .await;

    let err = result.expect_err("a full refresh over stored output with no license must refuse");
    let message = err.to_string();
    assert!(
        message.contains("SourceRetentionExceeded"),
        "error must name the SourceRetentionExceeded refusal: {message}"
    );
    assert!(
        message.contains("events"),
        "error must name the exceeding source: {message}"
    );
    assert_eq!(
        row_count(&db_path, "agg"),
        rows_before,
        "a refused full refresh must leave the target table's rows untouched"
    );
}

/// Test 8: the same setup, but with `--allow-full-refresh` — the recompute
/// succeeds and reports the loss exactly once, naming the source and its
/// retained bound.
#[tokio::test]
async fn allow_full_refresh_licenses_the_rebuild_and_reports_it_once() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().to_path_buf();
    let db_path = project_dir.join("dev.duckdb");

    stage_project(&project_dir, &db_path);
    seed_events(&db_path).expect("seed events");

    let config = Arc::new(Config::load(&project_dir).expect("load config"));
    let (db, graph) = build_db_and_graph(&project_dir, &config);

    run(
        "run-forward",
        &config,
        &db,
        &graph,
        &project_dir,
        &db_path,
        &smelt_runtime::NoOpReporter,
        false,
        false,
    )
    .await
    .expect("initial forward build must succeed");

    let reporter = CapturingReporter::default();
    run(
        "run-licensed-full-refresh",
        &config,
        &db,
        &graph,
        &project_dir,
        &db_path,
        &reporter,
        true,
        true,
    )
    .await
    .expect("an explicitly licensed full refresh must succeed");

    let events = reporter.events_snapshot();
    let warnings: Vec<&String> = events
        .iter()
        .filter(|e| e.contains("SourceRetentionDowngraded"))
        .collect();
    assert_eq!(
        warnings.len(),
        1,
        "the licensed loss must be reported exactly once: {events:?}"
    );
    assert!(
        warnings[0].contains("events"),
        "the warning must name the source: {warnings:?}"
    );
    assert!(
        warnings[0].contains("3888000"),
        "the warning must name the retained bound (45 days in seconds): {warnings:?}"
    );
}

/// Test 9: a first build (no stored output) with `--full-refresh` and no
/// explicit license still succeeds — refusing would make the model
/// unbuildable forever — and reports the loss.
#[tokio::test]
async fn a_first_build_full_refresh_succeeds_and_reports_the_loss() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().to_path_buf();
    let db_path = project_dir.join("dev.duckdb");

    stage_project(&project_dir, &db_path);
    seed_events(&db_path).expect("seed events");

    let config = Arc::new(Config::load(&project_dir).expect("load config"));
    let (db, graph) = build_db_and_graph(&project_dir, &config);

    assert!(!table_exists(&db_path, "agg"));

    let reporter = CapturingReporter::default();
    run(
        "run-first-build",
        &config,
        &db,
        &graph,
        &project_dir,
        &db_path,
        &reporter,
        true,
        false,
    )
    .await
    .expect("a first-build full refresh must never be refused");

    assert!(table_exists(&db_path, "agg"));
    let events = reporter.events_snapshot();
    assert_eq!(
        events
            .iter()
            .filter(|e| e.contains("SourceRetentionDowngraded"))
            .count(),
        1,
        "a first build still reports the loss, never silently: {events:?}"
    );
}

/// Test 10: regression guard — an ordinary forward-only incremental run
/// (no `--full-refresh`) over the same fixture is unaffected by this gate:
/// it succeeds with no warning from it.
#[tokio::test]
async fn a_forward_only_run_is_unaffected_by_the_gate() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().to_path_buf();
    let db_path = project_dir.join("dev.duckdb");

    stage_project(&project_dir, &db_path);
    seed_events(&db_path).expect("seed events");

    let config = Arc::new(Config::load(&project_dir).expect("load config"));
    let (db, graph) = build_db_and_graph(&project_dir, &config);

    let reporter = CapturingReporter::default();
    run(
        "run-forward-only",
        &config,
        &db,
        &graph,
        &project_dir,
        &db_path,
        &reporter,
        false,
        false,
    )
    .await
    .expect("a forward-only run must succeed");

    assert!(table_exists(&db_path, "agg"));
    let events = reporter.events_snapshot();
    assert!(
        events
            .iter()
            .all(|e| !e.contains("SourceRetentionDowngraded")),
        "a forward-only run must not trigger the whole-table-recompute gate's warning: {events:?}"
    );
}
