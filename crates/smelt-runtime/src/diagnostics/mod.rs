//! The shared, pure model-diagnostics builder (`docs/specs/ui_model_diagnostics.md`
//! §Surface "smelt-runtime builder"): the single place a model's full derived
//! state — property set, relation contract, per-cell technique previews — is
//! assembled. `smelt-cli`'s `explain` report and `smelt-ui`'s diagnostics
//! endpoint are both thin, read-only renderers over [`ModelDiagnostics`]; they
//! must not re-derive any of this data themselves
//! (`docs/specs/ui_model_diagnostics.md` §Semantics "Thin-consumer boundary").
//!
//! This module operates purely over already-resolved facts (a [`ModelFile`],
//! its upstream models and sources, a caller-built [`BoundContext`]) — it
//! never touches a live backend or the maintenance ledger
//! (`docs/specs/ui_model_diagnostics.md` §Constraints).
//!
//! The per-cell technique-preview set (`PlanCellDiagnostics`) is added in a
//! later stage of the diagnostics builder; this module currently populates
//! the property set and relation contract halves of [`ModelDiagnostics`] and
//! carries an always-empty `cells` field as a typed placeholder.
//!
//! [`PropertySet`] and the property profile it feeds
//! (`docs/specs/property_diff.md` §"The property profile") are single-owned
//! in `smelt_logical::analysis::profile`, re-exported here so every existing
//! `smelt_runtime::diagnostics::PropertySet` import keeps working unchanged
//! (`docs/outcomes/20260905-property-diff/phases/02-plan.md` §"Design
//! decisions" — "Where").

mod preview;
mod relation_contract;

pub use preview::{
    build_plan_cell_diagnostics, Admissibility, PlanCellDiagnostics, PreviewStatement,
    TechniquePreview,
};
pub use relation_contract::{
    build_relation_contract, InboundEdgeContract, RelationContractClock, RelationContractProvider,
    RelationContractView,
};

use serde::Serialize;

use smelt_core::config::{Config, ContractConfig};
use smelt_core::graph::DependencyGraph;
use smelt_core::{ModelFile, SourceInfo};
use smelt_logical::analysis::profile::{ProbePlanEntry, PropertyProfile};
pub use smelt_logical::analysis::profile::{ProfileError, PropertySet};
use smelt_logical::analysis::source_bounds::BoundContext;
use smelt_logical::contract::ContractPointView;
use smelt_logical::maintenance::emit::MaintenanceDialect;
use smelt_logical::maintenance::{cell_trigger_address, ColumnGroup, PlanCell, Refusal};
use smelt_planner::SourceTimeseriesMap;

use crate::compile::{CompilerRegistry, EphemeralResolver};

/// Errors the diagnostics builder can surface. Fail-loud
/// (`CLAUDE.md` §"Fail-loud discipline"): a model whose SQL cannot be
/// classified into a property vector is reported as an error, never silently
/// defaulted to an empty/optimistic property set.
#[derive(Debug, thiserror::Error)]
pub enum DiagnosticsError {
    #[error(transparent)]
    PropertyDerivation(#[from] ProfileError),
}

/// A model's full derived state (`docs/specs/ui_model_diagnostics.md`
/// §Surface "smelt-runtime builder"): its property set, its own relation
/// contract plus every inbound edge's, and its maintenance plan's per-cell
/// technique previews. The single value both `smelt-cli`'s `explain` report
/// and `smelt-ui`'s diagnostics endpoint render, verbatim, never re-deriving
/// any field themselves.
#[derive(Debug, Clone, Serialize)]
pub struct ModelDiagnostics {
    pub model: String,
    /// The model's property profile (`docs/specs/property_diff.md` §"The
    /// property profile"): `properties`, `cell_verdicts`, `refusals`, and
    /// `probes`, flattened so the pre-existing `properties` key is
    /// unchanged and the other three sit beside it
    /// (`docs/specs/ui_model_diagnostics.md` §Surface). `smelt-cli`'s
    /// `explain` report and `smelt-ui`'s diagnostics endpoint render this
    /// value, never re-deriving any of its fields.
    #[serde(flatten)]
    pub profile: PropertyProfile,
    pub contract: RelationContractView,
    pub inbound_edges: Vec<InboundEdgeContract>,
    pub cells: Vec<PlanCellDiagnostics>,
}

/// Build a model's [`ModelDiagnostics`] — the property set, relation
/// contract, and per-cell technique-preview set
/// (`docs/specs/ui_model_diagnostics.md` §Surface).
///
/// Pure over already-resolved inputs (Salsa purity rule,
/// `docs/specs/architecture.md` §"Salsa purity rule (analysis)"): every
/// argument is already-derived data a caller assembles from its own
/// Salsa/discovery queries (the same shape `smelt-cli::explain`'s existing
/// report builders already take), so this function never touches a live
/// backend or the maintenance ledger
/// (`docs/specs/ui_model_diagnostics.md` §Constraints). `plan_cells` is the
/// model's already-derived `MaintenancePlan::cells` (empty for a model with
/// no maintenance plan — `full`-mode or a view); `schema`/`target`/
/// `registry`/`resolver`/`dialect`/`source_timeseries` are the same
/// already-resolved compilation facts `smelt-cli::explain`'s `--show-sql`
/// path assembles before calling the (moving) statement-group builder.
///
/// `write_unique_key` is a second, deliberately distinct unique-key input:
/// the effective *write*/MERGE-dedup key `Technique::ColumnScopedMerge`'s
/// preview must key on — `Config::get_incremental_with_metadata`'s
/// config-merged `batched.unique_key` (frontmatter wins wholesale over a
/// `smelt.yml` model override; the *only* surviving spelling for a
/// row-shaped, non-clocked join whose own `.sql` frontmatter cannot carry a
/// top-level `unique_key:` — `docs/specs/models.md` §"The Relation
/// Contract"). This is never the same fact as the model's own top-level
/// `unique_key:` (`model.metadata.unique_key`, read below as
/// `declared_unique_key`): that one is the *identity*/grain fact `row_identity`
/// derives P2 region-row-identity from, and is empty for exactly the
/// row-shaped models whose write key only exists via the `batched:` override
/// (`examples/timeseries/models/daily_events_enriched.sql`'s own doc
/// comment). Passing `declared_unique_key` to both purposes — the shape this
/// function used before this parameter existed — silently starved every such
/// model's `ColumnScopedMerge` preview of its real key.
/// `refusals` and `probe_entries` are the model's already-derived
/// maintenance-plan refusals (`MaintenancePlan::refusals`) and declared-fact
/// probe set (`smelt_runtime::probe_plan::probe_plan_for_model`'s output) —
/// folded, unchanged, into the returned [`ModelDiagnostics`]'s property
/// profile (`docs/specs/property_diff.md` §"The property profile") rather
/// than re-derived here. `contract_cfg` is the model's declared
/// `contract:` block (or `None`), resolved per cell through
/// `smelt_logical::contract::effective_contract` — the same single-owner
/// resolution `smelt-cli`'s `--json` `contract_point` uses, so the two can
/// never disagree.
#[allow(clippy::too_many_arguments)]
pub fn build_model_diagnostics(
    model: &ModelFile,
    models: &[ModelFile],
    model_upstream: &[String],
    source_infos: &[SourceInfo],
    bound_ctx: &BoundContext,
    plan_cells: &[PlanCell],
    schema: &str,
    target: &str,
    registry: &CompilerRegistry,
    resolver: &EphemeralResolver,
    dialect: MaintenanceDialect,
    source_timeseries: &SourceTimeseriesMap,
    write_unique_key: &[String],
    column_groups: &[ColumnGroup],
    refusals: &[Refusal],
    probe_entries: &[ProbePlanEntry],
    contract_cfg: Option<&ContractConfig>,
) -> Result<ModelDiagnostics, DiagnosticsError> {
    let profile = build_model_profile(
        model,
        bound_ctx,
        plan_cells,
        column_groups,
        refusals,
        probe_entries,
        contract_cfg,
    )?;

    let (contract, inbound_edges) =
        build_relation_contract(model, models, model_upstream, source_infos);

    let cells = plan_cells
        .iter()
        .map(|cell| {
            build_plan_cell_diagnostics(
                cell,
                model,
                schema,
                target,
                registry,
                resolver,
                dialect,
                write_unique_key,
                source_timeseries,
                column_groups,
            )
        })
        .collect();

    Ok(ModelDiagnostics {
        model: model.canonical_path(),
        profile,
        contract,
        inbound_edges,
        cells,
    })
}

/// The profile half of [`build_model_diagnostics`]: `PropertySet::derive`
/// plus the per-cell `effective_contract` fold and `PropertyProfile::
/// assemble`, with no registry, resolver, schema, or compile target —
/// everything a caller needs to get a [`PropertyProfile`] without paying
/// for `build_plan_cell_diagnostics`'s per-technique statement-group
/// preview (`docs/outcomes/20260905-property-diff/phases/05-plan.md` D7).
/// `build_model_diagnostics` calls this, so a profile still has exactly one
/// assembly path (`docs/specs/property_diff.md` §Constraints item 1).
pub fn build_model_profile(
    model: &ModelFile,
    bound_ctx: &BoundContext,
    plan_cells: &[PlanCell],
    column_groups: &[ColumnGroup],
    refusals: &[Refusal],
    probe_entries: &[ProbePlanEntry],
    contract_cfg: Option<&ContractConfig>,
) -> Result<PropertyProfile, DiagnosticsError> {
    let stripped_sql = smelt_parser::strip_frontmatter(&model.content).to_string();
    let declared_unique_key: Vec<String> = model
        .metadata
        .as_deref()
        .and_then(|m| m.unique_key.clone())
        .unwrap_or_default();

    let properties = PropertySet::derive(
        &model.canonical_path(),
        &stripped_sql,
        &declared_unique_key,
        bound_ctx,
    )?;

    let contract_points: Vec<ContractPointView> = plan_cells
        .iter()
        .map(|cell| {
            let group_columns: Vec<String> = column_groups
                .iter()
                .find(|g| g.name() == cell.group)
                .map(|g| g.columns.clone())
                .unwrap_or_default();
            let trigger_address = cell_trigger_address(&cell.trigger).unwrap_or_default();
            smelt_logical::contract::effective_contract(
                contract_cfg,
                &trigger_address,
                &group_columns,
            )
            .into()
        })
        .collect();

    Ok(PropertyProfile::assemble(
        properties,
        plan_cells,
        &contract_points,
        refusals,
        probe_entries,
    ))
}

/// Build the [`BoundContext`] a model's bound/reach derivation needs: one
/// `add_source` per upstream dependency that declares its own `timeseries:`
/// clock. Moved here, verbatim, from `smelt_cli::explain::build_bound_context`
/// (`docs/outcomes/20260905-property-diff/phases/04-plan.md` D9) so
/// `profiles_for_workspace` and the CLI's single-model report build it from
/// the same rule rather than two independent copies — `smelt-cli` keeps a
/// `pub use` re-export at the old path so existing call sites are
/// unaffected.
pub fn build_bound_context(
    model_name: &str,
    graph: &DependencyGraph,
    config: &Config,
) -> BoundContext {
    let mut ctx = BoundContext::new();
    for dep_name in graph.get_upstream(model_name) {
        if let Ok(dep_model) = graph.get_model(&dep_name) {
            let dep_meta = dep_model.metadata.as_deref();
            let ts = config
                .get_timeseries_with_metadata(&dep_name, dep_meta)
                .cloned()
                .or_else(|| dep_meta.and_then(|m| m.timeseries.clone()));
            if let Some(ts) = ts {
                ctx.add_source(&dep_name, &ts.partition_column);
            }
        }
    }
    ctx
}
