//! Real end-to-end proof for the succession-patch technique
//! (`docs/outcomes/20260906-scd2-keyed-succession/phases/05b-plan.md`,
//! tests 10-11): drives `tests/fixtures/succession/models/customer_history.sql`
//! through `execute_project` itself — the sanctioned single run entrypoint,
//! root `CLAUDE.md` §"Run pipeline parity rule" — never a direct call to
//! `resolve_live_succession_cell`/`execute_succession_maintenance`, mirroring
//! `column_scoped_merge_e2e.rs`'s own harness pattern.

use std::path::Path;
use std::sync::Arc;

use smelt_backend::Backend;
use smelt_backend_duckdb::DuckDbBackend;
use smelt_core::config::Config;
use smelt_core::graph::DependencyGraph;
use smelt_core::ModelDiscovery;
use smelt_runtime::execute::{BackendFactory, BackendFuture};
use smelt_runtime::types::ExecuteRequest;
use smelt_runtime::{execute_project, NoOpReporter};
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

fn copy_dir_recursive(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).expect("create dst dir");
    for entry in std::fs::read_dir(src).expect("read src dir") {
        let entry = entry.expect("dir entry");
        let file_type = entry.file_type().expect("file type");
        let dst_path = dst.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_recursive(&entry.path(), &dst_path);
        } else {
            std::fs::copy(entry.path(), &dst_path).expect("copy file");
        }
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

fn request(start: &str, end: &str) -> ExecuteRequest {
    ExecuteRequest {
        target: "dev".to_string(),
        select: vec!["customer_history".to_string()],
        exclude: vec![],
        start: Some(start.to_string()),
        end: Some(end.to_string()),
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
        retry_max: Some(0),
        retry_backoff_ms: Some(0),
        resume: false,
        technique_overrides: vec![],
    }
}

/// The fixture's own source table's physical name
/// (`SourceInfo::db_name_for_target`'s default mapping: `<schema>.<address
/// segments joined with _>`, i.e. `main.sources_customer_changes` for
/// `models/sources/customer_changes.yml`'s `["sources", "customer_changes"]`
/// address).
const SOURCE_TABLE: &str = "main.sources_customer_changes";

fn setup_project() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
    let source_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/succession")
        .canonicalize()
        .expect("tests/fixtures/succession exists");
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().join("project");
    copy_dir_recursive(&source_dir, &project_dir);
    let db_path = tmp.path().join("run.duckdb");
    (tmp, project_dir, db_path)
}

fn stage_source(db_path: &Path) {
    let conn = duckdb::Connection::open(db_path).expect("open duckdb");
    conn.execute_batch(&format!(
        "CREATE TABLE {SOURCE_TABLE} (customer_id INTEGER, changed_at TIMESTAMP, arrival_date \
         DATE, tier VARCHAR)"
    ))
    .expect("create source table");
}

fn insert_event(db_path: &Path, id: i64, changed_at: &str, arrival_date: &str, tier: &str) {
    let conn = duckdb::Connection::open(db_path).expect("reopen duckdb");
    conn.execute_batch(&format!(
        "INSERT INTO {SOURCE_TABLE} VALUES ({id}, TIMESTAMP '{changed_at}', DATE \
         '{arrival_date}', '{tier}')"
    ))
    .expect("insert event");
}

fn row_count(db_path: &Path, sql: &str) -> i64 {
    let conn = duckdb::Connection::open(db_path).expect("reopen duckdb");
    conn.query_row(&format!("SELECT count(*) FROM ({sql}) AS t"), [], |r| {
        r.get(0)
    })
    .expect("row count")
}

async fn run(project_dir: &Path, db_path: &Path, config: &Arc<Config>, start: &str, end: &str) {
    let backend_factory = DuckDbBackendFactory {
        db_path: db_path.to_path_buf(),
    };
    let (db, graph) = build_db_and_graph(project_dir, config);
    execute_project(
        format!("run-{start}"),
        request(start, end),
        Arc::clone(config),
        graph,
        db,
        project_dir,
        &backend_factory,
        &NoOpReporter,
        CancellationToken::new(),
    )
    .await
    .unwrap_or_else(|e| panic!("execute_project run [{start}, {end}) failed: {e}"));
}

/// Test 10: `succession_model_runs_through_execute_project` — two
/// `execute_project` runs over consecutive windows produce exactly the
/// model SQL's own full-refresh result.
#[tokio::test]
async fn succession_model_runs_through_execute_project() {
    let (_tmp, project_dir, db_path) = setup_project();
    let config = Arc::new(Config::load(&project_dir).expect("load smelt.yml"));
    stage_source(&db_path);
    insert_event(&db_path, 1, "2026-01-01 08:00:00", "2026-01-01", "gold");
    insert_event(&db_path, 1, "2026-01-02 08:00:00", "2026-01-02", "silver");

    run(&project_dir, &db_path, &config, "2026-01-01", "2026-01-02").await;
    run(&project_dir, &db_path, &config, "2026-01-02", "2026-01-03").await;

    let oracle = format!(
        "SELECT customer_id, changed_at, tier, \
         LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at) AS valid_to FROM \
         {SOURCE_TABLE}"
    );
    let diff = row_count(
        &db_path,
        &format!(
            "(SELECT customer_id, changed_at, tier, valid_to FROM main.customer_history) EXCEPT \
             ALL ({oracle})"
        ),
    );
    assert_eq!(
        diff, 0,
        "the maintained table must match the model SQL's own full-refresh oracle"
    );
    let reverse_diff = row_count(
        &db_path,
        &format!(
            "({oracle}) EXCEPT ALL (SELECT customer_id, changed_at, tier, valid_to FROM \
             main.customer_history)"
        ),
    );
    assert_eq!(reverse_diff, 0);
}

/// Test 11: `late_event_in_a_later_arrival_window_splices` — an old
/// event-time value arriving in a LATER arrival window repairs its
/// neighbour's `valid_to`: the first run sees only customer 1's initial
/// event (no successor, `valid_to` NULL); the second run's arrival window
/// delivers an event whose OWN `changed_at` is earlier than a value that
/// would otherwise need no repair, proving the window-forward driver
/// recomputes neighbours from the union of presented rows and the new
/// batch rather than only ever appending.
#[tokio::test]
async fn late_event_in_a_later_arrival_window_splices() {
    let (_tmp, project_dir, db_path) = setup_project();
    let config = Arc::new(Config::load(&project_dir).expect("load smelt.yml"));
    stage_source(&db_path);
    // Day 1 arrival: only the LATER event (2026-01-03) arrives first.
    insert_event(&db_path, 1, "2026-01-03 08:00:00", "2026-01-01", "silver");

    run(&project_dir, &db_path, &config, "2026-01-01", "2026-01-02").await;

    let valid_to_before: Option<String> = {
        let conn = duckdb::Connection::open(&db_path).expect("reopen");
        conn.query_row(
            "SELECT CAST(valid_to AS VARCHAR) FROM main.customer_history WHERE customer_id = 1 \
             AND changed_at = TIMESTAMP '2026-01-03 08:00:00'",
            [],
            |r| r.get(0),
        )
        .expect("read valid_to before")
    };
    assert!(
        valid_to_before.is_none(),
        "the only event so far has no successor: {valid_to_before:?}"
    );

    // Day 2 arrival: an EARLIER event-time (2026-01-01) lands late, in a
    // LATER arrival window.
    insert_event(&db_path, 1, "2026-01-01 08:00:00", "2026-01-02", "gold");
    run(&project_dir, &db_path, &config, "2026-01-02", "2026-01-03").await;

    let conn = duckdb::Connection::open(&db_path).expect("reopen");
    let valid_to_after: String = conn
        .query_row(
            "SELECT CAST(valid_to AS VARCHAR) FROM main.customer_history WHERE customer_id = 1 \
             AND changed_at = TIMESTAMP '2026-01-01 08:00:00'",
            [],
            |r| r.get(0),
        )
        .expect("read valid_to after");
    assert_eq!(
        valid_to_after, "2026-01-03 08:00:00",
        "the late-arriving earlier event must splice in as the predecessor of the already-\
         presented row"
    );
    let row_count_total: i64 = conn
        .query_row("SELECT count(*) FROM main.customer_history", [], |r| {
            r.get(0)
        })
        .expect("row count");
    assert_eq!(row_count_total, 2);
}
