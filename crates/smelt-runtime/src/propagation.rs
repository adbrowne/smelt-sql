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

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use anyhow::{bail, Context, Result};
use chrono::Datelike;

use smelt_core::config::{Grain as ConfigGrain, Granularity, RefreshStrategy};
use smelt_core::sources::{MutationProfile as SourceMutationKind, SourceInfo};
use smelt_core::ModelFile;
use smelt_logical::analysis::join_shape::JoinContext;
use smelt_logical::analysis::output_delta::{self, OutputDelta, OutputDeltaFacts};
use smelt_logical::maintenance::edge_type::type_edge;
use smelt_logical::maintenance::propagate::{
    day_ordinal, ordinal_to_iso, required_inputs, DayInterval, Edge, PartitionGrain,
};
use smelt_logical::maintenance::skeleton::skeleton_columns;
use smelt_logical::maintenance::{
    ColumnGroup, MutationProfile as PlanMutationProfile, PartitionLocal, SourceFacts,
};
use smelt_state::landed_deltas::LandedDeltaStore;

use crate::types::KeyedRestriction;

/// One caller-declared per-source delta: the partitions that landed on
/// `source` (bare name, the `sources.` breadcrumb stripped — matches
/// [`Edge::upstream`]'s naming convention) since the last propagation.
#[derive(Debug, Clone)]
pub struct SourceDelta {
    pub source: String,
    pub landed: DayInterval,
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
/// end exclusive) into a [`DayInterval`] of day ordinals. A named CLI error
/// (never a panic) on a malformed range.
pub fn parse_landed_range(value: &str) -> Result<DayInterval> {
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
    Ok(DayInterval::new(
        day_ordinal(
            start_date.year() as i64,
            start_date.month(),
            start_date.day(),
        ),
        day_ordinal(end_date.year() as i64, end_date.month(), end_date.day()),
    ))
}

/// Pair up `--source`/`--landed` flags positionally (the Nth `--source`
/// pairs with the Nth `--landed`) into [`SourceDelta`]s. A named CLI error
/// (never a panic) when the two lists' lengths disagree — `--landed`
/// without a matching `--source`, or vice versa. The no-watermark
/// delegating wrapper (`docs/specs/incremental_models.md` §Surface, "Run
/// flags"): existing callers keep today's behaviour exactly — bare spelling
/// only, equal counts required, no unpaired-source resolution. Use
/// [`pair_source_deltas_with_watermarks`] to also honour the qualified
/// `<address>=<start>..<end>` spelling and resolve an unpaired source from
/// its persisted watermark.
pub fn pair_source_deltas(sources: &[String], landed: &[String]) -> Result<Vec<SourceDelta>> {
    pair_source_deltas_with_watermarks(sources, landed, None, "")
}

/// One `--landed` value's spelling (`incremental_models.md` §Surface,
/// "Run flags"): bare `<start>..<end>` (paired positionally with the
/// `--source` at the same index) or address-qualified
/// `<address>=<start>..<end>` (paired by address, no positional
/// constraint). A bare value never contains `=` (ISO dates don't), so its
/// presence discriminates the two spellings unambiguously.
enum LandedSpelling<'a> {
    Bare,
    Qualified(&'a str, &'a str),
}

fn classify_landed(value: &str) -> LandedSpelling<'_> {
    match value.split_once('=') {
        Some((addr, range)) => LandedSpelling::Qualified(addr, range),
        None => LandedSpelling::Bare,
    }
}

/// Resolve a `--source` with no paired `--landed` from its persisted
/// watermark: `[watermark, now)`. A source with neither a paired `--landed`
/// nor a persisted watermark is a named run error (`run_state.md`
/// §"Per-source watermark": "the refusal names the missing watermark") —
/// never a silent per-source skip that would quietly under-propagate.
fn resolve_from_watermark(
    source: &str,
    store: Option<&LandedDeltaStore>,
    now: &str,
) -> Result<SourceDelta> {
    let watermark = store.and_then(|s| s.watermark(source));
    match watermark {
        Some(w) => Ok(SourceDelta {
            source: source.to_string(),
            landed: DayInterval::new(iso_to_day_ordinal(w)?, iso_to_day_ordinal(now)?),
        }),
        None => bail!(
            "--source '{source}' has neither a paired --landed nor a persisted watermark — pass \
             --landed <start>..<end> (or --landed {source}=<start>..<end>) for it, or run a prior \
             `smelt run` over it so a watermark exists"
        ),
    }
}

fn iso_to_day_ordinal(value: &str) -> Result<i64> {
    let date = chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .with_context(|| format!("malformed ISO date '{value}'"))?;
    Ok(day_ordinal(date.year() as i64, date.month(), date.day()))
}

/// Pair up `--source`/`--landed` flags into [`SourceDelta`]s, honouring both
/// `--landed` spellings and resolving an unpaired source from `store`'s
/// persisted watermark (`docs/specs/run_state.md` §"Per-source watermark").
///
/// - **Bare** `<start>..<end>` values pair positionally, same as
///   [`pair_source_deltas`] — requires equal `--source`/`--landed` counts,
///   unchanged. An entirely empty `landed` list is the one exception: every
///   `--source` is then unpaired and resolves from its watermark.
/// - **Qualified** `<address>=<start>..<end>` values pair by address, no
///   positional constraint; a `--source` absent from the qualified list
///   resolves from its watermark.
/// - Mixing the two spellings in one invocation is refused.
pub fn pair_source_deltas_with_watermarks(
    sources: &[String],
    landed: &[String],
    store: Option<&LandedDeltaStore>,
    now: &str,
) -> Result<Vec<SourceDelta>> {
    let has_bare = landed
        .iter()
        .any(|l| matches!(classify_landed(l), LandedSpelling::Bare));
    let has_qualified = landed
        .iter()
        .any(|l| matches!(classify_landed(l), LandedSpelling::Qualified(_, _)));
    if has_bare && has_qualified {
        bail!(
            "--landed values must not mix the bare '<start>..<end>' and qualified \
             '<address>=<start>..<end>' spellings in one invocation"
        );
    }

    if has_qualified {
        let mut by_addr: HashMap<String, DayInterval> = HashMap::new();
        for l in landed {
            let LandedSpelling::Qualified(addr, range) = classify_landed(l) else {
                unreachable!("has_qualified implies every entry classifies as Qualified");
            };
            by_addr.insert(normalize_source_address(addr), parse_landed_range(range)?);
        }
        return sources
            .iter()
            .map(|src| {
                let normalized = normalize_source_address(src);
                match by_addr.get(&normalized) {
                    Some(landed) => Ok(SourceDelta {
                        source: normalized,
                        landed: *landed,
                    }),
                    None => resolve_from_watermark(&normalized, store, now),
                }
            })
            .collect();
    }

    if landed.is_empty() && store.is_some() && !sources.is_empty() {
        return sources
            .iter()
            .map(|src| resolve_from_watermark(&normalize_source_address(src), store, now))
            .collect();
    }

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

/// Map a declared `timeseries.granularity` to the propagation graph's own
/// grain axis. Only `day`/`month` have a graph axis today — sub-day and
/// coarser-than-month axes are deferred (`incremental_models.md` §Known
/// Divergences: "Hour granularity is declared surface... the propagation
/// layer is day-ordinal; sub-day axes are deferred"). Fails loud
/// (`MaintenanceGraphUnsupportedNode`) rather than silently mis-widening a
/// granularity this module doesn't understand.
fn granularity_grain(g: Granularity) -> Result<PartitionGrain> {
    match g {
        Granularity::Day => Ok(PartitionGrain::Day),
        Granularity::Month => Ok(PartitionGrain::Month),
        other => bail!(
            "MaintenanceGraphUnsupportedNode: declared granularity {other:?} has no day/month \
             propagation-graph axis yet — sub-day and >month axes are deferred"
        ),
    }
}

fn source_grain(info: &SourceInfo) -> Result<PartitionGrain> {
    match &info.timeseries {
        Some(ts) => granularity_grain(ts.granularity),
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
                    Some(ts) => granularity_grain(ts.granularity),
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
            Some(ts) => granularity_grain(ts.granularity),
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
        clamp_days,
        locality_admitted,
        key_scope_by_model,
        ..
    } = derive_clamp_and_locality(models, source_infos)?;

    let workspace_verdicts = workspace_output_delta_verdicts(models, source_infos);
    let mut upstream_group_cache: BTreeMap<String, Vec<(ColumnGroup, OutputDelta)>> =
        BTreeMap::new();
    let mut consumer_facts_cache: BTreeMap<String, (BTreeSet<String>, Vec<ColumnGroup>)> =
        BTreeMap::new();

    let mut edges = Vec::with_capacity(clamp_days.len());
    for ((upstream, downstream), (before_days, after_days)) in clamp_days {
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

        let consumer_key_scope = key_scope_by_model
            .get(&downstream)
            .cloned()
            .unwrap_or_default();
        edges.push(Edge {
            upstream,
            downstream,
            before_days,
            after_days,
            upstream_grain,
            downstream_grain,
            components,
            consumer_key_scope,
        });
    }
    Ok(edges)
}

/// The per-workspace facts [`build_forward_graph`] derives from every
/// model's `MaintenancePlan` in one pass: the widest scan-clamp margin seen
/// per `(upstream, downstream)` edge, and the key-temporal-locality
/// admission verdict for every `grain: key` model this workspace derives a
/// plan for. Factored out of `build_forward_graph` so
/// [`refuse_bare_keyed_origins`] can consult the SAME admission verdicts —
/// never re-deriving them — without threading a new field through
/// `build_forward_graph`'s own `Vec<Edge>` return type (which several
/// existing callers, including tests, already destructure directly).
struct ClampAndLocality {
    clamp_days: BTreeMap<(String, String), (i64, i64)>,
    locality_admitted: BTreeMap<String, bool>,
    /// The established [`smelt_logical::maintenance::locality::LocalitySlice`]
    /// route for every `grain: key` model this workspace admits key temporal
    /// locality for (Phase D3,
    /// `docs/plans/20260715-composed-axes-conditional-maintenance.md` —
    /// "Key→partition projection of observed deltas"). Consulted by
    /// [`plan_since_upstream_with_observed_deltas`] to project a recorded
    /// observed delta to exact partition dirt via
    /// `smelt_logical::maintenance::propagate::project_observed_delta` —
    /// never re-derived; the SAME verdict `locality_admitted` above folds
    /// `Some(_).is_some()` from.
    key_locality_slice: BTreeMap<String, smelt_logical::maintenance::locality::LocalitySlice>,
    /// Every maintained model's own derived key scope (Phase 3,
    /// `docs/outcomes/20260816-scheduler-delta-signatures/outcome.md`): the
    /// key columns of a `PlanCell::key_scope` this model's own derived
    /// `MaintenancePlan` carries on ANY of its cells (populated by the
    /// key-addressed dispatch substitution, `smelt-logical::maintenance::
    /// derive`) — folded here so [`build_forward_graph`] can attach it as an
    /// inbound edge's [`Edge::consumer_key_scope`] without a second
    /// derivation. Absent for a model whose own plan carries no key-scoped
    /// cell.
    key_scope_by_model: BTreeMap<String, Vec<String>>,
}

fn derive_clamp_and_locality(
    models: &[ModelFile],
    source_infos: &[SourceInfo],
) -> Result<ClampAndLocality> {
    // Upstream maintained-model composed outputs admitted so far, keyed by
    // model address (the same key `model_edges` names an upstream by) —
    // mirrors `smelt-db::lib.rs`'s `ref_model_source_facts`/
    // `model_source_granularities` handling: a `grain: key` model whose
    // driving source is ANOTHER maintained model's own admitted composed
    // output must see that upstream as both a `SourceFacts` candidate
    // (`derive_new_data`'s `inputs.source(source)` lookup, which a bare
    // `ModelEdge` never populates — model edges only ever clamp a
    // **partition**-addressed downstream's creation cell,
    // `append_model_edge_cells`'s `output_partition_col` early return) and a
    // clocked-granularity candidate for the locality gate's own structural
    // precondition, not only declared `sources.*` refs
    // (`docs/plans/20260719-prod-w8-composed-axes-followups.md` Phase 6).
    // `smelt-db`'s Salsa queries resolve this recursively for free via
    // memoized recursion; this call site has no query-recursion to lean on,
    // so it re-runs the whole per-model pass to a fixed point instead —
    // each pass can only add a candidate an upstream model's OWN admission
    // resolved in the previous pass, so a chain of N maintained models
    // converges within N passes (never re-deriving admission itself, only
    // widening which already-derived verdicts are visible as candidates).
    //
    // This convergence argument assumes an acyclic model-ref graph. This
    // call site runs before `DependencyGraph::execution_order()` (the real
    // cycle detector) on at least one call path, and `build_forward_graph`
    // itself only rejects a literal self-reference — not a longer cycle.
    // A composed-source candidate can also *remove* an admission (flipping
    // `single_clocked_granularity` from unambiguous to ambiguous), so
    // monotonicity isn't guaranteed on a cyclic graph either — a naive
    // unbounded loop could hang, or oscillate with a period the consecutive-
    // state equality check below wouldn't catch. Bound the loop at
    // `models.len() + 1` passes (enough for the documented N-model
    // convergence argument plus one confirmation pass) and fail loud rather
    // than hang, per root `CLAUDE.md` §"Fail-loud discipline".
    let max_passes = models.len() + 1;
    let mut composed_sources: BTreeMap<String, (SourceFacts, Granularity)> = BTreeMap::new();
    // The workspace's output-delta verdicts, derived once — the SAME map
    // `build_forward_graph`'s own `type_edge` call reads
    // (`workspace_output_delta_verdicts`), so a model edge's `output_shape`
    // (below) is never a second, independent derivation of the same fact.
    let workspace_verdicts = workspace_output_delta_verdicts(models, source_infos);

    for _pass in 0..max_passes {
        let ClampAndLocality {
            clamp_days,
            locality_admitted,
            key_locality_slice,
            key_scope_by_model,
        } = derive_clamp_and_locality_pass(
            models,
            source_infos,
            &composed_sources,
            &workspace_verdicts,
        )?;

        let mut next_composed_sources: BTreeMap<String, (SourceFacts, Granularity)> =
            BTreeMap::new();
        for (addr, admitted) in &locality_admitted {
            if !admitted {
                continue;
            }
            let Some(slice) = key_locality_slice.get(addr) else {
                continue;
            };
            let Some(Some(granularity)) = models
                .iter()
                .find(|m| &m.canonical_path() == addr)
                .and_then(|m| m.metadata.as_deref())
                .map(|m| m.timeseries.as_ref().map(|t| t.granularity))
            else {
                continue;
            };
            let facts = SourceFacts {
                name: addr.clone(),
                // Mirrors `smelt-db::lib.rs::ref_model_source_facts`: a
                // composed maintained output's rows, once written, are not
                // retroactively mutated by a later run touching a different
                // slice — the same append-only posture a declared
                // `timeseries:` source with no explicit `mutation_profile:
                // mutable` gets by default.
                mutation: PlanMutationProfile::AppendOnly,
                partition_col: Some(slice.partition_column().to_string()),
                unique_key: Vec::new(),
                allow_full_scan: false,
            };
            next_composed_sources.insert(addr.clone(), (facts, granularity));
        }

        // `SourceFacts` carries no `PartialEq` — compare the
        // (name, partition_col, granularity) signature convergence tracks
        // by instead of the whole struct.
        let sig = |m: &BTreeMap<String, (SourceFacts, Granularity)>| {
            m.iter()
                .map(|(addr, (facts, g))| (addr.clone(), facts.partition_col.clone(), *g))
                .collect::<Vec<_>>()
        };
        if sig(&next_composed_sources) == sig(&composed_sources) {
            return Ok(ClampAndLocality {
                clamp_days,
                locality_admitted,
                key_locality_slice,
                key_scope_by_model,
            });
        }
        composed_sources = next_composed_sources;
    }

    bail!(
        "MaintenanceGraphUnsupportedNode: the composed-source fixed-point derivation did not \
         converge within {max_passes} passes over {} model(s) — the model-ref graph likely \
         contains a cycle among maintained `grain: key` composed models (the documented \
         N-model convergence argument assumes an acyclic model-ref graph)",
        models.len()
    );
}

/// One pass of [`derive_clamp_and_locality`]'s per-model derivation, over a
/// caller-supplied `composed_sources` candidate map (an upstream maintained
/// model's own admitted-composed-output `SourceFacts` + declared
/// granularity, as folded by a PRIOR pass — see that function's own doc
/// comment for why this is a fixed-point iteration rather than a single
/// walk). Never mutates its input; the caller drives convergence.
fn derive_clamp_and_locality_pass(
    models: &[ModelFile],
    source_infos: &[SourceInfo],
    composed_sources: &BTreeMap<String, (SourceFacts, Granularity)>,
    workspace_verdicts: &BTreeMap<String, OutputDeltaFacts>,
) -> Result<ClampAndLocality> {
    let model_by_addr: BTreeMap<String, &ModelFile> =
        models.iter().map(|m| (m.canonical_path(), m)).collect();

    // (upstream, downstream) -> widest (before_days, after_days) seen across
    // every cell that derives a clamp for that pair.
    let mut clamp_days: BTreeMap<(String, String), (i64, i64)> = BTreeMap::new();

    // Per-address key-temporal-locality verdict for every `grain: key`
    // model this workspace derives a plan for — folded from the SAME
    // `MaintenancePlan::key_locality` `smelt explain` reads (Phase A5),
    // never re-derived (`CLAUDE.md` §"Maintenance-plan purity"). Populated
    // below as every model is visited, then consulted by `model_grain`
    // once the full edge list is assembled.
    let mut locality_admitted: BTreeMap<String, bool> = BTreeMap::new();
    let mut key_locality_slice: BTreeMap<
        String,
        smelt_logical::maintenance::locality::LocalitySlice,
    > = BTreeMap::new();
    // Every model's own derived key scope (`ClampAndLocality::
    // key_scope_by_model`'s own doc comment) — populated from
    // `PlanCell::key_scope` inside the per-model pass below.
    let mut key_scope_by_model: BTreeMap<String, Vec<String>> = BTreeMap::new();

    for model in models {
        let Some(metadata) = model.metadata.as_deref() else {
            continue;
        };
        if metadata.refresh != Some(RefreshStrategy::Incremental) || metadata.grain.is_none() {
            continue;
        }
        let table = model.canonical_path();
        let sql = smelt_parser::strip_frontmatter(&model.content);

        let mut sources: Vec<SourceFacts> = Vec::new();
        let mut model_edges: Vec<smelt_logical::maintenance::derive::ModelEdge> = Vec::new();
        let mut explicitly_mutable: HashSet<String> = HashSet::new();
        let mut seen: BTreeSet<String> = BTreeSet::new();
        // `(bare name, SourceInfo)` pairs for every declared `sources.*` ref
        // this model reads — the same shape `smelt-db::queries::maintenance::
        // build_key_recurrences` consumes (`smelt-db`'s own
        // `derive_model_maintenance_plan_with_edges` call site,
        // `crates/smelt-db/src/lib.rs`) — so route 3's declared
        // `key_recurrence` sub-route admits identically here as it does for
        // `smelt explain` (previously this call site passed `&[]`
        // unconditionally, so a route-3 declared-sub-route composed node —
        // `examples/web_analytics`'s own flagship `silver.events_deduped` —
        // never established locality in the graph layer and its own bare
        // `PartitionGrain::Keyed` classification made
        // `classify_keyed_edges` fail-loud refuse ANY graph containing it,
        // origin or not).
        let mut source_refs: Vec<(String, Option<SourceInfo>)> = Vec::new();
        for r in &model.refs {
            let segs = r.smelt_ref.to_path();
            let bare = bare_name(&segs);
            if !seen.insert(bare.clone()) {
                continue;
            }
            if segs.first().map(|s| s.as_str()) == Some("sources") {
                if let Some(info) = source_infos.iter().find(|s| s.address_segments == segs) {
                    sources.push(smelt_db::queries::maintenance::source_facts(
                        &bare,
                        Some(info),
                        true,
                    ));
                    if info
                        .mutation_profile
                        .as_ref()
                        .is_some_and(|m| m.kind == SourceMutationKind::Mutable)
                    {
                        explicitly_mutable.insert(bare.clone());
                    }
                    source_refs.push((bare.clone(), Some(info.clone())));
                }
                continue;
            }
            let addr = bare.clone();
            if addr == table {
                bail!(
                    "MaintenanceGraphUnsupportedNode: '{table}' is self-referential — the \
                     propagation graph refuses a table-graph cycle rather than treating it as \
                     a day axis (time-unrolled self-edges are not yet supported)"
                );
            }
            if let Some(upstream_model) = model_by_addr.get(&addr) {
                let up_meta = upstream_model.metadata.as_deref();
                let is_maintained =
                    up_meta.map(|m| m.refresh == Some(RefreshStrategy::Incremental)) == Some(true);
                if is_maintained {
                    // A maintained-model upstream is a plan edge of the same
                    // standing as a `sources.*` ref (`incremental_models.md`
                    // §"Upstream model edges"): route it through the SAME
                    // edge-aware derivation `smelt explain` uses
                    // (`derive_model_maintenance_plan_with_edges` →
                    // `append_model_edge_cells`), so the propagation clamp for
                    // this edge equals the creation cell's clamp and an
                    // upstream whose clock cannot be derived is a recorded
                    // refusal (contributing no walkable edge), never a silently
                    // permissive `MutableSnapshot { allow_full_scan: true }`
                    // whole-table synthesis.
                    let clock_col = up_meta
                        .and_then(|m| m.timeseries.as_ref())
                        .map(|ts| ts.partition_column.clone());
                    // Sibling spellings of `clock_col` within the upstream's
                    // own SQL (`ModelEdge::clock_col_aliases`'s doc comment).
                    let clock_col_aliases = clock_col
                        .as_deref()
                        .map(|c| {
                            smelt_logical::analysis::source_bounds::defining_expr_siblings(
                                &upstream_model.content,
                                c,
                            )
                        })
                        .unwrap_or_default();
                    let unique_key = up_meta
                        .and_then(|m| m.unique_key.clone())
                        .unwrap_or_default();
                    // The upstream's own derived output-delta shape
                    // (`ModelEdge::output_shape`'s doc comment): the meet
                    // across whatever per-column-group verdicts
                    // `upstream_output_delta_groups` derives for it — the
                    // SAME per-workspace fold `build_forward_graph`'s own
                    // `type_edge` call reads, never re-derived differently.
                    // `None` when the upstream contributes no groups at all
                    // (unresolvable upstream) rather than an optimistic
                    // guess.
                    let output_shape = upstream_output_delta_groups(
                        &bare,
                        &model_by_addr,
                        source_infos,
                        workspace_verdicts,
                    )
                    .into_iter()
                    .map(|(_, shape)| shape)
                    .reduce(OutputDelta::meet);
                    model_edges.push(smelt_logical::maintenance::derive::ModelEdge {
                        name: bare.clone(),
                        clock_col,
                        clock_col_aliases,
                        unique_key,
                        output_shape,
                    });
                } else {
                    // A `full`-mode or view upstream delivers no incremental
                    // delta (`incremental_models.md` §"Upstream model edges":
                    // "participates in mutation/backfill triggers only") — it
                    // has no creation cell in `smelt explain` either. It is
                    // still a real dependency the backward-resolution graph
                    // must stage, so register it as an unclocked whole-table
                    // edge via the plain source path (the honest widen for an
                    // input with no interval structure).
                    let partition_col = up_meta
                        .and_then(|m| m.timeseries.as_ref())
                        .map(|ts| ts.partition_column.clone());
                    sources.push(SourceFacts {
                        name: bare.clone(),
                        mutation: PlanMutationProfile::MutableSnapshot,
                        partition_col,
                        unique_key: vec![],
                        allow_full_scan: true,
                    });
                }
            }
        }

        // A `grain: key` model's driving source may itself be another
        // maintained model's locality-admitted composed output, not just a
        // declared `sources:` entry — `derive_new_data`'s `inputs.source(source)`
        // lookup (via the `Trigger::NewData` this model's `SourceFacts` list
        // drives) is already agnostic to provenance, so publish every
        // referenced upstream model that cleared the locality gate in a
        // PRIOR pass into the same `SourceFacts` candidate list a declared
        // source populates — mirroring `smelt-db::lib.rs`'s own
        // `ref_model_source_facts`/`model_source_granularities` handling at
        // its `maintenance_plan_report` call site
        // (`docs/plans/20260719-prod-w8-composed-axes-followups.md` Phase 6).
        // A bare `ModelEdge` alone is not enough: `append_model_edge_cells`
        // only ever clamps a **partition**-addressed downstream's creation
        // cell (`output_partition_col` early return), so a `grain: key`
        // downstream needs the composed upstream as an actual `SourceFacts`
        // entry to get a `Trigger::NewData` cell at all. Scoped to `grain:
        // key` models only — a `grain: partition` downstream's pushdown
        // against a composed upstream is already derived through
        // `smelt-logical`'s own model-graph registry, not this path.
        if metadata.grain == Some(ConfigGrain::Key) {
            for edge in &model_edges {
                if let Some((facts, _)) = composed_sources.get(&edge.name) {
                    if !sources.iter().any(|s| s.name == facts.name) {
                        sources.push(facts.clone());
                    }
                }
            }
        }

        // The locality gate's granularity-equality structural precondition
        // (`smelt_logical::maintenance::locality::establish_locality`)
        // needs the driving source's own declared granularity — the same
        // value `smelt-db`'s `check_file_diagnostics` (the `smelt explain`
        // path) computes via `single_clocked_granularity` over every
        // declared source this model references (now including the
        // composed-output `SourceFacts` just pushed above), so a `grain:
        // key` model admits (or refuses) locality identically here and
        // there. The "exactly one clocked candidate, else undecided" rule
        // (`single_clocked_granularity`) is unchanged — this only widens
        // the candidate pool fed into it.
        let driving_source_granularity = if metadata.grain == Some(ConfigGrain::Key) {
            let clocked_granularities: Vec<Granularity> = sources
                .iter()
                .filter_map(|s| {
                    source_infos
                        .iter()
                        .find(|info| bare_name(&info.address_segments) == s.name)
                        .and_then(|info| info.timeseries.as_ref())
                        .map(|ts| ts.granularity)
                        .or_else(|| composed_sources.get(&s.name).map(|(_, g)| *g))
                })
                .collect();
            smelt_logical::maintenance::locality::single_clocked_granularity(clocked_granularities)
        } else {
            None
        };

        let Some(result) = smelt_db::queries::maintenance::derive_model_maintenance_plan_with_edges(
            &sql,
            &table,
            metadata,
            &sources,
            &explicitly_mutable,
            &model_edges,
            driving_source_granularity,
            // Route 3's declared `key_recurrence` fallback (B3,
            // `incremental_shapes.md` §"Key temporal locality" route 3): the
            // SAME `(bare name, key_recurrence)` list `smelt-db`'s own
            // `derive_model_maintenance_plan_with_edges` call site builds
            // via `build_key_recurrences` over the declared `sources.*`
            // refs this model reads, so a route-3 declared-sub-route
            // composed node admits identically here as it does for `smelt
            // explain` — never a separately re-derived admission.
            &smelt_db::queries::maintenance::build_key_recurrences(&source_refs),
            // The propagation graph walk only reads model-edge `NewData`
            // creation cells — a `ColumnAdded` trigger never affects them,
            // so no deployed-schema snapshot is needed here.
            &[],
            // Same rationale as `build_key_recurrences` above: the SAME
            // declared `referential_integrity` facts `smelt-db`'s own
            // `smelt explain` derivation reaches
            // (`build_source_referential_integrity`), so a declared-route
            // `UpstreamMutation` cell's closure is real here too, never a
            // separately re-derived admission.
            &smelt_db::queries::maintenance::build_source_referential_integrity(&source_refs),
            // Not (yet) backend-aware at this call site — the propagation
            // graph walk reads dirt-interval facts, not the executed
            // technique, so an un-downgraded ideal plan is the correct
            // input here (same posture as `smelt explain`'s report path).
            smelt_logical::maintenance::availability::StateAvailability::all(),
        ) else {
            continue;
        };

        // The admitted key-temporal-locality verdict for this model, if
        // it's `grain: key` — `Some(true)` only once `establish_locality`
        // (`smelt_logical::maintenance::locality`) has admitted it (Phase
        // A5's `key_locality` fold). Consulted by `model_grain` below, once
        // every model in the workspace has been visited.
        if metadata.grain == Some(ConfigGrain::Key) {
            locality_admitted.insert(table.clone(), result.plan.key_locality.is_some());
            if let Some(key_locality) = result.plan.key_locality.as_ref() {
                key_locality_slice.insert(table.clone(), key_locality.slice.clone());
            }
        }

        for cell in &result.plan.cells {
            // Phase 3 (`docs/outcomes/20260816-scheduler-delta-signatures/
            // outcome.md`): a cell substituted onto the key-addressed route
            // (phase 2's dispatch, `derive.rs`'s `key_scope: Some(key_scope)`
            // push site) names the key columns this model's own maintenance
            // plan restricts by — fold it once per model so
            // `build_forward_graph` can attach it to every inbound edge as
            // `Edge::consumer_key_scope`, never re-deriving it.
            if let Some(key_scope) = &cell.key_scope {
                key_scope_by_model.insert(table.clone(), key_scope.keys.clone());
            }
            for clamp in &cell.scans {
                let e = Edge::from_clamp(&table, clamp);
                let entry = clamp_days
                    .entry((clamp.source.clone(), table.clone()))
                    .or_insert((0, 0));
                entry.0 = entry.0.max(e.before_days);
                entry.1 = entry.1.max(e.after_days);
            }
            // A read the derivation could not bound (`PartitionLocal::No`)
            // carries no `ScanClamp` — `cell.scans` stays empty for it
            // (`PlanCell::scans`'s own doc comment: "empty for reads the
            // derivation could not bound — those surface in
            // `partition_local` instead"). Register a zero-margin edge for
            // it anyway so the propagation/resolution graph has a node to
            // walk to at all; the source's own grain (`Unclocked` for an
            // undeclared-timeseries source or model — see `source_grain`/
            // `model_grain` below) is what actually widens every dirty/
            // required interval through it to `DayInterval::WHOLE` via
            // `PartitionGrain::align_outward`, never this margin.
            // `incremental_models.md` §"Backward resolution — what must
            // exist": "The required slice of an unclocked source is the
            // whole table."
            if let PartitionLocal::No { source, .. } = &cell.partition_local {
                clamp_days
                    .entry((source.clone(), table.clone()))
                    .or_insert((0, 0));
                continue;
            }
            // A locality-admitted composed node's own creation cell
            // (`Grain::Key`'s `FoldDelta` corner, `derive_new_data`) is
            // key-addressed, not partition-addressed: it structurally never
            // carries a `ScanClamp` (`partition_local: PartitionLocal::Yes,
            // scans: vec![]` unconditionally — there is no partition axis
            // for `project_source_link` to bound against). This edge's real
            // key→partition inbound margin (B2, "Key→partition dirt
            // projection through composed nodes") is derived from the SAME
            // admitted `KeyLocality` verdict `locality_admitted` above was
            // folded from — `smelt_logical::maintenance::propagate::
            // locality_margin_days` maps the verdict's route (exact for
            // routes 1–2, widened by `r` + margins for route 3) to the day
            // margin this edge carries, so the composed node participates
            // in the graph as a clocked node with a REAL inbound edge
            // instead of the placeholder-exact zero margin the pre-B2 state
            // used (and, before B1, no edge at all). Gated on the admitted
            // locality verdict — never on the mere presence of a
            // `timeseries:` block — so a bare keyed model's own creation
            // cell still contributes no edge at all (unchanged from before
            // this phase; its refusal is `NoAdmissibleTechnique` at
            // plan-derivation time or `MaintenanceGraphUnsupportedNode` if
            // it's otherwise reached as a graph node).
            if cell.scans.is_empty() && locality_admitted.get(&table).copied() == Some(true) {
                if let smelt_logical::maintenance::Trigger::NewData { source } = &cell.trigger {
                    let key_locality = result.plan.key_locality.as_ref().expect(
                        "locality_admitted[table] == Some(true) is set (a few lines above) from \
                         result.plan.key_locality.is_some(), so key_locality must be Some here",
                    );
                    let (before_days, after_days) =
                        smelt_logical::maintenance::propagate::locality_margin_days(
                            &key_locality.slice,
                        );
                    let entry = clamp_days
                        .entry((source.clone(), table.clone()))
                        .or_insert((0, 0));
                    entry.0 = entry.0.max(before_days);
                    entry.1 = entry.1.max(after_days);
                }
            }
        }
    }

    Ok(ClampAndLocality {
        clamp_days,
        locality_admitted,
        key_locality_slice,
        key_scope_by_model,
    })
}

/// Refuse fail-loud when a `--source`/`--landed` delta origin names a
/// **bare** keyed model (`grain: key`, no `timeseries:` declared or one
/// declared but locality not established) whose own derived output-delta
/// shape is `General` or absent — even when the origin has no edge in the
/// assembled graph at all. A bare keyed model whose only downstream reader
/// can't derive a clock for it contributes no walkable edge
/// (`build_forward_graph`'s own "underivable upstream clock is a recorded
/// refusal" behaviour), so without this check an origin naming such a
/// model would otherwise be a **silent no-op** — the delta seeds a dirty
/// entry `propagate` never reflects through any edge, and
/// `plan_since_upstream` prints "nothing to run" and exits 0.
///
/// Narrowed (phase 5, `docs/outcomes/20260809-output-delta-typing/outcome.md`):
/// mirrors [`smelt_logical::maintenance::propagate::classify_keyed_edges`]'s
/// own admission rule — a bare keyed origin whose derived output-delta
/// shape carries at least one non-`General` column group (i.e. an
/// admitted `Addressing::Keyed` component, `type_edge` would derive for
/// its own outbound edges) passes through untouched; the origin case just
/// isn't reachable by `classify_keyed_edges`'s edge-only check, since an
/// edge-less origin is never visited by it. Consults the SAME
/// `locality_admitted` verdict [`build_forward_graph`] derives and the SAME
/// `workspace_output_delta_verdicts` fold, never re-implementing either. A
/// **locality-admitted** composed origin (B1–B3, `incremental_models.md`
/// §"Key temporal locality") passes through untouched regardless — this
/// refusal only ever fires on the bare, `General`-or-absent-shape case.
fn refuse_bare_keyed_origins(
    models: &[ModelFile],
    source_infos: &[SourceInfo],
    deltas: &[SourceDelta],
) -> Result<()> {
    let model_by_addr: BTreeMap<String, &ModelFile> =
        models.iter().map(|m| (m.canonical_path(), m)).collect();
    let ClampAndLocality {
        locality_admitted, ..
    } = derive_clamp_and_locality(models, source_infos)?;
    let workspace_verdicts = workspace_output_delta_verdicts(models, source_infos);

    for delta in deltas {
        let Some(model) = model_by_addr.get(&delta.source) else {
            continue;
        };
        let Some(metadata) = model.metadata.as_deref() else {
            continue;
        };
        if metadata.grain != Some(ConfigGrain::Key) {
            continue;
        }
        let admitted = locality_admitted
            .get(&delta.source)
            .copied()
            .unwrap_or(false);
        if admitted {
            continue;
        }
        let has_addressable_shape = workspace_verdicts
            .get(&delta.source.to_ascii_lowercase())
            .is_some_and(|facts| {
                facts
                    .columns
                    .iter()
                    .any(|(_, shape)| !matches!(shape, OutputDelta::General { .. }))
            });
        if has_addressable_shape {
            continue;
        }
        bail!(
            "MaintenanceGraphUnsupportedNode: '{}' is keyed-grain without an admitted time \
             axis: it has no partition axis for interval dirt to propagate over. Declare a \
             timeseries: block and establish key temporal locality \
             (docs/specs/incremental_models.md §\"Key temporal locality\") to admit it as a \
             locality-admitted composed node that participates in propagation like any other \
             clocked node, or ensure its own derived output-delta shape is addressable (keyed \
             dirt-sets propagate for an admitted KeyedUpsert shape — \
             docs/specs/incremental_models.md §\"Keyed dirt-sets and the narrowed refusal\")",
            delta.source
        );
    }
    Ok(())
}

/// One propagated run: `model` must run over `[start, end)` (ISO dates), or
/// the whole table when `start`/`end` are `None` (an unclocked source's
/// delta — `incremental_models.md` §"The graph layer": "A delta on an
/// unclocked source dirties the whole model... the full-table run is a
/// declared cost").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropagatedRun {
    pub model: String,
    pub start: Option<String>,
    pub end: Option<String>,
}

/// The full `--since-upstream` plan: the propagated per-model runs (in
/// dependency order) plus a human-readable rendering of the dirty set
/// (per-edge and per-model) to print **before** any run executes
/// (`incremental_models.md` §CLI: "Prints the dirty set before acting").
#[derive(Debug, Clone, Default)]
pub struct SinceUpstreamPlan {
    pub runs: Vec<PropagatedRun>,
    pub dirty_set_report: String,
    /// The keyed dirt-set channel's own per-model merge (`model` → the
    /// [`smelt_logical::maintenance::propagate::KeyedDirt`] records that
    /// dirtied it) — the keyed-channel counterpart to `runs`/
    /// `dirty_set_report`'s interval-channel reporting. Populated whenever
    /// the assembled graph carries an admitted keyed edge, regardless of
    /// whether [`plan_since_upstream_with_keyed_seeds`] seeded it (an
    /// unseeded admitted edge still propagates a symbolic
    /// [`smelt_logical::maintenance::propagate::KeyValues::Unresolved`]
    /// record).
    pub keyed_dirty: BTreeMap<String, Vec<smelt_logical::maintenance::propagate::KeyedDirt>>,
}

fn render_interval(iv: &DayInterval) -> String {
    if iv.is_whole() {
        "whole table".to_string()
    } else {
        format!("[{}, {})", ordinal_to_iso(iv.start), ordinal_to_iso(iv.end))
    }
}

/// Compute the forward-propagation plan for `deltas` over the real
/// per-workspace graph (`build_forward_graph`), in `order` (the caller's
/// topological execution order — models absent from `order` are ignored,
/// mirroring how `propagate` only ever dirties nodes reachable from the
/// declared edges).
///
/// A thin wrapper over [`plan_since_upstream_with_observed_deltas`] with an
/// empty observed-delta lookup — every model-edge origin falls back to its
/// declared `--landed` window unwidened-and-unprojected (the D1
/// widen-never-narrow rule's "absent" case, since an empty lookup can never
/// contain a recorded row). Live wiring of the real `_smelt_observed_delta`
/// warehouse table into this call is CLI/backend-read work outside this
/// phase's critical files (`crates/smelt-state/src/`,
/// `crates/smelt-runtime/src/propagation.rs`,
/// `crates/smelt-logical/src/maintenance/{locality,propagate}.rs`) —
/// tracked in `docs/plans/20260715-composed-axes-conditional-maintenance.md`
/// Phase D3's "Decisions taken" note.
pub fn plan_since_upstream(
    models: &[ModelFile],
    source_infos: &[SourceInfo],
    order: &[String],
    deltas: &[SourceDelta],
) -> Result<SinceUpstreamPlan> {
    plan_since_upstream_with_observed_deltas(models, source_infos, order, deltas, &BTreeMap::new())
}

/// One `(model, window_start, window_end)` key into the observed-delta
/// lookup [`plan_since_upstream_with_observed_deltas`] consults — the SAME
/// three-part key `smelt_state::ddl_duckdb`'s `_smelt_observed_delta` table
/// uses as its own `PRIMARY KEY`. `window_start`/`window_end` are ISO date
/// strings, matching [`ordinal_to_iso`]'s own rendering of a
/// [`SourceDelta::landed`] window.
pub type ObservedDeltaKey = (String, String, String);

/// The read-side lookup for `--since-upstream`'s observed-delta
/// consultation: `None` for a `(model, window)` key means "never recorded"
/// (`docs/specs/incremental_models.md` §"The graph layer" — "Empty and
/// absent are distinct"; the widen-never-narrow fallback to the declared
/// `--landed` window applies), while `Some` — even with both vectors empty
/// — means a conditional write ran and recorded its (possibly empty)
/// changed-row set for that exact window.
pub type ObservedDeltaLookup = BTreeMap<ObservedDeltaKey, smelt_state::ddl_duckdb::ObservedDelta>;

/// The exact set of [`ObservedDeltaKey`]s
/// [`plan_since_upstream_with_observed_deltas`] would consult for `deltas` —
/// one key per delta whose `source` names a locality-admitted composed
/// model, derived from the SAME [`derive_clamp_and_locality`]
/// `key_locality_slice` the planner itself reads for eligibility (never a
/// second, independently re-derived "is this origin locality-admitted"
/// rule). A delta whose origin has no admitted key-temporal-locality route
/// (a raw `sources.*` address, or a bare `grain: partition` model) is not
/// eligible at all and contributes no key — a live resolver has nothing to
/// read for it, exactly as [`plan_since_upstream_with_observed_deltas`]
/// itself never looks such an origin up in `observed`.
///
/// Pure — no backend I/O. The live resolver (`propagation_live.rs`) calls
/// this first to know which `(model, window)` keys to actually read off the
/// warehouse, then passes the resulting [`ObservedDeltaLookup`] to
/// [`plan_since_upstream_with_observed_deltas`].
pub fn observed_delta_keys_to_read(
    models: &[ModelFile],
    source_infos: &[SourceInfo],
    deltas: &[SourceDelta],
) -> Result<Vec<ObservedDeltaKey>> {
    let ClampAndLocality {
        key_locality_slice, ..
    } = derive_clamp_and_locality(models, source_infos)?;
    Ok(deltas
        .iter()
        .filter(|d| key_locality_slice.contains_key(&d.source))
        .map(|d| {
            (
                d.source.clone(),
                ordinal_to_iso(d.landed.start),
                ordinal_to_iso(d.landed.end),
            )
        })
        .collect())
}

/// One `(upstream, consumer)` keyed-seed descriptor
/// (`docs/outcomes/20260816-scheduler-delta-signatures/phases/07-plan.md`):
/// everything [`crate::propagation_live::resolve_keyed_seeds`] needs to run
/// ONE group-grain sidecar diff for a consumer's own admitted key-addressed
/// edge onto `upstream`. Bare model addresses and bare db names only —
/// schema qualification is the live half's job
/// (`crate::execute::model_edge_source_identity`), mirroring
/// [`observed_delta_keys_to_read`]'s own "the pure half names WHAT to read,
/// the live half reads it" split.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyedSeedDiff {
    pub upstream: String,
    pub upstream_db_name: String,
    pub consumer: String,
    pub consumer_db_name: String,
    pub upstream_keys: Vec<String>,
    pub digest_columns: Vec<String>,
    pub consumer_clean_sql: String,
}

/// The exact set of [`KeyedSeedDiff`] descriptors the live keyed-seed
/// resolver needs to read for `deltas` — one descriptor per `(upstream,
/// consumer)` pair where `upstream` names a delta origin that is a
/// maintained model in `models` AND `consumer` (any other model in the
/// workspace) admits a key-addressed model-edge cell onto it
/// ([`smelt_db::queries::maintenance::derive_model_maintenance_plan_with_edges`]
/// via [`crate::maintenance_driver::resolve_live_key_addressed_model_edge_cells`]
/// — the SAME resolver the run loop's own dispatch composition calls, never a
/// second independent derivation). A delta origin absent from `models` (a
/// declared `sources.*` address) contributes no descriptor at all — there is
/// no upstream `ModelFile` for any consumer to key-address against.
///
/// An upstream's seed is the **union** of every one of its consumers' own
/// diffs (`docs/specs/incremental_models.md` §"Keyed dirt-sets and the
/// narrowed refusal": the sidecar partition identity is per `(upstream,
/// consumer)`, since each consumer hashes its own digest projection) — this
/// function only enumerates the per-consumer descriptors; the union itself
/// happens in [`fold_keyed_seed_values`], after each descriptor's own diff
/// has been read live.
///
/// Pure — assumes DuckDB when deriving which cells a consumer admits (the
/// group-grain sidecar diff is DuckDB-only regardless of the actual run
/// target; the non-DuckDB degradation is the LIVE read side's job, not
/// descriptor generation — see [`keyed_seed_diff_result_to_key_values`]).
pub fn keyed_seed_diffs_to_read(
    models: &[ModelFile],
    source_infos: &[SourceInfo],
    deltas: &[SourceDelta],
) -> Result<Vec<KeyedSeedDiff>> {
    let model_by_addr: HashMap<String, ModelFile> = models
        .iter()
        .map(|m| (m.canonical_path(), m.clone()))
        .collect();
    let mut out = Vec::new();
    for delta in deltas {
        let Some(upstream_model) = model_by_addr.get(&delta.source) else {
            continue;
        };
        for consumer in models {
            let table = consumer.canonical_path();
            if table == delta.source {
                continue;
            }
            let Some(metadata) = consumer.metadata.as_deref() else {
                continue;
            };
            let model_edges =
                crate::execute::model_edges_for(consumer, &model_by_addr, source_infos);
            let (sources, explicitly_mutable) =
                crate::execute::build_maint_source_facts(consumer, source_infos);
            let sql = smelt_parser::strip_frontmatter(&consumer.content);
            let cells = crate::maintenance_driver::resolve_live_key_addressed_model_edge_cells(
                &sql,
                &table,
                metadata,
                &sources,
                &explicitly_mutable,
                &model_edges,
                smelt_dialect::SqlDialect::DuckDB,
            )?;
            for (edge_name, _cell, _key_scope, upstream_keys, digest_columns, _write) in cells {
                if edge_name != delta.source {
                    continue;
                }
                out.push(KeyedSeedDiff {
                    upstream: edge_name,
                    upstream_db_name: upstream_model.db_name_owned(),
                    consumer: table.clone(),
                    consumer_db_name: consumer.db_name_owned(),
                    upstream_keys,
                    digest_columns,
                    consumer_clean_sql: sql.clone(),
                });
            }
        }
    }
    Ok(out)
}

/// Turn one [`KeyedSeedDiff`]'s live sidecar-diff result into a
/// [`smelt_logical::maintenance::propagate::KeyValues`] (`docs/specs/
/// incremental_models.md` §"Unresolved seeds"): `Ok` folds to `Resolved`
/// (even an empty `Vec` — nothing changed is resolved-and-empty, never
/// unresolved); a non-DuckDB target's `BackendError::UnsupportedFeature`
/// folds to `Unresolved` naming the dialect, the honest degradation rather
/// than a run failure or a fabricated empty set; any other error propagates
/// (fail loud).
pub fn keyed_seed_diff_result_to_key_values(
    result: std::result::Result<Vec<String>, smelt_backend::BackendError>,
) -> Result<smelt_logical::maintenance::propagate::KeyValues> {
    use smelt_logical::maintenance::propagate::KeyValues;
    match result {
        Ok(values) => Ok(KeyValues::Resolved(values)),
        Err(smelt_backend::BackendError::UnsupportedFeature { dialect, feature }) => {
            Ok(KeyValues::Unresolved {
                reason: format!(
                    "'{dialect}' does not support {feature} — the group-grain sidecar diff \
                     that resolves a keyed seed live is DuckDB-only"
                ),
            })
        }
        Err(e) => Err(e.into()),
    }
}

/// Fold every [`KeyValues`](smelt_logical::maintenance::propagate::KeyValues)
/// resolved for one upstream's own consumers into the single seed
/// [`plan_since_upstream_with_keyed_seeds`] consumes for that upstream —
/// the **union** rule §"Keyed dirt-sets and the narrowed refusal" pins.
/// `Resolved` folds by set union (sorted, deduplicated) — a diff that found
/// nothing contributes nothing but does not itself make the fold
/// `Unresolved` (empty-and-resolved is not the same as unresolved). An empty
/// `results` slice or one containing only `Unresolved` entries folds to
/// `Resolved(vec![])` — one consumer's genuine "nothing changed" — UNLESS
/// EVERY entry is `Unresolved`, in which case the fold stays `Unresolved`
/// (naming the first reason) rather than silently claiming resolution no
/// consumer actually achieved.
pub fn fold_keyed_seed_values(
    results: Vec<smelt_logical::maintenance::propagate::KeyValues>,
) -> smelt_logical::maintenance::propagate::KeyValues {
    use smelt_logical::maintenance::propagate::KeyValues;
    let mut values: Vec<String> = Vec::new();
    let mut any_resolved = false;
    let mut first_unresolved_reason: Option<String> = None;
    for kv in results {
        match kv {
            KeyValues::Resolved(v) => {
                any_resolved = true;
                values.extend(v);
            }
            KeyValues::Unresolved { reason } => {
                if first_unresolved_reason.is_none() {
                    first_unresolved_reason = Some(reason);
                }
            }
        }
    }
    if any_resolved {
        values.sort();
        values.dedup();
        KeyValues::Resolved(values)
    } else if let Some(reason) = first_unresolved_reason {
        KeyValues::Unresolved { reason }
    } else {
        // No descriptors at all for this upstream — resolved-and-empty, the
        // same convention an actually-diffed-and-empty consumer gets.
        KeyValues::Resolved(Vec::new())
    }
}

/// [`plan_since_upstream`]'s full form: consults `observed` for every
/// delta origin that names a locality-admitted composed model
/// (`docs/specs/incremental_shapes.md` §"What the composed shape uniquely
/// enables" — exact `--landed` for model edges, Phase D3). For such an
/// origin:
/// - **absent** from `observed` (no entry for `(model, window_start,
///   window_end)`): falls back to the declared `--landed` window unchanged
///   (D1's widen-never-narrow rule — a consumer must not assume a narrower
///   form exists just because this machinery is live).
/// - **present and empty** (`ObservedDelta::is_empty()`): the origin
///   contributes **no** dirt at all — the graph half of the no-op cascade
///   (a fully-suppressed run's downstream has nothing to do).
/// - **present and non-empty**: the recorded `partitions` are projected to
///   exact partition-day intervals via
///   `smelt_logical::maintenance::propagate::project_observed_delta`,
///   using the SAME established [`smelt_logical::maintenance::locality::
///   LocalitySlice`] route `smelt explain`'s own `key_locality` reads
///   (never re-derived) — replacing the whole declared window with the
///   observed, possibly-narrower (or, under route 3, differently-widened)
///   partition set.
///
/// A delta origin that is a raw `sources.*` address, or a maintained model
/// without an admitted key-temporal-locality route (a bare `grain:
/// partition` model, say), is never looked up in `observed` at all — its
/// delta is always the declared `--landed` window, exactly as before this
/// phase.
pub fn plan_since_upstream_with_observed_deltas(
    models: &[ModelFile],
    source_infos: &[SourceInfo],
    order: &[String],
    deltas: &[SourceDelta],
    observed: &ObservedDeltaLookup,
) -> Result<SinceUpstreamPlan> {
    plan_since_upstream_live(
        models,
        source_infos,
        order,
        deltas,
        observed,
        &BTreeMap::new(),
    )
}

/// [`plan_since_upstream`]'s keyed-seed sibling: mirrors
/// [`plan_since_upstream_with_observed_deltas`]'s shape but instead of
/// projecting a recorded observed delta, seeds the keyed dirt-set channel
/// with `keyed_seeds` (node name → resolved
/// [`smelt_logical::maintenance::propagate::KeyValues`]) via
/// `smelt_logical::maintenance::propagate::propagate_with_keys`. The
/// resolved values reach the returned plan's [`SinceUpstreamPlan::keyed_dirty`]
/// and are rendered into `dirty_set_report` alongside the interval channel.
/// Live resolution of the seed values themselves (reading the actual
/// changed keys off the backend) is phase 5's work
/// (`docs/outcomes/20260816-scheduler-delta-signatures/outcome.md`); this
/// function only threads an already-resolved seed map through to the graph.
pub fn plan_since_upstream_with_keyed_seeds(
    models: &[ModelFile],
    source_infos: &[SourceInfo],
    order: &[String],
    deltas: &[SourceDelta],
    keyed_seeds: &BTreeMap<String, smelt_logical::maintenance::propagate::KeyValues>,
) -> Result<SinceUpstreamPlan> {
    plan_since_upstream_live(
        models,
        source_infos,
        order,
        deltas,
        &BTreeMap::new(),
        keyed_seeds,
    )
}

/// The full form both [`plan_since_upstream_with_observed_deltas`] and
/// [`plan_since_upstream_with_keyed_seeds`] delegate to — both channels
/// (observed deltas + keyed seeds) at once. `pub` so
/// `smelt-cli`'s `run_since_upstream` can call it directly once it has
/// resolved BOTH a live observed-delta lookup and a live keyed-seed map
/// (`docs/outcomes/20260816-scheduler-delta-signatures/phases/07-plan.md`)
/// — the two existing wrappers stay as the single-channel convenience
/// entry points tests already depend on.
pub fn plan_since_upstream_live(
    models: &[ModelFile],
    source_infos: &[SourceInfo],
    order: &[String],
    deltas: &[SourceDelta],
    observed: &ObservedDeltaLookup,
    keyed_seeds: &BTreeMap<String, smelt_logical::maintenance::propagate::KeyValues>,
) -> Result<SinceUpstreamPlan> {
    refuse_bare_keyed_origins(models, source_infos, deltas)?;
    let edges = build_forward_graph(models, source_infos)?;
    let ClampAndLocality {
        key_locality_slice, ..
    } = derive_clamp_and_locality(models, source_infos)?;

    let mut source_deltas: BTreeMap<String, Vec<DayInterval>> = BTreeMap::new();
    for d in deltas {
        let projected: Vec<DayInterval> = match key_locality_slice.get(&d.source) {
            Some(slice) => {
                let key = (
                    d.source.clone(),
                    ordinal_to_iso(d.landed.start),
                    ordinal_to_iso(d.landed.end),
                );
                match observed.get(&key) {
                    // Present and empty: a fully-suppressed run recorded
                    // nothing to propagate — the graph half of the no-op
                    // cascade.
                    Some(od) if od.is_empty() => Vec::new(),
                    // Present and non-empty: project the recorded
                    // partitions to exact dirt via the model's own
                    // established locality route.
                    Some(od) => smelt_logical::maintenance::propagate::project_observed_delta(
                        slice,
                        &od.partitions,
                    ),
                    // Absent: never recorded (e.g. a run that predates
                    // conditional-write recording) — widen-never-narrow
                    // fallback to the declared window.
                    None => vec![d.landed],
                }
            }
            // Not a locality-admitted composed-model origin at all (a raw
            // source, or a model with no admitted key-temporal-locality
            // route) — always the declared window, unchanged.
            None => vec![d.landed],
        };
        source_deltas
            .entry(d.source.clone())
            .or_default()
            .extend(projected);
    }

    let prop = smelt_logical::maintenance::propagate::propagate_with_keys(
        &edges,
        &source_deltas,
        keyed_seeds,
    )
    .map_err(|e| anyhow::anyhow!("MaintenanceGraphUnsupportedNode: {e}",))?;

    let mut report = String::from("Dirty set (--since-upstream):\n");
    if prop.per_edge.is_empty() && prop.per_edge_keys.is_empty() {
        report.push_str("  (no source landed a delta that any model reads — nothing to run)\n");
    }
    for ((downstream, upstream), intervals) in &prop.per_edge {
        for iv in intervals {
            report.push_str(&format!(
                "  {downstream} <- {upstream}: {}\n",
                render_interval(iv)
            ));
        }
    }
    for ((downstream, upstream), keyed) in &prop.per_edge_keys {
        for kd in keyed {
            report.push_str(&format!(
                "  {downstream} <- {upstream}: keyed {:?} = {:?}\n",
                kd.keys, kd.values
            ));
        }
    }
    // Only real models (nodes in the caller's topological `order`) are ever
    // run — `prop.dirty` also carries the seeded deltas themselves (a delta
    // origin is its own "dirty" entry before any edge reflects it). A raw
    // *source* origin is filtered by `order_set` (sources are never in the
    // topological model order); a *maintained-model* origin (a `--source
    // <model-address>` delta, `incremental_models.md` §"Upstream model edges":
    // "a model's landed delta is the output window a completed run wrote for
    // it") IS in `order`, but its run already happened — it must propagate to
    // its downstreams without being re-run itself. `origin_names` excludes
    // both.
    let order_set: BTreeSet<&str> = order.iter().map(|s| s.as_str()).collect();
    let origin_names: BTreeSet<&str> = deltas.iter().map(|d| d.source.as_str()).collect();
    for (model, intervals) in &prop.dirty {
        if !order_set.contains(model.as_str()) || origin_names.contains(model.as_str()) {
            continue;
        }
        for iv in intervals {
            report.push_str(&format!("  RUN {model}: {}\n", render_interval(iv)));
        }
    }

    let mut runs = Vec::new();
    for name in order {
        if origin_names.contains(name.as_str()) {
            continue;
        }
        let Some(intervals) = prop.dirty.get(name) else {
            continue;
        };
        for iv in intervals {
            if iv.is_whole() {
                runs.push(PropagatedRun {
                    model: name.clone(),
                    start: None,
                    end: None,
                });
            } else {
                runs.push(PropagatedRun {
                    model: name.clone(),
                    start: Some(ordinal_to_iso(iv.start)),
                    end: Some(ordinal_to_iso(iv.end)),
                });
            }
        }
    }

    Ok(SinceUpstreamPlan {
        runs,
        dirty_set_report: report,
        keyed_dirty: prop.keyed_dirty,
    })
}

/// Convert a [`SinceUpstreamPlan::keyed_dirty`] channel into the
/// [`ExecuteRequest::keyed_restrictions`](crate::types::ExecuteRequest::keyed_restrictions)
/// wire shape the CLI's `run_since_upstream` populates each per-model
/// request from (`docs/specs/incremental_models.md` §"Restrictions compose
/// by union", phase 5,
/// `docs/outcomes/20260816-scheduler-delta-signatures/phases/05-plan.md`).
/// Pure data conversion — only [`smelt_logical::maintenance::propagate::
/// KeyValues::Resolved`] entries contribute a [`KeyedRestriction`]; an
/// unresolved entry contributes **nothing** to the map (never narrows the
/// union its consumer computes) rather than erroring or defaulting to an
/// empty restriction. Values are sorted and deduplicated per entry, mirroring
/// the union arithmetic performed downstream in `maintenance_driver.rs`.
pub fn keyed_restrictions_from_plan(
    plan: &SinceUpstreamPlan,
) -> BTreeMap<String, Vec<KeyedRestriction>> {
    let mut out: BTreeMap<String, Vec<KeyedRestriction>> = BTreeMap::new();
    for (model, keyed) in &plan.keyed_dirty {
        for kd in keyed {
            let smelt_logical::maintenance::propagate::KeyValues::Resolved(values) = &kd.values
            else {
                continue;
            };
            let mut values = values.clone();
            values.sort();
            values.dedup();
            out.entry(model.clone())
                .or_default()
                .push(KeyedRestriction {
                    upstream: kd.from.clone(),
                    keys: kd.keys.clone(),
                    values,
                });
        }
    }
    out
}

/// The resolved backward-resolution plan for `smelt build --include-upstreams`:
/// the per-ancestor required slices (raw sources to stage, model regions to
/// build) plus the ancestor-first/target-last build order, and a
/// human-readable rendering to print **before** any build executes (mirrors
/// [`SinceUpstreamPlan`]'s "print before acting" shape).
#[derive(Debug, Clone, Default)]
pub struct ResolvedBuildPlan {
    /// The models to build, in dependency order, target last. Raw sources
    /// are never in this list — they are staged (verified to exist), not
    /// built.
    pub build_order: Vec<PropagatedRun>,
    pub report: String,
}

/// Resolve what must exist upstream for `target` to be correct over
/// `period`, over the SAME real per-workspace graph
/// [`build_forward_graph`] assembles for `--since-upstream` — the graph
/// layer's two directions share one edge object
/// (`incremental_models.md` §"The clamp both directions"). Delegates the
/// actual reverse-topological resolution to
/// [`smelt_logical::maintenance::propagate::required_inputs`]; this
/// function only assembles the graph, renders the report, and shapes the
/// per-model build order the CLI executes.
pub fn resolve_build_plan(
    models: &[ModelFile],
    source_infos: &[SourceInfo],
    target: &str,
    period: DayInterval,
) -> Result<ResolvedBuildPlan> {
    let edges = build_forward_graph(models, source_infos)?;

    let resolved = required_inputs(&edges, target, period)
        .map_err(|e| anyhow::anyhow!("MaintenanceGraphUnsupportedNode: {e}"))?;

    // A required node with at least one inbound edge is a model to build;
    // everything else (a leaf of the ancestor sub-DAG) is a raw source to
    // stage — exactly `required_inputs`'s own `build_order` membership test.
    let buildable: BTreeSet<&str> = resolved.build_order.iter().map(|s| s.as_str()).collect();

    let mut report = String::from("Required upstream slices (--include-upstreams):\n");
    for (node, intervals) in &resolved.required {
        let verb = if buildable.contains(node.as_str()) {
            "BUILD"
        } else {
            "STAGE"
        };
        for iv in intervals {
            report.push_str(&format!("  {verb} {node}: {}\n", render_interval(iv)));
        }
    }
    report.push_str(&format!(
        "Build order: {}\n",
        resolved.build_order.join(", ")
    ));

    let mut build_order = Vec::with_capacity(resolved.build_order.len());
    for name in &resolved.build_order {
        let intervals = resolved.required.get(name).cloned().unwrap_or_default();
        for iv in intervals {
            if iv.is_whole() {
                build_order.push(PropagatedRun {
                    model: name.clone(),
                    start: None,
                    end: None,
                });
            } else {
                build_order.push(PropagatedRun {
                    model: name.clone(),
                    start: Some(ordinal_to_iso(iv.start)),
                    end: Some(ordinal_to_iso(iv.end)),
                });
            }
        }
    }

    Ok(ResolvedBuildPlan {
        build_order,
        report,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_source_address_strips_smelt_and_sources_prefixes() {
        assert_eq!(normalize_source_address("bronze"), "bronze");
        assert_eq!(normalize_source_address("sources.bronze"), "bronze");
        assert_eq!(normalize_source_address("smelt.sources.bronze"), "bronze");
        assert_eq!(normalize_source_address("sources.raw.bronze"), "raw.bronze");
    }

    #[test]
    fn parse_landed_range_parses_iso_dates() {
        let iv = parse_landed_range("2026-01-03..2026-01-04").expect("parse");
        assert_eq!(iv.start + 1, iv.end);
    }

    #[test]
    fn parse_landed_range_rejects_malformed_input() {
        assert!(parse_landed_range("not-a-range").is_err());
        assert!(parse_landed_range("2026-01-04..2026-01-03").is_err());
        assert!(parse_landed_range("bogus..2026-01-03").is_err());
    }

    #[test]
    fn pair_source_deltas_rejects_length_mismatch() {
        let err = pair_source_deltas(
            &["sources.bronze".to_string(), "sources.aux".to_string()],
            &["2026-01-01..2026-01-02".to_string()],
        )
        .expect_err("mismatched counts must error");
        assert!(err.to_string().contains("--source and --landed"));
    }

    #[test]
    fn missing_landed_resolves_watermark_to_now_span() {
        let mut store = LandedDeltaStore::default();
        store.advance_watermark("bronze", "2026-01-01");

        // No --landed at all: resolves from the watermark to `now`.
        let deltas = pair_source_deltas_with_watermarks(
            &["sources.bronze".to_string()],
            &[],
            Some(&store),
            "2026-01-10",
        )
        .expect("resolve from watermark");
        assert_eq!(deltas.len(), 1);
        assert_eq!(deltas[0].source, "bronze");
        assert_eq!(
            deltas[0].landed,
            DayInterval::new(
                iso_to_day_ordinal("2026-01-01").unwrap(),
                iso_to_day_ordinal("2026-01-10").unwrap()
            )
        );

        // An explicit --landed for the same source overrides the watermark.
        let deltas = pair_source_deltas_with_watermarks(
            &["sources.bronze".to_string()],
            &["2026-02-01..2026-02-05".to_string()],
            Some(&store),
            "2026-01-10",
        )
        .expect("explicit landed overrides");
        assert_eq!(
            deltas[0].landed,
            parse_landed_range("2026-02-01..2026-02-05").unwrap()
        );
    }

    #[test]
    fn qualified_landed_pairs_by_address() {
        let deltas = pair_source_deltas_with_watermarks(
            &["sources.bronze".to_string(), "sources.aux".to_string()],
            &["aux=2026-01-05..2026-01-06".to_string()],
            None,
            "2026-01-10",
        );
        // "bronze" is unpaired with no watermark available -> named error.
        assert!(deltas.is_err());

        let mut store = LandedDeltaStore::default();
        store.advance_watermark("bronze", "2026-01-01");
        let deltas = pair_source_deltas_with_watermarks(
            &["sources.bronze".to_string(), "sources.aux".to_string()],
            &["aux=2026-01-05..2026-01-06".to_string()],
            Some(&store),
            "2026-01-10",
        )
        .expect("qualified pairing with watermark fallback");
        assert_eq!(deltas[0].source, "bronze");
        assert_eq!(
            deltas[0].landed,
            DayInterval::new(
                iso_to_day_ordinal("2026-01-01").unwrap(),
                iso_to_day_ordinal("2026-01-10").unwrap()
            )
        );
        assert_eq!(deltas[1].source, "aux");
        assert_eq!(
            deltas[1].landed,
            parse_landed_range("2026-01-05..2026-01-06").unwrap()
        );

        // Mixing bare and qualified spellings is refused.
        let err = pair_source_deltas_with_watermarks(
            &["sources.bronze".to_string(), "sources.aux".to_string()],
            &[
                "2026-01-01..2026-01-02".to_string(),
                "aux=2026-01-05..2026-01-06".to_string(),
            ],
            None,
            "2026-01-10",
        )
        .expect_err("mixed spellings must be refused");
        assert!(err.to_string().contains("must not mix"));
    }

    #[test]
    fn source_without_landed_or_watermark_is_named_error() {
        let err = pair_source_deltas_with_watermarks(
            &["sources.bronze".to_string()],
            &[],
            Some(&LandedDeltaStore::default()),
            "2026-01-10",
        )
        .expect_err("no landed and no watermark must error");
        let msg = err.to_string();
        assert!(msg.contains("bronze"), "error must name the source: {msg}");
        assert!(
            msg.contains("watermark"),
            "error must name the missing watermark: {msg}"
        );
    }

    #[test]
    fn pair_source_deltas_pairs_positionally() {
        let deltas = pair_source_deltas(
            &["sources.bronze".to_string(), "sources.aux".to_string()],
            &[
                "2026-01-01..2026-01-02".to_string(),
                "2026-01-05..2026-01-06".to_string(),
            ],
        )
        .expect("pair");
        assert_eq!(deltas[0].source, "bronze");
        assert_eq!(deltas[1].source, "aux");
    }
}
