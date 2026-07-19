//! `--dry-run` renders the emitted maintenance statements
//! (`docs/specs/cli.md` §"`--dry-run` prints the maintenance statements"):
//! `execute_project` with `dry_run: true` reports, per maintained model, the
//! identical `StatementGroup` list the single-owner emitters
//! (`smelt_logical::maintenance::emit`) produce for the invocation's window —
//! with real literal bounds (no `{{window_start}}` placeholders) — and never
//! opens a backend or executes anything.

use std::path::Path;
use std::sync::{Arc, Mutex};

use smelt_backend::Backend;
use smelt_core::config::Config;
use smelt_core::graph::DependencyGraph;
use smelt_core::ModelDiscovery;
use smelt_logical::maintenance::emit::StatementGroup;
use smelt_logical::maintenance::emit::{emit_delete_insert, MaintenanceDialect, Region};
use smelt_runtime::execute::{execute_project, BackendFactory, BackendFuture};
use smelt_runtime::reporter::{ChunkInfo, RunReporter};
use smelt_runtime::types::ExecuteRequest;
use tokio_util::sync::CancellationToken;

/// One recorded `maintenance_statements` callback: model name, the chunk
/// window (index, total, start, end), and the emitted group.
type RecordedCall = (
    String,
    Option<(usize, usize, String, String)>,
    StatementGroup,
);

/// Captures every `maintenance_statements` callback the runtime emits — the
/// observation point the spec ties `--dry-run` to.
#[derive(Default)]
struct RecordingReporter {
    calls: Mutex<Vec<RecordedCall>>,
}

impl RunReporter for RecordingReporter {
    fn maintenance_statements(
        &self,
        _run_id: &str,
        model: &str,
        chunk: Option<&ChunkInfo>,
        group: &StatementGroup,
    ) {
        self.calls.lock().unwrap().push((
            model.to_string(),
            chunk.map(|c| (c.index, c.total, c.start.clone(), c.end.clone())),
            group.clone(),
        ));
    }
}

/// A `BackendFactory` that panics if `create` is ever called — a dry-run must
/// never construct a backend (`docs/specs/cli.md` §"`--dry-run` prints the
/// maintenance statements": "never executes a statement against the target").
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

const PARTITION_MODEL: &str = "---\n\
     materialization: table\n\
     refresh: incremental\n\
     grain: partition\n\
     timeseries:\n\
     \x20\x20partition_column: event_date\n\
     \x20\x20event_time_column: event_date\n\
     \x20\x20granularity: day\n\
     ---\n\
     SELECT * FROM (VALUES (DATE '2024-01-01', 10), (DATE '2024-01-02', 20)) \
     AS t(event_date, amount)";

fn write_project(project_dir: &Path, db_path: &Path) -> Arc<Config> {
    std::fs::create_dir_all(project_dir.join("models")).unwrap();
    write_model(project_dir, "daily_events", PARTITION_MODEL);
    let smelt_yml = format!(
        "name: dry_run_test\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: {db}\n    schema: main\ndefault_materialization: table\ntarget: dev\n",
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
    }
}

/// A dry-run over a `grain: partition` fixture reports, for the model, the
/// identical statement list `emit_delete_insert` produces for that window —
/// with real literal bounds — asserted by re-deriving the emitter output from
/// the reported group's own parsed region + body, not a golden string.
#[tokio::test]
async fn dry_run_reports_emitted_statements() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project_dir = tmp.path();
    let db_path = project_dir.join("run.duckdb");
    let config = write_project(project_dir, &db_path);

    let (db, graph) = build_db_and_graph(project_dir, &config);
    let reporter = RecordingReporter::default();

    execute_project(
        "dry-run-report".to_string(),
        dry_run_request("2024-01-01", "2024-01-03"),
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

    let calls = reporter.calls.lock().unwrap();
    let groups: Vec<_> = calls
        .iter()
        .filter(|(m, _, _)| m == "daily_events")
        .collect();
    assert_eq!(
        groups.len(),
        1,
        "fully-batch-safe model over one window emits exactly one DELETE+INSERT group: {:?}",
        calls
    );

    let (_, chunk, group) = groups[0];

    // Real literal bounds — no symbolic placeholders.
    assert!(
        group.transactional,
        "region DELETE+INSERT must be transactional"
    );
    assert_eq!(group.statements.len(), 2);
    let delete_sql = &group.statements[0].sql;
    let insert_sql = &group.statements[1].sql;
    assert!(
        !delete_sql.contains("{{window_start}}") && !delete_sql.contains("{{window_end}}"),
        "dry-run literals must be real, not placeholders: {delete_sql}"
    );
    assert!(
        delete_sql.contains("'2024-01-01'") && delete_sql.contains("'2024-01-03'"),
        "expected the real invocation window literals: {delete_sql}"
    );
    assert!(delete_sql.starts_with("DELETE FROM main.daily_events WHERE "));
    assert!(insert_sql.starts_with("INSERT INTO main.daily_events "));

    // The chunk info names the real window.
    let (idx, total, start, end) = chunk.as_ref().expect("chunk info present");
    assert_eq!((*idx, *total), (0, 1));
    assert_eq!((start.as_str(), end.as_str()), ("2024-01-01", "2024-01-03"));

    // Re-derive the emitter output from the reported group's own region + body
    // and assert byte-equality: the reported statements come from the emitter,
    // not an independently-authored string.
    let where_clause = delete_sql
        .strip_prefix("DELETE FROM main.daily_events WHERE ")
        .expect("delete shape");
    let parts: Vec<&str> = where_clause.split(" AND ").collect();
    let start_lit = parts[0]
        .strip_prefix("event_date >= ")
        .expect("start literal");
    let end_lit = parts[1].strip_prefix("event_date < ").expect("end literal");
    let body = insert_sql
        .strip_prefix("INSERT INTO main.daily_events ")
        .expect("insert shape");
    let expected = emit_delete_insert(
        "main.daily_events",
        "event_date",
        &Region {
            start: start_lit.to_string(),
            end: end_lit.to_string(),
        },
        body,
        MaintenanceDialect::DuckDb,
    );
    assert_eq!(
        &expected, group,
        "dry-run reported group must be byte-identical to a direct emitter call over the same inputs"
    );
}

/// A dry-run creates no backend and materialises no target table: the target
/// DuckDB file is never created, and the panic backend factory is never
/// invoked (`docs/specs/cli.md` §"`--dry-run` prints the maintenance
/// statements": "never executes a statement against the target").
#[tokio::test]
async fn dry_run_executes_nothing() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project_dir = tmp.path();
    let db_path = project_dir.join("run.duckdb");
    let config = write_project(project_dir, &db_path);

    let (db, graph) = build_db_and_graph(project_dir, &config);

    execute_project(
        "dry-run-nothing".to_string(),
        dry_run_request("2024-01-01", "2024-01-03"),
        Arc::clone(&config),
        graph,
        db,
        project_dir,
        &PanicBackendFactory,
        &smelt_runtime::NoOpReporter,
        CancellationToken::new(),
    )
    .await
    .expect("dry-run execute_project");

    assert!(
        !db_path.exists(),
        "dry-run must not create the target database file: {}",
        db_path.display()
    );

    // Belt-and-braces: opening the DB now shows the target table was never
    // created (the file only appears here because we open it).
    let backend = smelt_backend_duckdb::DuckDbBackend::new(&db_path, "main")
        .await
        .expect("open duckdb after dry-run");
    assert!(
        !backend
            .table_exists("main", "daily_events")
            .await
            .unwrap_or(false),
        "dry-run must not materialise the target table"
    );
}
