//! Thin Salsa wrapper around `smelt_logical::maintenance::derive::derive_maintenance_plan`
//! (`incremental_models.md` §Surface "The plan (derived, reported)").
//!
//! Per the Salsa purity rule (`architecture.md` §"Salsa purity rule
//! (analysis)"), this module only *assembles inputs* — resolved source
//! facts, the declared output shape, the derived column groups/skeleton —
//! and calls the pure derivation in `smelt-logical`. It never re-implements
//! admission, locality, or ledger logic itself. The `#[salsa::tracked]`
//! query in `smelt-db/src/lib.rs` (`maintenance_plan`) is the only caller;
//! everything below is a plain function so it can be unit-tested without a
//! Salsa database.

use std::collections::HashMap;

use smelt_core::config::{
    Grain as ConfigGrain, Granularity, MaintenanceConfig, RefreshStrategy, ScanBoundsConfig,
    ScanBoundsRequire, ScanBoundsViolation,
};
use smelt_core::sources::{MutationProfile as SourceMutationKind, SourceInfo};
use smelt_core::ModelMetadata;
use smelt_logical::analysis::{select_stmt_items, SelectItemKind};
use smelt_logical::maintenance::derive::{
    derive_maintenance_plan_with_referential_integrity, FoldSpec, ModelInputs,
    SourceReferentialIntegrity,
};
use smelt_logical::maintenance::granularity::{check_declared_granularity, GranularityMismatch};
use smelt_logical::maintenance::grouping::{derive_column_groups, DegenerateColumn};
use smelt_logical::maintenance::locality::{
    establish_locality, partition_column_provably_not_null, single_clocked_granularity,
    LocalityInputs,
};
use smelt_logical::maintenance::skeleton::skeleton_columns;
use smelt_logical::maintenance::{
    identity_not_derivable_plan, locality_refused_plan, recurrence_mismatch_plan, ColumnGroup,
    Grain as PlanGrain, MaintenancePlan, MutationProfile as PlanMutationProfile, OutputSpec,
    SourceFacts, Trigger,
};
use smelt_logical::rules::cumulative::{
    declared_unique_key_matches, group_by_unique_key as derive_group_by_unique_key,
    OnceWriteAdmission,
};
use smelt_types::SqlFunction;

/// Everything `maintenance_plan` derives for one model: the raw plan (cells
/// and admission refusals) plus the column groups the `maintenance.cells[]`
/// frontmatter check reuses — a single derivation feeds both, per the
/// maintenance-plan-purity invariant ("derived once by pure functions;
/// consumers never re-derive it").
#[derive(Debug, Clone, Default)]
pub struct MaintenancePlanResult {
    pub plan: MaintenancePlan,
    pub column_groups: Vec<ColumnGroup>,
    /// Every column whose provenance couldn't be resolved and whose
    /// derivation fell back to the whole-model group
    /// (`grouping::derive_column_groups`'s `GroupingResult::degenerate`).
    /// Non-empty here is the only reliable signal of a genuine whole-model
    /// collapse — `column_groups.len() == 1` alone is neither necessary nor
    /// sufficient (a legitimately single-group model with 2+ mutable
    /// sources is not degenerate; a degenerate collapse against a
    /// single-source model still has `column_groups.len() == 1` with only
    /// one source in `mutation_sensitivity`).
    pub degenerate: Vec<DegenerateColumn>,
    /// This model's decomposed-state summary — one entry per presented
    /// column that folds through hidden state columns
    /// (`docs/outcomes/20260809-rung2-state-shapes` row 9), empty for a
    /// rung-1 model or one this function derives without classifying (every
    /// site here except `smelt_db::maintenance_plan_report`, which is the
    /// single caller that runs the keyed classifier and populates this
    /// field — `smelt-db/src/lib.rs`'s Salsa purity rule: this crate's own
    /// internal derivation never re-decides which columns are
    /// state-bearing).
    pub state_columns: Vec<smelt_logical::StateColumnSummary>,
    /// This model's three derived execution postures
    /// (`incremental_shapes.md` §"Derived execution postures",
    /// `docs/outcomes/20260815-keyed-grain-residue` phase 4) — `None` for a
    /// model that never classifies as `grain: key` (nothing to derive
    /// postures over), populated by the same `smelt-db/src/lib.rs` caller
    /// that fills `state_columns` from the same classification call.
    pub execution_postures: Option<smelt_logical::ExecutionPostures>,
    /// The run shape [`execution_postures`] qualifies — `Some(true)` for
    /// snapshot-reconcile (zero clocked driving sources), `Some(false)` for
    /// window-forward, `None` alongside `execution_postures: None`. A
    /// second field rather than folded into `ExecutionPostures` itself: the
    /// run shape depends on the classification's `driving_source`, not on
    /// `aggregator_columns` alone, so it can't be derived by
    /// `execution_postures`'s pure column-slice signature.
    pub is_snapshot_reconcile: Option<bool>,
    /// This model's per-column change-comparability (P3,
    /// `model_properties.md` §"Change comparability") — the SAME
    /// `analysis::walk::model_property_vector` call `derive_fold_spec` (or,
    /// for a `grain: partition` model with no fold spec, a dedicated call
    /// below) already makes, surfaced here so a `write:` pin's equivalence
    /// proof ([`smelt_logical::maintenance::cell_equivalence_proof`]) and
    /// `smelt explain` both read the one derivation rather than re-walking
    /// the model's SQL (`CLAUDE.md` §"Maintenance-plan purity"). Empty for
    /// every early-refusal path (`key_per_partition`, a declared/derived
    /// `unique_key` mismatch, a locality refusal) — those never reach the
    /// walk at all.
    pub comparability: Vec<smelt_logical::analysis::walk::ColumnComparability>,
    /// The keyed-succession classifier's `Recognized`-verdict advisories
    /// (`docs/outcomes/20260906-scd2-keyed-succession/outcome.md` criterion
    /// 2) — populated only on the succession branch of
    /// `derive_model_maintenance_plan` (`metadata.resolved_grain()` is
    /// `None`), empty everywhere else. Never changes `plan` (structurally
    /// carried off it, see `smelt_logical::maintenance::succession::
    /// SuccessionDerivation`'s own doc comment).
    pub succession_advisories: Vec<smelt_logical::analysis::succession::SuccessionAdvisory>,
    /// Every argument the succession-patch emitters take, derived once from
    /// the classifier's verdict (`smelt_logical::maintenance::succession::
    /// SuccessionRecipe::from_verdict`) — populated only on the same
    /// succession branch as `succession_advisories`, `None` everywhere else
    /// including a `NotSuccession` verdict on that branch. The runtime
    /// driver (`docs/outcomes/20260906-scd2-keyed-succession/
    /// phases/05b-plan.md`) reads this rather than re-parsing the model's
    /// SQL (`CLAUDE.md` §"Maintenance-plan purity").
    pub succession_recipe: Option<smelt_logical::maintenance::succession::SuccessionRecipe>,
}

mod diagnostics;
mod facts_and_fold;
mod plan;
mod plan_helpers;
mod refusal_diag;
#[cfg(test)]
mod tests;
mod write_pin;

pub use diagnostics::maintenance_plan_diagnostics;
pub use facts_and_fold::{derive_fold_spec, effective_scan_bounds, source_facts};
pub use plan::{derive_model_maintenance_plan, derive_model_maintenance_plan_with_edges};
pub use plan_helpers::{
    build_key_recurrences, build_source_facts, build_source_referential_integrity,
    build_succession_context, cell_column_group_violations, single_clocked_source_granularity,
};
pub use refusal_diag::{
    diagnostic_for_refusal, ContractStateRefusalDiagnostic, MaintenancePlanDiagnostics,
    MaintenanceRefusal, StateDowngradeDiagnostic, WritePinDiagnostic,
};
pub use write_pin::{
    backend_dialect_for, backend_write_capabilities_for, keyed_fold_effective_override,
    keyed_fold_write_pin, matching_write_pin, write_pin_diagnostics,
};
