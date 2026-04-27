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

pub use analysis::temporal::{
    analyze_temporal_dependencies, compute_effective_window, granularity_period_days,
    EffectiveWindow, TemporalDependency, TemporalOffset, TemporalSource,
};
pub use graph::{ModelGraph, ModelInfo};
pub use rules::incremental::{analyze_batch_safety, BatchSafety};
pub use rules::Planner;
pub use types::{
    ExecutionStep, Frontmatter, Granularity, IncrementalConfig, IncrementalSafetyOverrides,
    IncrementalStrategy, Opportunity, OpportunityData, Transformation, Weekday,
};
