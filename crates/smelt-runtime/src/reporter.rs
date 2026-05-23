//! Transport-agnostic progress reporting for the run pipeline.
//!
//! `smelt-cli` implements [`RunReporter`] as a stdout/spinner reporter;
//! `smelt-ui` implements it as a broadcast adapter over its
//! `RunProgressEvent` channel; tests implement a captured-log variant via
//! [`NoOpReporter`] or a small in-memory recorder.
//!
//! Every method has a default no-op body so consumers can implement only the
//! callbacks they care about. Each method takes `&self`, allowing reporters
//! to manage interior mutability or shared state (channels, atomics, locks)
//! as needed.

use std::time::Duration;

/// Sink for run-progress callbacks. Implementations are responsible for the
/// transport (stdout, broadcast channel, log capture); the runtime emits the
/// events.
///
/// The runtime calls these in a well-defined order during a successful run:
///
/// ```text
/// run_started
///   for each model:
///     model_started
///     (batch_completed)* — only for incremental models
///     model_completed
/// run_completed
/// ```
///
/// A failed or cancelled run calls `run_failed` or `run_cancelled`
/// respectively in place of `run_completed`. A consumer may receive any
/// subset of these events depending on where execution stopped.
pub trait RunReporter: Send + Sync {
    /// Run dispatch begins. `models` is the post-selection ordered list of
    /// model names that will execute (test models, generator files, etc.
    /// already filtered out by `select_executable_models`).
    fn run_started(&self, _run_id: &str, _models: &[String], _total_batches: usize) {}

    /// A model's execution begins.
    fn model_started(
        &self,
        _run_id: &str,
        _model: &str,
        _model_index: usize,
        _models_total: usize,
    ) {
    }

    /// One batch of an incremental model completed. `batch_index` is
    /// 0-based; `batches_total` is the count of batches in this model's
    /// plan.
    fn batch_completed(
        &self,
        _run_id: &str,
        _model: &str,
        _batch_index: usize,
        _batches_total: usize,
        _row_count: usize,
        _duration: Duration,
    ) {
    }

    /// A model finished — full-refresh table, view, or the final batch of an
    /// incremental model. `row_count` is the total across all batches for
    /// incremental models.
    fn model_completed(&self, _run_id: &str, _model: &str, _row_count: usize, _duration: Duration) {
    }

    /// All models finished successfully.
    fn run_completed(&self, _run_id: &str, _total_rows: usize, _duration: Duration) {}

    /// Run aborted. `model` identifies which model the failure occurred in
    /// (if it had begun executing); `error` is a human-readable message.
    fn run_failed(&self, _run_id: &str, _model: Option<&str>, _error: &str) {}

    /// Run cancelled via the cancellation token.
    fn run_cancelled(&self, _run_id: &str) {}
}

/// No-op reporter: discards all events. Used by tests and by run paths that
/// do not need progress reporting (e.g. internal tooling).
#[derive(Debug, Default, Clone, Copy)]
pub struct NoOpReporter;

impl RunReporter for NoOpReporter {}
