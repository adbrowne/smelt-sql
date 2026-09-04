//! The single `smelt-runtime` derivation seam
//! (`docs/outcomes/20260904-state-residency/outcome.md` phase 5): every
//! runtime consumer of `smelt-db`'s `derive_model_maintenance_plan{,
//! _with_edges}` reads the result through [`derive_resolved`] /
//! [`derive_resolved_with_edges`] instead, which apply
//! [`smelt_logical::maintenance::availability::resolve_availability`]
//! before any caller sees a cell (`state.md` §"The degradation contract",
//! step 2). No other module in this crate may call the `smelt-db` functions
//! directly — enforced structurally by
//! `crates/smelt-runtime/tests/availability_seam.rs`'s
//! `every_runtime_derivation_goes_through_the_availability_seam`.

use std::collections::HashSet;

use smelt_core::config::{Config, Granularity};
use smelt_core::sources::KeyRecurrence;
use smelt_core::ModelMetadata;
use smelt_dialect::SqlDialect;
use smelt_logical::maintenance::availability::{
    realisable_state_structures, resolve_availability, StateAvailability,
};
use smelt_logical::maintenance::derive::{ModelEdge, SourceReferentialIntegrity};
use smelt_logical::maintenance::SourceFacts;

use smelt_db::queries::maintenance::MaintenancePlanResult;

/// The availability a run's target actually has: the intersection of what
/// `dialect` can realise and what `config.state.warehouse_tables` permits
/// (`state.md` §"Opting out of warehouse bookkeeping"). Built once per
/// target, not per model — a run's dialect and `state.warehouse_tables`
/// never vary across the models it drives against the same target.
pub fn availability_for_run(dialect: SqlDialect, config: &Config) -> StateAvailability {
    StateAvailability::resolve(
        config.state.warehouse_tables,
        &realisable_state_structures(dialect),
    )
}

/// Availability-resolved wrapper over
/// [`smelt_db::queries::maintenance::derive_model_maintenance_plan`] — the
/// only place in `smelt-runtime` that function may be called from.
#[allow(clippy::too_many_arguments)]
pub fn derive_resolved(
    sql: &str,
    table: &str,
    metadata: &ModelMetadata,
    sources: &[SourceFacts],
    explicitly_mutable: &HashSet<String>,
    driving_source_granularity: Option<Granularity>,
    key_recurrences: &[(String, KeyRecurrence)],
    deployed_column_names: &[String],
    source_referential_integrity: &SourceReferentialIntegrity,
    deployed_model_sql: Option<&str>,
    deployed_partition_column: Option<&str>,
    availability: &StateAvailability,
) -> Option<MaintenancePlanResult> {
    let mut result = smelt_db::queries::maintenance::derive_model_maintenance_plan(
        sql,
        table,
        metadata,
        sources,
        explicitly_mutable,
        driving_source_granularity,
        key_recurrences,
        deployed_column_names,
        source_referential_integrity,
        deployed_model_sql,
        deployed_partition_column,
    )?;
    resolve_availability(&mut result.plan.cells, availability);
    Some(result)
}

/// Availability-resolved wrapper over
/// [`smelt_db::queries::maintenance::derive_model_maintenance_plan_with_edges`]
/// — the only place in `smelt-runtime` that function may be called from.
#[allow(clippy::too_many_arguments)]
pub fn derive_resolved_with_edges(
    sql: &str,
    table: &str,
    metadata: &ModelMetadata,
    sources: &[SourceFacts],
    explicitly_mutable: &HashSet<String>,
    model_edges: &[ModelEdge],
    driving_source_granularity: Option<Granularity>,
    key_recurrences: &[(String, KeyRecurrence)],
    deployed_column_names: &[String],
    source_referential_integrity: &SourceReferentialIntegrity,
    deployed_model_sql: Option<&str>,
    deployed_partition_column: Option<&str>,
    availability: &StateAvailability,
) -> Option<MaintenancePlanResult> {
    let mut result = smelt_db::queries::maintenance::derive_model_maintenance_plan_with_edges(
        sql,
        table,
        metadata,
        sources,
        explicitly_mutable,
        model_edges,
        driving_source_granularity,
        key_recurrences,
        deployed_column_names,
        source_referential_integrity,
        deployed_model_sql,
        deployed_partition_column,
    )?;
    resolve_availability(&mut result.plan.cells, availability);
    Some(result)
}
