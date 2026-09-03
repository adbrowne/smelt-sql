//! Merge-ledger bookkeeping for re-run-tolerant (`Grade::Idempotent`)
//! window-forward keyed models (`docs/outcomes/20260815-keyed-grain-residue/
//! phases/02-plan.md`; `docs/specs/incremental_shapes.md` §"The
//! transactional frontier write (merge ledger)" — "every window-forward
//! keyed model maintains a per-model frontier", unqualified by grading).
//!
//! Fixture: `device_daily` is a `grain: key` model with a `MAX` combiner —
//! `WindowedKeyedRule::ledger_grade` grades a lattice combiner like `MAX`
//! `Grade::Idempotent` (`crates/smelt-runtime/src/cumulative.rs`), so a
//! merge for this model was, before this phase, never recorded in the
//! reconciliation ledger at all. This suite proves, against a real DuckDB
//! backend through the real `execute_project` pipeline
//! (`docs/specs/architecture.md` §"Run pipeline parity rule"):
//! - every merged window (including the table-creating first step) leaves a
//!   ledger row;
//! - re-running an already-recorded window is a silent no-op — never a
//!   `KeyedReprocessedWindow` refusal, unlike the additive-graded case;
//! - a snapshot-reconcile keyed model (a different execution path entirely)
//!   writes no frontier record at all.

use std::path::Path;
use std::sync::Arc;

use smelt_backend::Backend;
use smelt_backend_duckdb::DuckDbBackend;
use smelt_core::config::{Config, Target};
use smelt_core::graph::DependencyGraph;
use smelt_core::ModelDiscovery;
use smelt_runtime::execute::{execute_project, BackendFactory, BackendFuture};
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

fn stage_idempotent_project(project_dir: &Path, db_path: &Path) {
    std::fs::create_dir_all(project_dir.join("models/sources")).unwrap();

    let source_yml = r#"description: Raw per-device events.
columns:
  - name: device_id
    type: INTEGER
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
"#;
    std::fs::write(project_dir.join("models/sources/events.yml"), source_yml).unwrap();

    // `MAX` combiner => a lattice combiner => `Grade::Idempotent`
    // (`WindowedKeyedRule::ledger_grade`'s doc comment).
    let model_sql = r#"---
materialization: table
refresh: incremental
grain: key
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
SELECT
    device_id,
    event_date,
    MAX(amount) AS max_amount
FROM smelt.sources.events
GROUP BY 1, 2
"#;
    std::fs::write(project_dir.join("models/device_daily.sql"), model_sql).unwrap();

    let smelt_yml = format!(
        "name: keyed_frontier_bookkeeping_test\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: {db}\n    schema: main\ndefault_materialization: table\n",
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
            (1, DATE '2026-01-01', 10.0),
            (2, DATE '2026-01-01', 5.0),
            (1, DATE '2026-01-02', 20.0),
            (2, DATE '2026-01-02', 8.0)
        ) AS t(device_id, event_date, amount);
        "#,
    )?;
    Ok(())
}

fn stage_snapshot_reconcile_project(project_dir: &Path, db_path: &Path) {
    std::fs::create_dir_all(project_dir.join("models/sources")).unwrap();

    let source_yml = r#"description: Raw per-device rows, no clock.
columns:
  - name: device_id
    type: INTEGER
  - name: amount
    type: DOUBLE
mutation_profile:
  kind: mutable_snapshot
"#;
    std::fs::write(project_dir.join("models/sources/devices.yml"), source_yml).unwrap();

    let model_sql = r#"---
materialization: table
refresh: incremental
grain: key
maintenance:
  scan_bounds:
    per_source:
      devices:
        allow_full_scan: true
---
SELECT
    device_id,
    ANY_VALUE(amount) AS amount
FROM smelt.sources.devices
GROUP BY 1
"#;
    std::fs::write(project_dir.join("models/device_snapshot.sql"), model_sql).unwrap();

    let smelt_yml = format!(
        "name: keyed_frontier_bookkeeping_snapshot_test\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: {db}\n    schema: main\ndefault_materialization: table\n",
        db = db_path.display()
    );
    std::fs::write(project_dir.join("smelt.yml"), smelt_yml).unwrap();
}

fn seed_devices(db_path: &Path) -> anyhow::Result<()> {
    let conn = duckdb::Connection::open(db_path)?;
    conn.execute_batch(
        r#"
        CREATE SCHEMA IF NOT EXISTS main;
        CREATE OR REPLACE TABLE main.sources_devices AS
        SELECT * FROM (VALUES
            (1, 10.0),
            (2, 5.0)
        ) AS t(device_id, amount);
        "#,
    )?;
    Ok(())
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

fn run_request(start: Option<&str>, end: Option<&str>) -> ExecuteRequest {
    ExecuteRequest {
        target: "dev".to_string(),
        select: vec![],
        exclude: vec![],
        start: start.map(str::to_string),
        end: end.map(str::to_string),
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

async fn run(
    project_dir: &Path,
    db_path: &Path,
    run_id: &str,
    config: &Arc<Config>,
    db: &Arc<tokio::sync::Mutex<smelt_db::Database>>,
    graph: &Arc<tokio::sync::Mutex<DependencyGraph>>,
    request: ExecuteRequest,
) -> anyhow::Result<()> {
    execute_project(
        run_id.to_string(),
        request,
        Arc::clone(config),
        Arc::clone(graph),
        Arc::clone(db),
        project_dir,
        &PlainDuckDbFactory {
            db_path: db_path.to_path_buf(),
        },
        &smelt_runtime::NoOpReporter,
        CancellationToken::new(),
    )
    .await
    .map(|_| ())
}

fn ledger_row_count(db_path: &Path, model: &str) -> i64 {
    let conn = duckdb::Connection::open(db_path).unwrap();
    conn.query_row(
        "SELECT COUNT(*) FROM main._smelt_ledger WHERE model_name = ?",
        [model],
        |r| r.get(0),
    )
    .unwrap()
}

/// Every merged window — including the table-creating first step — leaves a
/// ledger row keyed `(model, whole-row group, input, partition)`.
#[tokio::test]
async fn idempotent_keyed_model_records_every_merged_window() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().to_path_buf();
    let db_path = project_dir.join("dev.duckdb");

    stage_idempotent_project(&project_dir, &db_path);
    seed_events(&db_path).expect("seed events");

    let config = Arc::new(Config::load(&project_dir).expect("load config"));
    let (db, graph) = build_db_and_graph(&project_dir, &config);

    run(
        &project_dir,
        &db_path,
        "idempotent-keyed-frontier",
        &config,
        &db,
        &graph,
        run_request(Some("2026-01-01"), Some("2026-01-03")),
    )
    .await
    .expect("two-day run over an idempotent keyed model succeeds");

    assert_eq!(
        ledger_row_count(&db_path, "device_daily"),
        2,
        "each of the two merged day partitions must leave its own ledger row"
    );
}

/// Re-running an already-recorded window is a silent no-op (`ON CONFLICT DO
/// NOTHING`) — never the `KeyedReprocessedWindow` refusal the additive-graded
/// case gets, and the ledger row count does not grow.
#[tokio::test]
async fn re_running_a_recorded_window_is_a_no_op_not_a_refusal() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().to_path_buf();
    let db_path = project_dir.join("dev.duckdb");

    stage_idempotent_project(&project_dir, &db_path);
    seed_events(&db_path).expect("seed events");

    let config = Arc::new(Config::load(&project_dir).expect("load config"));
    let (db, graph) = build_db_and_graph(&project_dir, &config);

    run(
        &project_dir,
        &db_path,
        "idempotent-keyed-frontier-first",
        &config,
        &db,
        &graph,
        run_request(Some("2026-01-01"), Some("2026-01-03")),
    )
    .await
    .expect("first run succeeds");

    let count_after_first = ledger_row_count(&db_path, "device_daily");
    assert_eq!(count_after_first, 2);

    // Re-run over the SAME range — must succeed (not refuse) and must not
    // grow the ledger row count.
    run(
        &project_dir,
        &db_path,
        "idempotent-keyed-frontier-second",
        &config,
        &db,
        &graph,
        run_request(Some("2026-01-01"), Some("2026-01-03")),
    )
    .await
    .expect("re-running an already-recorded window must succeed, not refuse");

    assert_eq!(
        ledger_row_count(&db_path, "device_daily"),
        count_after_first,
        "re-merging an already-recorded window must not grow the ledger row count"
    );
}

/// A snapshot-reconcile keyed model (`execute_snapshot_reconcile` —
/// `crates/smelt-runtime/src/cumulative.rs`) does not run through
/// `run_windowed_keyed_maintenance` at all, so it writes no frontier record
/// — the ledger table is never even created for a project containing only
/// this model.
#[tokio::test]
async fn snapshot_reconcile_model_writes_no_frontier_record() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().to_path_buf();
    let db_path = project_dir.join("dev.duckdb");

    stage_snapshot_reconcile_project(&project_dir, &db_path);
    seed_devices(&db_path).expect("seed devices");

    let config = Arc::new(Config::load(&project_dir).expect("load config"));
    let (db, graph) = build_db_and_graph(&project_dir, &config);

    run(
        &project_dir,
        &db_path,
        "snapshot-reconcile-frontier",
        &config,
        &db,
        &graph,
        run_request(None, None),
    )
    .await
    .expect("snapshot-reconcile run succeeds");

    let conn = duckdb::Connection::open(&db_path).unwrap();
    let ledger_exists: bool = conn
        .query_row(
            "SELECT COUNT(*) > 0 FROM information_schema.tables \
             WHERE table_schema = 'main' AND table_name = '_smelt_ledger'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        !ledger_exists,
        "a snapshot-reconcile keyed model must never create the merge ledger table"
    );
}
