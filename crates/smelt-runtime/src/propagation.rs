//! Forward propagation — `smelt run --since-upstream`
//! (`maintenance_plan.md` §"The graph layer", §CLI).
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
//! `derive_model_maintenance_plan` and `smelt-logical`'s pure `propagate`;
//! it never re-implements admission or the graph composition math itself.

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
/// coarser-than-month axes are deferred (`maintenance_plan.md` §Known
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
fn model_grain(model: &ModelFile) -> Result<PartitionGrain> {
    let Some(metadata) = model.metadata.as_deref() else {
        return Ok(PartitionGrain::Unclocked);
    };
    match metadata.grain {
        Some(ConfigGrain::Key) => Ok(PartitionGrain::Keyed),
        _ => match metadata.timeseries.as_ref() {
            Some(ts) => granularity_grain(ts.granularity),
            None => Ok(PartitionGrain::Unclocked),
        },
    }
}

/// Build the real per-workspace propagation graph: one [`Edge`] per
/// `(upstream, downstream)` pair a model's derived `MaintenancePlan` admits
/// a `ScanClamp` for, widened to the maximum clamp margin across every cell
/// that derives one for that pair (widen-never-narrow, `maintenance_plan.md`
/// §"The graph layer"). `upstream` is either a raw source (a `sources.*`
/// ref) or another model in `models` — both resolve through the same
/// `derive_model_maintenance_plan` call, so a chain of models composes
/// through the SAME clamp-derivation path a single source edge does.
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

    // (upstream, downstream) -> widest (before_days, after_days) seen across
    // every cell that derives a clamp for that pair.
    let mut clamp_days: BTreeMap<(String, String), (i64, i64)> = BTreeMap::new();

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
        let mut explicitly_mutable: HashSet<String> = HashSet::new();
        let mut seen: BTreeSet<String> = BTreeSet::new();
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
                let partition_col = upstream_model
                    .metadata
                    .as_deref()
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

        let Some(result) = smelt_db::queries::maintenance::derive_model_maintenance_plan(
            &sql,
            &table,
            metadata,
            &sources,
            &explicitly_mutable,
        ) else {
            continue;
        };

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
            // `maintenance_plan.md` §"Backward resolution — what must
            // exist": "The required slice of an unclocked source is the
            // whole table."
            if let PartitionLocal::No { source, .. } = &cell.partition_local {
                clamp_days
                    .entry((source.clone(), table.clone()))
                    .or_insert((0, 0));
            }
        }
    }

    let mut edges = Vec::with_capacity(clamp_days.len());
    for ((upstream, downstream), (before_days, after_days)) in clamp_days {
        let upstream_grain = if let Some(info) = source_infos
            .iter()
            .find(|s| bare_name(&s.address_segments) == upstream)
        {
            source_grain(info)?
        } else if let Some(m) = model_by_addr.get(&upstream) {
            model_grain(m)?
        } else {
            PartitionGrain::Unclocked
        };
        let downstream_model = model_by_addr.get(&downstream).with_context(|| {
            format!("internal: '{downstream}' not found among discovered models")
        })?;
        let downstream_grain = model_grain(downstream_model)?;
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

/// One propagated run: `model` must run over `[start, end)` (ISO dates), or
/// the whole table when `start`/`end` are `None` (an unclocked source's
/// delta — `maintenance_plan.md` §"The graph layer": "A delta on an
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
/// (`maintenance_plan.md` §CLI: "Prints the dirty set before acting").
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
pub fn plan_since_upstream(
    models: &[ModelFile],
    source_infos: &[SourceInfo],
    order: &[String],
    deltas: &[SourceDelta],
) -> Result<SinceUpstreamPlan> {
    let edges = build_forward_graph(models, source_infos)?;

    let mut source_deltas: BTreeMap<String, Vec<DayInterval>> = BTreeMap::new();
    for d in deltas {
        source_deltas
            .entry(d.source.clone())
            .or_default()
            .push(d.landed);
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
    // run — `prop.dirty` also carries the seeded source deltas themselves
    // (a source is its own "dirty" entry before any edge reflects it), which
    // are not runnable models and must not appear as a RUN line.
    let order_set: BTreeSet<&str> = order.iter().map(|s| s.as_str()).collect();
    for (model, intervals) in &prop.dirty {
        if !order_set.contains(model.as_str()) {
            continue;
        }
        for iv in intervals {
            report.push_str(&format!("  RUN {model}: {}\n", render_interval(iv)));
        }
    }

    let mut runs = Vec::new();
    for name in order {
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
/// (`maintenance_plan.md` §"The clamp both directions"). Delegates the
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
