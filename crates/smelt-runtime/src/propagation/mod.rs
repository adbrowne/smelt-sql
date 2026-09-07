//! Forward propagation — `smelt run --since-upstream`
//! (`incremental_models.md` §"The graph layer", §CLI).
//!
//! This module assembles the real per-workspace propagation graph from
//! every model's derived [`MaintenancePlan`](smelt_logical::maintenance::MaintenancePlan)
//! scan clamps — the same clamps that size the maintenance SQL itself,
//! never a hand-typed number — and drives `smelt_logical::maintenance::
//! propagate::propagate` over the caller-declared per-source deltas
//! (`--source <address> --landed <start>..<end>`, repeatable). Per the
//! ratified decision (`docs/plans/20260707-maintenance-plan-impl.md` §
//! "Blocked phases", 2026-07-10), the delta *source* is explicit: no
//! `smelt-state` watermark is read or written here.
//!
//! Per the "Maintenance-plan purity" invariant (root `CLAUDE.md`), this
//! module only *assembles* — it calls `smelt-db`'s pure
//! `derive_model_maintenance_plan_with_edges` (the SAME edge-aware
//! derivation `smelt explain` consumes, so a maintained-model upstream's
//! propagation clamp equals the creation cell's clamp) and `smelt-logical`'s
//! pure `propagate`; it never re-implements admission or the graph
//! composition math itself.

mod clamp_locality;
mod since_upstream;

use clamp_locality::{derive_clamp_and_locality, ClampAndLocality};

pub use since_upstream::{
    load_observed_delta_lookup, plan_since_upstream, plan_since_upstream_with_observed_deltas,
    resolve_build_plan, resolve_run_window, scope_plan_to_selection, ObservedDeltaKey,
    ObservedDeltaLookup, PropagatedRun, ResolvedBuildPlan, SinceUpstreamPlan,
};

use std::collections::{BTreeMap, BTreeSet, HashSet};

use anyhow::{bail, Context, Result};
use chrono::Datelike;

use smelt_core::config::{Grain as ConfigGrain, Granularity, RefreshStrategy};
use smelt_core::sources::{MutationProfile as SourceMutationKind, SourceInfo};
use smelt_core::ModelFile;
use smelt_logical::analysis::join_shape::JoinContext;
use smelt_logical::analysis::output_delta::{self, OutputDelta, OutputDeltaFacts};
use smelt_logical::maintenance::edge_type::type_edge;
use smelt_logical::maintenance::grouping::{dirt_scope, GroupingResult};
use smelt_logical::maintenance::propagate::{
    day_ordinal, day_start, ordinal_to_iso, propagate, required_inputs, Edge, PartitionGrain,
    PartitionInterval, DAY_SECONDS,
};
use smelt_logical::maintenance::skeleton::skeleton_columns;
use smelt_logical::maintenance::{
    ColumnGroup, MutationProfile as PlanMutationProfile, PartitionLocal, SourceFacts,
};

/// One caller-declared per-source delta: the partitions that landed on
/// `source` (bare name, the `sources.` breadcrumb stripped — matches
/// [`Edge::upstream`]'s naming convention) since the last propagation.
#[derive(Debug, Clone)]
pub struct SourceDelta {
    pub source: String,
    pub landed: PartitionInterval,
}

/// Bare name matching `smelt-db::queries::maintenance::source_facts`'s own
/// convention: strip a leading `sources.` or `models.` namespace breadcrumb
/// only, never collapse a multi-segment address to its last leaf. A
/// `smelt.models.<addr>` ref's own segments carry the `models` keyword
/// literally (`SmeltRef::to_path`'s doc comment), which is never part of
/// `ModelFile::canonical_path()` — stripping it here is what lets a
/// model-reference address resolve against `model_by_addr`.
pub(crate) fn bare_name(segs: &[String]) -> String {
    match segs.split_first() {
        Some((first, rest)) if first == "sources" || first == "models" => rest.join("."),
        _ => segs.join("."),
    }
}

/// Parse a CLI-supplied source address (`sources.bronze`, `bronze`, or
/// `smelt.sources.bronze`) into the bare name used as [`Edge`]'s upstream
/// key. Mirrors [`bare_name`] exactly, applied after stripping an optional
/// leading `smelt.` (the CLI's universal "printed identifiers round-trip"
/// convention, `cli.md` §"Argument resolution and `--scope`").
pub fn normalize_source_address(addr: &str) -> String {
    let addr = addr.strip_prefix("smelt.").unwrap_or(addr);
    let segs: Vec<String> = addr.split('.').map(|s| s.to_string()).collect();
    bare_name(&segs)
}

/// Parse a `--landed <start>..<end>` value (ISO `YYYY-MM-DD..YYYY-MM-DD`,
/// end exclusive) into a [`PartitionInterval`] of exact seconds, at the two
/// dates' own midnight boundaries. A named CLI error (never a panic) on a
/// malformed range.
pub fn parse_landed_range(value: &str) -> Result<PartitionInterval> {
    let (start, end) = value
        .split_once("..")
        .with_context(|| format!("malformed --landed range '{value}': expected <start>..<end>"))?;
    let start_date = chrono::NaiveDate::parse_from_str(start, "%Y-%m-%d")
        .with_context(|| format!("malformed --landed range '{value}': invalid start date"))?;
    let end_date = chrono::NaiveDate::parse_from_str(end, "%Y-%m-%d")
        .with_context(|| format!("malformed --landed range '{value}': invalid end date"))?;
    if end_date < start_date {
        bail!("malformed --landed range '{value}': end is before start");
    }
    Ok(PartitionInterval::new(
        day_start(day_ordinal(
            start_date.year() as i64,
            start_date.month(),
            start_date.day(),
        )),
        day_start(day_ordinal(
            end_date.year() as i64,
            end_date.month(),
            end_date.day(),
        )),
    ))
}

/// Pair up `--source`/`--landed` flags positionally (the Nth `--source`
/// pairs with the Nth `--landed`) into [`SourceDelta`]s. A named CLI error
/// (never a panic) when the two lists' lengths disagree — `--landed`
/// without a matching `--source`, or vice versa.
pub fn pair_source_deltas(sources: &[String], landed: &[String]) -> Result<Vec<SourceDelta>> {
    if sources.len() != landed.len() {
        bail!(
            "--source and --landed must be passed the same number of times ({} --source vs {} \
             --landed) — each --source is paired with the --landed at the same position",
            sources.len(),
            landed.len()
        );
    }
    sources
        .iter()
        .zip(landed.iter())
        .map(|(src, lnd)| {
            Ok(SourceDelta {
                source: normalize_source_address(src),
                landed: parse_landed_range(lnd)?,
            })
        })
        .collect()
}

/// [`smelt_core::config::Weekday`] (the declared `week_start` surface) to
/// [`chrono::Weekday`] (`PartitionGrain::Week`'s own alignment type) — the
/// two enums name the same seven days; this is the single conversion point.
fn chrono_weekday(w: &smelt_core::config::Weekday) -> chrono::Weekday {
    use smelt_core::config::Weekday as W;
    match w {
        W::Monday => chrono::Weekday::Mon,
        W::Tuesday => chrono::Weekday::Tue,
        W::Wednesday => chrono::Weekday::Wed,
        W::Thursday => chrono::Weekday::Thu,
        W::Friday => chrono::Weekday::Fri,
        W::Saturday => chrono::Weekday::Sat,
        W::Sunday => chrono::Weekday::Sun,
    }
}

/// Map a declared `timeseries.granularity` to the propagation graph's own
/// grain axis — total over every [`Granularity`] variant (`hour`…`year`),
/// each with a real graph axis (`incremental_models.md` §"The graph
/// layer"). `week_start` sources `PartitionGrain::Week`'s own alignment
/// boundary from the SAME `TimeseriesConfig.week_start` declaration (or its
/// default, Monday) — never a second, independently-defaulted value.
fn granularity_grain(
    g: Granularity,
    week_start: Option<&smelt_core::config::Weekday>,
) -> Result<PartitionGrain> {
    Ok(match g {
        Granularity::Hour => PartitionGrain::Hour,
        Granularity::Day => PartitionGrain::Day,
        Granularity::Week => PartitionGrain::Week {
            start_dow: week_start
                .map(chrono_weekday)
                .unwrap_or(chrono::Weekday::Mon),
        },
        Granularity::Month => PartitionGrain::Month,
        Granularity::Quarter => PartitionGrain::Quarter,
        Granularity::Year => PartitionGrain::Year,
    })
}

fn source_grain(info: &SourceInfo) -> Result<PartitionGrain> {
    match &info.timeseries {
        Some(ts) => granularity_grain(ts.granularity, ts.week_start.as_ref()),
        None => Ok(PartitionGrain::Unclocked),
    }
}

/// A node absent a maintenance plan (not `refresh: incremental`, or an
/// upstream model this workspace doesn't derive a plan for) defaults to
/// [`PartitionGrain::Unclocked`] — the safe "no interval structure" widen,
/// never a silent narrow.
///
/// `locality_admitted` is the per-address key-temporal-locality verdict
/// (`smelt_logical::maintenance::MaintenancePlan::key_locality`, folded by
/// [`build_forward_graph`] from the SAME derivation `smelt explain` reads —
/// never re-derived here) for every `grain: key` model in the workspace.
/// A `grain: key` model whose locality gate admitted (the composed shape,
/// `incremental_shapes.md` §"Key temporal locality (the time-partitioned
/// output)") is a clocked node at its declared `timeseries.granularity`
/// like any other node (§"The graph layer": "A locality-admitted
/// time-partitioned keyed output is not refused"); a **bare** keyed model —
/// no `timeseries:` declared, or one declared but not admitted — stays
/// [`PartitionGrain::Keyed`] for
/// [`smelt_logical::maintenance::propagate::classify_keyed_edges`] to
/// classify (admit through the keyed dirt-set channel, or refuse). Admission keys off the locality **verdict**, never off the
/// mere presence of a `timeseries:` block.
fn model_grain(
    model: &ModelFile,
    locality_admitted: &BTreeMap<String, bool>,
) -> Result<PartitionGrain> {
    let Some(metadata) = model.metadata.as_deref() else {
        return Ok(PartitionGrain::Unclocked);
    };
    match metadata.grain {
        Some(ConfigGrain::Key) => {
            let admitted = locality_admitted
                .get(&model.canonical_path())
                .copied()
                .unwrap_or(false);
            if admitted {
                match metadata.timeseries.as_ref() {
                    Some(ts) => granularity_grain(ts.granularity, ts.week_start.as_ref()),
                    // Unreachable in practice: locality admission requires a
                    // declared `timeseries:` block. Fail closed to `Keyed`
                    // (refused) rather than assume a grain.
                    None => Ok(PartitionGrain::Keyed),
                }
            } else {
                Ok(PartitionGrain::Keyed)
            }
        }
        _ => match metadata.timeseries.as_ref() {
            Some(ts) => granularity_grain(ts.granularity, ts.week_start.as_ref()),
            None => Ok(PartitionGrain::Unclocked),
        },
    }
}

/// This model's declared `sources.*` refs as [`output_delta::SourceFacts`],
/// the per-model input the output-delta walk reads — mirrors
/// [`derive_clamp_and_locality_pass`]'s own declared-source collection, but
/// only the fail-closed leaf-seeding facts the output-delta proof needs
/// (`analysis::output_delta::SourceFacts::from_source_info`), not the
/// maintenance-plan `SourceFacts` shape.
pub(crate) fn model_output_delta_sources(
    model: &ModelFile,
    source_infos: &[SourceInfo],
) -> Vec<output_delta::SourceFacts> {
    let mut sources = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for r in &model.refs {
        let segs = r.smelt_ref.to_path();
        if segs.first().map(|s| s.as_str()) != Some("sources") {
            continue;
        }
        let bare = bare_name(&segs);
        if !seen.insert(bare.clone()) {
            continue;
        }
        if let Some(info) = source_infos.iter().find(|s| s.address_segments == segs) {
            sources.push(output_delta::SourceFacts::from_source_info(&bare, info));
        }
    }
    sources
}

/// This model's skeleton columns (`maintenance::skeleton::skeleton_columns`)
/// from its own declared `unique_key`/`timeseries.partition_column` — the
/// SAME two facts `smelt-db`'s own maintenance-plan derivation reads to seed
/// the identical call, never re-derived differently here.
pub(crate) fn model_skeleton_columns(model: &ModelFile, sql: &str) -> BTreeSet<String> {
    let metadata = model.metadata.as_deref();
    let declared_unique_key = metadata
        .and_then(|m| m.unique_key.clone())
        .unwrap_or_default();
    let partition_col = metadata
        .and_then(|m| m.timeseries.as_ref())
        .map(|ts| ts.partition_column.clone());
    skeleton_columns(sql, &declared_unique_key, partition_col.as_deref())
}

/// Build the per-model [`output_delta::ModelDeltaInput`] records for the
/// cross-model output-delta fold (`output_delta::derive_workspace_output_deltas`),
/// over EVERY model in the workspace — a downstream model-reference leaf may
/// name any model, not only the `refresh: incremental` ones
/// `derive_clamp_and_locality_pass` restricts itself to.
pub(crate) fn workspace_output_delta_verdicts(
    models: &[ModelFile],
    source_infos: &[SourceInfo],
) -> BTreeMap<String, OutputDeltaFacts> {
    let inputs: Vec<output_delta::ModelDeltaInput> = models
        .iter()
        .map(|model| {
            let sql = smelt_parser::strip_frontmatter(&model.content);
            output_delta::ModelDeltaInput {
                address: model.canonical_path(),
                sql,
                ctx: JoinContext::new(),
                sources: model_output_delta_sources(model, source_infos),
            }
        })
        .collect();
    output_delta::derive_workspace_output_deltas(&inputs)
}

/// The upstream's own output-delta verdict per [`ColumnGroup`]
/// (`incremental_models.md` §"The graph layer" → "Typed edges") — a model
/// upstream folds `workspace_verdicts` (cross-model references resolve to
/// the upstream's own upstream's derived shape); a raw source upstream
/// seeds directly from its declared mutation profile
/// (`output_delta::source_output_delta`). Neither (an upstream this
/// workspace cannot locate) contributes no groups — `type_edge` then
/// derives no component for it, never a fabricated one.
pub(crate) fn upstream_output_delta_groups(
    upstream: &str,
    model_by_addr: &BTreeMap<String, &ModelFile>,
    source_infos: &[SourceInfo],
    workspace_verdicts: &BTreeMap<String, OutputDeltaFacts>,
) -> Vec<(ColumnGroup, OutputDelta)> {
    if let Some(model) = model_by_addr.get(upstream) {
        let sql = smelt_parser::strip_frontmatter(&model.content);
        let sources = model_output_delta_sources(model, source_infos);
        let skeleton = model_skeleton_columns(model, &sql);
        return output_delta::derive_output_delta_with_model_verdicts(
            &sql,
            &JoinContext::new(),
            &sources,
            &skeleton,
            workspace_verdicts,
        );
    }
    if let Some(info) = source_infos
        .iter()
        .find(|s| bare_name(&s.address_segments) == upstream)
    {
        let facts = output_delta::SourceFacts::from_source_info(upstream, info);
        return output_delta::source_output_delta(&facts, info);
    }
    Vec::new()
}

/// The downstream consumer's own read columns + derived column groups
/// (`incremental_models.md` §"The graph layer" → "Typed edges") — the two
/// facts [`type_edge`] projects an upstream's shape through.
fn consumer_output_delta_facts(
    model: &ModelFile,
    source_infos: &[SourceInfo],
) -> (BTreeSet<String>, Vec<ColumnGroup>) {
    let sql = smelt_parser::strip_frontmatter(&model.content);
    let sources = model_output_delta_sources(model, source_infos);
    let skeleton = model_skeleton_columns(model, &sql);
    let read_columns = output_delta::referenced_column_names(&sql);
    let groups = output_delta::derive_consumer_column_groups(&sql, &sources, &skeleton);
    (read_columns, groups)
}

/// The downstream consumer's own raw [`GroupingResult`], used only for
/// [`smelt_logical::maintenance::grouping::dirt_scope`] (`incremental_models.md`
/// §"The graph layer" → "Column-group-scoped dirt") — a separate cache from
/// [`consumer_output_delta_facts`] since it needs the whole result
/// (`value_only_sources` included), not just the `Vec<ColumnGroup>` that
/// function's callers need.
fn consumer_grouping_result(model: &ModelFile, source_infos: &[SourceInfo]) -> GroupingResult {
    let sql = smelt_parser::strip_frontmatter(&model.content);
    let sources = model_output_delta_sources(model, source_infos);
    let skeleton = model_skeleton_columns(model, &sql);
    output_delta::derive_consumer_grouping_result(&sql, &sources, &skeleton)
}

/// Build the real per-workspace propagation graph: one [`Edge`] per
/// `(upstream, downstream)` pair a model's derived `MaintenancePlan` admits
/// a `ScanClamp` for, widened to the maximum clamp margin across every cell
/// that derives one for that pair (widen-never-narrow, `incremental_models.md`
/// §"The graph layer"). `upstream` is either a raw source (a `sources.*`
/// ref) or another model in `models`. Both resolve through the same
/// `derive_model_maintenance_plan_with_edges` call `smelt explain` uses: a
/// `sources.*` ref becomes a `SourceFacts` and a maintained-model ref
/// becomes a `ModelEdge`, so a maintained-model edge's clamp equals the
/// creation cell's clamp `smelt explain` reports and an underivable upstream
/// clock is a refusal (no walkable edge) rather than a silently permissive
/// whole-table synthesis (`incremental_models.md` §"Upstream model edges"). A
/// `full`-mode / view upstream carries no incremental delta, so it stays on
/// the plain source path as an unclocked whole-table dependency the
/// backward-resolution graph must still stage.
///
/// Refuses (`MaintenanceGraphUnsupportedNode`) fail-loud on a
/// self-referential model (a ref to its own address) before any interval
/// math runs. A keyed-grain node is left in the graph for
/// [`smelt_logical::maintenance::propagate::propagate`]'s own
/// `classify_keyed_edges` to classify (so both `propagate` and
/// `required_inputs` share exactly one admission/refusal implementation,
/// per that module's own composition law).
pub fn build_forward_graph(models: &[ModelFile], source_infos: &[SourceInfo]) -> Result<Vec<Edge>> {
    let model_by_addr: BTreeMap<String, &ModelFile> =
        models.iter().map(|m| (m.canonical_path(), m)).collect();

    let ClampAndLocality {
        clamp_seconds,
        footprint_seconds,
        locality_admitted,
        ..
    } = derive_clamp_and_locality(models, source_infos)?;

    let workspace_verdicts = workspace_output_delta_verdicts(models, source_infos);
    let mut upstream_group_cache: BTreeMap<String, Vec<(ColumnGroup, OutputDelta)>> =
        BTreeMap::new();
    let mut consumer_facts_cache: BTreeMap<String, (BTreeSet<String>, Vec<ColumnGroup>)> =
        BTreeMap::new();
    let mut consumer_grouping_cache: BTreeMap<String, GroupingResult> = BTreeMap::new();

    let mut edges = Vec::with_capacity(clamp_seconds.len());
    for ((upstream, downstream), (before_seconds, after_seconds)) in clamp_seconds {
        let upstream_grain = if let Some(info) = source_infos
            .iter()
            .find(|s| bare_name(&s.address_segments) == upstream)
        {
            source_grain(info)?
        } else if let Some(m) = model_by_addr.get(&upstream) {
            model_grain(m, &locality_admitted)?
        } else {
            PartitionGrain::Unclocked
        };
        let downstream_model = model_by_addr.get(&downstream).with_context(|| {
            format!("internal: '{downstream}' not found among discovered models")
        })?;
        let downstream_grain = model_grain(downstream_model, &locality_admitted)?;

        let upstream_verdicts = upstream_group_cache
            .entry(upstream.clone())
            .or_insert_with(|| {
                upstream_output_delta_groups(
                    &upstream,
                    &model_by_addr,
                    source_infos,
                    &workspace_verdicts,
                )
            })
            .clone();
        let (consumer_read_columns, consumer_groups) = consumer_facts_cache
            .entry(downstream.clone())
            .or_insert_with(|| consumer_output_delta_facts(downstream_model, source_infos))
            .clone();
        let components = type_edge(
            &upstream,
            &upstream_verdicts,
            &consumer_read_columns,
            &consumer_groups,
        );

        let downstream_grouping = consumer_grouping_cache
            .entry(downstream.clone())
            .or_insert_with(|| consumer_grouping_result(downstream_model, source_infos));
        let dirtied_groups = dirt_scope(&upstream, downstream_grouping);

        let footprint = footprint_seconds
            .get(&(upstream.clone(), downstream.clone()))
            .copied()
            .flatten();

        edges.push(Edge {
            upstream,
            downstream,
            before_seconds,
            after_seconds,
            footprint_seconds: footprint,
            upstream_grain,
            downstream_grain,
            components,
            dirtied_groups,
        });
    }
    Ok(edges)
}
