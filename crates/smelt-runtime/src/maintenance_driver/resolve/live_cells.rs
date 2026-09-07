use anyhow::{bail, Result};
use smelt_logical::analysis::source_bounds::BoundResult;
use smelt_logical::maintenance::availability::StateAvailability;
use smelt_logical::maintenance::choice::{
    effective_override, resolve_cell_choice, resolve_cell_write_suppression, ChosenTechnique,
    WriteSuppression,
};
use smelt_logical::maintenance::derive::SourceReferentialIntegrity;
use smelt_logical::maintenance::{PlanCell, ScanClamp, SourceFacts, Technique, Trigger};
use std::collections::HashSet;

/// Find the first `explicitly_mutable` source whose `Trigger::
/// UpstreamMutation` cell resolves live to `Technique::ColumnScopedMerge`
/// (via `smelt_logical::maintenance::choice::resolve_cell_choice`, see below)
/// in the model's derived [`MaintenancePlan`] — the regular incremental
/// execution loop's per-run
/// technique choice (MP11), as distinct from [`resolve_incremental_strategy`]
/// above, which only maps the creation trigger. Per the "Maintenance-plan
/// purity" invariant (root `CLAUDE.md`), this calls
/// `derive_model_maintenance_plan` exactly once and only reads the result —
/// it never re-implements admission itself.
///
/// Returns the matched source name, its admitted [`PlanCell`], and the
/// resolved [`WriteSuppression`] verdict (T1, `docs/plans/
/// 20260715-composed-axes-conditional-maintenance.md` Phase C4) for the
/// cell's own mutation-sensitive column group, so the caller can pick the
/// right physical primitive from `cell.partition_local` (a genuine
/// `ScanClamp` licenses the horizon-clamped [`execute_column_scoped_merge`];
/// an accepted full scan has no horizon and takes
/// [`execute_column_scoped_merge_full`] instead). `None` when the model
/// carries no maintenance plan, declares no explicitly-mutable source, or no
/// source resolves live — the caller's safe default is the existing
/// region-recompute batch loop, unchanged.
///
/// `WriteSuppression` is resolved here (not re-derived by the caller) from
/// the same `sql`'s P3 change-comparability walk
/// (`smelt_logical::analysis::walk::model_property_vector`, never a fresh ad
/// hoc scan — `architecture.md` §"Property composition walk rule") and the
/// cell's own P2 `row_identity` (already carried on `PlanCell`, C3), folded
/// via `choice::resolve_write_suppression`. The cell's raw column list comes
/// from `result.column_groups` (the same derivation's own `ColumnGroup`s),
/// matched by `PlanCell::group`'s display name — the plan-purity invariant's
/// "derived once, never re-derived" extends to this lookup, not a second
/// column-grouping pass.
///
/// This is the ladder's single production dispatch site for the
/// Fold/Recompute/RederiveColumns family dimension
/// (`smelt_logical::maintenance::choice::resolve_cell_choice`) — a
/// frontmatter `cells[].technique` hard pin or `cells[].prefer` soft
/// preference on this trigger's cell is threaded in via
/// [`effective_override`] and actually consulted, rather than the
/// pin-less two-way resolver this call site used before (Phase 2,
/// `docs/plans/20260719-prod-w7-bakeoff.md`). An inadmissible hard pin
/// surfaces as [`smelt_logical::maintenance::choice::ChoiceRefusal`],
/// mapped here to a real `Err` — the fail-loud discipline (root
/// `CLAUDE.md`) forbids silently falling back to region recompute for a
/// pin the derived plan does not admit.
#[allow(clippy::too_many_arguments)]
pub fn resolve_live_column_scoped_cell(
    sql: &str,
    table: &str,
    metadata: &smelt_core::ModelMetadata,
    sources: &[SourceFacts],
    explicitly_mutable: &HashSet<String>,
    backend_supports_column_scoped_merge: bool,
    technique_overrides: &[crate::types::CellTechniqueOverride],
    availability: &StateAvailability,
) -> Result<Option<(String, PlanCell, WriteSuppression)>> {
    let Some(result) = crate::maintenance_availability::derive_resolved(
        sql,
        table,
        metadata,
        sources,
        explicitly_mutable,
        // Not (yet) plumbed with the driving source's declared granularity
        // at this call site — a keyed model with its own `timeseries:`
        // block fails the locality gate's granularity-equality precondition
        // closed here, same as before this phase (`smelt-db`'s own
        // diagnostic path, `maintenance_plan_diagnostics`, has the real
        // value; the runtime execution path,
        // `smelt-runtime::cumulative::execute_cumulative_aggregate`, is
        // this phase's actual slice-pruning consumer).
        None,
        // Not (yet) plumbed with declared `key_recurrence` bounds at this
        // call site, for the same reason as the granularity `None` above —
        // this resolver only inspects mutation-trigger cells, which key
        // temporal locality's routes do not gate.
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
    // Request overrides enter the SAME `effective_override` ladder as
    // frontmatter `cells[]` entries, converted to the matching shape
    // (`prefer`/`write` left `None` — request scope only carries a hard
    // technique pin). `matching_cell` (in `smelt-logical`, not touched by
    // this phase) is first-match-wins, so request overrides are placed
    // BEFORE the frontmatter cells in the combined slice: that is how
    // "request scope is narrower than file scope" (`docs/plans/
    // 20260719-prod-w7-bakeoff.md` Phase 3, decision B1) is realized —
    // a request override for a cell also pinned in frontmatter is found
    // first and wins.
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
        // A trigger commonly derives MULTIPLE sibling cells, one per
        // membership-sensitive column group a shared join admits
        // (`docs/plans/20260808-membership-sensitivity.md` Phase 1) — every
        // one of them must be offered a chance to match a `cells[]`
        // override scoped to ITS OWN columns, never only the first
        // (`MaintenancePlan::cell_for`'s own doc comment on this exact bug,
        // `docs/plans/20260808-membership-sensitivity.md` Phase 3's fix).
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
        // Fail-loud: a HARD `cells[on: source].technique` pin whose
        // `columns` address NONE of this trigger's own sibling groups is a
        // dangling/misconfigured pin — under the pre-Phase-3 first-match
        // lookup it would silently never be consulted by anything; refuse
        // instead of vanishing (root `CLAUDE.md` §"Fail-loud discipline").
        // A soft `prefer` in the same situation is not flagged here — it
        // never refuses even when it names a resolvable technique the cell
        // doesn't have (`resolve_cell_choice`'s own contract).
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
            // An already-validated `cells[].write` pin for this cell
            // (`smelt-db`'s pre-execution diagnostic gate already ran
            // `resolve_write_pin`'s registry/capability/equivalence checks —
            // an invalid pin never reaches here, the run would already have
            // been refused with `MaintenanceWritePatternUnavailable`/
            // `MaintenanceWriteAddressingRefused`); this only re-resolves
            // the *name* to its registry entry so `resolve_cell_choice` can
            // consult which [`smelt_logical::maintenance::WriteSelection`]
            // it maps to, never re-deriving admission itself.
            let write_pin = smelt_db::queries::maintenance::matching_write_pin(
                cell,
                &result.column_groups,
                cells_cfg,
            )
            .and_then(|pin_name| smelt_logical::maintenance::lookup_write_pattern(&pin_name));
            // The override ladder (`defaults.prefer` → `cells[].prefer` →
            // `cells[].technique`, narrower scope winning) narrowed to THIS
            // sibling cell's own trigger + column group — the SAME
            // `overrides` value feeds both the family choice below and the
            // write-suppression variant resolution further down, so a
            // `cells[].technique` entry naming e.g. `suppress`/
            // `unconditional` for this cell is visible to both dimensions
            // from one ladder evaluation.
            let overrides = effective_override(
                metadata
                    .maintenance
                    .as_ref()
                    .and_then(|m| m.defaults.as_ref()),
                &combined_cells,
                source,
                group_columns,
            );
            let chosen = resolve_cell_choice(
                Some(cell),
                &trigger,
                &overrides,
                write_pin,
                backend_supports_column_scoped_merge,
            )
            .map_err(|refusal| anyhow::anyhow!(refusal.to_string()))?;
            if chosen != ChosenTechnique::Admitted(Technique::ColumnScopedMerge) {
                continue;
            }
            // Fold the write-suppression proof (P2/P3) and its variant
            // resolution (first-build/definition-change-backfill posture, or
            // an explicit `prefer`/`technique` override on this dimension)
            // into one shared resolver — the same one the `--show-sql`
            // preview builder calls, so a printed statement can never drift
            // from what this live run executes
            // (`incremental_models.md` §"Statement emission (single owner)").
            //
            // A `technique: suppress` pin forcing suppression on over a genuine
            // P2/P3 proof failure is a hard `ChoiceRefusal`, propagated as a
            // real run error below — mirroring how the family dimension's
            // own `resolve_cell_choice` refusal above already fails the run,
            // never a silent fallback to region recompute
            // (`incremental_models.md` §"Per-cell write addressing" →
            // "User pins").
            let suppression = resolve_cell_write_suppression(sql, group_columns, cell, &overrides)
                .map_err(|refusal| anyhow::anyhow!(refusal.to_string()))?;
            return Ok(Some((source.clone(), cell.clone(), suppression)));
        }
    }
    Ok(None)
}

/// Resolve a live `Trigger::ColumnAdded` cell that resolves to
/// `Technique::InPlaceUpdate` (`docs/plans/20260809-sensitivity-precision.md`
/// Phase 6, `docs/specs/definition_deltas.md` §"The verdict per column group") — the production entry point for the definition-change
/// trigger, distinct from [`resolve_live_column_scoped_cell`]/
/// [`resolve_live_membership_recompute_cell`] above (which only ever
/// inspect `NewData`/`UpstreamMutation` cells).
///
/// `deployed_column_names` is the caller's own I/O: `smelt-runtime` is the
/// one caller with real access to the deployed-schema snapshot the runtime
/// `schema_evolution` module already reads/writes
/// (`crate::schema_evolution::infer_deployed_columns`/
/// `save_deployed_schema`) — `derive_model_maintenance_plan` itself does no
/// I/O (Salsa-purity rule). An empty slice (no known deployed schema) derives
/// no trigger at all, same as `smelt-db`'s own diagnostic path.
///
/// Returns the admitted cell plus its ready-to-execute `(column,
/// expression)` assignment pairs — the added columns' own defining
/// expressions read straight from the model's current SQL via
/// [`smelt_logical::maintenance::derive::column_def_from_sql`], the SAME
/// source [`crate::diagnostics::build_technique_statements`]'s
/// `Technique::InPlaceUpdate` preview arm reads, and the same source the
/// `PureBackfill` classification (`smelt_logical::analysis::
/// definition_change::classify_definition_change`) was proven against —
/// never a fresh re-derivation of either the trigger or the assignments.
/// `None` when the model carries no maintenance plan, no deployed snapshot
/// is known, or no cell resolves to `InPlaceUpdate` (no `ColumnAdded`
/// trigger fired, the added column(s) classified `UpstreamRederive`, or a
/// skeleton add refused).
pub fn resolve_live_in_place_update_cell(
    sql: &str,
    table: &str,
    metadata: &smelt_core::ModelMetadata,
    sources: &[SourceFacts],
    deployed_column_names: &[String],
    availability: &StateAvailability,
) -> Option<(PlanCell, Vec<(String, String)>)> {
    if deployed_column_names.is_empty() {
        return None;
    }
    let result = crate::maintenance_availability::derive_resolved(
        sql,
        table,
        metadata,
        sources,
        &HashSet::new(),
        None,
        &[],
        deployed_column_names,
        &SourceReferentialIntegrity::new(),
        None,
        None,
        availability,
        &[],
    )?;
    let cell = result
        .plan
        .cells
        .iter()
        .find(|c| {
            matches!(c.trigger, Trigger::ColumnAdded { .. })
                && c.technique == Technique::InPlaceUpdate
        })?
        .clone();
    let Trigger::ColumnAdded { columns } = &cell.trigger else {
        unreachable!("filtered above")
    };
    let mut assignments = Vec::with_capacity(columns.len());
    for col in columns {
        let def = smelt_logical::maintenance::derive::column_def_from_sql(sql, col)?;
        assignments.push((col.clone(), def.expr.syntax().text().to_string()));
    }
    Some((cell, assignments))
}

/// Widen a derived [`ScanClamp`]'s forward reach to at least `batch_width`
/// before handing it to [`execute_column_scoped_merge`] as the horizon `H`.
///
/// `dimension_batch_sql` is already scoped to the current batch's
/// `[start, end)` window (`inject_time_filter`/`inject_source_filters`,
/// `execute.rs`) before `execute_column_scoped_merge` applies its OWN
/// horizon clamp on top. Passing the raw derived `scan.after` straight
/// through would risk NARROWING that already-correct window whenever a
/// batch spans more than the source's own derived margin (e.g. a
/// multi-day backfill batch over a day-granularity clamp), silently
/// dropping the batch's earlier rows from the merge — the horizon clamp
/// may only ever WIDEN the batch window, never narrow it.
pub fn widen_horizon_for_batch(
    scan: &ScanClamp,
    batch_width: smelt_logical::analysis::source_bounds::Seconds,
) -> BoundResult {
    let after = if scan.after.0 > batch_width.0 {
        scan.after
    } else {
        batch_width
    };
    BoundResult::Bounded {
        source_partition_col: scan.column.clone(),
        before: scan.before,
        after,
    }
}
