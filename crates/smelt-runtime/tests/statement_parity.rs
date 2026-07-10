//! Statement-parity CI gate (`docs/specs/architecture.md` §"Constraints &
//! Invariants" item 12; `docs/specs/maintenance_plan.md` §"Statement
//! emission (single owner)"): the SQL text a run actually executes must be
//! byte-identical to the single-owner emitters' output. This file proves
//! it by capturing the real statements sent to a live DuckDB connection
//! during a real `execute_project` run and diffing them against a direct
//! call of the emitter with the batch's own inputs.
//!
//! Phase 1 covers the region `DELETE`+`INSERT` family only (`IncrementalStrategy::
//! DeleteInsert`); the keyed-fold and column-scoped-MERGE families land in
//! later phases of `docs/plans/20260710-emit-unification.md`.

use std::path::Path;
use std::sync::{Arc, Mutex};

use arrow::array::RecordBatch;
use arrow::datatypes::SchemaRef;
use async_trait::async_trait;

use smelt_backend::{
    Backend, BackendCapabilities, BackendError, PartitionRange, SqlDialect, StatementGroup,
};
use smelt_backend_duckdb::DuckDbBackend;
use smelt_core::config::{Config, Target};
use smelt_core::graph::DependencyGraph;
use smelt_core::ModelDiscovery;
use smelt_logical::maintenance::emit::{emit_delete_insert, MaintenanceDialect, Region};
use smelt_runtime::execute::{execute_project, BackendFactory, BackendFuture};
use smelt_runtime::types::ExecuteRequest;
use tokio_util::sync::CancellationToken;

/// Wraps a real [`DuckDbBackend`], delegating every call, but recording the
/// [`StatementGroup`] passed to `execute_statement_group` — the single
/// point every emitted maintenance statement flows through on its way to
/// the connection (`docs/specs/maintenance_plan.md` §"Statement emission
/// (single owner)"). Recording here, rather than trusting the emitter was
/// called with the "right" inputs, is what proves *executed* SQL, not just
/// *constructed* SQL, matches the emitter's output.
struct RecordingBackend {
    inner: DuckDbBackend,
    groups: Mutex<Vec<StatementGroup>>,
}

impl RecordingBackend {
    fn new(inner: DuckDbBackend) -> Self {
        Self {
            inner,
            groups: Mutex::new(Vec::new()),
        }
    }

    fn recorded_groups(&self) -> Vec<StatementGroup> {
        self.groups.lock().unwrap().clone()
    }
}

#[async_trait]
impl Backend for RecordingBackend {
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

    async fn merge_into(
        &self,
        schema: &str,
        table: &str,
        source_sql: &str,
        unique_key: &[String],
    ) -> Result<(), BackendError> {
        self.inner
            .merge_into(schema, table, source_sql, unique_key)
            .await
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

    async fn execute_statement_group(&self, group: &StatementGroup) -> Result<(), BackendError> {
        self.groups.lock().unwrap().push(group.clone());
        self.inner.execute_statement_group(group).await
    }
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
            // `execute_project` owns the returned `Box<dyn Backend>`; we
            // keep a second handle via the `Arc` above purely to read back
            // the recorded groups after the run completes.
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
    async fn merge_into(
        &self,
        schema: &str,
        table: &str,
        source_sql: &str,
        unique_key: &[String],
    ) -> Result<(), BackendError> {
        self.0
            .merge_into(schema, table, source_sql, unique_key)
            .await
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
    async fn execute_statement_group(&self, group: &StatementGroup) -> Result<(), BackendError> {
        self.0.execute_statement_group(group).await
    }
}

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
    db.set_active_target(config.target.clone().map(|t| Arc::from(t.as_str())));

    let graph = DependencyGraph::build(sql_models, None).expect("build graph");

    (
        Arc::new(tokio::sync::Mutex::new(db)),
        Arc::new(tokio::sync::Mutex::new(graph)),
    )
}

fn make_request(target: &str, start: &str, end: &str) -> ExecuteRequest {
    ExecuteRequest {
        target: target.to_string(),
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
    }
}

/// The region DELETE+INSERT family (`IncrementalStrategy::DeleteInsert`):
/// every statement `execute_project` actually sends to the DuckDB
/// connection for a timeseries-partitioned batched model must be
/// byte-identical to `emit_delete_insert` called directly with that batch's
/// own inputs (table, partition column, region, compiled SQL).
#[tokio::test]
async fn region_recompute_statements_come_from_the_emitter() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project_dir = tmp.path();
    std::fs::create_dir_all(project_dir.join("models")).unwrap();

    // Self-contained: no upstream ref/source needed to exercise the region
    // DELETE+INSERT family — the output clamp wraps the model's own SELECT
    // regardless of where its data comes from.
    write_model(
        project_dir,
        "daily_events",
        "---\n\
         materialization: table\n\
         refresh: incremental\n\
         grain: partition\n\
         timeseries:\n\
         \x20\x20partition_column: event_date\n\
         \x20\x20event_time_column: event_date\n\
         \x20\x20granularity: day\n\
         ---\n\
         SELECT * FROM (VALUES (DATE '2024-01-01', 10), (DATE '2024-01-02', 20)) AS t(event_date, amount)",
    );

    let db_path = project_dir.join("run.duckdb");
    let smelt_yml = format!(
        "name: statement_parity_test\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: {db}\n    schema: main\ndefault_materialization: table\ntarget: dev\n",
        db = db_path.display()
    );
    std::fs::write(project_dir.join("smelt.yml"), &smelt_yml).unwrap();

    let config = Arc::new(Config::load(project_dir).expect("load config"));

    // Run 1: the table does not exist yet — this run always hits the
    // `create_table_as` first-run path, never `delete_and_insert_transactional`.
    // Statement parity for this family is a *second-run* concern.
    {
        let (db, graph) = build_db_and_graph(project_dir, &config);
        let backend_slot: Arc<Mutex<Option<Arc<RecordingBackend>>>> = Arc::new(Mutex::new(None));
        let factory = RecordingBackendFactory {
            db_path: db_path.clone(),
            backend: Arc::clone(&backend_slot),
        };
        execute_project(
            "statement-parity-run-1".to_string(),
            make_request("dev", "2024-01-01", "2024-01-02"),
            Arc::clone(&config),
            graph,
            db,
            project_dir,
            &factory,
            &smelt_runtime::NoOpReporter,
            CancellationToken::new(),
        )
        .await
        .expect("execute_project run 1 (first-run create)");
    }

    // Run 2: the table exists — this run must dispatch `IncrementalStrategy::
    // DeleteInsert`, and its statements are what this test asserts against.
    let (db, graph) = build_db_and_graph(project_dir, &config);
    let backend_slot: Arc<Mutex<Option<Arc<RecordingBackend>>>> = Arc::new(Mutex::new(None));
    let factory = RecordingBackendFactory {
        db_path: db_path.clone(),
        backend: Arc::clone(&backend_slot),
    };

    let request = make_request("dev", "2024-01-01", "2024-01-03");
    let cancel = CancellationToken::new();
    let outcome = execute_project(
        "statement-parity-run-2".to_string(),
        request,
        Arc::clone(&config),
        graph,
        db,
        project_dir,
        &factory,
        &smelt_runtime::NoOpReporter,
        cancel,
    )
    .await
    .expect("execute_project run 2 (incremental)");

    assert!(
        outcome.models.contains_key("daily_events"),
        "daily_events must have run: {:?}",
        outcome.models.keys().collect::<Vec<_>>()
    );

    let backend = backend_slot
        .lock()
        .unwrap()
        .clone()
        .expect("backend recorded");
    let groups = backend.recorded_groups();
    assert!(
        !groups.is_empty(),
        "at least one DELETE+INSERT group must have executed"
    );

    for group in &groups {
        assert!(
            group.transactional,
            "region DELETE+INSERT must be transactional"
        );
        assert_eq!(group.statements.len(), 2);
        assert!(group.statements[0]
            .sql
            .starts_with("DELETE FROM main.daily_events WHERE"));
        assert!(group.statements[1]
            .sql
            .starts_with("INSERT INTO main.daily_events "));

        // Re-derive the same group directly from the emitter, from the
        // executed statements' own region literals (parsed back out of the
        // DELETE text) plus the INSERT's own body — proving the executed
        // text is exactly what the emitter produces, not merely
        // emitter-shaped.
        let delete_sql = &group.statements[0].sql;
        let where_clause = delete_sql
            .strip_prefix("DELETE FROM main.daily_events WHERE ")
            .expect("delete shape");
        // where_clause: "event_date >= 'START' AND event_date < 'END'"
        let parts: Vec<&str> = where_clause.split(" AND ").collect();
        let start_lit = parts[0]
            .strip_prefix("event_date >= ")
            .expect("start literal");
        let end_lit = parts[1].strip_prefix("event_date < ").expect("end literal");
        let body = group.statements[1]
            .sql
            .strip_prefix("INSERT INTO main.daily_events ")
            .expect("insert shape");

        let region = Region {
            start: start_lit.to_string(),
            end: end_lit.to_string(),
        };
        let expected = emit_delete_insert(
            "main.daily_events",
            "event_date",
            &region,
            body,
            MaintenanceDialect::DuckDb,
        );
        assert_eq!(
            &expected, group,
            "executed group must be byte-identical to a direct emitter call over the same inputs"
        );
    }
}
