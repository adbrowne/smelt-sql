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

use std::collections::BTreeMap;

use serde::Serialize;

use smelt_core::config::{Grain as ContractGrain, TimeseriesConfig};
use smelt_core::{Granularity, ModelFile, SourceInfo};
use smelt_logical::analysis::join_shape::JoinContext;
use smelt_logical::analysis::source_bounds::{derive_model_bounds, BoundContext, BoundResult};
use smelt_logical::analysis::walk::{
    model_property_vector, ColumnComparability, ColumnDeterminism, ColumnDiscriminant, DerivedFd,
    Grain as PropertyGrain,
};
use smelt_logical::maintenance::derive::row_identity;
use smelt_logical::maintenance::RowIdentityVerdict;

/// Errors the diagnostics builder can surface. Fail-loud
/// (`CLAUDE.md` §"Fail-loud discipline"): a model whose SQL cannot be
/// classified into a property vector is reported as an error, never silently
/// defaulted to an empty/optimistic property set.
#[derive(Debug, thiserror::Error)]
pub enum DiagnosticsError {
    #[error("could not derive the property set for model {model}: SQL did not parse into an analyzable query tree")]
    PropertyDerivation { model: String },
}

/// The model's full derived-property set (`docs/specs/model_properties.md`
/// §Surface), serialized from the existing single-owner walk output
/// ([`smelt_logical::analysis::walk::model_property_vector`]) plus the two
/// other already-derived, single-call facts reachable at whole-model scope:
/// region row identity ([`row_identity`]) and per-source bound/reach
/// ([`derive_model_bounds`]). This struct never re-derives any of these
/// facts — it is an adapter over the existing walk/derive outputs, adding
/// `Serialize` and giving the composed shape one name.
///
/// Scope note: `model_properties.md` §Surface catalogues properties beyond
/// what is folded into a single top-level `PropertyVector`/bound-map call —
/// several (event-time monotonicity trace, partition alignment, fan-out/
/// cardinality, skeleton-role extraction, …) are scope-, join-, or
/// column-position facts the walk computes internally per node but does not
/// yet expose as a single whole-model derivation, and several catalogue rows
/// are themselves `not-yet`/`partial` maturity in the spec. `PropertySet`
/// covers every `built`-maturity property reachable from one already-derived
/// per-model call; extending it to the remaining catalogue rows needs new
/// plumbing to locate their inputs (e.g. the event-time expression's AST
/// node) and is left to a follow-up phase rather than invented here.
#[derive(Debug, Clone, Serialize)]
pub struct PropertySet {
    /// Output columns of the model, in projection order.
    pub columns: Vec<String>,
    /// The proven grain (keys). Empty ⇒ unkeyed
    /// (`model_properties.md` §"Fan-out / cardinality" — the grain a
    /// conditional write's row identity is built from, see `row_identity`
    /// below).
    pub grain: PropertyGrain,
    /// Query-derived functional dependencies (`model_properties.md` — the
    /// FD set implied by grain + literal columns).
    pub functional_dependencies: Vec<DerivedFd>,
    /// Per-column determinism (`model_properties.md` §"Determinism (run vs
    /// row) and the nondeterminism predicate").
    pub determinism: Vec<ColumnDeterminism>,
    /// Per-column change-comparability (`model_properties.md` §"Change
    /// comparability").
    pub comparability: Vec<ColumnComparability>,
    /// Per-column aggregate discriminants (`model_properties.md` §"Algebraic
    /// discriminants").
    pub discriminants: Vec<ColumnDiscriminant>,
    /// Output columns that are constant literals here, name → literal text.
    pub literal_columns: Vec<(String, String)>,
    /// Whether an output column crosses a set operation whose branches are
    /// not proven key-disjoint — a structural barrier for FD survival.
    pub has_set_op_barrier: bool,
    /// Whether an input join proves `OneToMany` (row-multiplying).
    pub has_fan_out_join: bool,
    /// The model's own region row identity (`model_properties.md` §"Region
    /// row identity"): declared `unique_key` → proven grain key → the
    /// identity-free `WholeRow` fallback.
    pub row_identity: RowIdentityVerdict,
    /// Per-upstream-source bound/reach (`model_properties.md` §"Unified
    /// bound / reach derivation"), keyed by source name.
    pub source_bounds: BTreeMap<String, BoundResult>,
}

impl PropertySet {
    /// Derive a model's [`PropertySet`] from its (frontmatter-stripped) SQL,
    /// its declared `unique_key`, and a caller-built [`BoundContext`]
    /// (mirroring `smelt-cli::explain::compute_source_bounds`'s own
    /// construction: one `BoundContext::add_source` per upstream source with
    /// a declared timeseries clock).
    fn derive(
        model_name: &str,
        sql: &str,
        declared_unique_key: &[String],
        bound_ctx: &BoundContext,
    ) -> Result<Self, DiagnosticsError> {
        let vector = model_property_vector(sql, &JoinContext::new()).ok_or_else(|| {
            DiagnosticsError::PropertyDerivation {
                model: model_name.to_string(),
            }
        })?;
        let identity = row_identity(declared_unique_key, sql);
        let source_bounds: BTreeMap<String, BoundResult> =
            derive_model_bounds(sql, bound_ctx).into_iter().collect();

        Ok(PropertySet {
            columns: vector.columns,
            grain: vector.grain,
            functional_dependencies: vector.fds,
            determinism: vector.determinism,
            comparability: vector.comparability,
            discriminants: vector.discriminants,
            literal_columns: vector.literal_columns,
            has_set_op_barrier: vector.has_set_op_barrier,
            has_fan_out_join: vector.has_fan_out_join,
            row_identity: identity,
            source_bounds,
        })
    }
}

/// The clock slot's shared fields (`docs/specs/models.md` §"The Relation
/// Contract": clock and identity "carry identical field paths" across both
/// providers). Rendered identically whichever provider filled it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RelationContractClock {
    pub event_time_column: String,
    pub partition_column: String,
    pub granularity: Granularity,
}

/// One provider's fill of the Relation Contract's **declared-and-checked**
/// shape-defining slots — the clock and identity — plus the derived `grain`
/// label that summarizes them (`docs/specs/models.md` §"Refresh axis",
/// §"The Relation Contract"; `docs/specs/sources.md` §"The source as a
/// Relation Contract provider"). Both a source and a model output are
/// rendered through this one struct — a consumer never needs to know which
/// provider filled it (`clock`/`identity` are the two fields every provider
/// fills through the same field paths).
///
/// `clock`/`identity`/`derived_grain` are all `None` for a provider that
/// declares neither fact — legal for a source (no admission gate to fail),
/// reported rather than refused.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RelationContractView {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clock: Option<RelationContractClock>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identity: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub derived_grain: Option<ContractGrain>,
}

impl RelationContractView {
    /// Provider-agnostic construction from the raw declared facts — reads
    /// `Option<&TimeseriesConfig>` / `Option<&[String]>` directly rather than
    /// a `SourceInfo` or `ModelMetadata`, so there is exactly one derivation
    /// reused by both providers (`smelt_core::config::derive_grain`), never a
    /// source-specific or model-specific reimplementation.
    pub fn from_facts(
        timeseries: Option<&TimeseriesConfig>,
        unique_key: Option<&[String]>,
    ) -> Self {
        let derived_grain = smelt_core::config::derive_grain(
            timeseries.is_some(),
            unique_key,
            timeseries.map(|t| t.partition_column.as_str()),
        );
        RelationContractView {
            clock: timeseries.map(|ts| RelationContractClock {
                event_time_column: ts.event_time_column.clone(),
                partition_column: ts.partition_column.clone(),
                granularity: ts.granularity,
            }),
            identity: unique_key.map(|k| k.to_vec()),
            derived_grain,
        }
    }
}

/// Which provider filled one inbound edge's [`RelationContractView`] — a
/// declared `sources.*` ref or an upstream maintained model
/// (`docs/specs/incremental_models.md` §"Upstream model edges": the graph
/// layer treats both edge kinds as the same standing).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationContractProvider {
    Source,
    Model,
}

/// One inbound edge's provider identity and Relation Contract fill
/// (`docs/specs/models.md` §"The Relation Contract").
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct InboundEdgeContract {
    pub name: String,
    pub provider: RelationContractProvider,
    pub contract: RelationContractView,
}

impl InboundEdgeContract {
    pub fn source(name: String, contract: RelationContractView) -> Self {
        InboundEdgeContract {
            name,
            provider: RelationContractProvider::Source,
            contract,
        }
    }

    pub fn model(name: String, contract: RelationContractView) -> Self {
        InboundEdgeContract {
            name,
            provider: RelationContractProvider::Model,
            contract,
        }
    }
}

/// Assemble a model's own [`RelationContractView`] plus its inbound edges'
/// contracts (`docs/specs/models.md` §"The Relation Contract").
///
/// Model-to-model edges come from `model_upstream` (`DependencyGraph::
/// get_upstream`) — resolved against `models` for each upstream's own
/// declared clock/identity. Source edges are read directly off `model`'s own
/// `smelt.sources.*` refs (`model.refs`), because the graph layer's
/// dependency map excludes per-entity source refs entirely
/// (`smelt_core::graph::DependencyGraph::build` filters `first == "sources"`
/// out of `deps`) — the graph layer's model/source distinction, not a second
/// one invented here. Edges are sorted by name for a deterministic report.
///
/// This is the single owner of relation-contract derivation
/// (`docs/specs/ui_model_diagnostics.md` §Semantics "Thin-consumer
/// boundary"): `smelt-cli`'s `explain` report calls this function rather
/// than deriving the contract itself.
pub fn build_relation_contract(
    model: &ModelFile,
    models: &[ModelFile],
    model_upstream: &[String],
    source_infos: &[SourceInfo],
) -> (RelationContractView, Vec<InboundEdgeContract>) {
    let own_metadata = model.metadata.as_deref();
    let own_contract = RelationContractView::from_facts(
        own_metadata.and_then(|m| m.timeseries.as_ref()),
        own_metadata.and_then(|m| m.unique_key.as_deref()),
    );

    let mut edges: Vec<InboundEdgeContract> = model_upstream
        .iter()
        .filter_map(|name| {
            models
                .iter()
                .find(|m| &m.canonical_path() == name)
                .map(|m| {
                    let md = m.metadata.as_deref();
                    InboundEdgeContract::model(
                        name.clone(),
                        RelationContractView::from_facts(
                            md.and_then(|m| m.timeseries.as_ref()),
                            md.and_then(|m| m.unique_key.as_deref()),
                        ),
                    )
                })
        })
        .collect();

    for r in &model.refs {
        let segs = r.smelt_ref.to_path();
        if segs.first().map(String::as_str) != Some("sources") {
            continue;
        }
        // `SourceInfo::address_segments` is the full scan-root-stripped path,
        // `sources` segment included (`discover_source_infos` /
        // `ModelDiscovery::compute_address_segments`) — the same segments a
        // `smelt.sources.<...>` ref carries, so no prefix-stripping here.
        let Some(info) = source_infos.iter().find(|s| s.address_segments == segs) else {
            continue;
        };
        let name = segs.join(".");
        if edges.iter().any(|e| e.name == name) {
            continue;
        }
        edges.push(InboundEdgeContract::source(
            name,
            RelationContractView::from_facts(info.timeseries.as_ref(), info.unique_key.as_deref()),
        ));
    }

    edges.sort_by(|a, b| a.name.cmp(&b.name));

    (own_contract, edges)
}

/// A maintenance cell's technique-preview set
/// (`docs/specs/ui_model_diagnostics.md` §Surface "smelt-runtime builder";
/// §Semantics "Technique preview set"). Populated by a later stage of the
/// diagnostics builder — this phase carries the typed placeholder only, and
/// [`ModelDiagnostics::cells`] is always empty until then.
#[derive(Debug, Clone, Serialize)]
pub struct PlanCellDiagnostics {
    /// The cell's column group name (`smelt_logical::maintenance::PlanCell::group`).
    pub group: String,
}

/// A model's full derived state (`docs/specs/ui_model_diagnostics.md`
/// §Surface "smelt-runtime builder"): its property set, its own relation
/// contract plus every inbound edge's, and (once a later stage of this
/// builder populates it) its maintenance plan's per-cell technique previews.
/// The single value both `smelt-cli`'s `explain` report and `smelt-ui`'s
/// diagnostics endpoint render, verbatim, never re-deriving any field
/// themselves.
#[derive(Debug, Clone, Serialize)]
pub struct ModelDiagnostics {
    pub model: String,
    pub properties: PropertySet,
    pub contract: RelationContractView,
    pub inbound_edges: Vec<InboundEdgeContract>,
    /// Always empty in this stage of the builder — populated by the
    /// technique-preview stage.
    pub cells: Vec<PlanCellDiagnostics>,
}

/// Build a model's [`ModelDiagnostics`] — the property set and relation
/// contract only; `cells` is always empty until the technique-preview stage
/// of this builder lands (`docs/specs/ui_model_diagnostics.md` §Surface).
///
/// Pure over already-resolved inputs (Salsa purity rule,
/// `docs/specs/architecture.md` §"Salsa purity rule (analysis)"): every
/// argument is already-derived data a caller assembles from its own
/// Salsa/discovery queries (the same shape `smelt-cli::explain`'s existing
/// report builders already take), so this function never touches a live
/// backend or the maintenance ledger
/// (`docs/specs/ui_model_diagnostics.md` §Constraints).
pub fn build_model_diagnostics(
    model: &ModelFile,
    models: &[ModelFile],
    model_upstream: &[String],
    source_infos: &[SourceInfo],
    bound_ctx: &BoundContext,
) -> Result<ModelDiagnostics, DiagnosticsError> {
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

    let (contract, inbound_edges) =
        build_relation_contract(model, models, model_upstream, source_infos);

    Ok(ModelDiagnostics {
        model: model.canonical_path(),
        properties,
        contract,
        inbound_edges,
        cells: Vec::new(),
    })
}
