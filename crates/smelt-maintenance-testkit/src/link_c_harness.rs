//! Link-C in-process harness (`docs/research/20260705-property-discovery-loop.md`
//! §2.3, §3a; `docs/plans/20260705-property-discovery-loop.md` phase B / cell
//! `P0-1`). Drives smelt's REAL run pipeline — `smelt_runtime::execute_project`,
//! the sanctioned single entry point (root `CLAUDE.md` §"Run pipeline parity
//! rule") — in-process over a temp DuckDB, with **no hand-injected `WHERE`**.
//! This is the gating deliverable the rest of the property-discovery loop's
//! Link-C cells build on: never use
//! `crates/smelt-cli/tests/incremental/main.rs::run_incremental_sequence` /
//! `execute_model_incremental`, which bypass the analyzer's own bound
//! derivation (`source_bounds::derive_model_bounds`) by injecting the filter
//! by hand — exactly the bug class this loop hunts (design N1).
//!
//! Graduated out of `smelt-cli`'s test target into this standalone dev-only
//! crate so it can be a shared dev-dependency of any consumer's test tree
//! (`smelt-cli`, and `smelt-runtime` where no dependency cycle results)
//! without duplicating the harness
//! (`docs/research/20260705-refresh-as-maintenance-plan/08-code-placement.md`
//! §3, M3).
//!
//! Mirrors the plumbing pattern already proven in
//! `crates/smelt-runtime/tests/execute_parity.rs`, generalised into a reusable
//! `LinkCProject` fixture + a `SqlCapturingReporter` that records each model's
//! fully-resolved compiled SQL (`RunReporter::model_compiled`) so a cell can
//! assert the derived time filter is actually present in what smelt emits —
//! never a filter the test itself injected.

#![allow(dead_code)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::Result;
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
use smelt_runtime::reporter::RunReporter;
use smelt_runtime::types::{ExecuteRequest, RunOutcome};
use smelt_runtime::{execute_project, NoOpReporter};
use tokio_util::sync::CancellationToken;

use crate::recipe::ConformanceTarget;

/// `BackendFactory` that always opens the same on-disk DuckDB file — the
/// harness never needs multi-target dispatch.
pub struct DuckDbBackendFactory {
    pub db_path: PathBuf,
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

/// Captures the fully-resolved SQL `execute_project` compiles for each model,
/// keyed by model name, overwritten per batch (last batch wins) — good enough
/// for cells that just need to see the derived filter clause once per run.
#[derive(Default)]
pub struct SqlCapturingReporter {
    compiled: Mutex<HashMap<String, Vec<String>>>,
}

impl SqlCapturingReporter {
    pub fn new() -> Self {
        Self::default()
    }

    /// All SQL strings compiled for `model`, in batch order (one entry for a
    /// non-batched model; one per incremental batch otherwise).
    pub fn sql_for(&self, model: &str) -> Vec<String> {
        self.compiled
            .lock()
            .unwrap()
            .get(model)
            .cloned()
            .unwrap_or_default()
    }
}

impl RunReporter for SqlCapturingReporter {
    fn model_compiled(&self, _run_id: &str, model: &str, sql: &str) {
        self.compiled
            .lock()
            .unwrap()
            .entry(model.to_string())
            .or_default()
            .push(sql.to_string());
    }
}

/// Wraps a real [`DuckDbBackend`], delegating every call, but recording the
/// [`StatementGroup`] passed to `execute_statement_group` — the single point
/// every emitted maintenance statement flows through on its way to the
/// connection (`docs/specs/incremental_models.md` §"Statement emission
/// (single owner)"). Promoted from
/// `crates/smelt-runtime/tests/statement_parity.rs`'s own private
/// `RecordingBackend` into this shared harness (`docs/outcomes/
/// 20260809-repair-family` phase 8): the repair family's live dispatch
/// (`Technique::PerGroupRecompute`/`RepairWrite::DiffPatch`) does not yet
/// route through `RunReporter::maintenance_statements`
/// ([`SqlCapturingReporter`] only ever observes the compiled model SELECT,
/// never the executed maintenance DML), so a conformance case that needs to
/// see the ACTUAL statements a run sent — not just the fact that it
/// dispatched the right named strategy — needs this lower-level capture
/// channel instead.
pub struct RecordingBackend {
    inner: DuckDbBackend,
    groups: Mutex<Vec<StatementGroup>>,
}

impl RecordingBackend {
    /// Every [`StatementGroup`] this backend executed, in call order.
    pub fn recorded_groups(&self) -> Vec<StatementGroup> {
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

/// `BackendFactory` that opens a fresh [`RecordingBackend`] against
/// `db_path`, stashing an `Arc` handle to it in `backend` (a slot the caller
/// pre-allocates and reads back after the run) — the same
/// callback-into-a-shared-slot shape `statement_parity.rs`'s own
/// `RecordingBackendFactory` uses, needed because `BackendFactory::create`
/// hands ownership of the constructed backend to `execute_project` itself.
struct RecordingBackendFactory {
    db_path: PathBuf,
    backend: Arc<Mutex<Option<Arc<RecordingBackend>>>>,
}

impl BackendFactory for RecordingBackendFactory {
    fn create<'a>(
        &'a self,
        _target_name: &'a str,
        target_config: &'a smelt_core::config::Target,
        _project_dir: &'a Path,
    ) -> BackendFuture<'a> {
        let path = self.db_path.clone();
        let schema = target_config.schema.clone();
        let slot = Arc::clone(&self.backend);
        Box::pin(async move {
            let inner = DuckDbBackend::new(&path, &schema)
                .await
                .map_err(|e| anyhow::anyhow!("DuckDB init failed: {}", e))?;
            let recording = Arc::new(RecordingBackend {
                inner,
                groups: Mutex::new(Vec::new()),
            });
            *slot.lock().unwrap() = Some(Arc::clone(&recording));
            Ok(Box::new(RecordingBackendHandle(recording)) as Box<dyn Backend>)
        })
    }
}

/// Thin `Backend` forwarder so [`RecordingBackendFactory::create`] can hand
/// `execute_project` an owned `Box<dyn Backend>` while the caller keeps its
/// own `Arc<RecordingBackend>` handle alive in the shared slot — delegates
/// every call straight through.
struct RecordingBackendHandle(Arc<RecordingBackend>);

#[async_trait]
impl Backend for RecordingBackendHandle {
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
    async fn execute_statement_group(&self, group: &StatementGroup) -> Result<(), BackendError> {
        self.0.execute_statement_group(group).await
    }
}

/// A staged smelt project directory + the parsed `Config` `execute_project`
/// needs. Re-discovers models from disk each `run` so a between-run source
/// mutation (append/update staged by the caller) is picked up — the whole point
/// of the run-schedule driver (design §3b): the source must be able to change
/// between runs, not be fully pre-populated up front.
/// Whether [`LinkCProject::run`] removes `project_dir/.smelt` before each
/// run — the state-residency outcome's headline claim made executable
/// (`docs/outcomes/20260904-state-residency/phases/09-plan.md`): under
/// [`Self::BetweenRuns`] the equivalence oracle must still hold with no
/// on-disk `.smelt/` continuity between run steps, since the reconciliation
/// ledger and every other correctness structure now live in the engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StateDeletion {
    #[default]
    Retain,
    BetweenRuns,
}

pub struct LinkCProject {
    pub project_dir: PathBuf,
    pub db_path: PathBuf,
    pub config: Arc<Config>,
    pub state_deletion: StateDeletion,
    deletions: Arc<AtomicUsize>,
    nonempty_deletions: Arc<AtomicUsize>,
}

impl LinkCProject {
    /// Load `Config` from an already-staged `project_dir` (models/ + smelt.yml
    /// written by the caller).
    pub fn load(project_dir: PathBuf, db_path: PathBuf) -> Result<Self> {
        let config = Arc::new(Config::load(&project_dir)?);
        Ok(Self {
            project_dir,
            db_path,
            config,
            state_deletion: StateDeletion::Retain,
            deletions: Arc::new(AtomicUsize::new(0)),
            nonempty_deletions: Arc::new(AtomicUsize::new(0)),
        })
    }

    /// Builder: opt this project into removing `project_dir/.smelt` before
    /// every [`Self::run`] call.
    pub fn with_state_deletion(mut self, mode: StateDeletion) -> Self {
        self.state_deletion = mode;
        self
    }

    /// Builder: point this project at a different [`Config`] — used by
    /// callers that clone a loaded project's config to synthesize a
    /// scratch target (e.g. `smelt-cli/tests/bakeoff_seam.rs`).
    pub fn with_config(mut self, config: Arc<Config>) -> Self {
        self.config = config;
        self
    }

    /// Total number of runs that found (and removed) an on-disk `.smelt/`
    /// directory, empty or not.
    pub fn deletions_observed(&self) -> usize {
        self.deletions.load(Ordering::SeqCst)
    }

    /// Number of removed `.smelt/` directories that were non-empty — the
    /// anti-vacuity signal: a leg that only ever deletes empty directories
    /// is not exercising state-residency at all.
    pub fn nonempty_deletions_observed(&self) -> usize {
        self.nonempty_deletions.load(Ordering::SeqCst)
    }

    /// If `state_deletion == BetweenRuns`, remove `project_dir/.smelt` now,
    /// recording whether it existed and was non-empty. Called from
    /// [`Self::run`] before `execute_project`, so every post-run manifest
    /// read-back in the harness still observes the run that just happened.
    fn maybe_delete_state_dir(&self) -> Result<()> {
        if self.state_deletion != StateDeletion::BetweenRuns {
            return Ok(());
        }
        let state_dir = self.project_dir.join(".smelt");
        let existed = state_dir.exists();
        if existed {
            let nonempty = std::fs::read_dir(&state_dir)?.next().is_some();
            std::fs::remove_dir_all(&state_dir)?;
            self.deletions.fetch_add(1, Ordering::SeqCst);
            if nonempty {
                self.nonempty_deletions.fetch_add(1, Ordering::SeqCst);
            }
        }
        Ok(())
    }

    fn build_db_and_graph(
        &self,
    ) -> (
        Arc<tokio::sync::Mutex<smelt_db::Database>>,
        Arc<tokio::sync::Mutex<DependencyGraph>>,
    ) {
        let discovery = ModelDiscovery::new(self.project_dir.clone(), self.config.paths.clone());
        let sql_models = discovery.discover_models().expect("discover_models");

        let mut db = smelt_db::Database::default();
        let project = db.set_project_input(self.project_dir.clone(), String::new());
        let source_files: Vec<_> = sql_models
            .iter()
            .map(|m| {
                db.set_source_file(m.path.clone(), m.content.clone(), self.project_dir.clone())
            })
            .collect();
        db.set_workspace(source_files, vec![project]);

        let graph = DependencyGraph::build(sql_models, None).expect("build graph");

        (
            Arc::new(tokio::sync::Mutex::new(db)),
            Arc::new(tokio::sync::Mutex::new(graph)),
        )
    }

    /// Run one `execute_project` call over the CURRENT on-disk state of
    /// `project_dir` through the real bound-derivation path, no hand-injected
    /// `WHERE`.
    pub async fn run(
        &self,
        run_id: &str,
        request: ExecuteRequest,
        reporter: &dyn RunReporter,
    ) -> Result<RunOutcome> {
        self.maybe_delete_state_dir()?;
        let (db, graph) = self.build_db_and_graph();
        let outcome = execute_project(
            run_id.to_string(),
            request,
            Arc::clone(&self.config),
            graph,
            db,
            &self.project_dir,
            &DuckDbBackendFactory {
                db_path: self.db_path.clone(),
            },
            reporter,
            CancellationToken::new(),
        )
        .await?;
        Ok(outcome)
    }

    /// Convenience: run with `NoOpReporter` when the cell only cares about the
    /// materialized table contents, not the compiled SQL.
    pub async fn run_quiet(&self, run_id: &str, request: ExecuteRequest) -> Result<RunOutcome> {
        self.run(run_id, request, &NoOpReporter).await
    }

    /// [`Self::run`], but through a [`RecordingBackend`] instead of a plain
    /// [`DuckDbBackendFactory`] — returns the [`RunOutcome`] plus an `Arc`
    /// handle to the backend so the caller can inspect every executed
    /// [`StatementGroup`] afterwards (`RecordingBackend::recorded_groups`).
    /// The one channel that observes ACTUAL executed maintenance DML rather
    /// than the compiled model SELECT [`SqlCapturingReporter`] captures.
    pub async fn run_recording(
        &self,
        run_id: &str,
        request: ExecuteRequest,
    ) -> Result<(RunOutcome, Arc<RecordingBackend>)> {
        let (db, graph) = self.build_db_and_graph();
        let backend_slot: Arc<Mutex<Option<Arc<RecordingBackend>>>> = Arc::new(Mutex::new(None));
        let factory = RecordingBackendFactory {
            db_path: self.db_path.clone(),
            backend: Arc::clone(&backend_slot),
        };
        let outcome = execute_project(
            run_id.to_string(),
            request,
            Arc::clone(&self.config),
            graph,
            db,
            &self.project_dir,
            &factory,
            &NoOpReporter,
            CancellationToken::new(),
        )
        .await?;
        let backend = backend_slot
            .lock()
            .unwrap()
            .clone()
            .expect("RecordingBackendFactory must have populated the slot");
        Ok((outcome, backend))
    }

    /// [`Self::run`] generalised over a [`ConformanceTarget`]
    /// (`docs/plans/20260720-prod-w9-spark-conformance-twin.md` Phases 2-3):
    /// selects the backend factory arm by target. `DuckDb` reproduces
    /// [`Self::run`]'s exact behaviour (the `DuckDbBackendFactory` above);
    /// `SparkDelta` drives the exact same `execute_project` real run pipeline
    /// through [`SparkBackendFactory`] (behind the `spark` feature — without
    /// it, `SparkDelta` is unreachable since no caller constructs one).
    pub async fn run_with_target(
        &self,
        target: ConformanceTarget,
        run_id: &str,
        request: ExecuteRequest,
        reporter: &dyn RunReporter,
    ) -> Result<RunOutcome> {
        match target {
            ConformanceTarget::DuckDb => self.run(run_id, request, reporter).await,
            ConformanceTarget::SparkDelta { schema } => {
                // The schema reaches the run through the project's own
                // `smelt.yml` (`render::render_smelt_yml_for` wrote it from
                // this same target), exactly as the BigQuery arm's dataset
                // does — the factory reads config, not this binding. Bound
                // outside the `cfg` so the no-`spark` build does not see an
                // unused variable.
                let _ = &schema;
                #[cfg(feature = "spark")]
                {
                    let (db, graph) = self.build_db_and_graph();
                    let outcome = execute_project(
                        run_id.to_string(),
                        request,
                        Arc::clone(&self.config),
                        graph,
                        db,
                        &self.project_dir,
                        &SparkBackendFactory,
                        reporter,
                        CancellationToken::new(),
                    )
                    .await?;
                    Ok(outcome)
                }
                #[cfg(not(feature = "spark"))]
                {
                    unimplemented!(
                        "ConformanceTarget::SparkDelta requires the `spark` feature on \
                         smelt-maintenance-testkit"
                    )
                }
            }
            ConformanceTarget::BigQuery { dataset } => {
                #[cfg(feature = "bigquery")]
                {
                    let _ = &dataset;
                    let (db, graph) = self.build_db_and_graph();
                    let outcome = execute_project(
                        run_id.to_string(),
                        request,
                        Arc::clone(&self.config),
                        graph,
                        db,
                        &self.project_dir,
                        &BigQueryBackendFactory,
                        reporter,
                        CancellationToken::new(),
                    )
                    .await?;
                    Ok(outcome)
                }
                #[cfg(not(feature = "bigquery"))]
                {
                    let _ = dataset;
                    unimplemented!(
                        "ConformanceTarget::BigQuery requires the `bigquery` feature on \
                         smelt-maintenance-testkit"
                    )
                }
            }
        }
    }

    /// Open a fresh connection to the harness's DuckDB file — for the cell's
    /// own read-back / oracle comparison after a run.
    pub fn connect(&self) -> Result<duckdb::Connection> {
        Ok(duckdb::Connection::open(&self.db_path)?)
    }

    /// Open a fresh [`DuckDbBackend`] against the harness's DuckDB file, as a
    /// `dyn Backend` — for a cell that wants to route its own seeding/oracle
    /// comparison through the `Backend` trait
    /// (`docs/plans/20260720-prod-w9-spark-conformance-twin.md` Phase 2)
    /// rather than [`Self::connect`]'s raw `duckdb::Connection`. Schema is
    /// always `main`, matching every staging helper in this crate
    /// (`render.rs`'s `stage`/`stage_keyed`/`stage_composed`).
    pub async fn backend(&self) -> Result<Box<dyn Backend>> {
        let backend = DuckDbBackend::new(&self.db_path, "main")
            .await
            .map_err(|e| anyhow::anyhow!("DuckDB backend open failed: {}", e))?;
        Ok(Box::new(backend))
    }

    /// [`Self::backend`] generalised over a [`ConformanceTarget`] (Phase 3):
    /// `DuckDb` reproduces [`Self::backend`]'s exact behaviour; `SparkDelta`
    /// opens a direct Spark/Delta connection to the dedicated conformance
    /// schema ([`open_spark_conformance_backend`]) — the Spark twin's own
    /// harness-internal seeding/oracle-comparison channel, never a raw
    /// host-filesystem read (spec's backend-client-API requirement).
    pub async fn backend_for_target(&self, target: ConformanceTarget) -> Result<Box<dyn Backend>> {
        match target {
            ConformanceTarget::DuckDb => self.backend().await,
            ConformanceTarget::SparkDelta { schema } => {
                // Bound outside the `cfg` so the no-`spark` build, whose arm
                // never reads it, does not see an unused variable.
                let _ = &schema;
                #[cfg(feature = "spark")]
                {
                    open_spark_conformance_backend_in(&self.db_path, &schema).await
                }
                #[cfg(not(feature = "spark"))]
                {
                    unimplemented!(
                        "ConformanceTarget::SparkDelta requires the `spark` feature on \
                         smelt-maintenance-testkit"
                    )
                }
            }
            ConformanceTarget::BigQuery { dataset } => {
                #[cfg(feature = "bigquery")]
                {
                    open_bigquery_backend(&dataset).await
                }
                #[cfg(not(feature = "bigquery"))]
                {
                    let _ = dataset;
                    unimplemented!(
                        "ConformanceTarget::BigQuery requires the `bigquery` feature on \
                         smelt-maintenance-testkit"
                    )
                }
            }
        }
    }
}

/// Open a Spark/Delta backend bound to the dedicated conformance schema
/// (`crate::recipe::SPARK_CONFORMANCE_SCHEMA`) — the Spark-arm counterpart of
/// [`LinkCProject::backend`]'s hardcoded `main`-schema DuckDB connection.
/// `db_path` is only consulted for [`crate::recipe::spark_warehouse_dir`]'s
/// env-unset fallback; the Spark arm never opens it as a file.
#[cfg(feature = "spark")]
pub async fn open_spark_conformance_backend(db_path: &Path) -> Result<Box<dyn Backend>> {
    open_spark_conformance_backend_in(db_path, crate::recipe::SPARK_CONFORMANCE_SCHEMA).await
}

/// [`open_spark_conformance_backend`] parametrized over the schema — the
/// Spark-arm counterpart of [`open_bigquery_conformance_backend`]'s dataset
/// parameter, and the seam that lets a case's full-refresh oracle twin land
/// in different physical storage than its incremental project.
///
/// Spark's warehouse is ONE persistent Delta store shared by every project
/// in a binary, so unlike DuckDB (a private `.duckdb` file per project) the
/// schema is the only thing separating two projects' tables. `SparkBackend::
/// new` issues `CREATE DATABASE IF NOT EXISTS`, so a twin schema needs no
/// separate provisioning step.
#[cfg(feature = "spark")]
pub async fn open_spark_conformance_backend_in(
    db_path: &Path,
    schema: &str,
) -> Result<Box<dyn Backend>> {
    use smelt_backend_spark::SparkBackend;

    let connect_url = crate::recipe::spark_connect_url();
    let warehouse = crate::recipe::spark_warehouse_dir(db_path);
    let warehouse_str = warehouse.to_str().ok_or_else(|| {
        anyhow::anyhow!("Spark warehouse path must be valid UTF-8: {warehouse:?}")
    })?;

    let backend = SparkBackend::new(
        &connect_url,
        "spark_catalog",
        schema,
        Some(warehouse_str),
        true,
    )
    .await
    .map_err(|e| anyhow::anyhow!("Spark backend init failed: {e}"))?;
    Ok(Box::new(backend))
}

/// Open a BigQuery backend bound to `dataset` — the BigQuery-arm counterpart
/// of [`open_spark_conformance_backend`], parametrized over the dataset
/// rather than one fixed schema constant since every case in the pool
/// isolates in its own fresh dataset
/// (`crate::recipe::bq_conformance_dataset`). Project/location/token are
/// read from the environment (`crate::recipe::bq_project`/`bq_location`/
/// `bq_access_token`) — smelt never falls back to Google
/// application-default credentials (`docs/specs/multi_backend.md` §Surface).
#[cfg(feature = "bigquery")]
pub async fn open_bigquery_backend(dataset: &str) -> Result<Box<dyn Backend>> {
    use smelt_backend_bigquery::BigQueryBackend;

    let project = crate::recipe::bq_project()
        .ok_or_else(|| anyhow::anyhow!("BigQuery arm requires SMELT_BQ_PROJECT"))?;
    let location = crate::recipe::bq_location();
    let token = crate::recipe::bq_access_token().ok_or_else(|| {
        anyhow::anyhow!(
            "BigQuery arm requires SMELT_BQ_ACCESS_TOKEN. Mint one with \
             `bash scripts/bigquery-auth.sh` then `source scripts/bigquery-env.sh`."
        )
    })?;

    let backend = BigQueryBackend::new(&project, dataset, location.as_deref(), &token)
        .await
        .map_err(|e| anyhow::anyhow!("BigQuery backend init failed: {e}"))?;
    Ok(Box::new(backend))
}

/// Build a fully qualified, backtick-quoted BigQuery table name
/// `` `project.dataset.name` `` from `SMELT_BQ_PROJECT` and `dataset` — the
/// staging DDL helpers' shared name-quoting routine
/// (`smelt-backend-bigquery`'s own `sql::qualified_name` is private to that
/// crate, so this mirrors its shape rather than reusing it).
#[cfg(feature = "bigquery")]
pub(crate) fn bigquery_qualified_name(dataset: &str, name: &str) -> Result<String> {
    let project = crate::recipe::bq_project()
        .ok_or_else(|| anyhow::anyhow!("BigQuery arm requires SMELT_BQ_PROJECT"))?;
    Ok(format!("`{project}.{dataset}.{name}`"))
}

/// `BackendFactory` that opens a BigQuery backend from whatever `Target`
/// config `execute_project` resolves for the run's target name — mirrors
/// `crates/smelt-backends/src/lib.rs::create_backend`'s production BigQuery
/// arm (project/dataset from the target, access token from
/// `SMELT_BQ_ACCESS_TOKEN`) so the harness's BigQuery run path exercises the
/// same field resolution real runs do, the same duplication-over-reuse
/// convention [`SparkBackendFactory`] already established for the Spark arm
/// rather than adding a `smelt-backends` dependency to this dev-only crate.
#[cfg(feature = "bigquery")]
pub struct BigQueryBackendFactory;

#[cfg(feature = "bigquery")]
impl BackendFactory for BigQueryBackendFactory {
    fn create<'a>(
        &'a self,
        target_name: &'a str,
        target_config: &'a smelt_core::config::Target,
        _project_dir: &'a Path,
    ) -> BackendFuture<'a> {
        Box::pin(async move {
            use smelt_backend_bigquery::BigQueryBackend;

            let project = target_config
                .project
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("BigQuery target requires 'project' field"))?;
            let dataset = target_config
                .dataset
                .as_deref()
                .unwrap_or(&target_config.schema);
            let access_token = std::env::var("SMELT_BQ_ACCESS_TOKEN").map_err(|_| {
                anyhow::anyhow!(
                    "BigQuery target {target_name:?} requires SMELT_BQ_ACCESS_TOKEN. Mint one \
                     with `bash scripts/bigquery-auth.sh` then `source scripts/bigquery-env.sh`."
                )
            })?;

            let backend = BigQueryBackend::new(
                project,
                dataset,
                target_config.location.as_deref(),
                &access_token,
            )
            .await
            .map_err(|e| anyhow::anyhow!("BigQuery backend init failed: {}", e))?;
            Ok(Box::new(backend) as Box<dyn Backend>)
        })
    }
}

/// `BackendFactory` that opens a Spark/Delta backend from whatever
/// `Target` config `execute_project` resolves for the run's target name —
/// mirrors `crates/smelt-cli/src/backend_factory.rs::CliBackendFactory`'s
/// production Spark arm (`smelt_backends::create_backend`'s `BackendType::Spark`
/// case) so the harness's Spark run path exercises the same field
/// resolution (`connect_url`/`catalog`/`schema`/`warehouse`/`format`) real
/// runs do, rather than a harness-only shortcut.
#[cfg(feature = "spark")]
pub struct SparkBackendFactory;

#[cfg(feature = "spark")]
impl BackendFactory for SparkBackendFactory {
    fn create<'a>(
        &'a self,
        _target_name: &'a str,
        target_config: &'a smelt_core::config::Target,
        _project_dir: &'a Path,
    ) -> BackendFuture<'a> {
        Box::pin(async move {
            use smelt_backend_spark::SparkBackend;
            use smelt_core::config::TableFormat;

            let connect_url = target_config
                .connect_url
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("Spark target requires 'connect_url'"))?;
            let catalog = target_config.catalog.as_deref().unwrap_or("spark_catalog");
            let use_delta = target_config
                .table_format()
                .map(|f| matches!(f, TableFormat::Delta))
                .unwrap_or(true);

            let backend = SparkBackend::new(
                connect_url,
                catalog,
                &target_config.schema,
                target_config.warehouse.as_deref(),
                use_delta,
            )
            .await
            .map_err(|e| anyhow::anyhow!("Spark backend init failed: {}", e))?;
            Ok(Box::new(backend) as Box<dyn Backend>)
        })
    }
}

/// Drive `fut` to completion from a **fresh OS thread carrying its own Tokio
/// runtime**, regardless of whether the calling thread is itself already
/// driving a runtime (`docs/plans/20260720-prod-w9-spark-conformance-twin.md`
/// Phase 2 review finding: several staging helpers — `render.rs`'s
/// `stage`/`stage_keyed`/`stage_composed` — are called from both plain
/// `#[test]` functions and from inside `#[tokio::test] async fn` bodies via
/// `CaseContext::stage_partition`/`stage_keyed`, so neither a bare
/// `Runtime::new().block_on(..)` on the current thread (panics: "Cannot
/// start a runtime from within a runtime" when already inside one) nor
/// `tokio::task::block_in_place` (panics outside a multi-thread-flavor
/// runtime, and these `#[tokio::test]`s use the default current-thread
/// flavor) is safe here. A brand-new OS thread has no Tokio context at all,
/// so opening a fresh single-use `Runtime` there and blocking on it is safe
/// unconditionally. Kept here (rather than duplicated per call site) since
/// every staging helper needs it and this module is `LinkCProject`'s home.
pub(crate) fn block_on_isolated<F>(fut: F) -> F::Output
where
    F: std::future::Future + Send + 'static,
    F::Output: Send + 'static,
{
    match std::thread::spawn(move || {
        tokio::runtime::Runtime::new()
            .unwrap_or_else(|e| panic!("tokio runtime for isolated blocking call: {e}"))
            .block_on(fut)
    })
    .join()
    {
        Ok(output) => output,
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

/// A minimal `ExecuteRequest` for `target`, all incremental/backfill knobs at
/// their default; callers override `start`/`end`/`batch_size_days` as the
/// cell's run schedule needs.
pub fn base_request(target: &str) -> ExecuteRequest {
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
        jobs: None,
        retry_max: None,
        retry_backoff_ms: None,
        resume: false,
        technique_overrides: vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recipe::{arb_recipe, RecipePool};
    use crate::render;
    use crate::verdict::{classify, Verdict};
    use proptest::strategy::{Strategy, ValueTree};
    use proptest::test_runner::TestRunner;

    /// `state_deletion_removes_a_populated_state_dir_before_each_run`
    /// (`docs/outcomes/20260904-state-residency/phases/09-plan.md` test 1):
    /// with `StateDeletion::BetweenRuns`, a second `run` finds no `.smelt/`
    /// left by the first — the toggle removes it before `execute_project`
    /// runs again — and the deletion counter records that the removed
    /// directory was non-empty.
    #[test]
    fn state_deletion_removes_a_populated_state_dir_before_each_run() {
        let mut runner = TestRunner::deterministic();
        let strat = arb_recipe(RecipePool::partition_append_only());

        // Draw the first recipe the deterministic sequence admits — this is
        // a harness unit test, not the generative gate, so one admitted
        // case is all it needs.
        let (_tmp, project) = loop {
            let recipe = strat.new_tree(&mut runner).unwrap().current();
            let tmp = tempfile::TempDir::new().expect("tempdir");
            let project_dir = tmp.path().join("project");
            std::fs::create_dir_all(&project_dir).expect("create project dir");
            let db_path = tmp.path().join("db.duckdb");
            let project = render::stage(&recipe, &project_dir, &db_path).expect("stage recipe");
            match classify(&project, &recipe).expect("classify") {
                Verdict::Refused(_) => continue,
                Verdict::Admitted(_) => break (tmp, project),
            }
        };

        let project = project.with_state_deletion(StateDeletion::BetweenRuns);
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");

        let state_dir = project.project_dir.join(".smelt");

        // Run 1: no `.smelt/` predates this project, so nothing is deleted
        // yet; the run populates it (`state.mode: intervals`, the fixture
        // default — `render.rs`).
        let mut request = base_request("dev");
        request.full_refresh = true;
        rt.block_on(project.run_quiet("run-0", request))
            .expect("run 0");
        assert!(
            state_dir.exists(),
            ".smelt/ should exist after the first run under state.mode: intervals"
        );
        assert_eq!(project.deletions_observed(), 0);

        // Run 2: `maybe_delete_state_dir` fires before `execute_project`,
        // removing the directory run 1 just populated.
        let mut request = base_request("dev");
        request.full_refresh = true;
        rt.block_on(project.run_quiet("run-1", request))
            .expect("run 1");

        assert_eq!(
            project.deletions_observed(),
            1,
            "run 2 should have deleted the .smelt/ dir run 1 left behind"
        );
        assert_eq!(
            project.nonempty_deletions_observed(),
            1,
            "the deleted .smelt/ dir was non-empty (run 1 wrote manifests into it)"
        );
    }
}
