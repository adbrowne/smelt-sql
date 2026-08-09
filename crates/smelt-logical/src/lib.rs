pub mod analysis;
pub mod backbuild;
pub mod data_tests;
pub mod graph;
pub mod logical;
pub mod lowering;
pub mod maintenance;
pub mod plan_builder;
pub mod rules;
pub mod types;

pub use analysis::monotonicity::{
    trace_event_time, trace_event_time_declared, EventTimeTrace, Monotonicity, NotTraceableKind,
    Offset,
};
pub use analysis::source_bounds::{BoundContext, BoundResult, InjectionPoint, Seconds};
pub use analysis::temporal::{
    analyze_temporal_dependencies, compute_effective_window, granularity_period_days,
    EffectiveWindow, TemporalDependency, TemporalOffset, TemporalSource,
};
pub use analysis::{analyze_select, SelectAnalysis, SelectItemKind};
pub use data_tests::{
    lower_column_test, resolve_not_null_verdict, resolve_unique_verdict, ScanLowering, TestVerdict,
};
pub use graph::{ModelGraph, ModelInfo};
pub use logical::{
    parse_function_properties, Cardinality, FnId, FunctionProperties, JoinSpec, LogicalNode, Plan,
    Provenance, ProvenanceTag,
};
pub use lowering::as_struct::{as_struct_to_sql, backend_supports_struct_literal};
pub use plan_builder::{build_logical_plan_pure, FnCallInput};
pub use rules::cumulative::{
    classify_cumulative, combiner_for, group_by_unique_key, state_column_summary, AggregatorColumn,
    CrossPartitionCombiner, CumulativeClassification, DrivingSource, KeyedDiagnostic,
    SourceTimeseriesMap, StateColumnSummary,
};
pub use rules::incremental::{
    analyze_batch_safety, batch_safety_from_bounds, derive_model_source_bounds, BatchSafety,
};
pub use rules::rule_diagnostics::{
    collect_path_refs, detect_builtin_rules, IncrementalRule, KeyedRule, PlannerRule, RuleContext,
    RuleDiagnostic, RuleDiagnosticCode, RuleSeverity,
};
pub use types::{
    ExecutionStep, Frontmatter, Granularity, IncrementalStrategy, Opportunity, OpportunityData,
    PartitionGrainConfig, PartitionGrainSafetyOverrides, Transformation, Weekday,
};
