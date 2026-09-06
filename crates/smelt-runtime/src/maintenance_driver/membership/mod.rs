use super::*;
use anyhow::{bail, Result};
use smelt_logical::analysis::join_shape::JoinContext;
use smelt_logical::analysis::walk::model_property_vector;
use smelt_logical::maintenance::availability::StateAvailability;
use smelt_logical::maintenance::choice::{
    effective_override, resolve_cell_choice, resolve_cell_write_suppression, ChosenTechnique,
    WriteSuppression,
};
use smelt_logical::maintenance::derive::SourceReferentialIntegrity;
use smelt_logical::maintenance::{PlanCell, RowIdentity, SourceFacts, Technique, Trigger};
use std::collections::HashSet;

/// Find the first `explicitly_mutable` source whose `Trigger::
/// UpstreamMutation` cell resolves live to `Technique::DeleteInsert` — the
/// membership-sensitive counterpart of [`resolve_live_column_scoped_cell`]
/// above, added for the keyed run loop (`docs/plans/
/// 20260808-membership-sensitivity.md` Phase 2) and extended to the keyless
/// (`RowIdentity::WholeRow`) shape by `docs/outcomes/
/// 20260815-definition-delta-migrate/phases/27c-plan.md` — this function's
/// caller in `execute.rs`'s non-keyed batch loop consumes only the keyless
/// arm (`MembershipRecomputeWrite::StagedKeyless`); the keyed
/// (`StagedRecompute`/`DiffPatch`) arms stay the keyed-run-loop's own
/// concern, called from `execute.rs`'s `plan_is_keyed` branch.
///
/// Per `incremental_models.md` §"The plan matrix": "A membership-sensitive
/// group … must be repaired by a technique that can create and delete rows:
/// the recompute family (delete+insert, change-suppressed where the staged
/// candidate is comparable), never a column-scoped merge, which cannot fix
/// which rows exist." `derive_model_maintenance_plan` (Phase 1 of that plan)
/// now assigns exactly such a cell `Technique::DeleteInsert` +
/// `Corner::RecomputeRegion` for a membership-sensitive column group.
///
/// A `Technique::DeleteInsert` cell with `RowIdentity::Key(key)` where `key`
/// is empty is skipped (a degenerate proof this resolver has never had a
/// lowering for) — every other `Key(_)`/`WholeRow` cell is surfaced, with
/// the row-identity shape deciding which `MembershipRecomputeWrite` arm the
/// caller receives: `Key(_)` resolves through the same keyed proof this
/// function always ran (`resolve_cell_write_suppression`/`emit_staged_
/// candidate_conditional_recompute`); `WholeRow` resolves through
/// `smelt_logical::maintenance::choice::resolve_keyless_staged_suppression`
/// over the model's full payload column set and
/// `smelt_logical::maintenance::emit::
/// emit_staged_candidate_conditional_keyless`.
///
/// This function only ever surfaces a cell when [`resolve_write_variant`]
/// resolves `WriteSuppression::Suppressed` — `emit_staged_candidate_
/// conditional` has no unconditional counterpart (unlike the column-scoped
/// `MERGE` family), so an `Unconditional`/refused verdict falls through to
/// `None`, same fail-soft posture `resolve_live_column_scoped_cell` already
/// has for its own write-variant dimension (see that function's own doc
/// comment on the "known gap" this mirrors).
///
/// **Departed keys.** Dispatches to [`smelt_logical::maintenance::emit::
/// emit_staged_candidate_conditional_recompute`] (`docs/plans/
/// 20260808-membership-sensitivity.md` Phase 3), not the region-scoped
/// [`smelt_logical::maintenance::emit::emit_staged_candidate_conditional`] —
/// this resolver's `candidate_select` is always the model's own FULL
/// (unwindowed) recompute, so a stored row whose key is entirely absent from
/// it has genuinely *departed* (e.g. the dimension row a fact joined on was
/// itself deleted) rather than merely being out of a run's touched region,
/// and the recompute variant's extra anti-join `DELETE` removes it.
/// The write route a live membership-sensitive `Technique::DeleteInsert`
/// cell ([`resolve_live_membership_recompute_cell`]) resolves to — the
/// staged-candidate conditional recompute
/// ([`execute_staged_membership_recompute`]), or a `write: diff_patch` pin
/// over that same cell, routed through [`execute_diff_patch`] instead
/// (`docs/outcomes/20260815-definition-delta-migrate/phases/12-plan.md`).
/// The `diff_patch` leg's delete-leg completeness is sound unconditionally:
/// this resolver's candidate is always the model's own FULL (unwindowed)
/// recompute — a region recompute's own coverage IS its slice, by
/// construction, mirroring `resolve_cell_choice`'s `DeleteLeg::Complete`
/// grant for the region `DeleteInsert` default.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MembershipRecomputeWrite {
    /// `emit_staged_candidate_conditional_recompute`'s change-suppressed
    /// conditional `DELETE`+`INSERT`.
    StagedRecompute { compared_columns: Vec<String> },
    /// [`emit_diff_patch`]'s diff-then-patch pattern, admitted via
    /// [`smelt_logical::maintenance::diff_patch::admit_diff_patch`].
    DiffPatch { compared_columns: Vec<String> },
    /// [`smelt_logical::maintenance::emit::emit_staged_candidate_conditional_keyless`]'s
    /// region-grained whole-row conditional `DELETE`+`INSERT`
    /// (`docs/outcomes/20260815-definition-delta-migrate/phases/27c-plan.md`)
    /// — the `RowIdentity::WholeRow` realisation, reached only when
    /// [`smelt_logical::maintenance::choice::resolve_keyless_staged_suppression`]
    /// admits over the model's full payload column set.
    StagedKeyless { compared_columns: Vec<String> },
}

/// A live membership-recompute cell as
/// [`resolve_live_membership_recompute_cell`] returns it: the trigger's
/// source name, the cell itself, its column group's own derived columns,
/// and the resolved write route.
pub type LiveMembershipRecomputeCell = (String, PlanCell, Vec<String>, MembershipRecomputeWrite);

pub fn resolve_live_membership_recompute_cell(
    sql: &str,
    table: &str,
    metadata: &smelt_core::ModelMetadata,
    sources: &[SourceFacts],
    explicitly_mutable: &HashSet<String>,
    technique_overrides: &[crate::types::CellTechniqueOverride],
    availability: &StateAvailability,
) -> Result<Option<LiveMembershipRecomputeCell>> {
    let Some(result) = crate::maintenance_availability::derive_resolved(
        sql,
        table,
        metadata,
        sources,
        explicitly_mutable,
        None,
        &[],
        // This resolver only inspects `UpstreamMutation` cells — a
        // `ColumnAdded` trigger never affects them, so no deployed-schema
        // snapshot is needed here.
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
    for source in explicitly_mutable {
        let trigger = Trigger::UpstreamMutation {
            source: source.clone(),
        };
        // Same sibling-cell fix as `resolve_live_column_scoped_cell` above
        // (`docs/plans/20260808-membership-sensitivity.md` Phase 3) — a
        // trigger can derive multiple membership-sensitive sibling cells,
        // and a `cells[]` override must be matched against each one's own
        // columns, never only the first.
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
        if let Some(dangling) = smelt_logical::maintenance::choice::unaddressed_technique_pin(
            &combined_cells,
            source,
            &sibling_group_columns,
        ) {
            bail!(
                "MaintenanceUnboundedFootprint: cells[on: {source}].technique pin (columns: \
                 {:?}) does not address any of this trigger's own derived column groups ({:?}) \
                 — a hard technique pin must name columns belonging to exactly one of the \
                 trigger's admitted cells, never columns absent from every one of them",
                dangling.columns,
                sibling_group_columns,
            );
        }
        for (cell, group_columns) in sibling_cells.iter().zip(sibling_group_columns.iter()) {
            if cell.technique != Technique::DeleteInsert {
                continue;
            }
            // A proven `RowIdentity::Key` (non-empty) routes through the
            // keyed staged-candidate/diff_patch legs below; `RowIdentity::
            // WholeRow` routes through the keyless leg (`docs/outcomes/
            // 20260815-definition-delta-migrate/phases/27c-plan.md`) instead
            // of being skipped outright — a `Key(vec![])` is a degenerate
            // proof this resolver has never had a lowering for and stays
            // skipped.
            let key: Option<&Vec<String>> = match &cell.row_identity.identity {
                RowIdentity::Key(key) if !key.is_empty() => Some(key),
                RowIdentity::Key(_) => continue,
                RowIdentity::WholeRow => None,
            };
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
                source,
                group_columns,
            );
            // `resolve_cell_choice`'s resolvable set for this cell is `{recompute,
            // DeleteInsert}` (the cell's own admitted technique IS the always-
            // available region recompute for this family, per `resolve_cell_
            // choice`'s own doc comment: "the second live alternative … is the
            // always-admissible whole-region recompute"). Absent an override that
            // asks for something this narrow resolver has no lowering for, both
            // resolvable members land here as `Admitted(Technique::DeleteInsert)`
            // — a `RegionRecompute` choice from a `technique: recompute` pin/
            // `prefer` is handled the same way `resolve_live_column_scoped_cell`
            // handles it: it simply isn't THIS live cell, so this source is
            // skipped and the caller's own default (the plain incremental batch
            // loop, unaware of this dimension) applies.
            let chosen = resolve_cell_choice(
                Some(cell),
                &trigger,
                &overrides,
                write_pin,
                // Column-scoped MERGE backend capability is irrelevant to this
                // resolver's own resolvable set (`{recompute, DeleteInsert}`
                // never contains `ColumnScopedMerge`) — passed `false` so a
                // `write_pin`/pin naming `ColumnScopedMerge` correctly refuses
                // here rather than appearing spuriously "live".
                false,
            )
            .map_err(|refusal| anyhow::anyhow!(refusal.to_string()))?;
            let comparability = model_property_vector(sql, &JoinContext::new())
                .map(|v| v.comparability)
                .unwrap_or_default();
            match chosen {
                ChosenTechnique::Admitted(Technique::DeleteInsert) if key.is_some() => {
                    // A `technique: suppress` pin whose P2/P3 proof refused
                    // propagates as a real run error (`incremental_models.md`
                    // §"Per-cell write addressing" → "User pins") — never a
                    // silent fallback to region recompute.
                    let suppression =
                        resolve_cell_write_suppression(sql, group_columns, cell, &overrides)
                            .map_err(|refusal| anyhow::anyhow!(refusal.to_string()))?;
                    // `emit_staged_candidate_conditional` has no
                    // unconditional counterpart (unlike
                    // `emit_column_scoped_merge`/`emit_column_scoped_merge_
                    // suppressed`) — an `Unconditional` verdict here has no
                    // sound lowering this resolver can hand the caller, so
                    // it is treated exactly like a refused write-variant:
                    // skip this source, fall through to the caller's safe
                    // default.
                    let WriteSuppression::Suppressed { compared_columns } = suppression else {
                        continue;
                    };
                    return Ok(Some((
                        source.clone(),
                        cell.clone(),
                        group_columns.clone(),
                        MembershipRecomputeWrite::StagedRecompute { compared_columns },
                    )));
                }
                ChosenTechnique::Admitted(Technique::DeleteInsert) => {
                    // `key.is_none()` here — a `RowIdentity::WholeRow` cell
                    // (`docs/outcomes/20260815-definition-delta-migrate/
                    // phases/27c-plan.md`). `resolve_write_suppression`
                    // (the keyed proof `resolve_cell_write_suppression`
                    // calls) refuses solely because the identity is
                    // `WholeRow`, before it ever inspects column
                    // comparability — so this arm never calls it, and
                    // instead runs the keyless proof directly over the
                    // model's full payload column set (every selected
                    // column participates in a whole-row diff, not just this
                    // cell's own mutation-sensitive group).
                    let output_columns: Vec<String> = result
                        .column_groups
                        .iter()
                        .flat_map(|g| g.columns.clone())
                        .collect();
                    let suppression =
                        smelt_logical::maintenance::choice::resolve_keyless_staged_suppression(
                            &output_columns,
                            &comparability,
                            &cell.row_identity,
                        );
                    let WriteSuppression::Suppressed { compared_columns } = suppression else {
                        continue;
                    };
                    return Ok(Some((
                        source.clone(),
                        cell.clone(),
                        group_columns.clone(),
                        MembershipRecomputeWrite::StagedKeyless { compared_columns },
                    )));
                }
                ChosenTechnique::DiffPatch {
                    recompute: Technique::DeleteInsert,
                    delete_leg,
                } => {
                    // The candidate this resolver's caller writes is always
                    // the model's own FULL unwindowed recompute — a region
                    // recompute's own coverage IS its slice, so the delete
                    // leg is sound regardless of what `resolve_cell_choice`
                    // proved (it already grants `Complete` here too, but
                    // this arm does not depend on that — it re-derives its
                    // own completeness argument rather than trusting an
                    // upstream `Omitted` it would otherwise have to refuse
                    // on for no real reason).
                    let _ = delete_leg;
                    let admitted = smelt_logical::maintenance::diff_patch::admit_diff_patch(
                        group_columns,
                        &comparability,
                        &cell.row_identity,
                        Ok(()),
                    )
                    .map_err(|refusal| {
                        anyhow::anyhow!(
                            "MaintenanceDiffPatchRefused: a `write: diff_patch` pin over a \
                             membership-sensitive Technique::DeleteInsert cell for group '{}' \
                             could not be admitted: {refusal:?}",
                            cell.group,
                        )
                    })?;
                    return Ok(Some((
                        source.clone(),
                        cell.clone(),
                        group_columns.clone(),
                        MembershipRecomputeWrite::DiffPatch {
                            compared_columns: admitted.compared_columns,
                        },
                    )));
                }
                _ => continue,
            }
        }
    }
    Ok(None)
}

mod execute;
pub use execute::{execute_staged_keyless_recompute, execute_staged_membership_recompute};
