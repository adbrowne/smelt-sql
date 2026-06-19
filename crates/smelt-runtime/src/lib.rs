//! Compile + execute driver for smelt projects.
//!
//! `smelt-runtime` composes the analysis-layer crates (`smelt-parser`,
//! `smelt-db`, `smelt-core`, `smelt-planner`) into the lifecycle stages above
//! analysis: selection / filtering, SQL compilation (function-body
//! resolution, ephemeral inlining, type-cast wrapping, time-filter
//! injection), the pre-execution diagnostic gate, and the per-model execute
//! loop (batch dispatch, manifest writes, interval-store updates).
//!
//! Both `smelt-cli` and `smelt-ui` consume this crate through a single
//! `execute_project(request, reporter)` entry point and contribute only
//! surface concerns (argument parsing, progress reporting, HTTP
//! serialization). `smelt-lsp` does *not* depend on this crate; its needs
//! are met entirely by the analysis layer.
//!
//! See `docs/specs/architecture.md` → "Run pipeline parity rule (CLI ↔ UI)"
//! for the normative invariant.

pub mod compile;
pub mod cumulative;
pub mod execute;
pub mod fn_bodies;
pub mod gate;
pub mod meta_eval;
pub mod python;
pub mod reporter;
pub mod safety;
pub mod schema_evolution;
pub mod select;
pub mod transformer;
pub mod types;
pub mod windowing;

pub use compile::{
    build_source_bound_map, expand_function_calls, resolve_refs_in_sql,
    substitute_params_with_named, CompiledModel, CompilerRegistry, EphemeralResolver, SqlCompiler,
    UpstreamSchemas,
};
pub use cumulative::{
    build_cumulative_merge_sql, classify_cumulative_sql, execute_cumulative_aggregate,
};
pub use execute::{build_source_timeseries_map, execute_project, BackendFactory, BackendFuture};
pub use fn_bodies::{build_fn_body_map, build_fn_body_map_from_model_files, FnBodyMap};
pub use gate::{format_gate_errors, gate_diagnostics, GateDiagnostic};
pub use python::discover_python_models;
pub use reporter::{NoOpReporter, RunReporter};
pub use select::{select_executable_models, SelectionPlan, SelectionRequest};
pub use transformer::{
    inject_source_filters, inject_time_filter, SourceBound, TimeRange, TransformError,
};
pub use types::{ExecuteRequest, ModelPlanRecord, ModelStrategy, PlanSummary, RunOutcome};

#[cfg(test)]
mod tests {
    use super::*;

    // ─── inject_time_filter smoke tests (the helper is fully covered by
    //     transformer.rs's own #[cfg(test)] module; these are the
    //     phase-1 plan's TDD anchors against the new home) ─────────────

    #[test]
    fn test_inject_time_filter_appends_to_where() {
        let sql = "SELECT * FROM smelt.models.orders WHERE status = 'active'";
        let range = TimeRange {
            start: "2024-01-15".into(),
            end: "2024-01-18".into(),
        };
        let result = inject_time_filter(sql, "created_at", &range).unwrap();
        assert!(result.contains("WHERE status = 'active'"));
        assert!(result.contains("AND (created_at >= '2024-01-15' AND created_at < '2024-01-18')"));
    }

    #[test]
    fn test_inject_time_filter_creates_where_when_absent() {
        let sql = "SELECT * FROM smelt.models.orders";
        let range = TimeRange {
            start: "2024-01-15".into(),
            end: "2024-01-18".into(),
        };
        let result = inject_time_filter(sql, "created_at", &range).unwrap();
        assert!(
            result.contains("WHERE created_at >= '2024-01-15' AND created_at < '2024-01-18'"),
            "missing injected WHERE in: {result}"
        );
    }

    #[test]
    fn test_inject_time_filter_bails_without_from() {
        let sql = "SELECT 1 + 1";
        let range = TimeRange {
            start: "2024-01-15".into(),
            end: "2024-01-18".into(),
        };
        let result = inject_time_filter(sql, "created_at", &range);
        assert!(matches!(result, Err(TransformError::NoFromClause)));
    }

    // ─── RunReporter is object-safe and NoOpReporter swallows events ──

    #[test]
    fn test_run_reporter_default_no_op() {
        let r: Box<dyn RunReporter> = Box::new(NoOpReporter);
        // No-op reporter accepts every callback without panicking. We
        // exercise every method so the default-impl path is touched.
        r.run_started("rid", &["a".into()], 1);
        r.model_started("rid", "a", 0, 1);
        r.batch_completed("rid", "a", 0, 1, 42, std::time::Duration::from_millis(1));
        r.model_completed("rid", "a", 42, std::time::Duration::from_millis(1));
        r.run_completed("rid", 42, std::time::Duration::from_millis(1));
        r.run_failed("rid", Some("a"), "boom");
        r.run_cancelled("rid");
    }
}
