use super::*;
use anyhow::{bail, Result};
use smelt_backend::BackendError;
use smelt_logical::analysis::fingerprint::Projection as FingerprintProjection;
use smelt_logical::analysis::join_shape::JoinContext;
use smelt_logical::analysis::walk::model_property_vector;
use smelt_logical::maintenance::availability::StateAvailability;
use smelt_logical::maintenance::choice::{effective_override, resolve_cell_choice};
use smelt_logical::maintenance::derive::SourceReferentialIntegrity;
use smelt_logical::maintenance::repair::{discovery_posture, RepairDiscoveryPosture};
use smelt_logical::maintenance::{PlanCell, SourceFacts, Technique, Trigger};
use std::collections::HashSet;

#[allow(clippy::too_many_arguments)]
pub fn resolve_live_per_group_recompute_cell(
    sql: &str,
    table: &str,
    metadata: &smelt_core::ModelMetadata,
    sources: &[SourceFacts],
    explicitly_mutable: &HashSet<String>,
    technique_overrides: &[crate::types::CellTechniqueOverride],
    dialect: SqlDialect,
    supports_fingerprint_sidecar: bool,
    availability: &StateAvailability,
) -> Result<Option<LiveRepairCell>> {
    let Some(result) = crate::maintenance_availability::derive_resolved(
        sql,
        table,
        metadata,
        sources,
        explicitly_mutable,
        // Same `None`/`&[]` posture as the sibling resolvers above: the
        // repair cell's own admission (affected-key discovery + bounded
        // slice) reads neither the driving source's declared granularity,
        // nor declared key recurrences, nor a deployed-schema snapshot.
        None,
        &[],
        &[],
        &SourceReferentialIntegrity::new(),
        None,
        None,
        availability,
        &[],
    ) else {
        return Ok(None);
    };
    let cells_cfg: &[smelt_core::config::MaintenanceCellConfig] = metadata
        .maintenance
        .as_ref()
        .map(|m| m.cells.as_slice())
        .unwrap_or(&[]);
    let request_cells: Vec<smelt_core::config::MaintenanceCellConfig> = technique_overrides
        .iter()
        .map(|o| smelt_core::config::MaintenanceCellConfig {
            columns: o.columns.clone(),
            on: o.on.clone(),
            prefer: None,
            technique: Some(o.technique),
            write: None,
        })
        .collect();
    let combined_cells: Vec<smelt_core::config::MaintenanceCellConfig> = request_cells
        .iter()
        .cloned()
        .chain(cells_cfg.iter().cloned())
        .collect();

    for facts in sources {
        for trigger in [
            Trigger::NewData {
                source: facts.name.clone(),
            },
            Trigger::UpstreamMutation {
                source: facts.name.clone(),
            },
        ] {
            let sibling_cells: Vec<PlanCell> = result.plan.cells_for(&trigger).cloned().collect();
            if sibling_cells.is_empty() {
                continue;
            }
            let sibling_group_columns: Vec<Vec<String>> = sibling_cells
                .iter()
                .map(|c| {
                    result
                        .column_groups
                        .iter()
                        .find(|g| g.name() == c.group)
                        .map(|g| g.columns.clone())
                        .unwrap_or_default()
                })
                .collect();
            // Same dangling-pin fail-loud gate the sibling resolvers apply
            // (`docs/plans/20260808-membership-sensitivity.md` Phase 3).
            if let Some(dangling) = smelt_logical::maintenance::choice::unaddressed_technique_pin(
                &combined_cells,
                &facts.name,
                &sibling_group_columns,
            ) {
                bail!(
                    "MaintenanceUnboundedFootprint: cells[on: {}].technique pin (columns: \
                     {:?}) does not address any of this trigger's own derived column groups \
                     ({:?}) — a hard technique pin must name columns belonging to exactly one \
                     of the trigger's admitted cells, never columns absent from every one of \
                     them",
                    facts.name,
                    dangling.columns,
                    sibling_group_columns,
                );
            }
            for (cell, group_columns) in sibling_cells.iter().zip(sibling_group_columns.iter()) {
                if cell.technique != Technique::PerGroupRecompute {
                    continue;
                }
                // Fail-loud BEFORE the choice ladder: an unprovable group
                // key is an internal inconsistency, not an override
                // outcome.
                let key = repair_cell_key(cell)?;
                let write_pin = smelt_db::queries::maintenance::matching_write_pin(
                    cell,
                    &result.column_groups,
                    cells_cfg,
                )
                .and_then(|pin_name| smelt_logical::maintenance::lookup_write_pattern(&pin_name));
                let overrides = effective_override(
                    metadata
                        .maintenance
                        .as_ref()
                        .and_then(|m| m.defaults.as_ref()),
                    &combined_cells,
                    &facts.name,
                    group_columns,
                );
                let chosen = resolve_cell_choice(
                    Some(cell),
                    &trigger,
                    &overrides,
                    write_pin,
                    // `{recompute, PerGroupRecompute}` never contains
                    // `ColumnScopedMerge`, so the backend's MERGE capability
                    // is irrelevant to this resolver's resolvable set — the
                    // same `false` the membership resolver passes.
                    false,
                )
                .map_err(|refusal| anyhow::anyhow!(refusal.to_string()))?;
                // A `technique: recompute` pin (→ `RegionRecompute`) is
                // simply not THIS live cell: skip the source and let the
                // caller's own default apply, exactly as the sibling
                // resolvers do for a choice they have no lowering for. A
                // `write: diff_patch` pin routes here too, but only when its
                // underlying recompute is `PerGroupRecompute` — the sole
                // recompute `resolve_cell_choice` ever grants
                // `DeleteLeg::Complete` (the region `DeleteInsert` default
                // has no lowering and fails loud by name rather than
                // silently falling through to the default write);
                // [`resolve_repair_write`] carries the full decision table.
                let comparability = model_property_vector(sql, &JoinContext::new())
                    .map(|v| v.comparability)
                    .unwrap_or_default();
                let Some(write) = resolve_repair_write(
                    &chosen,
                    group_columns,
                    &comparability,
                    &cell.row_identity,
                    &cell.group,
                )?
                else {
                    continue;
                };
                // The cell's own derived slice — obligation 4's bounded
                // per-group read footprint, matched to the trigger's own
                // source (never another source's clamp).
                let Some(slice) = cell
                    .scans
                    .iter()
                    .find(|c| c.source == facts.name)
                    .or_else(|| cell.scans.first())
                else {
                    bail!(
                        "MaintenanceRepairSliceMissing: a Technique::PerGroupRecompute cell for \
                         group '{}' on source '{}' carries no derived ScanClamp — the bounded \
                         per-group read slice is admission obligation 4 and is never assumed",
                        cell.group,
                        facts.name,
                    );
                };
                // P9 (`docs/specs/incremental_models.md` §"The repair
                // family" — "Obligation 7 over a `mutable_snapshot`
                // source"): a source with no native change feed and no
                // tombstone/change history needs the group-grain sidecar
                // diff to witness a wholly-deleted group; every other
                // posture keeps the ordinary clamped current-source scan. A
                // `ChangeFeed` source has no discovery posture at all — the
                // repair family is refused for it upstream at derivation
                // time (`derive::derive_new_data`), so a live cell here
                // should never carry one; a `None` posture bails loud
                // rather than silently defaulting to a scan that may drop
                // rows.
                let Some(posture) = discovery_posture(facts.mutation) else {
                    bail!(
                        "MaintenanceRepairDiscoveryPostureMissing: a Technique::\
                         PerGroupRecompute cell for group '{}' resolved a change_feed source \
                         '{}' — the repair family has no fingerprint-sidecar discovery for a \
                         change feed and should never have admitted this cell",
                        cell.group,
                        facts.name,
                    );
                };
                let discovery = if posture == RepairDiscoveryPosture::SidecarDiff {
                    if !supports_fingerprint_sidecar {
                        return Err(BackendError::unsupported(
                            dialect.name(),
                            "group-grain fingerprint-sidecar affected-key discovery for a \
                             mutable_snapshot repair source (P9)",
                        )
                        .into());
                    }
                    let digest_columns: Vec<String> =
                        match cell.fingerprint_projections.get(&facts.name) {
                            Some(FingerprintProjection::Columns(cols)) => {
                                cols.iter().cloned().collect()
                            }
                            _ => Vec::new(),
                        };
                    if digest_columns.is_empty() {
                        bail!(
                            "MaintenanceRepairDigestColumnsMissing: a Technique::PerGroupRecompute \
                             cell for group '{}' resolved a MutableSnapshot delta posture on \
                             source '{}' with no P4 fingerprint projection columns — the \
                             group-grain sidecar digest has nothing to hash",
                            cell.group,
                            facts.name,
                        );
                    }
                    RepairDiscovery::SidecarDiff { digest_columns }
                } else {
                    RepairDiscovery::ClampedScan
                };
                return Ok(Some((
                    facts.name.clone(),
                    cell.clone(),
                    key,
                    slice.clone(),
                    write,
                    discovery,
                )));
            }
        }
    }
    Ok(None)
}
