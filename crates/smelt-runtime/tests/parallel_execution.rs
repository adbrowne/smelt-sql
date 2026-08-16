//! Phase 5 TDD tests for the DAG-parallel scheduler (`--jobs`).
//!
//! `docs/plans/20260719-prod-w2-operability.md` §"Phase 5: DAG-parallel
//! execution with `--jobs`". Uses a real DuckDB backend for correctness
//! assertions and a delegating "delaying" backend (artificial `sleep`
//! injected before the real DuckDB call) to prove models actually run
//! concurrently in wall-clock time when `jobs > 1` — a assertion that is
//! only true once the sequential loop is replaced by a wavefront scheduler.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use arrow::array::RecordBatch;
use arrow::datatypes::SchemaRef;
use async_trait::async_trait;
use smelt_backend::{Backend, BackendCapabilities, BackendError, PartitionRange, SqlDialect};
use smelt_backend_duckdb::DuckDbBackend;
use smelt_core::config::{Config, Materialization as CfgMaterialization, ModelConfig, Target};
use smelt_core::graph::DependencyGraph;
use smelt_core::ModelDiscovery;
use smelt_runtime::execute::{BackendFactory, BackendFuture};
use smelt_runtime::reporter::RunReporter;
use smelt_runtime::types::{ExecuteRequest, RunOutcome};
use smelt_runtime::{execute_project, NoOpReporter};
use tokio_util::sync::CancellationToken;

// ── Fixture helpers (mirrors execute_parity.rs) ────────────────────────────

fn write_model(project_dir: &Path, name: &str, content: &str) {
    let path = project_dir.join("models").join(format!("{}.sql", name));
    std::fs::write(path, content).expect("write model file");
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

fn base_request(target: &str, jobs: Option<usize>) -> ExecuteRequest {
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
        jobs,
        retry_max: None,
        retry_backoff_ms: None,
        resume: false,
        technique_overrides: vec![],
        keyed_restrictions: std::collections::BTreeMap::new(),
    }
}

fn write_config(project_dir: &Path, db_path: &Path) {
    let smelt_yml = format!(
        "name: parallel_test\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: {db}\n    schema: main\ndefault_materialization: table\n",
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
        name: "parallel_test".to_string(),
        version: 1,
        paths: vec!["models".to_string()],
        targets,
        default_materialization: CfgMaterialization::Table,
        models: HashMap::<String, ModelConfig>::new(),
        python: None,
        target: None,
        state: Default::default(),
        maintenance: None,
        probes: Default::default(),
    })
}

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

fn assert_outcomes_equivalent(a: &RunOutcome, b: &RunOutcome, label: &str) {
    let mut a_keys: Vec<&String> = a.models.keys().collect();
    let mut b_keys: Vec<&String> = b.models.keys().collect();
    a_keys.sort();
    b_keys.sort();
    assert_eq!(
        a_keys, b_keys,
        "[{label}] model keys differ: a={a_keys:?} b={b_keys:?}"
    );
    for key in &a_keys {
        let ra = &a.models[*key];
        let rb = &b.models[*key];
        assert_eq!(
            ra.row_count, rb.row_count,
            "[{label}] row_count differs for model '{key}'"
        );
        assert_eq!(
            ra.strategy, rb.strategy,
            "[{label}] strategy differs for model '{key}'"
        );
    }
}

// ── Test 1: --jobs 1 parity ────────────────────────────────────────────────

#[tokio::test]
async fn jobs_1_report_identical_to_default_pipeline() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project_dir = tmp.path();
    std::fs::create_dir_all(project_dir.join("models")).unwrap();

    write_model(project_dir, "base", "SELECT 1 AS id, 'hello' AS label");
    write_model(project_dir, "derived", "SELECT id, label FROM smelt.base");
    write_model(project_dir, "leaf_a", "SELECT id FROM smelt.derived");
    write_model(project_dir, "leaf_b", "SELECT id FROM smelt.derived");

    let db1 = project_dir.join("run1.duckdb");
    let db2 = project_dir.join("run2.duckdb");

    write_config(project_dir, &db1);
    let config1 = make_config(&db1);
    let (db_arc1, graph_arc1) = build_db_and_graph(project_dir, &config1);
    let outcome_jobs1 = execute_project(
        "run-jobs1".to_string(),
        base_request("dev", Some(1)),
        Arc::clone(&config1),
        graph_arc1,
        db_arc1,
        project_dir,
        &DuckDbBackendFactory { db_path: db1 },
        &NoOpReporter,
        CancellationToken::new(),
    )
    .await
    .expect("--jobs 1 run must succeed");

    write_config(project_dir, &db2);
    let config2 = make_config(&db2);
    let (db_arc2, graph_arc2) = build_db_and_graph(project_dir, &config2);
    let outcome_default = execute_project(
        "run-default".to_string(),
        base_request("dev", None),
        Arc::clone(&config2),
        graph_arc2,
        db_arc2,
        project_dir,
        &DuckDbBackendFactory { db_path: db2 },
        &NoOpReporter,
        CancellationToken::new(),
    )
    .await
    .expect("default (available-parallelism) run must succeed");

    assert_outcomes_equivalent(&outcome_jobs1, &outcome_default, "jobs1-vs-default");
    assert_eq!(outcome_jobs1.total_rows, outcome_default.total_rows);
}

// ── Test 2: upstream always completes before downstream ───────────────────

#[derive(Default)]
struct TimestampReporter {
    started: Mutex<HashMap<String, Instant>>,
    completed: Mutex<HashMap<String, Instant>>,
}

impl RunReporter for TimestampReporter {
    fn model_started(&self, _run_id: &str, model: &str, _idx: usize, _total: usize) {
        self.started
            .lock()
            .unwrap()
            .insert(model.to_string(), Instant::now());
    }

    fn model_completed(&self, _run_id: &str, model: &str, _rows: usize, _dur: Duration) {
        self.completed
            .lock()
            .unwrap()
            .insert(model.to_string(), Instant::now());
    }
}

#[tokio::test]
async fn upstream_always_completes_before_downstream() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project_dir = tmp.path();
    std::fs::create_dir_all(project_dir.join("models")).unwrap();

    // Diamond: root -> {left, right} -> sink, plus an independent chain
    // x -> y to widen the DAG.
    write_model(project_dir, "root", "SELECT 1 AS id");
    write_model(project_dir, "branch_a", "SELECT id FROM smelt.root");
    write_model(project_dir, "branch_b", "SELECT id FROM smelt.root");
    write_model(
        project_dir,
        "sink",
        "SELECT l.id FROM smelt.branch_a AS l JOIN smelt.branch_b AS r USING (id)",
    );
    write_model(project_dir, "x", "SELECT 2 AS id");
    write_model(project_dir, "y", "SELECT id FROM smelt.x");

    let db_path = project_dir.join("run.duckdb");
    write_config(project_dir, &db_path);
    let config = make_config(&db_path);
    let (db_arc, graph_arc) = build_db_and_graph(project_dir, &config);

    let reporter = TimestampReporter::default();
    execute_project(
        "run-order".to_string(),
        base_request("dev", Some(4)),
        config,
        graph_arc,
        db_arc,
        project_dir,
        &DuckDbBackendFactory { db_path },
        &reporter,
        CancellationToken::new(),
    )
    .await
    .expect("run must succeed");

    let started = reporter.started.lock().unwrap();
    let completed = reporter.completed.lock().unwrap();

    let edges: &[(&str, &str)] = &[
        ("root", "branch_a"),
        ("root", "branch_b"),
        ("branch_a", "sink"),
        ("branch_b", "sink"),
        ("x", "y"),
    ];
    for (up, down) in edges {
        let up_completed = completed
            .get(*up)
            .unwrap_or_else(|| panic!("no model_completed for '{up}'"));
        let down_started = started
            .get(*down)
            .unwrap_or_else(|| panic!("no model_started for '{down}'"));
        assert!(
            up_completed <= down_started,
            "DAG edge {up} -> {down} violated: {up} completed at {up_completed:?}, \
             {down} started at {down_started:?}"
        );
    }
}

// ── Test 3: deterministic reporter/manifest ordering across runs ──────────

#[derive(Default)]
struct OrderCapturingReporter {
    order: Mutex<Vec<String>>,
}

impl RunReporter for OrderCapturingReporter {
    fn model_started(&self, _run_id: &str, model: &str, _idx: usize, _total: usize) {
        self.order.lock().unwrap().push(model.to_string());
    }
}

#[tokio::test]
async fn report_order_deterministic_across_runs() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project_dir = tmp.path();
    std::fs::create_dir_all(project_dir.join("models")).unwrap();

    write_model(project_dir, "root", "SELECT 1 AS id");
    write_model(project_dir, "branch_a", "SELECT id FROM smelt.root");
    write_model(project_dir, "branch_b", "SELECT id FROM smelt.root");
    write_model(
        project_dir,
        "sink",
        "SELECT l.id FROM smelt.branch_a AS l JOIN smelt.branch_b AS r USING (id)",
    );

    // Build the graph/Salsa-DB ONCE and reuse it (via `Arc::clone`) across
    // both runs. `DependencyGraph::execution_order` breaks same-in-degree
    // ties via `HashMap` iteration, whose relative order is stable across
    // repeated calls on the SAME `HashMap` instance but is *not* guaranteed
    // identical between two independently-constructed instances of an
    // otherwise-equal graph (a pre-existing property of `execution_order`,
    // unrelated to the scheduler under test) — so this test fixes the
    // baseline `execution_order` by construction and asserts the
    // scheduler's *own* flush ordering is deterministic relative to it,
    // which is the guarantee `--jobs` actually promises.
    let db_path = project_dir.join("shared.duckdb");
    write_config(project_dir, &db_path);
    let config = make_config(&db_path);
    let (db_arc, graph_arc) = build_db_and_graph(project_dir, &config);

    async fn run_once(
        project_dir: &Path,
        run_id: &str,
        config: Arc<Config>,
        graph_arc: Arc<tokio::sync::Mutex<DependencyGraph>>,
        db_arc: Arc<tokio::sync::Mutex<smelt_db::Database>>,
    ) -> Vec<String> {
        let db_path = project_dir.join(format!("{run_id}.duckdb"));
        write_config(project_dir, &db_path);
        let reporter = OrderCapturingReporter::default();
        execute_project(
            run_id.to_string(),
            base_request("dev", Some(4)),
            config,
            graph_arc,
            db_arc,
            project_dir,
            &DuckDbBackendFactory { db_path },
            &reporter,
            CancellationToken::new(),
        )
        .await
        .expect("run must succeed");
        reporter.order.into_inner().unwrap()
    }

    let order_a = run_once(
        project_dir,
        "run-a",
        Arc::clone(&config),
        Arc::clone(&graph_arc),
        Arc::clone(&db_arc),
    )
    .await;
    let order_b = run_once(project_dir, "run-b", config, graph_arc, db_arc).await;

    assert_eq!(
        order_a, order_b,
        "reporter model_started order must be deterministic across --jobs 4 runs"
    );
    // Sanity: every model appeared exactly once.
    assert_eq!(order_a.len(), 4);
}

// ── Test 4: models actually overlap in wall-clock time under --jobs > 1 ───
//
// The other three tests would pass even against a purely sequential loop
// that merely accepts (and ignores) the `jobs` field. This test proves real
// concurrency: a delegating backend injects an artificial `sleep` before
// each `create_table_as` call (outside the real DuckDB connection mutex) and
// records the sleep's [start, end) window per model. Two independent models
// (`indep_a`, `indep_b`, no edge between them) must have overlapping windows
// when `jobs >= 2` — impossible under a strictly sequential per-model loop.

struct DelayingBackend {
    inner: DuckDbBackend,
    delay: Duration,
    windows: Arc<Mutex<HashMap<String, (Instant, Instant)>>>,
}

#[async_trait]
impl Backend for DelayingBackend {
    async fn execute_sql(&self, sql: &str) -> Result<Vec<RecordBatch>, BackendError> {
        self.inner.execute_sql(sql).await
    }

    async fn create_table_as(
        &self,
        schema: &str,
        name: &str,
        sql: &str,
    ) -> Result<(), BackendError> {
        let start = Instant::now();
        tokio::time::sleep(self.delay).await;
        let end = Instant::now();
        self.windows
            .lock()
            .unwrap()
            .insert(name.to_string(), (start, end));
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

struct DelayingBackendFactory {
    db_path: std::path::PathBuf,
    delay: Duration,
    windows: Arc<Mutex<HashMap<String, (Instant, Instant)>>>,
}

impl BackendFactory for DelayingBackendFactory {
    fn create<'a>(
        &'a self,
        _target_name: &'a str,
        target_config: &'a smelt_core::config::Target,
        _project_dir: &'a Path,
    ) -> BackendFuture<'a> {
        let path = self.db_path.clone();
        let schema = target_config.schema.clone();
        let delay = self.delay;
        let windows = Arc::clone(&self.windows);
        Box::pin(async move {
            let inner = DuckDbBackend::new(&path, &schema)
                .await
                .map_err(|e| anyhow::anyhow!("DuckDB init failed: {}", e))?;
            Ok(Box::new(DelayingBackend {
                inner,
                delay,
                windows,
            }) as Box<dyn Backend>)
        })
    }
}

#[tokio::test]
async fn independent_models_overlap_in_wall_clock_when_jobs_gt_1() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project_dir = tmp.path();
    std::fs::create_dir_all(project_dir.join("models")).unwrap();

    // No edge between indep_a and indep_b — same wave, safe to run
    // concurrently.
    write_model(project_dir, "indep_a", "SELECT 1 AS id");
    write_model(project_dir, "indep_b", "SELECT 2 AS id");

    let db_path = project_dir.join("run.duckdb");
    write_config(project_dir, &db_path);
    let config = make_config(&db_path);
    let (db_arc, graph_arc) = build_db_and_graph(project_dir, &config);

    let windows = Arc::new(Mutex::new(HashMap::new()));
    let factory = DelayingBackendFactory {
        db_path,
        delay: Duration::from_millis(200),
        windows: Arc::clone(&windows),
    };

    execute_project(
        "run-overlap".to_string(),
        base_request("dev", Some(2)),
        config,
        graph_arc,
        db_arc,
        project_dir,
        &factory,
        &NoOpReporter,
        CancellationToken::new(),
    )
    .await
    .expect("run must succeed");

    let windows = windows.lock().unwrap();
    let (a_start, a_end) = windows["indep_a"];
    let (b_start, b_end) = windows["indep_b"];
    assert!(
        a_start < b_end && b_start < a_end,
        "indep_a [{a_start:?}, {a_end:?}) and indep_b [{b_start:?}, {b_end:?}) \
         must overlap under --jobs 2 (they have no dependency edge)"
    );
}
