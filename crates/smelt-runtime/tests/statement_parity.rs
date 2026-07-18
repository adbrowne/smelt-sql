//! Statement-parity CI gate (`docs/specs/architecture.md` §"Constraints &
//! Invariants" item 12; `docs/specs/incremental_models.md` §"Statement
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

use arrow::array::{Array, Int64Array, RecordBatch};
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
    emit_keyed_fold_suppressed, emit_recurrence_bound_probe, MaintenanceDialect, Region,
    TargetSlicePredicate,
};
use smelt_logical::maintenance::locality::LocalitySlice;
use smelt_planner::{
    AggregatorColumn, CrossPartitionCombiner, CumulativeClassification, DrivingSource,
};
use smelt_runtime::execute::{execute_project, BackendFactory, BackendFuture};
use smelt_runtime::maintenance_driver::{driving_steps, run_windowed_keyed_maintenance};
use smelt_runtime::types::ExecuteRequest;
use tokio_util::sync::CancellationToken;

/// Wraps a real [`DuckDbBackend`], delegating every call, but recording the
/// [`StatementGroup`] passed to `execute_statement_group` — the single
/// point every emitted maintenance statement flows through on its way to
/// the connection (`docs/specs/incremental_models.md` §"Statement emission
/// (single owner)"). Recording here, rather than trusting the emitter was
/// called with the "right" inputs, is what proves *executed* SQL, not just
/// *constructed* SQL, matches the emitter's output.
struct RecordingBackend {
    inner: DuckDbBackend,
    groups: Mutex<Vec<StatementGroup>>,
    /// Every raw SQL string handed to `execute_sql` directly (not via
    /// `execute_statement_group`) — the checked route-3 out-of-slice match
    /// probe runs this way (`maintenance_driver::run_windowed_keyed_
    /// maintenance`'s probe-gate, not a `StatementGroup`), so `groups`
    /// alone would miss it.
    sql_log: Mutex<Vec<String>>,
}

impl RecordingBackend {
    fn new(inner: DuckDbBackend) -> Self {
        Self {
            inner,
            groups: Mutex::new(Vec::new()),
            sql_log: Mutex::new(Vec::new()),
        }
    }

    fn recorded_groups(&self) -> Vec<StatementGroup> {
        self.groups.lock().unwrap().clone()
    }

    fn recorded_sql(&self) -> Vec<String> {
        self.sql_log.lock().unwrap().clone()
    }
}

#[async_trait]
impl Backend for RecordingBackend {
    async fn execute_sql(&self, sql: &str) -> Result<Vec<RecordBatch>, BackendError> {
        self.sql_log.lock().unwrap().push(sql.to_string());
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
        run_checks: false,
        checks: vec![],
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
    // (`docs/specs/incremental_models.md` §"Statement emission (single
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

/// First-run bootstrap for a **self-referential** partition-grain model
/// (`docs/specs/incremental_models.md` §"First-run and backfill" — "First-run
/// bootstrap for a self-referential model"): building from scratch (no
/// pre-seeded target table) must emit exactly ONE statement group before
/// any region `DELETE`+`INSERT` — a plain `CREATE TABLE main.running_balance
/// (…)` with no `SELECT` — byte-identical to a direct call of
/// `emit_create_empty_table` with the same table name/columns/dialect.
/// Every batch's own region `DELETE`+`INSERT` group after it must still
/// match `emit_delete_insert`, exactly like the non-self-referential family
/// above — the bootstrap only replaces the otherwise-impossible first-run
/// `CREATE TABLE … AS SELECT …`, it does not change any later batch's
/// technique.
#[tokio::test]
async fn self_referential_bootstrap_statements_come_from_the_emitter() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project_dir = tmp.path();
    std::fs::create_dir_all(project_dir.join("models/sources")).unwrap();

    write_model(
        project_dir,
        "running_balance",
        "---\n\
         materialization: table\n\
         refresh: incremental\n\
         grain: partition\n\
         timeseries:\n\
         \x20\x20partition_column: d\n\
         \x20\x20event_time_column: d\n\
         \x20\x20granularity: day\n\
         batched:\n\
         \x20\x20unique_key: [d]\n\
         ---\n\
         SELECT d, balance FROM (\n\
         \x20\x20SELECT\n\
         \x20\x20\x20\x20t.d AS d,\n\
         \x20\x20\x20\x20COALESCE(bal.balance, 0) + SUM(t.amt) AS balance\n\
         \x20\x20FROM smelt.sources.transactions t\n\
         \x20\x20LEFT JOIN smelt.running_balance bal\n\
         \x20\x20\x20\x20ON bal.d >= t.d - INTERVAL '1 day' AND bal.d < t.d\n\
         \x20\x20GROUP BY t.d, bal.balance\n\
         ) inner_balance",
    );
    std::fs::write(
        project_dir.join("models/sources/transactions.yml"),
        "description: statement-parity self-ref source.\n\
         mutation_profile: append_only\n\
         columns:\n\
         \x20\x20- name: d\n\
         \x20\x20\x20\x20type: DATE\n\
         \x20\x20- name: amt\n\
         \x20\x20\x20\x20type: DOUBLE\n",
    )
    .unwrap();

    let db_path = project_dir.join("run.duckdb");
    let smelt_yml = format!(
        "name: statement_parity_self_ref_test\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: {db}\n    schema: main\ndefault_materialization: table\ntarget: dev\n",
        db = db_path.display()
    );
    std::fs::write(project_dir.join("smelt.yml"), &smelt_yml).unwrap();

    // Seed the source table only — deliberately NO pre-created
    // `main.running_balance` target, proving the bootstrap builds it.
    {
        let conn = duckdb::Connection::open(&db_path).expect("open duckdb");
        conn.execute_batch(
            "CREATE SCHEMA IF NOT EXISTS main;\n\
             CREATE TABLE main.sources_transactions (d DATE, amt DOUBLE);\n\
             INSERT INTO main.sources_transactions VALUES \
             (DATE '2024-01-01', 10.0), (DATE '2024-01-02', 5.0);",
        )
        .expect("seed source table");
    }

    let config = Arc::new(Config::load(project_dir).expect("load config"));
    let (db, graph) = build_db_and_graph(project_dir, &config);
    let backend_slot: Arc<Mutex<Option<Arc<RecordingBackend>>>> = Arc::new(Mutex::new(None));
    let factory = RecordingBackendFactory {
        db_path: db_path.clone(),
        backend: Arc::clone(&backend_slot),
    };

    execute_project(
        "statement-parity-self-ref-run".to_string(),
        make_request("dev", "2024-01-01", "2024-01-03"),
        Arc::clone(&config),
        graph,
        db,
        project_dir,
        &factory,
        &smelt_runtime::NoOpReporter,
        CancellationToken::new(),
    )
    .await
    .expect("execute_project self-referential from-scratch run");

    let backend = backend_slot
        .lock()
        .unwrap()
        .clone()
        .expect("backend recorded");
    let groups = backend.recorded_groups();
    assert!(
        !groups.is_empty(),
        "at least the bootstrap CREATE TABLE group must have executed"
    );

    // First group: the bootstrap, non-transactional, exactly one
    // `CREATE TABLE main.running_balance (…)` statement with no `SELECT`.
    let bootstrap = &groups[0];
    assert!(
        !bootstrap.transactional,
        "the bootstrap CREATE TABLE is not a DELETE+INSERT pair"
    );
    assert_eq!(bootstrap.statements.len(), 1);
    let bootstrap_sql = &bootstrap.statements[0].sql;
    assert!(
        bootstrap_sql.starts_with("CREATE TABLE main.running_balance ("),
        "bootstrap statement: {bootstrap_sql}"
    );
    assert!(
        !bootstrap_sql.contains("SELECT"),
        "the bootstrap must be a plain empty CREATE TABLE, not a CREATE TABLE … AS SELECT: \
         {bootstrap_sql}"
    );

    // Re-derive the same statement directly from the emitter over the
    // columns parsed back out of the executed DDL text, proving byte
    // parity rather than merely emitter-shaped text.
    let col_defs = bootstrap_sql
        .strip_prefix("CREATE TABLE main.running_balance (")
        .and_then(|s| s.strip_suffix(')'))
        .expect("bootstrap DDL shape");
    let columns: Vec<(String, smelt_types::DataType)> = col_defs
        .split(", ")
        .map(|col| {
            let (name, ty) = col.split_once(' ').expect("column definition shape");
            (
                name.to_string(),
                smelt_types::parse_type(ty).expect("column type text"),
            )
        })
        .collect();
    let expected = smelt_logical::maintenance::emit::emit_create_empty_table(
        "main.running_balance",
        &columns,
        MaintenanceDialect::DuckDb,
    );
    assert_eq!(
        &expected, bootstrap,
        "executed bootstrap group must be byte-identical to a direct emitter call over the \
         same table/columns"
    );

    // Every subsequent group is the ordinary region DELETE+INSERT family —
    // the bootstrap only replaces the impossible first-run CTAS, it does
    // not change any later batch's technique.
    for group in &groups[1..] {
        assert!(
            group.transactional,
            "region DELETE+INSERT must be transactional"
        );
        assert_eq!(group.statements.len(), 2);
        assert!(group.statements[0]
            .sql
            .starts_with("DELETE FROM main.running_balance WHERE"));
        assert!(group.statements[1]
            .sql
            .starts_with("INSERT INTO main.running_balance "));
    }

    // Result-equivalence: the maintained trajectory must equal a full
    // sequential re-derivation from the source's current contents.
    assert!(
        multiset_equal(
            backend.as_ref(),
            "SELECT balance FROM main.running_balance WHERE d = DATE '2024-01-01'",
            "SELECT 10.0 AS balance",
        )
        .await,
        "day 1 balance must equal the sequential expectation"
    );
    assert!(
        multiset_equal(
            backend.as_ref(),
            "SELECT balance FROM main.running_balance WHERE d = DATE '2024-01-02'",
            "SELECT 15.0 AS balance",
        )
        .await,
        "day 2 balance must equal the sequential expectation"
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

    // Step 2: combiner-aware MERGE. `first_seen`/`last_seen` are both
    // Comparable (MIN/MAX are registry-backed deterministic functions) over
    // a proven `device_id` key, so a real run now resolves `Suppressed`
    // (`docs/plans/20260715-composed-axes-conditional-maintenance.md`
    // Phase C6 — `resolve_cumulative_write_suppression`, wired into
    // `execute_cumulative_aggregate`) — the matched arm carries an `IS
    // DISTINCT FROM` guard over both fold columns.
    let merge_sql = &groups[1].statements[0].sql;
    assert_eq!(groups[1].statements.len(), 1);
    let prefix = "MERGE INTO main.device_user_edges AS target USING (";
    let suffix = ") AS delta ON target.device_id = delta.device_id \
                  WHEN MATCHED AND (target.first_seen IS DISTINCT FROM (LEAST(target.first_seen, \
                  delta.first_seen)) OR target.last_seen IS DISTINCT FROM (GREATEST(target.\
                  last_seen, delta.last_seen))) THEN UPDATE SET first_seen = LEAST(target.\
                  first_seen, delta.first_seen), last_seen = GREATEST(target.last_seen, \
                  delta.last_seen) WHEN NOT MATCHED THEN INSERT *";
    assert!(
        merge_sql.starts_with(prefix) && merge_sql.ends_with(suffix),
        "unexpected merge statement: {merge_sql}"
    );
    let delta_select = &merge_sql[prefix.len()..merge_sql.len() - suffix.len()];
    let expected_merge = emit_keyed_fold_suppressed(
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
        None,
        &["first_seen".to_string(), "last_seen".to_string()],
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

/// The slice-predicated keyed-fold family: a `refresh: keyed` model that
/// also declares its own `timeseries:` block, admitted through key temporal
/// locality's route 1 (key-embedded — `partition_column` is itself a
/// `unique_key` column, `docs/specs/incremental_models.md` §"Key temporal
/// locality (the time-partitioned output)"; `docs/plans/20260715-composed-
/// axes-conditional-maintenance.md` Phase A2). The established
/// [`smelt_logical::maintenance::locality::LocalitySlice`] licenses a
/// `target.<partition_column> BETWEEN ...` predicate on the `MERGE`'s `ON`
/// clause (`emit_keyed_fold`'s `slice` parameter) — this is
/// `keyed_fold_statements_come_from_the_emitter` above with one addition (the
/// keyed model's own `timeseries:` block), proving the *slice-carrying*
/// MERGE `execute_project` actually runs is still byte-identical to a direct
/// `emit_keyed_fold` call with that same slice, not merely slice-shaped
/// text.
///
/// `MAX` (not `SUM`) keeps the cell `Grade::Idempotent`
/// (`WindowedKeyedRule::ledger_grade`), so the step routes through
/// `Backend::execute_statement_group` — the funnel this test's
/// `RecordingBackend` records — rather than the ledger-interleaved additive
/// path, matching `keyed_fold_statements_come_from_the_emitter`'s own choice
/// of combiner.
///
/// This is also the doubly-predicated statement-parity leg
/// (`docs/plans/20260715-composed-axes-conditional-maintenance.md` Phase
/// C6): `max_amount` is Comparable (a registry-backed deterministic
/// aggregate) over the proven `{device_id, event_date}` key, so a real run
/// now resolves `WriteSuppression::Suppressed`
/// (`resolve_cumulative_write_suppression`, wired into `execute_cumulative_
/// aggregate`) — the executed `MERGE` carries **both** the slice predicate
/// on the `ON` clause's target read AND the `IS DISTINCT FROM` suppression
/// arm on the matched clause, byte-identical to a direct `emit_keyed_fold_
/// suppressed` call with the same slice.
#[tokio::test]
async fn keyed_fold_slice_predicated_merge_statements_come_from_the_emitter() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project_dir = tmp.path();
    std::fs::create_dir_all(project_dir.join("models/sources")).unwrap();

    std::fs::write(
        project_dir.join("models/sources/events.yml"),
        "description: statement-parity locality source.\n\
         mutation_profile: append_only\n\
         timeseries:\n\
         \x20\x20event_time_column: event_date\n\
         \x20\x20partition_column: event_date\n\
         \x20\x20granularity: day\n\
         columns:\n\
         \x20\x20- name: device_id\n\
         \x20\x20\x20\x20type: INTEGER\n\
         \x20\x20- name: event_date\n\
         \x20\x20\x20\x20type: DATE\n\
         \x20\x20- name: amount\n\
         \x20\x20\x20\x20type: DOUBLE\n",
    )
    .unwrap();

    // Route 1 (key-embedded): `event_date` is both the model's own
    // `timeseries.partition_column` and a `unique_key` column (GROUP BY 1,
    // 2) — the same composed shape `crates/smelt-runtime/tests/
    // locality_route1_slice_pruning.rs` exercises end-to-end (result
    // equivalence + slice-shape assertions). This test's own contribution is
    // the statement-parity leg: byte-identity against a direct emitter
    // call, the missing coverage this phase's review flagged.
    write_model(
        project_dir,
        "device_daily",
        "---\n\
         materialization: table\n\
         refresh: incremental\n\
         grain: key\n\
         timeseries:\n\
         \x20\x20event_time_column: event_date\n\
         \x20\x20partition_column: event_date\n\
         \x20\x20granularity: day\n\
         ---\n\
         SELECT device_id, event_date, MAX(amount) AS max_amount \
         FROM smelt.sources.events GROUP BY device_id, event_date",
    );

    let db_path = project_dir.join("run.duckdb");
    let smelt_yml = format!(
        "name: keyed_slice_statement_parity_test\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: {db}\n    schema: main\ndefault_materialization: table\ntarget: dev\n",
        db = db_path.display()
    );
    std::fs::write(project_dir.join("smelt.yml"), &smelt_yml).unwrap();

    {
        let conn = duckdb::Connection::open(&db_path).expect("open duckdb");
        conn.execute_batch(
            "CREATE SCHEMA IF NOT EXISTS main;\n\
             CREATE TABLE main.sources_events (device_id INTEGER, event_date DATE, amount DOUBLE);\n\
             INSERT INTO main.sources_events VALUES \
             (1, DATE '2024-01-01', 10.0), \
             (1, DATE '2024-01-02', 20.0), \
             (2, DATE '2024-01-02', 5.0);",
        )
        .expect("seed source table");
    }

    let config = Arc::new(Config::load(project_dir).expect("load config"));

    // Window 1: day 1 alone — first-run CREATE TABLE ... AS, no MERGE yet.
    {
        let (db, graph) = build_db_and_graph(project_dir, &config);
        let backend_slot: Arc<Mutex<Option<Arc<RecordingBackend>>>> = Arc::new(Mutex::new(None));
        let factory = RecordingBackendFactory {
            db_path: db_path.clone(),
            backend: Arc::clone(&backend_slot),
        };
        execute_project(
            "keyed-slice-statement-parity-run-1".to_string(),
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
        .expect("window 1 (create) must run");
    }

    // Window 2: day 2 alone — a single MERGE step carrying the locality
    // slice (zero margin, since the model's SQL has no lookback construct:
    // the slice is exactly this step's own date).
    let (db, graph) = build_db_and_graph(project_dir, &config);
    let backend_slot: Arc<Mutex<Option<Arc<RecordingBackend>>>> = Arc::new(Mutex::new(None));
    let factory = RecordingBackendFactory {
        db_path: db_path.clone(),
        backend: Arc::clone(&backend_slot),
    };
    execute_project(
        "keyed-slice-statement-parity-run-2".to_string(),
        make_request("dev", "2024-01-02", "2024-01-03"),
        Arc::clone(&config),
        graph,
        db,
        project_dir,
        &factory,
        &smelt_runtime::NoOpReporter,
        CancellationToken::new(),
    )
    .await
    .expect("window 2 (slice-predicated merge) must run");

    let backend = backend_slot
        .lock()
        .unwrap()
        .clone()
        .expect("backend recorded");
    let groups = backend.recorded_groups();
    assert_eq!(
        groups.len(),
        1,
        "window 2 covers exactly one day-step, one MERGE group: {:?}",
        groups
    );

    let merge_sql = &groups[0].statements[0].sql;
    assert_eq!(groups[0].statements.len(), 1);

    let key = vec!["device_id".to_string(), "event_date".to_string()];
    let folds = vec![(
        "max_amount".to_string(),
        "GREATEST(target.max_amount, delta.max_amount)".to_string(),
    )];
    let slice = TargetSlicePredicate::Range {
        partition_column: "event_date".to_string(),
        lower: "2024-01-02".to_string(),
        upper: "2024-01-02".to_string(),
    };

    let prefix = "MERGE INTO main.device_daily AS target USING (";
    let suffix = ") AS delta ON target.device_id = delta.device_id AND \
                  target.event_date = delta.event_date AND \
                  target.event_date BETWEEN '2024-01-02' AND '2024-01-02' \
                  WHEN MATCHED AND (target.max_amount IS DISTINCT FROM (GREATEST(target.\
                  max_amount, delta.max_amount))) THEN UPDATE SET \
                  max_amount = GREATEST(target.max_amount, delta.max_amount) \
                  WHEN NOT MATCHED THEN INSERT *";
    assert!(
        merge_sql.starts_with(prefix) && merge_sql.ends_with(suffix),
        "unexpected slice-predicated merge statement: {merge_sql}"
    );
    assert!(
        merge_sql.contains("BETWEEN") && merge_sql.contains("IS DISTINCT FROM"),
        "the composed model's suppressed merge must carry BOTH the slice predicate and the \
         suppression arm: {merge_sql}"
    );
    let delta_select = &merge_sql[prefix.len()..merge_sql.len() - suffix.len()];

    let expected = emit_keyed_fold_suppressed(
        "main.device_daily",
        &key,
        &folds,
        delta_select,
        Some(&slice),
        &["max_amount".to_string()],
        MaintenanceDialect::DuckDb,
    );
    assert_eq!(
        &expected, &groups[0],
        "executed slice-predicated MERGE group must be byte-identical to a direct emitter call \
         over the same table/key/folds/slice/delta_select"
    );

    // Result-equivalence: the CREATE (window 1) + slice-predicated MERGE
    // (window 2) statements the run actually executed must leave
    // `device_daily` multiset-equal to a full refresh of the model's own
    // aggregation over every seeded row.
    assert!(
        multiset_equal(
            backend.as_ref(),
            "SELECT * FROM main.device_daily",
            "SELECT device_id, event_date, MAX(amount) AS max_amount \
             FROM main.sources_events GROUP BY device_id, event_date"
        )
        .await,
        "the CREATE+slice-predicated-MERGE statements execute_project actually ran must \
         reproduce a full refresh"
    );
}

/// Statement-parity leg for the **checked route-3** (recurrence-bounded,
/// declared `r`) merge (`docs/specs/incremental_models.md` §"Key temporal
/// locality", route 3; `docs/plans/20260715-composed-axes-conditional-
/// maintenance.md` Phase A4): the out-of-slice match probe and the merge
/// itself are each byte-identical to a direct call of their single-owner
/// emitters (`emit_recurrence_bound_probe`, `emit_keyed_fold`).
///
/// Driven directly through `maintenance_driver::run_windowed_keyed_
/// maintenance` (not the full `execute_project` pipeline): route 3's
/// flagship shape needs an extremal-fold (`MIN`/`MAX`) partition column,
/// which trips the *unrelated* NOT-NULL diagnostic `execute_project`'s
/// pre-execution gate enforces regardless of locality admission — the
/// same pre-existing blocker `docs/specs/incremental_models.md` §Known
/// Divergences documents for route 2's own real-fixture coverage. Calling
/// the driver directly still proves the actual SQL a run executes matches
/// the emitters, the parity gate's whole point; it does not touch the
/// (separately tracked) nullability gap.
#[tokio::test]
async fn recurrence_bound_probe_and_checked_merge_come_from_the_emitters() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let db_path = tmp.path().join("run.duckdb");
    {
        let conn = duckdb::Connection::open(&db_path).expect("open duckdb");
        conn.execute_batch(
            "CREATE SCHEMA IF NOT EXISTS main;\n\
             CREATE TABLE main.raw_events (event_id INTEGER, event_ts TIMESTAMP, event_date DATE);",
        )
        .expect("create raw_events");
    }
    let inner = DuckDbBackend::new(&db_path, "main")
        .await
        .expect("open backend");
    let backend = RecordingBackend::new(inner);

    let classification = CumulativeClassification {
        unique_key: vec!["event_id".to_string()],
        aggregator_columns: vec![AggregatorColumn {
            output_name: "last_seen_date".to_string(),
            per_partition_agg: "MAX".to_string(),
            cross_partition_combiner: CrossPartitionCombiner::Max,
        }],
        driving_source: DrivingSource {
            name: "smelt.sources.raw.events".to_string(),
            timeseries: smelt_core::config::TimeseriesConfig {
                event_time_column: "event_ts".to_string(),
                partition_column: "event_date".to_string(),
                granularity: smelt_core::config::Granularity::Day,
                week_start: None,
                assert_monotonic: false,
            },
        },
    };
    let slice = LocalitySlice::RecurrenceBounded {
        partition_column: "last_seen_date".to_string(),
        margin_before: smelt_logical::analysis::source_bounds::Seconds::days(3),
        margin_after: smelt_logical::analysis::source_bounds::Seconds::ZERO,
        r: smelt_logical::analysis::source_bounds::Seconds::days(3),
    };
    let compile_step = |step: &smelt_runtime::maintenance_driver::MaintenanceStep| {
        Ok(format!(
            "SELECT event_id, MAX(event_date) AS last_seen_date FROM main.raw_events \
             WHERE event_date = '{}' GROUP BY event_id",
            step.partition_value
        ))
    };

    backend
        .execute_sql(
            "INSERT INTO main.raw_events VALUES (1, TIMESTAMP '2026-02-01 00:00:00', DATE \
             '2026-02-01')",
        )
        .await
        .expect("insert day 1");
    let create_steps = driving_steps(
        "2026-02-01",
        "2026-02-02",
        &smelt_core::config::Granularity::Day,
    )
    .expect("steps");
    run_windowed_keyed_maintenance(
        &backend,
        "events_last_seen",
        "main",
        "events_last_seen",
        &create_steps,
        &classification,
        Some(&slice),
        &smelt_logical::maintenance::choice::WriteSuppression::Unconditional {
            why: "test asserts the unconditional checked-merge shape".to_string(),
        },
        compile_step,
    )
    .await
    .expect("day 1 create must succeed");

    // Day 2: an in-bound redelivery — the probe must run, find no
    // violation, and the merge must apply.
    backend
        .execute_sql(
            "INSERT INTO main.raw_events VALUES (1, TIMESTAMP '2026-02-02 00:00:00', DATE \
             '2026-02-02')",
        )
        .await
        .expect("insert day 2");
    let steps = driving_steps(
        "2026-02-02",
        "2026-02-03",
        &smelt_core::config::Granularity::Day,
    )
    .expect("steps");
    run_windowed_keyed_maintenance(
        &backend,
        "events_last_seen",
        "main",
        "events_last_seen",
        &steps,
        &classification,
        Some(&slice),
        &smelt_logical::maintenance::choice::WriteSuppression::Unconditional {
            why: "test asserts the unconditional checked-merge shape".to_string(),
        },
        compile_step,
    )
    .await
    .expect("in-bound redelivery must merge cleanly");

    // The probe: byte-identical to a direct `emit_recurrence_bound_probe`
    // call over this step's own delta SELECT and slice lower bound
    // (2026-02-02 widened backward by r=3 days → 2026-01-30).
    let executed = backend.recorded_sql();
    let probe_sql = executed
        .iter()
        .find(|s| s.contains("__recurrence_violations"))
        .expect("the checked route must execute the out-of-slice match probe");
    let delta_select = "SELECT event_id, MAX(event_date) AS last_seen_date FROM main.raw_events \
                         WHERE event_date = '2026-02-02' GROUP BY event_id";
    let expected_probe = emit_recurrence_bound_probe(
        "main.events_last_seen",
        &["event_id".to_string()],
        "last_seen_date",
        delta_select,
        "2026-01-30",
    );
    assert_eq!(
        probe_sql, &expected_probe.sql,
        "executed probe must be byte-identical to a direct emitter call"
    );

    // The merge: byte-identical to a direct `emit_keyed_fold` call with the
    // same `Range` predicate the checked route resolves to (same shape as
    // route 1's window).
    let groups = backend.recorded_groups();
    let merge_group = groups
        .iter()
        .find(|g| g.statements[0].sql.starts_with("MERGE INTO"))
        .expect("the merge action must have executed via execute_statement_group");
    let range_slice = TargetSlicePredicate::Range {
        partition_column: "last_seen_date".to_string(),
        lower: "2026-01-30".to_string(),
        upper: "2026-02-02".to_string(),
    };
    let expected_merge = emit_keyed_fold(
        "main.events_last_seen",
        &["event_id".to_string()],
        &[(
            "last_seen_date".to_string(),
            "GREATEST(target.last_seen_date, delta.last_seen_date)".to_string(),
        )],
        delta_select,
        Some(&range_slice),
        MaintenanceDialect::DuckDb,
    );
    assert_eq!(
        merge_group, &expected_merge,
        "executed checked-merge group must be byte-identical to a direct emitter call"
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
        run_checks: false,
        checks: vec![],
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

/// Phase C4 (`docs/plans/20260715-composed-axes-conditional-maintenance.md`)
/// — the change-suppressed column-scoped MERGE (T1) dispatches through
/// `maintenance_driver::execute_column_scoped_merge_full` exactly like the
/// unconditional variant above, but building its `StatementGroup` via
/// `emit_column_scoped_merge_suppressed` and handing it straight to
/// `Backend::execute_statement_group` — never `Backend::merge_into` (which
/// would route back through the unconditional emitter). This proves the
/// EXECUTED statement text is byte-identical to a direct call of
/// `emit_column_scoped_merge_suppressed` over the same inputs, the same
/// property `column_scoped_merge_statements_come_from_the_emitter` proves
/// for the unconditional variant.
#[tokio::test]
async fn suppressed_column_scoped_merge_statements_come_from_the_emitter() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("test.duckdb");
    let inner = DuckDbBackend::new(&db_path, "main")
        .await
        .expect("open duckdb");
    let backend = RecordingBackend::new(inner);

    backend
        .execute_sql("CREATE TABLE main.dim_users (user_id BIGINT, tier VARCHAR)")
        .await
        .expect("create target table");
    backend
        .execute_sql("INSERT INTO main.dim_users VALUES (1, 'bronze'), (2, 'silver')")
        .await
        .expect("seed target table");
    backend
        .execute_sql("CREATE TABLE main.sources_users (user_id BIGINT, tier VARCHAR)")
        .await
        .expect("create dim table");
    backend
        .execute_sql("INSERT INTO main.sources_users VALUES (1, 'gold'), (2, 'silver')")
        .await
        .expect("seed dim table (user_id=1 mutated)");

    let dimension_batch_sql = "SELECT u.user_id, u.tier FROM main.sources_users u";
    let suppression = smelt_logical::maintenance::choice::WriteSuppression::Suppressed {
        compared_columns: vec!["tier".to_string()],
    };

    let window = smelt_backend::PartitionRange {
        column: String::new(),
        start: "2026-01-01".to_string(),
        end: "2026-01-02".to_string(),
    };
    smelt_runtime::maintenance_driver::execute_column_scoped_merge_full(
        &backend,
        "main",
        "dim_users",
        &["user_id".to_string()],
        dimension_batch_sql,
        &suppression,
        &window,
    )
    .await
    .expect("suppressed column-scoped merge must succeed");

    let groups = backend.recorded_groups();
    let merge_groups: Vec<_> = groups
        .iter()
        .filter(|g| g.statements[0].sql.starts_with("MERGE INTO"))
        .collect();
    assert_eq!(merge_groups.len(), 1, "exactly one MERGE group: {groups:?}");
    let group = merge_groups[0];
    assert!(!group.transactional);
    assert_eq!(group.statements.len(), 1);

    let expected = smelt_logical::maintenance::emit::emit_column_scoped_merge_suppressed(
        "main.dim_users",
        &["user_id".to_string()],
        dimension_batch_sql,
        &["tier".to_string()],
        MaintenanceDialect::DuckDb,
    );
    assert_eq!(
        &expected, group,
        "executed suppressed MERGE group must be byte-identical to a direct emitter call over \
         the same inputs"
    );

    // Result-equivalence: the same full-refresh oracle property the
    // unconditional leg proves.
    assert!(
        multiset_equal(
            &backend,
            "SELECT * FROM main.dim_users",
            "SELECT user_id, tier FROM main.sources_users"
        )
        .await,
        "the suppressed MERGE must reproduce a full refresh"
    );
}

/// T5 (`docs/plans/20260715-composed-axes-conditional-maintenance.md` Phase
/// D2) — structural no-authoring gate extension for observed-delta
/// recording. `docs/specs/incremental_models.md` §"The graph layer" —
/// "Observed deltas on model edges" places the recorded delta in the SAME
/// warehouse-resident, "bookkeeping" class as the reconciliation ledger
/// (`smelt_state::ddl_duckdb::generate_ledger_table_ddl`/
/// `generate_ledger_insert_sql`): D1's ruling is that this is smelt-state
/// storage for a run's own byproduct, not a maintenance statement the run's
/// *write* executes, so `smelt_logical::maintenance::emit`'s single-owner
/// rule does not apply to it, and `no_maintenance_statement_authoring_
/// outside_the_emitter` above is not extended with an allowlist entry (the
/// recording query is a `SELECT ... LEFT JOIN`, not one of that gate's
/// forbidden `DELETE FROM `/`MERGE INTO `/`CREATE TABLE {}.{} AS`/
/// `CREATE TEMP TABLE ` shapes — confirmed by that test's own green run,
/// unmodified by this phase).
///
/// What IS asserted here, as the phase's own structural gate: the "one
/// comparison, two consumers" claim
/// (`crate::maintenance_driver::changed_row_predicate`'s doc comment) — the
/// observed-delta recording query's `IS DISTINCT FROM` guard must be
/// BYTE-IDENTICAL to the suppressed MERGE's own matched-arm guard over the
/// same `compared_columns`, so change-suppression and delta-recording can
/// never silently diverge on what counts as "changed".
#[test]
fn observed_delta_predicate_matches_suppressed_merge_guard_byte_for_byte() {
    let compared_columns = vec!["tier".to_string(), "email".to_string()];

    let merge_group = smelt_logical::maintenance::emit::emit_column_scoped_merge_suppressed(
        "main.dim_users",
        &["user_id".to_string()],
        "SELECT * FROM main.sources_users",
        &compared_columns,
        MaintenanceDialect::DuckDb,
    );
    let merge_sql = &merge_group.statements[0].sql;

    let record_sql = smelt_runtime::maintenance_driver::changed_row_predicate(
        "target",
        "source",
        &compared_columns,
    );

    assert!(
        merge_sql.contains(&record_sql),
        "the recorded-delta predicate must appear byte-identical inside the suppressed MERGE's \
         own matched-arm guard — predicate: {record_sql:?}, MERGE: {merge_sql:?}"
    );

    // The recording query built off the SAME predicate carries it verbatim
    // too — a second cross-check at the query-assembly level, not just the
    // bare predicate.
    let changed_keys_query = smelt_runtime::maintenance_driver::changed_keys_select(
        "main.dim_users",
        &["user_id".to_string()],
        "SELECT * FROM main.sources_users",
        &compared_columns,
        None,
    );
    assert!(
        changed_keys_query.contains(&record_sql),
        "changed_keys_select must carry the identical predicate text, got: {changed_keys_query:?}"
    );
}

/// Phase C5 (`docs/plans/20260715-composed-axes-conditional-maintenance.md`)
/// — the change-suppressed keyed-fold `MERGE` (T1 for `refresh: keyed`
/// models): `emit_keyed_fold_suppressed` carries the same suppression
/// predicate machinery as C4's `emit_column_scoped_merge_suppressed`, but
/// compares the stored value against the fold's own combine expression.
/// This is a direct-dispatch leg (no `execute_project` model pipeline
/// involved, matching this phase's "runtime e2e" test — the abstract
/// `MaintenancePlan`/`choice::resolve_keyed_write_mechanism` this phase adds
/// is not yet wired into the live `refresh: keyed` per-partition loop
/// (`smelt_runtime::cumulative`); that wiring is out of this phase's file
/// scope). It proves two things over a real DuckDB connection:
///
/// - The executed `StatementGroup` is byte-identical to a direct
///   `emit_keyed_fold_suppressed` call over the same inputs.
/// - A `run_marker` fold column — only ever overwritten when the matched
///   arm's `UPDATE SET` actually fires — proves the suppressed row was
///   never written at all (not merely that it landed on the same bits): a
///   device whose delta contributes zero new events (`event_count`
///   unchanged after the additive combine) keeps its **prior** run's
///   marker, while a device whose combined result differs gets the new
///   run's marker, and a brand-new device is inserted with it.
#[tokio::test]
async fn suppressed_keyed_fold_statements_come_from_the_emitter() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("test.duckdb");
    let inner = DuckDbBackend::new(&db_path, "main")
        .await
        .expect("open duckdb");
    let backend = RecordingBackend::new(inner);

    backend
        .execute_sql(
            "CREATE TABLE main.device_daily (device_id BIGINT, event_count BIGINT, run_marker \
             VARCHAR)",
        )
        .await
        .expect("create target table");
    backend
        .execute_sql("INSERT INTO main.device_daily VALUES (1, 5, 'run1'), (2, 3, 'run1')")
        .await
        .expect("seed target table");

    // Device 1's delta contributes zero new events (an unchanged-effect
    // re-run); device 2's delta genuinely adds events; device 3 is brand
    // new.
    let delta_select = "SELECT * FROM (VALUES (1, 0, 'run2'), (2, 4, 'run2'), (3, 10, 'run2')) AS \
                         t(device_id, event_count, run_marker)";
    let folds = vec![
        (
            "event_count".to_string(),
            "target.event_count + delta.event_count".to_string(),
        ),
        ("run_marker".to_string(), "delta.run_marker".to_string()),
    ];
    let key = vec!["device_id".to_string()];
    let compared_columns = vec!["event_count".to_string()];

    let group = emit_keyed_fold_suppressed(
        "main.device_daily",
        &key,
        &folds,
        delta_select,
        None,
        &compared_columns,
        MaintenanceDialect::DuckDb,
    );
    backend
        .execute_statement_group(&group)
        .await
        .expect("suppressed keyed-fold merge must succeed");

    let recorded = backend.recorded_groups();
    assert_eq!(recorded.len(), 1);
    assert_eq!(&recorded[0], &group);
    let expected = emit_keyed_fold_suppressed(
        "main.device_daily",
        &key,
        &folds,
        delta_select,
        None,
        &compared_columns,
        MaintenanceDialect::DuckDb,
    );
    assert_eq!(
        &expected, &recorded[0],
        "executed suppressed keyed-fold group must be byte-identical to a direct emitter call \
         over the same inputs"
    );

    let rows = backend
        .execute_sql(
            "SELECT device_id, event_count, run_marker FROM main.device_daily ORDER BY device_id",
        )
        .await
        .expect("read back target");
    let batch = &rows[0];
    let markers: Vec<String> = {
        let col = batch
            .column(2)
            .as_any()
            .downcast_ref::<arrow::array::StringArray>()
            .expect("run_marker is a string column");
        (0..col.len()).map(|i| col.value(i).to_string()).collect()
    };
    assert_eq!(
        markers,
        vec!["run1".to_string(), "run2".to_string(), "run2".to_string()],
        "device 1's suppressed row must keep its prior run's marker (never written); device 2 \
         (changed) and device 3 (new) must carry the new run's marker"
    );

    assert!(
        multiset_equal(
            &backend,
            "SELECT device_id, event_count FROM main.device_daily",
            "SELECT device_id, event_count FROM (VALUES (1, 5), (2, 7), (3, 10)) AS \
             t(device_id, event_count)"
        )
        .await,
        "the suppressed keyed-fold merge must reproduce the full-refresh oracle's combined state"
    );
}

/// Phase C5 — the staged-candidate conditional `DELETE`+`INSERT` (T2): the
/// merge-less keyed-shaped realisation. Proves the executed `StatementGroup`
/// is byte-identical to a direct `emit_staged_candidate_conditional` call,
/// that the same `run_marker` technique proves an unchanged row is never
/// touched (its prior marker survives), and that a mid-group failure rolls
/// back the whole transaction — including the staged temp relation's own
/// `CREATE` — leaving no temp relation behind.
#[tokio::test]
async fn staged_candidate_conditional_statements_come_from_the_emitter() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("test.duckdb");
    let inner = DuckDbBackend::new(&db_path, "main")
        .await
        .expect("open duckdb");
    let backend = RecordingBackend::new(inner);

    backend
        .execute_sql(
            "CREATE TABLE main.dim_users (user_id BIGINT, tier VARCHAR, run_marker VARCHAR)",
        )
        .await
        .expect("create target table");
    backend
        .execute_sql(
            "INSERT INTO main.dim_users VALUES (1, 'bronze', 'run1'), (2, 'silver', 'run1'), (3, \
             'gold', 'run1')",
        )
        .await
        .expect("seed target table");

    // user 1: unchanged tier ('bronze' -> 'bronze'); user 2: changed tier;
    // user 4: brand new. user 3 is absent from the candidate set (out of
    // this run's touched region) and must be left untouched entirely.
    let candidate_select = "SELECT * FROM (VALUES (1, 'bronze', 'run2'), (2, 'platinum', \
                             'run2'), (4, 'new', 'run2')) AS t(user_id, tier, run_marker)";
    let key = vec!["user_id".to_string()];
    let compared_columns = vec!["tier".to_string()];

    let group = smelt_logical::maintenance::emit::emit_staged_candidate_conditional(
        "main.dim_users",
        "__smelt_staged_dim_users",
        &key,
        candidate_select,
        &compared_columns,
        MaintenanceDialect::DuckDb,
    );
    backend
        .execute_statement_group(&group)
        .await
        .expect("staged-candidate conditional write must succeed");

    let recorded = backend.recorded_groups();
    assert_eq!(recorded.len(), 1);
    let expected = smelt_logical::maintenance::emit::emit_staged_candidate_conditional(
        "main.dim_users",
        "__smelt_staged_dim_users",
        &key,
        candidate_select,
        &compared_columns,
        MaintenanceDialect::DuckDb,
    );
    assert_eq!(
        &expected, &recorded[0],
        "executed staged-candidate group must be byte-identical to a direct emitter call over \
         the same inputs"
    );

    let rows = backend
        .execute_sql("SELECT user_id, tier, run_marker FROM main.dim_users ORDER BY user_id")
        .await
        .expect("read back target");
    let batch = &rows[0];
    let tiers: Vec<String> = {
        let col = batch
            .column(1)
            .as_any()
            .downcast_ref::<arrow::array::StringArray>()
            .expect("tier is a string column");
        (0..col.len()).map(|i| col.value(i).to_string()).collect()
    };
    let markers: Vec<String> = {
        let col = batch
            .column(2)
            .as_any()
            .downcast_ref::<arrow::array::StringArray>()
            .expect("run_marker is a string column");
        (0..col.len()).map(|i| col.value(i).to_string()).collect()
    };
    assert_eq!(
        tiers,
        vec![
            "bronze".to_string(),
            "platinum".to_string(),
            "gold".to_string(),
            "new".to_string()
        ]
    );
    assert_eq!(
        markers,
        vec![
            "run1".to_string(), // user 1: suppressed, never deleted/reinserted
            "run2".to_string(), // user 2: changed, deleted+reinserted
            "run1".to_string(), // user 3: absent from candidate set, untouched
            "run2".to_string(), // user 4: new, inserted
        ],
        "an unchanged staged candidate must never delete/reinsert its row (prior marker \
         survives); a changed or new row must carry the new run's marker"
    );

    assert!(
        multiset_equal(
            &backend,
            "SELECT user_id, tier FROM main.dim_users",
            "SELECT user_id, tier FROM (VALUES (1, 'bronze'), (2, 'platinum'), (3, 'gold'), (4, \
             'new')) AS t(user_id, tier)"
        )
        .await,
        "the staged-candidate conditional write must reproduce the full-refresh oracle"
    );

    let staged_relations = backend
        .execute_sql(
            "SELECT count(*) FROM duckdb_tables() WHERE table_name = \
             '__smelt_staged_dim_users'",
        )
        .await
        .expect("query duckdb_tables");
    let count = staged_relations[0]
        .column(0)
        .as_any()
        .downcast_ref::<Int64Array>()
        .expect("count(*) is Int64")
        .value(0);
    assert_eq!(
        count, 0,
        "the staged temp relation must be dropped by the end of a successful group"
    );
}

/// A mid-group failure (the candidate `INSERT`'s projection does not match
/// the staged relation's own `CREATE`-derived shape — a column-count
/// mismatch DuckDB rejects) must roll back the **entire** transaction,
/// including the temp relation's own `CREATE`: no temp relation is left
/// behind, and the target table is completely untouched.
#[tokio::test]
async fn staged_candidate_interrupted_run_leaves_no_temp_relation_behind() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("test.duckdb");
    let inner = DuckDbBackend::new(&db_path, "main")
        .await
        .expect("open duckdb");
    let backend = RecordingBackend::new(inner);

    backend
        .execute_sql("CREATE TABLE main.dim_users (user_id BIGINT, tier VARCHAR)")
        .await
        .expect("create target table");
    backend
        .execute_sql("INSERT INTO main.dim_users VALUES (1, 'bronze')")
        .await
        .expect("seed target table");

    // Hand-build a group whose CREATE (shape-only, `LIMIT 0` over a 2-column
    // projection) disagrees with its own INSERT (a 3-column projection) —
    // the same shape `emit_staged_candidate_conditional` would build if a
    // caller ever violated its full-row-projection contract. DuckDB rejects
    // the INSERT with a column-count mismatch mid-transaction.
    let mut group = smelt_logical::maintenance::emit::emit_staged_candidate_conditional(
        "main.dim_users",
        "__smelt_staged_broken",
        &["user_id".to_string()],
        "SELECT user_id, tier FROM (VALUES (1, 'bronze')) AS t(user_id, tier)",
        &["tier".to_string()],
        MaintenanceDialect::DuckDb,
    );
    group.statements[1] = smelt_logical::maintenance::emit::MaintenanceStatement {
        sql: "INSERT INTO __smelt_staged_broken SELECT user_id, tier, 'extra' FROM (VALUES (1, \
              'bronze')) AS t(user_id, tier, junk)"
            .to_string(),
    };

    let result = backend.execute_statement_group(&group).await;
    assert!(
        result.is_err(),
        "the deliberately-broken INSERT must fail: {result:?}"
    );

    let staged_relations = backend
        .execute_sql(
            "SELECT count(*) FROM duckdb_tables() WHERE table_name = '__smelt_staged_broken'",
        )
        .await
        .expect("query duckdb_tables");
    let count = staged_relations[0]
        .column(0)
        .as_any()
        .downcast_ref::<Int64Array>()
        .expect("count(*) is Int64")
        .value(0);
    assert_eq!(
        count, 0,
        "a rolled-back transaction must leave no staged temp relation behind — its own CREATE \
         is part of the same failed transaction"
    );

    assert!(
        multiset_equal(
            &backend,
            "SELECT user_id, tier FROM main.dim_users",
            "SELECT user_id, tier FROM (VALUES (1, 'bronze')) AS t(user_id, tier)"
        )
        .await,
        "the target table must be completely untouched by a rolled-back staged-candidate group"
    );
}

/// `docs/plans/20260715-composed-axes-conditional-maintenance.md` Phase C6's
/// own real-fixture requirement: `examples/web_analytics`'s
/// `silver.events_deduped` (the flagship composed shape — key-addressed via
/// `event_id`, time-partitioned via `first_seen_date`, admitted through
/// route 3's declared `key_recurrence`) driven through the SAME real model
/// text `smelt run` executes for that example, via `execute_project` and a
/// `RecordingBackend` so the executed SQL can be inspected directly — the
/// real-fixture counterpart to `keyed_fold_slice_predicated_merge_
/// statements_come_from_the_emitter`'s synthetic composed fixture above.
///
/// Only the two files this model actually needs
/// (`models/sources/raw/events.yml`, `models/silver/events_deduped.sql`)
/// are copied byte-for-byte off disk — not the whole example (which also
/// needs `smelt-datagen`-generated Parquet + the `functions/` dir neither
/// model here calls) — into a fresh scratch project, seeded directly via
/// `raw.events` INSERTs rather than a full datagen run.
///
/// Day 1 seeds `event_id` 1; day 2 redelivers the SAME `event_id` 1 with
/// byte-identical payload fields (only `arrival_time`, a column this model
/// never selects, would differ in a real redelivery — irrelevant here) —
/// exactly `datagen.yaml`'s `redelivery:` storm, collapsed to one pair —
/// alongside a genuinely new `event_id` 2. Day 2's `MERGE` step must carry
/// **both** predicates (the route-3 `RecurrenceBounded` slice on the target
/// read, `IS DISTINCT FROM` suppression on the matched arm) and must write
/// zero rows for the redelivered key while still inserting the new one.
#[tokio::test]
async fn events_deduped_composed_suppression_storm_rerun_writes_zero_rows() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project_dir = tmp.path();
    std::fs::create_dir_all(project_dir.join("models/sources/raw")).unwrap();
    std::fs::create_dir_all(project_dir.join("models/silver")).unwrap();

    let fixture_root = repo_root().join("examples/web_analytics");
    let events_yml = std::fs::read_to_string(fixture_root.join("models/sources/raw/events.yml"))
        .expect("read examples/web_analytics/models/sources/raw/events.yml");
    let events_deduped_sql =
        std::fs::read_to_string(fixture_root.join("models/silver/events_deduped.sql"))
            .expect("read examples/web_analytics/models/silver/events_deduped.sql");
    std::fs::write(
        project_dir.join("models/sources/raw/events.yml"),
        &events_yml,
    )
    .unwrap();
    std::fs::write(
        project_dir.join("models/silver/events_deduped.sql"),
        &events_deduped_sql,
    )
    .unwrap();

    let db_path = project_dir.join("run.duckdb");
    let smelt_yml = format!(
        "name: events_deduped_storm_test\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: {db}\n    schema: main\ndefault_materialization: table\ntarget: dev\n",
        db = db_path.display()
    );
    std::fs::write(project_dir.join("smelt.yml"), &smelt_yml).unwrap();

    {
        let conn = duckdb::Connection::open(&db_path).expect("open duckdb");
        conn.execute_batch(
            "CREATE SCHEMA IF NOT EXISTS raw;\n\
             CREATE TABLE raw.events (\n\
               event_id BIGINT, device_id INTEGER, user_id INTEGER, seconds_in_day INTEGER,\n\
               event_time VARCHAR, arrival_time VARCHAR, utm_campaign VARCHAR, payload VARCHAR,\n\
               event_date VARCHAR\n\
             );\n\
             -- Day 1: event_id 1 first seen.\n\
             INSERT INTO raw.events VALUES (\n\
               1, 10, NULL, 100, '2026-04-01T00:01:40', '2026-04-01T00:01:41', NULL,\n\
               '{\"event_name\": \"page_view\", \"platform\": \"web\", \"url\": \"/home\"}',\n\
               '2026-04-01'\n\
             );\n\
             -- Day 2: event_id 1 redelivered (byte-identical payload fields — only\n\
             -- arrival_time, never selected by this model, differs), plus a\n\
             -- genuinely new event_id 2.\n\
             INSERT INTO raw.events VALUES (\n\
               1, 10, NULL, 100, '2026-04-01T00:01:40', '2026-04-02T00:01:41', NULL,\n\
               '{\"event_name\": \"page_view\", \"platform\": \"web\", \"url\": \"/home\"}',\n\
               '2026-04-01'\n\
             );\n\
             INSERT INTO raw.events VALUES (\n\
               2, 11, NULL, 200, '2026-04-02T00:03:20', '2026-04-02T00:03:21', NULL,\n\
               '{\"event_name\": \"page_view\", \"platform\": \"web\", \"url\": \"/pricing\"}',\n\
               '2026-04-02'\n\
             );",
        )
        .expect("seed raw.events");
    }

    let config = Arc::new(Config::load(project_dir).expect("load config"));

    // Window 1: day 1 alone — first-run CREATE, no MERGE yet.
    {
        let (db, graph) = build_db_and_graph(project_dir, &config);
        let backend_slot: Arc<Mutex<Option<Arc<RecordingBackend>>>> = Arc::new(Mutex::new(None));
        let factory = RecordingBackendFactory {
            db_path: db_path.clone(),
            backend: Arc::clone(&backend_slot),
        };
        execute_project(
            "events-deduped-storm-run-1".to_string(),
            make_request("dev", "2026-04-01", "2026-04-02"),
            Arc::clone(&config),
            graph,
            db,
            project_dir,
            &factory,
            &smelt_runtime::NoOpReporter,
            CancellationToken::new(),
        )
        .await
        .expect("day 1 (create) must run");
    }

    // Window 2: day 2 — the redelivery-storm step, a single MERGE.
    let (db, graph) = build_db_and_graph(project_dir, &config);
    let backend_slot: Arc<Mutex<Option<Arc<RecordingBackend>>>> = Arc::new(Mutex::new(None));
    let factory = RecordingBackendFactory {
        db_path: db_path.clone(),
        backend: Arc::clone(&backend_slot),
    };
    execute_project(
        "events-deduped-storm-run-2".to_string(),
        make_request("dev", "2026-04-02", "2026-04-03"),
        Arc::clone(&config),
        graph,
        db,
        project_dir,
        &factory,
        &smelt_runtime::NoOpReporter,
        CancellationToken::new(),
    )
    .await
    .expect("day 2 (redelivery-storm merge) must run");

    let backend = backend_slot
        .lock()
        .unwrap()
        .clone()
        .expect("backend recorded");
    let groups = backend.recorded_groups();
    let merge_group = groups
        .iter()
        .find(|g| g.statements[0].sql.starts_with("MERGE INTO"))
        .expect("day 2 must execute exactly one MERGE group");
    let merge_sql = &merge_group.statements[0].sql;

    assert!(
        merge_sql.contains("BETWEEN"),
        "the composed model's merge must carry the route-3 recurrence-bounded slice on the \
         target read: {merge_sql}"
    );
    assert!(
        merge_sql.contains("IS DISTINCT FROM"),
        "the composed model's merge must carry the suppression arm: {merge_sql}"
    );

    // Zero-write proof: reissue the exact statement text the run recorded
    // — DuckDB's own `MERGE` returns the count of rows it actually
    // modified (`crates/smelt-runtime/tests/technique_lowering.rs::
    // merge_affected_row_count`'s own technique). The run already brought
    // the target to its converged state, so replaying the identical
    // statement now must match every row (`event_id` 1's redelivered
    // duplicate, `event_id` 2 already inserted) but write none of them.
    let replay = backend
        .execute_sql(merge_sql)
        .await
        .expect("replaying the recorded merge must succeed");
    let batch = replay.first().expect("MERGE returns one Count row");
    let affected = batch
        .column(0)
        .as_any()
        .downcast_ref::<Int64Array>()
        .expect("Count column is Int64")
        .value(0);
    assert_eq!(
        affected, 0,
        "replaying day 2's already-converged merge must write zero rows — the redelivery \
         storm's unchanged payload must be fully suppressed"
    );

    // Result-equivalence: the maintained state must still equal a full
    // refresh of the model's own MIN-fold dedup over every seeded row.
    assert!(
        multiset_equal(
            backend.as_ref(),
            "SELECT event_id, device_id, user_id, first_seen_date FROM main.silver_events_deduped",
            "SELECT event_id, MIN(device_id) AS device_id, MIN(user_id) AS user_id, \
             MIN(CAST(event_date AS DATE)) AS first_seen_date FROM raw.events GROUP BY event_id"
        )
        .await,
        "the composed suppressed-merge run must still reproduce a full refresh"
    );
}

// =============================================================================
// T3 — delta-restricted region recompute over a model edge (`docs/plans/
// 20260715-composed-axes-conditional-maintenance.md` Phase E3): the
// statements `maintenance_driver::execute_delete_insert_with_delta_
// restriction` actually executes must be byte-identical to a direct call of
// `emit_delete_insert_delta_restricted`/`emit_delete_insert` with the same
// inputs — the same proof shape as the suppressed-MERGE and staged-
// candidate legs above.
// =============================================================================

/// A licensed restriction (`P1` Closed ∧ a non-empty recorded delta) must
/// execute exactly `emit_delete_insert_delta_restricted`'s own output, byte
/// for byte.
#[tokio::test]
async fn delta_restricted_recompute_statements_come_from_the_emitter() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("test.duckdb");
    let inner = DuckDbBackend::new(&db_path, "main")
        .await
        .expect("open duckdb");
    let backend = RecordingBackend::new(inner);

    backend
        .execute_sql("CREATE TABLE main.enriched (event_id VARCHAR, event_date DATE, tier VARCHAR)")
        .await
        .expect("create target table");
    backend
        .execute_sql(
            "INSERT INTO main.enriched VALUES ('ev-1', '2026-07-01', 'OLD'), \
             ('ev-2', '2026-07-01', 'OLD')",
        )
        .await
        .expect("seed target table");
    backend
        .execute_sql(
            "CREATE TABLE main.enrichment_recompute (event_id VARCHAR, event_date DATE, tier VARCHAR)",
        )
        .await
        .expect("create recompute source");
    backend
        .execute_sql(
            "INSERT INTO main.enrichment_recompute VALUES ('ev-1', '2026-07-01', 'NEW'), \
             ('ev-2', '2026-07-01', 'NEW')",
        )
        .await
        .expect("seed recompute source");

    let ensure_sql = smelt_state::ddl_duckdb::generate_observed_delta_table_ddl("main");
    backend
        .execute_sql(&ensure_sql)
        .await
        .expect("ensure observed-delta table");
    let upsert_sql = smelt_state::ddl_duckdb::generate_observed_delta_upsert_sql(
        "main",
        "silver.fact",
        "2026-07-01",
        "2026-07-02",
        "SELECT * FROM (VALUES ('ev-1', NULL)) AS t(delta_key, delta_partition)",
    );
    backend
        .execute_sql(&upsert_sql)
        .await
        .expect("record the upstream observed delta");

    let region = smelt_logical::maintenance::emit::Region {
        start: "'2026-07-01'".to_string(),
        end: "'2026-07-02'".to_string(),
    };
    let body = "SELECT event_id, event_date, tier FROM main.enrichment_recompute";
    let closure = smelt_logical::maintenance::SkeletonSourceClosure::Closed;

    smelt_runtime::maintenance_driver::execute_delete_insert_with_delta_restriction(
        &backend,
        "main",
        "enriched",
        "event_date",
        &region,
        body,
        Some("event_id"),
        Some(&closure),
        "silver.fact",
        "2026-07-01",
        "2026-07-02",
        smelt_logical::maintenance::emit::MaintenanceDialect::DuckDb,
    )
    .await
    .expect("delta-restricted recompute must succeed");

    let groups = backend.recorded_groups();
    let delete_insert_groups: Vec<_> = groups
        .iter()
        .filter(|g| g.statements[0].sql.starts_with("DELETE FROM main.enriched"))
        .collect();
    assert_eq!(
        delete_insert_groups.len(),
        1,
        "exactly one delta-restricted DELETE+INSERT group: {groups:?}"
    );
    let group = delete_insert_groups[0];

    let expected = smelt_logical::maintenance::emit::emit_delete_insert_delta_restricted(
        "main.enriched",
        "event_date",
        &region,
        body,
        "event_id",
        &["ev-1".to_string()],
        smelt_logical::maintenance::emit::MaintenanceDialect::DuckDb,
    );
    assert_eq!(
        group.statements, expected.statements,
        "the executed delta-restricted group must be byte-identical to a direct emitter call \
         over the same inputs"
    );
    assert_eq!(group.transactional, expected.transactional);
}

/// An `Open` closure (or an absent/empty delta — asserted below) must
/// execute exactly `emit_delete_insert`'s own unrestricted output, never a
/// partially-restricted variant.
#[tokio::test]
async fn open_closure_recompute_statements_come_from_the_unrestricted_emitter() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("test.duckdb");
    let inner = DuckDbBackend::new(&db_path, "main")
        .await
        .expect("open duckdb");
    let backend = RecordingBackend::new(inner);

    backend
        .execute_sql("CREATE TABLE main.enriched (event_id VARCHAR, event_date DATE, tier VARCHAR)")
        .await
        .expect("create target table");
    backend
        .execute_sql(
            "CREATE TABLE main.enrichment_recompute (event_id VARCHAR, event_date DATE, tier VARCHAR)",
        )
        .await
        .expect("create recompute source");

    let region = smelt_logical::maintenance::emit::Region {
        start: "'2026-07-01'".to_string(),
        end: "'2026-07-02'".to_string(),
    };
    let body = "SELECT event_id, event_date, tier FROM main.enrichment_recompute";
    let closure = smelt_logical::maintenance::SkeletonSourceClosure::Open {
        reason: "test".to_string(),
    };

    smelt_runtime::maintenance_driver::execute_delete_insert_with_delta_restriction(
        &backend,
        "main",
        "enriched",
        "event_date",
        &region,
        body,
        Some("event_id"),
        Some(&closure),
        "silver.fact",
        "2026-07-01",
        "2026-07-02",
        smelt_logical::maintenance::emit::MaintenanceDialect::DuckDb,
    )
    .await
    .expect("unrestricted recompute must succeed");

    let groups = backend.recorded_groups();
    let delete_insert_groups: Vec<_> = groups
        .iter()
        .filter(|g| g.statements[0].sql.starts_with("DELETE FROM main.enriched"))
        .collect();
    assert_eq!(delete_insert_groups.len(), 1, "{groups:?}");
    let group = delete_insert_groups[0];

    let expected = smelt_logical::maintenance::emit::emit_delete_insert(
        "main.enriched",
        "event_date",
        &region,
        body,
        smelt_logical::maintenance::emit::MaintenanceDialect::DuckDb,
    );
    assert_eq!(
        group.statements, expected.statements,
        "an Open closure must execute the byte-identical unrestricted emitter output"
    );
}

// =============================================================================
// Structural gate: no maintenance-statement authoring outside the emitter
// (`docs/specs/incremental_models.md` §"Statement emission (single owner)";
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
/// predates `incremental_models.md`'s single-owner emitters entirely and
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
            || line.contains("CREATE TABLE {}.{} AS")
            // The staged-candidate conditional DELETE+INSERT's temp
            // relation (T2, `docs/plans/20260715-composed-axes-conditional-
            // maintenance.md` Phase C5) — a distinctive shape with no
            // pre-existing production match, unlike a bare `DROP TABLE `
            // (which the generic table-lifecycle helpers already construct
            // legitimately, outside any maintenance-statement family — see
            // `Backend::drop_table_if_exists`'s own implementations).
            || line.contains("CREATE TEMP TABLE ");
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
/// (`docs/specs/incremental_models.md` §"Statement emission (single owner)").
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
         (docs/specs/incremental_models.md §\"Statement emission (single owner)\") — backends must \
         execute an emitted StatementGroup, never author their own SQL text:\n{}",
        hits.iter()
            .map(|h| format!("  {}:{}: {}", h.file.display(), h.line_no, h.text))
            .collect::<Vec<_>>()
            .join("\n")
    );
}
