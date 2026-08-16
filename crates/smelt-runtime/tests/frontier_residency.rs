//! State residency phase 4 (`docs/outcomes/20260816-state-residency/
//! phases/04-plan.md`; `docs/specs/incremental_models.md` §"The frontier
//! record (reconciliation ledger)"): the reconciliation ledger's frontier
//! (idempotent) grading moves engine-resident, off `.smelt/`. Real-fixture,
//! DuckDB-backed coverage exercised through `execute_project`.

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use arrow::array::RecordBatch;
use arrow::datatypes::SchemaRef;
use async_trait::async_trait;
use smelt_backend::{
    Backend, BackendCapabilities, BackendError, PartitionRange, SqlDialect, StatementGroup,
};
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

/// Same shape as [`REVENUE_MODEL_SQL`], but with rows spread across three
/// distinct `revenue_date` days (2024-12-25/26/27) so a 1-day-batched run
/// over the full window produces three distinct per-batch regions.
const REVENUE_MULTI_DAY_MODEL_SQL: &str = r#"---
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
    (2, 200.00, TIMESTAMP '2024-12-25 14:00:00'),
    (3, 300.00, TIMESTAMP '2024-12-26 09:00:00'),
    (4, 400.00, TIMESTAMP '2024-12-27 11:00:00')
) AS t(user_id, amount, transaction_timestamp)
GROUP BY 1, 2
"#;

fn setup_project(tmp: &tempfile::TempDir) -> (std::path::PathBuf, Config) {
    setup_project_with_sql(tmp, REVENUE_MODEL_SQL)
}

fn setup_project_with_sql(
    tmp: &tempfile::TempDir,
    model_sql: &str,
) -> (std::path::PathBuf, Config) {
    let project_dir = tmp.path().to_path_buf();
    std::fs::create_dir_all(project_dir.join("models")).unwrap();
    std::fs::write(project_dir.join("models").join("revenue.sql"), model_sql).unwrap();

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
        keyed_restrictions: std::collections::BTreeMap::new(),
    }
}

fn make_request_batched(start: &str, end: &str, batch_size_days: u32) -> ExecuteRequest {
    ExecuteRequest {
        batch_size_days: Some(batch_size_days),
        ..make_request(start, end)
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

// ── Phase 7: fuse the frontier reset into the region recompute's own write
//    transaction (`docs/outcomes/20260816-state-residency/phases/
//    07-plan.md`). ──────────────────────────────────────────────────────

async fn frontier_region_starts(backend: &DuckDbBackend, model: &str) -> Vec<String> {
    let batches = backend
        .execute_sql(&format!(
            "SELECT region_start FROM main._smelt_frontier \
             WHERE model_name = '{model}' ORDER BY region_start"
        ))
        .await
        .expect("query frontier table");
    let mut out = Vec::new();
    for batch in &batches {
        use arrow::array::{Array, StringArray};
        let col = batch
            .column(0)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        for i in 0..batch.num_rows() {
            out.push(col.value(i).to_string());
        }
    }
    out
}

/// An existing target (created by a prior bootstrap run) recomputed over a
/// 3-day window with 1-day batches yields three `_smelt_frontier` rows, one
/// per recomputed batch region — not one whole-range row.
#[tokio::test]
async fn frontier_record_is_written_per_batch_region() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (project_dir, config) = setup_project_with_sql(&tmp, REVENUE_MULTI_DAY_MODEL_SQL);
    let config = Arc::new(config);
    let (db, graph) = build_db_and_graph(&project_dir, &config);
    let db_path = project_dir.join("run.duckdb");
    let factory = DuckDbBackendFactory {
        db_path: db_path.clone(),
    };

    // Bootstrap: creates the target over day 25 only.
    execute_project(
        "run-1".to_string(),
        make_request("2024-12-25", "2024-12-26"),
        Arc::clone(&config),
        Arc::clone(&graph),
        Arc::clone(&db),
        &project_dir,
        &factory,
        &NoOpReporter,
        CancellationToken::new(),
    )
    .await
    .expect("bootstrap run must succeed");

    // Second run over the full 3-day window, 1-day batches: the target
    // already exists for every batch, so every batch fuses its write with
    // its own frontier reset.
    execute_project(
        "run-2".to_string(),
        make_request_batched("2024-12-25", "2024-12-28", 1),
        Arc::clone(&config),
        Arc::clone(&graph),
        Arc::clone(&db),
        &project_dir,
        &factory,
        &NoOpReporter,
        CancellationToken::new(),
    )
    .await
    .expect("batched recompute run must succeed");

    let backend = DuckDbBackend::new(&db_path, "main")
        .await
        .expect("reopen duckdb");
    let region_starts = frontier_region_starts(&backend, "revenue").await;
    assert_eq!(
        region_starts,
        vec!["2024-12-25", "2024-12-26", "2024-12-27"],
        "expected one frontier row per recomputed batch region, not one whole-range row"
    );
}

/// The first run (table created by `CREATE TABLE AS`, no existing target to
/// fuse against) still records the whole-range frontier entry via the
/// retained after-the-loop path.
#[tokio::test]
async fn bootstrap_run_still_records_the_frontier() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (project_dir, config) = setup_project_with_sql(&tmp, REVENUE_MULTI_DAY_MODEL_SQL);
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
    .expect("bootstrap run must succeed");

    let backend = DuckDbBackend::new(&db_path, "main")
        .await
        .expect("reopen duckdb");
    let region_starts = frontier_region_starts(&backend, "revenue").await;
    assert_eq!(
        region_starts,
        vec!["2024-12-25"],
        "the bootstrap CREATE TABLE AS run has no existing target to fuse against, so it \
         keeps the after-the-loop whole-range frontier record"
    );
}

// ── FailingFrontierBackend — delegates to a real DuckDB backend, but fails
//    the Nth call to `execute_write_and_reset_frontier` with a
//    non-transient error, simulating a batch write that never commits. ────

struct FailingFrontierBackend {
    inner: DuckDbBackend,
    fail_at_call: usize,
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl Backend for FailingFrontierBackend {
    async fn execute_sql(&self, sql: &str) -> Result<Vec<RecordBatch>, BackendError> {
        self.inner.execute_sql(sql).await
    }

    async fn create_table_as(
        &self,
        schema: &str,
        name: &str,
        sql: &str,
    ) -> Result<(), BackendError> {
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

    async fn execute_write_and_reset_frontier(
        &self,
        ensure_sql: &str,
        write_group: &StatementGroup,
        reset_delete_sql: &str,
        insert_sql: &str,
    ) -> Result<(), BackendError> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        if call == self.fail_at_call {
            return Err(BackendError::execution_failed(
                "revenue",
                "injected frontier-reset failure",
            ));
        }
        self.inner
            .execute_write_and_reset_frontier(ensure_sql, write_group, reset_delete_sql, insert_sql)
            .await
    }
}

struct FailingFrontierBackendFactory {
    db_path: std::path::PathBuf,
    fail_at_call: usize,
    calls: Arc<AtomicUsize>,
}

impl BackendFactory for FailingFrontierBackendFactory {
    fn create<'a>(
        &'a self,
        _target_name: &'a str,
        target_config: &'a smelt_core::config::Target,
        _project_dir: &'a Path,
    ) -> BackendFuture<'a> {
        let path = self.db_path.clone();
        let schema = target_config.schema.clone();
        let fail_at_call = self.fail_at_call;
        let calls = Arc::clone(&self.calls);
        Box::pin(async move {
            let backend = DuckDbBackend::new(&path, &schema)
                .await
                .map_err(|e| anyhow::anyhow!("DuckDB init failed: {}", e))?;
            Ok(Box::new(FailingFrontierBackend {
                inner: backend,
                fail_at_call,
                calls,
            }) as Box<dyn Backend>)
        })
    }
}

/// A batch write that fails (here, at the fused frontier-reset transaction)
/// rolls back atomically: an earlier batch's already-committed fused write
/// keeps its own frontier row, but the failed batch's region gets none, and
/// a batch after it never runs.
#[tokio::test]
async fn failed_batch_write_records_no_frontier_row() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (project_dir, config) = setup_project_with_sql(&tmp, REVENUE_MULTI_DAY_MODEL_SQL);
    let config = Arc::new(config);
    let (db, graph) = build_db_and_graph(&project_dir, &config);
    let db_path = project_dir.join("run.duckdb");

    // Bootstrap: creates the target over day 25 only, unfused.
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
    .expect("bootstrap run must succeed");

    // Second run: 3 fused batches (25, 26, 27) over the now-existing
    // target; fail the SECOND fused `execute_write_and_reset_frontier`
    // call (batch region 2024-12-26..27).
    let calls = Arc::new(AtomicUsize::new(0));
    let result = execute_project(
        "run-2".to_string(),
        make_request_batched("2024-12-25", "2024-12-28", 1),
        Arc::clone(&config),
        Arc::clone(&graph),
        Arc::clone(&db),
        &project_dir,
        &FailingFrontierBackendFactory {
            db_path: db_path.clone(),
            fail_at_call: 2,
            calls,
        },
        &NoOpReporter,
        CancellationToken::new(),
    )
    .await;
    assert!(result.is_err(), "the injected failure must fail the run");

    let backend = DuckDbBackend::new(&db_path, "main")
        .await
        .expect("reopen duckdb");
    let region_starts = frontier_region_starts(&backend, "revenue").await;
    assert_eq!(
        region_starts,
        vec!["2024-12-25"],
        "only the first (already-committed) fused batch's region has a frontier row; the \
         failed batch's region has none, and the third batch never ran"
    );

    let table_rows = backend
        .execute_sql("SELECT COUNT(*) AS n FROM main.revenue")
        .await
        .expect("query revenue table");
    let n: usize = {
        use arrow::array::{Array, Int64Array};
        table_rows[0]
            .column(0)
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap()
            .value(0) as usize
    };
    assert_eq!(
        n, 2,
        "only day 25's 2 rows were ever committed — the failed batch's day-26 write rolled back"
    );
}
