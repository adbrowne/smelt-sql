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
//! `docs/plans/20260710-emit-unification.md` Phases 1–3 — the repair
//! family's per-group recompute (`Technique::PerGroupRecompute`,
//! `docs/specs/incremental_models.md` §"The repair family"), and the
//! succession-patch family (`Technique::SuccessionPatch`) — both the
//! window-forward patch loop and its `--full-refresh` rebuild counterpart
//! (`docs/outcomes/20260906-scd2-keyed-succession/phases/05c-plan.md`).
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
        rebuild: false,
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

mod column_scoped_merge;
mod delta_region;
mod fingerprint_backbuild;
mod keyed_fold_pins_and_previews;
mod region_and_keyed_fold;
mod repair_and_key_addressed;
mod staged_candidate_conditional;
mod structural_and_ledger;
mod succession;
