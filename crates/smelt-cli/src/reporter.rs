//! CLI `RunReporter` implementation and `--show-plan` formatting helpers.
//!
//! `CliReporter` (the plan calls it `StdoutReporter`) forwards runtime
//! progress events to `tracing` / stderr and prints compiled SQL to stdout
//! when `--verbose` or `--dry-run` is active.

use smelt_logical::maintenance::emit::StatementGroup;
use smelt_runtime::reporter::{ChunkInfo, RunReporter};
use smelt_runtime::types::ModelStrategy;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tracing::info;

/// CLI reporter that forwards runtime events to tracing / stderr.
///
/// - `model_compiled` on a dry-run prints "Would run: {model}" + compiled SQL.
/// - `model_compiled` with verbose (non-dry-run) prints the SQL with a comment
///   header, so it surfaces without requiring `RUST_LOG=debug`.
pub struct CliReporter {
    pub verbose: bool,
    pub dry_run: bool,
    #[allow(dead_code)]
    pub show_results: bool,
    model_count: AtomicUsize,
}

impl CliReporter {
    pub fn new(verbose: bool, dry_run: bool, show_results: bool) -> Self {
        Self {
            verbose,
            dry_run,
            show_results,
            model_count: AtomicUsize::new(0),
        }
    }
}

impl RunReporter for CliReporter {
    fn run_started(&self, _run_id: &str, models: &[String], _total_batches: usize) {
        self.model_count.store(models.len(), Ordering::Relaxed);
        info!("Running {} model(s)…", models.len());
    }

    fn model_started(&self, _run_id: &str, model: &str, idx: usize, total: usize) {
        info!("Running model: {} ({}/{})", model, idx + 1, total);
    }

    fn batch_completed(
        &self,
        _run_id: &str,
        _model: &str,
        batch_idx: usize,
        batches_total: usize,
        row_count: usize,
        duration: Duration,
    ) {
        if batches_total > 1 {
            info!(
                "  batch {}/{}: {} rows ({:?})",
                batch_idx + 1,
                batches_total,
                row_count,
                duration
            );
        }
    }

    fn model_compiled(&self, _run_id: &str, model: &str, sql: &str) {
        if self.dry_run {
            println!("-- Would run: {} (materialization shown below)", model);
            if !sql.is_empty() {
                println!("{}", sql.trim_end());
            }
            println!();
        } else if self.verbose {
            println!("-- {}", model);
            println!("{}", sql);
            println!();
        }
    }

    fn maintenance_statements(
        &self,
        _run_id: &str,
        _model: &str,
        chunk: Option<&ChunkInfo>,
        group: &StatementGroup,
    ) {
        // `--dry-run` prints the maintenance statements this invocation would
        // execute (`docs/specs/cli.md` §"`--dry-run` prints the maintenance
        // statements"). A real run does not re-print them — its progress is the
        // `batch_completed`/`model_completed` summary. A backbuild whose range
        // was split into chunks introduces each chunk's block with a boundary
        // line naming its `[start, end)` window and position.
        if !self.dry_run {
            return;
        }
        if let Some(c) = chunk {
            if c.total > 1 {
                println!(
                    "-- chunk {}/{}: [{}, {})",
                    c.index + 1,
                    c.total,
                    c.start,
                    c.end
                );
            }
        }
        print!("{}", crate::explain::render_statement_group_text(group, ""));
    }

    fn model_completed(&self, _run_id: &str, model: &str, row_count: usize, duration: Duration) {
        info!("{} done ({} rows, {:?})", model, row_count, duration);
    }

    fn run_completed(&self, _run_id: &str, _total_rows: usize, duration: Duration) {
        let count = self.model_count.load(Ordering::Relaxed);
        eprintln!(
            "smelt: built {} model(s) in {:.2}s",
            count,
            duration.as_secs_f64(),
        );
    }

    fn run_failed(&self, _run_id: &str, model: Option<&str>, error: &str) {
        if let Some(m) = model {
            eprintln!("smelt: run failed at model '{}': {}", m, error);
        } else {
            eprintln!("smelt: run failed: {}", error);
        }
    }

    fn run_cancelled(&self, _run_id: &str) {
        eprintln!("smelt: run cancelled");
    }
}

/// Format a `ModelStrategy` as a short human-readable label for `--show-plan`.
pub fn format_strategy(strategy: &ModelStrategy) -> String {
    match strategy {
        ModelStrategy::FullRefresh => "full-refresh".to_string(),
        ModelStrategy::Incremental {
            partition_column,
            granularity,
        } => format!("incremental (by {}, {})", partition_column, granularity),
        ModelStrategy::Keyed => "keyed".to_string(),
        ModelStrategy::Ephemeral => "ephemeral".to_string(),
        ModelStrategy::Skipped { .. } => "skipped".to_string(),
    }
}
