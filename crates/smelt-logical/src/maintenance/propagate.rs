//! Cross-model dirty-partition propagation — the graph layer
//! (`incremental_models.md` §"The graph layer").
//!
//! Runs start from *what changed upstream*, not from a cron tick: given the
//! partition intervals that landed on each source, this module computes
//! which partitions of every downstream model must run, by composing each
//! edge's derived scan clamp through the graph. `smelt-runtime::propagation`
//! assembles the real per-workspace [`Edge`] list from every model's derived
//! `MaintenancePlan` scan clamps and drives `smelt run --since-upstream`
//! through [`propagate`]; this module stays pure data + pure functions
//! (Salsa-purity compatible by construction) and is exercised directly by
//! its own test suite for the composition math (chains, fan-out, diamonds,
//! granularity mapping, adjointness).
//!
//! The per-edge rule is the scan/footprint reflection
//! (`01-framework.md` §5) lifted to whole partitions: an edge whose
//! downstream reads the upstream over `[s − before, e + after)` (the
//! [`ScanClamp`](super::ScanClamp)) means an upstream delta of days
//! `[a, b)` dirties downstream partitions `[a − after, b + before)`. A
//! model's merged dirt across its inbound edges is then the delta its own
//! consumers see, recursively (topological order).
//!
//! Known boundaries (`incremental_models.md` §Known Divergences):
//! - **Day-ordinal intervals, Day/Month grains**: intervals are whole days
//!   (clamp seconds ceiled *outward* — a 36h lookback dirties 2 whole
//!   partitions; widening is safe, narrowing never is), anchored to the
//!   civil calendar so a coarse axis ([`PartitionGrain::Month`]) can align
//!   them outward to real month boundaries per hop. Edge grains are
//!   *declared* by the caller; deriving Month from a `date_trunc` grouping
//!   is future classifier work, as is an hourly (sub-day) axis.
//! - **Whole-partition dirt**: a dirty partition is dirty for every column
//!   group; per-edge results let the caller pick the right trigger cell
//!   (recompute-region for a driving-source delta, column-scoped merge for
//!   an enrichment delta), but column-group-scoped dirt is future work.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use super::edge_type::{Addressing, EdgeComponent};
use super::{locality, ScanClamp};
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
    /// The whole-axis sentinel: dirt or a requirement with no interval
    /// bound (an unclocked source's delta or slice — ratified P6). Chosen
    /// far outside any civil date a pipeline touches (±1,000,000 days ≈
    /// ±2700 years) yet safely inside `i64` for clamp shifts and calendar
    /// math. Consumers check [`DayInterval::is_whole`], never the raw
    /// bounds.
    pub const WHOLE: DayInterval = DayInterval {
        start: -1_000_000,
        end: 1_000_000,
    };

    pub fn new(start: i64, end: i64) -> Self {
        DayInterval { start, end }
    }

    /// Whether this interval means "the whole table" — it contains the
    /// [`DayInterval::WHOLE`] sentinel (clamp shifts only ever widen it).
    pub fn is_whole(&self) -> bool {
        self.start <= Self::WHOLE.start && self.end >= Self::WHOLE.end
    }

    fn is_empty(&self) -> bool {
        self.start >= self.end
    }
}

#[cfg(test)]
mod day_interval_tests {
    use super::*;

    /// `is_whole` requires BOTH bounds to have widened past the `WHOLE`
    /// sentinel — an interval that widened only one side (start clamped to
    /// `WHOLE.start` but `end` still finite, or vice versa) is not the whole
    /// table. Pins the `&&` (not `||`) in `is_whole`'s definition.
    #[test]
    fn is_whole_requires_both_bounds_to_have_widened() {
        assert!(
            !DayInterval::new(DayInterval::WHOLE.start, 0).is_whole(),
            "a widened start with a finite end must not read as whole"
        );
        assert!(
            !DayInterval::new(0, DayInterval::WHOLE.end).is_whole(),
            "a widened end with a finite start must not read as whole"
        );
        assert!(
            DayInterval::WHOLE.is_whole(),
            "an interval widened on both sides must read as whole"
        );
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

// ---------------------------------------------------------------------------
// Civil-calendar math (pure, dependency-free): day ordinals are days since
// 1970-01-01 (the civil epoch), so grain alignment can find real month
// boundaries. Algorithms after Howard Hinnant's `days_from_civil` /
// `civil_from_days`.
// ---------------------------------------------------------------------------

/// Day ordinal (days since 1970-01-01) of a civil date.
pub fn day_ordinal(year: i64, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400; // [0, 399]
    let m = month as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + day as i64 - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146_097 + doe - 719_468
}

/// Civil `(year, month, day)` of a day ordinal.
pub fn civil_from_ordinal(ordinal: i64) -> (i64, u32, u32) {
    let z = ordinal + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// ISO date literal (`DATE 'YYYY-MM-DD'` body) of a day ordinal.
pub fn ordinal_to_iso(ordinal: i64) -> String {
    let (y, m, d) = civil_from_ordinal(ordinal);
    format!("{y:04}-{m:02}-{d:02}")
}

/// The grain of a table's partition axis. Intervals are always stored in
/// day ordinals; a coarser grain constrains them to aligned boundaries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PartitionGrain {
    #[default]
    Day,
    /// One partition per calendar month.
    Month,
    /// No clock at all (an unclocked lookup/dim): every delta on and every
    /// requirement of this axis is the whole table (ratified P6) — there is
    /// no interval structure to be finer against.
    Unclocked,
    /// Key-addressed end-state: no partition axis. Refused fail-loud in the
    /// propagation graph (ratified P7; S12 keyed dirt-sets are the later
    /// refinement) — silently treating it as a day axis would be
    /// wrong-and-quiet.
    Keyed,
}

impl PartitionGrain {
    /// Align an interval **outward** to this grain's boundaries — a touched
    /// month is a whole dirty (or required) month; an unclocked axis is
    /// all-or-nothing. Outward maps are monotone, so sufficiency composes
    /// hop by hop; narrowing never would.
    fn align_outward(&self, iv: &DayInterval) -> DayInterval {
        match self {
            PartitionGrain::Day => *iv,
            PartitionGrain::Month => {
                let (sy, sm, _) = civil_from_ordinal(iv.start);
                let (ey, em, _) = civil_from_ordinal(iv.end - 1);
                let (ny, nm) = if em == 12 { (ey + 1, 1) } else { (ey, em + 1) };
                DayInterval {
                    start: day_ordinal(sy, sm, 1),
                    end: day_ordinal(ny, nm, 1),
                }
            }
            PartitionGrain::Unclocked => DayInterval::WHOLE,
            // A keyed-grain node has no partition axis to align outward to —
            // an admitted keyed origin's seeded interval is never itself
            // interpreted spatially (`classify_keyed_edges` routes every
            // outbound edge of a keyed-grain node through the keyed
            // dirt-set channel, never `reflect`/`require`); this identity
            // pass-through only keeps the node's own `dirty` entry
            // non-empty so the propagation loop visits it.
            PartitionGrain::Keyed => *iv,
        }
    }
}

/// Route-aware day-margin projection for a locality-admitted composed
/// node's own inbound (driving-source → composed-output) edge
/// (`incremental_models.md` §"What the composed shape uniquely enables"):
/// routes 1–2 project **exactly** — [`locality::LocalitySlice::DeltaValues`]
/// (route 2, key-determined) carries no margin at all, and
/// [`locality::LocalitySlice::Window`] (route 1 key-embedded, or route 3
/// with a statically-derived `r` already folded in) projects only the
/// driving source's own derived read margin. Route 3 with a **declared**
/// `r` ([`locality::LocalitySlice::RecurrenceBounded`]) widens backward by
/// the declared/derived recurrence bound plus lateness/skew margins —
/// already folded into that variant's own `margin_before` by
/// `locality::establish_locality`'s route-3 derivation, so no separate `r`
/// arithmetic is needed here. Never narrows — a route this function doesn't
/// recognize is a compile error (exhaustive match), not a silent default.
pub fn locality_margin_days(slice: &locality::LocalitySlice) -> (i64, i64) {
    use locality::LocalitySlice;
    match slice {
        LocalitySlice::DeltaValues { .. } => (0, 0),
        LocalitySlice::Window {
            margin_before,
            margin_after,
            ..
        }
        | LocalitySlice::RecurrenceBounded {
            margin_before,
            margin_after,
            ..
        } => (clamp_days(*margin_before), clamp_days(*margin_after)),
    }
}

/// Parse an ISO `YYYY-MM-DD` date literal to a day ordinal. Not a general
/// user-input parser — the only strings this ever sees are `smelt-state`'s
/// own `CAST(... AS VARCHAR)` projections of a `DATE`-typed partition
/// column (`crates/smelt-runtime/src/maintenance_driver.rs`'s
/// `changed_keys_select`), so a malformed string here is an internal
/// programming-error signal, not unrecognised user input — the caller
/// ([`project_observed_delta`]) skips an unparseable entry rather than
/// panicking, keeping this module's "pure, no I/O, never panics on its
/// inputs" contract intact.
fn parse_iso_day(s: &str) -> Option<i64> {
    let mut parts = s.splitn(3, '-');
    let y: i64 = parts.next()?.parse().ok()?;
    let m: u32 = parts.next()?.parse().ok()?;
    let d: u32 = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some(day_ordinal(y, m, d))
}

/// Project a locality-admitted composed model's **key-level observed
/// delta** (`smelt-state`'s recorded `partitions` column,
/// `docs/specs/incremental_models.md` §"The graph layer" — "Observed
/// deltas on model edges") to partition-day intervals via its established
/// key-temporal-locality route, so forward propagation dirties exactly the
/// touched partitions instead of the whole declared write window.
///
/// - **Routes 1–2 project exactly** — each observed partition's own day,
///   nothing more: [`locality::LocalitySlice::DeltaValues`] (route 2,
///   key-determined) and a route-1 [`locality::LocalitySlice::Window`]
///   (`recurrence_bounded: false` — key-embedded). A key-embedded or
///   key-determined output's touched partition for a given key IS the
///   partition that key's row lives in; there is no "could also be
///   elsewhere" case to widen for.
/// - **Both route-3 sub-routes widen** by `r` plus lateness/skew margins
///   (`incremental_models.md` §"Row movement", §"What the composed shape
///   uniquely enables"): a key that most recently touched partition `p`
///   may still hold a row recorded up to `r` earlier that this write's
///   observed-delta record does not itself carry.
///   - The **statically-derived** sub-route renders as
///     [`locality::LocalitySlice::Window`] with `recurrence_bounded: true`
///     — structurally the same shape route 1 uses, but its
///     `margin_before`/`margin_after` folds in a proven key-recurrence
///     bound `r` rather than a bare read-side lateness margin, so it must
///     widen the same way the declared sub-route does.
///   - The **declared** sub-route
///     ([`locality::LocalitySlice::RecurrenceBounded`]) widens by that
///     variant's own `margin_before`/`margin_after` (already includes `r`,
///     per its own doc comment).
///
/// An empty `partitions` slice projects to an empty `Vec` — the graph half
/// of the no-op cascade (an empty recorded delta propagates nothing).
/// Malformed partition strings are skipped, never panic (see
/// [`parse_iso_day`]). Never narrows below the observed partitions
/// themselves in any route (asserted as a property by this module's own
/// test suite) — widen-never-narrow.
pub fn project_observed_delta(
    slice: &locality::LocalitySlice,
    partitions: &[String],
) -> Vec<DayInterval> {
    use locality::LocalitySlice;
    let (before, after) = match slice {
        LocalitySlice::DeltaValues { .. } => (0, 0),
        LocalitySlice::Window {
            recurrence_bounded: false,
            ..
        } => (0, 0),
        LocalitySlice::Window {
            margin_before,
            margin_after,
            recurrence_bounded: true,
            ..
        } => (clamp_days(*margin_before), clamp_days(*margin_after)),
        LocalitySlice::RecurrenceBounded {
            margin_before,
            margin_after,
            ..
        } => (clamp_days(*margin_before), clamp_days(*margin_after)),
    };
    let intervals: Vec<DayInterval> = partitions
        .iter()
        .filter_map(|p| parse_iso_day(p))
        .map(|day| DayInterval::new(day - before, day + 1 + after))
        .collect();
    normalize(intervals)
}

/// One keyed dirt-set record on the keyed channel (`incremental_models.md`
/// §"The graph layer" → "Keyed dirt-sets and the narrowed refusal"): a
/// **symbolic** key-addressed delta — the upstream's admitted
/// [`Addressing::Keyed`] key columns and the upstream node's own name — not
/// a materialised key-value set. Value-level affected-key discovery stays
/// with the run-time mechanism (§"Affected-key discovery"); this is only
/// the graph-layer marker that an edge's dirt is key-addressed rather than
/// interval-addressed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyedDirt {
    pub keys: Vec<String>,
    pub from: String,
}

/// Per-edge classification against [`PartitionGrain::Keyed`] endpoints
/// (`classify_keyed_edges`'s return shape): an edge touching neither
/// endpoint as keyed-grain routes through ordinary interval math
/// unchanged; a keyed-grain edge whose component vector carries an
/// admitted [`Addressing::Keyed`] routes through the keyed dirt-set
/// channel instead of being refused.
enum KeyedAdmission {
    NotKeyed,
    Admitted(Vec<String>),
}

/// The keyed-grain endpoint of `e`, if either side is
/// [`PartitionGrain::Keyed`] — upstream checked first (a node absent any
/// inbound edge in the graph, the only way a bare keyed node's own edges
/// are ever assembled per `smelt-runtime::propagation::build_forward_graph`,
/// only ever appears as an edge's `upstream`).
fn keyed_endpoint(e: &Edge) -> Option<(&str, PartitionGrain)> {
    if e.upstream_grain == PartitionGrain::Keyed {
        Some((&e.upstream, e.upstream_grain))
    } else if e.downstream_grain == PartitionGrain::Keyed {
        Some((&e.downstream, e.downstream_grain))
    } else {
        None
    }
}

/// Classify one edge: admit iff some component carries an
/// [`Addressing::Keyed`] (`incremental_models.md` §"The graph layer" →
/// "Keyed dirt-sets and the narrowed refusal"). Otherwise refuse fail-loud
/// — naming the degrading operator when a [`Addressing::WholeModel`]
/// component names one, or the original bare-keyed wording (ratified P7)
/// verbatim when the component vector is empty (fail-closed: no derived
/// shape at all is not evidence of an addressable one). A **locality-
/// admitted** keyed node (the composed shape, `incremental_models.md`
/// §"Key temporal locality (the time-partitioned output)") never reaches
/// this check as [`PartitionGrain::Keyed`] at all — the caller
/// (`smelt-runtime::propagation::build_forward_graph`) classifies it by its
/// declared `timeseries.granularity` instead, so it composes through the
/// graph like any other clocked node (`incremental_models.md` §"The graph
/// layer": "A locality-admitted time-partitioned keyed output is not
/// refused").
fn classify_keyed_edge(e: &Edge) -> Result<KeyedAdmission, String> {
    let Some((name, grain)) = keyed_endpoint(e) else {
        return Ok(KeyedAdmission::NotKeyed);
    };
    debug_assert_eq!(grain, PartitionGrain::Keyed);

    if let Some(keys) = e.components.iter().find_map(|c| match &c.addressing {
        Addressing::Keyed { keys } => Some(keys.clone()),
        _ => None,
    }) {
        return Ok(KeyedAdmission::Admitted(keys));
    }

    if e.components.is_empty() {
        return Err(format!(
            "'{name}' is keyed-grain without an admitted time axis: it has no partition axis \
             for interval dirt to propagate over. Declare a timeseries: block and establish \
             key temporal locality (docs/specs/incremental_models.md §\"Key temporal \
             locality\") to admit it as a locality-admitted composed node that participates in \
             propagation like any other clocked node — keyed dirt-sets over a bare keyed node \
             are not yet supported (S12)"
        ));
    }

    let degraded_by = e
        .components
        .iter()
        .find_map(|c| match &c.addressing {
            Addressing::WholeModel { degraded_by } => Some(degraded_by.clone()),
            _ => None,
        })
        .unwrap_or_else(|| "no component carries an admitted addressing".to_string());
    Err(format!(
        "'{name}' is keyed-grain and its derived shape degraded to general ({degraded_by}): \
         there is no partition axis or keyed dirt-set for interval dirt to propagate over. \
         Address the degrading operator named above, or declare a timeseries: block and \
         establish key temporal locality (docs/specs/incremental_models.md §\"Keyed dirt-sets \
         and the narrowed refusal\"), to admit propagation"
    ))
}

/// Classify every edge in `edges` against [`PartitionGrain::Keyed`]
/// endpoints, up front — mirrors the pre-phase-5 `refuse_keyed_nodes`'s own
/// eager whole-graph scan: a graph containing even one unaddressed keyed
/// edge fails closed before any propagation/resolution runs, regardless of
/// whether that edge is reached by the current call's dirt.
fn classify_keyed_edges(edges: &[Edge]) -> Result<Vec<KeyedAdmission>, String> {
    edges.iter().map(classify_keyed_edge).collect()
}

/// One dependency edge: `downstream` reads `upstream` under a derived scan
/// clamp of `(before_days, after_days)` whole days, between axes of the
/// stated grains. The clamp applies in days; each hop then aligns outward
/// to the receiving axis's grain (`10-dependency-propagation.md` §7:
/// `align_outward ∘ reflect ∘ align_outward`).
#[derive(Debug, Clone)]
pub struct Edge {
    pub upstream: String,
    pub downstream: String,
    pub before_days: i64,
    pub after_days: i64,
    pub upstream_grain: PartitionGrain,
    pub downstream_grain: PartitionGrain,
    /// The typed component vector for this edge (`incremental_models.md`
    /// §"The graph layer" → "Typed edges"), derived by
    /// [`super::edge_type::type_edge`]. Advisory for a non-keyed edge —
    /// `propagate`/`required_inputs` still route those on interval math
    /// alone, unchanged since phase 3. **Acted on** for an edge touching a
    /// [`PartitionGrain::Keyed`] endpoint (phase 5,
    /// `docs/outcomes/20260809-output-delta-typing/outcome.md`): an
    /// admitted [`super::edge_type::Addressing::Keyed`] component routes
    /// the edge through the keyed dirt-set channel instead of a fail-loud
    /// refusal. Empty on [`Edge::from_clamp`] (today's untyped-edge
    /// behaviour); populate via [`Edge::with_components`].
    pub components: Vec<EdgeComponent>,
}

impl Edge {
    /// Build the edge from a cell's derived [`ScanClamp`] — the same number
    /// that sizes the maintenance SQL sizes the propagation. Day grain on
    /// both axes; override the grain fields for coarser tables. Carries no
    /// typed components — today's untyped-edge behaviour.
    pub fn from_clamp(downstream: &str, clamp: &ScanClamp) -> Self {
        Edge {
            upstream: clamp.source.clone(),
            downstream: downstream.to_string(),
            before_days: clamp_days(clamp.before),
            after_days: clamp_days(clamp.after),
            upstream_grain: PartitionGrain::Day,
            downstream_grain: PartitionGrain::Day,
            components: Vec::new(),
        }
    }

    /// Attach a typed component vector (`super::edge_type::type_edge`) to an
    /// already-built edge.
    pub fn with_components(mut self, components: Vec<EdgeComponent>) -> Self {
        self.components = components;
        self
    }

    /// The downstream partitions an upstream delta of `[a, b)` dirties:
    /// the scan reflection `[a − after, b + before)`, aligned outward to
    /// the downstream axis's grain.
    fn reflect(&self, delta: &DayInterval) -> DayInterval {
        self.downstream_grain.align_outward(&DayInterval {
            start: delta.start - self.after_days,
            end: delta.end + self.before_days,
        })
    }

    /// The upstream partitions a downstream requirement of `[s, e)` needs:
    /// the scan clamp directly `[s − before, e + after)`, aligned outward
    /// to the upstream axis's grain.
    fn require(&self, requirement: &DayInterval) -> DayInterval {
        self.upstream_grain.align_outward(&DayInterval {
            start: requirement.start - self.before_days,
            end: requirement.end + self.after_days,
        })
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
    /// `(model, upstream)` → the keyed dirt-set records that edge produced
    /// (`incremental_models.md` §"The graph layer" → "Keyed dirt-sets and
    /// the narrowed refusal") — the keyed-channel counterpart to `per_edge`,
    /// additive alongside it, never replacing it.
    pub per_edge_keys: BTreeMap<(String, String), Vec<KeyedDirt>>,
    /// `model` → merged keyed dirt-set records across all admitted inbound
    /// keyed edges — the keyed-channel counterpart to `dirty`.
    pub keyed_dirty: BTreeMap<String, Vec<KeyedDirt>>,
}

/// The grain of `node`'s partition axis, read off any edge that touches it
/// (edges carry both endpoint grains; a node absent from all edges defaults
/// to day grain).
fn node_grain(edges: &[Edge], node: &str) -> PartitionGrain {
    for e in edges {
        if e.upstream == node {
            return e.upstream_grain;
        }
        if e.downstream == node {
            return e.downstream_grain;
        }
    }
    PartitionGrain::Day
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
    let keyed_admission = classify_keyed_edges(edges)?;
    let order = topo_order(edges, source_deltas.keys().map(|s| s.as_str()))?;

    let mut result = Propagation::default();
    // Seed: the source deltas are the sources' own "dirty" intervals,
    // aligned outward to each source's grain (a coarse-grained source's
    // delta is whole partitions by definition). A keyed-grain source's
    // interval is never interpreted spatially — it only marks the node
    // non-empty so the loop below visits its outbound (keyed-admitted)
    // edges.
    for (source, intervals) in source_deltas {
        let grain = node_grain(edges, source);
        result.dirty.insert(
            source.clone(),
            normalize(intervals.iter().map(|iv| grain.align_outward(iv)).collect()),
        );
    }

    for node in order {
        let node_dirty = result.dirty.get(node).cloned().unwrap_or_default();
        if node_dirty.is_empty() {
            continue;
        }
        for (idx, e) in edges.iter().enumerate() {
            if e.upstream != node {
                continue;
            }
            match &keyed_admission[idx] {
                KeyedAdmission::Admitted(keys) => {
                    let kd = KeyedDirt {
                        keys: keys.clone(),
                        from: e.upstream.clone(),
                    };
                    result
                        .per_edge_keys
                        .entry((e.downstream.clone(), e.upstream.clone()))
                        .or_default()
                        .push(kd.clone());
                    result
                        .keyed_dirty
                        .entry(e.downstream.clone())
                        .or_default()
                        .push(kd);
                    // A downstream that is itself keyed-grain stays on the
                    // keyed channel alone; a clocked (or unclocked) consumer
                    // of a keyed node gets whole-table interval dirt so it
                    // still runs (`incremental_models.md` §"The graph
                    // layer": widen-never-narrow — never nothing).
                    if e.downstream_grain != PartitionGrain::Keyed {
                        let per_edge = result
                            .per_edge
                            .entry((e.downstream.clone(), e.upstream.clone()))
                            .or_default();
                        per_edge.push(DayInterval::WHOLE);
                        *per_edge = normalize(per_edge.clone());
                        let model_dirty = result.dirty.entry(e.downstream.clone()).or_default();
                        model_dirty.push(DayInterval::WHOLE);
                        *model_dirty = normalize(model_dirty.clone());
                    }
                }
                KeyedAdmission::NotKeyed => {
                    let reflected: Vec<DayInterval> =
                        node_dirty.iter().map(|iv| e.reflect(iv)).collect();
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
    let keyed_admission = classify_keyed_edges(edges)?;
    let order = topo_order(edges, [target])?;

    let mut required: BTreeMap<String, Vec<DayInterval>> = BTreeMap::new();
    // A request for part of a coarse partition is a request for the whole
    // partition — align the period outward to the target's grain.
    required.insert(
        target.to_string(),
        vec![node_grain(edges, target).align_outward(&period)],
    );

    // Reverse topological order: a node's requirement is final (every
    // consumer already contributed) before it is pushed up its inbound
    // edges.
    for node in order.iter().rev() {
        let Some(node_required) = required.get(*node).cloned() else {
            continue;
        };
        for (idx, e) in edges.iter().enumerate() {
            if e.downstream != *node {
                continue;
            }
            // An admitted keyed edge has no partition axis to clamp
            // against — the upstream (keyed) ancestor requires the whole
            // table, never clamp arithmetic on a nonexistent axis.
            let widened: Vec<DayInterval> = match &keyed_admission[idx] {
                KeyedAdmission::Admitted(_) => vec![DayInterval::WHOLE],
                KeyedAdmission::NotKeyed => node_required.iter().map(|iv| e.require(iv)).collect(),
            };
            let upstream_required = required.entry(e.upstream.clone()).or_default();
            upstream_required.extend(widened);
            *upstream_required = normalize(upstream_required.clone());
        }
    }

    // Build order: the required models (nodes with an inbound edge) in
    // forward topological order — ancestors of the target precede it, so
    // the target is last. The target itself is ALWAYS included even when it
    // has no inbound edge (e.g. a `refresh: full` model, or any target with
    // no upstream edges registered in the graph) — "no upstream deps" only
    // means an empty required-slices set for its ancestors, never an empty
    // build for the target the caller actually asked for.
    let has_inbound: BTreeSet<&str> = edges.iter().map(|e| e.downstream.as_str()).collect();
    let build_order: Vec<String> = order
        .iter()
        .filter(|n| required.contains_key(**n) && (has_inbound.contains(**n) || **n == target))
        .map(|n| n.to_string())
        .collect();

    Ok(RequiredSlices {
        required,
        build_order,
    })
}

// ---------------------------------------------------------------------------
// Phase B2 (`docs/plans/20260715-composed-axes-conditional-maintenance.md`):
// `locality_margin_days`'s route-aware mapping, and the composed-node
// forward/backward projection it feeds (`propagate`/`required_inputs`
// already generically apply an edge's `(before_days, after_days)` margin —
// this module's own composition math — so these tests exercise that SAME
// machinery with margins actually derived from a [`locality::LocalitySlice`]
// rather than a hand-typed clamp, pinning that routes 1–2 project exactly
// and route 3 widens, and that widening never narrows the exact result.
// ---------------------------------------------------------------------------
#[cfg(test)]
mod locality_margin_tests {
    use super::*;

    fn edge(before_days: i64, after_days: i64) -> Edge {
        Edge {
            upstream: "source".to_string(),
            downstream: "composed".to_string(),
            before_days,
            after_days,
            upstream_grain: PartitionGrain::Day,
            downstream_grain: PartitionGrain::Day,
            components: Vec::new(),
        }
    }

    #[test]
    fn delta_values_route_2_projects_zero_margin() {
        let slice = locality::LocalitySlice::DeltaValues {
            partition_column: "d".to_string(),
        };
        assert_eq!(locality_margin_days(&slice), (0, 0));
    }

    #[test]
    fn window_route_1_or_static_route_3_projects_the_derived_margin() {
        let slice = locality::LocalitySlice::Window {
            partition_column: "d".to_string(),
            margin_before: Seconds::days(2),
            margin_after: Seconds::days(1),
            recurrence_bounded: true,
        };
        assert_eq!(locality_margin_days(&slice), (2, 1));
    }

    #[test]
    fn window_route_projects_ceil_of_a_partial_day_margin() {
        // A 36h backward margin must widen to 2 whole days — a partial-day
        // margin widens to the whole partition it touches (this module's
        // own `clamp_days` rule, reused here rather than reimplemented).
        let slice = locality::LocalitySlice::Window {
            partition_column: "d".to_string(),
            margin_before: Seconds(36 * 3600),
            margin_after: Seconds::ZERO,
            recurrence_bounded: true,
        };
        assert_eq!(locality_margin_days(&slice), (2, 0));
    }

    #[test]
    fn recurrence_bounded_route_3_declared_projects_the_folded_margin() {
        // `margin_before` already has `r` folded in per the variant's own
        // doc comment — no separate `r` arithmetic in `locality_margin_days`.
        let slice = locality::LocalitySlice::RecurrenceBounded {
            partition_column: "d".to_string(),
            margin_before: Seconds::days(7),
            margin_after: Seconds::ZERO,
            r: Seconds::days(5),
        };
        assert_eq!(locality_margin_days(&slice), (7, 0));
    }

    /// An upstream delta `[a, b)` through a zero-margin (route 1/2, no
    /// lookback) composed edge dirties downstream exactly the same interval
    /// (mod grain alignment, which is a no-op at Day grain).
    #[test]
    fn zero_margin_edge_projects_the_exact_delta_forward() {
        let e = edge(0, 0);
        let delta = DayInterval::new(10, 12);
        assert_eq!(e.reflect(&delta), delta);
    }

    /// The same delta through a nonzero-margin (route 3) edge dirties the
    /// widened interval — strictly larger than the exact delta.
    #[test]
    fn nonzero_margin_edge_widens_the_delta_forward() {
        let e = edge(3, 1);
        let delta = DayInterval::new(10, 12);
        let widened = e.reflect(&delta);
        assert_eq!(widened, DayInterval::new(9, 15));
        assert!(widened.start <= delta.start && delta.end <= widened.end);
        assert_ne!(widened, delta, "a nonzero margin must actually widen");
    }

    /// Property (table-driven over a few margin values, not a single
    /// example): the projected forward interval always **contains** the
    /// exact delta — widen-never-narrow holds regardless of which margin a
    /// route derives.
    #[test]
    fn forward_projection_always_contains_the_exact_delta() {
        let delta = DayInterval::new(100, 103);
        for (before, after) in [(0, 0), (1, 0), (0, 1), (2, 3), (7, 5)] {
            let e = edge(before, after);
            let projected = e.reflect(&delta);
            assert!(
                projected.start <= delta.start && delta.end <= projected.end,
                "margin ({before}, {after}): projected {projected:?} must contain exact {delta:?}"
            );
        }
    }

    /// Backward resolution (`required_inputs`) is route-aware the same way:
    /// a zero-margin composed edge resolves a downstream period to the exact
    /// same upstream slice; a nonzero-margin edge resolves to the widened
    /// slice.
    #[test]
    fn required_inputs_resolves_route_aware_through_a_composed_edge() {
        let period = DayInterval::new(20, 21);

        let exact_edges = vec![edge(0, 0)];
        let exact = required_inputs(&exact_edges, "composed", period).expect("resolve");
        assert_eq!(exact.required.get("source"), Some(&vec![period]));

        let widened_edges = vec![edge(3, 1)];
        let widened = required_inputs(&widened_edges, "composed", period).expect("resolve");
        assert_eq!(
            widened.required.get("source"),
            Some(&vec![DayInterval::new(17, 22)])
        );
    }
}

// ---------------------------------------------------------------------------
// Phase D3 (`docs/plans/20260715-composed-axes-conditional-maintenance.md`):
// `project_observed_delta` — key→partition projection of a composed
// model's recorded observed delta via its established locality route.
// ---------------------------------------------------------------------------
#[cfg(test)]
mod project_observed_delta_tests {
    use super::*;

    fn window_slice() -> locality::LocalitySlice {
        locality::LocalitySlice::Window {
            partition_column: "d".to_string(),
            margin_before: Seconds::days(2),
            margin_after: Seconds::days(1),
            recurrence_bounded: false,
        }
    }

    /// Route 3's **statically-derived** sub-route: a `Window` slice whose
    /// margin folds in a proven key-recurrence bound `r` (not just a
    /// read-side lateness margin). Distinguished from `window_slice()`
    /// (route 1) only by `recurrence_bounded: true` — the two are
    /// otherwise the same struct shape (`locality::LocalitySlice::Window`'s
    /// own doc comment).
    fn static_recurrence_window_slice() -> locality::LocalitySlice {
        locality::LocalitySlice::Window {
            partition_column: "d".to_string(),
            margin_before: Seconds::days(3),
            margin_after: Seconds::ZERO,
            recurrence_bounded: true,
        }
    }

    fn delta_values_slice() -> locality::LocalitySlice {
        locality::LocalitySlice::DeltaValues {
            partition_column: "d".to_string(),
        }
    }

    fn recurrence_bounded_slice() -> locality::LocalitySlice {
        locality::LocalitySlice::RecurrenceBounded {
            partition_column: "d".to_string(),
            margin_before: Seconds::days(5),
            margin_after: Seconds::ZERO,
            r: Seconds::days(4),
        }
    }

    #[test]
    fn route_1_window_projects_exactly_the_observed_partitions() {
        let partitions = vec!["2026-01-10".to_string(), "2026-01-12".to_string()];
        let projected = project_observed_delta(&window_slice(), &partitions);
        assert_eq!(
            projected,
            vec![
                DayInterval::new(day_ordinal(2026, 1, 10), day_ordinal(2026, 1, 11)),
                DayInterval::new(day_ordinal(2026, 1, 12), day_ordinal(2026, 1, 13)),
            ],
            "route 1 (Window, recurrence_bounded: false) must project each observed partition \
             to its own exact day, ignoring the slice's own read-side lateness margin"
        );
    }

    #[test]
    fn route_3_static_window_widens_backward_by_the_folded_margin() {
        // Regression: a `Window` slice with `recurrence_bounded: true` (the
        // statically-derived route-3 sub-route) must widen by its own
        // margin the same way `RecurrenceBounded` does — the earlier bug
        // treated every `Window` as route 1 and silently applied zero
        // widening here, which is a silent under-propagation of a real,
        // reachable admission path (`incremental_models.md` §"Row
        // movement": both route-3 sub-routes widen; only routes 1–2
        // project exactly).
        let partitions = vec!["2026-01-10".to_string()];
        let projected = project_observed_delta(&static_recurrence_window_slice(), &partitions);
        let day = day_ordinal(2026, 1, 10);
        assert_eq!(
            projected,
            vec![DayInterval::new(day - 3, day + 1)],
            "a statically-derived route-3 Window (recurrence_bounded: true) must widen backward \
             by its folded margin, not project the observed partition exactly"
        );
    }

    #[test]
    fn route_2_delta_values_projects_exactly_the_observed_partitions() {
        let partitions = vec!["2026-01-10".to_string()];
        let projected = project_observed_delta(&delta_values_slice(), &partitions);
        assert_eq!(
            projected,
            vec![DayInterval::new(
                day_ordinal(2026, 1, 10),
                day_ordinal(2026, 1, 11)
            )]
        );
    }

    #[test]
    fn route_3_recurrence_bounded_widens_backward_by_r_and_margins() {
        let partitions = vec!["2026-01-10".to_string()];
        let projected = project_observed_delta(&recurrence_bounded_slice(), &partitions);
        let day = day_ordinal(2026, 1, 10);
        assert_eq!(
            projected,
            vec![DayInterval::new(day - 5, day + 1)],
            "route 3's declared r (already folded into margin_before) must widen the observed \
             partition backward, not just project it exactly"
        );
    }

    /// Route 3's forward half: a `RecurrenceBounded` slice with a nonzero
    /// `margin_after` must widen the projected interval forward too, not
    /// just backward — pins the `day + 1 + after` half of the projection
    /// formula (only the backward `day - before` half was previously
    /// exercised by a nonzero fixture).
    #[test]
    fn route_3_recurrence_bounded_widens_forward_by_margin_after() {
        let slice = locality::LocalitySlice::RecurrenceBounded {
            partition_column: "d".to_string(),
            margin_before: Seconds::days(5),
            margin_after: Seconds::days(2),
            r: Seconds::days(4),
        };
        let partitions = vec!["2026-01-10".to_string()];
        let projected = project_observed_delta(&slice, &partitions);
        let day = day_ordinal(2026, 1, 10);
        assert_eq!(
            projected,
            vec![DayInterval::new(day - 5, day + 3)],
            "a nonzero margin_after must widen the projected interval forward beyond day + 1"
        );
    }

    #[test]
    fn empty_partitions_project_to_nothing() {
        assert_eq!(project_observed_delta(&window_slice(), &[]), Vec::new());
        assert_eq!(
            project_observed_delta(&recurrence_bounded_slice(), &[]),
            Vec::new()
        );
    }

    #[test]
    fn malformed_partition_strings_are_skipped_not_panicked() {
        let partitions = vec!["not-a-date".to_string(), "2026-01-10".to_string()];
        let projected = project_observed_delta(&window_slice(), &partitions);
        assert_eq!(
            projected,
            vec![DayInterval::new(
                day_ordinal(2026, 1, 10),
                day_ordinal(2026, 1, 11)
            )]
        );
    }

    /// Property: for every route, the projection never narrows below the
    /// observed partitions themselves — every observed day's own
    /// `[d, d+1)` slice is fully contained in the projected result.
    #[test]
    fn projection_never_narrows_below_the_observed_partitions() {
        let partitions = vec![
            "2026-02-01".to_string(),
            "2026-02-15".to_string(),
            "2026-03-01".to_string(),
        ];
        for slice in [
            window_slice(),
            static_recurrence_window_slice(),
            delta_values_slice(),
            recurrence_bounded_slice(),
        ] {
            let projected = project_observed_delta(&slice, &partitions);
            for p in &partitions {
                let day = parse_iso_day(p).expect("valid test date");
                let observed = DayInterval::new(day, day + 1);
                assert!(
                    projected
                        .iter()
                        .any(|iv| iv.start <= observed.start && observed.end <= iv.end),
                    "observed partition {p} (day {day}) must be covered by the projection \
                     {projected:?} under slice {slice:?}"
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Phase 3 (`docs/outcomes/20260809-output-delta-typing/outcome.md`): typed
// `Edge::components` are advisory this phase — `propagate`/`required_inputs`
// read only `before_days`/`after_days`/the grain fields, unchanged by
// whether a component vector is attached. These tests pin that an edge's
// interval-math behaviour is byte-identical with or without components, for
// both a window-addressed component and a degraded `WholeModel` one.
// ---------------------------------------------------------------------------
#[cfg(test)]
mod typed_edge_advisory_tests {
    use super::*;
    use crate::analysis::output_delta::OutputDelta;
    use crate::maintenance::edge_type::{Addressing, EdgeComponent};

    fn window_component() -> EdgeComponent {
        EdgeComponent {
            group: "{amount}".to_string(),
            shape: OutputDelta::AppendOnlyWindow {
                axis: "event_date".to_string(),
            },
            addressing: Addressing::Window {
                axis: "event_date".to_string(),
            },
            columns: vec!["amount".to_string()],
        }
    }

    fn whole_model_component() -> EdgeComponent {
        EdgeComponent {
            group: "{weight}".to_string(),
            shape: OutputDelta::General {
                reason: "'dims' is a mutable snapshot".to_string(),
            },
            addressing: Addressing::WholeModel {
                degraded_by: "'dims' is a mutable snapshot".to_string(),
            },
            columns: vec!["weight".to_string()],
        }
    }

    fn base_edge(before_days: i64, after_days: i64) -> Edge {
        Edge {
            upstream: "source".to_string(),
            downstream: "downstream".to_string(),
            before_days,
            after_days,
            upstream_grain: PartitionGrain::Day,
            downstream_grain: PartitionGrain::Day,
            components: Vec::new(),
        }
    }

    fn source_deltas(source: &str, iv: DayInterval) -> BTreeMap<String, Vec<DayInterval>> {
        let mut m = BTreeMap::new();
        m.insert(source.to_string(), vec![iv]);
        m
    }

    #[test]
    fn typed_edge_forward_matches_untyped_for_window_components() {
        let untyped = base_edge(1, 2);
        let typed = base_edge(1, 2).with_components(vec![window_component()]);
        let delta = source_deltas("source", DayInterval::new(10, 12));

        let untyped_result = propagate(&[untyped], &delta).expect("untyped propagates");
        let typed_result = propagate(&[typed], &delta).expect("typed propagates");

        assert_eq!(
            untyped_result.dirty, typed_result.dirty,
            "attaching window components must not change forward propagation"
        );
        assert_eq!(untyped_result.per_edge, typed_result.per_edge);
    }

    #[test]
    fn adjoint_holds_with_typed_components() {
        // A small fixed DAG: source -> mid -> target, edges carrying window
        // components throughout.
        let e1 = Edge {
            upstream: "source".to_string(),
            downstream: "mid".to_string(),
            before_days: 1,
            after_days: 0,
            upstream_grain: PartitionGrain::Day,
            downstream_grain: PartitionGrain::Day,
            components: vec![window_component()],
        };
        let e2 = Edge {
            upstream: "mid".to_string(),
            downstream: "target".to_string(),
            before_days: 0,
            after_days: 1,
            upstream_grain: PartitionGrain::Day,
            downstream_grain: PartitionGrain::Day,
            components: vec![window_component()],
        };
        let edges = vec![e1, e2];

        let period = DayInterval::new(20, 21);
        let required = required_inputs(&edges, "target", period).expect("resolve");
        let source_required = required.required.get("source").cloned().unwrap_or_default();

        let deltas = source_deltas("source", source_required[0]);
        let forward = propagate(&edges, &deltas).expect("propagate");
        let target_dirty = forward.dirty.get("target").cloned().unwrap_or_default();

        assert!(
            target_dirty
                .iter()
                .any(|iv| iv.start <= period.start && period.end <= iv.end),
            "forward(backward(P)) must contain P: target dirty {target_dirty:?} must cover \
             {period:?}"
        );
    }

    #[test]
    fn whole_model_component_does_not_narrow_dirt() {
        let untyped = base_edge(1, 2);
        let typed = base_edge(1, 2).with_components(vec![whole_model_component()]);
        let delta = source_deltas("source", DayInterval::new(10, 12));

        let untyped_result = propagate(&[untyped], &delta).expect("untyped propagates");
        let typed_result = propagate(&[typed], &delta).expect("typed propagates");

        let untyped_dirty = untyped_result
            .dirty
            .get("downstream")
            .cloned()
            .unwrap_or_default();
        let typed_dirty = typed_result
            .dirty
            .get("downstream")
            .cloned()
            .unwrap_or_default();

        assert_eq!(
            untyped_dirty, typed_dirty,
            "a WholeModel-degraded component must not narrow interval dirt below the untyped \
             edge's own result this phase (components are advisory, not yet acted on)"
        );
    }
}
