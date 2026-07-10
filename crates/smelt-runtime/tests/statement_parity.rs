//! Statement-parity CI gate (`docs/specs/architecture.md` §"Constraints &
//! Invariants" item 12; `docs/specs/maintenance_plan.md` §"Statement
//! emission (single owner)"): the SQL text a run actually executes must be
//! byte-identical to the single-owner emitters' output. This file proves
//! it by capturing the real statements sent to a live DuckDB connection
//! during a real `execute_project` run and diffing them against a direct
//! call of the emitter with the batch's own inputs.
//!
//! Covers the region `DELETE`+`INSERT` family (`IncrementalStrategy::
//! DeleteInsert`), the keyed-fold family (`refresh: keyed`), and the
//! column-scoped `MERGE` family (`Technique::ColumnScopedMerge`) —
//! `docs/plans/20260710-emit-unification.md` Phases 1–3.
//!
//! Each family's leg additionally proves **result**-equivalence to a full
//! refresh (`multiset_equal`, the Link-C oracle also used by
//! `crates/smelt-logical/tests/maintenance_plan_conformance.rs`), not just
//! byte-equal statement text — this is the production-execution half of the
//! "matches execution" proof `maintenance_plan_conformance.rs`'s own HOLDS
//! legs cannot make themselves, since `smelt-logical` cannot depend on
//! `smelt-runtime` to call `execute_project` (`docs/plans/
//! 20260710-emit-unification.md` Phase 4).
//!
//! This file also carries the structural no-authoring gate
//! (`no_maintenance_statement_authoring_outside_the_emitter`): a source
//! scan asserting the region `DELETE FROM`/keyed+column-scoped
//! `MERGE INTO`/keyed first-run `CREATE TABLE {}.{} AS`-shaped statement
//! text is not constructed anywhere in `smelt-backend*/src` or
//! `smelt-runtime/src` production code outside the single-owner emitters.

use std::path::Path;
use std::sync::{Arc, Mutex};

use arrow::array::{Int64Array, RecordBatch};
use arrow::datatypes::SchemaRef;
use async_trait::async_trait;

use smelt_backend::{
    Backend, BackendCapabilities, BackendError, PartitionRange, SqlDialect, StatementGroup,
};
use smelt_backend_duckdb::DuckDbBackend;
use smelt_core::config::{Config, Target};
use smelt_core::graph::DependencyGraph;
use smelt_core::ModelDiscovery;
use smelt_logical::maintenance::emit::{
    emit_column_scoped_merge, emit_create_table_as, emit_delete_insert, emit_keyed_fold,
    MaintenanceDialect, Region,
};
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

    fn supports_column_scoped_merge(&self) -> bool {
        self.inner.supports_column_scoped_merge()
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

    // `merge_into` is deliberately not overridden here: the `Backend`
    // trait's default implementation builds the `StatementGroup` via
    // `emit_column_scoped_merge` and calls `self.execute_statement_group`,
    // which routes through the override below — overriding `merge_into`
    // itself (forwarding straight to `self.inner.merge_into`) would bypass
    // this struct's own `execute_statement_group` override and record
    // nothing.

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
    fn supports_column_scoped_merge(&self) -> bool {
        self.0.supports_column_scoped_merge()
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
    // `merge_into` is deliberately not overridden here — see the identical
    // note on `RecordingBackend`'s impl above; the trait default's
    // `execute_statement_group` call routes through this struct's own
    // override below.
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

/// The Link-C oracle (`crates/smelt-runtime/tests/oracle/mod.rs`,
/// `crates/smelt-logical/tests/maintenance_plan_conformance.rs`'s own copy):
/// two relations are equal multisets iff `EXCEPT ALL` is empty in both
/// directions. Runs through the real recording backend's `execute_sql` —
/// the same connection the run itself executed against — so this is a
/// same-connection, post-run check of the *result*, not a re-derivation.
async fn except_all_count(backend: &dyn Backend, left_sql: &str, right_sql: &str) -> i64 {
    let batches = backend
        .execute_sql(&format!(
            "SELECT count(*) FROM (({left_sql}) EXCEPT ALL ({right_sql})) AS d"
        ))
        .await
        .expect("except all count query");
    let batch = batches.first().expect("count query returns one batch");
    let counts = batch
        .column(0)
        .as_any()
        .downcast_ref::<Int64Array>()
        .expect("count(*) column is Int64");
    counts.value(0)
}

async fn multiset_equal(backend: &dyn Backend, left_sql: &str, right_sql: &str) -> bool {
    except_all_count(backend, left_sql, right_sql).await == 0
        && except_all_count(backend, right_sql, left_sql).await == 0
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

    // Result-equivalence: the region DELETE+INSERT statements the run
    // actually executed must leave `daily_events` multiset-equal to a full
    // refresh of the model's own SQL — the technique the plan describes
    // (`docs/specs/maintenance_plan.md` §"Statement emission (single
    // owner)") reproduces a full refresh, not merely emitter-shaped text.
    let full_refresh_sql = "SELECT * FROM (VALUES (DATE '2024-01-01', 10), \
                             (DATE '2024-01-02', 20)) AS t(event_date, amount)";
    assert!(
        multiset_equal(
            backend.as_ref(),
            "SELECT * FROM main.daily_events",
            full_refresh_sql
        )
        .await,
        "the DELETE+INSERT statements execute_project actually ran must reproduce a full refresh"
    );
}

/// The keyed fold family (`refresh: keyed`, `grain: key`): every statement
/// `execute_project` sends for the windowed-keyed-maintenance driver's
/// steps — the first-run `CREATE TABLE … AS` and each following step's
/// `MERGE` — must be byte-identical to `emit_create_table_as`/
/// `emit_keyed_fold` called directly with that step's own inputs.
///
/// The fixture uses only `MIN`/`MAX` aggregator columns (no `SUM`), so the
/// cell grades `Grade::Idempotent`
/// (`WindowedKeyedRule::ledger_grade` — "additive iff any combiner is
/// `Sum`") and every step's create-or-merge action routes through
/// `Backend::execute_statement_group`, the same funnel the region family
/// uses — the `Grade::Additive` ledger-interleaved path
/// (`Backend::fold_ledger_delta`) is untouched by this phase
/// (`docs/plans/20260710-emit-unification.md` Phase 2 implementation
/// shape).
#[tokio::test]
async fn keyed_fold_statements_come_from_the_emitter() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project_dir = tmp.path();
    std::fs::create_dir_all(project_dir.join("models")).unwrap();

    write_model(
        project_dir,
        "events",
        "---\n\
         materialization: table\n\
         timeseries:\n\
         \x20\x20partition_column: event_date\n\
         \x20\x20event_time_column: event_date\n\
         \x20\x20granularity: day\n\
         ---\n\
         SELECT * FROM (VALUES \
         (DATE '2024-01-01', 1, TIMESTAMP '2024-01-01 01:00:00'), \
         (DATE '2024-01-02', 1, TIMESTAMP '2024-01-02 02:00:00'), \
         (DATE '2024-01-02', 2, TIMESTAMP '2024-01-02 03:00:00')) \
         AS t(event_date, device_id, event_ts)",
    );
    write_model(
        project_dir,
        "device_user_edges",
        "---\n\
         materialization: table\n\
         refresh: incremental\n\
         grain: key\n\
         ---\n\
         SELECT device_id, MIN(event_ts) AS first_seen, MAX(event_ts) AS last_seen \
         FROM smelt.events GROUP BY device_id",
    );

    let db_path = project_dir.join("run.duckdb");
    let smelt_yml = format!(
        "name: keyed_statement_parity_test\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: {db}\n    schema: main\ndefault_materialization: table\ntarget: dev\n",
        db = db_path.display()
    );
    std::fs::write(project_dir.join("smelt.yml"), &smelt_yml).unwrap();

    let config = Arc::new(Config::load(project_dir).expect("load config"));

    let (db, graph) = build_db_and_graph(project_dir, &config);
    let backend_slot: Arc<Mutex<Option<Arc<RecordingBackend>>>> = Arc::new(Mutex::new(None));
    let factory = RecordingBackendFactory {
        db_path: db_path.clone(),
        backend: Arc::clone(&backend_slot),
    };

    // One window covering both driving-source partitions: step 1
    // (2024-01-01) hits the first-run CREATE arm; step 2 (2024-01-02) hits
    // the MERGE arm.
    let request = make_request("dev", "2024-01-01", "2024-01-03");
    let outcome = execute_project(
        "keyed-statement-parity-run".to_string(),
        request,
        Arc::clone(&config),
        graph,
        db,
        project_dir,
        &factory,
        &smelt_runtime::NoOpReporter,
        CancellationToken::new(),
    )
    .await
    .expect("execute_project (keyed)");

    assert!(
        outcome.models.contains_key("device_user_edges"),
        "device_user_edges must have run: {:?}",
        outcome.models.keys().collect::<Vec<_>>()
    );

    let backend = backend_slot
        .lock()
        .unwrap()
        .clone()
        .expect("backend recorded");
    let groups = backend.recorded_groups();
    assert_eq!(
        groups.len(),
        2,
        "two steps must each execute exactly one statement group: {:?}",
        groups
    );

    // Step 1: first-run CREATE TABLE ... AS.
    let create_sql = &groups[0].statements[0].sql;
    assert_eq!(groups[0].statements.len(), 1);
    assert!(
        create_sql.starts_with("CREATE TABLE main.device_user_edges AS "),
        "unexpected create statement: {create_sql}"
    );
    let create_select = create_sql
        .strip_prefix("CREATE TABLE main.device_user_edges AS ")
        .expect("create shape");
    let expected_create = emit_create_table_as(
        "main.device_user_edges",
        create_select,
        MaintenanceDialect::DuckDb,
    );
    assert_eq!(
        &expected_create, &groups[0],
        "executed CREATE group must be byte-identical to a direct emitter call"
    );

    // Step 2: combiner-aware MERGE.
    let merge_sql = &groups[1].statements[0].sql;
    assert_eq!(groups[1].statements.len(), 1);
    let prefix = "MERGE INTO main.device_user_edges AS target USING (";
    let suffix = ") AS delta ON target.device_id = delta.device_id \
                  WHEN MATCHED THEN UPDATE SET first_seen = LEAST(target.first_seen, delta.first_seen), \
                  last_seen = GREATEST(target.last_seen, delta.last_seen) \
                  WHEN NOT MATCHED THEN INSERT *";
    assert!(
        merge_sql.starts_with(prefix) && merge_sql.ends_with(suffix),
        "unexpected merge statement: {merge_sql}"
    );
    let delta_select = &merge_sql[prefix.len()..merge_sql.len() - suffix.len()];
    let expected_merge = emit_keyed_fold(
        "main.device_user_edges",
        &["device_id".to_string()],
        &[
            (
                "first_seen".to_string(),
                "LEAST(target.first_seen, delta.first_seen)".to_string(),
            ),
            (
                "last_seen".to_string(),
                "GREATEST(target.last_seen, delta.last_seen)".to_string(),
            ),
        ],
        delta_select,
        MaintenanceDialect::DuckDb,
    );
    assert_eq!(
        &expected_merge, &groups[1],
        "executed MERGE group must be byte-identical to a direct emitter call"
    );

    // Result-equivalence: the CREATE + MERGE statements the run actually
    // executed must leave `device_user_edges` multiset-equal to a full
    // refresh of the model's own aggregation over the driving source's
    // materialized output.
    assert!(
        multiset_equal(
            backend.as_ref(),
            "SELECT * FROM main.device_user_edges",
            "SELECT device_id, MIN(event_ts) AS first_seen, MAX(event_ts) AS last_seen \
             FROM main.events GROUP BY device_id"
        )
        .await,
        "the CREATE+MERGE statements execute_project actually ran must reproduce a full refresh"
    );
}

/// Copy `examples/timeseries` into a scratch directory so the run's
/// `.smelt/` state never lands inside the checked-in example (mirrors
/// `crates/smelt-runtime/tests/technique_lowering.rs`'s
/// `column_scoped_merge_e2e::copy_dir_recursive`).
fn copy_dir_recursive(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).expect("create dst dir");
    for entry in std::fs::read_dir(src).expect("read src dir") {
        let entry = entry.expect("dir entry");
        let file_type = entry.file_type().expect("file type");
        let dst_path = dst.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_recursive(&entry.path(), &dst_path);
        } else {
            std::fs::copy(entry.path(), &dst_path).expect("copy file");
        }
    }
}

fn select_request(target: &str, model: &str, start: &str, end: &str) -> ExecuteRequest {
    ExecuteRequest {
        target: target.to_string(),
        select: vec![model.to_string()],
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

/// The column-scoped `MERGE` family (`Technique::ColumnScopedMerge`, MP11):
/// re-runs `technique_lowering.rs::column_scoped_merge_e2e`'s
/// `examples/timeseries/daily_events_enriched` fixture — a fact+dimension
/// enrichment whose `raw.users` mutation drives the `{user_name}` cell's
/// live column-scoped MERGE — through the recording reporter/backend, and
/// asserts the executed `MERGE` is byte-identical to a direct call of
/// `emit_column_scoped_merge` over the same table/unique_key/source_select.
#[tokio::test]
async fn column_scoped_merge_statements_come_from_the_emitter() {
    let source_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/timeseries")
        .canonicalize()
        .expect("examples/timeseries exists");

    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().join("project");
    copy_dir_recursive(&source_dir, &project_dir);

    let db_path = tmp.path().join("run.duckdb");
    let config = Arc::new(Config::load(&project_dir).expect("load smelt.yml"));

    // Stage the two source tables `execute_project` reads (same fixture
    // data as `technique_lowering.rs::column_scoped_merge_e2e`).
    {
        let backend = DuckDbBackend::new(&db_path, "main")
            .await
            .expect("open duckdb");
        backend
            .execute_sql(
                "CREATE TABLE main.sources_raw_events (event_id INTEGER, user_id INTEGER, \
                 event_type VARCHAR, event_timestamp TIMESTAMP)",
            )
            .await
            .expect("create events source table");
        backend
            .execute_sql(
                "INSERT INTO main.sources_raw_events VALUES \
                 (1, 1, 'login', TIMESTAMP '2025-01-10 08:00:00'), \
                 (2, 2, 'login', TIMESTAMP '2025-01-10 09:00:00')",
            )
            .await
            .expect("seed events");
        backend
            .execute_sql(
                "CREATE TABLE main.sources_raw_users (user_id INTEGER, user_name VARCHAR, \
                 signup_date DATE)",
            )
            .await
            .expect("create users source table");
        backend
            .execute_sql(
                "INSERT INTO main.sources_raw_users VALUES \
                 (1, 'Alice', DATE '2025-01-01'), (2, 'Bob', DATE '2025-01-02')",
            )
            .await
            .expect("seed users");
    }

    let request = select_request("dev", "daily_events_enriched", "2025-01-10", "2025-01-11");

    // Run 1: creates the target (table doesn't exist yet) — never the
    // column-scoped MERGE path.
    {
        let (db, graph) = build_db_and_graph(&project_dir, &config);
        let backend_slot: Arc<Mutex<Option<Arc<RecordingBackend>>>> = Arc::new(Mutex::new(None));
        let factory = RecordingBackendFactory {
            db_path: db_path.clone(),
            backend: Arc::clone(&backend_slot),
        };
        execute_project(
            "column-scoped-merge-parity-run-1".to_string(),
            request.clone(),
            Arc::clone(&config),
            graph,
            db,
            &project_dir,
            &factory,
            &smelt_runtime::NoOpReporter,
            CancellationToken::new(),
        )
        .await
        .expect("first run (create) must succeed");
    }

    // Mutate the dimension in place, making the `{user_name}` cell live.
    {
        let backend = DuckDbBackend::new(&db_path, "main")
            .await
            .expect("reopen duckdb");
        backend
            .execute_sql("UPDATE main.sources_raw_users SET user_name = 'Alicia' WHERE user_id = 1")
            .await
            .expect("mutate dimension");
    }

    // Run 2: the dimension mutation dispatches the column-scoped MERGE.
    let (db, graph) = build_db_and_graph(&project_dir, &config);
    let backend_slot: Arc<Mutex<Option<Arc<RecordingBackend>>>> = Arc::new(Mutex::new(None));
    let factory = RecordingBackendFactory {
        db_path: db_path.clone(),
        backend: Arc::clone(&backend_slot),
    };
    let outcome = execute_project(
        "column-scoped-merge-parity-run-2".to_string(),
        request,
        Arc::clone(&config),
        graph,
        db,
        &project_dir,
        &factory,
        &smelt_runtime::NoOpReporter,
        CancellationToken::new(),
    )
    .await
    .expect("second run (column-scoped merge) must succeed");

    let record = outcome
        .models
        .get("daily_events_enriched")
        .expect("daily_events_enriched ran");
    assert_eq!(
        record.strategy, "column_scoped_merge",
        "the dimension mutation must dispatch the column-scoped MERGE technique"
    );

    let backend = backend_slot
        .lock()
        .unwrap()
        .clone()
        .expect("backend recorded");
    let groups = backend.recorded_groups();
    let merge_groups: Vec<_> = groups
        .iter()
        .filter(|g| g.statements[0].sql.starts_with("MERGE INTO"))
        .collect();
    assert_eq!(
        merge_groups.len(),
        1,
        "exactly one column-scoped MERGE group must have executed: {:?}",
        groups
    );

    let group = merge_groups[0];
    assert!(
        !group.transactional,
        "a single-statement group needs no transaction wrapper"
    );
    assert_eq!(group.statements.len(), 1);

    let sql = &group.statements[0].sql;
    let prefix = "MERGE INTO main.daily_events_enriched AS target USING (";
    let suffix = ") AS source ON target.event_id = source.event_id \
                  WHEN MATCHED THEN UPDATE SET * \
                  WHEN NOT MATCHED THEN INSERT *";
    assert!(
        sql.starts_with(prefix) && sql.ends_with(suffix),
        "unexpected merge statement: {sql}"
    );
    let source_select = &sql[prefix.len()..sql.len() - suffix.len()];

    let expected = emit_column_scoped_merge(
        "main.daily_events_enriched",
        &["event_id".to_string()],
        source_select,
        MaintenanceDialect::DuckDb,
    );
    assert_eq!(
        &expected, group,
        "executed MERGE group must be byte-identical to a direct emitter call over the same inputs"
    );

    // Result-equivalence: the column-scoped MERGE the run actually executed
    // must leave `daily_events_enriched` multiset-equal to a full refresh of
    // the model's own fact+dimension join, post-mutation.
    assert!(
        multiset_equal(
            backend.as_ref(),
            "SELECT * FROM main.daily_events_enriched",
            "SELECT e.event_id, date_trunc('day', e.event_timestamp) AS event_date, \
             e.user_id, e.event_type, u.user_name \
             FROM main.sources_raw_events e \
             JOIN main.sources_raw_users u ON e.user_id = u.user_id"
        )
        .await,
        "the column-scoped MERGE execute_project actually ran must reproduce a full refresh"
    );
}

// =============================================================================
// Structural gate: no maintenance-statement authoring outside the emitter
// (`docs/specs/maintenance_plan.md` §"Statement emission (single owner)";
// `docs/plans/20260710-emit-unification.md` Phase 4). Same `rg`-over-sources
// style as `crates/smelt-core/tests/hardening_budget.rs`: a source scan, not
// a runtime assertion, so it catches a regression at review time rather than
// only when a fixture happens to exercise the reintroduced text.
// =============================================================================

/// Repo root, two levels up from this crate's manifest dir
/// (`crates/smelt-runtime` → `crates` → repo root) — the same derivation
/// `crates/smelt-core/tests/hardening_budget.rs::repo_root` uses.
fn repo_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates dir")
        .parent()
        .expect("repo root")
        .to_path_buf()
}

/// One forbidden-shape hit: `(file, 1-based line number, line text)`.
struct StatementAuthoringHit {
    file: std::path::PathBuf,
    line_no: usize,
    text: String,
}

/// Known, pre-existing, out-of-scope matches this gate does not fail on —
/// `(file path suffix, distinguishing substring of the offending line)`.
/// Removing an entry without fixing (or re-justifying) the underlying
/// authoring is itself the review signal this gate exists to raise.
///
/// Every entry here belongs to `Backend::delete_partitions`/
/// `Backend::insert_overwrite`, serving `IncrementalStrategy::
/// InsertOverwrite` — a per-partition materialization strategy that
/// predates `maintenance_plan.md`'s single-owner emitters entirely and
/// that no live derivation selects today: `smelt_runtime::
/// maintenance_driver::resolve_incremental_strategy` and the batch loop's
/// own dispatch (`crates/smelt-runtime/src/execute.rs`) only ever resolve
/// `IncrementalStrategy::DeleteInsert`; `Append`/`InsertOverwrite` have no
/// construction site outside their own enum definition, a CLI display-name
/// mapping (`smelt-cli/src/helpers.rs`), and unit tests. Retiring this dead
/// code (or routing it through `emit_delete_insert` too, closing the
/// remaining gap) is out of Phase 4's file scope (`docs/plans/
/// 20260710-emit-unification.md` Phase 4 "Critical files" — the backend
/// crates are not listed); tracked as follow-up, not fixed here.
const STATEMENT_AUTHORING_ALLOWLIST: &[(&str, &str)] = &[
    (
        "smelt-backend-duckdb/src/lib.rs",
        "DELETE FROM {} WHERE {} >= '{}' AND {} < '{}'",
    ),
    (
        "smelt-backend-duckdb/src/lib.rs",
        "DELETE FROM {} WHERE {} IN (SELECT DISTINCT {} FROM ({}))",
    ),
    (
        "smelt-backend-spark/src/sql.rs",
        "DELETE FROM {} WHERE {} IN ({})",
    ),
    (
        "smelt-backend-spark/src/sql.rs",
        "DELETE FROM {} WHERE {} >= '{}' AND {} < '{}'",
    ),
];

fn statement_authoring_is_allowlisted(path: &Path, line: &str) -> bool {
    let normalized = path.to_string_lossy().replace('\\', "/");
    STATEMENT_AUTHORING_ALLOWLIST
        .iter()
        .any(|(file_suffix, substr)| normalized.ends_with(file_suffix) && line.contains(substr))
}

/// Scan one production `.rs` file for forbidden maintenance-statement
/// shapes. Stops at the first `#[cfg(test)]` line (test fixtures — e.g.
/// `maintenance_driver.rs`'s in-memory `SumRule`/`RecordingBackend` — build
/// deliberately statement-shaped strings to exercise dispatch without a
/// real backend; that is not production authoring) — the same truncation
/// `hardening_budget.rs::count_println_in_file` uses. Skips comment lines
/// (`//`, `///`, `//!`) since the forbidden shapes appear in doc comments
/// describing the emitter's own output.
fn scan_statement_authoring_file(path: &Path, hits: &mut Vec<StatementAuthoringHit>) {
    let content = std::fs::read_to_string(path).unwrap_or_default();
    for (idx, line) in content.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("#[cfg(test)]") {
            break;
        }
        if trimmed.starts_with("//") {
            continue;
        }
        let forbidden = line.contains("DELETE FROM ")
            || line.contains("MERGE INTO ")
            || line.contains("CREATE TABLE {}.{} AS");
        if !forbidden {
            continue;
        }
        if statement_authoring_is_allowlisted(path, line) {
            continue;
        }
        hits.push(StatementAuthoringHit {
            file: path.to_path_buf(),
            line_no: idx + 1,
            text: line.trim().to_string(),
        });
    }
}

fn scan_statement_authoring_dir(dir: &Path, hits: &mut Vec<StatementAuthoringHit>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            // `tests/` subdirectories hold integration tests, not
            // production code — this file (`crates/smelt-runtime/tests/`)
            // is itself outside every scanned `src/` tree.
            if path.file_name().map(|n| n == "tests").unwrap_or(false) {
                continue;
            }
            scan_statement_authoring_dir(&path, hits);
        } else if path.extension().map(|e| e == "rs").unwrap_or(false) {
            // `tests.rs` (e.g. `smelt-backend-spark/src/tests.rs`) is a
            // unit-test module file, not production code.
            if path.file_name().map(|n| n == "tests.rs").unwrap_or(false) {
                continue;
            }
            scan_statement_authoring_file(&path, hits);
        }
    }
}

/// Structural gate: `DELETE FROM`/`MERGE INTO`/`CREATE TABLE {}.{} AS`-shaped
/// statement text must not be constructed anywhere in `smelt-backend*/src`
/// or `smelt-runtime/src` production code outside the single-owner emitters
/// in `crates/smelt-logical/src/maintenance/emit.rs` (which is not scanned
/// — it is not a `smelt-backend*` or `smelt-runtime` crate). Backends
/// execute emitted `StatementGroup`s (`Backend::execute_statement_group`);
/// they never author maintenance-statement text of their own
/// (`docs/specs/maintenance_plan.md` §"Statement emission (single owner)").
#[test]
fn no_maintenance_statement_authoring_outside_the_emitter() {
    let crates_dir = repo_root().join("crates");
    let mut hits = Vec::new();
    for crate_name in [
        "smelt-backend",
        "smelt-backend-duckdb",
        "smelt-backend-spark",
        "smelt-backends",
        "smelt-runtime",
    ] {
        scan_statement_authoring_dir(&crates_dir.join(crate_name).join("src"), &mut hits);
    }
    assert!(
        hits.is_empty(),
        "maintenance-statement text constructed outside smelt-logical's single-owner emitters \
         (docs/specs/maintenance_plan.md §\"Statement emission (single owner)\") — backends must \
         execute an emitted StatementGroup, never author their own SQL text:\n{}",
        hits.iter()
            .map(|h| format!("  {}:{}: {}", h.file.display(), h.line_no, h.text))
            .collect::<Vec<_>>()
            .join("\n")
    );
}
