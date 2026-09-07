//! Named-diagnostic coverage for the window-forward keyed run window
//! requirement (`docs/specs/incremental_shapes.md` §"The key grain"): a
//! window-forward keyed run (a clocked driving source — not the
//! snapshot-reconcile shape) started with no event-time window must refuse
//! instead of silently drop-and-recreating the target from a whole-source
//! SELECT. `--full-refresh` remains the intentional rebuild escape.
//!
//! Fixture mirrors `keyed_reprocessed_window_refusal.rs`'s `device_daily`
//! model (clocked source + `timeseries:`, `SUM` combiner).

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

fn stage_window_forward_project(project_dir: &Path, db_path: &Path) {
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
    SUM(amount) AS total_amount
FROM smelt.sources.events
GROUP BY 1, 2
"#;
    std::fs::write(project_dir.join("models/device_daily.sql"), model_sql).unwrap();

    let smelt_yml = format!(
        "name: keyed_window_required_test\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: {db}\n    schema: main\ndefault_materialization: table\n",
        db = db_path.display()
    );
    std::fs::write(project_dir.join("smelt.yml"), smelt_yml).unwrap();
}

fn stage_snapshot_reconcile_project(project_dir: &Path, db_path: &Path) {
    std::fs::create_dir_all(project_dir.join("models/sources")).unwrap();

    // No `timeseries:` on the source — clockless driving source derives the
    // snapshot-reconcile run shape (no run window axis at all).
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
        "name: keyed_snapshot_reconcile_test\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: {db}\n    schema: main\ndefault_materialization: table\n",
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
            (2, DATE '2026-01-01', 5.0)
        ) AS t(device_id, event_date, amount);
        "#,
    )?;
    Ok(())
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

fn base_request() -> ExecuteRequest {
    ExecuteRequest {
        target: "dev".to_string(),
        select: vec![],
        exclude: vec![],
        start: None,
        end: None,
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
    }
}

fn table_exists(db_path: &Path, table: &str) -> bool {
    let conn = duckdb::Connection::open(db_path).unwrap();
    let count: i64 = conn
        .query_row(
            "SELECT count(*) FROM information_schema.tables WHERE table_schema = 'main' AND table_name = ?",
            [table],
            |r| r.get(0),
        )
        .unwrap();
    count > 0
}

/// A window-forward keyed run with no `--event-time-start`/`--event-time-end`
/// refuses instead of drop+recreating the target; the target table is not
/// created.
#[tokio::test]
async fn window_forward_keyed_run_without_window_refuses() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().to_path_buf();
    let db_path = project_dir.join("dev.duckdb");

    stage_window_forward_project(&project_dir, &db_path);
    seed_events(&db_path).expect("seed events");

    let config = Arc::new(Config::load(&project_dir).expect("load config"));
    let (db, graph) = build_db_and_graph(&project_dir, &config);

    let err = execute_project(
        "keyed-window-required-none".to_string(),
        base_request(),
        Arc::clone(&config),
        Arc::clone(&graph),
        Arc::clone(&db),
        &project_dir,
        &PlainDuckDbFactory {
            db_path: db_path.clone(),
        },
        &smelt_runtime::NoOpReporter,
        CancellationToken::new(),
    )
    .await
    .expect_err("windowless window-forward keyed run must refuse");

    let message = format!("{err:#}");
    assert!(
        message.contains("--event-time-start") && message.contains("--event-time-end"),
        "refusal must name both required flags: {message}"
    );
    assert!(
        message.contains("--full-refresh"),
        "refusal must point at the --full-refresh escape: {message}"
    );
    assert!(
        !table_exists(&db_path, "device_daily"),
        "target table must not be created by a refused run"
    );
}

/// One flag alone (no matching pair) refuses at the generic run-window
/// parse gate, before any per-model dispatch.
#[tokio::test]
async fn window_forward_keyed_run_with_only_start_refuses() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().to_path_buf();
    let db_path = project_dir.join("dev.duckdb");

    stage_window_forward_project(&project_dir, &db_path);
    seed_events(&db_path).expect("seed events");

    let config = Arc::new(Config::load(&project_dir).expect("load config"));
    let (db, graph) = build_db_and_graph(&project_dir, &config);

    let mut request = base_request();
    request.start = Some("2026-01-01".to_string());

    let err = execute_project(
        "keyed-window-required-partial".to_string(),
        request,
        Arc::clone(&config),
        Arc::clone(&graph),
        Arc::clone(&db),
        &project_dir,
        &PlainDuckDbFactory {
            db_path: db_path.clone(),
        },
        &smelt_runtime::NoOpReporter,
        CancellationToken::new(),
    )
    .await
    .expect_err("a lone --event-time-start must refuse");

    let message = format!("{err:#}");
    assert!(
        message.contains("start") && message.contains("end"),
        "refusal must name that both flags are required together: {message}"
    );
}

/// `--full-refresh` still drop+creates a window-forward keyed model with no
/// event-time window — the intentional rebuild escape survives.
#[tokio::test]
async fn window_forward_keyed_run_with_full_refresh_flag_rebuilds() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().to_path_buf();
    let db_path = project_dir.join("dev.duckdb");

    stage_window_forward_project(&project_dir, &db_path);
    seed_events(&db_path).expect("seed events");

    let config = Arc::new(Config::load(&project_dir).expect("load config"));
    let (db, graph) = build_db_and_graph(&project_dir, &config);

    let mut request = base_request();
    request.full_refresh = true;

    execute_project(
        "keyed-window-required-full-refresh".to_string(),
        request,
        Arc::clone(&config),
        Arc::clone(&graph),
        Arc::clone(&db),
        &project_dir,
        &PlainDuckDbFactory {
            db_path: db_path.clone(),
        },
        &smelt_runtime::NoOpReporter,
        CancellationToken::new(),
    )
    .await
    .expect("--full-refresh must still rebuild the target");

    assert!(
        table_exists(&db_path, "device_daily"),
        "--full-refresh must create the target table"
    );
}

/// Regression guard: the snapshot-reconcile arm above the new refusal is
/// unaffected — a clockless driving source's windowless keyed run still
/// executes (whole-source keyed MERGE), never refused by the new check.
#[tokio::test]
async fn snapshot_reconcile_keyed_run_without_window_still_runs() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().to_path_buf();
    let db_path = project_dir.join("dev.duckdb");

    stage_snapshot_reconcile_project(&project_dir, &db_path);
    seed_devices(&db_path).expect("seed devices");

    let config = Arc::new(Config::load(&project_dir).expect("load config"));
    let (db, graph) = build_db_and_graph(&project_dir, &config);

    execute_project(
        "keyed-snapshot-reconcile-none".to_string(),
        base_request(),
        Arc::clone(&config),
        Arc::clone(&graph),
        Arc::clone(&db),
        &project_dir,
        &PlainDuckDbFactory {
            db_path: db_path.clone(),
        },
        &smelt_runtime::NoOpReporter,
        CancellationToken::new(),
    )
    .await
    .expect("snapshot-reconcile keyed run with no window must still execute");

    assert!(
        table_exists(&db_path, "device_snapshot"),
        "snapshot-reconcile run must create the target table"
    );
}
