//! Phase 6 TDD tests for bounded retry on transient backend errors.
//!
//! `docs/plans/20260719-prod-w2-operability.md` §"Phase 6: Bounded retry for
//! transient backend errors". Mirrors the fixture style of
//! `execute_parity.rs` (real DuckDB backend) and `parallel_execution.rs`
//! (delegating wrapper backend that injects failures around a real DuckDB
//! backend). `FailingBackend` fails `create_table_as` — the write step
//! `Backend::execute_model`'s default implementation uses for a full-refresh
//! table — a configurable number of times with `BackendError::ConnectionFailed`
//! (transient) before delegating to the real backend.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
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
use smelt_runtime::reporter::RunReporter;
use smelt_runtime::types::ExecuteRequest;
use tokio_util::sync::CancellationToken;

// ── Fixture helpers (mirrors execute_parity.rs / parallel_execution.rs) ────

fn write_model(project_dir: &Path, name: &str, content: &str) {
    let path = project_dir.join("models").join(format!("{}.sql", name));
    std::fs::write(path, content).expect("write model file");
}

fn write_config(project_dir: &Path, db_path: &Path) {
    let smelt_yml = format!(
        "name: retry_test\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: {db}\n    schema: main\ndefault_materialization: table\n",
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
        name: "retry_test".to_string(),
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

fn base_request(target: &str, retry_max: Option<u32>) -> ExecuteRequest {
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
        jobs: Some(1),
        retry_max,
        // Small so a test that actually sleeps through backoff stays fast;
        // the delay is still deterministic (`retry_backoff_delay`), not
        // wall-clock-derived, so this does not make the test flaky.
        retry_backoff_ms: Some(1),
        resume: false,
        technique_overrides: vec![],
    }
}

// ── CapturingReporter — records model_retrying events ───────────────────────

#[derive(Default)]
struct CapturingReporter {
    retries: Mutex<Vec<(String, u32, u32)>>,
}

impl CapturingReporter {
    fn retry_count(&self) -> usize {
        self.retries.lock().unwrap().len()
    }
}

impl RunReporter for CapturingReporter {
    fn model_retrying(
        &self,
        _run_id: &str,
        model: &str,
        attempt: u32,
        retry_max: u32,
        _error: &str,
    ) {
        self.retries
            .lock()
            .unwrap()
            .push((model.to_string(), attempt, retry_max));
    }
}

// ── FailingBackend — delegates to a real DuckDB backend, but fails the
//    first `fail_times` calls to `create_table_as` (the write step
//    `execute_model`'s default full-refresh path uses) with a transient
//    `BackendError::ConnectionFailed`. ─────────────────────────────────────

struct FailingBackend {
    inner: DuckDbBackend,
    remaining_failures: Arc<AtomicU32>,
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl Backend for FailingBackend {
    async fn execute_sql(&self, sql: &str) -> Result<Vec<RecordBatch>, BackendError> {
        self.inner.execute_sql(sql).await
    }

    async fn create_table_as(
        &self,
        schema: &str,
        name: &str,
        sql: &str,
    ) -> Result<(), BackendError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        loop {
            let remaining = self.remaining_failures.load(Ordering::SeqCst);
            if remaining == 0 {
                break;
            }
            if self
                .remaining_failures
                .compare_exchange(remaining, remaining - 1, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                return Err(BackendError::connection_failed(
                    "injected transient connection failure",
                ));
            }
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

struct FailingBackendFactory {
    db_path: std::path::PathBuf,
    remaining_failures: Arc<AtomicU32>,
    calls: Arc<AtomicUsize>,
}

impl BackendFactory for FailingBackendFactory {
    fn create<'a>(
        &'a self,
        _target_name: &'a str,
        target_config: &'a smelt_core::config::Target,
        _project_dir: &'a Path,
    ) -> BackendFuture<'a> {
        let path = self.db_path.clone();
        let schema = target_config.schema.clone();
        let remaining_failures = Arc::clone(&self.remaining_failures);
        let calls = Arc::clone(&self.calls);
        Box::pin(async move {
            let inner = DuckDbBackend::new(&path, &schema)
                .await
                .map_err(|e| anyhow::anyhow!("DuckDB init failed: {}", e))?;
            Ok(Box::new(FailingBackend {
                inner,
                remaining_failures,
                calls,
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

// ── Tests ────────────────────────────────────────────────────────────────

/// A backend that fails its first two `create_table_as` calls with a
/// transient `ConnectionFailed`, then succeeds. `retry_max` defaults to 3
/// (unset), which is enough to absorb both failures — the model still
/// completes, and the reporter records exactly 2 `model_retrying` events.
#[tokio::test]
async fn transient_failure_retries_then_succeeds() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project_dir = tmp.path();
    std::fs::create_dir_all(project_dir.join("models")).unwrap();
    write_model(project_dir, "flaky", "SELECT 1 AS id");

    let db_path = project_dir.join("db.duckdb");
    write_config(project_dir, &db_path);
    let config = make_config(&db_path);
    let (db, graph) = build_db_and_graph(project_dir, &config);

    let remaining_failures = Arc::new(AtomicU32::new(2));
    let calls = Arc::new(AtomicUsize::new(0));
    let factory = FailingBackendFactory {
        db_path,
        remaining_failures: Arc::clone(&remaining_failures),
        calls: Arc::clone(&calls),
    };
    let reporter = CapturingReporter::default();

    let outcome = execute_project(
        "run-transient".to_string(),
        base_request("dev", None),
        config,
        graph,
        db,
        project_dir,
        &factory,
        &reporter,
        CancellationToken::new(),
    )
    .await
    .expect("run must succeed after transient failures are retried");

    assert_eq!(outcome.models["flaky"].row_count, 1);
    assert_eq!(
        reporter.retry_count(),
        2,
        "expected exactly 2 retry events, got {:?}",
        reporter.retries.lock().unwrap()
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        3,
        "create_table_as should have been called 3 times (2 failures + 1 success)"
    );
    assert_eq!(remaining_failures.load(Ordering::SeqCst), 0);
}

/// A syntactically valid but semantically failing statement (casting a
/// non-numeric string literal to `INT`) fails deterministically — DuckDB
/// will reject it identically on every attempt, so it must not be retried
/// at all: the run fails on the first attempt and the reporter records zero
/// retry events.
#[tokio::test]
async fn sql_error_is_not_retried() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project_dir = tmp.path();
    std::fs::create_dir_all(project_dir.join("models")).unwrap();
    write_model(project_dir, "bad_cast", "SELECT CAST('x' AS INT) AS id");

    let db_path = project_dir.join("db.duckdb");
    write_config(project_dir, &db_path);
    let config = make_config(&db_path);
    let (db, graph) = build_db_and_graph(project_dir, &config);

    let factory = PlainDuckDbBackendFactory { db_path };
    let reporter = CapturingReporter::default();

    let result = execute_project(
        "run-sql-error".to_string(),
        base_request("dev", None),
        config,
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
        "a semantically invalid cast must fail the run, not silently succeed"
    );
    assert_eq!(
        reporter.retry_count(),
        0,
        "a deterministic SQL error must never be retried, got {:?}",
        reporter.retries.lock().unwrap()
    );
}

/// A backend that fails every `create_table_as` call forever. With
/// `retry_max` bounded at 2, the model exhausts its retry budget and the
/// run fails — but only after exactly 2 retry attempts (3 total calls: the
/// original plus 2 retries), never retrying forever.
#[tokio::test]
async fn retries_exhausted_fails_model() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project_dir = tmp.path();
    std::fs::create_dir_all(project_dir.join("models")).unwrap();
    write_model(project_dir, "always_flaky", "SELECT 1 AS id");

    let db_path = project_dir.join("db.duckdb");
    write_config(project_dir, &db_path);
    let config = make_config(&db_path);
    let (db, graph) = build_db_and_graph(project_dir, &config);

    // Effectively "never succeeds" for the bounded number of attempts this
    // test allows (retry_max: 2 means at most 3 total calls).
    let remaining_failures = Arc::new(AtomicU32::new(1000));
    let calls = Arc::new(AtomicUsize::new(0));
    let factory = FailingBackendFactory {
        db_path,
        remaining_failures: Arc::clone(&remaining_failures),
        calls: Arc::clone(&calls),
    };
    let reporter = CapturingReporter::default();

    let result = execute_project(
        "run-exhausted".to_string(),
        base_request("dev", Some(2)),
        config,
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
        "a model that never recovers must fail once its retry budget is exhausted"
    );
    assert_eq!(
        reporter.retry_count(),
        2,
        "expected exactly retry_max (2) retry events, got {:?}",
        reporter.retries.lock().unwrap()
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        3,
        "expected 3 total create_table_as calls (1 original + 2 retries)"
    );
}
