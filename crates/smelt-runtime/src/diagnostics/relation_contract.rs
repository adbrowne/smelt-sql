use serde::Serialize;

use smelt_core::config::{Grain as ContractGrain, TimeseriesConfig};
use smelt_core::{Granularity, ModelFile, SourceInfo};

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
    /// A source's declared `mutation_profile.lateness`/`source_lateness`,
    /// rendered append-stable in both text and `--json` output as an
    /// orchestration-only world-fact — it is read by nothing in plan
    /// derivation (`docs/specs/model_properties.md` §Constraints "Declared
    /// lateness is orchestration-only"). Always `None` for a `Model`
    /// provider — lateness is a source-only declaration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lateness: Option<String>,
}

impl InboundEdgeContract {
    pub fn source(name: String, contract: RelationContractView, lateness: Option<String>) -> Self {
        InboundEdgeContract {
            name,
            provider: RelationContractProvider::Source,
            contract,
            lateness,
        }
    }

    pub fn model(name: String, contract: RelationContractView) -> Self {
        InboundEdgeContract {
            name,
            provider: RelationContractProvider::Model,
            contract,
            lateness: None,
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
        let lateness = info
            .mutation_profile
            .as_ref()
            .and_then(|mp| mp.lateness.as_ref())
            .map(|l| l.display.clone());
        edges.push(InboundEdgeContract::source(
            name,
            RelationContractView::from_facts(info.timeseries.as_ref(), info.unique_key.as_deref()),
            lateness,
        ));
    }

    edges.sort_by(|a, b| a.name.cmp(&b.name));

    (own_contract, edges)
}
