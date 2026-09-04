//! Statement-parity CI gate (`docs/specs/architecture.md` §"Constraints &
//! Invariants" item 12; `docs/specs/incremental_models.md` §"Statement
//! emission (single owner)"): the SQL text a run actually executes must be
//! byte-identical to the single-owner emitters' output. This file proves
//! it by capturing the real statements sent to a live DuckDB connection
//! during a real `execute_project` run and diffing them against a direct
//! call of the emitter with the batch's own inputs.
//!
//! Covers the region `DELETE`+`INSERT` family (`IncrementalStrategy::
//! DeleteInsert`), the keyed-fold family (`refresh: keyed`), the
//! column-scoped `MERGE` family (`Technique::ColumnScopedMerge`) —
//! `docs/plans/20260710-emit-unification.md` Phases 1–3 — and the repair
//! family's per-group recompute (`Technique::PerGroupRecompute`,
//! `docs/specs/incremental_models.md` §"The repair family").
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
//! Also covers the backbuild emitter family
//! (`crates/smelt-logical/src/backbuild/emit.rs`) — B1 in-place backfill,
//! the model-level `FullRefresh` baseline, and B3 upstream backfill — driven
//! directly through `smelt_runtime::definition_delta::{derive_plan,
//! apply_migration}` rather than `execute_project`, since backbuild's
//! dispatch point is a migration plan a caller applies explicitly, not a
//! step of an ordinary run.
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
use smelt_logical::backbuild::emit::{
    emit_alter_add_column, emit_column_backfill_update_from, emit_full_refresh,
    emit_in_place_update,
};
use smelt_logical::backbuild::{MigrationPlan, MigrationVerdict, Technique};
use smelt_logical::maintenance::emit::{
    emit_append_only_posture_probe, emit_column_scoped_merge, emit_create_table_as,
    emit_delete_insert, emit_departed_key_delete, emit_diff_patch, emit_keyed_fold,
    emit_keyed_fold_suppressed, emit_per_group_recompute, emit_recurrence_bound_probe,
    emit_source_mutation_fingerprint, emit_staged_candidate_conditional_recompute,
    MaintenanceDialect, Region, TargetSlicePredicate,
};
use smelt_logical::maintenance::locality::LocalitySlice;
use smelt_planner::{
    AggregatorColumn, CrossPartitionCombiner, CumulativeClassification, DrivingSource,
};
use smelt_runtime::definition_delta::{apply_migration, derive_plan};
use smelt_runtime::execute::{execute_project, BackendFactory, BackendFuture};
use smelt_runtime::maintenance_driver::{
    driving_steps, run_windowed_keyed_maintenance, RestrictionDeltaSource,
};
use smelt_runtime::types::ExecuteRequest;
use tokio_util::sync::CancellationToken;

/// A retry policy that never retries — the maintenance-driver call sites
/// this suite exercises directly (outside `execute_project`) have no
/// `ExecuteRequest`/run reporter to derive one from
/// (`docs/plans/20260719-prod-w2-operability.md` Phase 6).
const NO_OP_REPORTER: smelt_runtime::NoOpReporter = smelt_runtime::NoOpReporter;
fn no_retry_policy() -> smelt_runtime::RetryPolicy<'static> {
    smelt_runtime::RetryPolicy {
        retry_max: 0,
        base_backoff_ms: 0,
        run_id: "statement-parity-test",
        model_name: "statement-parity-test",
        reporter: &NO_OP_REPORTER,
    }
}

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

/// Recover the `affected_keys_select` sub-`SELECT`
/// [`smelt_runtime::maintenance_driver::repair_candidate_select`] embeds
/// verbatim inside its own `EXISTS (SELECT 1 FROM (<here>) AS
/// __smelt_repair_keys WHERE ...)` wrapper — the repair-family parity tests
/// below use this to recover the LIVE run's own discovered affected-key
/// relation rather than independently rebuilding it: for a
/// `MutationProfile::MutableSnapshot` source (P9,
/// `docs/specs/incremental_models.md` §"The repair family") the relation is
/// a backend-state-dependent `VALUES (...)` literal a test cannot
/// reconstruct without duplicating the live sidecar-diff read, so this
/// instead proves internal consistency: the SAME relation text
/// `repair_candidate_select` embedded is the one `emit_per_group_recompute`/
/// `emit_diff_patch` joins against.
fn extract_affected_keys_select(candidate_select: &str) -> String {
    let marker = "WHERE EXISTS (SELECT 1 FROM (";
    let start = candidate_select
        .find(marker)
        .expect("candidate_select must embed an EXISTS-wrapped affected-keys read")
        + marker.len();
    let suffix = ") AS __smelt_repair_keys WHERE ";
    let end = candidate_select[start..]
        .rfind(suffix)
        .expect("candidate_select must close the affected-keys read with __smelt_repair_keys")
        + start;
    candidate_select[start..end].to_string()
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
        jobs: None,
        retry_max: None,
        retry_backoff_ms: None,
        resume: false,
        technique_overrides: vec![],
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

/// State residency (`docs/outcomes/20260904-state-residency/outcome.md`
/// criterion 1): each DuckDB DeleteInsert batch's own reconciliation-ledger
/// reset — the idempotent `_smelt_ledger` DDL plus this batch's own
/// `[start, end)` region-recompute reset — must be sent to the connection
/// as raw SQL byte-identical to `generate_ledger_table_ddl`/
/// `generate_ledger_recompute_reset_sqls`'s own output, and must never
/// appear inside the write's own `StatementGroup` (bookkeeping never leaks
/// into the emitted write, `docs/specs/incremental_models.md` §"Statement
/// emission (single owner)").
#[tokio::test]
async fn ledger_recompute_reset_statements_come_from_the_state_builder() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project_dir = tmp.path();
    std::fs::create_dir_all(project_dir.join("models")).unwrap();

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
        "name: ledger_reset_test\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: {db}\n    schema: main\ndefault_materialization: table\ntarget: dev\n",
        db = db_path.display()
    );
    std::fs::write(project_dir.join("smelt.yml"), &smelt_yml).unwrap();

    let config = Arc::new(Config::load(project_dir).expect("load config"));

    // Run 1: first-run create — no ledger reset yet (the target doesn't
    // exist, so this run never reaches the DeleteInsert branch).
    {
        let (db, graph) = build_db_and_graph(project_dir, &config);
        let backend_slot: Arc<Mutex<Option<Arc<RecordingBackend>>>> = Arc::new(Mutex::new(None));
        let factory = RecordingBackendFactory {
            db_path: db_path.clone(),
            backend: Arc::clone(&backend_slot),
        };
        execute_project(
            "ledger-reset-run-1".to_string(),
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
        .expect("run 1 (first-run create)");
    }

    // Run 2: the table exists — two daily batches dispatch `IncrementalStrategy::
    // DeleteInsert`, each recording its own ledger reset.
    let (db, graph) = build_db_and_graph(project_dir, &config);
    let backend_slot: Arc<Mutex<Option<Arc<RecordingBackend>>>> = Arc::new(Mutex::new(None));
    let factory = RecordingBackendFactory {
        db_path: db_path.clone(),
        backend: Arc::clone(&backend_slot),
    };
    execute_project(
        "ledger-reset-run-2".to_string(),
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
    .expect("run 2 (incremental)");

    let backend = backend_slot
        .lock()
        .unwrap()
        .clone()
        .expect("backend recorded");
    let sql_log = backend.recorded_sql();

    let ensure_ddl = smelt_state::ddl_duckdb::generate_ledger_table_ddl("main");
    assert!(
        sql_log.iter().any(|s| s == &ensure_ddl),
        "the ledger's idempotent ensure DDL must be sent as raw SQL byte-identical to \
         `generate_ledger_table_ddl`: {sql_log:?}"
    );

    // No `batch_size_days` is set, so the whole `[start, end)` request range
    // runs as a single batch, not one batch per day.
    let expected_reset = smelt_state::ddl_duckdb::generate_ledger_recompute_reset_sqls(
        "main",
        "daily_events",
        "{*}",
        "2024-01-01",
        "2024-01-03",
        "self",
        "2024-01-03",
    );
    for stmt in &expected_reset {
        assert!(
            sql_log.contains(stmt),
            "the batch must record its ledger reset statement byte-identical to \
             `generate_ledger_recompute_reset_sqls`: {stmt}\nrecorded: {sql_log:?}"
        );
    }

    // Bookkeeping never leaks into the emitted write's own StatementGroup.
    let groups = backend.recorded_groups();
    for group in &groups {
        for stmt in &group.statements {
            assert!(
                !stmt.sql.contains("_smelt_ledger"),
                "ledger bookkeeping must never appear inside a maintenance StatementGroup: {}",
                stmt.sql
            );
        }
    }
}

/// First-run bootstrap for a **self-referential** partition-grain model
/// (`docs/specs/incremental_shapes.md` §"First-run and backfill" — "First-run
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
         maintenance:\n\
         \x20\x20scan_bounds:\n\
         \x20\x20\x20\x20per_source:\n\
         \x20\x20\x20\x20\x20\x20transactions:\n\
         \x20\x20\x20\x20\x20\x20\x20\x20allow_full_scan: true\n\
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

/// A `write: staged_candidate` pin (`docs/outcomes/
/// 20260815-definition-delta-migrate/phases/27g-plan.md`) on a `refresh:
/// keyed` model's driving-source cell must dispatch the merge-less
/// staged-candidate mechanism at run time instead of the ordinary `MERGE` —
/// even on a `MERGE`-capable backend (DuckDB), since an explicit pin is
/// never second-guessed. Same fixture as
/// `keyed_fold_statements_come_from_the_emitter` above, with one added
/// `maintenance.cells[]` pin.
#[tokio::test]
async fn staged_candidate_keyed_fold_statements_come_from_the_emitter() {
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
         maintenance:\n\
         \x20\x20cells:\n\
         \x20\x20\x20\x20- on: smelt.events\n\
         \x20\x20\x20\x20\x20\x20columns: []\n\
         \x20\x20\x20\x20\x20\x20write: staged_candidate\n\
         ---\n\
         SELECT device_id, MIN(event_ts) AS first_seen, MAX(event_ts) AS last_seen \
         FROM smelt.events GROUP BY device_id",
    );

    let db_path = project_dir.join("run.duckdb");
    let smelt_yml = format!(
        "name: staged_candidate_keyed_statement_parity_test\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: {db}\n    schema: main\ndefault_materialization: table\ntarget: dev\n",
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

    let request = make_request("dev", "2024-01-01", "2024-01-03");
    let outcome = execute_project(
        "staged-candidate-keyed-statement-parity-run".to_string(),
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
    .expect("execute_project (keyed, staged_candidate pin)");

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

    // Step 1: unaffected by the pin — no target table yet, so the driver
    // still takes the plain create branch.
    assert_eq!(groups[0].statements.len(), 1);
    assert!(groups[0].statements[0]
        .sql
        .starts_with("CREATE TABLE main.device_user_edges AS "));

    // Step 2: the pin selects the merge-less staged-candidate group — five
    // statements, transactional as one unit — never the MERGE.
    let group = &groups[1];
    assert_eq!(
        group.statements.len(),
        5,
        "staged-candidate pin must yield a 5-statement group: {:?}",
        group
    );
    assert!(group.transactional);
    assert!(group.statements[0].sql.starts_with("CREATE TEMP TABLE"));
    assert!(!group.statements.iter().any(|s| s.sql.contains("MERGE")));

    let insert_candidates_sql = &group.statements[1].sql;
    let candidate_select = insert_candidates_sql
        .strip_prefix("INSERT INTO __smelt_staged_device_user_edges ")
        .expect("insert-candidates shape");

    let folds = vec![
        (
            "first_seen".to_string(),
            "LEAST(target.first_seen, delta.first_seen)".to_string(),
        ),
        (
            "last_seen".to_string(),
            "GREATEST(target.last_seen, delta.last_seen)".to_string(),
        ),
    ];
    // Recover the step's own compiled delta SELECT from the candidate
    // SELECT's own `FROM (<delta_sql>) AS delta LEFT JOIN` shape (templated
    // with a placeholder so the surrounding prefix/suffix are derived from
    // the single-owner emitter itself, never hand-duplicated), then rebuild
    // the exact group a direct emitter call over that delta produces.
    let placeholder = "__PLACEHOLDER_DELTA__";
    let templated = smelt_logical::maintenance::emit::keyed_fold_candidate_select(
        "main.device_user_edges",
        &["device_id".to_string()],
        &folds,
        placeholder,
        MaintenanceDialect::DuckDb,
    );
    let (prefix, suffix) = templated.split_once(placeholder).unwrap();
    let actual_delta_sql = candidate_select
        .strip_prefix(prefix)
        .and_then(|s| s.strip_suffix(suffix))
        .expect("candidate_select must match keyed_fold_candidate_select's own shape");

    let expected_candidate_select = smelt_logical::maintenance::emit::keyed_fold_candidate_select(
        "main.device_user_edges",
        &["device_id".to_string()],
        &folds,
        actual_delta_sql,
        MaintenanceDialect::DuckDb,
    );
    assert_eq!(
        candidate_select, expected_candidate_select,
        "the executed candidate SELECT must be byte-identical to a direct \
         keyed_fold_candidate_select call over the step's own delta"
    );
    let expected_group = smelt_logical::maintenance::emit::emit_staged_candidate_conditional(
        "main.device_user_edges",
        "__smelt_staged_device_user_edges",
        &["device_id".to_string()],
        &expected_candidate_select,
        &["first_seen".to_string(), "last_seen".to_string()],
        MaintenanceDialect::DuckDb,
    );
    assert_eq!(
        group, &expected_group,
        "executed staged-candidate group must be byte-identical to a direct emitter call"
    );

    // Result-equivalence: the staged-candidate write path must still
    // reproduce a full refresh of the model's own aggregation.
    assert!(
        multiset_equal(
            backend.as_ref(),
            "SELECT * FROM main.device_user_edges",
            "SELECT device_id, MIN(event_ts) AS first_seen, MAX(event_ts) AS last_seen \
             FROM main.events GROUP BY device_id"
        )
        .await,
        "the staged-candidate statements execute_project actually ran must reproduce a full \
         refresh"
    );
}

/// A `write: keyed`/`keyed_conditional` pin on a backend that cannot run
/// `MERGE` at all must refuse the run before any write — the pin selects
/// the `MERGE` mechanism explicitly, so the driver must never silently
/// substitute the merge-less staged-candidate mechanism instead (`docs/
/// outcomes/20260815-definition-delta-migrate/phases/27g-plan.md`).
#[tokio::test]
async fn keyed_pin_on_a_merge_less_backend_refuses_before_any_write() {
    use smelt_logical::maintenance::choice::WriteSuppression;

    struct MergeLessBackend {
        calls: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl Backend for MergeLessBackend {
        async fn execute_sql(&self, sql: &str) -> Result<Vec<RecordBatch>, BackendError> {
            self.calls.lock().unwrap().push(sql.to_string());
            Ok(vec![])
        }
        async fn create_table_as(
            &self,
            _schema: &str,
            _name: &str,
            sql: &str,
        ) -> Result<(), BackendError> {
            self.calls.lock().unwrap().push(sql.to_string());
            Ok(())
        }
        async fn create_view_as(
            &self,
            _schema: &str,
            _name: &str,
            _sql: &str,
        ) -> Result<(), BackendError> {
            unreachable!()
        }
        async fn drop_table_if_exists(
            &self,
            _schema: &str,
            _name: &str,
        ) -> Result<(), BackendError> {
            Ok(())
        }
        async fn drop_view_if_exists(
            &self,
            _schema: &str,
            _name: &str,
        ) -> Result<(), BackendError> {
            Ok(())
        }
        async fn get_row_count(&self, _schema: &str, _name: &str) -> Result<usize, BackendError> {
            Ok(0)
        }
        async fn get_preview(
            &self,
            _schema: &str,
            _name: &str,
            _limit: usize,
        ) -> Result<Vec<RecordBatch>, BackendError> {
            Ok(vec![])
        }
        async fn table_exists(&self, _schema: &str, _name: &str) -> Result<bool, BackendError> {
            // Existing target — reaches the merge/write-mechanism branch,
            // not the first-run create.
            Ok(true)
        }
        async fn ensure_schema(&self, _schema: &str) -> Result<(), BackendError> {
            Ok(())
        }
        fn dialect(&self) -> SqlDialect {
            SqlDialect::SparkSQL
        }
        fn capabilities(&self) -> BackendCapabilities {
            BackendCapabilities {
                supports_merge: false,
                ..BackendCapabilities::spark()
            }
        }
        async fn load_table(
            &self,
            _schema: &str,
            _name: &str,
            _arrow_schema: SchemaRef,
            _batches: Vec<RecordBatch>,
        ) -> Result<(), BackendError> {
            unreachable!()
        }
        async fn delete_partitions(
            &self,
            _schema: &str,
            _name: &str,
            _partitions: &PartitionRange,
        ) -> Result<(), BackendError> {
            unreachable!()
        }
        async fn insert_into_from_query(
            &self,
            _schema: &str,
            _name: &str,
            _sql: &str,
        ) -> Result<(), BackendError> {
            unreachable!()
        }
        async fn insert_overwrite(
            &self,
            _schema: &str,
            _table: &str,
            _sql: &str,
            _partition: &PartitionRange,
        ) -> Result<(), BackendError> {
            unreachable!()
        }
    }

    let backend = MergeLessBackend {
        calls: Mutex::new(Vec::new()),
    };
    let classification = CumulativeClassification {
        unique_key: vec!["device_id".to_string()],
        aggregator_columns: vec![AggregatorColumn {
            output_name: "event_count".to_string(),
            per_partition_agg: "COUNT".to_string(),
            cross_partition_combiner: CrossPartitionCombiner::Sum,
            state: None,
        }],
        driving_source: DrivingSource {
            name: "smelt.events".to_string(),
            timeseries: None,
        },
    };
    let steps = driving_steps(
        "2024-01-01",
        "2024-01-02",
        &smelt_core::config::Granularity::Day,
    )
    .expect("steps");
    let suppression = WriteSuppression::Unconditional {
        why: "test exercises the keyed pin refusal, not suppression".to_string(),
    };
    let pin = smelt_logical::maintenance::lookup_write_pattern("keyed").expect("registered");

    let result = run_windowed_keyed_maintenance(
        &backend,
        "device_daily",
        "main",
        "device_daily",
        &steps,
        &classification,
        None,
        &suppression,
        Some(pin),
        |step| {
            Ok(format!(
                "SELECT device_id, COUNT(*) AS event_count FROM events WHERE d = '{}' GROUP BY \
                 device_id",
                step.partition_value
            ))
        },
        &no_retry_policy(),
        &smelt_runtime::probes::ProbePolicy::per_run(),
    )
    .await;

    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("device_daily"),
        "error must name the model: {err}"
    );
    assert!(err.contains("keyed"), "error must name the pin: {err}");
    assert!(
        backend.calls.lock().unwrap().is_empty(),
        "no write statement must be issued once the pin refuses: {:?}",
        backend.calls.lock().unwrap()
    );
}

/// The `smelt explain` `KeyedFold` preview for a state-bearing model (`AVG`,
/// `docs/outcomes/20260809-rung2-state-shapes` row 7) must carry the same
/// state-column folds as the executed `MERGE` — both now go through the
/// same single-owner `expand_aggregator_column_folds`
/// (`smelt_logical::maintenance::emit`, row 7's "single-owner statement
/// rule" move) and the same pre-compile `state_augmented_projection` step,
/// so they can never diverge.
#[tokio::test]
async fn keyed_fold_preview_matches_executed_statement_for_state_bearing_model() {
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
         (DATE '2024-01-01', 1, 10.0), \
         (DATE '2024-01-02', 1, 20.0), \
         (DATE '2024-01-02', 2, 30.0)) \
         AS t(event_date, device_id, amount)",
    );
    write_model(
        project_dir,
        "device_avg_amount",
        "---\n\
         materialization: table\n\
         refresh: incremental\n\
         grain: key\n\
         ---\n\
         SELECT device_id, AVG(amount) AS avg_amount \
         FROM smelt.events GROUP BY device_id",
    );

    let db_path = project_dir.join("run.duckdb");
    let smelt_yml = format!(
        "name: keyed_avg_statement_parity_test\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: {db}\n    schema: main\ndefault_materialization: table\ntarget: dev\n",
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
    // the MERGE arm — the one this test inspects.
    let request = make_request("dev", "2024-01-01", "2024-01-03");
    let outcome = execute_project(
        "keyed-avg-statement-parity-run".to_string(),
        request,
        Arc::clone(&config),
        Arc::clone(&graph),
        Arc::clone(&db),
        project_dir,
        &factory,
        &smelt_runtime::NoOpReporter,
        CancellationToken::new(),
    )
    .await
    .expect("execute_project (keyed, state-bearing)");
    assert!(
        outcome.models.contains_key("device_avg_amount"),
        "device_avg_amount must have run: {:?}",
        outcome.models.keys().collect::<Vec<_>>()
    );

    let backend = backend_slot
        .lock()
        .unwrap()
        .clone()
        .expect("backend recorded");
    // `AVG`'s hidden state is the first *additive* state this mechanism
    // admits (`docs/outcomes/20260809-rung2-state-shapes` row 7), so the
    // cell now grades `Grade::Additive` and routes through the
    // reconciliation-ledger path (`maintenance_driver::run_windowed_keyed_
    // maintenance`'s ledger-interleaved arm) — its statements go through
    // `Backend::execute_sql` directly, not `execute_statement_group`, so
    // this test reads `recorded_sql`, not `recorded_groups` (unlike the
    // `Idempotent`-graded `MIN`/`MAX` cells `keyed_fold_statements_come_
    // from_the_emitter` above inspects).
    let sql_log = backend.recorded_sql();
    let executed_merge_sql = sql_log
        .iter()
        .find(|sql| sql.starts_with("MERGE INTO main.device_avg_amount"))
        .cloned()
        .unwrap_or_else(|| panic!("no executed MERGE statement found: {sql_log:?}"));
    assert!(
        executed_merge_sql
            .contains("avg_amount__sum = target.avg_amount__sum + delta.avg_amount__sum")
            && executed_merge_sql
                .contains("avg_amount__count = target.avg_amount__count + delta.avg_amount__count"),
        "expected the executed MERGE to fold the hidden sum/count state additively: \
         {executed_merge_sql}"
    );

    // Now build the `smelt explain` `KeyedFold` preview for the same model
    // and assert it carries the identical state-column fold expressions.
    let sql_models =
        smelt_core::ModelDiscovery::new(project_dir.to_path_buf(), config.paths.clone())
            .discover_models()
            .expect("discover_models");
    let model = sql_models
        .iter()
        .find(|m| m.canonical_path() == "device_avg_amount")
        .expect("device_avg_amount model discovered");
    let metadata = model
        .metadata
        .as_deref()
        .expect("device_avg_amount declares frontmatter");
    let stripped_sql = smelt_parser::strip_frontmatter(&model.content).to_string();
    let sources = vec![smelt_logical::maintenance::SourceFacts {
        name: "events".to_string(),
        mutation: smelt_logical::maintenance::MutationProfile::AppendOnly,
        partition_col: Some("event_date".to_string()),
        unique_key: vec![],
        allow_full_scan: true,
    }];
    let plan_result = smelt_db::queries::maintenance::derive_model_maintenance_plan(
        &stripped_sql,
        "device_avg_amount",
        metadata,
        &sources,
        &std::collections::HashSet::new(),
        None,
        &[],
        &[],
        &std::collections::BTreeMap::new(),
        None,
        None,
    )
    .expect("device_avg_amount must derive a maintenance plan");
    let cell = plan_result
        .plan
        .cells
        .iter()
        .find(|c| c.technique == smelt_logical::maintenance::Technique::KeyedFold)
        .expect("device_avg_amount must admit a KeyedFold cell");

    let registry = smelt_runtime::CompilerRegistry::new(&config, &config.targets);
    let resolver = registry
        .get("dev")
        .build_ephemeral_resolver(&[], "main")
        .expect("no ephemerals");
    let graph_locked = graph.lock().await;
    let source_timeseries = smelt_runtime::build_source_timeseries_map(&graph_locked, &[]);
    drop(graph_locked);

    let plan_cell_diagnostics = smelt_runtime::diagnostics::build_plan_cell_diagnostics(
        cell,
        model,
        "main",
        "dev",
        &registry,
        &resolver,
        MaintenanceDialect::DuckDb,
        &[],
        &source_timeseries,
        &plan_result.column_groups,
    );
    let preview = plan_cell_diagnostics
        .technique_previews
        .iter()
        .find(|p| p.technique == smelt_logical::maintenance::Technique::KeyedFold)
        .expect("a KeyedFold preview must always be present");
    let preview_sql = preview
        .statements
        .first()
        .expect("the KeyedFold preview must render a statement")
        .sql
        .clone();

    for fragment in [
        "avg_amount__sum = target.avg_amount__sum + delta.avg_amount__sum",
        "avg_amount__count = target.avg_amount__count + delta.avg_amount__count",
        "avg_amount = (target.avg_amount__sum + delta.avg_amount__sum) / \
         (target.avg_amount__count + delta.avg_amount__count)",
    ] {
        assert!(
            preview_sql.contains(fragment) && executed_merge_sql.contains(fragment),
            "preview and executed statement must carry the identical state-column fold \
             `{fragment}` — preview: {preview_sql}\nexecuted: {executed_merge_sql}"
        );
    }

    // Phase 27a (`docs/outcomes/20260815-definition-delta-migrate/phases/
    // 27a-plan.md`): the preview's own change-suppressed matched-arm guard
    // must be byte-identical to what the live run actually executed — never
    // a preview that renders the unconditional arm while the live run
    // suppressed, or vice versa. `avg_amount` is a plain `AVG` (registry-
    // backed, P3 `Comparable`) over a proven `Key` row identity, so both
    // resolve `WriteSuppression::Suppressed` here.
    let guard = "target.avg_amount IS DISTINCT FROM \
                 ((target.avg_amount__sum + delta.avg_amount__sum) / \
                 (target.avg_amount__count + delta.avg_amount__count))";
    assert!(
        preview_sql.contains(guard) && executed_merge_sql.contains(guard),
        "preview and executed statement must carry the identical change-suppressed guard — \
         preview: {preview_sql}\nexecuted: {executed_merge_sql}"
    );
}

/// The slice-predicated keyed-fold family: a `refresh: keyed` model that
/// also declares its own `timeseries:` block, admitted through key temporal
/// locality's route 1 (key-embedded — `partition_column` is itself a
/// `unique_key` column, `docs/specs/incremental_shapes.md` §"Key temporal
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
         maintenance:\n\
         \x20\x20scan_bounds:\n\
         \x20\x20\x20\x20per_source:\n\
         \x20\x20\x20\x20\x20\x20events:\n\
         \x20\x20\x20\x20\x20\x20\x20\x20allow_full_scan: true\n\
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
/// declared `r`) merge (`docs/specs/incremental_shapes.md` §"Key temporal
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
            state: None,
        }],
        driving_source: DrivingSource {
            name: "smelt.sources.raw.events".to_string(),
            timeseries: Some(smelt_core::config::TimeseriesConfig {
                event_time_column: "event_ts".to_string(),
                partition_column: "event_date".to_string(),
                granularity: smelt_core::config::Granularity::Day,
                week_start: None,
                assert_monotonic: false,
            }),
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
        None,
        compile_step,
        &no_retry_policy(),
        &smelt_runtime::probes::ProbePolicy::per_run(),
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
        None,
        compile_step,
        &no_retry_policy(),
        &smelt_runtime::probes::ProbePolicy::per_run(),
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
        MaintenanceDialect::DuckDb,
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
        jobs: None,
        retry_max: None,
        retry_backoff_ms: None,
        resume: false,
        technique_overrides: vec![],
    }
}

/// The column-scoped `MERGE` family (`Technique::ColumnScopedMerge`, MP11).
///
/// **Reachability note** (`docs/plans/20260808-membership-sensitivity.md`
/// Phase 2): before that plan, `examples/timeseries/daily_events_enriched`'s
/// `raw.users` mutation drove the `{user_name}` cell's live column-scoped
/// MERGE, and this test drove it end to end through `execute_project`. Phase
/// 1 of that plan derives membership sensitivity directly from the join's
/// `ON e.user_id = u.user_id` predicate (a row-admission read), which makes
/// `{user_name}` — and every other column group that same join admits —
/// membership-sensitive, so the cell now admits `Technique::DeleteInsert`,
/// never `ColumnScopedMerge`
/// (`technique_lowering.rs::real_fixture_examples_timeseries_admits_
/// membership_recompute_cell` proves the derivation). No fixture in this
/// workspace reaches `ColumnScopedMerge` today: value sensitivity alone,
/// without any row-admission read of the SAME mutable source, has no
/// currently-shipped shape (every `mutation_profile: mutable_snapshot`
/// dimension example workspaces ship is also the driving join's own
/// partner). `ColumnScopedMerge`'s emitter parity is therefore proven the
/// same way the family's OTHER legs in this file prove theirs when no real
/// fixture reaches them — a direct call of the single production dispatch
/// function ([`execute_column_scoped_merge_full`]) against a `RecordingBackend`,
/// asserting the executed `MERGE` is byte-identical to a direct
/// `emit_column_scoped_merge` call over the same inputs. Tracked as a real
/// reachability gap, not silently worked around: `docs/plans/
/// 20260808-membership-sensitivity.md`'s Deferred section and
/// `incremental_models.md` §Known Divergences (Phase 4 of that plan).
#[tokio::test]
async fn column_scoped_merge_statements_come_from_the_emitter() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("test.duckdb");
    let inner = DuckDbBackend::new(&db_path, "main")
        .await
        .expect("open duckdb");
    let backend = RecordingBackend::new(inner);

    backend
        .execute_sql(
            "CREATE TABLE main.daily_events_enriched (event_id INTEGER, user_id INTEGER, \
             user_name VARCHAR)",
        )
        .await
        .expect("create target table");
    backend
        .execute_sql("INSERT INTO main.daily_events_enriched VALUES (1, 1, 'Alice'), (2, 2, 'Bob')")
        .await
        .expect("seed target table");
    backend
        .execute_sql(
            "CREATE TABLE main.sources_raw_users (event_id INTEGER, user_id INTEGER, user_name \
             VARCHAR)",
        )
        .await
        .expect("create dim/source table");
    backend
        .execute_sql("INSERT INTO main.sources_raw_users VALUES (1, 1, 'Alicia'), (2, 2, 'Bob')")
        .await
        .expect("seed source table (user 1 mutated)");

    let dimension_batch_sql = "SELECT event_id, user_id, user_name FROM main.sources_raw_users";
    let suppression = smelt_logical::maintenance::choice::WriteSuppression::Unconditional {
        why: "unit-level parity probe — the family's Unconditional variant is exercised, the \
              Suppressed one by `suppressed_column_scoped_merge_statements_come_from_the_emitter`"
            .to_string(),
    };
    let window = smelt_backend::PartitionRange {
        column: String::new(),
        start: "2025-01-10".to_string(),
        end: "2025-01-11".to_string(),
        axis: smelt_backend::PartitionAxis::Calendar,
    };
    smelt_runtime::maintenance_driver::execute_column_scoped_merge_full(
        &backend,
        "main",
        "daily_events_enriched",
        &["event_id".to_string()],
        dimension_batch_sql,
        &[],
        &suppression,
        &window,
        &no_retry_policy(),
    )
    .await
    .expect("column-scoped merge must succeed");

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

    let expected = emit_column_scoped_merge(
        "main.daily_events_enriched",
        &["event_id".to_string()],
        dimension_batch_sql,
        &[],
        MaintenanceDialect::DuckDb,
    );
    assert_eq!(
        &expected, group,
        "executed MERGE group must be byte-identical to a direct emitter call over the same inputs"
    );

    // Result-equivalence: the column-scoped MERGE actually executed must
    // leave the target multiset-equal to a full refresh of the source.
    assert!(
        multiset_equal(
            &backend,
            "SELECT * FROM main.daily_events_enriched",
            "SELECT event_id, user_id, user_name FROM main.sources_raw_users"
        )
        .await,
        "the column-scoped MERGE actually executed must reproduce a full refresh"
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
        axis: smelt_backend::PartitionAxis::Calendar,
    };
    smelt_runtime::maintenance_driver::execute_column_scoped_merge_full(
        &backend,
        "main",
        "dim_users",
        &["user_id".to_string()],
        dimension_batch_sql,
        &[],
        &suppression,
        &window,
        &no_retry_policy(),
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
        &[],
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

/// The keyed membership-recompute family
/// (`docs/plans/20260808-membership-sensitivity.md` Phase 2): drives
/// `examples/timeseries` with an added `grain: key` model (mirrors
/// `technique_lowering.rs::keyed_membership_recompute_e2e`'s fixture — a
/// `COUNT`-folded fact inner-joined to a `mutation_profile: mutable_snapshot`
/// dimension purely for row admission) through `execute_project` twice: a
/// creation run, then a dimension mutation that makes the `{event_count}`
/// cell's `Trigger::UpstreamMutation` live. Asserts the executed staged-
/// candidate `DELETE`+`INSERT` group is byte-identical to a direct
/// `emit_staged_candidate_conditional` call over the same table/key/
/// candidate-select/compared-columns.
#[tokio::test]
async fn delete_insert_suppressed_keyed_membership_statements_come_from_the_emitter() {
    const MODEL_SQL: &str = "SELECT t.user_id AS user_id, COUNT(t.transaction_id) AS \
         event_count FROM smelt.sources.raw.transactions t \
         JOIN smelt.sources.raw.users u ON t.user_id = u.user_id \
         GROUP BY t.user_id";
    const MODEL_FILE: &str = "---\n\
         materialization: table\n\
         refresh: incremental\n\
         grain: key\n\
         unique_key: user_id\n\
         maintenance:\n  \
           scan_bounds:\n    \
             per_source:\n      \
               raw.users:\n        \
                 allow_full_scan: true\n      \
               raw.transactions:\n        \
                 allow_full_scan: true\n\
         ---\n";

    let source_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/timeseries")
        .canonicalize()
        .expect("examples/timeseries exists");

    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().join("project");
    copy_dir_recursive(&source_dir, &project_dir);
    std::fs::write(
        project_dir.join("models/user_lifetime_status.sql"),
        format!("{MODEL_FILE}{MODEL_SQL}\n"),
    )
    .expect("write keyed model fixture");

    let db_path = tmp.path().join("run.duckdb");
    let config = Arc::new(Config::load(&project_dir).expect("load smelt.yml"));

    {
        let backend = DuckDbBackend::new(&db_path, "main")
            .await
            .expect("open duckdb");
        backend
            .execute_sql(
                "CREATE TABLE main.sources_raw_transactions (transaction_id INTEGER, user_id \
                 INTEGER, amount DECIMAL(10,2), transaction_timestamp TIMESTAMP, \
                 transaction_type VARCHAR)",
            )
            .await
            .expect("create transactions source table");
        backend
            .execute_sql(
                "INSERT INTO main.sources_raw_transactions VALUES \
                 (1, 1, 10.00, TIMESTAMP '2025-01-10 08:00:00', 'purchase'), \
                 (2, 2, 20.00, TIMESTAMP '2025-01-10 09:00:00', 'purchase')",
            )
            .await
            .expect("seed transactions");
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

    // Run 1: creation — never the membership-recompute path.
    {
        let (db, graph) = build_db_and_graph(&project_dir, &config);
        execute_project(
            "keyed-membership-parity-run-1".to_string(),
            select_request("dev", "user_lifetime_status", "2025-01-10", "2025-01-11"),
            Arc::clone(&config),
            graph,
            db,
            &project_dir,
            &RecordingBackendFactory {
                db_path: db_path.clone(),
                backend: Arc::new(Mutex::new(None)),
            },
            &smelt_runtime::NoOpReporter,
            CancellationToken::new(),
        )
        .await
        .expect("first run (create) must succeed");
    }

    // Mutate the dimension in place, making the `{event_count}` cell live.
    {
        let backend = DuckDbBackend::new(&db_path, "main")
            .await
            .expect("reopen duckdb");
        backend
            .execute_sql("UPDATE main.sources_raw_users SET user_name = 'Alicia' WHERE user_id = 1")
            .await
            .expect("mutate dimension");
    }

    // Run 2: the dimension mutation dispatches the staged-candidate
    // membership recompute.
    let (db, graph) = build_db_and_graph(&project_dir, &config);
    let backend_slot: Arc<Mutex<Option<Arc<RecordingBackend>>>> = Arc::new(Mutex::new(None));
    let factory = RecordingBackendFactory {
        db_path: db_path.clone(),
        backend: Arc::clone(&backend_slot),
    };
    let outcome = execute_project(
        "keyed-membership-parity-run-2".to_string(),
        select_request("dev", "user_lifetime_status", "2025-01-11", "2025-01-12"),
        Arc::clone(&config),
        graph,
        db,
        &project_dir,
        &factory,
        &smelt_runtime::NoOpReporter,
        CancellationToken::new(),
    )
    .await
    .expect("second run (membership recompute) must succeed");

    let record = outcome
        .models
        .get("user_lifetime_status")
        .expect("user_lifetime_status ran");
    assert_eq!(
        record.strategy, "delete_insert_suppressed",
        "the dimension mutation must dispatch the staged-candidate membership-recompute \
         technique"
    );

    let backend = backend_slot
        .lock()
        .unwrap()
        .clone()
        .expect("backend recorded");
    let groups = backend.recorded_groups();
    let staged_groups: Vec<_> = groups
        .iter()
        .filter(|g| {
            g.statements
                .first()
                .is_some_and(|s| s.sql.starts_with("CREATE TEMP TABLE"))
        })
        .collect();
    assert_eq!(
        staged_groups.len(),
        1,
        "exactly one staged-candidate group must have executed: {:?}",
        groups
    );
    let group = staged_groups[0];
    assert!(
        group.transactional,
        "the staged-candidate group is transactional"
    );
    assert_eq!(group.statements.len(), 6);

    // Recover the caller-composed `candidate_select` from the recorded
    // INSERT statement (statement index 1: `INSERT INTO {staged} {select}`)
    // and the staged relation name from statement 0's `CREATE TEMP TABLE
    // {name} AS SELECT * FROM ({select}) AS __smelt_staged_shape LIMIT 0`.
    let insert_sql = &group.statements[1].sql;
    let staged_relation = "__smelt_staged_user_lifetime_status";
    let candidate_prefix = format!("INSERT INTO {staged_relation} ");
    assert!(
        insert_sql.starts_with(&candidate_prefix),
        "unexpected staged-candidate INSERT statement: {insert_sql}"
    );
    let candidate_select = &insert_sql[candidate_prefix.len()..];

    let expected = smelt_logical::maintenance::emit::emit_staged_candidate_conditional_recompute(
        "main.user_lifetime_status",
        staged_relation,
        &["user_id".to_string()],
        candidate_select,
        &["event_count".to_string()],
        MaintenanceDialect::DuckDb,
    );
    assert_eq!(
        &expected, group,
        "executed staged-candidate group must be byte-identical to a direct emitter call over \
         the same inputs (the full-recompute variant — this cell's candidate_select is always \
         the model's own full unwindowed recompute, so a departed key must be genuinely \
         deleted, not merely left untouched)"
    );

    // Result-equivalence: the staged-candidate recompute actually executed
    // must leave the target multiset-equal to a full refresh of the model.
    assert!(
        multiset_equal(
            backend.as_ref(),
            "SELECT user_id, event_count FROM main.user_lifetime_status",
            "SELECT t.user_id, COUNT(t.transaction_id) AS event_count FROM \
             main.sources_raw_transactions t JOIN main.sources_raw_users u ON t.user_id = \
             u.user_id GROUP BY t.user_id"
        )
        .await,
        "the staged-candidate recompute actually executed must reproduce a full refresh"
    );
}

/// The keyless (whole-row) realisation (`docs/outcomes/
/// 20260815-definition-delta-migrate/phases/27c-plan.md`): a `grain:
/// partition` output with no `unique_key` and no `GROUP BY` — `RowIdentity::
/// WholeRow` — joined to a `mutation_profile: mutable_snapshot` dimension
/// with no declared `unique_key`/`referential_integrity` of its own (so the
/// join is never closure-pruned, keeping the group genuinely membership-
/// sensitive) must dispatch `MembershipRecomputeWrite::StagedKeyless`, whose
/// executed statements are byte-identical to a direct
/// `emit_staged_candidate_conditional_keyless` call over the batch's own
/// inputs.
#[tokio::test]
async fn staged_candidate_keyless_statements_come_from_the_emitter() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().join("project");
    std::fs::create_dir_all(project_dir.join("models/sources")).expect("mkdir models/sources");
    std::fs::write(
        project_dir.join("smelt.yml"),
        "name: keyless_membership_parity\nversion: 1\npaths:\n  - models\n\
         targets:\n  dev:\n    type: duckdb\n    schema: main\n\
         default_materialization: table\ntarget: dev\n",
    )
    .expect("write smelt.yml");
    std::fs::write(
        project_dir.join("models/sources/facts.yml"),
        "description: facts\ncolumns:\n- name: fact_id\n  type: INTEGER\n\
         - name: dim_id\n  type: INTEGER\n- name: event_date\n  type: DATE\n\
         - name: amount\n  type: INTEGER\n\
         mutation_profile:\n  kind: append_only\n\
         timeseries:\n  partition_column: event_date\n  event_time_column: event_date\n  \
         granularity: day\n",
    )
    .expect("write facts source yml");
    std::fs::write(
        project_dir.join("models/sources/dim.yml"),
        "description: dim\ncolumns:\n- name: dim_id\n  type: INTEGER\n\
         - name: tag\n  type: VARCHAR\n\
         mutation_profile:\n  kind: mutable_snapshot\n",
    )
    .expect("write dim source yml");
    write_model(
        &project_dir,
        "events_by_dim",
        "---\nmaterialization: table\nrefresh: incremental\ngrain: partition\n\
         timeseries:\n  partition_column: event_date\n  event_time_column: event_date\n  \
         granularity: day\nmaintenance:\n  scan_bounds:\n    per_source:\n      \
         dim:\n        allow_full_scan: true\n---\n\
         SELECT f.fact_id AS fact_id, f.event_date AS event_date, f.amount AS amount, d.tag AS \
         tag\nFROM smelt.sources.facts f\nJOIN smelt.sources.dim d ON f.dim_id = d.dim_id\n",
    );

    let db_path = tmp.path().join("run.duckdb");
    let config = Arc::new(Config::load(&project_dir).expect("load smelt.yml"));

    {
        let backend = DuckDbBackend::new(&db_path, "main")
            .await
            .expect("open duckdb");
        backend
            .execute_sql(
                "CREATE TABLE main.sources_facts (fact_id INTEGER, dim_id INTEGER, event_date \
                 DATE, amount INTEGER)",
            )
            .await
            .expect("create facts source table");
        backend
            .execute_sql(
                "INSERT INTO main.sources_facts VALUES \
                 (1, 1, DATE '2025-01-10', 10), (2, 2, DATE '2025-01-10', 20)",
            )
            .await
            .expect("seed facts");
        backend
            .execute_sql("CREATE TABLE main.sources_dim (dim_id INTEGER, tag VARCHAR)")
            .await
            .expect("create dim source table");
        backend
            .execute_sql("INSERT INTO main.sources_dim VALUES (1, 'a'), (2, 'b')")
            .await
            .expect("seed dim");
    }

    // Run 1: creation — never the membership-recompute path.
    {
        let (db, graph) = build_db_and_graph(&project_dir, &config);
        execute_project(
            "keyless-membership-parity-run-1".to_string(),
            select_request("dev", "events_by_dim", "2025-01-10", "2025-01-11"),
            Arc::clone(&config),
            graph,
            db,
            &project_dir,
            &RecordingBackendFactory {
                db_path: db_path.clone(),
                backend: Arc::new(Mutex::new(None)),
            },
            &smelt_runtime::NoOpReporter,
            CancellationToken::new(),
        )
        .await
        .expect("first run (create) must succeed");
    }

    // Mutate the dimension in place — the `{tag}` cell becomes live.
    {
        let backend = DuckDbBackend::new(&db_path, "main")
            .await
            .expect("reopen duckdb");
        backend
            .execute_sql("UPDATE main.sources_dim SET tag = 'z' WHERE dim_id = 1")
            .await
            .expect("mutate dimension");
    }

    // Run 2: the dimension mutation dispatches the staged-candidate keyless
    // membership recompute.
    let (db, graph) = build_db_and_graph(&project_dir, &config);
    let backend_slot: Arc<Mutex<Option<Arc<RecordingBackend>>>> = Arc::new(Mutex::new(None));
    let factory = RecordingBackendFactory {
        db_path: db_path.clone(),
        backend: Arc::clone(&backend_slot),
    };
    let outcome = execute_project(
        "keyless-membership-parity-run-2".to_string(),
        select_request("dev", "events_by_dim", "2025-01-11", "2025-01-12"),
        Arc::clone(&config),
        graph,
        db,
        &project_dir,
        &factory,
        &smelt_runtime::NoOpReporter,
        CancellationToken::new(),
    )
    .await
    .expect("second run (keyless membership recompute) must succeed");

    let record = outcome
        .models
        .get("events_by_dim")
        .expect("events_by_dim ran");
    assert_eq!(
        record.strategy, "delete_insert_suppressed",
        "the dimension mutation must dispatch the staged-candidate membership-recompute \
         technique"
    );

    let backend = backend_slot
        .lock()
        .unwrap()
        .clone()
        .expect("backend recorded");
    let groups = backend.recorded_groups();
    let staged_groups: Vec<_> = groups
        .iter()
        .filter(|g| {
            g.statements
                .first()
                .is_some_and(|s| s.sql.starts_with("CREATE TEMP TABLE"))
        })
        .collect();
    assert_eq!(
        staged_groups.len(),
        1,
        "exactly one staged-candidate group must have executed: {:?}",
        groups
    );
    let group = staged_groups[0];
    assert!(
        group.transactional,
        "the staged-candidate keyless group is transactional"
    );
    assert_eq!(group.statements.len(), 7);

    // Recover the caller-composed `candidate_select` from the recorded
    // INSERT statement (statement index 1).
    let staged_relation = "__smelt_staged_events_by_dim";
    let sentinel_relation = "__smelt_sentinel_events_by_dim";
    let insert_sql = &group.statements[1].sql;
    let candidate_prefix = format!("INSERT INTO {staged_relation} ");
    assert!(
        insert_sql.starts_with(&candidate_prefix),
        "unexpected staged-candidate INSERT statement: {insert_sql}"
    );
    let candidate_select = &insert_sql[candidate_prefix.len()..];

    let expected = smelt_logical::maintenance::emit::emit_staged_candidate_conditional_keyless(
        "main.events_by_dim",
        staged_relation,
        sentinel_relation,
        None,
        candidate_select,
        MaintenanceDialect::DuckDb,
    );
    assert_eq!(
        &expected, group,
        "executed staged-candidate keyless group must be byte-identical to a direct emitter \
         call over the same inputs"
    );

    // Result-equivalence: the staged-candidate recompute actually executed
    // must leave the target multiset-equal to a full refresh of the model.
    assert!(
        multiset_equal(
            backend.as_ref(),
            "SELECT fact_id, event_date, amount, tag FROM main.events_by_dim",
            "SELECT f.fact_id, f.event_date, f.amount, d.tag FROM main.sources_facts f JOIN \
             main.sources_dim d ON f.dim_id = d.dim_id"
        )
        .await,
        "the staged-candidate keyless recompute actually executed must reproduce a full refresh"
    );
}

/// The repair family (`docs/specs/incremental_models.md` §"The repair
/// family"): a keyed `MAX` fold over a **clocked, mutable** source refuses
/// the faithful-fold source-posture obligation, so the derived plan admits
/// `Technique::PerGroupRecompute` on the model's own `NewData` trigger
/// instead. The statements a real `execute_project` run sends to the
/// connection must be byte-identical to a direct `emit_per_group_recompute`
/// call over the batch's own inputs — plus the family's result-equivalence
/// leg against a full-refresh oracle.
#[tokio::test]
async fn per_group_recompute_statements_come_from_the_emitter() {
    const ORDERS_SOURCE_YML: &str = r#"description: Mutable order snapshot
columns:
- name: order_id
  type: INTEGER
- name: customer_id
  type: INTEGER
- name: amount
  type: DECIMAL(10,2)
- name: order_date
  type: TIMESTAMP
timeseries:
  event_time_column: order_date
  partition_column: order_date
  granularity: day
unique_key: [order_id]
mutation_profile:
  kind: mutable_snapshot
"#;
    const MODEL_SQL: &str = "SELECT customer_id, MAX(amount) AS max_amount \
         FROM smelt.sources.raw.orders \
         WHERE order_date BETWEEN TIMESTAMP '2025-01-14' - INTERVAL '3 days' AND TIMESTAMP \
         '2025-01-14' \
         GROUP BY customer_id";
    const MODEL_FILE: &str = "---\n\
         materialization: table\n\
         refresh: incremental\n\
         grain: key\n\
         unique_key: customer_id\n\
         ---\n";

    let source_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/timeseries")
        .canonicalize()
        .expect("examples/timeseries exists");

    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().join("project");
    copy_dir_recursive(&source_dir, &project_dir);
    std::fs::write(
        project_dir.join("models/sources/raw/orders.yml"),
        ORDERS_SOURCE_YML,
    )
    .expect("write orders source yml");
    std::fs::write(
        project_dir.join("models/customer_max_amount.sql"),
        format!("{MODEL_FILE}{MODEL_SQL}\n"),
    )
    .expect("write repair model fixture");

    let db_path = tmp.path().join("run.duckdb");
    let config = Arc::new(Config::load(&project_dir).expect("load smelt.yml"));

    {
        let backend = DuckDbBackend::new(&db_path, "main")
            .await
            .expect("open duckdb");
        backend
            .execute_sql(
                "CREATE TABLE main.sources_raw_orders (order_id INTEGER, customer_id INTEGER, \
                 amount DECIMAL(10,2), order_date TIMESTAMP)",
            )
            .await
            .expect("create orders source table");
        backend
            .execute_sql(
                "INSERT INTO main.sources_raw_orders VALUES \
                 (1, 1, 100.00, TIMESTAMP '2025-01-13 10:00:00'), \
                 (2, 1, 50.00, TIMESTAMP '2025-01-13 11:00:00'), \
                 (3, 2, 70.00, TIMESTAMP '2025-01-11 10:00:00')",
            )
            .await
            .expect("seed orders");
    }

    // Run 1: creation — nothing to repair yet, the fold's create path runs.
    {
        let (db, graph) = build_db_and_graph(&project_dir, &config);
        execute_project(
            "repair-parity-run-1".to_string(),
            select_request("dev", "customer_max_amount", "2025-01-11", "2025-01-14"),
            Arc::clone(&config),
            graph,
            db,
            &project_dir,
            &RecordingBackendFactory {
                db_path: db_path.clone(),
                backend: Arc::new(Mutex::new(None)),
            },
            &smelt_runtime::NoOpReporter,
            CancellationToken::new(),
        )
        .await
        .expect("first run (create) must succeed");
    }

    // The retraction `MAX` cannot undo: customer 1's top contribution is
    // corrected downward in place.
    {
        let backend = DuckDbBackend::new(&db_path, "main")
            .await
            .expect("reopen duckdb");
        backend
            .execute_sql("UPDATE main.sources_raw_orders SET amount = 10.00 WHERE order_id = 1")
            .await
            .expect("retract");
    }

    let (db, graph) = build_db_and_graph(&project_dir, &config);
    let backend_slot: Arc<Mutex<Option<Arc<RecordingBackend>>>> = Arc::new(Mutex::new(None));
    let factory = RecordingBackendFactory {
        db_path: db_path.clone(),
        backend: Arc::clone(&backend_slot),
    };
    let outcome = execute_project(
        "repair-parity-run-2".to_string(),
        select_request("dev", "customer_max_amount", "2025-01-16", "2025-01-17"),
        Arc::clone(&config),
        graph,
        db,
        &project_dir,
        &factory,
        &smelt_runtime::NoOpReporter,
        CancellationToken::new(),
    )
    .await
    .expect("second run (per-group recompute) must succeed");

    let record = outcome
        .models
        .get("customer_max_amount")
        .expect("customer_max_amount ran");
    assert_eq!(
        record.strategy, "per_group_recompute",
        "the retraction must dispatch the repair family, not the fold"
    );

    let backend = backend_slot
        .lock()
        .unwrap()
        .clone()
        .expect("backend recorded");
    let groups = backend.recorded_groups();
    let repair_groups: Vec<_> = groups
        .iter()
        .filter(|g| {
            g.statements
                .first()
                .is_some_and(|s| s.sql.starts_with("CREATE TEMP TABLE __smelt_repair_"))
        })
        .collect();
    assert_eq!(
        repair_groups.len(),
        1,
        "exactly one per-group-recompute group must have executed: {groups:?}"
    );
    let group = repair_groups[0];
    assert!(group.transactional, "the repair group is transactional");
    assert_eq!(group.statements.len(), 5);

    // Recover the caller-composed `candidate_select` from the recorded
    // `INSERT INTO {staged} {select}` (statement index 1).
    let staged_relation = "__smelt_repair_customer_max_amount";
    let candidate_prefix = format!("INSERT INTO {staged_relation} ");
    let insert_sql = &group.statements[1].sql;
    assert!(
        insert_sql.starts_with(&candidate_prefix),
        "unexpected staged INSERT statement: {insert_sql}"
    );
    let candidate_select = &insert_sql[candidate_prefix.len()..];

    // This is a `MutationProfile::MutableSnapshot` source, so the affected-
    // key relation is the group-grain sidecar diff (P9), not the append-only
    // clamped scan — a backend-state-dependent `VALUES (...)` literal
    // recovered straight from the executed candidate, per
    // `extract_affected_keys_select`'s own doc comment.
    let key = vec!["customer_id".to_string()];
    let affected_keys_select = extract_affected_keys_select(candidate_select);
    assert!(
        affected_keys_select.contains("__smelt_repair_group_keys(delta_key)"),
        "a MutableSnapshot source's affected-key relation must be the sidecar-diff-derived \
         literal keys relation: {affected_keys_select}"
    );

    let expected = emit_per_group_recompute(
        "main.customer_max_amount",
        staged_relation,
        &key,
        &affected_keys_select,
        candidate_select,
        MaintenanceDialect::DuckDb,
    );
    assert_eq!(
        &expected, group,
        "executed per-group-recompute group must be byte-identical to a direct emitter call \
         over the same inputs"
    );

    // Result-equivalence: the repair actually executed must leave the target
    // multiset-equal to a full refresh over the same inputs.
    assert!(
        multiset_equal(
            backend.as_ref(),
            "SELECT customer_id, max_amount FROM main.customer_max_amount",
            "SELECT customer_id, MAX(amount) AS max_amount FROM main.sources_raw_orders WHERE \
             order_date BETWEEN TIMESTAMP '2025-01-14' - INTERVAL '3 days' AND TIMESTAMP \
             '2025-01-14' GROUP BY customer_id"
        )
        .await,
        "the repair actually executed must reproduce a full refresh"
    );
}

/// Phase 7 (`docs/outcomes/20260809-output-delta-typing/phases/07-plan.md`):
/// a key-addressed model-edge cell's `Technique::PerGroupRecompute` group
/// must be byte-identical to a direct [`emit_per_group_recompute`] call —
/// the SAME parity proof `per_group_recompute_statements_come_from_the_
/// emitter` runs for the ordinary declared-source repair route, over a
/// clockless `KeyedUpsert` upstream model edge instead of a
/// `mutation_profile: mutable_snapshot` source.
#[tokio::test]
async fn key_addressed_model_edge_statements_come_from_the_emitter() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().join("project");
    std::fs::create_dir_all(project_dir.join("models/sources")).expect("mkdir models/sources");
    std::fs::write(
        project_dir.join("smelt.yml"),
        "name: key_addressed_parity\nversion: 1\npaths:\n  - models\n\
         targets:\n  dev:\n    type: duckdb\n    schema: main\n\
         default_materialization: view\n",
    )
    .expect("write smelt.yml");
    std::fs::write(
        project_dir.join("models/sources/payments.yml"),
        "description: payments\ncolumns:\n- name: user_id\n  type: INTEGER\n\
         - name: amount\n  type: DECIMAL(10,2)\n- name: d\n  type: DATE\n\
         mutation_profile:\n  kind: append_only\n\
         timeseries:\n  partition_column: d\n  event_time_column: d\n  granularity: day\n",
    )
    .expect("write payments source yml");
    write_model(
        &project_dir,
        "agg",
        "---\nmaterialization: table\nrefresh: incremental\ngrain: key\n\
         unique_key: user_id\nmaintenance:\n  scan_bounds:\n    per_source:\n      \
         payments:\n        allow_full_scan: true\n---\n\
         SELECT user_id, SUM(amount) AS total\nFROM smelt.sources.payments\n\
         GROUP BY user_id\n",
    );
    write_model(
        &project_dir,
        "downstream",
        "---\nmaterialization: table\nrefresh: incremental\ngrain: key\n\
         unique_key: user_id\n---\n\
         SELECT user_id, ANY_VALUE(total) AS total FROM smelt.agg GROUP BY user_id\n",
    );

    let db_path = tmp.path().join("run.duckdb");
    let config = Arc::new(Config::load(&project_dir).expect("load smelt.yml"));

    {
        let backend = DuckDbBackend::new(&db_path, "main")
            .await
            .expect("open duckdb");
        backend
            .execute_sql(
                "CREATE TABLE main.sources_payments (user_id INTEGER, amount DECIMAL(10,2), \
                 d DATE)",
            )
            .await
            .expect("create payments source table");
        backend
            .execute_sql(
                "INSERT INTO main.sources_payments VALUES \
                 (1, 100.00, DATE '2025-01-01'), (1, 50.00, DATE '2025-01-02'), \
                 (2, 70.00, DATE '2025-01-01')",
            )
            .await
            .expect("seed payments");
    }

    // `agg` is a clocked, `grain: key` window-forward model always run
    // unwindowed here — that now refuses without `--full-refresh`
    // (`docs/specs/incremental_shapes.md` §"The key grain"). Harmless for
    // `downstream`: `full_refresh` is only consulted by that one
    // windowless-keyed-run branch, never by the key-addressed model-edge
    // dispatch this test pins.
    let multi_select = |models: &[&str]| ExecuteRequest {
        target: "dev".to_string(),
        select: models.iter().map(|s| s.to_string()).collect(),
        exclude: vec![],
        start: None,
        end: None,
        batch_size_days: None,
        per_partition: false,
        full_refresh: true,
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
    };

    // Run 1: creation — nothing to fold yet.
    {
        let (db, graph) = build_db_and_graph(&project_dir, &config);
        execute_project(
            "key-edge-parity-run-1".to_string(),
            multi_select(&["agg", "downstream"]),
            Arc::clone(&config),
            graph,
            db,
            &project_dir,
            &RecordingBackendFactory {
                db_path: db_path.clone(),
                backend: Arc::new(Mutex::new(None)),
            },
            &smelt_runtime::NoOpReporter,
            CancellationToken::new(),
        )
        .await
        .expect("first run (create) must succeed");
    }

    // Mutate user 1's contribution in place.
    {
        let backend = DuckDbBackend::new(&db_path, "main")
            .await
            .expect("reopen duckdb");
        backend
            .execute_sql(
                "UPDATE main.sources_payments SET amount = 200.00 WHERE user_id = 1 AND \
                 amount = 100.00",
            )
            .await
            .expect("mutate payments");
    }

    let (db, graph) = build_db_and_graph(&project_dir, &config);
    let backend_slot: Arc<Mutex<Option<Arc<RecordingBackend>>>> = Arc::new(Mutex::new(None));
    let factory = RecordingBackendFactory {
        db_path: db_path.clone(),
        backend: Arc::clone(&backend_slot),
    };
    let outcome = execute_project(
        "key-edge-parity-run-2".to_string(),
        multi_select(&["agg", "downstream"]),
        Arc::clone(&config),
        graph,
        db,
        &project_dir,
        &factory,
        &smelt_runtime::NoOpReporter,
        CancellationToken::new(),
    )
    .await
    .expect("second run (key-addressed recompute) must succeed");

    let record = outcome.models.get("downstream").expect("downstream ran");
    assert_eq!(
        record.strategy, "per_group_recompute",
        "the upstream's key-addressed fold must dispatch the repair family"
    );

    let backend = backend_slot
        .lock()
        .unwrap()
        .clone()
        .expect("backend recorded");
    let groups = backend.recorded_groups();
    let repair_groups: Vec<_> = groups
        .iter()
        .filter(|g| {
            g.statements
                .first()
                .is_some_and(|s| s.sql.starts_with("CREATE TEMP TABLE __smelt_repair_"))
        })
        .collect();
    assert_eq!(
        repair_groups.len(),
        1,
        "exactly one key-addressed per-group-recompute group must have executed: {groups:?}"
    );
    let group = repair_groups[0];
    assert!(group.transactional, "the repair group is transactional");
    assert_eq!(group.statements.len(), 5);

    let staged_relation = "__smelt_repair_downstream";
    let candidate_prefix = format!("INSERT INTO {staged_relation} ");
    let insert_sql = &group.statements[1].sql;
    assert!(
        insert_sql.starts_with(&candidate_prefix),
        "unexpected staged INSERT statement: {insert_sql}"
    );
    let candidate_select = &insert_sql[candidate_prefix.len()..];

    let key = vec!["user_id".to_string()];
    let affected_keys_select = extract_affected_keys_select(candidate_select);
    assert!(
        affected_keys_select.contains("SELECT DISTINCT"),
        "a key-addressed cell's affected-key relation must be the key-restricted projection \
         over the upstream table: {affected_keys_select}"
    );
    assert!(
        !affected_keys_select
            .to_uppercase()
            .contains("__SMELT_REPAIR_GROUP_KEYS"),
        "a key-addressed cell must not route through the ordinary sidecar-literal-keys \
         relation shape — it reads the upstream table directly: {affected_keys_select}"
    );

    let expected = emit_per_group_recompute(
        "main.downstream",
        staged_relation,
        &key,
        &affected_keys_select,
        candidate_select,
        MaintenanceDialect::DuckDb,
    );
    assert_eq!(
        &expected, group,
        "executed key-addressed per-group-recompute group must be byte-identical to a direct \
         emitter call over the same inputs"
    );

    // Result-equivalence: the key-addressed fold actually executed must
    // leave the downstream equal to a full refresh over agg's current state.
    assert!(
        multiset_equal(
            backend.as_ref(),
            "SELECT user_id, total FROM main.downstream",
            "SELECT user_id, SUM(amount) AS total FROM main.sources_payments GROUP BY user_id"
        )
        .await,
        "the key-addressed fold actually executed must reproduce a full refresh"
    );
}

#[tokio::test]
async fn diff_patch_statements_come_from_the_emitter() {
    const ORDERS_SOURCE_YML: &str = r#"description: Mutable order snapshot
columns:
- name: order_id
  type: INTEGER
- name: customer_id
  type: INTEGER
- name: amount
  type: DECIMAL(10,2)
- name: order_date
  type: TIMESTAMP
timeseries:
  event_time_column: order_date
  partition_column: order_date
  granularity: day
unique_key: [order_id]
mutation_profile:
  kind: mutable_snapshot
"#;
    const MODEL_SQL: &str = "SELECT customer_id, MAX(amount) AS max_amount \
         FROM smelt.sources.raw.orders \
         WHERE order_date BETWEEN TIMESTAMP '2025-01-14' - INTERVAL '3 days' AND TIMESTAMP \
         '2025-01-14' \
         GROUP BY customer_id";
    const MODEL_FILE: &str = "---\n\
         materialization: table\n\
         refresh: incremental\n\
         grain: key\n\
         unique_key: customer_id\n\
         maintenance:\n\
         \x20\x20cells:\n\
         \x20\x20- on: raw.orders\n\
         \x20\x20\x20\x20columns: [max_amount]\n\
         \x20\x20\x20\x20write: diff_patch\n\
         ---\n";

    let source_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/timeseries")
        .canonicalize()
        .expect("examples/timeseries exists");

    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().join("project");
    copy_dir_recursive(&source_dir, &project_dir);
    std::fs::write(
        project_dir.join("models/sources/raw/orders.yml"),
        ORDERS_SOURCE_YML,
    )
    .expect("write orders source yml");
    std::fs::write(
        project_dir.join("models/customer_max_amount.sql"),
        format!("{MODEL_FILE}{MODEL_SQL}\n"),
    )
    .expect("write diff_patch model fixture");

    let db_path = tmp.path().join("run.duckdb");
    let config = Arc::new(Config::load(&project_dir).expect("load smelt.yml"));

    {
        let backend = DuckDbBackend::new(&db_path, "main")
            .await
            .expect("open duckdb");
        backend
            .execute_sql(
                "CREATE TABLE main.sources_raw_orders (order_id INTEGER, customer_id INTEGER, \
                 amount DECIMAL(10,2), order_date TIMESTAMP)",
            )
            .await
            .expect("create orders source table");
        backend
            .execute_sql(
                "INSERT INTO main.sources_raw_orders VALUES \
                 (1, 1, 100.00, TIMESTAMP '2025-01-13 10:00:00'), \
                 (2, 1, 50.00, TIMESTAMP '2025-01-13 11:00:00'), \
                 (3, 2, 70.00, TIMESTAMP '2025-01-11 10:00:00')",
            )
            .await
            .expect("seed orders");
    }

    // Run 1: creation — nothing to repair yet, the fold's create path runs.
    {
        let (db, graph) = build_db_and_graph(&project_dir, &config);
        execute_project(
            "diff-patch-parity-run-1".to_string(),
            select_request("dev", "customer_max_amount", "2025-01-11", "2025-01-14"),
            Arc::clone(&config),
            graph,
            db,
            &project_dir,
            &RecordingBackendFactory {
                db_path: db_path.clone(),
                backend: Arc::new(Mutex::new(None)),
            },
            &smelt_runtime::NoOpReporter,
            CancellationToken::new(),
        )
        .await
        .expect("first run (create) must succeed");
    }

    // The retraction `MAX` cannot undo: customer 1's top contribution is
    // corrected downward in place.
    {
        let backend = DuckDbBackend::new(&db_path, "main")
            .await
            .expect("reopen duckdb");
        backend
            .execute_sql("UPDATE main.sources_raw_orders SET amount = 10.00 WHERE order_id = 1")
            .await
            .expect("retract");
    }

    let (db, graph) = build_db_and_graph(&project_dir, &config);
    let backend_slot: Arc<Mutex<Option<Arc<RecordingBackend>>>> = Arc::new(Mutex::new(None));
    let factory = RecordingBackendFactory {
        db_path: db_path.clone(),
        backend: Arc::clone(&backend_slot),
    };
    let outcome = execute_project(
        "diff-patch-parity-run-2".to_string(),
        select_request("dev", "customer_max_amount", "2025-01-16", "2025-01-17"),
        Arc::clone(&config),
        graph,
        db,
        &project_dir,
        &factory,
        &smelt_runtime::NoOpReporter,
        CancellationToken::new(),
    )
    .await
    .expect("second run (diff_patch) must succeed");

    let record = outcome
        .models
        .get("customer_max_amount")
        .expect("customer_max_amount ran");
    assert_eq!(
        record.strategy, "diff_patch",
        "the write: diff_patch pin must dispatch the diff-patch write, not the repair family's \
         own targeted delete+insert"
    );

    let backend = backend_slot
        .lock()
        .unwrap()
        .clone()
        .expect("backend recorded");
    let groups = backend.recorded_groups();
    let diff_patch_groups: Vec<_> = groups
        .iter()
        .filter(|g| {
            g.statements
                .first()
                .is_some_and(|s| s.sql.starts_with("CREATE TEMP TABLE __smelt_diff_patch_"))
        })
        .collect();
    assert_eq!(
        diff_patch_groups.len(),
        1,
        "exactly one diff_patch group must have executed: {groups:?}"
    );
    let group = diff_patch_groups[0];
    assert!(group.transactional, "the diff_patch group is transactional");
    // Update leg + delete leg (PerGroupRecompute's own bounded-slice
    // admission discharges diff_patch's completeness premise, so the delete
    // leg is included) + create/insert-candidates/insert/drop = 6.
    assert_eq!(group.statements.len(), 6);
    assert!(
        group
            .statements
            .iter()
            .any(|s| s.sql.starts_with("DELETE") && s.sql.contains("NOT EXISTS")),
        "the delete leg must be present: {group:?}"
    );

    // Recover the caller-composed `candidate_select` from the recorded
    // `INSERT INTO {staged} {select}` (statement index 1).
    let staged_relation = "__smelt_diff_patch_customer_max_amount";
    let candidate_prefix = format!("INSERT INTO {staged_relation} ");
    let insert_sql = &group.statements[1].sql;
    assert!(
        insert_sql.starts_with(&candidate_prefix),
        "unexpected staged INSERT statement: {insert_sql}"
    );
    let candidate_select = &insert_sql[candidate_prefix.len()..];

    // This is a `MutationProfile::MutableSnapshot` source, so the affected-
    // key relation is the group-grain sidecar diff (P9), not the append-only
    // clamped scan — a backend-state-dependent `VALUES (...)` literal
    // recovered straight from the executed candidate, per
    // `extract_affected_keys_select`'s own doc comment.
    let key = vec!["customer_id".to_string()];
    let affected_keys_select = extract_affected_keys_select(candidate_select);
    assert!(
        affected_keys_select.contains("__smelt_repair_group_keys(delta_key)"),
        "a MutableSnapshot source's affected-key relation must be the sidecar-diff-derived \
         literal keys relation: {affected_keys_select}"
    );

    let slice_predicate = smelt_runtime::maintenance_driver::repair_slice_predicate(
        "customer_max_amount",
        &key,
        &affected_keys_select,
    );
    let expected = emit_diff_patch(
        "main.customer_max_amount",
        staged_relation,
        &key,
        candidate_select,
        &["max_amount".to_string()],
        &slice_predicate,
        &smelt_logical::maintenance::diff_patch::DeleteLeg::Complete,
        MaintenanceDialect::DuckDb,
    );
    assert_eq!(
        &expected, group,
        "executed diff_patch group must be byte-identical to a direct emitter call over the \
         same inputs"
    );

    // Result-equivalence: the diff_patch write actually executed must leave
    // the target multiset-equal to a full refresh over the same inputs.
    assert!(
        multiset_equal(
            backend.as_ref(),
            "SELECT customer_id, max_amount FROM main.customer_max_amount",
            "SELECT customer_id, MAX(amount) AS max_amount FROM main.sources_raw_orders WHERE \
             order_date BETWEEN TIMESTAMP '2025-01-14' - INTERVAL '3 days' AND TIMESTAMP \
             '2025-01-14' GROUP BY customer_id"
        )
        .await,
        "the diff_patch write actually executed must reproduce a full refresh"
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
        &[],
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

/// Phase 16 (`docs/outcomes/20260815-definition-delta-migrate/phases/
/// 16-plan.md`): the keyed-fold observed-delta recording's `IS DISTINCT
/// FROM` guard must be BYTE-IDENTICAL to `emit_keyed_fold_suppressed`'s own
/// matched-arm guard — one comparison (over the fold's own combine
/// expression, not the raw delta column), two consumers.
#[test]
fn keyed_fold_changed_key_select_matches_the_merge_guard() {
    let compared_columns = vec!["score".to_string()];
    let folds = vec![(
        "score".to_string(),
        "GREATEST(target.score, delta.score)".to_string(),
    )];

    let merge_group = smelt_logical::maintenance::emit::emit_keyed_fold_suppressed(
        "main.dim_scores",
        &["user_id".to_string()],
        &folds,
        "SELECT user_id, score FROM main.src_scores",
        None,
        &compared_columns,
        MaintenanceDialect::DuckDb,
    );
    let merge_sql = &merge_group.statements[0].sql;

    let record_predicate = smelt_runtime::maintenance_driver::keyed_fold_changed_row_predicate(
        &compared_columns,
        &folds,
    );

    assert!(
        merge_sql.contains(&record_predicate),
        "the recorded-delta predicate must appear byte-identical inside the suppressed keyed \
         fold's own matched-arm guard — predicate: {record_predicate:?}, MERGE: {merge_sql:?}"
    );

    let changed_keys_query = smelt_runtime::maintenance_driver::keyed_fold_changed_keys_select(
        "main.dim_scores",
        &["user_id".to_string()],
        "SELECT user_id, score FROM main.src_scores",
        &compared_columns,
        &folds,
        None,
    );
    assert!(
        changed_keys_query.contains(&record_predicate),
        "keyed_fold_changed_keys_select must carry the identical predicate text, got: \
         {changed_keys_query:?}"
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

/// The full-recompute variant (`docs/plans/20260808-membership-sensitivity.md`
/// Phase 3): the executed `StatementGroup` is byte-identical to a direct
/// `emit_staged_candidate_conditional_recompute` call, and — unlike its
/// region-scoped sibling above — a row whose key is entirely absent from the
/// candidate (user 3) is genuinely DELETED, never merely left untouched:
/// this variant's `candidate_select` always represents the model's own full
/// current state, so absence means departure. A matched-but-unchanged row
/// (user 1) is still suppressed (never deleted/reinserted), proving the
/// extra departed-key `DELETE` is a no-op over still-present keys.
#[tokio::test]
async fn staged_candidate_conditional_recompute_deletes_departed_keys() {
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

    // user 1: unchanged tier; user 2: changed tier; user 4: brand new. user
    // 3 is genuinely departed — the model's own full recompute no longer
    // produces a row for it at all (e.g. the dimension row a fact joined on
    // was deleted).
    let candidate_select = "SELECT * FROM (VALUES (1, 'bronze', 'run2'), (2, 'platinum', \
                             'run2'), (4, 'new', 'run2')) AS t(user_id, tier, run_marker)";
    let key = vec!["user_id".to_string()];
    let compared_columns = vec!["tier".to_string()];

    let group = emit_staged_candidate_conditional_recompute(
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
        .expect("staged-candidate recompute write must succeed");

    let recorded = backend.recorded_groups();
    assert_eq!(recorded.len(), 1);
    let expected = emit_staged_candidate_conditional_recompute(
        "main.dim_users",
        "__smelt_staged_dim_users",
        &key,
        candidate_select,
        &compared_columns,
        MaintenanceDialect::DuckDb,
    );
    assert_eq!(
        &expected, &recorded[0],
        "executed staged-candidate recompute group must be byte-identical to a direct emitter \
         call over the same inputs"
    );

    let rows = backend
        .execute_sql("SELECT user_id, tier, run_marker FROM main.dim_users ORDER BY user_id")
        .await
        .expect("read back target");
    let batch = &rows[0];
    let ids = batch
        .column(0)
        .as_any()
        .downcast_ref::<Int64Array>()
        .expect("user_id is Int64");
    assert_eq!(
        ids.len(),
        3,
        "user 3 (departed — absent from the full-recompute candidate) must be deleted, leaving \
         exactly users 1, 2, 4"
    );
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
        "user 1 (unchanged, suppressed) keeps its prior marker; user 2 (changed) and user 4 \
         (new) carry the new run's marker"
    );

    assert!(
        multiset_equal(
            &backend,
            "SELECT user_id, tier FROM main.dim_users",
            "SELECT user_id, tier FROM (VALUES (1, 'bronze'), (2, 'platinum'), (4, 'new')) AS \
             t(user_id, tier)"
        )
        .await,
        "the staged-candidate recompute write must reproduce the full-refresh oracle — no \
         departed row survives"
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
    let closure = smelt_logical::maintenance::SkeletonSourceClosure::Closed {
        row_preservation: smelt_logical::maintenance::RowPreservation::JoinShape,
    };

    smelt_runtime::maintenance_driver::execute_delete_insert_with_delta_restriction(
        &backend,
        "main",
        "enriched",
        "event_date",
        &region,
        body,
        body,
        Some("event_id"),
        Some(&closure),
        RestrictionDeltaSource::ModelEdge {
            upstream_model: "silver.fact",
            window_start: "2026-07-01",
            window_end: "2026-07-02",
        },
        None,
        smelt_logical::maintenance::emit::MaintenanceDialect::DuckDb,
        &no_retry_policy(),
        &smelt_runtime::probes::ProbePolicy::per_run(),
        &[],
        &[],
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

/// State residency (`docs/outcomes/20260904-state-residency/outcome.md`
/// criterion 1): the delta-restricted branch of `execute_delete_insert_
/// with_delta_restriction` — phase 2 left this path recording no
/// reconciliation-ledger reset at all — must, when handed non-empty
/// `ensure_sqls`/`pre_write_sqls`, route the write through `Backend::
/// execute_write_with_bookkeeping` and record the SAME reset pair a caller
/// (`execute.rs`) builds via `generate_ledger_recompute_reset_sqls`, byte
/// for byte, alongside its own delta-restricted DELETE+INSERT.
#[tokio::test]
async fn delta_restricted_recompute_records_the_ledger_reset() {
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
    let closure = smelt_logical::maintenance::SkeletonSourceClosure::Closed {
        row_preservation: smelt_logical::maintenance::RowPreservation::JoinShape,
    };

    let ledger_ensure_sqls = vec![smelt_state::ddl_duckdb::generate_ledger_table_ddl("main")];
    let ledger_pre_write_sqls = smelt_state::ddl_duckdb::generate_ledger_recompute_reset_sqls(
        "main",
        "silver.enriched",
        "{*}",
        "2026-07-01",
        "2026-07-02",
        "self",
        "2026-07-02",
    );

    smelt_runtime::maintenance_driver::execute_delete_insert_with_delta_restriction(
        &backend,
        "main",
        "enriched",
        "event_date",
        &region,
        body,
        body,
        Some("event_id"),
        Some(&closure),
        RestrictionDeltaSource::ModelEdge {
            upstream_model: "silver.fact",
            window_start: "2026-07-01",
            window_end: "2026-07-02",
        },
        None,
        smelt_logical::maintenance::emit::MaintenanceDialect::DuckDb,
        &no_retry_policy(),
        &smelt_runtime::probes::ProbePolicy::per_run(),
        &ledger_ensure_sqls,
        &ledger_pre_write_sqls,
    )
    .await
    .expect("delta-restricted recompute with ledger bookkeeping must succeed");

    let sql_log = backend.recorded_sql();
    assert!(
        sql_log.contains(&ledger_ensure_sqls[0]),
        "the ledger ensure DDL must be sent as raw SQL: {sql_log:?}"
    );
    for stmt in &ledger_pre_write_sqls {
        assert!(
            sql_log.contains(stmt),
            "the delta-restricted branch must record the SAME ledger reset a plain DeleteInsert \
             write would (byte-identical to `generate_ledger_recompute_reset_sqls`): {stmt}\n\
             recorded: {sql_log:?}"
        );
    }

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
    for stmt in &group.statements {
        assert!(
            !stmt.sql.contains("_smelt_ledger"),
            "ledger bookkeeping must never appear inside the maintenance StatementGroup: {}",
            stmt.sql
        );
    }

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
        "the write group itself must still be byte-identical to the emitter's own output — \
         bookkeeping must not alter what gets written"
    );
}

/// The region family's own change-suppressed conditional variant
/// (`RegionWrite::Suppressed`, `docs/outcomes/20260815-definition-delta-
/// migrate/phases/27b-plan.md`) executes exactly `emit_diff_patch`'s own
/// output — no delta restriction admitted (`restrict_column: None`), so the
/// dispatch falls through past the T3 arm straight to the region-write
/// dimension.
#[tokio::test]
async fn region_conditional_write_matches_the_emitted_group_byte_for_byte() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("test.duckdb");
    let inner = DuckDbBackend::new(&db_path, "main")
        .await
        .expect("open duckdb");
    let backend = RecordingBackend::new(inner);

    backend
        .execute_sql(
            "CREATE TABLE main.regions (region_id VARCHAR, region_date DATE, amount INTEGER)",
        )
        .await
        .expect("create target table");
    backend
        .execute_sql(
            "INSERT INTO main.regions VALUES ('r1', '2026-07-01', 10), ('r2', '2026-07-01', 20)",
        )
        .await
        .expect("seed target table");

    let region = smelt_logical::maintenance::emit::Region {
        start: "'2026-07-01'".to_string(),
        end: "'2026-07-02'".to_string(),
    };
    let body = "SELECT region_id, region_date, amount FROM (VALUES \
                ('r1', DATE '2026-07-01', 10), ('r2', DATE '2026-07-01', 25)) \
                AS t(region_id, region_date, amount)";
    let region_write = smelt_logical::maintenance::choice::RegionWrite::Suppressed {
        key: vec!["region_id".to_string()],
        compared_columns: vec!["amount".to_string()],
    };

    smelt_runtime::maintenance_driver::execute_delete_insert_with_delta_restriction(
        &backend,
        "main",
        "regions",
        "region_date",
        &region,
        body,
        body,
        None,
        None,
        RestrictionDeltaSource::ModelEdge {
            upstream_model: "sources.regions_raw",
            window_start: "2026-07-01",
            window_end: "2026-07-02",
        },
        Some(&region_write),
        MaintenanceDialect::DuckDb,
        &no_retry_policy(),
        &smelt_runtime::probes::ProbePolicy::per_run(),
        &[],
        &[],
    )
    .await
    .expect("suppressed region recompute must succeed");

    let groups = backend.recorded_groups();
    let diff_patch_groups: Vec<_> = groups
        .iter()
        .filter(|g| {
            g.statements[0]
                .sql
                .starts_with("CREATE TEMP TABLE __smelt_diff_patch_main_regions")
        })
        .collect();
    assert_eq!(
        diff_patch_groups.len(),
        1,
        "exactly one staged diff_patch group: {groups:?}"
    );
    let group = diff_patch_groups[0];

    let slice_predicate = region.predicate(Some("main.regions"), "region_date");
    let expected = emit_diff_patch(
        "main.regions",
        "__smelt_diff_patch_main_regions",
        &["region_id".to_string()],
        body,
        &["amount".to_string()],
        &slice_predicate,
        &smelt_logical::maintenance::diff_patch::DeleteLeg::Complete,
        MaintenanceDialect::DuckDb,
    );
    assert_eq!(
        group.statements, expected.statements,
        "the executed region conditional group must be byte-identical to a direct emitter call \
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
        body,
        Some("event_id"),
        Some(&closure),
        RestrictionDeltaSource::ModelEdge {
            upstream_model: "silver.fact",
            window_start: "2026-07-01",
            window_end: "2026-07-02",
        },
        None,
        smelt_logical::maintenance::emit::MaintenanceDialect::DuckDb,
        &no_retry_policy(),
        &smelt_runtime::probes::ProbePolicy::per_run(),
        &[],
        &[],
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

/// Phase F3 (`docs/plans/20260715-composed-axes-conditional-maintenance.md`)
/// — the fingerprint-sidecar diff query is emitter-authored (unlike the T5
/// observed-delta recording query above, which D1 ruled smelt-state
/// bookkeeping): `smelt_runtime::maintenance_driver::
/// diff_fingerprint_sidecar_changed_keys`/`refresh_fingerprint_sidecar` must
/// execute SQL text byte-identical to a direct call of
/// `smelt_logical::maintenance::emit::emit_fingerprint_sidecar_diff`/
/// `emit_fingerprint_digest_select` and
/// `smelt_state::ddl_duckdb::generate_fingerprint_sidecar_refresh_sql`/
/// `_gc_sql` over the same resolved inputs — this is a direct-dispatch leg
/// (no `execute_project` model pipeline involved, matching the precedent
/// `suppressed_keyed_fold_statements_come_from_the_emitter` documents: the
/// sidecar is not yet wired into the live trigger/technique-selection
/// pipeline, that wiring is a later phase's scope).
#[tokio::test]
async fn fingerprint_sidecar_diff_and_refresh_statements_come_from_the_emitter() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("test.duckdb");
    let inner = DuckDbBackend::new(&db_path, "main")
        .await
        .expect("open duckdb");
    let backend = RecordingBackend::new(inner);

    backend
        .execute_sql(
            "CREATE TABLE main.dim_users (id INTEGER, name VARCHAR, tier VARCHAR, notes VARCHAR)",
        )
        .await
        .expect("create source table");
    backend
        .execute_sql(
            "INSERT INTO main.dim_users VALUES \
             (1, 'Alice', 'gold', 'n1'), (2, 'Bob', 'silver', 'n2'), (3, 'Cara', 'gold', 'n3')",
        )
        .await
        .expect("seed source table");

    let projection = smelt_logical::analysis::fingerprint::Projection::Columns(
        ["name".to_string(), "tier".to_string()]
            .into_iter()
            .collect(),
    );
    let source_key = vec!["id".to_string()];
    let all_source_columns = vec![
        "id".to_string(),
        "name".to_string(),
        "tier".to_string(),
        "notes".to_string(),
    ];
    // Phase F4 — the consuming model's SQL text, folded into the sidecar's
    // identity stamp (`compute_fingerprint_sidecar_stamp`).
    let model_sql = "SELECT id, name, tier FROM smelt.sources.dim_users";

    // Run 1: absent sidecar — every source row is "changed" (whole-table
    // delta), and this diff also creates the sidecar table.
    let changed = smelt_runtime::maintenance_driver::diff_fingerprint_sidecar_changed_keys(
        &backend,
        "main",
        "smelt.sources.dim_users",
        "main.dim_users",
        &source_key,
        &projection,
        &all_source_columns,
        model_sql,
    )
    .await
    .expect("first diff against an absent sidecar");
    let mut changed_sorted = changed.clone();
    changed_sorted.sort();
    assert_eq!(
        changed_sorted,
        vec!["1".to_string(), "2".to_string(), "3".to_string()],
        "an absent sidecar must report every current source row as changed"
    );

    // The executed diff SQL must be byte-identical to a direct emitter call
    // over the same resolved inputs.
    let identity = smelt_logical::analysis::fingerprint::projection_identity(&projection);
    let stamp =
        smelt_runtime::maintenance_driver::compute_fingerprint_sidecar_stamp(&identity, model_sql);
    let expected_diff_sql = smelt_logical::maintenance::emit::emit_fingerprint_sidecar_diff(
        "main.dim_users",
        &source_key,
        &["name".to_string(), "tier".to_string()],
        "main._smelt_fingerprint_sidecar",
        "smelt.sources.dim_users",
        &identity,
        &stamp,
        MaintenanceDialect::DuckDb,
    );
    let recorded_sql = backend.recorded_sql();
    assert!(
        recorded_sql.contains(&expected_diff_sql),
        "executed diff SQL must be byte-identical to a direct emitter call: {recorded_sql:?}"
    );

    // Refresh: populate the sidecar (a trivial, empty write_group — this
    // leg tests statement byte-identity, not the write/refresh
    // transactionality already covered by
    // `smelt-backend-duckdb`'s own unit tests).
    let empty_write_group = StatementGroup {
        statements: vec![],
        transactional: false,
    };
    smelt_runtime::maintenance_driver::refresh_fingerprint_sidecar(
        &backend,
        "main",
        "smelt.sources.dim_users",
        "main.dim_users",
        &source_key,
        &projection,
        &all_source_columns,
        model_sql,
        &empty_write_group,
    )
    .await
    .expect("sidecar refresh");

    let expected_digest_select = smelt_logical::maintenance::emit::emit_fingerprint_digest_select(
        "main.dim_users",
        &source_key,
        &["name".to_string(), "tier".to_string()],
        MaintenanceDialect::DuckDb,
    );
    let expected_refresh_sql = smelt_state::ddl_duckdb::generate_fingerprint_sidecar_refresh_sql(
        "main",
        "smelt.sources.dim_users",
        &identity,
        &stamp,
        &expected_digest_select,
    );
    let expected_gc_sql = smelt_state::ddl_duckdb::generate_fingerprint_sidecar_gc_sql(
        "main",
        "smelt.sources.dim_users",
        &identity,
        &expected_digest_select,
    );
    let recorded_sql = backend.recorded_sql();
    assert!(
        recorded_sql.contains(&expected_refresh_sql),
        "executed refresh SQL must be byte-identical to a direct emitter/ddl call: {recorded_sql:?}"
    );
    assert!(
        recorded_sql.contains(&expected_gc_sql),
        "executed GC SQL must be byte-identical to a direct emitter/ddl call: {recorded_sql:?}"
    );

    // Run 2: mutate exactly 2 of the 3 rows' projected columns — the diff
    // must report exactly those 2 keys, never the untouched third.
    backend
        .execute_sql("UPDATE main.dim_users SET tier = 'platinum' WHERE id = 1")
        .await
        .expect("mutate row 1");
    backend
        .execute_sql("UPDATE main.dim_users SET name = 'Roberta' WHERE id = 2")
        .await
        .expect("mutate row 2");

    let changed_after_edit =
        smelt_runtime::maintenance_driver::diff_fingerprint_sidecar_changed_keys(
            &backend,
            "main",
            "smelt.sources.dim_users",
            "main.dim_users",
            &source_key,
            &projection,
            &all_source_columns,
            model_sql,
        )
        .await
        .expect("second diff after a targeted edit");
    let mut changed_after_edit_sorted = changed_after_edit;
    changed_after_edit_sorted.sort();
    assert_eq!(
        changed_after_edit_sorted,
        vec!["1".to_string(), "2".to_string()],
        "the diff must report exactly the 2 edited keys, never the untouched third"
    );

    // An edit to a column OUTSIDE the P4 projection (`notes`) must yield an
    // EMPTY changed set once the sidecar reflects that edit's siblings.
    smelt_runtime::maintenance_driver::refresh_fingerprint_sidecar(
        &backend,
        "main",
        "smelt.sources.dim_users",
        "main.dim_users",
        &source_key,
        &projection,
        &all_source_columns,
        model_sql,
        &empty_write_group,
    )
    .await
    .expect("second sidecar refresh");
    backend
        .execute_sql("UPDATE main.dim_users SET notes = 'edited' WHERE id = 3")
        .await
        .expect("mutate row 3's out-of-projection column");
    let changed_out_of_projection =
        smelt_runtime::maintenance_driver::diff_fingerprint_sidecar_changed_keys(
            &backend,
            "main",
            "smelt.sources.dim_users",
            "main.dim_users",
            &source_key,
            &projection,
            &all_source_columns,
            model_sql,
        )
        .await
        .expect("third diff after an out-of-projection edit");
    assert!(
        changed_out_of_projection.is_empty(),
        "an edit outside the P4 projection must never dirty the changed-key set: \
         {changed_out_of_projection:?}"
    );
}

// =============================================================================
// Backbuild statement-parity legs (`crates/smelt-logical/src/backbuild/
// emit.rs`, `docs/outcomes/20260815-definition-delta-migrate/phases/
// 30-plan.md`): the same "executed byte-identical to a direct emitter call"
// proof as the maintenance families above, driven directly through
// `smelt_runtime::definition_delta::{derive_plan, apply_migration}` —
// backbuild's own single dispatch point, mirroring the "drive the single
// dispatch point" rationale
// `recurrence_bound_probe_and_checked_merge_come_from_the_emitters` already
// documents. Plus the same result-equivalence leg (`multiset_equal` against a
// full refresh) the maintenance families carry.
// =============================================================================

/// Shared staging for the three backbuild legs below: writes every model in
/// `models` (each `(name, v1_sql)`), deploys them via a real
/// `execute_project` run so schema tracking records `model_sql`/columns for
/// each, then rewrites `target_model`'s file to `v2_sql`, re-discovers the
/// workspace, and re-derives the migration plan via `definition_delta::
/// derive_plan` — the same single derivation `smelt migrate`/the run gate/
/// `smelt explain` all read. Returns the derived plan and a fresh
/// `RecordingBackend` opened on the same DuckDB file (not yet applied — each
/// leg calls `apply_migration` itself, since the skeleton-change leg applies
/// a hand-built full-refresh plan rather than `derived.plan` itself, whose
/// `statements` are empty for a `SkeletonChange` verdict).
async fn stage_and_migrate(
    target_model: &str,
    models: &[(&str, &str)],
    v2_sql: &str,
) -> (
    smelt_runtime::definition_delta::DerivedPlan,
    RecordingBackend,
    tempfile::TempDir,
) {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project_dir = tmp.path().to_path_buf();
    std::fs::create_dir_all(project_dir.join("models")).unwrap();
    for (name, sql) in models {
        write_model(&project_dir, name, sql);
    }

    let db_path = project_dir.join("run.duckdb");
    let smelt_yml = format!(
        "name: backbuild_statement_parity_test\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: {db}\n    schema: main\ndefault_materialization: table\ntarget: dev\n",
        db = db_path.display()
    );
    std::fs::write(project_dir.join("smelt.yml"), &smelt_yml).unwrap();

    let config = Arc::new(Config::load(&project_dir).expect("load config"));

    // Deploy v1 through a real run so schema tracking records every model's
    // `model_sql` and columns.
    {
        let (db, graph) = build_db_and_graph(&project_dir, &config);
        let backend_slot: Arc<Mutex<Option<Arc<RecordingBackend>>>> = Arc::new(Mutex::new(None));
        let factory = RecordingBackendFactory {
            db_path: db_path.clone(),
            backend: backend_slot,
        };
        execute_project(
            "backbuild-parity-deploy".to_string(),
            make_request("dev", "2024-01-01", "2024-01-02"),
            Arc::clone(&config),
            graph,
            db,
            &project_dir,
            &factory,
            &smelt_runtime::NoOpReporter,
            CancellationToken::new(),
        )
        .await
        .expect("execute_project v1 deploy");
    }

    // Rewrite the target model to v2 and re-discover the workspace.
    write_model(&project_dir, target_model, v2_sql);
    let discovery = ModelDiscovery::new(project_dir.clone(), config.paths.clone());
    let sql_models = discovery.discover_models().expect("discover_models v2");

    let mut db2 = smelt_db::Database::default();
    let project = db2.set_project_input(project_dir.clone(), String::new());
    let source_files: Vec<_> = sql_models
        .iter()
        .map(|m| db2.set_source_file(m.path.clone(), m.content.clone(), project_dir.clone()))
        .collect();
    db2.set_workspace(source_files, vec![project]);
    db2.set_active_target(config.target.clone().map(|t| Arc::from(t.as_str())));

    let target = sql_models
        .iter()
        .find(|m| m.name == target_model)
        .expect("target model discovered")
        .clone();

    let file_store = smelt_state::file_store::FileStore::new(&project_dir, "dev");
    let deployed = file_store
        .load_schema(&target.db_name_owned())
        .expect("load deployed schema")
        .expect("the v1 deploy must have recorded a schema");
    let before_sql_raw = deployed
        .model_sql
        .clone()
        .expect("the v1 deploy must have recorded model_sql");

    let derived = derive_plan(
        &file_store,
        &target,
        &sql_models,
        None,
        &db2,
        &before_sql_raw,
        &deployed.columns,
    )
    .expect("derive_plan");

    let inner = DuckDbBackend::new(&db_path, "main")
        .await
        .expect("open duckdb for migration");
    let backend = RecordingBackend::new(inner);

    (derived, backend, tmp)
}

/// B1 (`Technique::SelfDerivedColumnAdd`): a new column that is a pure
/// function of existing stored columns backfills via `ALTER TABLE ADD
/// COLUMN` + an in-place `UPDATE`, both byte-identical to a direct
/// `emit_alter_add_column`/`emit_in_place_update` call over the plan's own
/// derived inputs — never merely emitter-shaped text.
#[tokio::test]
async fn backbuild_in_place_backfill_statements_come_from_the_emitter() {
    const V1: &str = "---\nmaterialization: table\n---\n\
SELECT id, amount, discount FROM (VALUES (1, 100, 20), (2, 200, 50)) AS t(id, amount, discount)\n";
    const V2: &str = "---\nmaterialization: table\n---\n\
SELECT id, amount, discount, amount - discount AS net_amount FROM (VALUES (1, 100, 20), (2, 200, 50)) AS t(id, amount, discount)\n";

    let (derived, backend, _tmp) = stage_and_migrate("net_orders", &[("net_orders", V1)], V2).await;

    assert_eq!(derived.plan.groups.len(), 1, "{:?}", derived.plan.groups);
    let group = &derived.plan.groups[0];
    assert_eq!(group.verdict, MigrationVerdict::BackfillInPlace);
    assert_eq!(group.options.len(), 1);
    assert_eq!(group.options[0].technique, Technique::SelfDerivedColumnAdd);

    let sql_type = derived
        .inputs
        .added_column_types
        .get("net_amount")
        .expect("net_amount type inferred")
        .clone();
    let expected_alter = emit_alter_add_column(&derived.inputs.table, "net_amount", &sql_type);
    let expected_update = emit_in_place_update(
        &derived.inputs.table,
        &[("net_amount".to_string(), "amount - discount".to_string())],
    );
    let expected_statements = vec![expected_alter, expected_update];
    assert_eq!(group.options[0].statements, expected_statements);
    assert_eq!(derived.plan.statements, expected_statements);

    apply_migration(&backend, &derived.plan)
        .await
        .expect("apply_migration");
    assert_eq!(
        backend.recorded_sql(),
        expected_statements,
        "executed SQL must be byte-identical to a direct emitter call over the plan's own inputs"
    );

    assert!(
        multiset_equal(
            &backend,
            &format!("SELECT * FROM {}", derived.inputs.table),
            "SELECT id, amount, discount, amount - discount AS net_amount FROM (VALUES \
             (1, 100, 20), (2, 200, 50)) AS t(id, amount, discount)"
        )
        .await,
        "the backfill statements must reproduce a full refresh of the after-definition"
    );
}

/// A skeleton (grain) change admits no in-place backfill technique
/// (`MigrationVerdict::SkeletonChange`) — the only honest route is the
/// always-present model-level `FullRefresh` baseline, byte-identical to a
/// direct `emit_full_refresh` call.
#[tokio::test]
async fn backbuild_full_refresh_statement_comes_from_the_emitter() {
    const V1: &str = "---\nmaterialization: table\n---\n\
SELECT id, amount, discount FROM (VALUES (1, 100, 20), (2, 200, 50)) AS t(id, amount, discount)\n";
    const V2_SKELETON_CHANGE: &str = "---\nmaterialization: table\n---\n\
SELECT id, amount, discount, count(*) AS n FROM (VALUES (1, 100, 20), (2, 200, 50)) AS t(id, amount, discount) GROUP BY id, amount, discount\n";

    let (derived, backend, _tmp) =
        stage_and_migrate("net_orders", &[("net_orders", V1)], V2_SKELETON_CHANGE).await;

    assert_eq!(derived.plan.groups.len(), 1, "{:?}", derived.plan.groups);
    assert_eq!(
        derived.plan.groups[0].verdict,
        MigrationVerdict::SkeletonChange
    );
    assert!(
        derived.plan.groups[0].options.is_empty(),
        "a skeleton change admits no targeted technique: {:?}",
        derived.plan.groups[0].options
    );

    let expected_full_refresh = emit_full_refresh(&derived.inputs.table, &derived.inputs.after_sql);
    assert_eq!(
        derived.plan.full_refresh.statements,
        vec![expected_full_refresh.clone()]
    );

    // The caller (not `derive_plan`) is the one that decides to fall back to
    // the full-refresh option on a `SkeletonChange` verdict — build that
    // plan explicitly, the same shape `apply_migration_executes_plan_
    // statements_in_order` (`crates/smelt-runtime/src/definition_delta.rs`)
    // hand-builds.
    let full_refresh_plan = MigrationPlan {
        model: derived.plan.model.clone(),
        table: derived.plan.table.clone(),
        groups: vec![],
        full_refresh: derived.plan.full_refresh.clone(),
        statements: derived.plan.full_refresh.statements.clone(),
    };
    apply_migration(&backend, &full_refresh_plan)
        .await
        .expect("apply_migration");
    assert_eq!(
        backend.recorded_sql(),
        vec![expected_full_refresh],
        "executed SQL must be byte-identical to a direct emit_full_refresh call"
    );

    assert!(
        multiset_equal(
            &backend,
            &format!(
                "SELECT id, amount, discount, n FROM {}",
                derived.inputs.table
            ),
            "SELECT id, amount, discount, count(*) AS n FROM (VALUES (1, 100, 20), (2, 200, 50)) \
             AS t(id, amount, discount) GROUP BY id, amount, discount"
        )
        .await,
        "the full-refresh statement must reproduce a full refresh of the after-definition"
    );
}

/// B3 (`Technique::UpstreamPullthrough`): an added column that pulls through
/// an upstream already in the FROM tree, bound via the upstream's declared
/// `unique_key`, backfills via `ALTER TABLE ADD COLUMN` + a column-scoped
/// `UPDATE ... FROM`, byte-identical to a direct `emit_alter_add_column`/
/// `emit_column_backfill_update_from` call.
#[tokio::test]
async fn backbuild_upstream_backfill_statements_come_from_the_emitter() {
    const CUSTOMERS: &str = "---\nmaterialization: table\nunique_key:\n  - customer_id\n---\n\
SELECT customer_id, name FROM (VALUES (1, 'Alice'), (2, 'Bob')) AS t(customer_id, name)\n";
    const ORDERS_V1: &str = "---\nmaterialization: table\n---\n\
SELECT o.order_id AS order_id, o.customer_id AS customer_id, \
customers.customer_id AS customers_customer_id \
FROM (VALUES (1, 1), (2, 2)) AS o(order_id, customer_id) \
JOIN smelt.customers AS customers ON o.customer_id = customers.customer_id\n";
    const ORDERS_V2: &str = "---\nmaterialization: table\n---\n\
SELECT o.order_id AS order_id, o.customer_id AS customer_id, \
customers.customer_id AS customers_customer_id, customers.name AS customer_name \
FROM (VALUES (1, 1), (2, 2)) AS o(order_id, customer_id) \
JOIN smelt.customers AS customers ON o.customer_id = customers.customer_id\n";

    let (derived, backend, _tmp) = stage_and_migrate(
        "orders",
        &[("customers", CUSTOMERS), ("orders", ORDERS_V1)],
        ORDERS_V2,
    )
    .await;

    assert_eq!(derived.plan.groups.len(), 1, "{:?}", derived.plan.groups);
    let group = &derived.plan.groups[0];
    assert_eq!(group.verdict, MigrationVerdict::Rederive);
    assert_eq!(group.options.len(), 1);
    assert_eq!(group.options[0].technique, Technique::UpstreamPullthrough);

    let sql_type = derived
        .inputs
        .added_column_types
        .get("customer_name")
        .expect("customer_name type inferred")
        .clone();
    let expected_alter = emit_alter_add_column(&derived.inputs.table, "customer_name", &sql_type);
    let expected_update = emit_column_backfill_update_from(
        &derived.inputs.table,
        &[("customer_name".to_string(), "u.name".to_string())],
        "customers",
        "u",
        &[(
            "customers_customer_id".to_string(),
            "customer_id".to_string(),
        )],
    );
    let expected_statements = vec![expected_alter, expected_update];
    assert_eq!(group.options[0].statements, expected_statements);
    assert_eq!(derived.plan.statements, expected_statements);

    apply_migration(&backend, &derived.plan)
        .await
        .expect("apply_migration");
    assert_eq!(
        backend.recorded_sql(),
        expected_statements,
        "executed SQL must be byte-identical to a direct emitter call over the plan's own inputs"
    );

    assert!(
        multiset_equal(
            &backend,
            &format!("SELECT * FROM {}", derived.inputs.table),
            "SELECT o.order_id AS order_id, o.customer_id AS customer_id, \
             customers.customer_id AS customers_customer_id, customers.name AS customer_name \
             FROM (VALUES (1, 1), (2, 2)) AS o(order_id, customer_id) \
             JOIN customers ON o.customer_id = customers.customer_id"
        )
        .await,
        "the backfill statements must reproduce a full refresh of the after-definition"
    );
}

/// The default `retain_departed` point's runtime half (`docs/outcomes/
/// 20260815-definition-delta-migrate/phases/32b-plan.md`): a snapshot-
/// reconcile keyed run's executed statements are exactly `emit_keyed_fold`
/// + `emit_departed_key_delete`, sent as one `transactional: true`
/// `StatementGroup` — and the post-run table is multiset-equal to a full
/// refresh of the new source (the departed key is gone from both).
#[tokio::test]
async fn snapshot_reconcile_delete_leg_parity() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path();
    std::fs::create_dir_all(project_dir.join("models/sources")).unwrap();

    std::fs::write(
        project_dir.join("models/sources/devices.yml"),
        "description: Raw per-device rows, no clock.\n\
         columns:\n\
         \x20\x20- name: device_id\n\
         \x20\x20\x20\x20type: INTEGER\n\
         \x20\x20- name: amount\n\
         \x20\x20\x20\x20type: DOUBLE\n\
         mutation_profile:\n\
         \x20\x20kind: mutable_snapshot\n",
    )
    .unwrap();

    write_model(
        project_dir,
        "device_snapshot",
        "---\n\
         materialization: table\n\
         refresh: incremental\n\
         grain: key\n\
         maintenance:\n\
         \x20\x20scan_bounds:\n\
         \x20\x20\x20\x20per_source:\n\
         \x20\x20\x20\x20\x20\x20devices:\n\
         \x20\x20\x20\x20\x20\x20\x20\x20allow_full_scan: true\n\
         ---\n\
         SELECT device_id, ANY_VALUE(amount) AS amount FROM smelt.sources.devices GROUP BY 1",
    );

    let db_path = project_dir.join("run.duckdb");
    let smelt_yml = format!(
        "name: statement_parity_departed_key_test\nversion: 1\npaths:\n  - models\ntargets:\n  \
         dev:\n    type: duckdb\n    database: {db}\n    schema: main\n\
         default_materialization: table\ntarget: dev\n",
        db = db_path.display()
    );
    std::fs::write(project_dir.join("smelt.yml"), &smelt_yml).unwrap();

    let config = Arc::new(Config::load(project_dir).expect("load config"));

    let mut request = make_request("dev", "2024-01-01", "2024-01-01");
    request.start = None;
    request.end = None;

    // Run 1: table does not exist — the create path, no delete leg to prove.
    {
        let (db, graph) = build_db_and_graph(project_dir, &config);
        let conn = duckdb::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE SCHEMA IF NOT EXISTS main; \
             CREATE OR REPLACE TABLE main.sources_devices AS \
             SELECT * FROM (VALUES (1, 10.0), (2, 5.0)) AS t(device_id, amount);",
        )
        .unwrap();
        drop(conn);

        let backend_slot = Arc::new(Mutex::new(None));
        let factory = RecordingBackendFactory {
            db_path: db_path.clone(),
            backend: Arc::clone(&backend_slot),
        };
        execute_project(
            "snapshot-reconcile-parity-run-1".to_string(),
            request.clone(),
            Arc::clone(&config),
            Arc::clone(&graph),
            Arc::clone(&db),
            project_dir,
            &factory,
            &NO_OP_REPORTER,
            CancellationToken::new(),
        )
        .await
        .expect("first run creates the table");
    }

    // Device 2 departs the source.
    {
        let conn = duckdb::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE OR REPLACE TABLE main.sources_devices AS \
             SELECT * FROM (VALUES (1, 10.0)) AS t(device_id, amount);",
        )
        .unwrap();
    }

    let (db, graph) = build_db_and_graph(project_dir, &config);
    let backend_slot = Arc::new(Mutex::new(None));
    let factory = RecordingBackendFactory {
        db_path: db_path.clone(),
        backend: Arc::clone(&backend_slot),
    };
    execute_project(
        "snapshot-reconcile-parity-run-2".to_string(),
        request,
        Arc::clone(&config),
        Arc::clone(&graph),
        Arc::clone(&db),
        project_dir,
        &factory,
        &NO_OP_REPORTER,
        CancellationToken::new(),
    )
    .await
    .expect("reconcile run deletes the departed key");

    let backend = backend_slot
        .lock()
        .unwrap()
        .take()
        .expect("backend recorded");
    let groups = backend.recorded_groups();
    let reconcile_group = groups
        .iter()
        .find(|g| g.statements.iter().any(|s| s.sql.starts_with("MERGE INTO")))
        .expect("reconcile run must execute via execute_statement_group");

    assert!(
        reconcile_group.transactional,
        "the merge + departed-key delete must execute as one transactional group"
    );

    // Recover the compiler's own delta SELECT (type-cast-wrapped, with its
    // header comment) from the executed merge text — the same "read the
    // embedded relation back" approach `extract_affected_keys_select` above
    // uses for the repair family, since the compiled SQL a real run embeds
    // is not byte-reconstructable from the model's source text alone.
    let merge_sql = &reconcile_group.statements[0].sql;
    let using_marker = "USING (";
    let delta_start = merge_sql.find(using_marker).expect("USING clause") + using_marker.len();
    let delta_end_marker = ") AS delta ON";
    let delta_end = merge_sql.rfind(delta_end_marker).expect("delta alias");
    let delta_select = &merge_sql[delta_start..delta_end];
    let expected_merge = emit_keyed_fold_suppressed(
        "main.device_snapshot",
        &["device_id".to_string()],
        &[("amount".to_string(), "delta.amount".to_string())],
        delta_select,
        None,
        &["amount".to_string()],
        MaintenanceDialect::DuckDb,
    );
    let expected_delete = smelt_logical::contract::retain_departed::reconcile_disposition(None);
    assert_eq!(
        expected_delete,
        smelt_logical::contract::retain_departed::DepartedKeyDisposition::Delete,
        "sanity: undeclared retain_departed resolves to the default delete point"
    );
    let expected_delete_stmt = emit_departed_key_delete(
        "main.device_snapshot",
        &["device_id".to_string()],
        delta_select,
        MaintenanceDialect::DuckDb,
    );

    assert_eq!(
        reconcile_group.statements.len(),
        2,
        "expected exactly the merge and the delete, got: {:#?}",
        reconcile_group.statements
    );
    assert_eq!(
        reconcile_group.statements[0], expected_merge.statements[0],
        "executed merge must be byte-identical to a direct emit_keyed_fold call"
    );
    assert_eq!(
        reconcile_group.statements[1], expected_delete_stmt,
        "executed delete must be byte-identical to a direct emit_departed_key_delete call"
    );

    assert!(
        multiset_equal(
            &*backend,
            "SELECT device_id, amount FROM main.device_snapshot",
            delta_select,
        )
        .await,
        "the post-run table must be multiset-equal to a full refresh of the new source"
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
/// `Backend::insert_overwrite` — DELETE/INSERT-OVERWRITE SQL that predates
/// `incremental_models.md`'s single-owner emitters entirely. `IncrementalStrategy`
/// has one dispatchable variant, `DeleteInsert`; `smelt_runtime::
/// maintenance_driver::resolve_incremental_strategy` and the batch loop's
/// own dispatch (`crates/smelt-runtime/src/execute.rs`) only ever resolve it.
/// `insert_into_from_query`/`insert_overwrite` remain on the `Backend` trait
/// as the capability that would admit an append-only or overwrite strategy
/// once plan derivation selects one; no plan derivation calls them today.
/// Routing this hand-authored SQL through `emit_delete_insert` too, closing
/// the remaining gap, is out of Phase 4's file scope (`docs/plans/
/// 20260710-emit-unification.md` Phase 4 "Critical files" — the backend
/// crates are not listed); tracked as follow-up, not fixed here.
const STATEMENT_AUTHORING_ALLOWLIST: &[(&str, &str)] = &[
    (
        "smelt-backend-duckdb/src/lib.rs",
        "DELETE FROM {} WHERE {} >= {} AND {} < {}",
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
        "DELETE FROM {} WHERE {} >= {} AND {} < {}",
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
            || line.contains("CREATE TEMP TABLE ")
            // The backbuild family (`crates/smelt-logical/src/backbuild/
            // emit.rs`): `ALTER TABLE ` covers B1/B2/B3's `ADD`/`RENAME` and
            // C1's `DROP` (no other production code in the scanned crates
            // issues a bare `ALTER TABLE ` DDL string); `CREATE OR REPLACE
            // TABLE ` is the always-present model-level `FullRefresh`
            // baseline, distinct from the region family's own qualified
            // `CREATE TABLE {}.{} AS` shape above; `__backbuild_diff` is the
            // derived-table alias `emit_difference_insert` (E2/E4) wraps its
            // own `after_sql` argument in — a marker string with no
            // legitimate production match anywhere outside that one
            // authoring site, representative of the in-place-UPDATE/
            // difference-INSERT half of the family the way `CREATE TEMP
            // TABLE ` is representative of the staged-candidate shape above
            // (not every backbuild statement shape has an equally unique
            // marker; this one does, and catching a stray copy of it is
            // enough to catch a re-authored difference/branch INSERT).
            || line.contains("ALTER TABLE ")
            || line.contains("CREATE OR REPLACE TABLE ")
            || line.contains("__backbuild_diff");
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

/// `src/`-relative file paths excluded from the scan entirely: the two
/// maintenance/backbuild single-owner emitter modules
/// (`docs/specs/architecture.md` §"Constraints & Invariants" item 12 —
/// every maintenance/backbuild statement is the output of a pure emitter in
/// one of these two files; scanning them for the shapes they themselves
/// author would be circular), plus `smelt-state`'s three per-dialect
/// schema-evolution DDL modules. Schema-evolution DDL is declared a
/// *separate* single-owner family, outside the maintenance/backbuild
/// emitter rule (`docs/specs/incremental_models.md` §"Statement emission
/// (single owner)"): it is multi-dialect and covers struct/nested/
/// nullability operations the backbuild emitters have no forms for, and
/// `smelt-state` sits below `smelt-logical`, so it cannot call into
/// `backbuild::emit`. `ddl_duckdb.rs` is the actual per-dialect renderer
/// owner; `ddl_spark.rs`/`ddl_bigquery.rs` are excluded on the same
/// per-dialect-owner basis even though their DDL shapes (backtick-quoted
/// identifiers, `ADD COLUMNS (...)`, `SET DATA TYPE`) don't match this
/// scan's DuckDB-flavored `ALTER TABLE `/`UPDATE ` shapes anyway.
const EMITTER_MODULE_EXCLUSIONS: &[&str] = &[
    "smelt-logical/src/maintenance/emit.rs",
    "smelt-logical/src/backbuild/emit.rs",
    "smelt-state/src/ddl_duckdb.rs",
    "smelt-state/src/ddl_spark.rs",
    "smelt-state/src/ddl_bigquery.rs",
];

fn is_emitter_module(path: &Path) -> bool {
    let normalized = path.to_string_lossy().replace('\\', "/");
    EMITTER_MODULE_EXCLUSIONS
        .iter()
        .any(|suffix| normalized.ends_with(suffix))
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
            if is_emitter_module(&path) {
                continue;
            }
            scan_statement_authoring_file(&path, hits);
        }
    }
}

/// Structural gate: `DELETE FROM`/`MERGE INTO`/`CREATE TABLE {}.{} AS`-shaped
/// statement text must not be constructed anywhere in `smelt-backend*/src`,
/// `smelt-runtime/src`, or `smelt-logical/src` production code outside the
/// two single-owner emitter modules
/// (`crates/smelt-logical/src/maintenance/emit.rs`,
/// `crates/smelt-logical/src/backbuild/emit.rs` —
/// [`EMITTER_MODULE_EXCLUSIONS`], excluded rather than unscanned entirely so
/// a *new* statement-shaped file dropped anywhere else in `smelt-logical`
/// is still caught). `smelt-logical` joined the scan in
/// `docs/plans/20260808-substrate-unification.md` ("emitter unification and
/// gate extension") — the no-authoring rule already applied crate-wide in
/// spec (`docs/specs/architecture.md` §"Constraints & Invariants" item 12:
/// "backends execute, never author"), this widens the structural gate to
/// match. Backends execute emitted `StatementGroup`s
/// (`Backend::execute_statement_group`); they never author
/// maintenance-statement text of their own
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
        "smelt-logical",
        "smelt-state",
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

/// The append-only posture probe's dispatch site
/// (`smelt_runtime::source_probes::dispatch_and_record_append_only_postures`)
/// must execute SQL byte-identical to a direct
/// `emit_append_only_posture_probe`/`emit_append_only_baseline_snapshot`
/// call over the same inputs (`docs/outcomes/20260809-probe-backed-facts/
/// outcome.md` phase 6). This drives `dispatch_and_record_append_only_
/// postures` directly against a [`RecordingBackend`] rather than the full
/// `execute_project` pipeline — the same rationale
/// `recurrence_bound_probe_and_checked_merge_come_from_the_emitters` gives:
/// this driver is the single point every append-only posture probe and
/// baseline-refresh statement flows through, so calling it directly still
/// proves *executed* SQL matches the emitter's output, without needing a
/// full staged workspace to reach this one call site twice (once via a
/// full-refresh model, once via an incremental batch).
#[tokio::test]
async fn append_only_posture_probe_and_baseline_snapshot_come_from_the_emitters() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let db_path = tmp.path().join("run.duckdb");
    {
        let conn = duckdb::Connection::open(&db_path).expect("open duckdb");
        conn.execute_batch(
            "CREATE SCHEMA IF NOT EXISTS raw;\n\
             CREATE TABLE raw.events (event_date DATE, payload TEXT);\n\
             INSERT INTO raw.events VALUES (DATE '2026-01-01', 'a'), (DATE '2026-01-02', 'b');",
        )
        .expect("stage raw.events");
    }
    let inner = DuckDbBackend::new(&db_path, "main")
        .await
        .expect("open backend");
    let backend = RecordingBackend::new(inner);

    let parse = smelt_parser::parse("SELECT * FROM smelt.sources.raw.events");
    let refs = smelt_parser::ast::File::cast(parse.syntax())
        .map(|f| smelt_core::extract_refs(&f))
        .unwrap_or_default();
    let model_path = std::path::PathBuf::from("models/m.sql");
    let model = smelt_core::ModelFile {
        name: "m".to_string(),
        path: model_path.clone(),
        content: "SELECT * FROM smelt.sources.raw.events".to_string(),
        refs,
        parse_errors: Vec::new(),
        metadata: None,
        kind: smelt_core::ModelKind::Sql,
        model_id: smelt_core::ModelId::from_path(model_path),
        address_segments: vec!["m".to_string()],
    };
    let source = smelt_core::sources::SourceInfo {
        path: std::path::PathBuf::from("/tmp/fake.yml"),
        address_segments: vec![
            "sources".to_string(),
            "raw".to_string(),
            "events".to_string(),
        ],
        columns: vec![smelt_core::sources::SourceColumn {
            name: "payload".to_string(),
            data_type: smelt_types::DataType::Text,
            nullable: true,
            description: None,
        }],
        description: None,
        name_override: Some(smelt_core::sources::SourceNameOverride::Literal(
            "raw.events".to_string(),
        )),
        tags: vec![],
        timeseries: Some(smelt_core::config::TimeseriesConfig {
            event_time_column: "event_date_ts".to_string(),
            partition_column: "event_date".to_string(),
            granularity: smelt_core::config::Granularity::Day,
            week_start: None,
            assert_monotonic: false,
        }),
        mutation_profile: Some(smelt_core::sources::SourceMutationProfile::from_kind(
            smelt_core::sources::MutationProfile::AppendOnly,
        )),
        source_lateness: None,
        watermark: None,
        unique_key: None,
        retention: None,
        referential_integrity: None,
    };

    let mut baselines = smelt_state::source_postures::SourcePostureStore::default();
    baselines.record(
        "raw.events",
        vec![
            smelt_state::source_postures::SourcePosturePartition {
                partition_value: "2026-01-01".to_string(),
                recorded_count: 1,
                recorded_fingerprint: "not-the-real-fingerprint".to_string(),
            },
            smelt_state::source_postures::SourcePosturePartition {
                partition_value: "2026-01-02".to_string(),
                recorded_count: 1,
                recorded_fingerprint: "irrelevant-for-the-open-partition".to_string(),
            },
        ],
    );

    let probes = smelt_runtime::source_probes::append_only_posture_probes(
        "m",
        "m creation",
        &model,
        &[source],
        &baselines,
        "dev",
        "raw",
        MaintenanceDialect::DuckDb,
    );
    assert_eq!(probes.len(), 1);

    // Direct emitter calls over the exact same inputs the probe builder used.
    let expected_probe_sql = emit_append_only_posture_probe(
        "raw.events",
        "event_date",
        &["payload".to_string()],
        &[
            smelt_logical::maintenance::emit::AppendOnlyBaselinePartition {
                partition_value: "2026-01-01".to_string(),
                recorded_count: 1,
                recorded_fingerprint: "not-the-real-fingerprint".to_string(),
                check_fingerprint: true,
            },
            smelt_logical::maintenance::emit::AppendOnlyBaselinePartition {
                partition_value: "2026-01-02".to_string(),
                recorded_count: 1,
                recorded_fingerprint: "irrelevant-for-the-open-partition".to_string(),
                check_fingerprint: false,
            },
        ],
        MaintenanceDialect::DuckDb,
    )
    .sql;
    let (probe_sql, snapshot_sql) = match &probes[0].action {
        smelt_runtime::source_probes::SourcePostureAction::Verify { sql, snapshot_sql } => {
            (sql.clone(), snapshot_sql.clone())
        }
        smelt_runtime::source_probes::SourcePostureAction::Establish { .. } => {
            panic!("a recorded baseline must build a Verify action, not Establish")
        }
    };
    assert_eq!(probe_sql, expected_probe_sql);

    let expected_snapshot_sql =
        smelt_logical::maintenance::emit::emit_append_only_baseline_snapshot(
            "raw.events",
            "event_date",
            &["payload".to_string()],
            MaintenanceDialect::DuckDb,
        )
        .sql;
    assert_eq!(snapshot_sql, expected_snapshot_sql);

    // The probe fires (the recorded fingerprint for the closed partition
    // is deliberately wrong) — dispatch fails loud before any snapshot
    // executes, and the ONLY SQL actually run is the probe statement,
    // byte-identical to the direct emitter call.
    let err = smelt_runtime::source_probes::dispatch_and_record_append_only_postures(
        &backend,
        &smelt_runtime::probes::ProbePolicy::per_run(),
        &probes,
    )
    .await
    .expect_err("the mismatched closed-partition fingerprint must fail loud");
    assert!(err.to_string().contains("SourceMutationProfileViolated"));

    let executed = backend.recorded_sql();
    assert_eq!(
        executed,
        vec![expected_probe_sql.clone()],
        "the dispatch site must execute exactly the emitted probe SQL, nothing more"
    );
}

/// The mutation-happened discrimination gate
/// (`smelt_runtime::mutation_probe::gate_upstream_mutation_dispatch`) must
/// execute SQL byte-identical to a direct `emit_source_mutation_fingerprint`
/// call over the same inputs (`docs/specs/incremental_models.md` §"When a
/// mutation cell dispatches") — the statement-emission single-owner rule
/// (`CLAUDE.md` §"Maintenance-plan purity").
#[tokio::test]
async fn source_mutation_fingerprint_comes_from_the_emitter() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let db_path = tmp.path().join("run.duckdb");
    {
        let conn = duckdb::Connection::open(&db_path).expect("open duckdb");
        conn.execute_batch(
            "CREATE SCHEMA IF NOT EXISTS raw;\n\
             CREATE TABLE raw.dim_users (user_id INTEGER, status TEXT);\n\
             INSERT INTO raw.dim_users VALUES (1, 'active'), (2, 'inactive');",
        )
        .expect("stage raw.dim_users");
    }
    let inner = DuckDbBackend::new(&db_path, "main")
        .await
        .expect("open backend");
    let backend = RecordingBackend::new(inner);

    let digest_columns = vec!["user_id".to_string(), "status".to_string()];
    let expected_sql = emit_source_mutation_fingerprint(
        "raw.dim_users",
        &digest_columns,
        MaintenanceDialect::DuckDb,
    )
    .sql;

    let (verdict, refreshed) = smelt_runtime::mutation_probe::gate_upstream_mutation_dispatch(
        &backend,
        "m",
        "raw.dim_users",
        "raw.dim_users",
        &digest_columns,
        MaintenanceDialect::DuckDb,
        None,
    )
    .await
    .expect("gate must succeed against a live backend");

    assert_eq!(
        verdict,
        smelt_runtime::mutation_probe::MutationVerdict::Dispatch,
        "no recorded baseline must always dispatch"
    );
    assert_eq!(refreshed.recorded_count, 2);
    assert_eq!(refreshed.digest_columns, digest_columns);

    let executed = backend.recorded_sql();
    assert_eq!(
        executed,
        vec![expected_sql],
        "the gate must execute exactly the emitted fingerprint SQL, nothing more"
    );

    // A second gate call against the SAME baseline (nothing changed) must
    // observe the identical fingerprint and report NoOp.
    let (verdict2, _refreshed2) = smelt_runtime::mutation_probe::gate_upstream_mutation_dispatch(
        &backend,
        "m",
        "raw.dim_users",
        "raw.dim_users",
        &digest_columns,
        MaintenanceDialect::DuckDb,
        Some(&refreshed),
    )
    .await
    .expect("gate must succeed against a live backend");
    assert_eq!(
        verdict2,
        smelt_runtime::mutation_probe::MutationVerdict::NoOp
    );
}

// =============================================================================
// State residency (`docs/outcomes/20260904-state-residency/outcome.md`
// criterion 1): the reconciliation ledger's region-recompute reset shares
// ONE backend transaction with the write it protects — proven directly
// against `DuckDbBackend::execute_write_with_bookkeeping` rather than
// through the full `execute_project` pipeline, since provoking a mid-batch
// write failure through the real pipeline has no clean seam.
// =============================================================================

/// A valid ledger reset as `pre_write_sqls`, paired with a deliberately
/// invalid write `StatementGroup`: the call must error, and `_smelt_ledger`
/// must hold no row for the region the failed write never actually wrote —
/// proving "same transaction as the maintained write", not merely "runs
/// alongside it".
#[tokio::test]
async fn ledger_reset_rolls_back_with_a_failed_write() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("test.duckdb");
    let backend = DuckDbBackend::new(&db_path, "main")
        .await
        .expect("open duckdb");

    let ensure_sqls = vec![smelt_state::ddl_duckdb::generate_ledger_table_ddl("main")];
    let pre_write_sqls = smelt_state::ddl_duckdb::generate_ledger_recompute_reset_sqls(
        "main",
        "rollback_model",
        "{*}",
        "2026-08-01",
        "2026-08-02",
        "self",
        "2026-08-02",
    );
    let write_group = StatementGroup {
        statements: vec![smelt_backend::MaintenanceStatement {
            sql: "INSERT INTO main.does_not_exist VALUES (1)".to_string(),
        }],
        transactional: false,
    };

    let result = backend
        .execute_write_with_bookkeeping(&ensure_sqls, &pre_write_sqls, &write_group)
        .await;
    assert!(
        result.is_err(),
        "the failed write must surface an error, not silently swallow it"
    );

    // The ensure DDL is idempotent DDL run OUTSIDE the transaction (same
    // precedent as `Backend::fold_ledger_delta`'s `ensure_sql`), so the
    // table exists even after the rollback; the query below proves it holds
    // no row, not that it's absent.
    let rows = backend
        .execute_sql("SELECT COUNT(*) FROM main._smelt_ledger WHERE model_name = 'rollback_model'")
        .await
        .expect("query ledger row count");
    let batch = rows.first().expect("COUNT returns one row");
    let count = batch
        .column(0)
        .as_any()
        .downcast_ref::<Int64Array>()
        .expect("COUNT column is Int64")
        .value(0);
    assert_eq!(
        count, 0,
        "a failed write must leave no reconciliation-ledger reset row behind"
    );
}

/// A non-DuckDB dialect's DeleteInsert batch write emits no `_smelt_ledger`
/// SQL at all — the skip is now driven by the run's resolved
/// `StateAvailability` (`docs/outcomes/20260904-state-residency/
/// outcome.md` phase 5), not a raw `backend.dialect() == DuckDB` check.
/// The old `RunReporter` stand-in method for this skip is retired entirely
/// (phase 6): the affected cell's own recorded `MaintenanceStateDowngraded` is the
/// user-visible channel now, surfaced by `smelt explain`
/// (`crates/smelt-cli/tests/explain_maintenance.rs`) — this test asserts
/// only the emitted-statement set, which is the half this crate owns.
/// Uses a fully mocked `Backend` (never a real connection) so the dialect
/// mismatch between the claimed `SqlDialect::SparkSQL` and no real Spark
/// engine can never itself cause a spurious failure — this test is about
/// which SQL gets BUILT, not whether it executes against a live warehouse.
#[tokio::test]
async fn ledger_reset_is_skipped_on_a_non_duckdb_dialect() {
    struct NonDuckDbBackend {
        calls: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl Backend for NonDuckDbBackend {
        async fn execute_sql(&self, sql: &str) -> Result<Vec<RecordBatch>, BackendError> {
            self.calls.lock().unwrap().push(sql.to_string());
            Ok(vec![])
        }
        async fn create_table_as(
            &self,
            _schema: &str,
            _name: &str,
            sql: &str,
        ) -> Result<(), BackendError> {
            self.calls.lock().unwrap().push(sql.to_string());
            Ok(())
        }
        async fn create_view_as(
            &self,
            _schema: &str,
            _name: &str,
            _sql: &str,
        ) -> Result<(), BackendError> {
            unreachable!()
        }
        async fn drop_table_if_exists(
            &self,
            _schema: &str,
            _name: &str,
        ) -> Result<(), BackendError> {
            Ok(())
        }
        async fn drop_view_if_exists(
            &self,
            _schema: &str,
            _name: &str,
        ) -> Result<(), BackendError> {
            Ok(())
        }
        async fn get_row_count(&self, _schema: &str, _name: &str) -> Result<usize, BackendError> {
            Ok(0)
        }
        async fn get_preview(
            &self,
            _schema: &str,
            _name: &str,
            _limit: usize,
        ) -> Result<Vec<RecordBatch>, BackendError> {
            Ok(vec![])
        }
        async fn table_exists(&self, _schema: &str, _name: &str) -> Result<bool, BackendError> {
            Ok(true)
        }
        async fn ensure_schema(&self, _schema: &str) -> Result<(), BackendError> {
            Ok(())
        }
        fn dialect(&self) -> SqlDialect {
            SqlDialect::SparkSQL
        }
        fn capabilities(&self) -> BackendCapabilities {
            BackendCapabilities::spark()
        }
        async fn load_table(
            &self,
            _schema: &str,
            _name: &str,
            _arrow_schema: SchemaRef,
            _batches: Vec<RecordBatch>,
        ) -> Result<(), BackendError> {
            unreachable!()
        }
        async fn delete_partitions(
            &self,
            _schema: &str,
            _name: &str,
            _partition: &PartitionRange,
        ) -> Result<(), BackendError> {
            unreachable!()
        }
        async fn insert_into_from_query(
            &self,
            _schema: &str,
            _name: &str,
            _sql: &str,
        ) -> Result<(), BackendError> {
            unreachable!()
        }
        async fn insert_overwrite(
            &self,
            _schema: &str,
            _table: &str,
            _sql: &str,
            _partition: &PartitionRange,
        ) -> Result<(), BackendError> {
            unreachable!()
        }
    }

    struct NonDuckDbFactory {
        calls: Arc<Mutex<Vec<String>>>,
    }
    impl BackendFactory for NonDuckDbFactory {
        fn create<'a>(
            &'a self,
            _target_name: &'a str,
            _target_config: &'a Target,
            _project_dir: &'a Path,
        ) -> BackendFuture<'a> {
            let calls = Arc::clone(&self.calls);
            Box::pin(async move { Ok(Box::new(NonDuckDbBackend { calls }) as Box<dyn Backend>) })
        }
    }

    let tmp = tempfile::tempdir().expect("tempdir");
    let project_dir = tmp.path();
    std::fs::create_dir_all(project_dir.join("models")).unwrap();
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
         SELECT * FROM (VALUES (DATE '2024-01-01', 10)) AS t(event_date, amount)",
    );
    // `type: spark` (`docs/outcomes/20260904-state-residency/outcome.md`
    // phase 5): the run's availability resolution reads the target's
    // *declared* dialect from `smelt.yml`
    // (`sql_dialect_for_target`/`availability_for_run`), never the mocked
    // backend's own `dialect()` claim — so this fixture's target type must
    // itself say `spark` for the ledger-less skip this test exercises to
    // actually be reached.
    let smelt_yml = "name: ledger_skip_test\nversion: 1\npaths:\n  - models\ntargets:\n  \
                      dev:\n    type: spark\n    schema: main\n\
                      default_materialization: table\ntarget: dev\n";
    std::fs::write(project_dir.join("smelt.yml"), smelt_yml).unwrap();

    let config = Arc::new(Config::load(project_dir).expect("load config"));
    let (db, graph) = build_db_and_graph(project_dir, &config);

    let factory = NonDuckDbFactory {
        calls: Arc::new(Mutex::new(Vec::new())),
    };
    let calls_handle = Arc::clone(&factory.calls);
    execute_project(
        "ledger-skip-run".to_string(),
        make_request("dev", "2024-01-01", "2024-01-02"),
        Arc::clone(&config),
        graph,
        db,
        project_dir,
        &factory,
        &NO_OP_REPORTER,
        CancellationToken::new(),
    )
    .await
    .expect("a run over a non-DuckDB backend must still succeed");

    let calls = calls_handle.lock().unwrap();
    assert!(
        !calls.iter().any(|c| c.contains("_smelt_ledger")),
        "a non-DuckDB dialect must emit no ledger-reset SQL at all: {calls:?}"
    );
}
