//! Runtime coverage for the `contract.retain_departed` write-path
//! disposition (`docs/outcomes/20260815-definition-delta-migrate/phases/
//! 32b-plan.md`): a snapshot-reconcile keyed model deletes a key its
//! mutable-snapshot source no longer carries, in the same transaction as
//! the merge, unless `contract.retain_departed` is declared — in which case
//! the delete leg is suppressed and the reconcile anti-join probe runs
//! instead.
//!
//! Fixture mirrors `keyed_run_window_required.rs`'s snapshot-reconcile
//! `device_snapshot` model (clockless `mutable_snapshot` source, `grain:
//! key`).

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

fn stage_project(project_dir: &Path, db_path: &Path, contract_frontmatter: &str) {
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

    let model_sql = format!(
        r#"---
materialization: table
refresh: incremental
grain: key
maintenance:
  scan_bounds:
    per_source:
      devices:
        allow_full_scan: true
{contract_frontmatter}---
SELECT
    device_id,
    ANY_VALUE(amount) AS amount
FROM smelt.sources.devices
GROUP BY 1
"#
    );
    std::fs::write(project_dir.join("models/device_snapshot.sql"), model_sql).unwrap();

    let smelt_yml = format!(
        "name: departed_key_reconcile_test\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: {db}\n    schema: main\ndefault_materialization: table\n",
        db = db_path.display()
    );
    std::fs::write(project_dir.join("smelt.yml"), smelt_yml).unwrap();
}

fn seed_devices(db_path: &Path, rows: &[(i64, f64)]) -> anyhow::Result<()> {
    let conn = duckdb::Connection::open(db_path)?;
    let values = rows
        .iter()
        .map(|(id, amount)| format!("({id}, {amount})"))
        .collect::<Vec<_>>()
        .join(", ");
    conn.execute_batch(&format!(
        r#"
        CREATE SCHEMA IF NOT EXISTS main;
        CREATE OR REPLACE TABLE main.sources_devices AS
        SELECT * FROM (VALUES {values}) AS t(device_id, amount);
        "#
    ))?;
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

async fn run(project_dir: &Path, db_path: &Path, run_id: &str) -> anyhow::Result<()> {
    run_outcome(project_dir, db_path, run_id).await.map(|_| ())
}

async fn run_outcome(
    project_dir: &Path,
    db_path: &Path,
    run_id: &str,
) -> anyhow::Result<smelt_runtime::types::RunOutcome> {
    let config = Arc::new(Config::load(project_dir).expect("load config"));
    let (db, graph) = build_db_and_graph(project_dir, &config);
    execute_project(
        run_id.to_string(),
        base_request(),
        Arc::clone(&config),
        Arc::clone(&graph),
        Arc::clone(&db),
        project_dir,
        &PlainDuckDbFactory {
            db_path: db_path.to_path_buf(),
        },
        &smelt_runtime::NoOpReporter,
        CancellationToken::new(),
    )
    .await
}

fn stored_device_ids(db_path: &Path) -> Vec<i64> {
    let conn = duckdb::Connection::open(db_path).unwrap();
    let mut stmt = conn
        .prepare("SELECT device_id FROM main.device_snapshot ORDER BY device_id")
        .unwrap();
    stmt.query_map([], |r| r.get(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect()
}

/// The default point: a key present in the target but absent from the
/// incoming scan is deleted at reconcile — the stored table converges to a
/// full refresh of the new source.
#[tokio::test]
async fn snapshot_reconcile_deletes_departed_key() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().to_path_buf();
    let db_path = project_dir.join("dev.duckdb");

    stage_project(&project_dir, &db_path, "");
    seed_devices(&db_path, &[(1, 10.0), (2, 5.0)]).expect("seed");
    run(&project_dir, &db_path, "departed-key-run-1")
        .await
        .expect("first run creates the table");
    assert_eq!(stored_device_ids(&db_path), vec![1, 2]);

    // Device 2 departs the source.
    seed_devices(&db_path, &[(1, 10.0)]).expect("reseed");
    run(&project_dir, &db_path, "departed-key-run-2")
        .await
        .expect("reconcile run deletes the departed key");

    assert_eq!(
        stored_device_ids(&db_path),
        vec![1],
        "device 2 must be deleted once absent from the source scan"
    );
}

/// `contract.retain_departed: true` suppresses the delete leg: the departed
/// key survives the reconcile.
#[tokio::test]
async fn snapshot_reconcile_retains_departed_key_when_declared() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().to_path_buf();
    let db_path = project_dir.join("dev.duckdb");

    stage_project(
        &project_dir,
        &db_path,
        "contract:\n  retain_departed: true\n",
    );
    seed_devices(&db_path, &[(1, 10.0), (2, 5.0)]).expect("seed");
    run(&project_dir, &db_path, "retain-departed-run-1")
        .await
        .expect("first run creates the table");
    assert_eq!(stored_device_ids(&db_path), vec![1, 2]);

    seed_devices(&db_path, &[(1, 10.0)]).expect("reseed");
    run(&project_dir, &db_path, "retain-departed-run-2")
        .await
        .expect("reconcile run retains the departed key");

    assert_eq!(
        stored_device_ids(&db_path),
        vec![1, 2],
        "device 2 must survive when contract.retain_departed is declared"
    );
}

/// With the tombstone form declared, an unmarked departed key fails the
/// reconcile probe — the run refuses instead of silently exempting an
/// un-tombstoned row from comparison.
#[tokio::test]
async fn retain_departed_probe_is_dispatched_pre_write() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().to_path_buf();
    let db_path = project_dir.join("dev.duckdb");

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
contract:
  retain_departed:
    tombstone: is_departed
---
SELECT
    device_id,
    ANY_VALUE(amount) AS amount,
    ANY_VALUE(false) AS is_departed
FROM smelt.sources.devices
GROUP BY 1
"#;
    std::fs::write(project_dir.join("models/device_snapshot.sql"), model_sql).unwrap();
    let smelt_yml = format!(
        "name: departed_key_reconcile_test\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: {db}\n    schema: main\ndefault_materialization: table\n",
        db = db_path.display()
    );
    std::fs::write(project_dir.join("smelt.yml"), smelt_yml).unwrap();

    seed_devices(&db_path, &[(1, 10.0), (2, 5.0)]).expect("seed");
    run(&project_dir, &db_path, "tombstone-run-1")
        .await
        .expect("first run creates the table");

    // Device 2 departs the source, but is never marked `is_departed = true`
    // in the stored table (a real pipeline would set that via a later
    // update the tombstone form licenses; this fixture never runs one).
    seed_devices(&db_path, &[(1, 10.0)]).expect("reseed");
    let err = run(&project_dir, &db_path, "tombstone-run-2")
        .await
        .expect_err("an unmarked departure under the tombstone form must refuse");

    let message = format!("{err:#}");
    assert!(
        message.contains("retain_departed") && message.contains("not marked departed"),
        "refusal must name the unmarked-departure violation: {message}"
    );
    assert!(
        message.contains("ContractDepartedKeyUnmarked"),
        "refusal must name the diagnostic code: {message}"
    );
}

/// The declared `retain_departed` point's reconcile anti-join probe is
/// recorded on this model's manifest entry with the retained-departed
/// count in `observed` (`docs/specs/run_state.md` §"Run manifest").
#[tokio::test]
async fn retain_departed_probe_is_recorded_with_the_retained_count() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().to_path_buf();
    let db_path = project_dir.join("dev.duckdb");

    stage_project(
        &project_dir,
        &db_path,
        "contract:\n  retain_departed: true\n",
    );
    seed_devices(&db_path, &[(1, 10.0), (2, 5.0), (3, 1.0)]).expect("seed");
    run(&project_dir, &db_path, "probe-recorded-run-1")
        .await
        .expect("first run creates the table");

    // Devices 2 and 3 depart the source.
    seed_devices(&db_path, &[(1, 10.0)]).expect("reseed");
    let outcome = run_outcome(&project_dir, &db_path, "probe-recorded-run-2")
        .await
        .expect("reconcile run retains the departed keys");

    let record = outcome
        .models
        .get("device_snapshot")
        .expect("device_snapshot must have a manifest entry");
    let probe = record
        .probes
        .iter()
        .find(|p| p.fact == "contract.retain_departed")
        .unwrap_or_else(|| {
            panic!(
                "expected a contract.retain_departed probe record, got {:?}",
                record.probes
            )
        });
    assert_eq!(probe.probe, "ContractDepartedKeyUnmarked");
    assert_eq!(probe.outcome, smelt_state::ProbeRecordOutcome::Dispatched);
    assert_eq!(probe.observed, Some(2));
}

/// The default point (no `contract.retain_departed` declared) records no
/// probe at all — the delete leg it runs instead stays measurement-free.
#[tokio::test]
async fn default_point_records_no_probe() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().to_path_buf();
    let db_path = project_dir.join("dev.duckdb");

    stage_project(&project_dir, &db_path, "");
    seed_devices(&db_path, &[(1, 10.0), (2, 5.0)]).expect("seed");
    run(&project_dir, &db_path, "no-probe-run-1")
        .await
        .expect("first run creates the table");

    seed_devices(&db_path, &[(1, 10.0)]).expect("reseed");
    let outcome = run_outcome(&project_dir, &db_path, "no-probe-run-2")
        .await
        .expect("reconcile run deletes the departed key");

    let record = outcome
        .models
        .get("device_snapshot")
        .expect("device_snapshot must have a manifest entry");
    assert!(
        record.probes.is_empty(),
        "the default point must record no probe: {:?}",
        record.probes
    );
}
