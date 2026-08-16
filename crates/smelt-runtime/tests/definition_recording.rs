//! Phase 6 (`docs/outcomes/20260816-definition-delta-migrate-v2/phases/
//! 06-plan.md`): every maintenance arm an incrementally maintained model can
//! complete through must record `DeployedSchema::definition_sql` on first
//! deployment — `smelt migrate` diffs against it, and before this phase the
//! cumulative (`grain: key`) and key-addressed-dispatch arms in
//! `execute.rs` both returned before the baseline-save block ever ran.
//! `!already_stored` is asserted directly too: a windowed run under
//! *changed* SQL must never overwrite the recorded definition, or the
//! pending definition delta would vanish before `smelt migrate` could see
//! it (`docs/specs/definition_deltas.md` §Detection).

use std::path::Path;
use std::sync::Arc;

fn write(dir: &Path, rel: &str, content: &str) {
    let path = dir.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

fn build_db_and_graph(
    project_dir: &Path,
    config: &smelt_core::config::Config,
) -> (
    Arc<tokio::sync::Mutex<smelt_db::Database>>,
    Arc<tokio::sync::Mutex<smelt_core::graph::DependencyGraph>>,
) {
    use smelt_core::ModelDiscovery;
    let discovery = ModelDiscovery::new(project_dir.to_path_buf(), config.paths.clone());
    let sql_models = discovery.discover_models().expect("discover_models");

    let mut db = smelt_db::Database::default();
    let project = db.set_project_input(project_dir.to_path_buf(), String::new());
    let source_files: Vec<_> = sql_models
        .iter()
        .map(|m| db.set_source_file(m.path.clone(), m.content.clone(), project_dir.to_path_buf()))
        .collect();
    db.set_workspace(source_files, vec![project]);
    db.set_active_target(
        config
            .target
            .clone()
            .map(|t| std::sync::Arc::from(t.as_str())),
    );

    let graph = smelt_core::graph::DependencyGraph::build(sql_models, None).expect("build graph");

    (
        Arc::new(tokio::sync::Mutex::new(db)),
        Arc::new(tokio::sync::Mutex::new(graph)),
    )
}

fn select_request(models: &[&str]) -> smelt_runtime::types::ExecuteRequest {
    smelt_runtime::types::ExecuteRequest {
        target: "dev".to_string(),
        select: models.iter().map(|s| s.to_string()).collect(),
        exclude: vec![],
        start: None,
        end: None,
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

struct DuckDbBackendFactory {
    db_path: std::path::PathBuf,
}

impl smelt_runtime::execute::BackendFactory for DuckDbBackendFactory {
    fn create<'a>(
        &'a self,
        _target_name: &'a str,
        target_config: &'a smelt_core::config::Target,
        _project_dir: &'a Path,
    ) -> smelt_runtime::execute::BackendFuture<'a> {
        let path = self.db_path.clone();
        let schema = target_config.schema.clone();
        Box::pin(async move {
            let backend = smelt_backend_duckdb::DuckDbBackend::new(&path, &schema)
                .await
                .map_err(|e| anyhow::anyhow!("DuckDB init failed: {}", e))?;
            Ok(Box::new(backend) as Box<dyn smelt_backend::Backend>)
        })
    }
}

async fn run(
    project_dir: &Path,
    db_path: &Path,
    config: &Arc<smelt_core::config::Config>,
    models: &[&str],
    run_id: &str,
) -> smelt_runtime::types::RunOutcome {
    let (db, graph) = build_db_and_graph(project_dir, config);
    smelt_runtime::execute_project(
        run_id.to_string(),
        select_request(models),
        Arc::clone(config),
        graph,
        db,
        project_dir,
        &DuckDbBackendFactory {
            db_path: db_path.to_path_buf(),
        },
        &smelt_runtime::NoOpReporter,
        tokio_util::sync::CancellationToken::new(),
    )
    .await
    .unwrap_or_else(|e| panic!("run '{run_id}' must succeed: {e}"))
}

fn recorded_definition(project_dir: &Path, model: &str) -> Option<String> {
    let config = smelt_core::config::Config::load(project_dir).expect("load smelt.yml");
    let file_store = smelt_state::file_store::FileStore::new(project_dir, "dev", config.state.mode);
    file_store
        .load_schema(model)
        .expect("load_schema must not error")
        .map(|s| s.definition_sql)
}

// ── Plain windowed (partition-grain) incremental model ──────────────────

const WINDOWED_SMELT_YML: &str = "\
name: definition_recording\nversion: 1\npaths:\n  - models\n\
targets:\n  dev:\n    type: duckdb\n    schema: main\n\
default_materialization: table\nstate:\n  mode: intervals\n";

const WINDOWED_SOURCE_YAML: &str = "\
description: events source
columns:
  - name: event_id
    type: INTEGER
  - name: event_timestamp
    type: TIMESTAMP
  - name: user_id
    type: INTEGER
";

const WINDOWED_MODEL_SQL: &str = "\
---\nmaterialization: table\nrefresh: incremental\ngrain: partition\n\
timeseries:\n  partition_column: event_date\n  event_time_column: event_timestamp\n  \
granularity: day\n---\n\
SELECT e.event_id, CAST(e.event_timestamp AS DATE) AS event_date, e.user_id\n\
FROM smelt.sources.events e\n";

fn stage_windowed_project(project_dir: &Path) {
    write(project_dir, "smelt.yml", WINDOWED_SMELT_YML);
    write(
        project_dir,
        "models/sources/events.yml",
        WINDOWED_SOURCE_YAML,
    );
    write(project_dir, "models/daily_events.sql", WINDOWED_MODEL_SQL);
}

async fn seed_windowed_events(db_path: &Path) {
    let backend = smelt_backend_duckdb::DuckDbBackend::new(db_path, "main")
        .await
        .expect("open duckdb");
    use smelt_backend::Backend;
    backend
        .execute_sql(
            "CREATE TABLE main.sources_events (\
             event_id INTEGER, event_timestamp TIMESTAMP, user_id INTEGER)",
        )
        .await
        .expect("create events source table");
    backend
        .execute_sql(
            "INSERT INTO main.sources_events VALUES \
             (1, TIMESTAMP '2025-01-10 08:00:00', 1), \
             (2, TIMESTAMP '2025-01-11 09:00:00', 2)",
        )
        .await
        .expect("seed events");
}

#[tokio::test]
async fn windowed_run_records_deployed_definition() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().join("project");
    let db_path = tmp.path().join("run.duckdb");
    stage_windowed_project(&project_dir);
    seed_windowed_events(&db_path).await;

    let config = Arc::new(smelt_core::config::Config::load(&project_dir).expect("load smelt.yml"));
    let mut request_models = select_request(&["daily_events"]);
    request_models.start = Some("2025-01-10".to_string());
    request_models.end = Some("2025-01-12".to_string());
    let (db, graph) = build_db_and_graph(&project_dir, &config);
    smelt_runtime::execute_project(
        "windowed-run-1".to_string(),
        request_models,
        Arc::clone(&config),
        graph,
        db,
        &project_dir,
        &DuckDbBackendFactory {
            db_path: db_path.clone(),
        },
        &smelt_runtime::NoOpReporter,
        tokio_util::sync::CancellationToken::new(),
    )
    .await
    .expect("windowed run must succeed");

    let expected = std::fs::read_to_string(project_dir.join("models/daily_events.sql")).unwrap();
    let recorded = recorded_definition(&project_dir, "daily_events")
        .expect("a windowed run must record a deployed definition");
    assert_eq!(recorded, expected);
}

#[tokio::test]
async fn windowed_run_after_rewrite_keeps_the_old_recorded_definition() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().join("project");
    let db_path = tmp.path().join("run.duckdb");
    stage_windowed_project(&project_dir);
    seed_windowed_events(&db_path).await;

    let config = Arc::new(smelt_core::config::Config::load(&project_dir).expect("load smelt.yml"));
    let original_sql =
        std::fs::read_to_string(project_dir.join("models/daily_events.sql")).unwrap();

    let mut request1 = select_request(&["daily_events"]);
    request1.start = Some("2025-01-10".to_string());
    request1.end = Some("2025-01-11".to_string());
    let (db, graph) = build_db_and_graph(&project_dir, &config);
    smelt_runtime::execute_project(
        "windowed-run-1".to_string(),
        request1,
        Arc::clone(&config),
        graph,
        db,
        &project_dir,
        &DuckDbBackendFactory {
            db_path: db_path.clone(),
        },
        &smelt_runtime::NoOpReporter,
        tokio_util::sync::CancellationToken::new(),
    )
    .await
    .expect("first windowed run must succeed");

    // Rewrite the model — a pending definition delta the recorded snapshot
    // must survive until `smelt migrate` sees it. Deliberately a
    // same-column-set edit (an added `WHERE`, not an added/removed column):
    // an additive column change would auto-migrate via `check_and_migrate`'s
    // own `MigrationAction::AlterTable` arm (a distinct, already-existing
    // mechanism from `smelt migrate`) and re-record the definition itself —
    // this edit must reach `NoChange` there instead, so the ONLY way to see
    // it is `smelt migrate`.
    write(
        &project_dir,
        "models/daily_events.sql",
        "---\nmaterialization: table\nrefresh: incremental\ngrain: partition\n\
         timeseries:\n  partition_column: event_date\n  event_time_column: event_timestamp\n  \
         granularity: day\n---\n\
         SELECT e.event_id, CAST(e.event_timestamp AS DATE) AS event_date, e.user_id\n\
         FROM smelt.sources.events e\n\
         WHERE e.event_id > 0\n",
    );

    let mut request2 = select_request(&["daily_events"]);
    request2.start = Some("2025-01-11".to_string());
    request2.end = Some("2025-01-12".to_string());
    let (db, graph) = build_db_and_graph(&project_dir, &config);
    smelt_runtime::execute_project(
        "windowed-run-2".to_string(),
        request2,
        Arc::clone(&config),
        graph,
        db,
        &project_dir,
        &DuckDbBackendFactory {
            db_path: db_path.clone(),
        },
        &smelt_runtime::NoOpReporter,
        tokio_util::sync::CancellationToken::new(),
    )
    .await
    .expect("second windowed run (under changed SQL) must succeed");

    let recorded = recorded_definition(&project_dir, "daily_events")
        .expect("the first run's recorded definition must still be there");
    assert_eq!(
        recorded, original_sql,
        "a windowed run under changed SQL must never overwrite the recorded definition"
    );
}

// ── Cumulative (grain: key) arm ──────────────────────────────────────────

fn stage_keyed_project(project_dir: &Path) {
    write(
        project_dir,
        "smelt.yml",
        "name: definition_recording_keyed\nversion: 1\npaths:\n  - models\n\
         targets:\n  dev:\n    type: duckdb\n    schema: main\n\
         default_materialization: table\nstate:\n  mode: intervals\n",
    );
    write(
        project_dir,
        "models/sources/payments.yml",
        "description: payments\ncolumns:\n- name: user_id\n  type: INTEGER\n\
         - name: amount\n  type: DECIMAL(10,2)\n- name: d\n  type: DATE\n\
         mutation_profile:\n  kind: append_only\n\
         timeseries:\n  partition_column: d\n  event_time_column: d\n  granularity: day\n",
    );
    write(
        project_dir,
        "models/agg.sql",
        "---\nmaterialization: table\nrefresh: incremental\ngrain: key\n\
         unique_key: user_id\n---\n\
         SELECT user_id, SUM(amount) AS total\nFROM smelt.sources.payments\n\
         GROUP BY user_id\n",
    );
}

async fn seed_keyed_payments(db_path: &Path) {
    let backend = smelt_backend_duckdb::DuckDbBackend::new(db_path, "main")
        .await
        .expect("open duckdb");
    use smelt_backend::Backend;
    backend
        .execute_sql(
            "CREATE TABLE main.sources_payments (user_id INTEGER, amount DECIMAL(10,2), d DATE)",
        )
        .await
        .expect("create payments source table");
    backend
        .execute_sql(
            "INSERT INTO main.sources_payments VALUES \
             (1, 100.00, DATE '2025-01-01'), (2, 70.00, DATE '2025-01-01')",
        )
        .await
        .expect("seed payments");
}

#[tokio::test]
async fn cumulative_run_records_deployed_definition() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().join("project");
    let db_path = tmp.path().join("run.duckdb");
    stage_keyed_project(&project_dir);
    seed_keyed_payments(&db_path).await;

    let config = Arc::new(smelt_core::config::Config::load(&project_dir).expect("load smelt.yml"));
    let outcome = run(&project_dir, &db_path, &config, &["agg"], "keyed-run-1").await;
    let record = outcome.models.get("agg").expect("agg ran");
    assert_eq!(
        record.strategy, "cumulative_aggregate",
        "expected the keyed cumulative arm to dispatch"
    );

    let expected = std::fs::read_to_string(project_dir.join("models/agg.sql")).unwrap();
    let recorded = recorded_definition(&project_dir, "agg")
        .expect("the cumulative arm must record a deployed definition");
    assert_eq!(recorded, expected);
}

// ── Key-addressed-dispatch arm ────────────────────────────────────────────

fn stage_key_addressed_project(project_dir: &Path) {
    write(
        project_dir,
        "smelt.yml",
        "name: definition_recording_key_addressed\nversion: 1\npaths:\n  - models\n\
         targets:\n  dev:\n    type: duckdb\n    schema: main\n\
         default_materialization: table\nstate:\n  mode: intervals\n",
    );
    write(
        project_dir,
        "models/sources/payments.yml",
        "description: payments\ncolumns:\n- name: user_id\n  type: INTEGER\n\
         - name: amount\n  type: DECIMAL(10,2)\n- name: d\n  type: DATE\n\
         mutation_profile:\n  kind: append_only\n\
         timeseries:\n  partition_column: d\n  event_time_column: d\n  granularity: day\n",
    );
    write(
        project_dir,
        "models/agg.sql",
        "---\nmaterialization: table\nrefresh: incremental\ngrain: key\n\
         unique_key: user_id\n---\n\
         SELECT user_id, SUM(amount) AS total\nFROM smelt.sources.payments\n\
         GROUP BY user_id\n",
    );
    write(
        project_dir,
        "models/downstream.sql",
        "---\nmaterialization: table\ntimeseries:\n  event_time_column: d\n  \
         partition_column: d\n  granularity: day\nrefresh: incremental\n\
         grain: partition\n---\n\
         SELECT DATE '2024-01-01' AS d, user_id, ANY_VALUE(total) AS total \
         FROM smelt.agg GROUP BY user_id\n",
    );
}

async fn seed_key_addressed_payments(db_path: &Path) {
    let backend = smelt_backend_duckdb::DuckDbBackend::new(db_path, "main")
        .await
        .expect("open duckdb");
    use smelt_backend::Backend;
    backend
        .execute_sql(
            "CREATE TABLE main.sources_payments (user_id INTEGER, amount DECIMAL(10,2), d DATE)",
        )
        .await
        .expect("create payments source table");
    backend
        .execute_sql(
            "INSERT INTO main.sources_payments VALUES \
             (1, 100.00, DATE '2025-01-01'), (2, 70.00, DATE '2025-01-01')",
        )
        .await
        .expect("seed payments");
}

#[tokio::test]
async fn key_addressed_run_records_deployed_definition() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().join("project");
    let db_path = tmp.path().join("run.duckdb");
    stage_key_addressed_project(&project_dir);
    seed_key_addressed_payments(&db_path).await;

    let config = Arc::new(smelt_core::config::Config::load(&project_dir).expect("load smelt.yml"));

    // Run 1: creation — nothing to repair yet, `downstream` takes the
    // ordinary route.
    run(
        &project_dir,
        &db_path,
        &config,
        &["agg", "downstream"],
        "kaddr-run-1",
    )
    .await;

    // Mutate a payment so run 2 has something to repair.
    {
        let backend = smelt_backend_duckdb::DuckDbBackend::new(&db_path, "main")
            .await
            .expect("reopen duckdb");
        use smelt_backend::Backend;
        backend
            .execute_sql(
                "UPDATE main.sources_payments SET amount = 200.00 \
                 WHERE user_id = 1 AND amount = 100.00",
            )
            .await
            .expect("mutate payments");
    }

    let outcome = run(
        &project_dir,
        &db_path,
        &config,
        &["agg", "downstream"],
        "kaddr-run-2",
    )
    .await;
    let record = outcome.models.get("downstream").expect("downstream ran");
    assert_eq!(
        record.strategy, "per_group_recompute",
        "expected the key-addressed-dispatch arm to dispatch"
    );

    let expected = std::fs::read_to_string(project_dir.join("models/downstream.sql")).unwrap();
    let recorded = recorded_definition(&project_dir, "downstream")
        .expect("the key-addressed-dispatch arm must record a deployed definition");
    assert_eq!(recorded, expected);
}
