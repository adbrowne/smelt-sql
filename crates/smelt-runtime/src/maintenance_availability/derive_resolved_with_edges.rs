use std::collections::HashSet;

use smelt_core::config::Granularity;
use smelt_core::sources::{KeyRecurrence, SourceInfo};
use smelt_core::ModelMetadata;
use smelt_logical::maintenance::availability::{resolve_availability, StateAvailability};
use smelt_logical::maintenance::derive::{ModelEdge, SourceReferentialIntegrity};
use smelt_logical::maintenance::SourceFacts;

use smelt_db::queries::maintenance::MaintenancePlanResult;

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
    source_refs: &[(String, Option<SourceInfo>)],
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
        source_refs,
    )?;
    resolve_availability(&mut result.plan.cells, availability);
    Some(result)
}
