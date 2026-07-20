//! Real-fixture, DuckDB-backed coverage for key temporal locality's
//! **route 1** (key-embedded) target-scan slice pruning
//! (`docs/specs/incremental_models.md` §"Key temporal locality (the
//! time-partitioned output)"; `docs/plans/20260715-composed-axes-
//! conditional-maintenance.md` Phase A2).
//!
//! Fixture: `device_daily` is a per-`(device_id, event_date)` keyed
//! aggregate (`grain: key` + its own `timeseries:` block) over an
//! append-only `events` source. `event_date` — the model's own partition
//! column — is itself a `unique_key` column, so route 1 admits: every
//! stored row's partition value is its own key's value, and the model's SQL
//! carries no lookback construct, so the derived read margin is zero.
//!
//! A two-window run (day 1 alone, then days 2–3 together) proves two
//! things through the real `execute_project` pipeline
//! (`docs/specs/architecture.md` §"Run pipeline parity rule"):
//!
//! 1. **Slice pruning is real, not cosmetic.** Window 2's merge action(s)
//!    carry a target-scan slice predicate confined to their own step's
//!    date — day 1's date never appears in it, even though day 1's row is
//!    already stored in the same target table.
//! 2. **End-state equivalence.** After both windows, `device_daily` is
//!    multiset-equal to a full refresh of the model's own aggregation over
//!    every seeded row — the slice-pruned merge changes *what is scanned*,
//!    never *what is correct*.

use std::path::Path;
use std::sync::{Arc, Mutex};

use arrow::array::RecordBatch;
use arrow::datatypes::SchemaRef;
use async_trait::async_trait;

use smelt_backend::{Backend, BackendCapabilities, BackendError, PartitionRange, SqlDialect};
use smelt_backend_duckdb::DuckDbBackend;
use smelt_core::config::{Config, Target};
use smelt_core::graph::DependencyGraph;
use smelt_core::ModelDiscovery;
use smelt_runtime::execute::{execute_project, BackendFactory, BackendFuture};
use smelt_runtime::types::ExecuteRequest;
use tokio_util::sync::CancellationToken;

/// Wraps a real [`DuckDbBackend`], delegating every call, but recording
/// every raw SQL string handed to [`Backend::execute_sql`] AND every
/// `action_sql` handed to [`Backend::fold_ledger_delta`] (the additive-fold
/// ledger path `SUM`-family combiners take —
/// `docs/specs/incremental_models.md` §"The reconciliation ledger" — which
/// bypasses `execute_statement_group` entirely). Between the two, every
/// MERGE this run actually executes is captured.
struct RecordingBackend {
    inner: DuckDbBackend,
    executed_sql: Mutex<Vec<String>>,
}

impl RecordingBackend {
    fn new(inner: DuckDbBackend) -> Self {
        Self {
            inner,
            executed_sql: Mutex::new(Vec::new()),
        }
    }

    fn merge_statements(&self) -> Vec<String> {
        self.executed_sql
            .lock()
            .unwrap()
            .iter()
            .filter(|s| s.starts_with("MERGE INTO") || s.contains(" MERGE INTO"))
            .cloned()
            .collect()
    }
}

#[async_trait]
impl Backend for RecordingBackend {
    async fn execute_sql(&self, sql: &str) -> Result<Vec<RecordBatch>, BackendError> {
        self.executed_sql.lock().unwrap().push(sql.to_string());
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
    // `fold_ledger_delta`'s default implementation (`smelt-backend`) routes
    // every one of its four statements through `self.execute_sql` — so the
    // override above already captures `action_sql` (the actual MERGE) as
    // long as this method itself is not separately overridden here to skip
    // it. No override needed: `DuckDbBackend` does not override
    // `fold_ledger_delta`, so the trait default (calling back through this
    // struct's own `execute_sql`) is what runs. Left unimplemented
    // deliberately — see the doc comment above.
}

struct RecordingBackendFactory {
    db_path: std::path::PathBuf,
    backend: Arc<Mutex<Option<Arc<RecordingBackend>>>>,
}

impl BackendFactory for RecordingBackendFactory {
    fn create<'a>(
        &'a self,
        _target_name: &'a str,
        target_config: &'a Target,
        _project_dir: &'a Path,
    ) -> BackendFuture<'a> {
        let path = self.db_path.clone();
        let schema = target_config.schema.clone();
        let slot = Arc::clone(&self.backend);
        Box::pin(async move {
            let inner = DuckDbBackend::new(&path, &schema)
                .await
                .map_err(|e| anyhow::anyhow!("DuckDB init failed: {}", e))?;
            let recording = Arc::new(RecordingBackend::new(inner));
            *slot.lock().unwrap() = Some(Arc::clone(&recording));
            Ok(Box::new(ArcBackend(recording)) as Box<dyn Backend>)
        })
    }
}

/// Thin `Backend` forwarder over an `Arc<RecordingBackend>` so the same
/// instance can be returned to `execute_project` (which needs ownership)
/// while the test keeps its own handle to read the recording back.
struct ArcBackend(Arc<RecordingBackend>);

#[async_trait]
impl Backend for ArcBackend {
    async fn execute_sql(&self, sql: &str) -> Result<Vec<RecordBatch>, BackendError> {
        self.0.execute_sql(sql).await
    }
    async fn create_table_as(
        &self,
        schema: &str,
        name: &str,
        sql: &str,
    ) -> Result<(), BackendError> {
        self.0.create_table_as(schema, name, sql).await
    }
    async fn create_view_as(
        &self,
        schema: &str,
        name: &str,
        sql: &str,
    ) -> Result<(), BackendError> {
        self.0.create_view_as(schema, name, sql).await
    }
    async fn drop_table_if_exists(&self, schema: &str, name: &str) -> Result<(), BackendError> {
        self.0.drop_table_if_exists(schema, name).await
    }
    async fn drop_view_if_exists(&self, schema: &str, name: &str) -> Result<(), BackendError> {
        self.0.drop_view_if_exists(schema, name).await
    }
    async fn get_row_count(&self, schema: &str, name: &str) -> Result<usize, BackendError> {
        self.0.get_row_count(schema, name).await
    }
    async fn get_preview(
        &self,
        schema: &str,
        name: &str,
        limit: usize,
    ) -> Result<Vec<RecordBatch>, BackendError> {
        self.0.get_preview(schema, name, limit).await
    }
    async fn table_exists(&self, schema: &str, name: &str) -> Result<bool, BackendError> {
        self.0.table_exists(schema, name).await
    }
    async fn ensure_schema(&self, schema: &str) -> Result<(), BackendError> {
        self.0.ensure_schema(schema).await
    }
    fn dialect(&self) -> SqlDialect {
        self.0.dialect()
    }
    fn capabilities(&self) -> BackendCapabilities {
        self.0.capabilities()
    }
    async fn load_table(
        &self,
        schema: &str,
        name: &str,
        arrow_schema: SchemaRef,
        batches: Vec<RecordBatch>,
    ) -> Result<(), BackendError> {
        self.0.load_table(schema, name, arrow_schema, batches).await
    }
    async fn delete_partitions(
        &self,
        schema: &str,
        name: &str,
        partition: &PartitionRange,
    ) -> Result<(), BackendError> {
        self.0.delete_partitions(schema, name, partition).await
    }
    async fn insert_into_from_query(
        &self,
        schema: &str,
        name: &str,
        sql: &str,
    ) -> Result<(), BackendError> {
        self.0.insert_into_from_query(schema, name, sql).await
    }
    async fn insert_overwrite(
        &self,
        schema: &str,
        table: &str,
        sql: &str,
        partition: &PartitionRange,
    ) -> Result<(), BackendError> {
        self.0.insert_overwrite(schema, table, sql, partition).await
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

    // Route 1 (key-embedded): `event_date` is both the model's own
    // `timeseries.partition_column` and a `unique_key` column (GROUP BY 1,
    // 2) — the composed shape (`docs/specs/incremental_models.md` §"Key
    // temporal locality"). No lookback construct in the SQL, so the
    // derived read margin is zero: the slice for a given step is exactly
    // that step's own date.
    let model_sql = r#"---
materialization: table
refresh: incremental
grain: key
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
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
        "name: locality_route1_test\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: {db}\n    schema: main\ndefault_materialization: table\n",
        db = db_path.display()
    );
    std::fs::write(project_dir.join("smelt.yml"), smelt_yml).unwrap();
}

/// Seed `main.sources_events` with rows on day 1 (2026-01-01, device 1) and
/// days 2–3 (2026-01-02, 2026-01-03; devices 1 and 2) — enough for the
/// second run window to touch both an existing key (device 1) and a
/// brand-new one (device 2).
fn seed_tables(db_path: &Path) -> anyhow::Result<()> {
    let conn = duckdb::Connection::open(db_path)?;
    conn.execute_batch(
        r#"
        CREATE SCHEMA IF NOT EXISTS main;
        CREATE OR REPLACE TABLE main.sources_events AS
        SELECT * FROM (VALUES
            (1, DATE '2026-01-01', 10.0),
            (1, DATE '2026-01-02', 20.0),
            (2, DATE '2026-01-02', 5.0),
            (1, DATE '2026-01-03', 30.0),
            (2, DATE '2026-01-03', 7.0)
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

fn multiset_equal(db_path: &Path, left_sql: &str, right_sql: &str) -> bool {
    let conn = duckdb::Connection::open(db_path).unwrap();
    let except1: i64 = conn
        .query_row(
            &format!("SELECT COUNT(*) FROM ({left_sql} EXCEPT ALL {right_sql})"),
            [],
            |r| r.get(0),
        )
        .unwrap();
    let except2: i64 = conn
        .query_row(
            &format!("SELECT COUNT(*) FROM ({right_sql} EXCEPT ALL {left_sql})"),
            [],
            |r| r.get(0),
        )
        .unwrap();
    except1 == 0 && except2 == 0
}

/// The two-window proof described in the module doc comment: window 2's
/// merge(s) carry a target-scan slice confined to their own dates (never
/// day 1's), and the end state after both windows equals a full refresh.
#[tokio::test]
async fn route1_slice_pruning_excludes_prior_window_and_matches_full_refresh() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().to_path_buf();
    let db_path = project_dir.join("dev.duckdb");

    stage_project(&project_dir, &db_path);
    seed_tables(&db_path).expect("seed tables");

    let config = Arc::new(Config::load(&project_dir).expect("load config"));
    let (db, graph) = build_db_and_graph(&project_dir, &config);

    // Window 1: day 1 alone — creates the target table.
    let backend_slot1: Arc<Mutex<Option<Arc<RecordingBackend>>>> = Arc::new(Mutex::new(None));
    let factory1 = RecordingBackendFactory {
        db_path: db_path.clone(),
        backend: Arc::clone(&backend_slot1),
    };
    execute_project(
        "locality-route1-window1".to_string(),
        run_request("2026-01-01", "2026-01-02"),
        Arc::clone(&config),
        Arc::clone(&graph),
        Arc::clone(&db),
        &project_dir,
        &factory1,
        &smelt_runtime::NoOpReporter,
        CancellationToken::new(),
    )
    .await
    .expect("window 1 (create) must run");

    // Window 2: days 2-3 — two merge steps into the already-populated table.
    let backend_slot2: Arc<Mutex<Option<Arc<RecordingBackend>>>> = Arc::new(Mutex::new(None));
    let factory2 = RecordingBackendFactory {
        db_path: db_path.clone(),
        backend: Arc::clone(&backend_slot2),
    };
    execute_project(
        "locality-route1-window2".to_string(),
        run_request("2026-01-02", "2026-01-04"),
        Arc::clone(&config),
        Arc::clone(&graph),
        Arc::clone(&db),
        &project_dir,
        &factory2,
        &smelt_runtime::NoOpReporter,
        CancellationToken::new(),
    )
    .await
    .expect("window 2 (merge) must run");

    let backend2 = backend_slot2
        .lock()
        .unwrap()
        .clone()
        .expect("backend recorded");
    let merges = backend2.merge_statements();
    assert_eq!(
        merges.len(),
        2,
        "window 2 covers two day-steps, each its own MERGE: {:?}",
        merges
    );

    // Each step's MERGE carries the slice predicate confined to its OWN
    // date — day 1 (2026-01-01) must never appear in either, even though
    // device 1's day-1 row already sits in the target table.
    for (idx, merge_sql) in merges.iter().enumerate() {
        assert!(
            merge_sql.contains("target.event_date BETWEEN"),
            "step {idx} MERGE must carry the target-scan slice predicate: {merge_sql}"
        );
        assert!(
            !merge_sql.contains("2026-01-01"),
            "step {idx} MERGE must not reference window 1's date \
             (would mean the target scan was not pruned): {merge_sql}"
        );
    }
    assert!(
        merges[0].contains("BETWEEN '2026-01-02' AND '2026-01-02'"),
        "step 0 (2026-01-02) slice must be exactly its own date (zero margin): {}",
        merges[0]
    );
    assert!(
        merges[1].contains("BETWEEN '2026-01-03' AND '2026-01-03'"),
        "step 1 (2026-01-03) slice must be exactly its own date (zero margin): {}",
        merges[1]
    );

    // End-state equivalence: the two-window run must equal a full refresh
    // of the model's own aggregation over every seeded row.
    assert!(
        multiset_equal(
            &db_path,
            "SELECT device_id, event_date, total_amount FROM main.device_daily",
            "SELECT device_id, event_date, SUM(amount) AS total_amount \
             FROM main.sources_events GROUP BY 1, 2"
        ),
        "device_daily must equal a full refresh of the aggregation"
    );
}
