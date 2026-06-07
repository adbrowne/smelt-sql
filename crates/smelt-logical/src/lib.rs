pub mod analysis;
pub mod graph;
pub mod logical;
pub mod lowering;
pub mod plan_builder;
pub mod rules;
pub mod types;

pub use analysis::source_bounds::{BoundContext, BoundResult, Seconds};
pub use analysis::temporal::{
    analyze_temporal_dependencies, compute_effective_window, granularity_period_days,
    EffectiveWindow, TemporalDependency, TemporalOffset, TemporalSource,
};
pub use analysis::{analyze_select, SelectAnalysis, SelectItemKind};
pub use graph::{ModelGraph, ModelInfo};
pub use logical::{
    parse_function_properties, Cardinality, FnId, FunctionProperties, JoinSpec, LogicalNode, Plan,
    Provenance, ProvenanceTag,
};
pub use lowering::as_struct::{as_struct_to_sql, backend_supports_struct_literal};
pub use plan_builder::{build_logical_plan_pure, FnCallInput};
pub use rules::cumulative::{
    classify_cumulative, combiner_for, AggregatorColumn, CrossPartitionCombiner,
    CumulativeClassification, CumulativeDiagnostic, DrivingSource, SourceTimeseriesMap,
};
pub use rules::incremental::{analyze_batch_safety, derive_model_source_bounds, BatchSafety};
pub use rules::rule_diagnostics::{
    collect_path_refs, detect_builtin_rules, CumulativeRule, IncrementalRule, PlannerRule,
    RuleContext, RuleDiagnostic, RuleDiagnosticCode, RuleSeverity,
};
pub use types::{
    ExecutionStep, Frontmatter, Granularity, IncrementalConfig, IncrementalSafetyOverrides,
    IncrementalStrategy, Opportunity, OpportunityData, Transformation, Weekday,
};
