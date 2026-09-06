use anyhow::{bail, Result};
use smelt_backend::IncrementalStrategy;
use smelt_core::config::CellTechnique;
use smelt_logical::analysis::source_bounds::BoundResult;
use smelt_logical::maintenance::availability::StateAvailability;
use smelt_logical::maintenance::choice::{
    effective_override, resolve_cell_choice, resolve_cell_write_suppression, ChosenTechnique,
    WriteSuppression,
};
use smelt_logical::maintenance::derive::SourceReferentialIntegrity;
use smelt_logical::maintenance::{
    MaintenancePlan, PlanCell, ScanClamp, SourceFacts, Technique, Trigger, WritePattern,
    WriteSelection,
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
