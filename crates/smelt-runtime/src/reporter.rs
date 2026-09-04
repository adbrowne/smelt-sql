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

use smelt_logical::maintenance::emit::StatementGroup;

/// Which chunk of a chunked range a [`StatementGroup`] belongs to, when the
/// batch-safety classification (or an explicit `--batch-size`/`--per-partition`)
/// splits a run/rebuild range into more than one window. `index` is 0-based;
/// `total` is the count of chunks for this model; `start`/`end` are the
/// `[start, end)` window this chunk covers, formatted as `YYYY-MM-DD`. A
/// single-chunk range still carries a `ChunkInfo` with `total == 1` — consumers
/// decide whether to render a boundary line (`smelt rebuild --dry-run` prints
/// one only when `total > 1`, `docs/specs/cli.md` §"`--dry-run` prints the
/// maintenance statements").
#[derive(Debug, Clone)]
pub struct ChunkInfo {
    pub index: usize,
    pub total: usize,
    pub start: String,
    pub end: String,
}

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

    /// A model's SQL has been compiled. Called after compilation and before
    /// execution. The `sql` argument is the fully-resolved SQL string.
    ///
    /// Consumers that implement `--verbose` (the CLI) print `sql` to stdout.
    /// Default: no-op. This is the Phase 4 hook; consumers that need verbose
    /// output implement it; others inherit the default.
    fn model_compiled(&self, _run_id: &str, _model: &str, _sql: &str) {}

    /// The maintenance statements a batch/chunk is about to execute (or, under
    /// `--dry-run`, would execute), as produced by the single-owner emitters in
    /// `smelt_logical::maintenance::emit` (`docs/specs/incremental_models.md`
    /// §"Statement emission (single owner)"). Called after `model_compiled` and
    /// before the batch's backend call (a real run) or in place of it (a
    /// dry-run), for every maintained (non-`full`) technique this runtime lowers
    /// to a `StatementGroup`. `chunk` names which window of a chunked range this
    /// group covers (`None` when the technique is not region-chunked, e.g. a
    /// keyed fold). Default: no-op; `smelt run`/`smelt rebuild --dry-run` and
    /// statement-parity tests are the consumers.
    fn maintenance_statements(
        &self,
        _run_id: &str,
        _model: &str,
        _chunk: Option<&ChunkInfo>,
        _group: &StatementGroup,
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

    /// A `smelt.check` executed during `smelt build`. `status` is one of
    /// `"pass"`, `"fail"`, `"warn"`, or `"target_not_built"`.
    fn check_result(&self, _run_id: &str, _check: &str, _status: &str, _row_count: usize) {}

    /// A model's statement-group execution hit a transient backend error
    /// (`BackendError::is_transient`) and is about to retry the whole group
    /// after a backoff delay (`docs/plans/20260719-prod-w2-operability.md`
    /// Phase 6). `attempt` is 1-based (this is the Nth retry); `retry_max`
    /// is the configured bound; `error` is the transient failure's display
    /// message. Called after the failed attempt and before the delayed
    /// re-attempt. Default: no-op; `smelt run`/`smelt build` verbose output
    /// and retry tests are the consumers.
    fn model_retrying(
        &self,
        _run_id: &str,
        _model: &str,
        _attempt: u32,
        _retry_max: u32,
        _error: &str,
    ) {
    }
}

/// No-op reporter: discards all events. Used by tests and by run paths that
/// do not need progress reporting (e.g. internal tooling).
#[derive(Debug, Default, Clone, Copy)]
pub struct NoOpReporter;

impl RunReporter for NoOpReporter {}
