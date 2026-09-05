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
        // `batch_completed`/`model_completed` summary. A rebuild whose range
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

/// Coarse classification of why a model failed, inferred from its recorded
/// error text. Nothing upstream of the report currently carries a
/// structured failure stage (`ModelFailure::error` is a flattened
/// `anyhow::Error::to_string()` — see `smelt-runtime`'s `execute.rs` abort
/// path), so this is a best-effort text classifier rather than a match over
/// a typed error, one leg of the grouped failure summary
/// (`docs/plans/20260719-prod-w3-adoption.md` Phase 6;
/// `docs/specs/cli.md` §Semantics "Failure summary").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FailureCause {
    /// The model's SQL/functions failed to compile (parse, type, or
    /// reference resolution).
    Compile,
    /// The compiled SQL ran but the backend rejected it (bad cast, DDL/DML
    /// failure, constraint violation).
    Execute,
    /// A `smelt check`/declarative-test failure downstream of a successful
    /// build.
    Check,
}

fn classify_failure_cause(error: &str) -> FailureCause {
    let lower = error.to_lowercase();
    if lower.contains("check failed") || lower.contains("constraint") {
        FailureCause::Check
    } else if lower.contains("compil")
        || lower.contains("parse error")
        || lower.contains("unresolved")
        || lower.contains("undefined")
    {
        FailureCause::Compile
    } else {
        FailureCause::Execute
    }
}

fn hint_for(cause: FailureCause) -> &'static str {
    match cause {
        FailureCause::Compile => {
            "check the model's SQL — run `smelt build --show-plan <file>` to see the compiled query"
        }
        FailureCause::Execute => {
            "re-run with -v for the full backend error, or `smelt run --show-plan` to inspect the plan"
        }
        FailureCause::Check => "run `smelt check` for the full check output",
    }
}

/// Print the end-of-run failure summary: one block naming every model that
/// failed this run, each with its first error line and a one-line hint
/// (`docs/plans/20260719-prod-w2-operability.md` Phase 8 TDD test
/// `failure_summary_lists_all_failed_models`; extended with hints by
/// `docs/plans/20260719-prod-w3-adoption.md` Phase 6). Reads the
/// just-written run report back from `.smelt/` — the report is the derived,
/// already-summarized view of the manifest (`docs/specs/run_state.md`
/// §"Run report"), so this prints from it rather than re-deriving the same
/// summary from the raw manifest. Silently does nothing if the report can't
/// be read back (e.g. a stateless project, or a pre-execution failure with
/// no run directory yet) — `run_failed`'s per-model lines above still ran
/// either way.
pub fn print_failure_summary(project_dir: &std::path::Path, target: &str, run_id: &str) {
    let file_store = smelt_state::file_store::FileStore::new(project_dir, target);
    let Ok(Some(report)) = file_store.load_report(run_id) else {
        return;
    };
    if report.failures.is_empty() {
        return;
    }
    eprintln!(
        "smelt: run {} failed — {} model(s) failed:",
        run_id,
        report.failures.len()
    );
    for failure in &report.failures {
        let first_line = failure.error.lines().next().unwrap_or(&failure.error);
        let cause = classify_failure_cause(&failure.error);
        eprintln!("  - {}: {}", failure.model, first_line);
        eprintln!("    hint: {}", hint_for(cause));
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
        ModelStrategy::MaterializedView => "materialized-view".to_string(),
        ModelStrategy::Ephemeral => "ephemeral".to_string(),
        ModelStrategy::Skipped { .. } => "skipped".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_backend_cast_error_as_execute() {
        let err = "Conversion Error: Could not convert string 'not_a_number' to INT32";
        assert_eq!(classify_failure_cause(err), FailureCause::Execute);
    }

    #[test]
    fn classifies_keyed_model_ddl_failure_as_execute() {
        let err = "Failed to create keyed model orders: Binder Error: table already exists";
        assert_eq!(classify_failure_cause(err), FailureCause::Execute);
    }

    #[test]
    fn classifies_constraint_violation_as_check() {
        let err = "Failed to create keyed model orders: constraint violation";
        assert_eq!(classify_failure_cause(err), FailureCause::Check);
    }

    #[test]
    fn classifies_unresolved_ref_as_compile() {
        let err = "Model 'orders' failed to compile: unresolved ref smelt.ref(\"missing\")";
        assert_eq!(classify_failure_cause(err), FailureCause::Compile);
    }

    #[test]
    fn classifies_check_failure_as_check() {
        let err = "Schema evolution check failed: incompatible column type change";
        assert_eq!(classify_failure_cause(err), FailureCause::Check);
    }

    #[test]
    fn every_cause_has_a_distinct_hint() {
        let hints = [
            hint_for(FailureCause::Compile),
            hint_for(FailureCause::Execute),
            hint_for(FailureCause::Check),
        ];
        for pair in hints.windows(2) {
            assert_ne!(pair[0], pair[1]);
        }
    }
}
