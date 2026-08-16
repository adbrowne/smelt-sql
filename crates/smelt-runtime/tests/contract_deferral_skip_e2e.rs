//! Real-DuckDB, `execute_project`-driven coverage for `contract.deferral`'s
//! two scheduling capabilities — run skipping and ledger-proven work
//! subsumption (`docs/specs/incremental_models.md` §"The contract lattice";
//! `docs/outcomes/20260809-contract-lattice-v1/phases/05-plan.md`). Mirrors
//! `keyed_reprocessed_window_refusal.rs`'s harness: a plain `DuckDbBackend`
//! driven through the real `execute_project` pipeline (Run Pipeline
//! Parity).
//!
//! Fixture: two partition-grain incremental models both consume the same
//! append-only clocked source (`raw.events`) — `upstream_advancer` has no
//! `contract:` declaration, `deferred_model` declares `contract.deferral:
//! '2 days'`. Selecting only `upstream_advancer` in a run advances the
//! shared `LandedDeltaStore` frontier for `raw.events` without
//! `deferred_model` ever running, which is how these tests open up a lag
//! between `deferred_model`'s own maintained frontier and the input
//! frontier without a second real clock.

use std::path::Path;
use std::sync::Arc;

use smelt_backend::Backend;
use smelt_backend_duckdb::DuckDbBackend;
use smelt_core::config::{Config, StateMode, Target};
use smelt_core::graph::DependencyGraph;
use smelt_core::ModelDiscovery;
use smelt_runtime::execute::{execute_project, BackendFactory, BackendFuture};
use smelt_runtime::types::ExecuteRequest;
use smelt_state::file_store::FileStore;
use smelt_state::RunOutcomeKind;
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

    let upstream_sql = r#"---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
---
SELECT event_date, SUM(amount) AS total_amount
FROM smelt.sources.events
GROUP BY 1
"#;
    std::fs::write(
        project_dir.join("models/upstream_advancer.sql"),
        upstream_sql,
    )
    .unwrap();

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

    // `probes: cadence: off` — this fixture's second scenario deliberately
    // drives `deferred_model`'s measured lag past `D` to exercise the
    // catch-up run (`run_license` licenses no skip once `lag > D`, mirroring
    // `run_license_runs_when_lag_exceeds_d`); with the default `PerRun`
    // cadence that same catch-up run would ALSO trip the
    // `ContractDeferralExceeded` probe (`docs/specs/model_properties.md`
    // §"Probe cadence": a genuinely exceeded declaration is a real
    // violation the probe is right to report). `cadence: off` trusts the
    // declaration instead, isolating the scheduling capability
    // (skip + subsumption) this phase adds from the phase-4 probe's own
    // orthogonal enforcement — a real deployment would tune cadence and D
    // together so routine catch-up runs do not also trip the probe.
    let smelt_yml = format!(
        "name: deferral_skip_e2e_test\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: {db}\n    schema: main\ndefault_materialization: table\nstate:\n  mode: intervals\nprobes:\n  cadence: off\n",
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
            (DATE '2026-01-02', 5.0),
            (DATE '2026-01-03', 7.0),
            (DATE '2026-01-04', 3.0),
            (DATE '2026-01-05', 9.0),
            (DATE '2026-01-06', 4.0)
        ) AS t(event_date, amount);
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

fn run_request(select: Vec<String>, start: &str, end: &str) -> ExecuteRequest {
    ExecuteRequest {
        target: "dev".to_string(),
        select,
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
        keyed_restrictions: std::collections::BTreeMap::new(),
    }
}

fn row_count(db_path: &Path, table: &str) -> i64 {
    let conn = duckdb::Connection::open(db_path).unwrap();
    conn.query_row(&format!("SELECT COUNT(*) FROM main.{table}"), [], |r| {
        r.get(0)
    })
    .unwrap()
}

#[allow(clippy::too_many_arguments)]
async fn run(
    label: &str,
    config: &Arc<Config>,
    db: &Arc<tokio::sync::Mutex<smelt_db::Database>>,
    graph: &Arc<tokio::sync::Mutex<DependencyGraph>>,
    project_dir: &Path,
    db_path: &Path,
    select: Vec<String>,
    start: &str,
    end: &str,
) -> anyhow::Result<smelt_runtime::types::RunOutcome> {
    execute_project(
        label.to_string(),
        run_request(select, start, end),
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
}

/// A run whose measured lag is inside the declared `D` is skipped —
/// recorded `skipped_deferral`, `outcome: Skipped`, and leaves the target
/// table and the interval ledger byte-unchanged.
#[tokio::test]
async fn deferred_run_is_recorded_skipped_and_writes_nothing() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().to_path_buf();
    let db_path = project_dir.join("dev.duckdb");

    stage_project(&project_dir, &db_path);
    seed_events(&db_path).expect("seed events");

    let config = Arc::new(Config::load(&project_dir).expect("load config"));
    let (db, graph) = build_db_and_graph(&project_dir, &config);

    // Run A: both models establish their maintained frontier at 2026-01-02
    // (the requested end — both stores record the exclusive end of the
    // covered range) and the shared source's landed-delta frontier at the
    // same date.
    run(
        "run-a",
        &config,
        &db,
        &graph,
        &project_dir,
        &db_path,
        vec![],
        "2026-01-01",
        "2026-01-02",
    )
    .await
    .expect("run A must succeed");

    let deferred_rows_after_a = row_count(&db_path, "deferred_model");
    assert_eq!(deferred_rows_after_a, 1);

    // Run B: only `upstream_advancer` runs, advancing the shared source's
    // landed-delta frontier to 2026-01-04 while `deferred_model` never
    // runs — its own maintained frontier stays at 2026-01-02. Lag is now 2
    // days, exactly `D`.
    run(
        "run-b",
        &config,
        &db,
        &graph,
        &project_dir,
        &db_path,
        vec!["upstream_advancer".to_string()],
        "2026-01-02",
        "2026-01-04",
    )
    .await
    .expect("run B must succeed");

    // Run C: `deferred_model` is selected — the measured lag (2 days) is
    // within the declared window (`D = 2 days`), so it must be skipped
    // rather than executed.
    let outcome = run(
        "run-c",
        &config,
        &db,
        &graph,
        &project_dir,
        &db_path,
        vec!["deferred_model".to_string()],
        "2026-01-04",
        "2026-01-05",
    )
    .await
    .expect("run C must succeed (a licensed skip, not a failure)");

    let record = outcome
        .models
        .get("deferred_model")
        .expect("deferred_model must have a manifest entry even when skipped");
    assert_eq!(record.strategy, "skipped_deferral");
    assert_eq!(record.outcome, RunOutcomeKind::Skipped);
    assert_eq!(record.row_count, 0);

    // The target table is untouched by the skip.
    assert_eq!(
        row_count(&db_path, "deferred_model"),
        deferred_rows_after_a,
        "a deferral-licensed skip must not write to the target table"
    );

    // The interval ledger records no new coverage for `deferred_model`.
    let file_store = FileStore::new(&project_dir, "dev", StateMode::Environments);
    let interval_store = file_store.load_intervals().expect("load intervals");
    let maintained = interval_store
        .get("deferred_model")
        .and_then(|mi| mi.latest_date())
        .expect("deferred_model has a recorded interval from run A");
    assert_eq!(
        maintained.format("%Y-%m-%d").to_string(),
        "2026-01-02",
        "a skipped run must not advance the interval ledger"
    );
}

/// A later run that exceeds the deferral window (lag > D) actually
/// executes, and — because a prior run manifest recorded `skipped_deferral`
/// for this model and this run's own write range covers the pending
/// window — its manifest entry records the subsumed window.
#[tokio::test]
async fn catch_up_run_records_the_subsumed_window() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().to_path_buf();
    let db_path = project_dir.join("dev.duckdb");

    stage_project(&project_dir, &db_path);
    seed_events(&db_path).expect("seed events");

    let config = Arc::new(Config::load(&project_dir).expect("load config"));
    let (db, graph) = build_db_and_graph(&project_dir, &config);

    // Run A: both models establish a maintained frontier at 2026-01-02.
    run(
        "run-a",
        &config,
        &db,
        &graph,
        &project_dir,
        &db_path,
        vec![],
        "2026-01-01",
        "2026-01-02",
    )
    .await
    .expect("run A must succeed");

    // Run B: advance the shared source's landed-delta frontier to
    // 2026-01-04 (lag 2, within D) without `deferred_model` running.
    run(
        "run-b",
        &config,
        &db,
        &graph,
        &project_dir,
        &db_path,
        vec!["upstream_advancer".to_string()],
        "2026-01-02",
        "2026-01-04",
    )
    .await
    .expect("run B must succeed");

    // Run C: `deferred_model` is skipped (lag 2 <= D 2), which is the
    // recorded prior skip run D's subsumption proof needs.
    run(
        "run-c",
        &config,
        &db,
        &graph,
        &project_dir,
        &db_path,
        vec!["deferred_model".to_string()],
        "2026-01-04",
        "2026-01-05",
    )
    .await
    .expect("run C must succeed (a licensed skip)");

    // Run D: advance the shared source's landed-delta frontier further, to
    // 2026-01-07 — lag against `deferred_model`'s still-2026-01-02
    // maintained frontier is now 5 days, past `D = 2 days`.
    run(
        "run-d",
        &config,
        &db,
        &graph,
        &project_dir,
        &db_path,
        vec!["upstream_advancer".to_string()],
        "2026-01-04",
        "2026-01-07",
    )
    .await
    .expect("run D must succeed");

    // Run E: `deferred_model`'s measured lag now exceeds `D`, so
    // `run_license` licenses no skip and it actually runs (`probes:
    // cadence: off` keeps the phase-4 deferral probe from separately
    // failing this genuinely-exceeded lag — see `stage_project`'s doc
    // comment). Its requested range [2026-01-01, 2026-01-07) covers the
    // entire pending window (2026-01-02 exclusive .. 2026-01-07
    // inclusive) — the covering run the subsumption proof requires.
    let outcome = run(
        "run-e",
        &config,
        &db,
        &graph,
        &project_dir,
        &db_path,
        vec!["deferred_model".to_string()],
        "2026-01-01",
        "2026-01-07",
    )
    .await
    .expect("run E must succeed — lag exceeds D, licensing a normal run");

    let record = outcome
        .models
        .get("deferred_model")
        .expect("deferred_model must have a manifest entry");
    assert_eq!(record.outcome, RunOutcomeKind::Success);
    let subsumed = record
        .subsumed
        .as_ref()
        .expect("the covering run must record the subsumed window");
    assert_eq!(subsumed.maintained_exclusive, "2026-01-02");
    assert_eq!(subsumed.input_inclusive, "2026-01-07");
}
