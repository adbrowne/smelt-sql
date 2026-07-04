pub mod analysis;
pub mod graph;
pub mod logical;
pub mod logical_plan_rules;
pub mod lowering;
pub mod plan_printer;
#[cfg(feature = "python")]
pub mod python_bridge;
pub mod rules;
pub mod types;

pub use analysis::source_bounds::{BoundContext, BoundResult, Seconds};
pub use analysis::temporal::{
    analyze_temporal_dependencies, compute_effective_window, granularity_period_days,
    EffectiveWindow, TemporalDependency, TemporalOffset, TemporalSource,
};
pub use graph::{ModelGraph, ModelInfo};
pub use rules::cumulative::{
    classify_cumulative, combiner_for, AggregatorColumn, CrossPartitionCombiner,
    CumulativeClassification, CumulativeDiagnostic, DrivingSource, SourceTimeseriesMap,
};
pub use rules::incremental::{analyze_batch_safety, derive_model_source_bounds, BatchSafety};
pub use rules::rule_diagnostics::{
    collect_path_refs, detect_builtin_rules, CumulativeRule, IncrementalRule, PlannerRule,
    RuleContext, RuleDiagnostic, RuleDiagnosticCode, RuleSeverity,
};
pub use rules::Planner;
pub use types::{
    BatchedConfig, BatchedSafetyOverrides, ExecutionStep, Frontmatter, Granularity,
    IncrementalStrategy, Opportunity, OpportunityData, Transformation, Weekday,
};
