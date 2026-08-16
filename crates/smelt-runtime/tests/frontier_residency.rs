//! State residency phase 4 (`docs/outcomes/20260816-state-residency/
//! phases/04-plan.md`; `docs/specs/incremental_models.md` §"The frontier
//! record (reconciliation ledger)"): the reconciliation ledger's frontier
//! (idempotent) grading moves engine-resident, off `.smelt/`. Real-fixture,
//! DuckDB-backed coverage exercised through `execute_project`.

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
    if let Ok(cfg) = Config::load(project_dir) {
        db.set_active_target(cfg.target.map(|t| std::sync::Arc::from(t.as_str())));
    }

    let graph = DependencyGraph::build(sql_models, None).expect("build graph");

    (
        Arc::new(tokio::sync::Mutex::new(db)),
        Arc::new(tokio::sync::Mutex::new(graph)),
    )
}

const REVENUE_MODEL_SQL: &str = r#"---
materialization: table
timeseries:
  event_time_column: transaction_timestamp
  partition_column: revenue_date
  granularity: day
refresh: incremental
grain: partition
---
SELECT
    DATE_TRUNC('day', transaction_timestamp) AS revenue_date,
    user_id,
    SUM(amount) AS total_revenue
FROM (VALUES
    (1, 100.00, TIMESTAMP '2024-12-25 10:00:00'),
    (2, 200.00, TIMESTAMP '2024-12-25 14:00:00')
) AS t(user_id, amount, transaction_timestamp)
GROUP BY 1, 2
"#;

fn setup_project(tmp: &tempfile::TempDir) -> (std::path::PathBuf, Config) {
    let project_dir = tmp.path().to_path_buf();
    std::fs::create_dir_all(project_dir.join("models")).unwrap();
    std::fs::write(
        project_dir.join("models").join("revenue.sql"),
        REVENUE_MODEL_SQL,
    )
    .unwrap();

    let db_path = project_dir.join("run.duckdb");
    let smelt_yml = format!(
        "name: frontier_residency_test\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: {db}\n    schema: main\ndefault_materialization: view\n",
        db = db_path.display()
    );
    std::fs::write(project_dir.join("smelt.yml"), &smelt_yml).unwrap();

    let config = Config::load(&project_dir).expect("load config");
    (project_dir, config)
}

fn make_request(start: &str, end: &str) -> ExecuteRequest {
    ExecuteRequest {
        target: "dev".to_string(),
        select: vec![],
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
        retry_max: None,
        retry_backoff_ms: None,
        resume: false,
        technique_overrides: vec![],
    }
}

/// A real incremental `execute_project` run records the frontier reset in
/// the engine-resident `_smelt_frontier` table, and never writes
/// `.smelt/targets/<target>/reconciliation.json` at all.
#[tokio::test]
async fn incremental_run_records_the_frontier_in_the_engine() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (project_dir, config) = setup_project(&tmp);
    let config = Arc::new(config);
    let (db, graph) = build_db_and_graph(&project_dir, &config);

    let db_path = project_dir.join("run.duckdb");
    execute_project(
        "run-1".to_string(),
        make_request("2024-12-25", "2024-12-26"),
        Arc::clone(&config),
        Arc::clone(&graph),
        Arc::clone(&db),
        &project_dir,
        &DuckDbBackendFactory {
            db_path: db_path.clone(),
        },
        &NoOpReporter,
        CancellationToken::new(),
    )
    .await
    .expect("incremental run must succeed");

    let backend = DuckDbBackend::new(&db_path, "main")
        .await
        .expect("reopen duckdb");
    let rows = backend
        .execute_sql(
            "SELECT model_name, grp, input_name, delta_id, region_start, region_end \
             FROM main._smelt_frontier WHERE model_name = 'revenue'",
        )
        .await
        .expect("query frontier table");
    let total_rows: usize = rows.iter().map(|b| b.num_rows()).sum();
    assert_eq!(
        total_rows, 1,
        "expected exactly one frontier row for the run's own region"
    );

    assert!(
        !project_dir
            .join(".smelt/targets/dev/reconciliation.json")
            .exists(),
        "the legacy JSON ledger must never be written by a residency-aware runtime"
    );
}

/// A `.smelt/targets/<target>/reconciliation.json` left by a pre-residency
/// binary — one `Frontier` record and one `DeltaIdentities` record — is
/// imported into `_smelt_frontier`/`_smelt_ledger` respectively on the next
/// run, and the legacy file is removed.
#[tokio::test]
async fn legacy_reconciliation_json_is_imported_then_removed() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (project_dir, config) = setup_project(&tmp);
    let config = Arc::new(config);
    let (db, graph) = build_db_and_graph(&project_dir, &config);

    let db_path = project_dir.join("run.duckdb");

    // Seed a legacy reconciliation.json under the target directory, as a
    // pre-residency binary would have left it, holding one Frontier entry
    // for `revenue` and one DeltaIdentities entry for `other_model`.
    let target_dir = project_dir.join(".smelt/targets/dev");
    std::fs::create_dir_all(&target_dir).unwrap();
    std::fs::write(
        project_dir.join(".smelt/meta.json"),
        r#"{"state_version":2}"#,
    )
    .unwrap();

    let legacy_json = r#"{
        "revenue": {
            "records": [
                {
                    "region": {"start": "2024-01-01", "end": "2024-01-02"},
                    "group": "{*}",
                    "entry": {"processed": {"Frontier": {"self": "2024-01-02"}}}
                }
            ]
        },
        "other_model": {
            "records": [
                {
                    "region": {"start": "2024-01-01", "end": "2024-01-02"},
                    "group": "{*}",
                    "entry": {"processed": {"DeltaIdentities": {"smelt.events": ["d1", "d2"]}}}
                }
            ]
        }
    }"#;
    std::fs::write(target_dir.join("reconciliation.json"), legacy_json).unwrap();

    execute_project(
        "run-1".to_string(),
        make_request("2024-12-25", "2024-12-26"),
        Arc::clone(&config),
        Arc::clone(&graph),
        Arc::clone(&db),
        &project_dir,
        &DuckDbBackendFactory {
            db_path: db_path.clone(),
        },
        &NoOpReporter,
        CancellationToken::new(),
    )
    .await
    .expect("run must succeed");

    let backend = DuckDbBackend::new(&db_path, "main")
        .await
        .expect("reopen duckdb");

    let frontier_rows = backend
        .execute_sql(
            "SELECT input_name, delta_id FROM main._smelt_frontier \
             WHERE model_name = 'revenue' AND region_start = '2024-01-01'",
        )
        .await
        .expect("query frontier table");
    let frontier_count: usize = frontier_rows.iter().map(|b| b.num_rows()).sum();
    assert_eq!(
        frontier_count, 1,
        "the legacy Frontier record must be imported into _smelt_frontier"
    );

    let ledger_rows = backend
        .execute_sql(
            "SELECT delta_id FROM main._smelt_ledger \
             WHERE model_name = 'other_model' AND input_name = 'smelt.events' \
             ORDER BY delta_id",
        )
        .await
        .expect("query ledger table");
    let ledger_count: usize = ledger_rows.iter().map(|b| b.num_rows()).sum();
    assert_eq!(
        ledger_count, 2,
        "both legacy DeltaIdentities entries must be imported into _smelt_ledger"
    );

    assert!(
        !target_dir.join("reconciliation.json").exists(),
        "the legacy file must be removed after import"
    );
}
