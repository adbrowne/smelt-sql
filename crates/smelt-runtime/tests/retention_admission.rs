//! Real-DuckDB, `execute_project`-driven coverage for the run-time
//! reach-versus-retention rolling re-evaluation
//! (`docs/outcomes/20260906-trimmed-history-sources/outcome.md` criterion 5,
//! `docs/outcomes/20260906-trimmed-history-sources/phases/05-plan.md` tests
//! 6-8). Mirrors `contract_deferral_skip_e2e.rs`'s harness: a plain
//! `DuckDbBackend` driven through the real `execute_project` pipeline (Run
//! Pipeline Parity).
//!
//! Fixture: one append-only, clocked source (`sources.events`) declares
//! `retention: '45 days'`. `agg` reads a 7-day lookback window over it —
//! admissible at authoring time, but a backfill run whose window is old
//! enough ages that reach past the retained bound. `unbounded` reads an
//! unbounded cumulative window over the same source — never admissible
//! (`RetentionVerdict::UnprovableWithin`), so every run over it records a
//! downgrade rather than a refusal.

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

/// Captures every `RunReporter` callback as a string, so a test can assert a
/// `maintenance_warning` fired (or didn't) without a bespoke reporter per
/// test.
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

    // Retention-bearing source: `sources.events`, bound at 45 days.
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

    // `agg`: a 7-day lookback (a bounded RANGE frame, not a `CURRENT_DATE`
    // predicate — `CURRENT_DATE` type-checks as an `UndeclaredColumn` in
    // today's dialect surface, which is orthogonal to this fixture and
    // would trip the pre-execution diagnostics gate) — admissible at
    // authoring time, but a run window old enough ages that reach past the
    // 45-day bound.
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

    // `unbounded`: a cumulative window over the same source — the reach is
    // never provably bounded, so this always records a downgrade rather
    // than a refusal, regardless of any run's window age. `allow_full_scan`
    // admits the resulting full-table maintenance scan
    // (`MaintenanceScanUnbounded`), which is orthogonal to this fixture.
    let unbounded_sql = r#"---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
maintenance:
  scan_bounds:
    per_source:
      events:
        allow_full_scan: true
---
SELECT event_date,
       SUM(amount) OVER (PARTITION BY event_date ORDER BY event_date
           RANGE BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW) AS running_amount
FROM smelt.sources.events
"#;
    std::fs::write(project_dir.join("models/unbounded.sql"), unbounded_sql).unwrap();

    let smelt_yml = format!(
        "name: retention_admission_test\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: {db}\n    schema: main\ndefault_materialization: table\n",
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

fn make_request(select: Vec<String>, start: Option<&str>, end: Option<&str>) -> ExecuteRequest {
    ExecuteRequest {
        target: "dev".to_string(),
        select,
        exclude: vec![],
        start: start.map(|s| s.to_string()),
        end: end.map(|e| e.to_string()),
        batch_size_days: None,
        per_partition: false,
        full_refresh: false,
        rebuild: false,
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
    select: Vec<String>,
    start: Option<&str>,
    end: Option<&str>,
) -> anyhow::Result<smelt_runtime::types::RunOutcome> {
    execute_project(
        label.to_string(),
        make_request(select, start, end),
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

/// Test 6: a model over a `retention: '45 days'` source, run with a window
/// starting well over 90 days before the run clock, fails with
/// `SourceRetentionExceeded` naming the source, and the model's target
/// table is never created.
#[tokio::test]
async fn a_backfill_window_older_than_retention_refuses_before_any_statement() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().to_path_buf();
    let db_path = project_dir.join("dev.duckdb");

    stage_project(&project_dir, &db_path);
    seed_events(&db_path).expect("seed events");

    let config = Arc::new(Config::load(&project_dir).expect("load config"));
    let (db, graph) = build_db_and_graph(&project_dir, &config);

    let result = run(
        "run-backfill",
        &config,
        &db,
        &graph,
        &project_dir,
        &db_path,
        &smelt_runtime::NoOpReporter,
        vec!["agg".to_string()],
        Some("2026-06-01"),
        Some("2026-06-02"),
    )
    .await;

    let err = result.expect_err("a backfill window aged past retention must refuse");
    let message = err.to_string();
    assert!(
        message.contains("SourceRetentionExceeded"),
        "error must name the SourceRetentionExceeded refusal: {message}"
    );
    assert!(
        message.contains("events"),
        "error must name the exceeding source: {message}"
    );
    assert!(
        !table_exists(&db_path, "agg"),
        "a refused model's target table must never be created"
    );
}

/// Test 7: the same project, run forward-only (no explicit `--start`/`--end`)
/// still succeeds — age zero never refuses, so steady-state maintenance is
/// unaffected by this check.
#[tokio::test]
async fn a_forward_only_run_over_the_same_model_still_succeeds() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().to_path_buf();
    let db_path = project_dir.join("dev.duckdb");

    stage_project(&project_dir, &db_path);
    seed_events(&db_path).expect("seed events");

    let config = Arc::new(Config::load(&project_dir).expect("load config"));
    let (db, graph) = build_db_and_graph(&project_dir, &config);

    let result = run(
        "run-forward",
        &config,
        &db,
        &graph,
        &project_dir,
        &db_path,
        &smelt_runtime::NoOpReporter,
        vec!["agg".to_string()],
        None,
        None,
    )
    .await;

    result.expect("a forward-only run must not be refused by the retention check");
    assert!(
        table_exists(&db_path, "agg"),
        "a successful run must create the model's target table"
    );
}

/// Test 8: a model whose reach into a `retention:`-bearing source can never
/// be proven bounded (`RetentionVerdict::UnprovableWithin`) surfaces its
/// recorded downgrade through the run reporter as a warning exactly once —
/// never silence, and never a refusal (there is no reach to age).
#[tokio::test]
async fn the_retention_downgrade_is_reported_once_per_run() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().to_path_buf();
    let db_path = project_dir.join("dev.duckdb");

    stage_project(&project_dir, &db_path);
    seed_events(&db_path).expect("seed events");

    let config = Arc::new(Config::load(&project_dir).expect("load config"));
    let (db, graph) = build_db_and_graph(&project_dir, &config);

    let reporter = CapturingReporter::default();
    let result = run(
        "run-unbounded",
        &config,
        &db,
        &graph,
        &project_dir,
        &db_path,
        &reporter,
        vec!["unbounded".to_string()],
        None,
        None,
    )
    .await;

    result.expect("an unprovable reach records a downgrade, never a refusal");

    let events = reporter.events_snapshot();
    let warnings: Vec<&String> = events
        .iter()
        .filter(|e| e.contains("SourceRetentionDowngraded"))
        .collect();
    assert_eq!(
        warnings.len(),
        1,
        "the downgrade must be reported exactly once per run: {events:?}"
    );
    assert!(
        warnings[0].contains("events"),
        "the warning must name the source: {warnings:?}"
    );
}
