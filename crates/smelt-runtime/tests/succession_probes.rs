//! Proof that a succession-patch run verifies its driving source's declared
//! append-only posture on the SAME terms as the ordinary `plan.incremental`
//! dispatch sites (`docs/outcomes/20260906-scd2-keyed-succession/phases/
//! 06c-plan.md`): a late append into a closed partition is tolerated and
//! refreshes the baseline, while a genuine in-place mutation fails the run
//! loud with `SourceMutationProfileViolated` before either the presented
//! table or the tombstone ledger is touched. Reuses `succession_frontiers.
//! rs`'s fixture harness, appending its own `state:` block to the copied
//! `smelt.yml` rather than editing the shared fixture.

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
    // Same opt-in as `succession_frontiers.rs`: the shared fixture defaults
    // to `StateMode::Stateless` (no `.smelt/` writes), but these tests read
    // both `source_postures.json` and the run manifest back.
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

/// Mutate an existing row's payload IN PLACE (same key + `changed_at`,
/// row count unchanged) — the fingerprint-only mutation of a closed
/// partition the append-only posture probe must catch.
fn mutate_event_payload(db_path: &Path, id: i64, changed_at: &str, new_tier: &str) {
    let conn = duckdb::Connection::open(db_path).expect("reopen duckdb");
    let updated = conn
        .execute(
            &format!(
                "UPDATE {SOURCE_TABLE} SET tier = '{new_tier}' WHERE customer_id = {id} AND \
                 changed_at = TIMESTAMP '{changed_at}'"
            ),
            [],
        )
        .expect("mutate event");
    assert_eq!(updated, 1, "expected to mutate exactly one row");
}

async fn snapshot_rows(
    db_path: &Path,
    relation: &str,
) -> Vec<std::collections::BTreeMap<String, String>> {
    let backend = DuckDbBackend::new(db_path, "main")
        .await
        .expect("open duckdb backend");
    let batches = backend
        .execute_sql(&format!("SELECT * FROM {relation} ORDER BY ALL"))
        .await
        .expect("snapshot relation");
    smelt_runtime::check_runner::batches_to_rows(&batches)
}

async fn run(
    project_dir: &Path,
    db_path: &Path,
    config: &Arc<Config>,
    start: &str,
    end: &str,
    rebuild: bool,
    full_refresh: bool,
) -> anyhow::Result<smelt_runtime::types::RunOutcome> {
    let backend_factory = DuckDbBackendFactory {
        db_path: db_path.to_path_buf(),
    };
    let (db, graph) = build_db_and_graph(project_dir, config);
    execute_project(
        format!("run-{start}-{end}-{rebuild}-{full_refresh}"),
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
}

/// Test 1: after one window-forward run, `.smelt/` carries a
/// `SourcePostureStore` entry for the model's `append_only` source.
#[tokio::test]
async fn succession_run_establishes_the_source_posture_baseline() {
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
    .await
    .expect("run must succeed");

    let file_store = FileStore::new(&project_dir, "dev");
    let postures = file_store
        .load_source_postures()
        .expect("load source postures");
    let recorded = postures
        .get("customer_changes")
        .expect("customer_changes has a recorded posture baseline after a succession run");
    assert!(
        !recorded.partitions.is_empty(),
        "the baseline must record at least the one populated partition"
    );
}

/// Test 2: that run's `ModelRunRecord.probes` holds one
/// `SourceMutationProfileViolated` / `mutation_profile.kind: append_only`
/// record — no longer the hardcoded empty `Vec` `build_succession_run_record`
/// used to return.
#[tokio::test]
async fn succession_run_record_carries_the_append_only_probe() {
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
    .await
    .expect("run must succeed");

    let file_store = FileStore::new(&project_dir, "dev");
    let manifest = file_store
        .load_run("run-2026-01-01-2026-01-02-false-false")
        .expect("load run")
        .expect("run manifest exists");
    let record = manifest
        .models
        .get("customer_history")
        .expect("customer_history has a run record");
    assert_eq!(
        record.probes.len(),
        1,
        "expected exactly one probe record, got {:?}",
        record.probes
    );
    assert_eq!(record.probes[0].probe, "SourceMutationProfileViolated");
    assert_eq!(record.probes[0].fact, "mutation_profile.kind: append_only");
}

/// Test 3: an in-place mutation of a row in an already-baselined (closed)
/// partition fails the next run loud, before either the presented table or
/// the tombstone ledger is touched.
#[tokio::test]
async fn succession_in_place_mutation_of_a_closed_partition_fails_loud() {
    let (_tmp, project_dir, db_path) = setup_project();
    let config = Arc::new(Config::load(&project_dir).expect("load smelt.yml"));
    stage_source(&db_path);
    insert_event(&db_path, 1, "2026-01-01 08:00:00", "2026-01-01", "gold");
    insert_event(&db_path, 2, "2026-01-02 08:00:00", "2026-01-02", "silver");

    // One run whose baseline SNAPSHOT scans the whole source table
    // (`emit_append_only_baseline_snapshot`), so both 2026-01-01 and
    // 2026-01-02 are recorded even though the run window only steps
    // 2026-01-01 — 2026-01-01 is then strictly below the recorded maximum
    // partition value, so it is CLOSED.
    run(
        &project_dir,
        &db_path,
        &config,
        "2026-01-01",
        "2026-01-02",
        false,
        false,
    )
    .await
    .expect("first run must succeed and establish the baseline");

    let presented_before = snapshot_rows(&db_path, "main.customer_history").await;
    let tombstones_before = snapshot_rows(
        &db_path,
        &format!(
            "main.{}",
            smelt_logical::maintenance::emit::tombstone_table_name("customer_history")
        ),
    )
    .await;

    mutate_event_payload(&db_path, 1, "2026-01-01 08:00:00", "platinum");

    let err = run(
        &project_dir,
        &db_path,
        &config,
        "2026-01-02",
        "2026-01-03",
        false,
        false,
    )
    .await
    .expect_err("a mutated closed partition must fail the run");
    let message = err.to_string();
    assert!(
        message.contains("SourceMutationProfileViolated"),
        "expected SourceMutationProfileViolated, got: {message}"
    );
    assert!(
        !message.contains("SuccessionClockTie"),
        "the append-only posture probe must fire before the fold, not incidentally through the \
         clock-tie probe: {message}"
    );

    let presented_after = snapshot_rows(&db_path, "main.customer_history").await;
    let tombstones_after = snapshot_rows(
        &db_path,
        &format!(
            "main.{}",
            smelt_logical::maintenance::emit::tombstone_table_name("customer_history")
        ),
    )
    .await;
    assert_eq!(
        presented_before, presented_after,
        "the presented table must be untouched by a refused run"
    );
    assert_eq!(
        tombstones_before, tombstones_after,
        "the tombstone ledger must be untouched by a refused run"
    );
}

/// Test 4: a late append into a closed partition is tolerated — the run
/// succeeds and refreshes the baseline (never a
/// `SourceMutationProfileViolated`).
#[tokio::test]
async fn succession_late_append_into_a_closed_partition_is_tolerated() {
    let (_tmp, project_dir, db_path) = setup_project();
    let config = Arc::new(Config::load(&project_dir).expect("load smelt.yml"));
    stage_source(&db_path);
    insert_event(&db_path, 1, "2026-01-01 08:00:00", "2026-01-01", "gold");
    insert_event(&db_path, 2, "2026-01-02 08:00:00", "2026-01-02", "silver");

    run(
        &project_dir,
        &db_path,
        &config,
        "2026-01-01",
        "2026-01-02",
        false,
        false,
    )
    .await
    .expect("first run must succeed and establish the baseline");

    // A genuine late APPEND into the now-closed 2026-01-01 partition —
    // a new row, not a mutation of an existing one.
    insert_event(&db_path, 3, "2026-01-01 09:00:00", "2026-01-01", "bronze");

    run(
        &project_dir,
        &db_path,
        &config,
        "2026-01-01",
        "2026-01-02",
        false,
        false,
    )
    .await
    .expect("a late append into a closed partition must be tolerated, not violated");

    let file_store = FileStore::new(&project_dir, "dev");
    let postures = file_store
        .load_source_postures()
        .expect("load source postures");
    let recorded = postures
        .get("customer_changes")
        .expect("customer_changes still has a recorded posture baseline");
    let day1 = recorded
        .partitions
        .iter()
        .find(|p| p.partition_value == "2026-01-01")
        .expect("2026-01-01 partition recorded");
    assert_eq!(
        day1.recorded_count, 2,
        "the refreshed baseline must reflect the appended row"
    );
}

/// Test 5: the `--full-refresh` arm dispatches the same source posture
/// probe as the window-forward arm (parity with the ordinary full-refresh
/// site at `crate::execute::project`).
#[tokio::test]
async fn succession_full_rebuild_verifies_the_source_posture_too() {
    let (_tmp, project_dir, db_path) = setup_project();
    let config = Arc::new(Config::load(&project_dir).expect("load smelt.yml"));
    stage_source(&db_path);
    insert_event(&db_path, 1, "2026-01-01 08:00:00", "2026-01-01", "gold");
    insert_event(&db_path, 2, "2026-01-02 08:00:00", "2026-01-02", "silver");

    run(
        &project_dir,
        &db_path,
        &config,
        "2026-01-01",
        "2026-01-02",
        false,
        false,
    )
    .await
    .expect("first run must succeed and establish the baseline");

    mutate_event_payload(&db_path, 1, "2026-01-01 08:00:00", "platinum");

    let err = run(
        &project_dir,
        &db_path,
        &config,
        "2026-01-01",
        "2026-01-03",
        false,
        true,
    )
    .await
    .expect_err("a full refresh must also verify the source posture");
    assert!(
        err.to_string().contains("SourceMutationProfileViolated"),
        "expected SourceMutationProfileViolated, got: {err}"
    );
}
