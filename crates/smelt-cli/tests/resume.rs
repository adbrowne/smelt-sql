#![cfg(feature = "duckdb")]
//! Phase 7 TDD tests for `smelt run --resume`
//! (`docs/plans/20260719-prod-w2-operability.md` §"Phase 7: `--resume` from
//! partial failure"; `docs/specs/run_state.md` §"`--resume` semantics").
//!
//! Fixture style mirrors `crates/smelt-runtime/tests/retry.rs`: a real
//! DuckDB backend wrapped by a thin failure-injecting shim, driven directly
//! through `smelt_runtime::execute_project` (the same entry point
//! `smelt-cli`'s `commands/run.rs` calls — Run Pipeline Parity).

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex};

use arrow::array::RecordBatch;
use arrow::datatypes::SchemaRef;
use async_trait::async_trait;
use smelt_backend::{Backend, BackendCapabilities, BackendError, PartitionRange, SqlDialect};
use smelt_backend_duckdb::DuckDbBackend;
use smelt_core::config::{Config, Materialization as CfgMaterialization, ModelConfig, Target};
use smelt_core::graph::DependencyGraph;
use smelt_core::ModelDiscovery;
use smelt_runtime::execute::{BackendFactory, BackendFuture};
use smelt_runtime::execute_project;
use smelt_runtime::reporter::NoOpReporter;
use smelt_runtime::types::ExecuteRequest;
use smelt_state::file_store::FileStore;
use smelt_state::RunOutcomeKind;
use tokio_util::sync::CancellationToken;

// ── Fixture helpers (mirrors retry.rs) ──────────────────────────────────────

fn write_model(project_dir: &Path, name: &str, content: &str) {
    let path = project_dir.join("models").join(format!("{}.sql", name));
    std::fs::write(path, content).expect("write model file");
}

fn write_config(project_dir: &Path, db_path: &Path) {
    let smelt_yml = format!(
        "name: resume_test\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: {db}\n    schema: main\ndefault_materialization: table\n",
        db = db_path.display()
    );
    std::fs::write(project_dir.join("smelt.yml"), smelt_yml).unwrap();
}

fn make_config(db_path: &Path) -> Arc<Config> {
    let mut targets = HashMap::new();
    targets.insert(
        "dev".to_string(),
        Target {
            target_type: "duckdb".to_string(),
            database: Some(db_path.to_string_lossy().into_owned()),
            schema: "main".to_string(),
            connect_url: None,
            catalog: None,
            warehouse: None,
            format: None,
            settings: None,
        },
    );
    Arc::new(Config {
        name: "resume_test".to_string(),
        version: 1,
        paths: vec!["models".to_string()],
        targets,
        default_materialization: CfgMaterialization::Table,
        models: HashMap::<String, ModelConfig>::new(),
        python: None,
        target: None,
        state: Default::default(),
        maintenance: None,
    })
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
    if let Ok(cfg) = smelt_core::config::Config::load(project_dir) {
        db.set_active_target(cfg.target.map(|t| std::sync::Arc::from(t.as_str())));
    }

    let graph = DependencyGraph::build(sql_models, None).expect("build graph");

    (
        Arc::new(tokio::sync::Mutex::new(db)),
        Arc::new(tokio::sync::Mutex::new(graph)),
    )
}

fn base_request(target: &str, resume: bool) -> ExecuteRequest {
    ExecuteRequest {
        target: target.to_string(),
        select: vec![],
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
        // Serial execution keeps `call_log` ordering assertions deterministic.
        jobs: Some(1),
        // A model that fails must fail on the FIRST attempt, not be
        // absorbed by Phase 6's bounded retry — these tests inject a
        // one-shot failure per model and assert on exactly one call.
        retry_max: Some(0),
        retry_backoff_ms: Some(1),
        resume,
    }
}

// ── SelectiveFailingBackend — delegates to a real DuckDB backend, but fails
//    the first `create_table_as` call for any table name in `fail_once_for`
//    (removing it from the set after failing, so a later call for the SAME
//    name — i.e. a resumed re-run of that model — succeeds). ───────────────

struct SelectiveFailingBackend {
    inner: DuckDbBackend,
    fail_once_for: Arc<Mutex<HashSet<String>>>,
    call_log: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl Backend for SelectiveFailingBackend {
    async fn execute_sql(&self, sql: &str) -> Result<Vec<RecordBatch>, BackendError> {
        self.inner.execute_sql(sql).await
    }

    async fn create_table_as(
        &self,
        schema: &str,
        name: &str,
        sql: &str,
    ) -> Result<(), BackendError> {
        self.call_log.lock().unwrap().push(name.to_string());
        let should_fail = self.fail_once_for.lock().unwrap().remove(name);
        if should_fail {
            return Err(BackendError::connection_failed(format!(
                "injected one-shot failure for '{name}'"
            )));
        }
        self.inner.create_table_as(schema, name, sql).await
    }

    async fn create_view_as(
        &self,
        schema: &str,
        name: &str,
        sql: &str,
    ) -> Result<(), BackendError> {
        self.inner.create_view_as(schema, name, sql).await
    }

    async fn drop_table_if_exists(&self, schema: &str, name: &str) -> Result<(), BackendError> {
        self.inner.drop_table_if_exists(schema, name).await
    }

    async fn drop_view_if_exists(&self, schema: &str, name: &str) -> Result<(), BackendError> {
        self.inner.drop_view_if_exists(schema, name).await
    }

    async fn get_row_count(&self, schema: &str, name: &str) -> Result<usize, BackendError> {
        self.inner.get_row_count(schema, name).await
    }

    async fn get_preview(
        &self,
        schema: &str,
        name: &str,
        limit: usize,
    ) -> Result<Vec<RecordBatch>, BackendError> {
        self.inner.get_preview(schema, name, limit).await
    }

    async fn table_exists(&self, schema: &str, name: &str) -> Result<bool, BackendError> {
        self.inner.table_exists(schema, name).await
    }

    async fn ensure_schema(&self, schema: &str) -> Result<(), BackendError> {
        self.inner.ensure_schema(schema).await
    }

    fn dialect(&self) -> SqlDialect {
        self.inner.dialect()
    }

    fn capabilities(&self) -> BackendCapabilities {
        self.inner.capabilities()
    }

    async fn load_table(
        &self,
        schema: &str,
        name: &str,
        arrow_schema: SchemaRef,
        batches: Vec<RecordBatch>,
    ) -> Result<(), BackendError> {
        self.inner
            .load_table(schema, name, arrow_schema, batches)
            .await
    }

    async fn delete_partitions(
        &self,
        schema: &str,
        name: &str,
        partition: &PartitionRange,
    ) -> Result<(), BackendError> {
        self.inner.delete_partitions(schema, name, partition).await
    }

    async fn insert_into_from_query(
        &self,
        schema: &str,
        name: &str,
        sql: &str,
    ) -> Result<(), BackendError> {
        self.inner.insert_into_from_query(schema, name, sql).await
    }

    async fn insert_overwrite(
        &self,
        schema: &str,
        table: &str,
        sql: &str,
        partition: &PartitionRange,
    ) -> Result<(), BackendError> {
        self.inner
            .insert_overwrite(schema, table, sql, partition)
            .await
    }
}

struct SelectiveFailingBackendFactory {
    db_path: std::path::PathBuf,
    fail_once_for: Arc<Mutex<HashSet<String>>>,
    call_log: Arc<Mutex<Vec<String>>>,
}

impl BackendFactory for SelectiveFailingBackendFactory {
    fn create<'a>(
        &'a self,
        _target_name: &'a str,
        target_config: &'a smelt_core::config::Target,
        _project_dir: &'a Path,
    ) -> BackendFuture<'a> {
        let path = self.db_path.clone();
        let schema = target_config.schema.clone();
        let fail_once_for = Arc::clone(&self.fail_once_for);
        let call_log = Arc::clone(&self.call_log);
        Box::pin(async move {
            let inner = DuckDbBackend::new(&path, &schema)
                .await
                .map_err(|e| anyhow::anyhow!("DuckDB init failed: {}", e))?;
            Ok(Box::new(SelectiveFailingBackend {
                inner,
                fail_once_for,
                call_log,
            }) as Box<dyn Backend>)
        })
    }
}

// ── CancellingBackend — delegates to a real DuckDB backend, but cancels the
//    shared `CancellationToken` right after `create_table_as` succeeds for a
//    named "trigger" table (simulating `smelt-ui`'s stop button landing
//    mid-run: the just-finished model committed, but everything still queued
//    observes cancellation on its next `execute_one_model` check). ─────────

struct CancellingBackend {
    inner: DuckDbBackend,
    cancel_after: String,
    token: CancellationToken,
}

#[async_trait]
impl Backend for CancellingBackend {
    async fn execute_sql(&self, sql: &str) -> Result<Vec<RecordBatch>, BackendError> {
        self.inner.execute_sql(sql).await
    }

    async fn create_table_as(
        &self,
        schema: &str,
        name: &str,
        sql: &str,
    ) -> Result<(), BackendError> {
        self.inner.create_table_as(schema, name, sql).await?;
        if name == self.cancel_after {
            self.token.cancel();
        }
        Ok(())
    }

    async fn create_view_as(
        &self,
        schema: &str,
        name: &str,
        sql: &str,
    ) -> Result<(), BackendError> {
        self.inner.create_view_as(schema, name, sql).await
    }

    async fn drop_table_if_exists(&self, schema: &str, name: &str) -> Result<(), BackendError> {
        self.inner.drop_table_if_exists(schema, name).await
    }

    async fn drop_view_if_exists(&self, schema: &str, name: &str) -> Result<(), BackendError> {
        self.inner.drop_view_if_exists(schema, name).await
    }

    async fn get_row_count(&self, schema: &str, name: &str) -> Result<usize, BackendError> {
        self.inner.get_row_count(schema, name).await
    }

    async fn get_preview(
        &self,
        schema: &str,
        name: &str,
        limit: usize,
    ) -> Result<Vec<RecordBatch>, BackendError> {
        self.inner.get_preview(schema, name, limit).await
    }

    async fn table_exists(&self, schema: &str, name: &str) -> Result<bool, BackendError> {
        self.inner.table_exists(schema, name).await
    }

    async fn ensure_schema(&self, schema: &str) -> Result<(), BackendError> {
        self.inner.ensure_schema(schema).await
    }

    fn dialect(&self) -> SqlDialect {
        self.inner.dialect()
    }

    fn capabilities(&self) -> BackendCapabilities {
        self.inner.capabilities()
    }

    async fn load_table(
        &self,
        schema: &str,
        name: &str,
        arrow_schema: SchemaRef,
        batches: Vec<RecordBatch>,
    ) -> Result<(), BackendError> {
        self.inner
            .load_table(schema, name, arrow_schema, batches)
            .await
    }

    async fn delete_partitions(
        &self,
        schema: &str,
        name: &str,
        partition: &PartitionRange,
    ) -> Result<(), BackendError> {
        self.inner.delete_partitions(schema, name, partition).await
    }

    async fn insert_into_from_query(
        &self,
        schema: &str,
        name: &str,
        sql: &str,
    ) -> Result<(), BackendError> {
        self.inner.insert_into_from_query(schema, name, sql).await
    }

    async fn insert_overwrite(
        &self,
        schema: &str,
        table: &str,
        sql: &str,
        partition: &PartitionRange,
    ) -> Result<(), BackendError> {
        self.inner
            .insert_overwrite(schema, table, sql, partition)
            .await
    }
}

struct CancellingBackendFactory {
    db_path: std::path::PathBuf,
    cancel_after: String,
    token: CancellationToken,
}

impl BackendFactory for CancellingBackendFactory {
    fn create<'a>(
        &'a self,
        _target_name: &'a str,
        target_config: &'a smelt_core::config::Target,
        _project_dir: &'a Path,
    ) -> BackendFuture<'a> {
        let path = self.db_path.clone();
        let schema = target_config.schema.clone();
        let cancel_after = self.cancel_after.clone();
        let token = self.token.clone();
        Box::pin(async move {
            let inner = DuckDbBackend::new(&path, &schema)
                .await
                .map_err(|e| anyhow::anyhow!("DuckDB init failed: {}", e))?;
            Ok(Box::new(CancellingBackend {
                inner,
                cancel_after,
                token,
            }) as Box<dyn Backend>)
        })
    }
}

struct PlainDuckDbBackendFactory {
    db_path: std::path::PathBuf,
}

impl BackendFactory for PlainDuckDbBackendFactory {
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

/// Query `SELECT id, val FROM main.<table> ORDER BY id` as `(i64, String)`
/// pairs, via a fresh read-only-ish DuckDB connection over the same file.
async fn read_id_val_rows(db_path: &Path, table: &str) -> Vec<(i64, String)> {
    let backend = DuckDbBackend::new(db_path, "main")
        .await
        .expect("open duckdb for assertions");
    let batches = backend
        .execute_sql(&format!("SELECT id, val FROM main.{table} ORDER BY id"))
        .await
        .expect("query table");
    let mut rows = Vec::new();
    for batch in &batches {
        let vals = batch
            .column(1)
            .as_any()
            .downcast_ref::<arrow::array::StringArray>()
            .expect("val column is Utf8");
        for i in 0..batch.num_rows() {
            let id: i64 = match batch.column(0).data_type() {
                arrow::datatypes::DataType::Int8 => batch
                    .column(0)
                    .as_any()
                    .downcast_ref::<arrow::array::Int8Array>()
                    .expect("id column is Int8")
                    .value(i) as i64,
                arrow::datatypes::DataType::Int16 => batch
                    .column(0)
                    .as_any()
                    .downcast_ref::<arrow::array::Int16Array>()
                    .expect("id column is Int16")
                    .value(i) as i64,
                arrow::datatypes::DataType::Int32 => batch
                    .column(0)
                    .as_any()
                    .downcast_ref::<arrow::array::Int32Array>()
                    .expect("id column is Int32")
                    .value(i) as i64,
                arrow::datatypes::DataType::Int64 => batch
                    .column(0)
                    .as_any()
                    .downcast_ref::<arrow::array::Int64Array>()
                    .expect("id column is Int64")
                    .value(i),
                other => panic!("unexpected id column type: {other:?}"),
            };
            rows.push((id, vals.value(i).to_string()));
        }
    }
    rows
}

// ── Tests ────────────────────────────────────────────────────────────────

/// Three-model chain `up -> mid -> down`. `mid` fails once on the first
/// (non-resume) run — the run aborts, leaving `up: success`, `mid: failed`,
/// `down: skipped` in an incomplete manifest. `smelt run --resume` must
/// execute only `mid` and its downstream `down`, leaving `up`'s
/// already-materialized table (and its manifest entry) untouched.
///
/// Also proves the resume + incremental interplay: the resumed run's final
/// `down` table content is asserted equal to a clean, un-interrupted run of
/// the identical model chain against a separate database (the
/// `maintenance_conformance`-style oracle comparison the Phase 7 review
/// checklist calls for).
#[tokio::test]
async fn resume_skips_previously_succeeded_models() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project_dir = tmp.path();
    std::fs::create_dir_all(project_dir.join("models")).unwrap();
    write_model(project_dir, "up", "SELECT 1 AS id, 'a' AS val");
    write_model(project_dir, "mid", "SELECT id, val FROM smelt.up");
    write_model(project_dir, "down", "SELECT id, val FROM smelt.mid");

    let db_path = project_dir.join("db.duckdb");
    write_config(project_dir, &db_path);
    let config = make_config(&db_path);

    let fail_once_for = Arc::new(Mutex::new(HashSet::from(["mid".to_string()])));
    let call_log = Arc::new(Mutex::new(Vec::new()));
    let factory = SelectiveFailingBackendFactory {
        db_path: db_path.clone(),
        fail_once_for: Arc::clone(&fail_once_for),
        call_log: Arc::clone(&call_log),
    };
    let reporter = NoOpReporter;

    // ── Run 1: fails partway through `mid`. ─────────────────────────────
    {
        let (db, graph) = build_db_and_graph(project_dir, &config);
        let result = execute_project(
            "run-1".to_string(),
            base_request("dev", false),
            Arc::clone(&config),
            graph,
            db,
            project_dir,
            &factory,
            &reporter,
            CancellationToken::new(),
        )
        .await;
        assert!(result.is_err(), "run 1 must fail — 'mid' was made to fail");
    }

    // The manifest from the failed run records every model's outcome.
    let file_store = FileStore::new(project_dir, "dev");
    let runs_after_failure = file_store.load_runs(None).unwrap();
    assert_eq!(
        runs_after_failure.len(),
        1,
        "the failed run's manifest must be persisted"
    );
    let failed_manifest = &runs_after_failure[0];
    assert!(
        failed_manifest.completed_at.is_none(),
        "a failed run's manifest is incomplete (completed_at: null)"
    );
    assert_eq!(
        failed_manifest.models["up"].outcome,
        RunOutcomeKind::Success
    );
    assert_eq!(
        failed_manifest.models["mid"].outcome,
        RunOutcomeKind::Failed
    );
    assert_eq!(
        failed_manifest.models["down"].outcome,
        RunOutcomeKind::Skipped
    );

    call_log.lock().unwrap().clear();

    // ── Run 2: `--resume`. ───────────────────────────────────────────────
    {
        let (db, graph) = build_db_and_graph(project_dir, &config);
        let outcome = execute_project(
            "run-2".to_string(),
            base_request("dev", true),
            Arc::clone(&config),
            graph,
            db,
            project_dir,
            &factory,
            &reporter,
            CancellationToken::new(),
        )
        .await
        .expect("--resume run must succeed — 'mid' no longer fails");

        assert_eq!(
            outcome.models["up"].outcome,
            RunOutcomeKind::Skipped,
            "'up' succeeded last time with an unchanged definition — must be resumed away"
        );
        assert_eq!(outcome.models["mid"].outcome, RunOutcomeKind::Success);
        assert_eq!(outcome.models["down"].outcome, RunOutcomeKind::Success);
    }

    let calls = call_log.lock().unwrap().clone();
    assert!(
        !calls.contains(&"up".to_string()),
        "'up' must not be re-executed under --resume, got calls: {calls:?}"
    );
    assert!(calls.contains(&"mid".to_string()));
    assert!(calls.contains(&"down".to_string()));

    // ── Equivalence: resumed final state == a clean, uninterrupted run. ──
    let oracle_dir = tempfile::tempdir().expect("oracle tempdir");
    let oracle_project = oracle_dir.path();
    std::fs::create_dir_all(oracle_project.join("models")).unwrap();
    write_model(oracle_project, "up", "SELECT 1 AS id, 'a' AS val");
    write_model(oracle_project, "mid", "SELECT id, val FROM smelt.up");
    write_model(oracle_project, "down", "SELECT id, val FROM smelt.mid");
    let oracle_db_path = oracle_project.join("db.duckdb");
    write_config(oracle_project, &oracle_db_path);
    let oracle_config = make_config(&oracle_db_path);
    let oracle_factory = PlainDuckDbBackendFactory {
        db_path: oracle_db_path.clone(),
    };
    let (oracle_db, oracle_graph) = build_db_and_graph(oracle_project, &oracle_config);
    execute_project(
        "run-oracle".to_string(),
        base_request("dev", false),
        oracle_config,
        oracle_graph,
        oracle_db,
        oracle_project,
        &oracle_factory,
        &reporter,
        CancellationToken::new(),
    )
    .await
    .expect("oracle run must succeed");

    let resumed_rows = read_id_val_rows(&db_path, "down").await;
    let oracle_rows = read_id_val_rows(&oracle_db_path, "down").await;
    assert_eq!(
        resumed_rows, oracle_rows,
        "a resumed run's final materialized state must equal a clean, uninterrupted run's"
    );
}

/// Independent standalone model `alone` (no relationship to the `up2 ->
/// mid2 -> down2` chain) succeeds on the first run; `down2` fails once. Its
/// SQL is then EDITED (changing its output) before `--resume` — a changed
/// definition must re-run even though `alone` never failed and is not
/// downstream of anything that did. `up2`/`mid2` (still `success`, still
/// unchanged) must stay skipped.
#[tokio::test]
async fn resume_reruns_model_whose_definition_changed() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project_dir = tmp.path();
    std::fs::create_dir_all(project_dir.join("models")).unwrap();
    write_model(project_dir, "alone", "SELECT 1 AS id, 'orig' AS val");
    write_model(project_dir, "up2", "SELECT 1 AS id, 'a' AS val");
    write_model(project_dir, "mid2", "SELECT id, val FROM smelt.up2");
    write_model(project_dir, "down2", "SELECT id, val FROM smelt.mid2");

    let db_path = project_dir.join("db.duckdb");
    write_config(project_dir, &db_path);
    let config = make_config(&db_path);

    let fail_once_for = Arc::new(Mutex::new(HashSet::from(["down2".to_string()])));
    let call_log = Arc::new(Mutex::new(Vec::new()));
    let factory = SelectiveFailingBackendFactory {
        db_path: db_path.clone(),
        fail_once_for: Arc::clone(&fail_once_for),
        call_log: Arc::clone(&call_log),
    };
    let reporter = NoOpReporter;

    {
        let (db, graph) = build_db_and_graph(project_dir, &config);
        let result = execute_project(
            "run-1".to_string(),
            base_request("dev", false),
            Arc::clone(&config),
            graph,
            db,
            project_dir,
            &factory,
            &reporter,
            CancellationToken::new(),
        )
        .await;
        assert!(
            result.is_err(),
            "run 1 must fail — 'down2' was made to fail"
        );
    }

    // Edit `alone`'s definition between runs.
    write_model(project_dir, "alone", "SELECT 1 AS id, 'changed' AS val");

    call_log.lock().unwrap().clear();

    {
        let (db, graph) = build_db_and_graph(project_dir, &config);
        let outcome = execute_project(
            "run-2".to_string(),
            base_request("dev", true),
            Arc::clone(&config),
            graph,
            db,
            project_dir,
            &factory,
            &reporter,
            CancellationToken::new(),
        )
        .await
        .expect("--resume run must succeed — 'down2' no longer fails");

        assert_eq!(
            outcome.models["alone"].outcome,
            RunOutcomeKind::Success,
            "an edited model must re-run under --resume even though it never failed"
        );
        assert_eq!(outcome.models["up2"].outcome, RunOutcomeKind::Skipped);
        assert_eq!(outcome.models["mid2"].outcome, RunOutcomeKind::Skipped);
        assert_eq!(outcome.models["down2"].outcome, RunOutcomeKind::Success);
    }

    let calls = call_log.lock().unwrap().clone();
    assert!(calls.contains(&"alone".to_string()));
    assert!(!calls.contains(&"up2".to_string()));
    assert!(!calls.contains(&"mid2".to_string()));
    assert!(calls.contains(&"down2".to_string()));

    let alone_rows = read_id_val_rows(&db_path, "alone").await;
    assert_eq!(
        alone_rows,
        vec![(1, "changed".to_string())],
        "the resumed run must have recompiled 'alone' with its NEW definition, not reused the stale table"
    );
}

/// Three-model chain `up3 -> mid3 -> down3`. The run's `CancellationToken`
/// is cancelled right after `up3` commits (simulating `smelt-ui`'s stop
/// button), so `mid3` and `down3` never execute. The cancelled run must
/// still persist a manifest — `completed_at: null`, `up3: success`,
/// `mid3`/`down3: skipped` — exactly like a hard-error abort, and a
/// subsequent `smelt run --resume` must pick it up and complete only the
/// skipped models (`docs/specs/run_state.md` §"`--resume` semantics": a
/// cancelled run leaves an incomplete manifest just like an aborted one).
#[tokio::test]
async fn resume_picks_up_cancelled_run() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project_dir = tmp.path();
    std::fs::create_dir_all(project_dir.join("models")).unwrap();
    write_model(project_dir, "up3", "SELECT 1 AS id, 'a' AS val");
    write_model(project_dir, "mid3", "SELECT id, val FROM smelt.up3");
    write_model(project_dir, "down3", "SELECT id, val FROM smelt.mid3");

    let db_path = project_dir.join("db.duckdb");
    write_config(project_dir, &db_path);
    let config = make_config(&db_path);
    let reporter = NoOpReporter;

    // ── Run 1: cancelled right after `up3` commits. ─────────────────────
    let token = CancellationToken::new();
    {
        let factory = CancellingBackendFactory {
            db_path: db_path.clone(),
            cancel_after: "up3".to_string(),
            token: token.clone(),
        };
        let (db, graph) = build_db_and_graph(project_dir, &config);
        let outcome = execute_project(
            "run-1".to_string(),
            base_request("dev", false),
            Arc::clone(&config),
            graph,
            db,
            project_dir,
            &factory,
            &reporter,
            token.clone(),
        )
        .await
        .expect("a cancelled run returns Ok, not Err");

        assert_eq!(outcome.models["up3"].outcome, RunOutcomeKind::Success);
        assert_eq!(outcome.models["mid3"].outcome, RunOutcomeKind::Skipped);
        assert_eq!(outcome.models["down3"].outcome, RunOutcomeKind::Skipped);
    }

    // The cancelled run's manifest must be persisted and incomplete.
    let file_store = FileStore::new(project_dir, "dev");
    let runs_after_cancel = file_store.load_runs(None).unwrap();
    assert_eq!(
        runs_after_cancel.len(),
        1,
        "the cancelled run's manifest must be persisted"
    );
    let cancelled_manifest = &runs_after_cancel[0];
    assert!(
        cancelled_manifest.completed_at.is_none(),
        "a cancelled run's manifest is incomplete (completed_at: null)"
    );
    assert_eq!(
        cancelled_manifest.models["up3"].outcome,
        RunOutcomeKind::Success
    );
    assert_eq!(
        cancelled_manifest.models["mid3"].outcome,
        RunOutcomeKind::Skipped
    );
    assert_eq!(
        cancelled_manifest.models["down3"].outcome,
        RunOutcomeKind::Skipped
    );

    // ── Run 2: `--resume`, this time with a plain (non-cancelling) backend
    //    so the run completes. ───────────────────────────────────────────
    let plain_factory = PlainDuckDbBackendFactory {
        db_path: db_path.clone(),
    };
    let (db, graph) = build_db_and_graph(project_dir, &config);
    let outcome = execute_project(
        "run-2".to_string(),
        base_request("dev", true),
        config,
        graph,
        db,
        project_dir,
        &plain_factory,
        &reporter,
        CancellationToken::new(),
    )
    .await
    .expect("--resume run must succeed after cancellation");

    assert_eq!(
        outcome.models["up3"].outcome,
        RunOutcomeKind::Skipped,
        "'up3' succeeded last time with an unchanged definition — must be resumed away"
    );
    assert_eq!(outcome.models["mid3"].outcome, RunOutcomeKind::Success);
    assert_eq!(outcome.models["down3"].outcome, RunOutcomeKind::Success);
}

/// A manifest can complete normally (`completed_at` set) yet still record a
/// non-`success` outcome for a model in the current selection — e.g. a
/// check failure that skips a model's downstream dependents without
/// aborting the whole run. `docs/specs/run_state.md` §"`--resume` semantics"
/// defines the resume candidate as the latest manifest with
/// `completed_at: null`, **or** whose selection overlaps the current one
/// and recorded at least one non-`success` outcome — this test drives that
/// second disjunct directly (constructing the completed-but-partial
/// manifest by hand, since reaching it via a real check failure would
/// require a much larger fixture) rather than only the `completed_at: null`
/// path the other tests in this file exercise.
#[tokio::test]
async fn resume_picks_up_completed_run_with_non_success_outcome() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project_dir = tmp.path();
    std::fs::create_dir_all(project_dir.join("models")).unwrap();
    write_model(project_dir, "solo", "SELECT 1 AS id, 'a' AS val");

    let db_path = project_dir.join("db.duckdb");
    write_config(project_dir, &db_path);
    let config = make_config(&db_path);
    let reporter = NoOpReporter;

    // ── Run 1: completes cleanly. ────────────────────────────────────────
    {
        let factory = PlainDuckDbBackendFactory {
            db_path: db_path.clone(),
        };
        let (db, graph) = build_db_and_graph(project_dir, &config);
        execute_project(
            "run-1".to_string(),
            base_request("dev", false),
            Arc::clone(&config),
            graph,
            db,
            project_dir,
            &factory,
            &reporter,
            CancellationToken::new(),
        )
        .await
        .expect("run 1 must succeed");
    }

    // Hand-construct a LATER, COMPLETED manifest recording `solo` as
    // `skipped` — the shape a check-failure-driven downstream skip would
    // produce, without needing to build a real check fixture. An empty
    // `definition_hash` never matches the model's current compiled hash, so
    // this also exercises the "always safe to re-run" fallback.
    let file_store = FileStore::new(project_dir, "dev");
    let mut models = HashMap::new();
    models.insert(
        "solo".to_string(),
        smelt_state::ModelRunRecord {
            strategy: "skipped".to_string(),
            time_range: None,
            partitions_updated: vec![],
            row_count: 0,
            duration_ms: 0,
            batch_safety: Some("skipped".to_string()),
            outcome: RunOutcomeKind::Skipped,
            definition_hash: String::new(),
        },
    );
    let constructed = smelt_state::RunManifest {
        run_id: "99999999-999999-ffffff".to_string(),
        started_at: chrono::Utc::now(),
        completed_at: Some(chrono::Utc::now()),
        models,
    };
    file_store
        .save_run(&constructed)
        .expect("save constructed manifest");

    // ── Run 2: `--resume` must pick up the constructed manifest (its
    //    `completed_at` is set, but it overlaps the current selection and
    //    has a non-`success` outcome) rather than erroring "nothing to
    //    resume", and must re-run `solo`. ────────────────────────────────
    let call_log = Arc::new(Mutex::new(Vec::new()));
    let factory = SelectiveFailingBackendFactory {
        db_path: db_path.clone(),
        fail_once_for: Arc::new(Mutex::new(HashSet::new())),
        call_log: Arc::clone(&call_log),
    };
    let (db, graph) = build_db_and_graph(project_dir, &config);
    let outcome = execute_project(
        "run-2".to_string(),
        base_request("dev", true),
        config,
        graph,
        db,
        project_dir,
        &factory,
        &reporter,
        CancellationToken::new(),
    )
    .await
    .expect("--resume must pick up the completed-but-partial manifest, not error");

    assert_eq!(outcome.models["solo"].outcome, RunOutcomeKind::Success);
    assert!(
        call_log.lock().unwrap().contains(&"solo".to_string()),
        "'solo' must re-run — its prior outcome in the resumed-from manifest was 'skipped'"
    );
}

/// `--resume` refuses (hard error, not a warning or silent full run) when
/// there is nothing to resume from: either no run manifest exists yet, or
/// the most recent run completed successfully
/// (`docs/specs/run_state.md` §"`--resume` semantics").
#[tokio::test]
async fn resume_without_failed_run_is_error() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project_dir = tmp.path();
    std::fs::create_dir_all(project_dir.join("models")).unwrap();
    write_model(project_dir, "solo", "SELECT 1 AS id, 'a' AS val");

    let db_path = project_dir.join("db.duckdb");
    write_config(project_dir, &db_path);
    let config = make_config(&db_path);
    let factory = PlainDuckDbBackendFactory {
        db_path: db_path.clone(),
    };
    let reporter = NoOpReporter;

    // (a) No manifest exists at all yet.
    {
        let (db, graph) = build_db_and_graph(project_dir, &config);
        let result = execute_project(
            "run-resume-no-history".to_string(),
            base_request("dev", true),
            Arc::clone(&config),
            graph,
            db,
            project_dir,
            &factory,
            &reporter,
            CancellationToken::new(),
        )
        .await;
        let err = result.expect_err("--resume with no run history must be a hard error");
        assert!(
            err.to_string().contains("--resume"),
            "error should name --resume: {err}"
        );
    }

    // (b) A run has completed successfully — nothing left to resume.
    {
        let (db, graph) = build_db_and_graph(project_dir, &config);
        execute_project(
            "run-clean".to_string(),
            base_request("dev", false),
            Arc::clone(&config),
            graph,
            db,
            project_dir,
            &factory,
            &reporter,
            CancellationToken::new(),
        )
        .await
        .expect("plain run must succeed");
    }
    {
        let (db, graph) = build_db_and_graph(project_dir, &config);
        let result = execute_project(
            "run-resume-after-success".to_string(),
            base_request("dev", true),
            config,
            graph,
            db,
            project_dir,
            &factory,
            &reporter,
            CancellationToken::new(),
        )
        .await;
        let err = result.expect_err(
            "--resume after a successful run must be a hard error, not a silent full run",
        );
        assert!(
            err.to_string().contains("--resume"),
            "error should name --resume: {err}"
        );
    }
}
