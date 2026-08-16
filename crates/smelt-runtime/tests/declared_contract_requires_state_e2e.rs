//! Real-DuckDB, `execute_project`-driven coverage for
//! `DeclaredContractRequiresState` (`docs/outcomes/20260816-state-residency/
//! phases/06-plan.md`): a project declaring `contract.deferral` under the
//! default `state.mode: stateless` posture must refuse the run via
//! `gate_diagnostics`, not silently execute an unmeasured promise
//! (`docs/specs/state.md` §"Declarations stay fail-loud"). Mirrors
//! `contract_deferral_skip_e2e.rs`'s harness shape, trimmed to the single
//! refusal scenario.

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

    let source_yml = r#"description: Raw events.
columns:
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

    // No `state:` block — the project stays at its default
    // `state.mode: stateless`, which withholds the interval ledger and
    // landed-delta record `contract.deferral`'s lag is measured against.
    let deferred_sql = r#"---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
contract:
  deferral: '2 days'
---
SELECT event_date, SUM(amount) AS total_amount
FROM smelt.sources.events
GROUP BY 1
"#;
    std::fs::write(project_dir.join("models/deferred_model.sql"), deferred_sql).unwrap();

    let smelt_yml = format!(
        "name: declared_contract_requires_state_e2e\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: {db}\n    schema: main\ndefault_materialization: table\n",
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
            (DATE '2026-01-01', 10.0),
            (DATE '2026-01-02', 5.0)
        ) AS t(event_date, amount);
        "#,
    )?;
    Ok(())
}

/// A project declaring `contract.deferral` under `state.mode: stateless`
/// refuses the run at the diagnostic-parity gate, naming
/// `DeclaredContractRequiresState`.
#[tokio::test]
async fn stateless_project_declaring_deferral_refuses_the_run() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().to_path_buf();
    let db_path = project_dir.join("dev.duckdb");

    stage_project(&project_dir, &db_path);
    seed_events(&db_path).expect("seed events");

    let config = Arc::new(Config::load(&project_dir).expect("load config"));

    let discovery = ModelDiscovery::new(project_dir.clone(), config.paths.clone());
    let sql_models = discovery.discover_models().expect("discover_models");

    let mut db = smelt_db::Database::default();
    let project = db.set_project_input(project_dir.clone(), String::new());
    let source_files: Vec<_> = sql_models
        .iter()
        .map(|m| db.set_source_file(m.path.clone(), m.content.clone(), project_dir.clone()))
        .collect();
    db.set_workspace(source_files, vec![project]);

    let graph = DependencyGraph::build(sql_models, None).expect("build graph");

    let db = Arc::new(tokio::sync::Mutex::new(db));
    let graph = Arc::new(tokio::sync::Mutex::new(graph));

    let request = ExecuteRequest {
        target: "dev".to_string(),
        select: vec![],
        exclude: vec![],
        start: Some("2026-01-01".to_string()),
        end: Some("2026-01-02".to_string()),
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
    };

    let result = execute_project(
        "run-a".to_string(),
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
    .await;

    let err = result.expect_err("a stateless project declaring deferral must refuse the run");
    assert!(
        err.to_string().contains("DeclaredContractRequiresState"),
        "expected the refusal to name DeclaredContractRequiresState, got: {err}"
    );
}
