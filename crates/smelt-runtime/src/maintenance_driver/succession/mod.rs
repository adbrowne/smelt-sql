//! The succession-patch technique's live-cell resolution and window-forward
//! dispatch (`docs/outcomes/20260906-scd2-keyed-succession/phases/
//! 05b-plan.md`) — a sibling of `key_addressed`/`column_scoped`/`membership`
//! but *not* a [`super::driver::WindowedKeyedRule`] impl: the trait's seams
//! are keyed-fold shaped (`WriteSuppression`, `KeyedWriteMechanism`,
//! `emit_create_table_as` from the step's delta), while the succession grain
//! needs a pre-write clock-tie probe, a two-statement transactional patch
//! group, and a second (tombstone) table's own DDL. `mod.rs` (this file) is
//! live-cell resolution; `execute.rs` is the step loop.

use std::collections::HashSet;

use anyhow::{bail, Result};

use smelt_core::sources::SourceInfo;
use smelt_core::ModelMetadata;
use smelt_logical::maintenance::availability::StateAvailability;
use smelt_logical::maintenance::succession::SuccessionRecipe;
use smelt_logical::maintenance::{SourceFacts, Technique};

mod execute;
pub use execute::{execute_succession_maintenance, rebuild_succession_state};

mod frontier;
pub(crate) use frontier::{build_succession_run_record, record_succession_frontiers};

#[cfg(test)]
mod tests;

/// A live succession-patch cell: the pure emitter-input recipe
/// (`SuccessionRecipe::from_verdict`) plus the physically resolved table
/// names and the driving source's own run axis — everything
/// [`execute_succession_maintenance`] needs, resolved exactly once here
/// (`CLAUDE.md` §"Maintenance-plan purity").
#[derive(Debug, Clone)]
pub struct SuccessionCell {
    pub recipe: SuccessionRecipe,
    /// `schema.table` — the presented (patched) target.
    pub presented_table: String,
    /// The driving source's own physical table name, resolved via
    /// `SourceInfo::db_name_for_target` — never the classifier's raw
    /// comparison spelling (`recipe.source_table`).
    pub source_table: String,
    /// The driving source's own declared `timeseries.partition_column` — the
    /// run axis a succession model steps over, deliberately distinct from
    /// `recipe.clock_col` for an arrival-partitioned source
    /// (`docs/specs/incremental_shapes.md` §"Run shape and late events").
    pub partition_column: String,
    pub granularity: smelt_core::config::Granularity,
}

/// Resolve a live `Technique::SuccessionPatch` cell for `table`, or `Ok(None)`
/// when the model is not `refresh: incremental`, its grain is declared (not
/// undeclared-succession), the classifier's own verdict is `NotSuccession`,
/// or the cell was state-downgraded to `Technique::DeleteInsert`
/// (`smelt_logical::maintenance::availability::resolve_availability`, already
/// applied by [`crate::maintenance_availability::derive_resolved`]).
///
/// `source_refs` is the `(bare source name, SourceInfo)` side channel the
/// keyed-succession classifier's `SuccessionContext` is built from
/// (`smelt_db::queries::maintenance::build_succession_context`) — build via
/// [`build_succession_source_refs`].
///
/// Unresolvable physical facts (the recipe's driving source has no matching
/// declaration, or that declaration has no `timeseries:` partition column)
/// refuse by name rather than silently degrading — the emitters need both to
/// build the window predicate and the event-delta `SELECT`.
#[allow(clippy::too_many_arguments)]
pub fn resolve_live_succession_cell(
    sql: &str,
    table: &str,
    metadata: &ModelMetadata,
    sources: &[SourceFacts],
    explicitly_mutable: &HashSet<String>,
    source_refs: &[(String, Option<SourceInfo>)],
    availability: &StateAvailability,
    schema: &str,
    model_target: &str,
    source_infos: &[SourceInfo],
) -> Result<Option<SuccessionCell>> {
    let Some(result) = crate::maintenance_availability::derive_resolved(
        sql,
        table,
        metadata,
        sources,
        explicitly_mutable,
        None,
        &[],
        &[],
        &smelt_logical::maintenance::derive::SourceReferentialIntegrity::new(),
        None,
        None,
        availability,
        source_refs,
    ) else {
        return Ok(None);
    };

    let is_live = result
        .plan
        .cells
        .iter()
        .any(|c| c.technique == Technique::SuccessionPatch);
    if !is_live {
        return Ok(None);
    }
    let Some(recipe) = result.succession_recipe else {
        return Ok(None);
    };

    let bare_source = recipe
        .source_table
        .strip_prefix("sources.")
        .unwrap_or(&recipe.source_table);
    let Some(info) = source_infos.iter().find(|info| {
        let segs = &info.address_segments;
        let bare = match segs.split_first() {
            Some((first, rest)) if first == "sources" => rest.join("."),
            _ => segs.join("."),
        };
        bare == bare_source
    }) else {
        bail!(
            "succession-patch cell for model '{table}' names driving source '{bare_source}', \
             but no matching source declaration was found — the physical source table cannot be \
             resolved"
        );
    };
    let Some(ts) = info.timeseries.as_ref() else {
        bail!(
            "succession-patch cell for model '{table}' drives off source '{bare_source}', but \
             that source declares no `timeseries:` block — the window-forward driver cannot \
             step it"
        );
    };
    let source_table = info.db_name_for_target(model_target, schema);
    let presented_table = format!("{schema}.{table}");

    Ok(Some(SuccessionCell {
        recipe,
        presented_table,
        source_table,
        partition_column: ts.partition_column.clone(),
        granularity: ts.granularity,
    }))
}

/// Build the `(bare source name, SourceInfo)` side channel
/// [`resolve_live_succession_cell`]'s `source_refs` argument needs, from a
/// model's own `refs` and the project's discovered `source_infos` — the
/// succession-specific counterpart of `build_maint_source_facts`
/// (`crate::execute::key_addressed`), over the SAME `(ref, source_info)`
/// pairs, matched the same bare-name way.
pub fn build_succession_source_refs(
    model_file: &smelt_core::ModelFile,
    source_infos: &[SourceInfo],
) -> Vec<(String, Option<SourceInfo>)> {
    model_file
        .refs
        .iter()
        .filter_map(|r| {
            let segs = r.smelt_ref.to_path();
            let info = source_infos.iter().find(|s| s.address_segments == segs)?;
            let bare = match segs.split_first() {
                Some((first, rest)) if first == "sources" => rest.join("."),
                _ => segs.join("."),
            };
            Some((bare, Some(info.clone())))
        })
        .collect()
}
