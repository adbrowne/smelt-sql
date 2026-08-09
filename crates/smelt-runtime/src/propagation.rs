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

use std::collections::{BTreeMap, BTreeSet, HashSet};

use anyhow::{bail, Context, Result};
use chrono::Datelike;

use smelt_core::config::{Grain as ConfigGrain, Granularity, RefreshStrategy};
use smelt_core::sources::{MutationProfile as SourceMutationKind, SourceInfo};
use smelt_core::ModelFile;
use smelt_logical::maintenance::propagate::{
    day_ordinal, ordinal_to_iso, propagate, required_inputs, DayInterval, Edge, PartitionGrain,
};
use smelt_logical::maintenance::{
    MutationProfile as PlanMutationProfile, PartitionLocal, SourceFacts,
};

/// One caller-declared per-source delta: the partitions that landed on
/// `source` (bare name, the `sources.` breadcrumb stripped — matches
/// [`Edge::upstream`]'s naming convention) since the last propagation.
#[derive(Debug, Clone)]
pub struct SourceDelta {
    pub source: String,
    pub landed: DayInterval,
}

/// Bare name matching `smelt-db::queries::maintenance::source_facts`'s own
/// convention: strip a leading `sources.` breadcrumb only, never collapse a
/// multi-segment address to its last leaf.
fn bare_name(segs: &[String]) -> String {
    match segs.split_first() {
        Some((first, rest)) if first == "sources" => rest.join("."),
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
/// `incremental_models.md` §"Key temporal locality (the time-partitioned
/// output)") is a clocked node at its declared `timeseries.granularity`
/// like any other node (§"The graph layer": "A locality-admitted
/// time-partitioned keyed output is not refused"); a **bare** keyed model —
/// no `timeseries:` declared, or one declared but not admitted — stays
/// [`PartitionGrain::Keyed`] for [`crate::propagate::refuse_keyed_nodes`]-
/// equivalent (`smelt_logical::maintenance::propagate::refuse_keyed_nodes`)
/// to refuse. Admission keys off the locality **verdict**, never off the
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
/// `refuse_keyed_nodes` to catch (so both `propagate` and `required_inputs`
/// share exactly one refusal implementation, per that module's own
/// composition law).
pub fn build_forward_graph(models: &[ModelFile], source_infos: &[SourceInfo]) -> Result<Vec<Edge>> {
    let model_by_addr: BTreeMap<String, &ModelFile> =
        models.iter().map(|m| (m.canonical_path(), m)).collect();

    let ClampAndLocality {
        clamp_days,
        locality_admitted,
        ..
    } = derive_clamp_and_locality(models, source_infos)?;

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
        edges.push(Edge {
            upstream,
            downstream,
            before_days,
            after_days,
            upstream_grain,
            downstream_grain,
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

    for _pass in 0..max_passes {
        let ClampAndLocality {
            clamp_days,
            locality_admitted,
            key_locality_slice,
        } = derive_clamp_and_locality_pass(models, source_infos, &composed_sources)?;

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
        // `PartitionGrain::Keyed` classification made `refuse_keyed_nodes`
        // fail-loud refuse ANY graph containing it, origin or not).
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
            let addr = segs.join(".");
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
                    model_edges.push(smelt_logical::maintenance::derive::ModelEdge {
                        name: bare.clone(),
                        clock_col,
                        clock_col_aliases,
                        unique_key,
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
            // `incremental_models.md` §"Key temporal locality" route 3): the
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
    })
}

/// Refuse fail-loud when a `--source`/`--landed` delta origin names a
/// **bare** keyed model (`grain: key`, no `timeseries:` declared, or one
/// declared but locality not established) — even when the origin has no
/// edge in the assembled graph at all. A bare keyed model whose only
/// downstream reader can't derive a clock for it contributes no walkable
/// edge (`build_forward_graph`'s own "underivable upstream clock is a
/// recorded refusal" behaviour), so without this check an origin naming
/// such a model would otherwise be a **silent no-op** — the delta seeds a
/// dirty entry `propagate` never reflects through any edge, and
/// `plan_since_upstream` prints "nothing to run" and exits 0. That is wrong
/// for the same reason [`smelt_logical::maintenance::propagate`]'s own
/// `refuse_keyed_nodes` fail-loud refuses a bare keyed node reached through
/// an edge (`incremental_models.md` §"The graph layer" — a keyed node
/// without an admitted time axis has no partition axis for interval dirt to
/// propagate over) — the origin case just isn't reachable by that edge-only
/// check, since an edge-less origin is never visited by it. Consults the
/// SAME `locality_admitted` verdict [`build_forward_graph`] derives (never
/// re-implementing admission); a **locality-admitted** composed origin
/// (B1–B3, `incremental_models.md` §"Key temporal locality") passes through
/// untouched — this refusal only ever fires on the bare case.
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
        if !admitted {
            bail!(
                "MaintenanceGraphUnsupportedNode: '{}' is keyed-grain without an admitted time \
                 axis: it has no partition axis for interval dirt to propagate over. Declare a \
                 timeseries: block and establish key temporal locality \
                 (docs/specs/incremental_models.md §\"Key temporal locality\") to admit it as a \
                 locality-admitted composed node that participates in propagation like any other \
                 clocked node — keyed dirt-sets over a bare keyed node are not yet supported \
                 (S12)",
                delta.source
            );
        }
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

/// [`plan_since_upstream`]'s full form: consults `observed` for every
/// delta origin that names a locality-admitted composed model
/// (`docs/specs/incremental_models.md` §"What the composed shape uniquely
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

    let prop = propagate(&edges, &source_deltas)
        .map_err(|e| anyhow::anyhow!("MaintenanceGraphUnsupportedNode: {e}",))?;

    let mut report = String::from("Dirty set (--since-upstream):\n");
    if prop.per_edge.is_empty() {
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
    })
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
