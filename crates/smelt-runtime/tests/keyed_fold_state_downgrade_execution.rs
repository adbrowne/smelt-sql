//! Gap 5 tests 6 and 7 (`docs/outcomes/20260913-trino-incremental/phases/
//! 03g-plan.md`): `state.warehouse_tables: none` on DuckDB is a
//! structure-less availability offline — the same availability shape Trino
//! has permanently — so it is the cheapest way to prove the plan-time
//! `FoldGrade` wiring end to end through the real `execute_project`
//! pipeline (`docs/specs/architecture.md` §"Run pipeline parity rule")
//! against a real DuckDB backend, without needing a live Trino coordinator.
//!
//! - An **additive** (`SUM`) keyed fold has no realisable reconciliation
//!   ledger under this availability and must downgrade to the whole-target
//!   rebuild every run (`resolve_keyed_fold_state_downgrade`'s dispatch in
//!   `execute/project/mod.rs`) — never reach the windowed-keyed driver's
//!   `Grade::Additive` guard, which would otherwise bail with
//!   `BackendError::unsupported`.
//! - An **idempotent** (`MAX`) keyed fold requires no correctness structure
//!   at all and keeps taking the keyed `MERGE` route unchanged.
//!
//! Both legs assert the target table equals a `--full-refresh` oracle
//! (the raw, unwindowed model SQL evaluated directly against DuckDB) after
//! every run step.

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

fn stage_project(project_dir: &Path, db_path: &Path, combiner_sql: &str) {
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

    let model_sql = format!(
        r#"---
materialization: table
refresh: incremental
grain: key
maintenance:
  scan_bounds:
    per_source:
      events:
        allow_full_scan: true
---
SELECT
    device_id,
    {combiner_sql} AS agg_amount
FROM smelt.sources.events
GROUP BY 1
"#
    );
    std::fs::write(project_dir.join("models/device_agg.sql"), model_sql).unwrap();

    let smelt_yml = format!(
        "name: keyed_fold_state_downgrade_test\nversion: 1\npaths:\n  - models\n\
         targets:\n  dev:\n    type: duckdb\n    database: {db}\n    schema: main\n\
         default_materialization: table\n\
         state:\n  warehouse_tables: none\n",
        db = db_path.display()
    );
    std::fs::write(project_dir.join("smelt.yml"), smelt_yml).unwrap();
}

fn seed_events(db_path: &Path, rows: &[(i64, &str, f64)]) {
    let conn = duckdb::Connection::open(db_path).unwrap();
    conn.execute_batch("CREATE SCHEMA IF NOT EXISTS main;")
        .unwrap();
    let values: Vec<String> = rows
        .iter()
        .map(|(id, date, amount)| format!("({id}, DATE '{date}', {amount})"))
        .collect();
    conn.execute_batch(&format!(
        "CREATE OR REPLACE TABLE main.sources_events AS
         SELECT * FROM (VALUES {}) AS t(device_id, event_date, amount);",
        values.join(", ")
    ))
    .unwrap();
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

fn run_request(start: &str, end: &str) -> ExecuteRequest {
    ExecuteRequest {
        target: "dev".to_string(),
        select: vec![],
        exclude: vec![],
        start: Some(start.to_string()),
        end: Some(end.to_string()),
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
        assume_external_steps_fresh: false,
    }
}

fn table_contents(db_path: &Path) -> Vec<(i64, f64)> {
    let conn = duckdb::Connection::open(db_path).unwrap();
    let mut stmt = conn
        .prepare("SELECT device_id, agg_amount FROM main.device_agg ORDER BY device_id")
        .unwrap();
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
}

fn full_refresh_oracle(db_path: &Path, combiner_sql: &str) -> Vec<(i64, f64)> {
    let conn = duckdb::Connection::open(db_path).unwrap();
    let query = format!(
        "SELECT device_id, {combiner_sql} AS agg_amount FROM main.sources_events \
         GROUP BY 1 ORDER BY 1"
    );
    let mut stmt = conn.prepare(&query).unwrap();
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
}

/// Gap 5 test 6: an additive (`SUM`) keyed fold, with `state.warehouse_tables:
/// none` denying the reconciliation ledger, runs to completion over two
/// windows and matches a `--full-refresh` oracle after each — the
/// windowed-keyed driver's `Grade::Additive` guard (which would bail with
/// `BackendError::unsupported`) is never reached because the plan layer
/// downgrades the cell to a whole-target rebuild before dispatch.
#[tokio::test]
async fn additive_keyed_fold_rebuilds_whole_target_under_warehouse_tables_none() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().to_path_buf();
    let db_path = project_dir.join("dev.duckdb");
    let combiner_sql = "SUM(amount)";

    stage_project(&project_dir, &db_path, combiner_sql);
    seed_events(&db_path, &[(1, "2026-01-01", 10.0), (2, "2026-01-01", 5.0)]);

    let config = Arc::new(Config::load(&project_dir).expect("load config"));
    let (db, graph) = build_db_and_graph(&project_dir, &config);

    execute_project(
        "additive-window-1".to_string(),
        run_request("2026-01-01", "2026-01-02"),
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
    .expect("window 1 run must succeed rather than hitting BackendError::unsupported");

    assert_eq!(
        table_contents(&db_path),
        full_refresh_oracle(&db_path, combiner_sql),
        "after window 1, the target must equal the full-refresh oracle"
    );

    // Window 2: new rows land for an existing device AND a brand new device.
    seed_events(
        &db_path,
        &[
            (1, "2026-01-01", 10.0),
            (2, "2026-01-01", 5.0),
            (1, "2026-01-02", 7.0),
            (3, "2026-01-02", 2.0),
        ],
    );

    execute_project(
        "additive-window-2".to_string(),
        run_request("2026-01-02", "2026-01-03"),
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
    .expect("window 2 run must succeed rather than hitting BackendError::unsupported");

    assert_eq!(
        table_contents(&db_path),
        full_refresh_oracle(&db_path, combiner_sql),
        "after window 2, the whole-target rebuild must equal the full-refresh oracle over ALL \
         events seen so far, not just window 2's own delta"
    );
}

/// Gap 5 test 7: an idempotent (`MAX`) keyed fold requires no correctness
/// structure and still takes the ordinary keyed `MERGE` route under the same
/// ledger-less availability — never downgraded — and matches the oracle
/// across two windows.
#[tokio::test]
async fn idempotent_keyed_fold_still_merges_under_warehouse_tables_none() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().to_path_buf();
    let db_path = project_dir.join("dev.duckdb");
    let combiner_sql = "MAX(amount)";

    stage_project(&project_dir, &db_path, combiner_sql);
    seed_events(&db_path, &[(1, "2026-01-01", 10.0), (2, "2026-01-01", 5.0)]);

    let config = Arc::new(Config::load(&project_dir).expect("load config"));
    let (db, graph) = build_db_and_graph(&project_dir, &config);

    execute_project(
        "idempotent-window-1".to_string(),
        run_request("2026-01-01", "2026-01-02"),
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
    .expect("window 1 run must succeed");

    assert_eq!(
        table_contents(&db_path),
        full_refresh_oracle(&db_path, combiner_sql),
        "after window 1, the target must equal the full-refresh oracle"
    );

    seed_events(
        &db_path,
        &[
            (1, "2026-01-01", 10.0),
            (2, "2026-01-01", 5.0),
            (1, "2026-01-02", 7.0),
            (3, "2026-01-02", 2.0),
        ],
    );

    execute_project(
        "idempotent-window-2".to_string(),
        run_request("2026-01-02", "2026-01-03"),
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
    .expect("window 2 run must succeed");

    assert_eq!(
        table_contents(&db_path),
        full_refresh_oracle(&db_path, combiner_sql),
        "after window 2, the keyed-merge route must equal the full-refresh oracle"
    );
}
