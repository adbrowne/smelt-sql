//! Phase 2 (`docs/outcomes/20260809-contract-lattice-v1/phases/02-plan.md`):
//! the `contract.frozen_horizon` write-eligibility clamp
//! (`docs/specs/incremental_models.md` §"Contract relaxations
//! (`contract:`)") narrows a partition-grain model's requested write range to
//! `end - H`, and leaves the range untouched when no `contract:` is
//! declared. Mirrors `crates/smelt-runtime/tests/dry_run_statements.rs`'s
//! harness — a dry-run `execute_project` call, observed via
//! `RunReporter::maintenance_statements`'s reported `ChunkInfo`.

use std::path::Path;
use std::sync::{Arc, Mutex};

use chrono::NaiveDate;
use smelt_core::config::Config;
use smelt_core::graph::DependencyGraph;
use smelt_core::ModelDiscovery;
use smelt_logical::maintenance::emit::StatementGroup;
use smelt_runtime::execute::{execute_project, BackendFactory, BackendFuture};
use smelt_runtime::reporter::{ChunkInfo, RunReporter};
use smelt_runtime::types::ExecuteRequest;
use tokio_util::sync::CancellationToken;

#[derive(Default)]
struct RecordingReporter {
    chunk_starts: Mutex<Vec<String>>,
}

impl RunReporter for RecordingReporter {
    fn maintenance_statements(
        &self,
        _run_id: &str,
        _model: &str,
        chunk: Option<&ChunkInfo>,
        _group: &StatementGroup,
    ) {
        if let Some(c) = chunk {
            self.chunk_starts.lock().unwrap().push(c.start.clone());
        }
    }
}

struct PanicBackendFactory;

impl BackendFactory for PanicBackendFactory {
    fn create<'a>(
        &'a self,
        _target_name: &'a str,
        _target_config: &'a smelt_core::config::Target,
        _project_dir: &'a Path,
    ) -> BackendFuture<'a> {
        Box::pin(async move {
            panic!("dry-run must not create a backend");
        })
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
    db.set_active_target(config.target.clone().map(|t| Arc::from(t.as_str())));

    let graph = DependencyGraph::build(sql_models, None).expect("build graph");

    (
        Arc::new(tokio::sync::Mutex::new(db)),
        Arc::new(tokio::sync::Mutex::new(graph)),
    )
}

fn write_project(project_dir: &Path, db_path: &Path, model_sql: &str) -> Arc<Config> {
    std::fs::create_dir_all(project_dir.join("models")).unwrap();
    std::fs::write(
        project_dir.join("models").join("daily_events.sql"),
        model_sql,
    )
    .unwrap();
    let smelt_yml = format!(
        "name: frozen_horizon_clamp_test\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: {db}\n    schema: main\ndefault_materialization: table\ntarget: dev\n",
        db = db_path.display()
    );
    std::fs::write(project_dir.join("smelt.yml"), &smelt_yml).unwrap();
    Arc::new(Config::load(project_dir).expect("load config"))
}

fn dry_run_request(start: &str, end: &str) -> ExecuteRequest {
    ExecuteRequest {
        target: "dev".to_string(),
        select: vec![],
        exclude: vec![],
        start: Some(start.to_string()),
        end: Some(end.to_string()),
        batch_size_days: None,
        per_partition: false,
        full_refresh: false,
        dry_run: true,
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
    }
}

const PARTITION_MODEL_TEMPLATE: &str = "---\n\
     materialization: table\n\
     refresh: incremental\n\
     grain: partition\n\
     timeseries:\n\
     \x20\x20partition_column: event_date\n\
     \x20\x20event_time_column: event_date\n\
     \x20\x20granularity: day\n\
     {CONTRACT}---\n\
     SELECT * FROM (VALUES (DATE '2024-01-01', 10), (DATE '2024-01-02', 20)) \
     AS t(event_date, amount)";

/// A `frozen_horizon: '30 days'` declaration narrows a wide requested range
/// so no reported chunk's start precedes `end - 30d`.
#[tokio::test]
async fn declared_horizon_narrows_the_write_range() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project_dir = tmp.path();
    let db_path = project_dir.join("run.duckdb");
    let model_sql =
        PARTITION_MODEL_TEMPLATE.replace("{CONTRACT}", "contract:\n  frozen_horizon: '30 days'\n");
    let config = write_project(project_dir, &db_path, &model_sql);

    let (db, graph) = build_db_and_graph(project_dir, &config);
    let reporter = RecordingReporter::default();

    execute_project(
        "frozen-horizon-clamp".to_string(),
        dry_run_request("2024-01-01", "2024-03-01"),
        Arc::clone(&config),
        graph,
        db,
        project_dir,
        &PanicBackendFactory,
        &reporter,
        CancellationToken::new(),
    )
    .await
    .expect("dry-run execute_project");

    let end = NaiveDate::parse_from_str("2024-03-01", "%Y-%m-%d").unwrap();
    let floor = end - chrono::Duration::days(30);

    let starts = reporter.chunk_starts.lock().unwrap();
    assert!(!starts.is_empty(), "expected at least one reported chunk");
    for start in starts.iter() {
        let start_date = NaiveDate::parse_from_str(start, "%Y-%m-%d")
            .unwrap_or_else(|e| panic!("chunk start '{start}' must parse: {e}"));
        assert!(
            start_date >= floor,
            "chunk start {start_date} precedes the frozen-horizon floor {floor}"
        );
    }
}

/// Absent `contract:`, the default point, the requested range is
/// byte-identical to today's batch decomposition — the first chunk starts
/// exactly at the requested start.
#[tokio::test]
async fn absent_contract_leaves_the_range_untouched() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project_dir = tmp.path();
    let db_path = project_dir.join("run.duckdb");
    let model_sql = PARTITION_MODEL_TEMPLATE.replace("{CONTRACT}", "");
    let config = write_project(project_dir, &db_path, &model_sql);

    let (db, graph) = build_db_and_graph(project_dir, &config);
    let reporter = RecordingReporter::default();

    execute_project(
        "frozen-horizon-absent".to_string(),
        dry_run_request("2024-01-01", "2024-03-01"),
        Arc::clone(&config),
        graph,
        db,
        project_dir,
        &PanicBackendFactory,
        &reporter,
        CancellationToken::new(),
    )
    .await
    .expect("dry-run execute_project");

    let starts = reporter.chunk_starts.lock().unwrap();
    assert!(!starts.is_empty(), "expected at least one reported chunk");
    assert_eq!(
        starts.iter().min().unwrap().as_str(),
        "2024-01-01",
        "absent contract must leave the requested start untouched"
    );
}
