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

/// `BackendFactory` that always opens the same on-disk DuckDB file,
/// mirroring `crates/smelt-runtime/tests/execute_parity.rs`'s harness.
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

/// Copy `examples/timeseries` into a scratch directory so the run's
/// `.smelt/` state (`FileStore::new(project_dir, target)`) never lands
/// inside the checked-in example.
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

fn request_for_day() -> ExecuteRequest {
    ExecuteRequest {
        target: "dev".to_string(),
        select: vec!["daily_events_enriched".to_string()],
        exclude: vec![],
        start: Some("2025-01-10".to_string()),
        end: Some("2025-01-11".to_string()),
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
    }
}

/// First run creates the target via the normal `Trigger::NewData`
/// region-recompute path (the table doesn't exist yet). A dimension
/// mutation is then applied directly to the staged `raw.users` source
/// table, and a SECOND `execute_project` call over the SAME window must
/// pick up the mutated dimension value.
///
/// `daily_events_enriched.sql` reads `raw.users` in its join's `ON`
/// predicate — a row-admission read — so the `{user_name}` group is
/// membership-sensitive (`docs/plans/20260808-membership-sensitivity.md`
/// Phase 1) and its `Trigger::UpstreamMutation` cell admits
/// `Technique::DeleteInsert`, never `ColumnScopedMerge`
/// (`real_fixture_examples_timeseries_admits_membership_recompute_cell`
/// above proves the derivation). This is a `grain: partition` output
/// with `WholeRow` row identity (no declared `unique_key`) — since
/// `docs/outcomes/20260815-definition-delta-migrate/phases/27c-plan.md`
/// this shape dispatches the keyless (whole-row) staged-candidate
/// conditional write (`maintenance_driver::execute_staged_keyless_
/// recompute`), reported as `RunOutcome.models["daily_events_enriched"]
/// .strategy == "delete_insert_suppressed"` on any run where the
/// dimension mutation is live (never on the creation run, which has no
/// prior state to diff against).
#[tokio::test]
async fn membership_recompute_dispatches_through_execute_project() {
    let source_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/timeseries")
        .canonicalize()
        .expect("examples/timeseries exists");

    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().join("project");
    copy_dir_recursive(&source_dir, &project_dir);

    let db_path = tmp.path().join("run.duckdb");
    let config = Arc::new(Config::load(&project_dir).expect("load smelt.yml"));
    let backend_factory = DuckDbBackendFactory {
        db_path: db_path.clone(),
    };

    // Stage the two source tables `execute_project` reads —
    // `smelt.sources.raw.events` / `smelt.sources.raw.users` resolve to
    // `main.sources_raw_events` / `main.sources_raw_users` under the
    // unified default source-name mapping (no `name:` override in
    // either source YAML) — directly via raw SQL. The CSV seed loader
    // is a separate CLI-level step `execute_project` itself does not
    // perform.
    {
        let backend = DuckDbBackend::new(&db_path, "main")
            .await
            .expect("open duckdb");
        backend
            .execute_sql(
                "CREATE TABLE main.sources_raw_events (event_id INTEGER, user_id INTEGER, \
                 event_type VARCHAR, event_timestamp TIMESTAMP)",
            )
            .await
            .expect("create events source table");
        backend
            .execute_sql(
                "INSERT INTO main.sources_raw_events VALUES \
                 (1, 1, 'login', TIMESTAMP '2025-01-10 08:00:00'), \
                 (2, 2, 'login', TIMESTAMP '2025-01-10 09:00:00')",
            )
            .await
            .expect("seed events");
        backend
            .execute_sql(
                "CREATE TABLE main.sources_raw_users (user_id INTEGER, user_name VARCHAR, \
                 signup_date DATE)",
            )
            .await
            .expect("create users source table");
        backend
            .execute_sql(
                "INSERT INTO main.sources_raw_users VALUES \
                 (1, 'Alice', DATE '2025-01-01'), (2, 'Bob', DATE '2025-01-02')",
            )
            .await
            .expect("seed users");
    }

    {
        let (db, graph) = build_db_and_graph(&project_dir, &config);
        let outcome = execute_project(
            "run-1".to_string(),
            request_for_day(),
            Arc::clone(&config),
            graph,
            db,
            &project_dir,
            &backend_factory,
            &NoOpReporter,
            CancellationToken::new(),
        )
        .await
        .expect("first run must succeed");
        let record = outcome
            .models
            .get("daily_events_enriched")
            .expect("daily_events_enriched ran");
        assert_eq!(
            record.strategy, "deleteinsert",
            "the creation run takes the region-recompute path (`Trigger::NewData`); this \
             fixture's membership-sensitive cell never reaches column-scoped MERGE"
        );
    }

    // Mutate the dimension in place — `raw.users` is declared
    // `mutation_profile: mutable_snapshot`; renaming user 1 broadcasts
    // to every fact row referencing them (the `{user_name}` group).
    {
        let backend = DuckDbBackend::new(&db_path, "main")
            .await
            .expect("reopen duckdb");
        backend
            .execute_sql("UPDATE main.sources_raw_users SET user_name = 'Alicia' WHERE user_id = 1")
            .await
            .expect("mutate dimension");
    }

    let (db, graph) = build_db_and_graph(&project_dir, &config);
    let outcome = execute_project(
        "run-2".to_string(),
        request_for_day(),
        Arc::clone(&config),
        graph,
        db,
        &project_dir,
        &backend_factory,
        &NoOpReporter,
        CancellationToken::new(),
    )
    .await
    .expect("second run must succeed");
    let record = outcome
        .models
        .get("daily_events_enriched")
        .expect("daily_events_enriched ran");
    assert_eq!(
        record.strategy, "delete_insert_suppressed",
        "a membership-sensitive dimension mutation over a WholeRow-identity cell dispatches \
         the keyless staged-candidate conditional write — never column-scoped MERGE, which \
         cannot repair which rows exist, and never the plain unconditional DELETE+INSERT \
         now that a live dispatch exists for this shape"
    );

    let conn = duckdb::Connection::open(&db_path).expect("reconnect");
    let maintained_user_name: String = conn
        .query_row(
            "SELECT user_name FROM main.daily_events_enriched WHERE user_id = 1 LIMIT 1",
            [],
            |row| row.get(0),
        )
        .expect("read maintained user_name");
    assert_eq!(
        maintained_user_name, "Alicia",
        "the region recompute must pick up the mutated dimension value"
    );

    let untouched_user_name: String = conn
        .query_row(
            "SELECT user_name FROM main.daily_events_enriched WHERE user_id = 2 LIMIT 1",
            [],
            |row| row.get(0),
        )
        .expect("read untouched user_name");
    assert_eq!(
        untouched_user_name, "Bob",
        "an unmutated dimension row's enrichment must be unchanged"
    );
}
