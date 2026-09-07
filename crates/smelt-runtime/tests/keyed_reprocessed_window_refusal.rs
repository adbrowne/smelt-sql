//! Named-diagnostic coverage for the reconciliation ledger's never-fold-
//! twice refusal (`docs/specs/incremental_shapes.md` §"Reprocessing" /
//! §Surface diagnostics table `KeyedReprocessedWindow`).
//!
//! Fixture: `device_daily` is a `grain: key` model with a `SUM` combiner —
//! `WindowedKeyedRule::ledger_grade` grades a `Sum` combiner
//! `Grade::Additive`, so every merge for this model is guarded by the
//! warehouse-resident reconciliation ledger
//! (`Backend::fold_ledger_delta`). Running the *same* window twice through
//! the real `execute_project` pipeline
//! (`docs/specs/architecture.md` §"Run pipeline parity rule") must refuse
//! the second run: the delta is already reflected in the ledger. This test
//! asserts the refusal names the diagnostic code (`KeyedReprocessedWindow`),
//! the window bounds, and the `--full-refresh` remedy the spec's
//! diagnostics table promises — and that the target table's contents are
//! byte-unchanged by the refused run.

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

fn stage_project(project_dir: &Path, db_path: &Path) {
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

    // `SUM` combiner => `Grade::Additive` => every merge is ledger-guarded
    // (`WindowedKeyedRule::ledger_grade`).
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
        "name: keyed_reprocessed_window_test\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: {db}\n    schema: main\ndefault_materialization: table\n",
        db = db_path.display()
    );
    std::fs::write(project_dir.join("smelt.yml"), smelt_yml).unwrap();
}

fn seed_tables(db_path: &Path) -> anyhow::Result<()> {
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
    }
}

fn table_contents(db_path: &Path) -> Vec<(i64, String, f64)> {
    let conn = duckdb::Connection::open(db_path).unwrap();
    let mut stmt = conn
        .prepare("SELECT device_id, CAST(event_date AS VARCHAR), total_amount FROM main.device_daily ORDER BY device_id")
        .unwrap();
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
}

/// Re-merging an already-reflected delta must refuse the run, and the
/// refusal must be the named `KeyedReprocessedWindow` diagnostic: it names
/// the code, the window bounds, and points at `--full-refresh`. State must
/// be unchanged after the refusal.
#[tokio::test]
async fn reprocessed_window_refusal_names_the_diagnostic() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().to_path_buf();
    let db_path = project_dir.join("dev.duckdb");

    stage_project(&project_dir, &db_path);
    seed_tables(&db_path).expect("seed tables");

    let config = Arc::new(Config::load(&project_dir).expect("load config"));
    let (db, graph) = build_db_and_graph(&project_dir, &config);

    // First run: creates the target table and reflects the window in the
    // reconciliation ledger.
    execute_project(
        "keyed-reprocessed-window-first".to_string(),
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
    .expect("first run (create + ledger reflect) must succeed");

    let contents_before = table_contents(&db_path);
    assert_eq!(
        contents_before.len(),
        2,
        "first run must populate both device rows: {:?}",
        contents_before
    );

    // Second run over the SAME window: the delta is already reflected in
    // the ledger — must refuse loudly, named.
    let err = execute_project(
        "keyed-reprocessed-window-second".to_string(),
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
    .expect_err("re-merge of an already-reflected window must refuse");

    let message = format!("{err:#}");
    assert!(
        message.contains("KeyedReprocessedWindow"),
        "refusal must name the diagnostic code KeyedReprocessedWindow: {message}"
    );
    assert!(
        message.contains("2026-01-01") && message.contains("2026-01-02"),
        "refusal must name the window bounds: {message}"
    );
    assert!(
        message.contains("--full-refresh"),
        "refusal must point at the --full-refresh remedy: {message}"
    );

    let contents_after = table_contents(&db_path);
    assert_eq!(
        contents_before, contents_after,
        "target table contents must be unchanged after the refused run"
    );
}
