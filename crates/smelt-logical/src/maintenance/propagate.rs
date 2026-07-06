//! Cross-model dirty-partition propagation — v0 tracer bullet.
//!
//! Runs start from *what changed upstream*, not from a cron tick: given the
//! partition intervals that landed on each source, this module computes
//! which partitions of every downstream model must run, by composing each
//! edge's derived scan clamp through the graph.
//!
//! The per-edge rule is the scan/footprint reflection
//! (`01-framework.md` §5) lifted to whole partitions: an edge whose
//! downstream reads the upstream over `[s − before, e + after)` (the
//! [`ScanClamp`](super::ScanClamp)) means an upstream delta of days
//! `[a, b)` dirties downstream partitions `[a − after, b + before)`. A
//! model's merged dirt across its inbound edges is then the delta its own
//! consumers see, recursively (topological order).
//!
//! v0 boundaries:
//! - **Day-granular**: every partition axis is whole days; clamp seconds are
//!   ceiled *outward* to days, so a 36h lookback dirties 2 whole partitions
//!   (widening is safe, narrowing never is). Grain mapping (a daily model
//!   feeding a monthly rollup) is the named next step — it slots in as an
//!   outward interval-alignment per edge, exactly where the day-ceiling
//!   sits today.
//! - **Whole-partition dirt**: a dirty partition is dirty for every column
//!   group; per-edge results let the caller pick the right trigger cell
//!   (recompute-region for a driving-source delta, column-scoped merge for
//!   an enrichment delta), but column-group-scoped dirt is future work.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use super::ScanClamp;
use crate::analysis::source_bounds::Seconds;

const DAY_SECONDS: u64 = 86_400;

/// Ceil a clamp margin to whole days — a partial-day margin must widen to
/// the whole partition it touches.
fn clamp_days(s: Seconds) -> i64 {
    s.0.div_ceil(DAY_SECONDS) as i64
}

/// A half-open interval `[start, end)` of day ordinals on some table's
/// partition axis (the caller picks the epoch; tests use days since the
/// scenario's first date).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct DayInterval {
    pub start: i64,
    pub end: i64,
}

impl DayInterval {
    pub fn new(start: i64, end: i64) -> Self {
        DayInterval { start, end }
    }

    fn is_empty(&self) -> bool {
        self.start >= self.end
    }
}

/// Sort and merge overlapping/adjacent intervals into a normal form.
pub fn normalize(mut intervals: Vec<DayInterval>) -> Vec<DayInterval> {
    intervals.retain(|i| !i.is_empty());
    intervals.sort();
    let mut merged: Vec<DayInterval> = Vec::new();
    for iv in intervals {
        match merged.last_mut() {
            Some(last) if iv.start <= last.end => last.end = last.end.max(iv.end),
            _ => merged.push(iv),
        }
    }
    merged
}

/// One dependency edge: `downstream` reads `upstream` under a derived scan
/// clamp of `(before_days, after_days)` whole days on the day axis.
#[derive(Debug, Clone)]
pub struct Edge {
    pub upstream: String,
    pub downstream: String,
    pub before_days: i64,
    pub after_days: i64,
}

impl Edge {
    /// Build the edge from a cell's derived [`ScanClamp`] — the same number
    /// that sizes the maintenance SQL sizes the propagation.
    pub fn from_clamp(downstream: &str, clamp: &ScanClamp) -> Self {
        Edge {
            upstream: clamp.source.clone(),
            downstream: downstream.to_string(),
            before_days: clamp_days(clamp.before),
            after_days: clamp_days(clamp.after),
        }
    }

    /// The downstream partitions an upstream delta of `[a, b)` dirties:
    /// the scan reflection, `[a − after, b + before)`.
    fn reflect(&self, delta: &DayInterval) -> DayInterval {
        DayInterval {
            start: delta.start - self.after_days,
            end: delta.end + self.before_days,
        }
    }
}

/// The propagation result: which partitions of which models must run.
#[derive(Debug, Clone, Default)]
pub struct Propagation {
    /// `(model, upstream)` → merged dirty intervals of `model` caused by
    /// that inbound edge. This is the trigger-cell key: the caller runs the
    /// plan cell for `Trigger::…{ source: upstream }` over these regions.
    pub per_edge: BTreeMap<(String, String), Vec<DayInterval>>,
    /// `model` → merged dirty intervals across all inbound edges — what the
    /// model's own consumers see as *their* upstream delta.
    pub dirty: BTreeMap<String, Vec<DayInterval>>,
}

/// Topological order (Kahn's algorithm) over every node mentioned in
/// `edges` plus `extra_nodes`. Errors on a cycle — neither direction of
/// propagation has a well-defined order over a cyclic edge set (a
/// self-referential model is a *time*-DAG, not a table-DAG; see the
/// research doc `10-dependency-propagation.md` §6 for the unrolling that
/// will admit it later).
fn topo_order<'a>(
    edges: &'a [Edge],
    extra_nodes: impl IntoIterator<Item = &'a str>,
) -> Result<Vec<&'a str>, String> {
    let mut nodes: BTreeSet<&str> = BTreeSet::new();
    for e in edges {
        nodes.insert(&e.upstream);
        nodes.insert(&e.downstream);
    }
    for n in extra_nodes {
        nodes.insert(n);
    }
    let mut in_degree: BTreeMap<&str, usize> = nodes.iter().map(|n| (*n, 0)).collect();
    for e in edges {
        if let Some(d) = in_degree.get_mut(e.downstream.as_str()) {
            *d += 1;
        }
    }
    let mut queue: VecDeque<&str> = in_degree
        .iter()
        .filter(|(_, d)| **d == 0)
        .map(|(n, _)| *n)
        .collect();
    let mut order = Vec::with_capacity(nodes.len());
    while let Some(node) = queue.pop_front() {
        order.push(node);
        for e in edges.iter().filter(|e| e.upstream == node) {
            let d = in_degree
                .get_mut(e.downstream.as_str())
                .ok_or_else(|| format!("unknown node '{}'", e.downstream))?;
            *d -= 1;
            if *d == 0 {
                queue.push_back(e.downstream.as_str());
            }
        }
    }
    if order.len() != nodes.len() {
        return Err("dependency graph has a cycle — propagation order is undefined".to_string());
    }
    Ok(order)
}

/// Propagate `source_deltas` (the partitions that landed per source) through
/// `edges`. Nodes are processed in topological order so a model's dirt is
/// complete (all inbound edges merged) before its consumers read it. Errors
/// on a cycle.
pub fn propagate(
    edges: &[Edge],
    source_deltas: &BTreeMap<String, Vec<DayInterval>>,
) -> Result<Propagation, String> {
    let order = topo_order(edges, source_deltas.keys().map(|s| s.as_str()))?;

    let mut result = Propagation::default();
    // Seed: the source deltas are the sources' own "dirty" intervals.
    for (source, intervals) in source_deltas {
        result
            .dirty
            .insert(source.clone(), normalize(intervals.clone()));
    }

    for node in order {
        let node_dirty = result.dirty.get(node).cloned().unwrap_or_default();
        if node_dirty.is_empty() {
            continue;
        }
        for e in edges.iter().filter(|e| e.upstream == node) {
            let reflected: Vec<DayInterval> = node_dirty.iter().map(|iv| e.reflect(iv)).collect();
            let per_edge = result
                .per_edge
                .entry((e.downstream.clone(), e.upstream.clone()))
                .or_default();
            per_edge.extend(reflected.iter().copied());
            *per_edge = normalize(per_edge.clone());
            let model_dirty = result.dirty.entry(e.downstream.clone()).or_default();
            model_dirty.extend(reflected);
            *model_dirty = normalize(model_dirty.clone());
        }
    }
    // Drop models that ended up with no dirt (reachable but untouched).
    result.dirty.retain(|_, v| !v.is_empty());
    Ok(result)
}

/// The backward resolution: everything that must *exist* for a target
/// period to be computable.
#[derive(Debug, Clone, Default)]
pub struct RequiredSlices {
    /// node → merged required partition intervals, for every ancestor of
    /// the target (raw sources *and* intermediate models) plus the target
    /// itself. For a raw source this is the data prerequisite (the slice to
    /// stage or verify); for a model it is the region to build.
    pub required: BTreeMap<String, Vec<DayInterval>>,
    /// The required **models** (nodes with at least one inbound edge) in
    /// dependency order, ending with the target — the order a bounded
    /// test/validation build materializes them.
    pub build_order: Vec<String>,
}

/// Resolve what must exist upstream for `target` to be correct over
/// `period`: walk the ancestor sub-DAG in *reverse* topological order
/// (consumers before producers), pushing each node's merged requirement up
/// its inbound edges using the scan clamp **directly** —
/// `[s, e)` downstream requires `[s − before, e + after)` upstream. This is
/// the dual of [`propagate`] (which uses the clamp *reflected*); the date
/// arithmetic runs backwards relative to the day-to-day forward runs.
///
/// The driving use case is the bounded test/validation build: "build
/// `target` for `period`, including upstreams" — stage exactly
/// `required[source]` per raw source, build `build_order` bottom-up, and
/// the target period equals what a build over complete history would
/// produce (proven by the tracer's sandbox test).
pub fn required_inputs(
    edges: &[Edge],
    target: &str,
    period: DayInterval,
) -> Result<RequiredSlices, String> {
    if period.is_empty() {
        return Ok(RequiredSlices::default());
    }
    let order = topo_order(edges, [target])?;

    let mut required: BTreeMap<String, Vec<DayInterval>> = BTreeMap::new();
    required.insert(target.to_string(), vec![period]);

    // Reverse topological order: a node's requirement is final (every
    // consumer already contributed) before it is pushed up its inbound
    // edges.
    for node in order.iter().rev() {
        let Some(node_required) = required.get(*node).cloned() else {
            continue;
        };
        for e in edges.iter().filter(|e| e.downstream == *node) {
            let widened: Vec<DayInterval> = node_required
                .iter()
                .map(|iv| DayInterval {
                    start: iv.start - e.before_days,
                    end: iv.end + e.after_days,
                })
                .collect();
            let upstream_required = required.entry(e.upstream.clone()).or_default();
            upstream_required.extend(widened);
            *upstream_required = normalize(upstream_required.clone());
        }
    }

    // Build order: the required models (nodes with an inbound edge) in
    // forward topological order — ancestors of the target precede it, so
    // the target is last.
    let has_inbound: BTreeSet<&str> = edges.iter().map(|e| e.downstream.as_str()).collect();
    let build_order: Vec<String> = order
        .iter()
        .filter(|n| required.contains_key(**n) && has_inbound.contains(**n))
        .map(|n| n.to_string())
        .collect();

    Ok(RequiredSlices {
        required,
        build_order,
    })
}
