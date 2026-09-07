use std::collections::{HashMap, HashSet};
use std::time::Duration as StdDuration;

use smelt_state::ModelRunRecord;

use crate::check_runner::CheckOutcome;
use crate::reporter::RunReporter;

/// Per-model result of one execution unit dispatched by the Phase 5
/// wavefront scheduler in [`execute_project`]. `Completed` carries every
/// piece of run-level state the pre-Phase-5 sequential loop mutated
/// in-place (`manifest`, `check_results`, `skip_set`, row count) so the
/// scheduler can merge it deterministically once this model's turn comes up
/// in `execution_order` sequence.
pub(crate) enum ModelOutcome {
    Completed(ModelSuccess),
    Cancelled,
}

pub(crate) struct ModelSuccess {
    pub(crate) manifest_entries: HashMap<String, ModelRunRecord>,
    pub(crate) check_results: Vec<CheckOutcome>,
    pub(crate) skip_set: HashSet<String>,
    pub(crate) rows: usize,
}

/// A single buffered [`RunReporter`] callback, recorded by [`EventSink`]
/// during one model's (possibly concurrent) execution and replayed onto the
/// real reporter later, strictly in `execution_order` sequence
/// (`docs/plans/20260719-prod-w2-operability.md` Phase 5: "Buffer per-model
/// reporter events and flush in `execution_order` sequence").
pub(crate) enum ReporterEvent {
    ModelStarted {
        model_index: usize,
        models_total: usize,
    },
    ModelCompiled {
        sql: String,
    },
    MaintenanceStatements {
        chunk: Option<crate::reporter::ChunkInfo>,
        group: smelt_logical::maintenance::emit::StatementGroup,
    },
    BatchCompleted {
        batch_index: usize,
        batches_total: usize,
        row_count: usize,
        duration: StdDuration,
    },
    ModelCompleted {
        row_count: usize,
        duration: StdDuration,
    },
    CheckResult {
        check: String,
        status: String,
        row_count: usize,
    },
    ModelRetrying {
        attempt: u32,
        retry_max: u32,
        error: String,
    },
}

/// Records [`RunReporter`] callbacks made during one model's execution
/// instead of forwarding them immediately — the wavefront scheduler may run
/// several models' execution units concurrently, and forwarding callbacks
/// as they happen would interleave them nondeterministically. Implements
/// [`RunReporter`] itself so the (otherwise unmodified) per-model execution
/// logic can call it under the shadowed name `reporter` with no rewriting.
#[derive(Default)]
pub(crate) struct EventSink {
    pub(crate) events: std::sync::Mutex<Vec<ReporterEvent>>,
}

impl EventSink {
    /// Record one event. The single lock site every [`RunReporter`] method
    /// below routes through, so `EventSink` needs exactly one poisoned-lock
    /// `.expect` for all of them combined rather than one per method
    /// (`.claude/hardening-baseline.txt` ratchet — see root `CLAUDE.md`
    /// §"Fail-loud discipline").
    fn push(&self, event: ReporterEvent) {
        self.events
            .lock()
            .expect("EventSink mutex poisoned")
            .push(event);
    }

    /// Number of `ModelRetrying` events buffered for this model — the final
    /// per-model retry count threaded into its `ModelRunRecord` (`error`/
    /// `retry_count` fields, `docs/plans/20260719-prod-w2-operability.md`
    /// Phase 8). Every retry attempt calls `model_retrying` exactly once, so
    /// this count is exact regardless of whether the model ultimately
    /// succeeded or failed.
    pub(crate) fn retry_count(&self) -> u32 {
        self.events
            .lock()
            .expect("EventSink mutex poisoned")
            .iter()
            .filter(|e| matches!(e, ReporterEvent::ModelRetrying { .. }))
            .count() as u32
    }

    /// Replay every buffered event onto `reporter`, in the order recorded.
    pub(crate) fn replay(&self, reporter: &dyn RunReporter, run_id: &str, model: &str) {
        for event in self.events.lock().expect("EventSink mutex poisoned").iter() {
            match event {
                ReporterEvent::ModelStarted {
                    model_index,
                    models_total,
                } => reporter.model_started(run_id, model, *model_index, *models_total),
                ReporterEvent::ModelCompiled { sql } => reporter.model_compiled(run_id, model, sql),
                ReporterEvent::MaintenanceStatements { chunk, group } => {
                    reporter.maintenance_statements(run_id, model, chunk.as_ref(), group)
                }
                ReporterEvent::BatchCompleted {
                    batch_index,
                    batches_total,
                    row_count,
                    duration,
                } => reporter.batch_completed(
                    run_id,
                    model,
                    *batch_index,
                    *batches_total,
                    *row_count,
                    *duration,
                ),
                ReporterEvent::ModelCompleted {
                    row_count,
                    duration,
                } => reporter.model_completed(run_id, model, *row_count, *duration),
                ReporterEvent::CheckResult {
                    check,
                    status,
                    row_count,
                } => reporter.check_result(run_id, check, status, *row_count),
                ReporterEvent::ModelRetrying {
                    attempt,
                    retry_max,
                    error,
                } => reporter.model_retrying(run_id, model, *attempt, *retry_max, error),
            }
        }
    }
}

impl RunReporter for EventSink {
    fn model_started(&self, _run_id: &str, _model: &str, model_index: usize, models_total: usize) {
        self.push(ReporterEvent::ModelStarted {
            model_index,
            models_total,
        });
    }

    fn model_compiled(&self, _run_id: &str, _model: &str, sql: &str) {
        self.push(ReporterEvent::ModelCompiled {
            sql: sql.to_string(),
        });
    }

    fn maintenance_statements(
        &self,
        _run_id: &str,
        _model: &str,
        chunk: Option<&crate::reporter::ChunkInfo>,
        group: &smelt_logical::maintenance::emit::StatementGroup,
    ) {
        self.push(ReporterEvent::MaintenanceStatements {
            chunk: chunk.cloned(),
            group: group.clone(),
        });
    }

    fn batch_completed(
        &self,
        _run_id: &str,
        _model: &str,
        batch_index: usize,
        batches_total: usize,
        row_count: usize,
        duration: StdDuration,
    ) {
        self.push(ReporterEvent::BatchCompleted {
            batch_index,
            batches_total,
            row_count,
            duration,
        });
    }

    fn model_completed(
        &self,
        _run_id: &str,
        _model: &str,
        row_count: usize,
        duration: StdDuration,
    ) {
        self.push(ReporterEvent::ModelCompleted {
            row_count,
            duration,
        });
    }

    fn check_result(&self, _run_id: &str, check: &str, status: &str, row_count: usize) {
        self.push(ReporterEvent::CheckResult {
            check: check.to_string(),
            status: status.to_string(),
            row_count,
        });
    }

    fn model_retrying(
        &self,
        _run_id: &str,
        _model: &str,
        attempt: u32,
        retry_max: u32,
        error: &str,
    ) {
        self.push(ReporterEvent::ModelRetrying {
            attempt,
            retry_max,
            error: error.to_string(),
        });
    }
}
