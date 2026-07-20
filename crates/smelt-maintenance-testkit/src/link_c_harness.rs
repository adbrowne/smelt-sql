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
use std::sync::{Arc, Mutex};

use anyhow::Result;
use smelt_backend::Backend;
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

/// A staged smelt project directory + the parsed `Config` `execute_project`
/// needs. Re-discovers models from disk each `run` so a between-run source
/// mutation (append/update staged by the caller) is picked up — the whole point
/// of the run-schedule driver (design §3b): the source must be able to change
/// between runs, not be fully pre-populated up front.
pub struct LinkCProject {
    pub project_dir: PathBuf,
    pub db_path: PathBuf,
    pub config: Arc<Config>,
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
        })
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

    /// [`Self::run`] generalised over a [`ConformanceTarget`]
    /// (`docs/plans/20260720-prod-w9-spark-conformance-twin.md` Phase 2):
    /// selects the backend factory arm by target. `DuckDb` reproduces
    /// [`Self::run`]'s exact behaviour (the `DuckDbBackendFactory` above);
    /// `SparkDelta` is `unimplemented!()` until a later phase of that plan
    /// wires up a Spark backend factory — never reachable today since no
    /// caller constructs a `SparkDelta`-targeted `LinkCProject`.
    pub async fn run_with_target(
        &self,
        target: ConformanceTarget,
        run_id: &str,
        request: ExecuteRequest,
        reporter: &dyn RunReporter,
    ) -> Result<RunOutcome> {
        match target {
            ConformanceTarget::DuckDb => self.run(run_id, request, reporter).await,
            ConformanceTarget::SparkDelta => unimplemented!(
                "Spark backend factory arm lands in a later phase of \
                 docs/plans/20260720-prod-w9-spark-conformance-twin.md"
            ),
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
