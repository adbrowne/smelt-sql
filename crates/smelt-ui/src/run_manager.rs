//! UI run manager: a thin surface adapter over `smelt_runtime::execute_project`.
//!
//! - HTTP `RunExecuteRequest` → `smelt_runtime::ExecuteRequest` conversion.
//! - `RunReporter` impl that forwards events to the UI's WebSocket broadcast
//!   channel as `RunProgressEvent::*` and updates the atomic counters that
//!   `GET /api/run/status` reads.
//! - `BackendFactory` impl that creates DuckDB backends (Spark not yet
//!   supported in UI mode).
//!
//! No execute logic lives here — that's the Run Pipeline Parity Rule's
//! contract. See `docs/specs/architecture.md` → "Run pipeline parity rule
//! (CLI ↔ UI)".

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration as StdDuration;

use anyhow::{Context, Result};
use tokio::sync::{broadcast, Mutex};
use tokio_util::sync::CancellationToken;

use smelt_backend::Backend;
use smelt_core::config::Config;
use smelt_core::graph::DependencyGraph;
use smelt_core::SourcesConfig;
use smelt_runtime::{BackendFactory, RunReporter};
use smelt_state::generate_run_id;

use crate::types::*;

/// Manages run execution state. Only one run at a time.
pub struct RunManager {
    inner: Mutex<RunManagerInner>,
    counters: Arc<RunCounters>,
    current_model: Arc<std::sync::Mutex<Option<String>>>,
    event_tx: broadcast::Sender<RunProgressEvent>,
    project_dir: PathBuf,
}

struct RunManagerInner {
    state: RunState,
    run_id: Option<String>,
    cancel_token: Option<CancellationToken>,
}

#[derive(Default)]
struct RunCounters {
    models_completed: AtomicUsize,
    models_total: AtomicUsize,
    batches_completed: AtomicUsize,
    batches_total: AtomicUsize,
}

impl RunManager {
    pub fn new(event_tx: broadcast::Sender<RunProgressEvent>, project_dir: PathBuf) -> Self {
        Self {
            inner: Mutex::new(RunManagerInner {
                state: RunState::Idle,
                run_id: None,
                cancel_token: None,
            }),
            counters: Arc::new(RunCounters::default()),
            current_model: Arc::new(std::sync::Mutex::new(None)),
            event_tx,
            project_dir,
        }
    }

    pub async fn status(&self) -> RunStatusResponse {
        let inner = self.inner.lock().await;
        let running = inner.state == RunState::Running;
        let current = self.current_model.lock().unwrap().clone();
        RunStatusResponse {
            state: inner.state,
            run_id: inner.run_id.clone(),
            current_model: current,
            models_completed: running
                .then(|| self.counters.models_completed.load(Ordering::Relaxed)),
            models_total: running.then(|| self.counters.models_total.load(Ordering::Relaxed)),
            batches_completed: running
                .then(|| self.counters.batches_completed.load(Ordering::Relaxed)),
            batches_total: running.then(|| self.counters.batches_total.load(Ordering::Relaxed)),
        }
    }

    /// Start a run. Returns Err if already running.
    pub async fn execute(
        self: &Arc<Self>,
        request: RunExecuteRequest,
        config: Arc<Config>,
        _sources: Arc<Option<SourcesConfig>>,
        graph: Arc<Mutex<DependencyGraph>>,
        db: Arc<Mutex<smelt_db::Database>>,
    ) -> Result<String, &'static str> {
        let mut inner = self.inner.lock().await;
        if inner.state == RunState::Running {
            return Err("A run is already in progress");
        }

        let run_id = generate_run_id();
        let cancel_token = CancellationToken::new();

        inner.state = RunState::Running;
        inner.run_id = Some(run_id.clone());
        inner.cancel_token = Some(cancel_token.clone());
        drop(inner);

        // Reset counters and current_model for this run.
        self.counters.models_completed.store(0, Ordering::Relaxed);
        self.counters.models_total.store(0, Ordering::Relaxed);
        self.counters.batches_completed.store(0, Ordering::Relaxed);
        self.counters.batches_total.store(0, Ordering::Relaxed);
        *self.current_model.lock().unwrap() = None;

        let manager = self.clone();
        let run_id_clone = run_id.clone();
        let project_dir = self.project_dir.clone();
        let reporter = BroadcastReporter {
            event_tx: self.event_tx.clone(),
            counters: self.counters.clone(),
            current_model: self.current_model.clone(),
        };
        let backend_factory = UiBackendFactory;
        let exec_request = to_runtime_request(request);

        tokio::spawn(async move {
            let result = smelt_runtime::execute_project(
                run_id_clone.clone(),
                exec_request,
                config,
                graph,
                db,
                &project_dir,
                &backend_factory,
                &reporter,
                cancel_token,
            )
            .await;

            let mut inner = manager.inner.lock().await;
            inner.state = RunState::Idle;
            inner.cancel_token = None;

            if let Err(e) = result {
                tracing::error!("Run {} failed: {}", run_id_clone, e);
            }
        });

        Ok(run_id)
    }

    pub async fn cancel(&self) -> bool {
        let inner = self.inner.lock().await;
        if let Some(ref token) = inner.cancel_token {
            token.cancel();
            true
        } else {
            false
        }
    }
}

/// Translate the UI's HTTP request shape into `smelt_runtime`'s.
fn to_runtime_request(request: RunExecuteRequest) -> smelt_runtime::ExecuteRequest {
    smelt_runtime::ExecuteRequest {
        target: request.target,
        select: request.select,
        exclude: request.exclude,
        start: Some(request.start),
        end: Some(request.end),
        batch_size_days: request.batch_size_days,
        per_partition: request.per_partition,
        full_refresh: false,
        dry_run: false,
        enforce_safety: true,
        allow_column_removal: false,
        allow_full_refresh: false,
        ephemeral_seed_ctes: vec![],
    }
}

/// Bridges `smelt_runtime`'s sync `RunReporter` callbacks to the UI's
/// async broadcast channel + atomic counters. Methods never block.
struct BroadcastReporter {
    event_tx: broadcast::Sender<RunProgressEvent>,
    counters: Arc<RunCounters>,
    current_model: Arc<std::sync::Mutex<Option<String>>>,
}

impl RunReporter for BroadcastReporter {
    fn run_started(&self, run_id: &str, models: &[String], total_batches: usize) {
        self.counters
            .models_total
            .store(models.len(), Ordering::Relaxed);
        self.counters
            .batches_total
            .store(total_batches, Ordering::Relaxed);
        let _ = self.event_tx.send(RunProgressEvent::RunStarted {
            run_id: run_id.to_string(),
            models: models.to_vec(),
            total_batches,
        });
    }

    fn model_started(&self, run_id: &str, model: &str, model_index: usize, models_total: usize) {
        *self.current_model.lock().unwrap() = Some(model.to_string());
        let _ = self.event_tx.send(RunProgressEvent::ModelStarted {
            run_id: run_id.to_string(),
            model: model.to_string(),
            model_index,
            models_total,
        });
    }

    fn batch_completed(
        &self,
        run_id: &str,
        model: &str,
        batch_index: usize,
        batches_total: usize,
        row_count: usize,
        duration: StdDuration,
    ) {
        self.counters
            .batches_completed
            .fetch_add(1, Ordering::Relaxed);
        let _ = self.event_tx.send(RunProgressEvent::BatchCompleted {
            run_id: run_id.to_string(),
            model: model.to_string(),
            batch_index,
            batches_total,
            row_count,
            duration_ms: duration.as_millis() as u64,
        });
    }

    fn model_completed(&self, run_id: &str, model: &str, row_count: usize, duration: StdDuration) {
        self.counters
            .models_completed
            .fetch_add(1, Ordering::Relaxed);
        let _ = self.event_tx.send(RunProgressEvent::ModelCompleted {
            run_id: run_id.to_string(),
            model: model.to_string(),
            row_count,
            duration_ms: duration.as_millis() as u64,
        });
    }

    fn run_completed(&self, run_id: &str, _total_rows: usize, duration: StdDuration) {
        let models_executed = self.counters.models_completed.load(Ordering::Relaxed);
        let _ = self.event_tx.send(RunProgressEvent::RunCompleted {
            run_id: run_id.to_string(),
            models_executed,
            duration_ms: duration.as_millis() as u64,
        });
    }

    fn run_failed(&self, run_id: &str, model: Option<&str>, error: &str) {
        let _ = self.event_tx.send(RunProgressEvent::RunFailed {
            run_id: run_id.to_string(),
            error: error.to_string(),
            model: model.map(|s| s.to_string()),
        });
    }

    fn run_cancelled(&self, run_id: &str) {
        let _ = self.event_tx.send(RunProgressEvent::RunCancelled {
            run_id: run_id.to_string(),
        });
    }
}

/// Backend factory for the UI (DuckDB only at present; Spark in CLI mode
/// only). Implements `smelt_runtime::BackendFactory` so the shared
/// `execute_project` stays backend-agnostic.
struct UiBackendFactory;

impl BackendFactory for UiBackendFactory {
    fn create<'a>(
        &'a self,
        target_name: &'a str,
        target_config: &'a smelt_core::config::Target,
        project_dir: &'a Path,
    ) -> smelt_runtime::BackendFuture<'a> {
        Box::pin(create_backend_inner(
            target_name,
            target_config,
            project_dir,
        ))
    }
}

#[allow(unreachable_code, unused_variables)]
async fn create_backend_inner(
    _target_name: &str,
    target_config: &smelt_core::config::Target,
    project_dir: &Path,
) -> Result<Box<dyn Backend>> {
    use smelt_core::config::BackendType;
    match target_config.backend_type() {
        BackendType::DuckDB => {
            #[cfg(feature = "duckdb")]
            {
                let database = target_config
                    .database
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("DuckDB target requires 'database' field"))?;
                let db_path = project_dir.join(database);
                let backend =
                    smelt_backend_duckdb::DuckDbBackend::new(&db_path, &target_config.schema)
                        .await
                        .with_context(|| format!("Failed to initialize DuckDB at {:?}", db_path))?;
                Ok(Box::new(backend))
            }
            #[cfg(not(feature = "duckdb"))]
            {
                anyhow::bail!("DuckDB feature not enabled")
            }
        }
        BackendType::Spark => {
            anyhow::bail!("Spark backend not yet supported in UI mode")
        }
    }
}
