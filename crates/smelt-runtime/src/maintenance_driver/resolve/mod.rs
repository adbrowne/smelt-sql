use anyhow::{bail, Result};
use smelt_backend::IncrementalStrategy;
use smelt_core::config::CellTechnique;
use smelt_logical::maintenance::availability::StateAvailability;
use smelt_logical::maintenance::choice::{
    effective_override, resolve_cell_choice, ChosenTechnique,
};
use smelt_logical::maintenance::derive::SourceReferentialIntegrity;
use smelt_logical::maintenance::{
    MaintenancePlan, PlanCell, SourceFacts, Technique, Trigger, WritePattern, WriteSelection,
};
use std::collections::HashSet;

/// Resolve the `IncrementalStrategy` a model's creation trigger (region
/// recompute over a partition-grain model) should actually execute, by
/// reading the technique the derived `MaintenancePlan` admitted instead of
/// a hardcoded constant (MP11, `docs/specs/incremental_models.md` §"Per-cell
/// admission"). Per the "Maintenance-plan purity" invariant (root
/// `CLAUDE.md`), the plan itself is derived exactly once by
/// `smelt-db`'s pure `derive_model_maintenance_plan` — this function calls
/// it and maps the result onto `smelt-backend`'s `IncrementalStrategy`; it
/// never re-implements admission.
///
/// `derive_new_data`'s `Grain::Partition` arm (`smelt-logical`) admits
/// `Technique::DeleteInsert` unconditionally for the creation trigger — no
/// refusal path exists there today — so this call site is a mechanism
/// swap, not an observable behaviour change: it exists so a future
/// admission rule for the creation cell takes effect here automatically,
/// without a second hand-maintained mapping to keep in sync. Falls back to
/// `backend_default` when the model carries no maintenance plan to derive
/// (e.g. `metadata.grain` unset — should not happen once `refresh:
/// incremental` requires a declared grain) or the admitted technique has no
/// `IncrementalStrategy` counterpart (a targeted-write technique never
/// serves the creation trigger's region-recompute corner).
/// The creation trigger's cell (whichever `Trigger::NewData` sibling this
/// resolver reads) is resolved through the SAME override ladder the
/// mutation/column-added paths already consult
/// (`smelt_logical::maintenance::choice::resolve_cell_choice`) rather than a
/// raw `cell.technique` read: a declared `cells[].write` pin, a hard
/// `cells[].technique` pin (refusing loudly when the resolvable set does not
/// contain it), a soft `defaults.prefer`/`cells[].prefer`, then the cell's
/// own admitted-and-live technique, then region recompute
/// (`incremental_models.md` §Design "Absent a cost model: the fixed
/// preference order").
#[allow(clippy::too_many_arguments)]
pub fn resolve_incremental_strategy(
    sql: &str,
    table: &str,
    metadata: &smelt_core::ModelMetadata,
    sources: &[SourceFacts],
    explicitly_mutable: &HashSet<String>,
    model_edges: &[smelt_logical::maintenance::derive::ModelEdge],
    backend_default: IncrementalStrategy,
    backend_supports_column_scoped_merge: bool,
    availability: &StateAvailability,
) -> Result<IncrementalStrategy> {
    let result = if model_edges.is_empty() {
        crate::maintenance_availability::derive_resolved(
            sql,
            table,
            metadata,
            sources,
            explicitly_mutable,
            // See the analogous call in `resolve_live_column_scoped_cell` above.
            None,
            // Not (yet) plumbed with declared `key_recurrence` bounds at this
            // call site — this resolver only reads the creation cell's
            // `Technique`, which route 3's declared sub-route does not affect
            // (a locality refusal already yields an empty-cells plan either
            // way, falling back to `backend_default` below).
            &[],
            // This resolver only reads the creation (`NewData`) cell — a
            // `ColumnAdded` trigger never affects it, so no deployed-schema
            // snapshot is needed here.
            &[],
            &SourceReferentialIntegrity::new(),
            None,
            None,
            availability,
            &[],
        )
    } else {
        // Edge-aware derivation — the SAME derivation
        // `resolve_live_delta_restriction_facts` uses, never a second one.
        crate::maintenance_availability::derive_resolved_with_edges(
            sql,
            table,
            metadata,
            sources,
            explicitly_mutable,
            model_edges,
            None,
            &[],
            &[],
            &SourceReferentialIntegrity::new(),
            None,
            None,
            availability,
            &[],
        )
    };
    let Some(result) = result else {
        return Ok(backend_default);
    };
    let cells_cfg: &[smelt_core::config::MaintenanceCellConfig] = metadata
        .maintenance
        .as_ref()
        .map(|m| m.cells.as_slice())
        .unwrap_or(&[]);
    // The whole-row creation cell's own `group` is the fixed cosmetic label
    // `{*}` (`derive_new_data`'s `Grain::Partition` arm), never one of the
    // model's own derived payload [`ColumnGroup`]s — so a `cells[].technique`
    // override naming a real output column is matched against the UNION of
    // every derived group's columns (every column the whole-row write
    // touches), not any single group's own members.
    let all_columns: Vec<String> = result
        .column_groups
        .iter()
        .flat_map(|g| g.columns.iter().cloned())
        .collect();

    if let Some(driving_edge) = model_edges.first() {
        let driving_trigger = Trigger::NewData {
            source: driving_edge.name.clone(),
        };
        if let Some(cell) = result.plan.cell_for(&driving_trigger) {
            return resolve_creation_cell_strategy(
                cell,
                &driving_edge.name,
                metadata,
                cells_cfg,
                &result.column_groups,
                &all_columns,
                backend_default,
                backend_supports_column_scoped_merge,
            );
        }
        let driving_edge_refused = result.plan.refusals.iter().any(|r| {
            matches!(
                r,
                smelt_logical::maintenance::Refusal::ReachNotDerivable { edge, .. }
                    if edge == &driving_edge.name
            )
        });
        let other_creation_cell =
            result.plan.cells.iter().any(|c| {
                matches!(&c.trigger, Trigger::NewData { .. }) && c.trigger != driving_trigger
            });
        if driving_edge_refused && !other_creation_cell {
            bail!(
                "model '{table}' cannot be maintained: upstream maintained model edge '{}' \
                 declares no timeseries clock and none is inferable, so its creation-trigger \
                 edge cannot be clamped to the output partition axis, and no other \
                 creation-trigger cell admits a technique for this run \
                 (docs/specs/incremental_models.md §\"Upstream model edges\")",
                driving_edge.name
            );
        }
        // The driving edge's own cell is absent but there is another
        // admissible creation-trigger cell (or the refusal is unrelated to
        // the driving edge) — fall through to the first-`NewData`-match
        // below, mirroring pre-edge-aware behaviour.
    }

    let creation_cell = result
        .plan
        .cells
        .iter()
        .find(|c| matches!(c.trigger, Trigger::NewData { .. }));
    match creation_cell {
        Some(cell) => {
            let Trigger::NewData { source } = &cell.trigger else {
                unreachable!("filtered to Trigger::NewData above")
            };
            resolve_creation_cell_strategy(
                cell,
                source,
                metadata,
                cells_cfg,
                &result.column_groups,
                &all_columns,
                backend_default,
                backend_supports_column_scoped_merge,
            )
        }
        None => Ok(backend_default),
    }
}

/// Resolve the technique the creation-trigger `cell` should actually
/// execute, consulting the SAME override ladder
/// (`smelt_logical::maintenance::choice::resolve_cell_choice`) every other
/// per-cell dispatch resolver uses: a validated `cells[].write` pin, a hard
/// `cells[].technique` pin (refusing loudly when the resolvable set does not
/// contain it), a soft `defaults.prefer`/`cells[].prefer`, then the cell's
/// own admitted-and-live technique, then region recompute
/// (`incremental_models.md` §Design "Absent a cost model: the fixed
/// preference order"). `IncrementalStrategy` has exactly one live variant
/// (`DeleteInsert`) today, so every non-`DeleteInsert` choice maps to
/// `backend_default` — the caller's own region-recompute fallback.
#[allow(clippy::too_many_arguments)]
fn resolve_creation_cell_strategy(
    cell: &PlanCell,
    trigger_address: &str,
    metadata: &smelt_core::ModelMetadata,
    cells_cfg: &[smelt_core::config::MaintenanceCellConfig],
    column_groups: &[smelt_logical::maintenance::ColumnGroup],
    all_columns: &[String],
    backend_default: IncrementalStrategy,
    backend_supports_column_scoped_merge: bool,
) -> Result<IncrementalStrategy> {
    let write_pin =
        smelt_db::queries::maintenance::matching_write_pin(cell, column_groups, cells_cfg)
            .and_then(|pin_name| smelt_logical::maintenance::lookup_write_pattern(&pin_name));
    let overrides = effective_override(
        metadata
            .maintenance
            .as_ref()
            .and_then(|m| m.defaults.as_ref()),
        cells_cfg,
        trigger_address,
        all_columns,
    );
    let chosen = resolve_cell_choice(
        Some(cell),
        &cell.trigger,
        &overrides,
        write_pin,
        backend_supports_column_scoped_merge,
    )
    .map_err(|refusal| anyhow::anyhow!(refusal.to_string()))?;
    Ok(match chosen {
        ChosenTechnique::Admitted(Technique::DeleteInsert) => IncrementalStrategy::DeleteInsert,
        _ => backend_default,
    })
}

/// Resolve the plain `Trigger::NewData` incremental fold's own per-cell
/// `deferral` scheduling verdict (`docs/outcomes/20260815-definition-delta-
/// migrate/phases/14-plan.md`) — the only trigger family where
/// `contract.cells[].deferral` is validly declarable (`resolve_deferral`
/// requires an interval-representable clock; every other live per-cell
/// dispatch resolver serves an inadmissible trigger, see phase 12's
/// decision log). Thin: derives the model's maintenance plan the same way
/// [`resolve_incremental_strategy`] does, reads its own column groups (the
/// fold's own groups — creation is whole-row, so a group here is exactly
/// one payload column-group the model derives, never re-implemented here),
/// and hands both that and the caller-resolved `cell_decisions` (already
/// licensed via `contract_probes::deferral_cell_decisions`'s own lag
/// comparison — this function makes no independent lag judgement) to
/// `smelt_logical::contract::deferral::fold_deferral_verdict`, the
/// single-owner coverage rule.
///
/// Returns `(Proceed, [])` whenever the model declares no cell-level
/// `deferral` at all, or has no `Trigger::NewData` cell to serve — never a
/// silent skip for an undeclared model.
pub fn resolve_fold_deferral(
    sql: &str,
    table: &str,
    metadata: &smelt_core::ModelMetadata,
    sources: &[SourceFacts],
    explicitly_mutable: &HashSet<String>,
    cell_decisions: &[crate::contract_probes::CellDeferralDecision],
    availability: &StateAvailability,
) -> (
    smelt_logical::contract::deferral::FoldDeferralVerdict,
    Vec<String>,
) {
    use smelt_logical::contract::deferral::{
        cell_address, fold_deferral_verdict, DeclaredFoldCell, FoldDeferralVerdict, RunLicense,
    };

    let no_deferral = (FoldDeferralVerdict::Proceed, Vec::new());

    let Some(cells_cfg) = metadata.contract.as_ref().map(|c| c.cells.as_slice()) else {
        return no_deferral;
    };
    let declared: Vec<DeclaredFoldCell> = cells_cfg
        .iter()
        .filter(|cell_cfg| cell_cfg.deferral.is_some())
        .filter_map(|cell_cfg| {
            let address = cell_address(&cell_cfg.columns, &cell_cfg.on);
            let decision = cell_decisions.iter().find(|d| d.address == address)?;
            Some(DeclaredFoldCell {
                address,
                columns: cell_cfg.columns.clone(),
                on: cell_cfg.on.clone(),
                skip_licensed: matches!(decision.license, RunLicense::Skip { .. }),
            })
        })
        .collect();
    if declared.is_empty() {
        return no_deferral;
    }

    let Some(result) = crate::maintenance_availability::derive_resolved(
        sql,
        table,
        metadata,
        sources,
        explicitly_mutable,
        None,
        &[],
        &[],
        &SourceReferentialIntegrity::new(),
        None,
        None,
        availability,
        &[],
    ) else {
        return no_deferral;
    };
    let Some(creation_source) = result.plan.cells.iter().find_map(|c| match &c.trigger {
        Trigger::NewData { source } => Some(source.clone()),
        _ => None,
    }) else {
        return no_deferral;
    };
    let fold_groups: Vec<(Vec<String>, String)> = result
        .column_groups
        .iter()
        .map(|g| (g.columns.clone(), creation_source.clone()))
        .collect();

    let verdict = fold_deferral_verdict(&declared, &fold_groups);
    let addresses = match &verdict {
        FoldDeferralVerdict::SkipFold { addresses } => addresses.clone(),
        FoldDeferralVerdict::Proceed => Vec::new(),
    };
    (verdict, addresses)
}

/// Which physical technique actually executes for one plan cell, resolved
/// from the derived [`MaintenancePlan`], the operator's optional hard pin
/// (`maintenance.cells[].technique`), and whether the target backend can
/// run a column-scoped `MERGE` at all
/// (`BackendCapabilities::supports_column_scoped_merge`, read via
/// `Backend::capabilities`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolvedTechnique {
    /// No live targeted-write cell for this trigger (unadmitted, or the
    /// backend lacks the capability, and no pin demands one): the caller
    /// falls back to the region-recompute `DELETE`+`INSERT` it already
    /// performs. This is the *safe default*, never a silent substitute for
    /// a technique the operator explicitly pinned.
    RegionRecompute,
    /// An admitted `Technique::ColumnScopedMerge` cell, live on a backend
    /// that can execute it.
    ColumnScopedMerge,
}

/// Legacy two-way (`RegionRecompute`/`ColumnScopedMerge`) resolver, retained
/// **only** for `crates/smelt-runtime/tests/technique_lowering.rs`'s narrow
/// unit coverage of that two-way choice in isolation. It has **zero
/// production call sites**: the live execute path resolves entirely through
/// `smelt_logical::maintenance::choice::resolve_cell_choice`, dispatched from
/// [`resolve_live_column_scoped_cell`] below (Phase 2, `docs/plans/
/// 20260719-prod-w7-bakeoff.md`). `pub` (not `pub(crate)`) only because
/// `technique_lowering.rs` is a `tests/` integration test compiled as a
/// separate crate and needs external visibility; do not add new production
/// callers — extend `resolve_cell_choice` and thread the result through
/// `resolve_live_column_scoped_cell` instead.
///
/// Resolve which technique executes for `trigger`, mirroring
/// `incremental_models.md` §"Per-cell admission": a `technique:` pin bypasses
/// the cost model, **never** admission — pinning `rederive_columns` for a
/// cell the plan did not admit (or that a capability-gapped backend cannot
/// run) is a hard, fail-loud error, not a silent fallback to
/// `RegionRecompute`. Absent a pin, an admitted+runnable `ColumnScopedMerge`
/// cell is preferred (the point of this phase — "first live cell where
/// execution differs by column group"); otherwise the safe region-recompute
/// default applies with no error (an unpinned model simply has no
/// column-scoped cell to run yet).
///
/// `write_pin` is an already-validated `cells[].write` registry entry
/// (`smelt_logical::maintenance::resolve_write_pin`'s `Ok` result —
/// registry/capability/equivalence already checked by the caller; this
/// function only asks whether the validated pattern's own
/// [`WriteSelection`] is realizable by THIS narrow (`ColumnScopedMerge` vs
/// `RegionRecompute`) resolver). When present it is consulted **before**
/// `pin` (the `cells[].technique` ladder) and decides the cell alone — same
/// precedence rule as `smelt_logical::maintenance::choice::
/// resolve_cell_choice`'s own write-pin consultation, so the two resolvers
/// agree on which pin wins when a cell carries both. A `write_pin` selecting
/// a technique this resolver has no arm for (`KeyedFold`/`InPlaceUpdate` —
/// this function's own scope is only ever the dimension-merge two-way
/// choice) refuses fail-loud rather than silently falling back to region
/// recompute for a pin that named something else.
pub fn resolve_cell_technique(
    plan: &MaintenancePlan,
    trigger: &Trigger,
    pin: Option<CellTechnique>,
    backend_supports_column_scoped_merge: bool,
) -> Result<ResolvedTechnique> {
    resolve_cell_technique_with_write_pin(
        plan,
        trigger,
        pin,
        None,
        backend_supports_column_scoped_merge,
    )
}

/// [`resolve_cell_technique`] plus an optional already-validated
/// `cells[].write` pin — see that function's doc comment for the full
/// contract and precedence rule. Split out as its own function so the
/// existing `pin`-only call sites (and this module's own unit tests) keep
/// compiling unchanged; production write-pin consultation happens through
/// this entry point once a caller has a resolved [`WritePattern`] in hand.
/// Like [`resolve_cell_technique`], this has no production call site — it
/// exists solely for `technique_lowering.rs`'s two-way unit coverage; the
/// live path is `resolve_cell_choice` via [`resolve_live_column_scoped_cell`].
pub fn resolve_cell_technique_with_write_pin(
    plan: &MaintenancePlan,
    trigger: &Trigger,
    pin: Option<CellTechnique>,
    write_pin: Option<&'static WritePattern>,
    backend_supports_column_scoped_merge: bool,
) -> Result<ResolvedTechnique> {
    let admitted = plan
        .cell_for(trigger)
        .is_some_and(|c| c.technique == Technique::ColumnScopedMerge);
    let live = admitted && backend_supports_column_scoped_merge;

    if let Some(pattern) = write_pin {
        return match pattern.selects() {
            WriteSelection::RegionRecompute => Ok(ResolvedTechnique::RegionRecompute),
            WriteSelection::Technique(Technique::ColumnScopedMerge) if live => {
                Ok(ResolvedTechnique::ColumnScopedMerge)
            }
            WriteSelection::Technique(Technique::ColumnScopedMerge) if admitted => bail!(
                "MaintenanceUnboundedFootprint: write pin '{}' for {trigger:?} resolves to a \
                 column-scoped MERGE admitted by the derived plan, but the target backend does \
                 not support column-scoped MERGE — a capability gap drops the technique from \
                 admission at plan time; refusing rather than silently falling back to a \
                 targeted write at runtime",
                pattern.name
            ),
            WriteSelection::Technique(Technique::ColumnScopedMerge) => bail!(
                "MaintenanceUnboundedFootprint: write pin '{}' for {trigger:?} names a cell the \
                 derived plan did not admit as a column-scoped MERGE — a write pin bypasses the \
                 cost model, never admission (`incremental_models.md` §\"Per-cell write \
                 addressing\"); refusing rather than lowering an unbounded-footprint targeted \
                 write at runtime",
                pattern.name
            ),
            WriteSelection::Technique(other) => bail!(
                "MaintenanceUnboundedFootprint: write pin '{}' for {trigger:?} selects {other:?}, \
                 which this dimension-merge resolver has no lowering for (only ColumnScopedMerge \
                 and the always-available region recompute are reachable here) — refusing rather \
                 than silently substituting a different technique than the one pinned",
                pattern.name
            ),
            // `diff_patch` is not reachable through this dimension-merge
            // resolver (only `ColumnScopedMerge` and the always-available
            // region recompute are) — no live routing exists for it yet in
            // any case (a later phase's scope), so this refuses the same
            // way the `Technique(other)` arm above does rather than
            // silently substituting a different technique.
            WriteSelection::DiffPatch => bail!(
                "MaintenanceUnboundedFootprint: write pin '{}' for {trigger:?} selects diff_patch, \
                 which this dimension-merge resolver has no lowering for (only ColumnScopedMerge \
                 and the always-available region recompute are reachable here) — refusing rather \
                 than silently substituting a different technique than the one pinned",
                pattern.name
            ),
        };
    }

    match pin {
        Some(CellTechnique::RederiveColumns) if live => Ok(ResolvedTechnique::ColumnScopedMerge),
        Some(CellTechnique::RederiveColumns) if admitted => bail!(
            "MaintenanceUnboundedFootprint: pinned technique 'rederive_columns' for {trigger:?} \
             is admitted by the derived plan, but the target backend does not support \
             column-scoped MERGE — a capability gap drops the technique from admission at plan \
             time; refusing rather than silently falling back to a targeted write at runtime"
        ),
        Some(CellTechnique::RederiveColumns) => bail!(
            "MaintenanceUnboundedFootprint: pinned technique 'rederive_columns' for {trigger:?} \
             names a cell the derived plan did not admit — a technique pin bypasses the cost \
             model, never admission (`incremental_models.md` §\"Per-cell admission\"); refusing \
             rather than lowering an unbounded-footprint targeted write at runtime"
        ),
        _ if live => Ok(ResolvedTechnique::ColumnScopedMerge),
        _ => Ok(ResolvedTechnique::RegionRecompute),
    }
}

mod live_cells;
pub use live_cells::{
    resolve_live_column_scoped_cell, resolve_live_in_place_update_cell, widen_horizon_for_batch,
};
