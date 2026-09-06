//! Proof that the succession window-forward driver records the same
//! maintained-interval and landed-delta frontiers the ordinary incremental
//! path does (`docs/outcomes/20260906-scd2-keyed-succession/phases/
//! 06b-plan.md`, tests 1-3), and that the whole-source rebuild path leaves
//! both untouched. Reuses `technique_lowering/succession_patch_e2e.rs`'s
//! fixture (`tests/fixtures/succession`) and harness shape — a single
//! `customer_history` succession model driven through `execute_project`,
//! never the private resolve/execute functions directly (Run Pipeline
//! Parity).

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

fn request(start: &str, end: &str, rebuild: bool, full_refresh: bool) -> ExecuteRequest {
    ExecuteRequest {
        target: "dev".to_string(),
        select: vec!["customer_history".to_string()],
        exclude: vec![],
        start: Some(start.to_string()),
        end: Some(end.to_string()),
        batch_size_days: None,
        per_partition: false,
        full_refresh,
        rebuild,
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

const SOURCE_TABLE: &str = "main.sources_customer_changes";

fn setup_project() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
    let source_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/succession")
        .canonicalize()
        .expect("tests/fixtures/succession exists");
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().join("project");
    copy_dir_recursive(&source_dir, &project_dir);
    // The shared fixture's `smelt.yml` declares no `state:` block, which
    // defaults to `StateMode::Stateless` (no `.smelt/` writes at all) — fine
    // for `succession_patch_e2e.rs`, which never reads `.smelt/`, but these
    // tests are specifically about `.smelt/`'s interval/landed-delta
    // frontiers, so opt this copy into `intervals` mode without touching the
    // shared fixture file other tests use unmodified.
    let smelt_yml_path = project_dir.join("smelt.yml");
    let mut smelt_yml = std::fs::read_to_string(&smelt_yml_path).expect("read smelt.yml");
    smelt_yml.push_str("\nstate:\n  mode: intervals\n");
    std::fs::write(&smelt_yml_path, smelt_yml).expect("write smelt.yml");
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

async fn run(
    project_dir: &Path,
    db_path: &Path,
    config: &Arc<Config>,
    start: &str,
    end: &str,
    rebuild: bool,
    full_refresh: bool,
) {
    let backend_factory = DuckDbBackendFactory {
        db_path: db_path.to_path_buf(),
    };
    let (db, graph) = build_db_and_graph(project_dir, config);
    execute_project(
        format!("run-{start}"),
        request(start, end, rebuild, full_refresh),
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

/// Test 1: a completed window-forward run records `[start, end)` in
/// `intervals.json` under the model's own `compute_model_hash` key —
/// `contract_probes::resolve_deferral_frontiers`'s maintained frontier.
#[tokio::test]
async fn succession_run_records_its_maintained_interval() {
    let (_tmp, project_dir, db_path) = setup_project();
    let config = Arc::new(Config::load(&project_dir).expect("load smelt.yml"));
    stage_source(&db_path);
    insert_event(&db_path, 1, "2026-01-01 08:00:00", "2026-01-01", "gold");

    run(
        &project_dir,
        &db_path,
        &config,
        "2026-01-01",
        "2026-01-02",
        false,
        false,
    )
    .await;

    let file_store = FileStore::new(&project_dir, "dev");
    let interval_store = file_store.load_intervals().expect("load intervals");
    let maintained = interval_store
        .get("customer_history")
        .and_then(|mi| mi.latest_date())
        .expect("customer_history has a recorded interval after a window-forward run");
    assert_eq!(maintained.format("%Y-%m-%d").to_string(), "2026-01-02");
}

/// Test 2: the same run records the driving source's landing in
/// `landed_deltas.json` as an interval-diffed append-only landing (rather
/// than never being recorded at all) — the input frontier half of the same
/// read-back. `customer_changes` declares `mutation_profile: append_only`
/// with a `timeseries:` clock, so its landing is interval-diffed, never
/// `LandedDelta::WholeTable`.
#[tokio::test]
async fn succession_run_records_its_source_landing() {
    let (_tmp, project_dir, db_path) = setup_project();
    let config = Arc::new(Config::load(&project_dir).expect("load smelt.yml"));
    stage_source(&db_path);
    insert_event(&db_path, 1, "2026-01-01 08:00:00", "2026-01-01", "gold");

    run(
        &project_dir,
        &db_path,
        &config,
        "2026-01-01",
        "2026-01-02",
        false,
        false,
    )
    .await;

    let file_store = FileStore::new(&project_dir, "dev");
    let landed_deltas = file_store.load_landed_deltas().expect("load landed deltas");
    let landing = landed_deltas
        .get("customer_changes")
        .expect("the driving source has a recorded landing after a window-forward run");
    let latest = landing
        .covered_intervals
        .last()
        .expect("at least one covered interval");
    assert_eq!(latest.end, "2026-01-02");
}

/// Test 3: `--full-refresh` and `smelt rebuild` re-derive the whole source
/// (no run window exists to record) and must leave both stores untouched.
#[tokio::test]
async fn succession_rebuild_records_no_frontier() {
    let (_tmp, project_dir, db_path) = setup_project();
    let config = Arc::new(Config::load(&project_dir).expect("load smelt.yml"));
    stage_source(&db_path);
    insert_event(&db_path, 1, "2026-01-01 08:00:00", "2026-01-01", "gold");

    run(
        &project_dir,
        &db_path,
        &config,
        "2026-01-01",
        "2026-01-02",
        true,
        true,
    )
    .await;

    let file_store = FileStore::new(&project_dir, "dev");
    let interval_store = file_store.load_intervals().expect("load intervals");
    assert!(
        interval_store.get("customer_history").is_none(),
        "a rebuild has no run window and must not record a maintained interval"
    );
    let landed_deltas = file_store.load_landed_deltas().expect("load landed deltas");
    assert!(
        landed_deltas.get("customer_changes").is_none(),
        "a rebuild has no run window and must not record a source landing"
    );
}
