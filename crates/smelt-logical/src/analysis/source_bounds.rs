//! Per-source bound derivation for incremental models.
//!
//! Walks the (expanded) model SQL and derives, for each `smelt.<path>` source
//! reference, how far outside the run window the source must be read in order
//! to produce correct output.  Two standard SQL forms are recognised:
//!
//! - **Form A** — window-frame `RANGE BETWEEN INTERVAL '…' PRECEDING/FOLLOWING`
//! - **Form B** — explicit WHERE/JOIN time filters with literal `INTERVAL` offsets
//!
//! The result is a `HashMap<String, BoundResult>` keyed by source name (the
//! `smelt.<path>` string without the `smelt.` prefix, matching the names in
//! `ModelInfo.refs`).

use serde::Serialize;
use std::collections::HashMap;

use crate::analysis::monotonicity::find_leaf_column_ref;
use crate::analysis::monotonicity::EventTimeTrace;
use crate::analysis::monotonicity::NotTraceableKind;

/// Duration expressed in whole seconds (always ≥ 0).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Default)]
pub struct Seconds(pub u64);

impl Seconds {
    pub const ZERO: Seconds = Seconds(0);

    pub fn minutes(n: u64) -> Seconds {
        Seconds(n * 60)
    }

    pub fn hours(n: u64) -> Seconds {
        Seconds(n * 3600)
    }

    pub fn days(n: u64) -> Seconds {
        Seconds(n * 86400)
    }

    pub fn weeks(n: u64) -> Seconds {
        Seconds(n * 7 * 86400)
    }

    /// Serialize as ISO-8601 duration string (e.g. "PT30M", "P1D", "PT0S").
    pub fn to_iso8601(&self) -> String {
        let s = self.0;
        if s == 0 {
            return "PT0S".to_string();
        }
        let days = s / 86400;
        let rem = s % 86400;
        let hours = rem / 3600;
        let rem = rem % 3600;
        let minutes = rem / 60;
        let seconds = rem % 60;

        let mut out = String::from("P");
        if days > 0 {
            out.push_str(&format!("{}D", days));
        }
        if hours > 0 || minutes > 0 || seconds > 0 {
            out.push('T');
            if hours > 0 {
                out.push_str(&format!("{}H", hours));
            }
            if minutes > 0 {
                out.push_str(&format!("{}M", minutes));
            }
            if seconds > 0 {
                out.push_str(&format!("{}S", seconds));
            }
        }
        out
    }
}

/// The single, unified representation of a parsed `INTERVAL '<value>'`
/// literal, shared by every interval-literal call site in this crate
/// (`source_bounds` Form A/B extraction, `monotonicity`'s constant-shift
/// classifier, and `temporal`'s day-granular scan). Seconds/minutes/hours/
/// days/weeks are uniform durations and fold to `Offset::Seconds`; month and
/// year are *not* uniform durations (a month is 28-31 days; a year is
/// 365-366) so they fold to `Offset::Symbolic` rather than an approximate
/// (and previously divergent — 30d here, 30d there, but computed twice)
/// day count. See `docs/specs/model_properties.md` "Unified bound / reach
/// derivation".
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum Offset {
    Seconds(Seconds),
    /// Non-uniform unit (month/year) — not folded to seconds.
    Symbolic(String),
    /// A constant integer shift over a monotone, non-temporal partition key
    /// (sequence id / offset / watermark) — e.g. the `+ 5` in `batch_id + 5`.
    /// Sign is preserved (`batch_id - 5` folds to `Integer(-5)`); unlike
    /// `Seconds` (an unsigned duration), this variant is signed because an
    /// integer key has no inherent "always forward" direction the way a
    /// temporal interval literal does. See `docs/specs/batched_models.md`
    /// §Surface "`partition_column` must be monotone".
    Integer(i64),
}

/// Parse an interval-literal value string (e.g. `"1 day"`, `"30 minutes"`,
/// `"1 month"`, or a bare `"5"` with no unit) into the unified [`Offset`]
/// representation. Returns `None` when `value` has no parseable leading
/// integer.
pub(crate) fn parse_interval(value: &str) -> Option<Offset> {
    let trimmed = value.trim();
    let upper = trimmed.to_uppercase();
    let parts: Vec<&str> = upper.split_whitespace().collect();
    if parts.is_empty() {
        return None;
    }

    let n: u64 = parts[0].parse().ok()?;
    let Some(unit) = parts.get(1).copied() else {
        // No unit — assume the bare number is already seconds.
        return Some(Offset::Seconds(Seconds(n)));
    };

    if unit.starts_with("SECOND") {
        Some(Offset::Seconds(Seconds(n)))
    } else if unit.starts_with("MINUTE") {
        Some(Offset::Seconds(Seconds::minutes(n)))
    } else if unit.starts_with("HOUR") {
        Some(Offset::Seconds(Seconds::hours(n)))
    } else if unit.starts_with("DAY") {
        Some(Offset::Seconds(Seconds::days(n)))
    } else if unit.starts_with("WEEK") {
        Some(Offset::Seconds(Seconds::weeks(n)))
    } else if unit.starts_with("MONTH") || unit.starts_with("YEAR") {
        Some(Offset::Symbolic(trimmed.to_string()))
    } else {
        Some(Offset::Seconds(Seconds(n)))
    }
}

/// The derived bound for one source reference.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BoundResult {
    /// The source must be read `before` seconds before run_start and
    /// `after` seconds after run_end.
    Bounded {
        source_partition_col: String,
        #[serde(serialize_with = "serialize_seconds")]
        before: Seconds,
        #[serde(serialize_with = "serialize_seconds")]
        after: Seconds,
    },
    /// The source requires reading unbounded history (e.g. cumulative aggregation).
    Unbounded,
    /// The analyzer cannot derive a bound from the SQL patterns present.
    NotDerivable,
}

fn serialize_seconds<S>(s: &Seconds, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(&s.to_iso8601())
}

/// Where the filter derived for a source should be written, per the
/// pushdown-depth walk (research `20260703-model-updates.md` §3.3).
///
/// The walk that produces a source's [`BoundResult`] also determines how deep
/// `event_time`'s filter can be pushed: a source with **no** lookback margin
/// (`Bounded` with `before == after == 0`, the "transparent slice") can be
/// filtered exactly once, at the source scan — the same filter that clamps
/// the output also prunes the read, so no outer wrap is needed. A source with
/// a genuine lookback/lookahead margin needs both layers: a widened scan at
/// the source *and* an exact output clamp above it, because the scan window
/// is legitimately wider than the output window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InjectionPoint {
    /// Push the filter all the way to the source scan; the outer clamp is
    /// redundant and is skipped.
    Source,
    /// Keep the outer output clamp in addition to the per-source scan
    /// filter — a lookback margin makes the two windows genuinely distinct.
    OuterClamp,
}

impl BoundResult {
    /// Classify the injection point implied by this bound (see
    /// [`InjectionPoint`]). A zero-margin `Bounded` source pushes all the way
    /// to the scan; `Unbounded` and `NotDerivable` conservatively keep the
    /// outer clamp (in practice `NotDerivable` is refused before this is
    /// consulted, and `Unbounded` routes to per-partition execution rather
    /// than batched pushdown).
    pub fn injection_point(&self) -> InjectionPoint {
        match self {
            BoundResult::Bounded { before, after, .. }
                if *before == Seconds::ZERO && *after == Seconds::ZERO =>
            {
                InjectionPoint::Source
            }
            _ => InjectionPoint::OuterClamp,
        }
    }

    /// Merge two bound results for the *same* source (union semantics):
    /// before = max(before_i), after = max(after_i).
    /// Any `Unbounded` forces `Unbounded`; any `NotDerivable` forces `NotDerivable`.
    pub fn merge(self, other: BoundResult) -> BoundResult {
        match (self, other) {
            (BoundResult::NotDerivable, _) | (_, BoundResult::NotDerivable) => {
                BoundResult::NotDerivable
            }
            (BoundResult::Unbounded, _) | (_, BoundResult::Unbounded) => BoundResult::Unbounded,
            (
                BoundResult::Bounded {
                    source_partition_col,
                    before: b1,
                    after: a1,
                },
                BoundResult::Bounded {
                    before: b2,
                    after: a2,
                    ..
                },
            ) => BoundResult::Bounded {
                source_partition_col,
                before: b1.max(b2),
                after: a1.max(a2),
            },
        }
    }
}

impl BoundResult {
    /// Series composition (`model_properties.md` §"The composition walk"):
    /// the margins of two sequentially-nested reaches **add** — a stacked
    /// window frame or a chained join band each widens how far outside the
    /// run window the source beneath it must be read. `NotDerivable` and
    /// `Unbounded` are absorbing, in that order of precedence (matching
    /// [`BoundResult::merge`], the parallel composition).
    pub fn then(self, other: BoundResult) -> BoundResult {
        match (self, other) {
            (BoundResult::NotDerivable, _) | (_, BoundResult::NotDerivable) => {
                BoundResult::NotDerivable
            }
            (BoundResult::Unbounded, _) | (_, BoundResult::Unbounded) => BoundResult::Unbounded,
            (
                BoundResult::Bounded {
                    source_partition_col,
                    before: b1,
                    after: a1,
                },
                BoundResult::Bounded {
                    before: b2,
                    after: a2,
                    ..
                },
            ) => BoundResult::Bounded {
                source_partition_col,
                before: Seconds(b1.0.saturating_add(b2.0)),
                after: Seconds(a1.0.saturating_add(a2.0)),
            },
        }
    }
}

/// Context for bound derivation: maps source name → its declared partition column.
/// Sources not present in the map are treated as lookups (no bound derived).
#[derive(Debug, Default)]
pub struct BoundContext {
    /// Maps source name (as it appears in `smelt.<path>` refs, e.g. "silver.events_parsed")
    /// to its `timeseries.partition_column`.
    pub source_partition_cols: HashMap<String, String>,
}

impl BoundContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_source(mut self, source: &str, partition_col: &str) -> Self {
        self.source_partition_cols
            .insert(source.to_string(), partition_col.to_string());
        self
    }

    pub fn add_source(&mut self, source: &str, partition_col: &str) {
        self.source_partition_cols
            .insert(source.to_string(), partition_col.to_string());
    }
}

/// Derive per-source bounds from the full model SQL (after function expansion).
///
/// `sql` is the expanded SQL text of the model (including any inlined function bodies).
/// `ctx` maps source names to their partition columns (only timeseries sources; lookups absent).
///
/// Returns a map from source name → bound. Only entries for timeseries sources in
/// `ctx` are produced; lookup sources (not in ctx) are absent from the result.
pub fn derive_model_bounds(sql: &str, ctx: &BoundContext) -> HashMap<String, BoundResult> {
    // The composition walk (`model_properties.md` §"The composition walk"):
    // each query-tree node derives reach from its OWN region only, children
    // compose in parallel (max, `BoundResult::merge`) across set-op arms and
    // join inputs, and a node's own reach composes in series (add,
    // `BoundResult::then`) onto every source beneath it — a stacked window
    // frame or a chained join band widens the sources it reads over, where a
    // whole-text scan would under-derive them by max-merging.
    // A tree with any `Unsupported` normalization (e.g. the redundantly-
    // parenthesized derived table function expansion emits, which the parser
    // does not nest) keeps the legacy whole-text derivation for every source
    // — the series composition applies only where the walk models the whole
    // tree, and never degrades coverage below the flat scan.
    let mut result: HashMap<String, BoundResult> =
        match crate::analysis::walk::QueryTree::from_sql(sql) {
            Some(tree) if !tree.root.has_unsupported() => {
                crate::analysis::walk::walk(&tree, &ReachTransfer { ctx })
            }
            _ => HashMap::new(),
        };

    // Floor: merge every context source's walk verdict with its whole-text
    // flat-scan derivation. The walk models reach only along a source's FROM-
    // leaf paths, so a source referenced *both* as a plain FROM leaf and inside
    // an expression-position subquery (which is not a walk node) would carry
    // only its leaf-path reach — the subquery region's band would be dropped,
    // and the injected scan filter would under-cover that reference site. The
    // flat scan reads every textual occurrence, so merging it in restores the
    // dropped band. `merge` takes max on margins and gives rejection precedence
    // (`Unbounded`/`NotDerivable` absorbing), so the walk may only WIDEN the
    // flat scan, never narrow it: series-added margins (SC-4: max(10d walk, 7d
    // flat) = 10d) and global symbolic/bare-LAG absorption both survive, and no
    // source ever falls below the flat-scan floor. This also covers sources
    // with no walk leaf at all (referenced only inside an expression subquery),
    // subsuming the old `!contains_key` fallback.
    for (source_name, partition_col) in &ctx.source_partition_cols {
        if let Some(flat) = derive_bound_for_source(sql, partition_col) {
            result
                .entry(source_name.clone())
                .and_modify(|existing| {
                    *existing =
                        std::mem::replace(existing, BoundResult::Unbounded).merge(flat.clone());
                })
                .or_insert(flat);
        }
    }

    result
}

/// One region's contribution to the series composition: the reach the
/// region's own frames/bands force, or an absorbing reject.
enum RegionReach {
    /// Additive margins (zero when the region has no temporal construct).
    Reach(Seconds, Seconds),
    Unbounded,
    NotDerivable,
}

/// Derive one region's own reach — the same Form A / Form B extraction and
/// fail-closed guards as the whole-text derivation, but over a single walk
/// node's region text, so nesting composes in the walk instead of being
/// flattened away.
fn derive_region_reach(region_upper: &str, partition_col: &str) -> RegionReach {
    if has_unbounded_forward_reach(region_upper) {
        return RegionReach::Unbounded;
    }
    if has_symbolic_interval_in_bound_position(region_upper) {
        return RegionReach::NotDerivable;
    }

    let partition_col_upper = partition_col.to_uppercase();
    let mut accumulated: Option<(Seconds, Seconds)> = None;
    let contributions = extract_form_a_bounds(region_upper)
        .into_iter()
        .chain(extract_form_b_bounds(region_upper, &partition_col_upper));
    for (before, after) in contributions {
        // Within one region, contributions are parallel (two window frames
        // over the same scope input read the same rows): max, not add.
        accumulated = Some(match accumulated {
            None => (before, after),
            Some((b, a)) => (b.max(before), a.max(after)),
        });
    }

    match accumulated {
        Some((before, after)) => RegionReach::Reach(before, after),
        None => {
            if has_bare_lag_lead_over(region_upper) {
                RegionReach::NotDerivable
            } else if has_unbounded_preceding_range(region_upper) {
                RegionReach::Unbounded
            } else {
                RegionReach::Reach(Seconds::ZERO, Seconds::ZERO)
            }
        }
    }
}

/// The bound/reach transfer function over the composition walk. Verdict: a
/// per-source map of [`BoundResult`]s for the sources visible beneath the
/// node. Leaves start at zero margin; each node's own-region reach
/// series-adds onto every source under it; parallel children (set-op arms,
/// join inputs, repeated CTE references) merge with max.
struct ReachTransfer<'a> {
    ctx: &'a BoundContext,
}

impl ReachTransfer<'_> {
    fn merge_child(out: &mut HashMap<String, BoundResult>, child: &HashMap<String, BoundResult>) {
        for (source, bound) in child {
            out.entry(source.clone())
                .and_modify(|existing| {
                    *existing =
                        std::mem::replace(existing, BoundResult::Unbounded).merge(bound.clone());
                })
                .or_insert_with(|| bound.clone());
        }
    }

    /// Fail-closed verdict for a construct the walk cannot see through: every
    /// context source is `NotDerivable` (`model_properties.md` §Constraints —
    /// absence of a proof is a rejection).
    fn all_not_derivable(&self) -> HashMap<String, BoundResult> {
        self.ctx
            .source_partition_cols
            .keys()
            .map(|source| (source.clone(), BoundResult::NotDerivable))
            .collect()
    }
}

impl crate::analysis::walk::Transfer for ReachTransfer<'_> {
    type Verdict = HashMap<String, BoundResult>;

    fn leaf(
        &self,
        leaf: &crate::analysis::walk::LeafInput<'_>,
        _cx: &crate::analysis::walk::NodeCx,
    ) -> Self::Verdict {
        // Context keys vary by caller: the planner keys sources by their
        // segment path (`sources.metrics`), the runtime by the full smelt
        // reference (`smelt.sources.metrics`). The walk leaf carries the
        // segment path; resolve either form, and key the verdict by the
        // CONTEXT's own key so every caller's downstream lookups match.
        let mut map = HashMap::new();
        let entry = self
            .ctx
            .source_partition_cols
            .get_key_value(leaf.name)
            .or_else(|| {
                self.ctx
                    .source_partition_cols
                    .get_key_value(&format!("smelt.{}", leaf.name))
            });
        if let Some((ctx_key, partition_col)) = entry {
            map.insert(
                ctx_key.clone(),
                BoundResult::Bounded {
                    source_partition_col: partition_col.clone(),
                    before: Seconds::ZERO,
                    after: Seconds::ZERO,
                },
            );
        }
        map
    }

    fn operator(
        &self,
        op: &crate::analysis::walk::OpNode<'_>,
        children: &[Self::Verdict],
        _cx: &crate::analysis::walk::NodeCx,
    ) -> Self::Verdict {
        use crate::analysis::walk::OpNode;
        match op {
            OpNode::Unsupported { .. } => self.all_not_derivable(),
            OpNode::SetOp(so) => {
                // Arms are parallel reads; CTE-definition children are
                // skipped — each reference site carries the definition's
                // verdict already.
                let mut out = HashMap::new();
                for child in &children[so.ctes.len()..] {
                    Self::merge_child(&mut out, child);
                }
                out
            }
            OpNode::Select(sn) => {
                // Skip the CTE-definition children — each reference site's
                // child already carries the definition's verdict, so repeated
                // references merge in parallel like any other pair of inputs.
                let input_children = &children[sn.ctes.len()..];
                let mut out = HashMap::new();
                for child in input_children {
                    Self::merge_child(&mut out, child);
                }
                if out.is_empty() {
                    return out;
                }
                let region_upper =
                    crate::analysis::walk::own_region_text(&sn.select).to_uppercase();
                // Region reach depends only on the partition column; compute
                // once per distinct column among the sources beneath.
                let mut per_col: HashMap<&str, RegionReach> = HashMap::new();
                let sources: Vec<String> = out.keys().cloned().collect();
                for source in &sources {
                    let Some(partition_col) = self.ctx.source_partition_cols.get(source) else {
                        continue;
                    };
                    let reach = per_col
                        .entry(partition_col.as_str())
                        .or_insert_with(|| derive_region_reach(&region_upper, partition_col));
                    let addition = match reach {
                        RegionReach::Reach(before, after) => BoundResult::Bounded {
                            source_partition_col: partition_col.clone(),
                            before: *before,
                            after: *after,
                        },
                        // An absorbing construct in this region rejects every
                        // context source, not just the ones beneath this node
                        // — the whole-text derivation's conservatism, kept
                        // deliberately (the region attribution narrows only
                        // the additive arithmetic, never a rejection).
                        RegionReach::Unbounded => {
                            return self
                                .ctx
                                .source_partition_cols
                                .keys()
                                .map(|s| (s.clone(), BoundResult::Unbounded))
                                .collect();
                        }
                        RegionReach::NotDerivable => return self.all_not_derivable(),
                    };
                    // A nonzero own-region reach is a temporal tie across this
                    // node's inputs (a band or frame over the joined rows).
                    // A source constrained relative to a sibling input's
                    // column inherits that sibling's own accumulated slack —
                    // chained join bands add along the path. The sibling
                    // resolution is conservative (max slack over the inputs
                    // that do not carry this source): it may over-widen a
                    // scan, never under-widen it.
                    let is_zero = matches!(
                        addition,
                        BoundResult::Bounded { before, after, .. }
                            if before == Seconds::ZERO && after == Seconds::ZERO
                    );
                    let sibling = if is_zero || input_children.len() < 2 {
                        (Seconds::ZERO, Seconds::ZERO)
                    } else {
                        let mut max_before = Seconds::ZERO;
                        let mut max_after = Seconds::ZERO;
                        for child in input_children {
                            if child.contains_key(source) {
                                continue;
                            }
                            for bound in child.values() {
                                if let BoundResult::Bounded { before, after, .. } = bound {
                                    max_before = max_before.max(*before);
                                    max_after = max_after.max(*after);
                                }
                            }
                        }
                        (max_before, max_after)
                    };
                    let addition = addition.then(BoundResult::Bounded {
                        source_partition_col: partition_col.clone(),
                        before: sibling.0,
                        after: sibling.1,
                    });
                    if let Some(bound) = out.get_mut(source) {
                        *bound = std::mem::replace(bound, BoundResult::Unbounded).then(addition);
                    }
                }
                out
            }
        }
    }
}

/// The single bound-derivation orchestration entry point: derive every
/// source's [`BoundResult`] from `sql`/`ctx` and pair it with its
/// [`InjectionPoint`] classification in the same walk, so the value used to
/// widen a source's scan and the value that decides whether the outer output
/// clamp is still needed can never disagree.
///
/// Both consumers that need "SQL + source list → bound + injection point"
/// call this rather than re-deriving it independently:
/// - the planner's pushdown-eligibility rule
///   (`rules::incremental::derive_model_source_bounds`), which first narrows
///   `ctx` for UNION/JOIN/derived-table constructs and then refuses the
///   model on any `NotDerivable` result;
/// - the runtime's SQL compiler (`smelt_runtime::compile::build_source_bound_map`),
///   which extracts only the `Bounded` results (an already-refused-or-allowed
///   model has nothing left to reject by the time it reaches compilation).
pub fn derive_and_classify_bounds(
    sql: &str,
    ctx: &BoundContext,
) -> HashMap<String, (InjectionPoint, BoundResult)> {
    derive_model_bounds(sql, ctx)
        .into_iter()
        .map(|(source_name, bound)| {
            let injection_point = bound.injection_point();
            (source_name, (injection_point, bound))
        })
        .collect()
}

/// Leaf classifier (`docs/specs/architecture.md` §"Property composition walk
/// rule") — also the whole-text fallback [`derive_model_bounds`] uses for a
/// tree the walk cannot normalize (`QueryNode::has_unsupported`). In that
/// fallback role it treats the whole model SQL as a single region, the same
/// extraction [`derive_region_reach`] runs per walk node; it never widens
/// coverage beyond what the walk-backed path derives, only substitutes for it
/// when the walk cannot see the tree.
///
/// Derive a bound for a single source given its partition column.
///
/// Returns `None` for lookup sources (no timeseries). For timeseries sources,
/// returns `Some(BoundResult)`.
///
/// Checks Form A (RANGE BETWEEN INTERVAL ... PRECEDING/FOLLOWING) and
/// Form B (WHERE col BETWEEN ... - INTERVAL ... AND ... + INTERVAL ...).
fn derive_bound_for_source(sql: &str, partition_col: &str) -> Option<BoundResult> {
    let upper = sql.to_uppercase();
    let partition_col_upper = partition_col.to_uppercase();

    // Fail-closed: a RANGE frame whose forward (FOLLOWING) bound is either
    // explicitly `UNBOUNDED FOLLOWING` or a `FOLLOWING` bound with no
    // parseable `INTERVAL` reads an unbounded amount of future history — the
    // mirror of `UNBOUNDED PRECEDING`. This must be checked *before* the
    // Form A/B accumulation below: a frame like `RANGE BETWEEN INTERVAL '1
    // day' PRECEDING AND UNBOUNDED FOLLOWING` has a genuinely-derivable
    // (nonzero) `before`, so it would otherwise accumulate into a `Bounded`
    // result and never reach the "no bound found at all" fallback where the
    // old `UNBOUNDED PRECEDING`-only check lived.
    if has_unbounded_forward_reach(&upper) {
        return Some(BoundResult::Unbounded);
    }

    // Fail-closed: an `INTERVAL '<n> month|year'` literal folds to
    // `Offset::Symbolic` (see `parse_interval`) — a calendar-relative unit
    // has no fixed length in seconds without a reference date, so it cannot
    // populate a `Bounded{before,after}` value. Per the proof discipline
    // (absence of a proof is a rejection, never an optimistic default —
    // `docs/specs/model_properties.md` §Constraints), refuse the whole
    // source rather than silently treating the symbolic literal as
    // zero-margin or approximating it to a fixed day count.
    if has_symbolic_interval_in_bound_position(&upper) {
        return Some(BoundResult::NotDerivable);
    }

    // Collect all Form A and Form B windows; merge them.
    let mut accumulated: Option<BoundResult> = None;

    // Form A: RANGE BETWEEN INTERVAL '…' PRECEDING AND ...
    // The interval applies to the source backing the window's ORDER BY column.
    // We look for any RANGE BETWEEN INTERVAL pattern in the SQL — if the column
    // participating in the window is the source's partition column (or the model
    // is ordering by it), we derive a bound.
    //
    // Heuristic: any RANGE BETWEEN INTERVAL in the SQL contributes a bound.
    // The interval before "PRECEDING" is the `before`; interval before "FOLLOWING" is `after`.
    let form_a_bounds = extract_form_a_bounds(&upper);
    for (before, after) in form_a_bounds {
        let bound = BoundResult::Bounded {
            source_partition_col: partition_col.to_string(),
            before,
            after,
        };
        accumulated = Some(match accumulated {
            None => bound,
            Some(acc) => acc.merge(bound),
        });
    }

    // Form B: WHERE col BETWEEN expr - INTERVAL '…' AND expr + INTERVAL '…'
    // or WHERE col >= expr - INTERVAL '…' AND col < expr + INTERVAL '…'
    // or WHERE col >= expr - INTERVAL '…' AND col <= expr
    let form_b_bounds = extract_form_b_bounds(&upper, &partition_col_upper);
    for (before, after) in form_b_bounds {
        let bound = BoundResult::Bounded {
            source_partition_col: partition_col.to_string(),
            before,
            after,
        };
        accumulated = Some(match accumulated {
            None => bound,
            Some(acc) => acc.merge(bound),
        });
    }

    // If we found at least one bound, return it. Otherwise the source is NOT derivable
    // only if the SQL has window functions without RANGE — bare LAG/LEAD without RANGE
    // means NotDerivable. A source that is never constrained at all (no intervals)
    // is BoundedZero (fully partition-local, before=0, after=0).
    match accumulated {
        Some(b) => Some(b),
        None => {
            // Check for bare window functions that have no RANGE clause.
            // A bare LAG/LEAD OVER (...) without RANGE BETWEEN is NotDerivable.
            if has_bare_lag_lead_over(&upper) {
                Some(BoundResult::NotDerivable)
            } else if has_unbounded_preceding_range(&upper) {
                Some(BoundResult::Unbounded)
            } else {
                // No temporal dependency — partition-local (Bounded with before=0, after=0).
                Some(BoundResult::Bounded {
                    source_partition_col: partition_col.to_string(),
                    before: Seconds::ZERO,
                    after: Seconds::ZERO,
                })
            }
        }
    }
}

/// Leaf classifier (`docs/specs/architecture.md` §"Property composition walk
/// rule"): invoked by [`derive_region_reach`] over one walk node's own region
/// text (and by [`derive_bound_for_source`] as the whole-text fallback).
///
/// Check whether the SQL has a bare LAG or LEAD with no RANGE BETWEEN clause.
///
/// A "bare" LAG/LEAD is one whose OVER clause lacks an explicit RANGE frame.
/// The heuristic: find OVER clauses containing LAG or LEAD function calls but
/// no RANGE keyword in that OVER block.
fn has_bare_lag_lead_over(upper_sql: &str) -> bool {
    // Look for LAG( or LEAD( patterns; then find the nearest OVER (...)
    // and check whether it contains "RANGE".
    let lag_patterns = ["LAG(", "LEAD("];
    for pattern in &lag_patterns {
        let mut search_from = 0;
        while let Some(pos) = upper_sql[search_from..].find(pattern) {
            let abs = search_from + pos;
            // Find "OVER" after this position
            if let Some(over_rel) = upper_sql[abs..].find("OVER") {
                let over_abs = abs + over_rel;
                // Find the paren block after OVER
                if let Some(paren_start_rel) = upper_sql[over_abs..].find('(') {
                    let paren_start = over_abs + paren_start_rel;
                    if let Some(over_content) = extract_balanced_parens_str(upper_sql, paren_start)
                    {
                        // If there's no RANGE keyword inside the OVER clause, it's bare.
                        if !over_content.contains("RANGE") {
                            return true;
                        }
                    }
                }
            }
            search_from = abs + 1;
        }
    }
    false
}

/// Leaf classifier (`docs/specs/architecture.md` §"Property composition walk
/// rule"): invoked by [`derive_region_reach`] over one walk node's own region
/// text (and by [`derive_bound_for_source`] as the whole-text fallback).
///
/// Check for RANGE BETWEEN UNBOUNDED PRECEDING in the SQL.
fn has_unbounded_preceding_range(upper_sql: &str) -> bool {
    upper_sql.contains("UNBOUNDED PRECEDING")
}

/// Leaf classifier (`docs/specs/architecture.md` §"Property composition walk
/// rule"): invoked by [`derive_region_reach`] over one walk node's own region
/// text (and by [`derive_bound_for_source`] as the whole-text fallback).
///
/// True if any `INTERVAL '...'` literal in the SQL parses to
/// `Offset::Symbolic` (a month/year literal) — a calendar-relative unit that
/// cannot populate a `Bounded{before,after}` value at all. A single scan
/// over every `INTERVAL` occurrence in the whole statement, rather than
/// scoping to the specific Form A/B position it was found at: over-refusing
/// a source that happens to use a symbolic interval anywhere (even one that
/// doesn't participate in a bound-relevant position) is the fail-closed
/// direction, consistent with the rest of this text-scanning walk (e.g.
/// [`has_unbounded_preceding_range`]).
fn has_symbolic_interval_in_bound_position(upper_sql: &str) -> bool {
    if !upper_sql.contains("INTERVAL") {
        return false;
    }
    let mut search_from = 0;
    let kw = "INTERVAL";
    while let Some(rel) = upper_sql[search_from..].find(kw) {
        let abs = search_from + rel;
        let after = &upper_sql[abs + kw.len()..];
        if matches!(
            parse_quoted_interval_offset(after),
            Some(Offset::Symbolic(_))
        ) {
            return true;
        }
        search_from = abs + 1;
    }
    false
}

/// Leaf classifier (`docs/specs/architecture.md` §"Property composition walk
/// rule"): invoked by [`derive_region_reach`] over one walk node's own region
/// text (and by [`derive_bound_for_source`] as the whole-text fallback).
///
/// Check whether any `RANGE BETWEEN` frame in the SQL has an unbounded or
/// unparseable forward (`FOLLOWING`) reach — the mirror of
/// [`has_unbounded_preceding_range`]. Unlike the `PRECEDING` check (a plain
/// substring search, safe because a bound that resolves to `(0, 0)` never
/// gets accumulated and so always falls through to this fallback), the
/// `FOLLOWING` side is checked per-frame via [`frame_forward_is_unbounded`]
/// so it still fires even when the *same* frame also carries a derivable,
/// nonzero backward margin (e.g. `RANGE BETWEEN INTERVAL '1 day' PRECEDING
/// AND UNBOUNDED FOLLOWING`) — that frame's `before` would otherwise
/// accumulate into a `Bounded` result before the "no bound found" fallback
/// (where a bare substring check would live) is ever reached.
fn has_unbounded_forward_reach(upper_sql: &str) -> bool {
    let keyword = "RANGE BETWEEN ";
    let mut search_from = 0;
    while let Some(rel) = upper_sql[search_from..].find(keyword) {
        let abs = search_from + rel;
        let after_range_between = &upper_sql[abs + keyword.len()..];
        if frame_forward_is_unbounded(after_range_between) {
            return true;
        }
        search_from = abs + 1;
    }
    false
}

/// Leaf classifier (`docs/specs/architecture.md` §"Property composition walk
/// rule"): sub-helper of [`has_unbounded_forward_reach`], scoped to the text
/// following one `RANGE BETWEEN` occurrence.
///
/// True if the text after `RANGE BETWEEN ` carries a `FOLLOWING` bound that
/// is either `UNBOUNDED FOLLOWING` or a `FOLLOWING` bound with no parseable
/// `INTERVAL` literal before it (an unrecognised/non-Form-A forward bound —
/// refuse rather than silently treat it as zero-margin). A frame with no
/// `FOLLOWING` at all (e.g. `... AND CURRENT ROW`) is not flagged.
fn frame_forward_is_unbounded(text: &str) -> bool {
    let Some(and_pos) = text.find(" AND ") else {
        return false;
    };
    let after_and = &text[and_pos + 5..];
    let Some(fol_pos) = after_and.find("FOLLOWING") else {
        return false;
    };
    let before_fol = &after_and[..fol_pos];
    if before_fol.contains("UNBOUNDED") {
        return true;
    }
    parse_interval_seconds_before(before_fol).is_none()
}

/// Leaf classifier (`docs/specs/architecture.md` §"Property composition walk
/// rule"): invoked by [`derive_region_reach`] over one walk node's own region
/// text (and by [`derive_bound_for_source`] as the whole-text fallback).
///
/// Extract Form A bounds: (before, after) from RANGE BETWEEN INTERVAL patterns.
///
/// Scans for `RANGE BETWEEN` followed by interval specs.
fn extract_form_a_bounds(upper_sql: &str) -> Vec<(Seconds, Seconds)> {
    let mut bounds = Vec::new();
    let keyword = "RANGE BETWEEN ";
    let mut search_from = 0;

    while let Some(rel) = upper_sql[search_from..].find(keyword) {
        let abs = search_from + rel;
        let after_range_between = &upper_sql[abs + keyword.len()..];

        let (before, after) = parse_between_bounds(after_range_between);
        if before > Seconds::ZERO || after > Seconds::ZERO {
            bounds.push((before, after));
        }
        search_from = abs + 1;
    }

    bounds
}

/// Leaf classifier (`docs/specs/architecture.md` §"Property composition walk
/// rule"): sub-helper of [`extract_form_a_bounds`], scoped to text following
/// one `RANGE BETWEEN` occurrence.
///
/// Parse "... PRECEDING AND ... FOLLOWING/CURRENT ROW" from text after "RANGE BETWEEN".
fn parse_between_bounds(text: &str) -> (Seconds, Seconds) {
    let mut before = Seconds::ZERO;
    let mut after = Seconds::ZERO;

    // Find PRECEDING part
    if let Some(prec_pos) = text.find("PRECEDING") {
        let before_prec = &text[..prec_pos];
        if let Some(interval_secs) = parse_interval_seconds_before(before_prec) {
            before = interval_secs;
        }
    }

    // Find FOLLOWING part (after AND)
    if let Some(and_pos) = text.find(" AND ") {
        let after_and = &text[and_pos + 5..];
        if let Some(fol_pos) = after_and.find("FOLLOWING") {
            let before_fol = &after_and[..fol_pos];
            if let Some(interval_secs) = parse_interval_seconds_before(before_fol) {
                after = interval_secs;
            }
        }
    }

    (before, after)
}

/// Leaf classifier (`docs/specs/architecture.md` §"Property composition walk
/// rule"): invoked by [`derive_region_reach`] over one walk node's own region
/// text (and by [`derive_bound_for_source`] as the whole-text fallback).
///
/// Extract Form B bounds: (before, after) from WHERE/JOIN BETWEEN/comparison with INTERVAL.
///
/// Patterns recognised:
/// - `col BETWEEN expr - INTERVAL '...' AND expr + INTERVAL '...'`
/// - `col BETWEEN expr - INTERVAL '...' AND expr`  (no lookahead)
/// - `col >= expr - INTERVAL '...'` and `col < expr + INTERVAL '...'`
///
/// The `partition_col_upper` column must be the one immediately to the left of
/// the matched `BETWEEN`/comparison operator (bare or table-qualified, e.g.
/// `E.EVENT_DATE`) — see [`lhs_column_is_partition_col`]. This scopes a Form-B
/// pattern to the source it actually filters, rather than attributing *any*
/// `BETWEEN ... INTERVAL ...` pattern found anywhere in the whole model SQL to
/// every source regardless of which column it constrains (the `SC-1` ledger
/// finding: a correlated-`EXISTS` predicate on `conversions.conversion_date`
/// was also being attributed to the unrelated `events.event_date` source,
/// spuriously widening its read). Cross-column rebase
/// (`WHERE b.event_ts_utc BETWEEN m.event_date_local - INTERVAL ... AND
/// m.event_date_local + INTERVAL ...`) is still supported: only the LHS
/// column is checked, not the anchor expression on the right of the operator.
fn extract_form_b_bounds(upper_sql: &str, partition_col_upper: &str) -> Vec<(Seconds, Seconds)> {
    let mut bounds = Vec::new();

    // Find BETWEEN ... INTERVAL ... AND ... INTERVAL ... patterns
    let keyword = "BETWEEN ";
    let mut search_from = 0;

    while let Some(rel) = upper_sql[search_from..].find(keyword) {
        let abs = search_from + rel;
        if !lhs_column_is_partition_col(upper_sql, abs, partition_col_upper) {
            search_from = abs + 1;
            continue;
        }
        let after_between = &upper_sql[abs + keyword.len()..];

        // The text after BETWEEN looks like:
        // "expr - INTERVAL '1 day' AND expr + INTERVAL '1 day'"
        // or
        // "expr - INTERVAL '1 day' AND expr"
        // Find AND at depth 0
        if let Some(and_pos) = find_and_at_depth0(after_between) {
            let lower_expr = &after_between[..and_pos];
            // Skip " AND", then truncate to the upper-bound *expression* —
            // without this, a zero-margin upper bound absorbs `+ INTERVAL`
            // literals from later, unrelated predicates.
            let upper_expr = expression_prefix(&after_between[and_pos + 4..]);

            // Check for INTERVAL in lower (before) expression
            let before = if lower_expr.contains("INTERVAL") {
                // Look for "- INTERVAL '...'" pattern — this is a lookback
                extract_interval_from_subtraction(lower_expr).unwrap_or(
                    extract_interval_seconds_in_text(lower_expr).unwrap_or(Seconds::ZERO),
                )
            } else {
                Seconds::ZERO
            };

            // Check for INTERVAL in upper (after) expression
            let after_secs = if upper_expr.contains("INTERVAL") {
                // Look for "+ INTERVAL '...'" pattern — this is a lookahead
                extract_interval_from_addition(upper_expr).unwrap_or(
                    extract_interval_seconds_in_text(upper_expr).unwrap_or(Seconds::ZERO),
                )
            } else {
                Seconds::ZERO
            };

            if before > Seconds::ZERO || after_secs > Seconds::ZERO {
                bounds.push((before, after_secs));
            }
        }

        search_from = abs + 1;
    }

    // Also check >= ... - INTERVAL / < ... + INTERVAL patterns
    let gte_bounds = extract_gte_lt_interval_bounds(upper_sql, partition_col_upper);
    bounds.extend(gte_bounds);

    // Also check >= ... - <int> / < ... + <int> patterns — the bare-integer
    // sibling of the INTERVAL-literal form above, for monotone non-temporal
    // partition keys (sequence id / offset / watermark). See
    // `docs/specs/batched_models.md` §Surface "`partition_column` must be
    // monotone".
    let bare_integer_bounds = extract_gte_lt_bare_integer_bounds(upper_sql, partition_col_upper);
    bounds.extend(bare_integer_bounds);

    bounds
}

/// Leaf classifier (`docs/specs/architecture.md` §"Property composition walk
/// rule"): sub-helper of [`extract_form_b_bounds`] / the `extract_gte_lt_*`
/// pair, scoped to the text immediately preceding one comparison operator.
///
/// True if the identifier immediately preceding `pos` in `upper_sql` (skipping
/// trailing whitespace), read as a bare or table-qualified column reference
/// (`EVENT_DATE` or `E.EVENT_DATE`), is the given partition column. Used to
/// scope a Form-B `BETWEEN`/`>=`/`<` match to the source it actually
/// constrains — see [`extract_form_b_bounds`].
fn lhs_column_is_partition_col(upper_sql: &str, pos: usize, partition_col_upper: &str) -> bool {
    let before = upper_sql[..pos].trim_end();
    let ident_start = before
        .rfind(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '.'))
        .map(|i| i + 1)
        .unwrap_or(0);
    let ident = &before[ident_start..];
    if ident.is_empty() {
        return false;
    }
    ident == partition_col_upper || ident.ends_with(&format!(".{partition_col_upper}"))
}

/// Find " AND " at parenthesis depth 0 within the text.
fn find_and_at_depth0(text: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut depth = 0i32;
    let and_kw = b" AND ";
    let mut i = 0;

    while i + 5 <= bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => depth -= 1,
            _ => {}
        }
        if depth == 0 && &bytes[i..i + 5] == and_kw {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Parse INTERVAL seconds from a subtraction expression like "expr - INTERVAL '1 day'".
/// Truncate `text` (already uppercased) to its leading *expression*: stop at
/// the first depth-0 boolean connective or clause keyword, or at a `)` that
/// closes a parenthesis opened before `text` began. Interval extraction runs
/// over this prefix only, so a bound never absorbs a `± INTERVAL` literal
/// belonging to a later, unrelated predicate (the within-source sibling of
/// the SC-1b cross-source leak; regression tests
/// `test_form_b_between_upper_bound_does_not_absorb_later_intervals` /
/// `test_gte_lte_bound_does_not_absorb_later_intervals`).
fn expression_prefix(text: &str) -> &str {
    const BOUNDARIES: [&[u8]; 16] = [
        b" AND ",
        b" OR ",
        b" WHERE ",
        b" QUALIFY ",
        b" GROUP ",
        b" ORDER ",
        b" HAVING ",
        b" LIMIT ",
        b" JOIN ",
        b" LEFT ",
        b" RIGHT ",
        b" INNER ",
        b" FULL ",
        b" CROSS ",
        b" ON ",
        b" UNION ",
    ];
    let bytes = text.as_bytes();
    let mut depth = 0i32;
    for i in 0..bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth < 0 {
                    // Closes a paren opened before this expression — the
                    // enclosing group (subquery, call) ends here. `)` is
                    // ASCII, so `i` is a char boundary.
                    return &text[..i];
                }
            }
            b' ' if depth == 0 && BOUNDARIES.iter().any(|b| bytes[i..].starts_with(b)) => {
                return &text[..i];
            }
            _ => {}
        }
    }
    text
}

fn extract_interval_from_subtraction(text: &str) -> Option<Seconds> {
    // Find "- INTERVAL" pattern
    let sub_kw = "- INTERVAL";
    if let Some(pos) = text.find(sub_kw) {
        let after = &text[pos + sub_kw.len()..];
        return parse_quoted_interval(after);
    }
    None
}

/// Parse INTERVAL seconds from an addition expression like "expr + INTERVAL '1 day'".
fn extract_interval_from_addition(text: &str) -> Option<Seconds> {
    let add_kw = "+ INTERVAL";
    if let Some(pos) = text.find(add_kw) {
        let after = &text[pos + add_kw.len()..];
        return parse_quoted_interval(after);
    }
    None
}

/// Extract interval seconds from any INTERVAL '...' in the text.
fn extract_interval_seconds_in_text(text: &str) -> Option<Seconds> {
    let kw = "INTERVAL";
    if let Some(pos) = text.find(kw) {
        let after = &text[pos + kw.len()..];
        return parse_quoted_interval(after);
    }
    None
}

/// Parse a quoted interval like " '1 day'" or " '30 minutes'" into the
/// unified [`Offset`] representation (see `parse_interval`).
fn parse_quoted_interval_offset(text: &str) -> Option<Offset> {
    let trimmed = text.trim();
    // Find the quoted value
    let quote_start = trimmed.find('\'')?;
    let rest = &trimmed[quote_start + 1..];
    let quote_end = rest.find('\'')?;
    let value = &rest[..quote_end];

    parse_interval(value)
}

/// Parse a quoted interval like " '1 day'" or " '30 minutes'" and return
/// `Seconds`. Returns `None` both when no interval is present and when the
/// interval is present but `Offset::Symbolic` (month/year) — the caller
/// cannot distinguish "absent" from "symbolic" here, which is why
/// [`has_symbolic_interval_in_bound_position`] runs its own dedicated,
/// fail-closed scan *before* Form A/B accumulation ever calls this.
fn parse_quoted_interval(text: &str) -> Option<Seconds> {
    match parse_quoted_interval_offset(text)? {
        Offset::Seconds(s) => Some(s),
        Offset::Symbolic(_) => None,
        // `parse_interval` (the parser `parse_quoted_interval_offset` calls
        // into) only ever folds a quoted `INTERVAL '<value>'` string to
        // `Seconds`/`Symbolic` — it has no syntax that yields `Integer`
        // (that variant is only ever constructed from a *bare*, non-INTERVAL
        // integer literal — see `monotonicity::classify_binary`). This arm
        // exists solely so the match stays exhaustive now that `Offset`
        // carries a third variant; it is unreachable in practice.
        Offset::Integer(_) => None,
    }
}

/// Extract interval seconds from text before a keyword (used for PRECEDING).
/// Text like "INTERVAL '30 MINUTES' PRECEDING" → parse before "PRECEDING".
fn parse_interval_seconds_before(text: &str) -> Option<Seconds> {
    // The text contains "INTERVAL '...' " followed by the boundary keyword.
    // Find INTERVAL and parse the quoted value.
    let kw = "INTERVAL";
    let pos = text.rfind(kw)?; // use rfind in case there's noise before
    let after = &text[pos + kw.len()..];
    parse_quoted_interval(after)
}

/// Extract bounds from >= / < (or <=) patterns with INTERVAL.
///
/// Leaf classifier (`docs/specs/architecture.md` §"Property composition walk
/// rule"): invoked by [`extract_form_b_bounds`], which is itself invoked by
/// [`derive_region_reach`] over one walk node's own region text (and by
/// [`derive_bound_for_source`] as the whole-text fallback).
///
/// Pattern: `col >= expr - INTERVAL '...'` (gives `before`)
/// and `col < expr + INTERVAL '...'` (gives `after`)
///
/// `partition_col_upper`-scoped: see [`lhs_column_is_partition_col`].
fn extract_gte_lt_interval_bounds(
    upper_sql: &str,
    partition_col_upper: &str,
) -> Vec<(Seconds, Seconds)> {
    let mut bounds = Vec::new();
    let mut before = Seconds::ZERO;
    let mut after_secs = Seconds::ZERO;
    let mut found = false;

    // Look for >= ... - INTERVAL
    let gte_kw = ">= ";
    let mut search_from = 0;
    while let Some(rel) = upper_sql[search_from..].find(gte_kw) {
        let abs = search_from + rel;
        if !lhs_column_is_partition_col(upper_sql, abs, partition_col_upper) {
            search_from = abs + 1;
            continue;
        }
        let after_gte = expression_prefix(&upper_sql[abs + gte_kw.len()..]);
        if after_gte.contains("- INTERVAL") {
            if let Some(s) = extract_interval_from_subtraction(after_gte) {
                before = before.max(s);
                found = true;
            }
        }
        search_from = abs + 1;
    }

    // Look for < ... + INTERVAL or <= ... + INTERVAL
    for lt_kw in &["< ", "<= "] {
        let mut search_from2 = 0;
        while let Some(rel) = upper_sql[search_from2..].find(lt_kw) {
            let abs = search_from2 + rel;
            if !lhs_column_is_partition_col(upper_sql, abs, partition_col_upper) {
                search_from2 = abs + 1;
                continue;
            }
            let after_lt = expression_prefix(&upper_sql[abs + lt_kw.len()..]);
            if after_lt.contains("+ INTERVAL") {
                if let Some(s) = extract_interval_from_addition(after_lt) {
                    after_secs = after_secs.max(s);
                    found = true;
                }
            }
            search_from2 = abs + 1;
        }
    }

    if found {
        bounds.push((before, after_secs));
    }

    bounds
}

/// Leaf classifier (`docs/specs/architecture.md` §"Property composition walk
/// rule"): invoked by [`extract_form_b_bounds`], which is itself invoked by
/// [`derive_region_reach`] over one walk node's own region text (and by
/// [`derive_bound_for_source`] as the whole-text fallback).
///
/// Extract bounds from bare-integer `>= expr - <int>` / `< expr + <int>` (or
/// `<= expr + <int>`) patterns — the non-temporal sibling of
/// [`extract_gte_lt_interval_bounds`], for monotone integer partition keys
/// (sequence id / offset / watermark) rather than timestamps.
///
/// Fail-closed scoping: this path only activates when the *whole* SQL
/// statement contains no `INTERVAL` keyword at all. A model mixing
/// `INTERVAL`-based temporal arithmetic and bare-integer arithmetic in the
/// same statement is not a supported shape here — rather than risk
/// misreading an unrelated bare number inside/adjacent to an INTERVAL-based
/// clause, this bare-integer extraction stays inert whenever `INTERVAL`
/// appears anywhere, leaving [`extract_gte_lt_interval_bounds`] as the sole
/// Form B extractor for that SQL (consistent with the conservative,
/// whole-statement scoping [`has_symbolic_interval_in_bound_position`]
/// already uses).
///
/// The magnitude is stored in the same `Seconds` (`u64`) field `Bounded`
/// already carries — for an integer-keyed source this is not literally an
/// elapsed-seconds duration, it is the constant integer shift's magnitude
/// reused as the generic "how far" unit `BoundResult` was already using for
/// merge/injection-point comparisons.
fn extract_gte_lt_bare_integer_bounds(
    upper_sql: &str,
    partition_col_upper: &str,
) -> Vec<(Seconds, Seconds)> {
    if upper_sql.contains("INTERVAL") {
        return Vec::new();
    }

    let mut bounds = Vec::new();
    let mut before = Seconds::ZERO;
    let mut after_secs = Seconds::ZERO;
    let mut found = false;

    let gte_kw = ">= ";
    let mut search_from = 0;
    while let Some(rel) = upper_sql[search_from..].find(gte_kw) {
        let abs = search_from + rel;
        if !lhs_column_is_partition_col(upper_sql, abs, partition_col_upper) {
            search_from = abs + 1;
            continue;
        }
        let after_gte = &upper_sql[abs + gte_kw.len()..];
        if let Some(n) = extract_scoped_bare_integer_after_op(after_gte, '-') {
            before = before.max(Seconds(n));
            found = true;
        }
        search_from = abs + 1;
    }

    for lt_kw in &["< ", "<= "] {
        let mut search_from2 = 0;
        while let Some(rel) = upper_sql[search_from2..].find(lt_kw) {
            let abs = search_from2 + rel;
            if !lhs_column_is_partition_col(upper_sql, abs, partition_col_upper) {
                search_from2 = abs + 1;
                continue;
            }
            let after_lt = &upper_sql[abs + lt_kw.len()..];
            if let Some(n) = extract_scoped_bare_integer_after_op(after_lt, '+') {
                after_secs = after_secs.max(Seconds(n));
                found = true;
            }
            search_from2 = abs + 1;
        }
    }

    if found {
        bounds.push((before, after_secs));
    }

    bounds
}

/// Within `text` (everything after a `>=`/`</<=` comparison operator, up to
/// but not including the next same-clause `AND`), find the last `<op> <digits>`
/// occurrence (e.g. `- 5`) and parse the digits. Scoping to the text before
/// the first ` AND ` keeps this from reading a number out of an unrelated,
/// later clause in the same statement.
fn extract_scoped_bare_integer_after_op(text: &str, op: char) -> Option<u64> {
    let scoped = match text.find(" AND ") {
        Some(pos) => &text[..pos],
        None => text,
    };
    let pat = format!("{op} ");
    let pos = scoped.rfind(pat.as_str())?;
    let after = scoped[pos + pat.len()..].trim_start();
    let digits: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    digits.parse::<u64>().ok()
}

/// Extract the content inside a balanced `(...)` starting at `paren_pos`.
fn extract_balanced_parens_str(sql: &str, paren_pos: usize) -> Option<String> {
    let bytes = sql.as_bytes();
    if paren_pos >= bytes.len() || bytes[paren_pos] != b'(' {
        return None;
    }
    let mut depth = 0usize;
    let mut start = None;
    let mut end = None;
    for (i, &b) in bytes[paren_pos..].iter().enumerate() {
        match b {
            b'(' => {
                depth += 1;
                if depth == 1 {
                    start = Some(paren_pos + i + 1);
                }
            }
            b')' => {
                depth -= 1;
                if depth == 0 {
                    end = Some(paren_pos + i);
                    break;
                }
            }
            _ => {}
        }
    }
    let s = start?;
    let e = end?;
    Some(sql[s..e].to_string())
}

/// Resolve the `smelt.<path>` source name a `TableRef` refers to, dot-joining
/// its path segments (e.g. `smelt.silver.events_parsed` → `"silver.events_parsed"`,
/// matching the ref names in `ModelInfo.refs`/`BoundContext::source_partition_cols`).
/// Returns `None` for table refs that are not `smelt.<path>` refs/calls
/// (bare identifiers, CTE references, subqueries) — those are not join
/// driving-fact candidates by this resolution (they have no known
/// partition column to trace against).
pub fn resolve_table_ref_source_name(table_ref: &smelt_parser::TableRef) -> Option<String> {
    if let Some(path_ref) = table_ref.smelt_path_ref() {
        let segs = path_ref.segments();
        if !segs.is_empty() {
            return Some(segs.join("."));
        }
    }
    if let Some(path_call) = table_ref.smelt_path_call() {
        let segs = path_call.segments();
        if !segs.is_empty() {
            return Some(segs.join("."));
        }
    }
    None
}

/// Build the list of `(alias_or_source_name, source_name)` pairs for every
/// `smelt.<path>` input in a FROM clause — the base table ref plus every
/// JOIN's table ref. The key is the table ref's explicit/implicit alias if
/// present, else the source name itself (matching how an unqualified
/// column reference would resolve). Non-ref inputs (subqueries, CTE
/// references) are skipped — they are not join driving-fact candidates.
pub fn from_clause_alias_sources(from_clause: &smelt_parser::FromClause) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut push = |table_ref: &smelt_parser::TableRef| {
        if let Some(source) = resolve_table_ref_source_name(table_ref) {
            let key = table_ref.alias().unwrap_or_else(|| source.clone());
            out.push((key, source));
        }
    };
    for table_ref in from_clause.table_refs() {
        push(&table_ref);
    }
    for join in from_clause.joins() {
        if let Some(table_ref) = join.table_ref() {
            push(&table_ref);
        }
    }
    out
}

/// Why a single anchor could not be resolved among a scope's joined inputs.
pub enum AnchorAmbiguity {
    /// No candidate among the joined inputs satisfied the test.
    NoCandidate,
    /// More than one candidate satisfied the test — their (alias-scoped)
    /// source names, for the caller's diagnostic.
    Multiple(Vec<String>),
}

/// Disambiguate, among the joined inputs of a scope (`alias_sources`, from
/// [`from_clause_alias_sources`]), the single input satisfying `is_candidate`
/// — the anchor-resolution primitive `model_properties.md` §"Driving-fact /
/// anchor resolution" names: exactly-one-candidate is required; zero or two
/// or more is fail-closed (never guess the anchor).
///
/// This is the shared disambiguation shared by [`resolve_join_driving_fact`]
/// (candidate test: re-traces the event-time expression against each
/// alias-scoped source) and `rules::cumulative::classify_cumulative`
/// (candidate test: is this alias's source registered with a `timeseries:`
/// block) — the two independent driving-fact resolvers this phase merges.
pub fn resolve_single_anchor<T>(
    alias_sources: &[(String, String)],
    mut is_candidate: impl FnMut(&str) -> Option<T>,
) -> Result<T, AnchorAmbiguity> {
    let mut found: Vec<(String, T)> = Vec::new();
    for (_, source_name) in alias_sources {
        if let Some(payload) = is_candidate(source_name) {
            found.push((source_name.clone(), payload));
        }
    }
    if found.len() > 1 {
        return Err(AnchorAmbiguity::Multiple(
            found.into_iter().map(|(name, _)| name).collect(),
        ));
    }
    match found.into_iter().next() {
        Some((_, payload)) => Ok(payload),
        None => Err(AnchorAmbiguity::NoCandidate),
    }
}

/// Resolve the "driving fact" among a join's inputs for a traced
/// `event_time_expr` (Join relaxation, `batched_models.md` §"Event-time
/// monotonicity trace").
///
/// `alias_sources` is the FROM/JOIN alias→source map (`from_clause_alias_sources`);
/// `full_ctx` maps every known timeseries source (reachable from the model's
/// refs) to its partition column.
///
/// Resolution never widens `trace_event_time`'s own decision logic — each
/// candidate is tested against a *singleton* `BoundContext` containing only
/// that one candidate's `(source, partition_column)`, so
/// `resolve_against_ctx`'s ambiguous-name fallback inside the pure primitive
/// never triggers; ambiguity is instead decided here, across candidates,
/// which is the "alias-scoped leaf resolution" this phase adds:
///
/// - If the traced expression's leaf column carries an explicit qualifier
///   (e.g. `f.event_ts`) that matches one of the FROM/JOIN aliases, only that
///   aliased input is tested — this disambiguates two joined inputs whose
///   partition columns happen to share a bare column *name*, which the
///   name-only primitive alone cannot.
/// - Otherwise every candidate is tested independently; exactly one
///   `Traceable` result is required. Zero or more than one candidate tracing
///   fails closed to `NotTraceable` (Constraint 12 — never guess the driving
///   fact).
pub fn resolve_join_driving_fact(
    event_time_expr: &smelt_parser::Expr,
    alias_sources: &[(String, String)],
    full_ctx: &BoundContext,
    declared_monotonic: bool,
) -> EventTimeTrace {
    let trace_against = |source_name: &str| -> Option<EventTimeTrace> {
        let partition_col = full_ctx.source_partition_cols.get(source_name)?;
        let singleton = BoundContext::new().with_source(source_name, partition_col);
        Some(crate::analysis::monotonicity::trace_event_time_declared(
            event_time_expr,
            &singleton,
            declared_monotonic,
        ))
    };

    if let Some(qualifier) =
        find_leaf_column_ref(event_time_expr).and_then(|c| c.qualifier().map(|q| q.to_string()))
    {
        return match alias_sources.iter().find(|(alias, _)| alias == &qualifier) {
            Some((_, source_name)) => {
                trace_against(source_name).unwrap_or_else(|| EventTimeTrace::NotTraceable {
                    reason: format!(
                        "join input aliased '{qualifier}' is not a known timeseries source"
                    ),
                    kind: NotTraceableKind::Disproven,
                })
            }
            None => EventTimeTrace::NotTraceable {
                reason: format!(
                    "column qualifier '{qualifier}' does not match any FROM/JOIN alias"
                ),
                kind: NotTraceableKind::Disproven,
            },
        };
    }

    match resolve_single_anchor(alias_sources, |source_name| {
        match trace_against(source_name) {
            Some(trace @ EventTimeTrace::Traceable { .. }) => Some(trace),
            _ => None,
        }
    }) {
        Ok(trace) => trace,
        Err(AnchorAmbiguity::NoCandidate) => EventTimeTrace::NotTraceable {
            reason: "no join input's source partition column traces the event-time projection"
                .to_string(),
            kind: NotTraceableKind::Disproven,
        },
        Err(AnchorAmbiguity::Multiple(names)) => EventTimeTrace::NotTraceable {
            reason: format!(
                "ambiguous driving fact: {} join inputs trace to the same unqualified column name",
                names.len()
            ),
            kind: NotTraceableKind::Disproven,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- Form A tests ----

    /// Form A: `LAG(x) OVER (PARTITION BY id ORDER BY ts RANGE BETWEEN INTERVAL '30 minutes' PRECEDING AND CURRENT ROW)`
    /// over a source partitioned by `event_date` derives `Bounded(event_date, 30min, 0)`.
    #[test]
    fn test_range_between_interval_preceding() {
        let sql = "SELECT id, ts, LAG(x) OVER (PARTITION BY id ORDER BY ts \
                   RANGE BETWEEN INTERVAL '30 minutes' PRECEDING AND CURRENT ROW) AS prev_x \
                   FROM source_table";
        let ctx = BoundContext::new().with_source("silver.events_parsed", "event_date");
        let bounds = derive_model_bounds(sql, &ctx);
        let bound = bounds.get("silver.events_parsed").unwrap();
        match bound {
            BoundResult::Bounded {
                source_partition_col,
                before,
                after,
            } => {
                assert_eq!(source_partition_col, "event_date");
                assert_eq!(*before, Seconds::minutes(30), "before must be 30 minutes");
                assert_eq!(*after, Seconds::ZERO, "after must be zero");
            }
            other => panic!("Expected Bounded, got {:?}", other),
        }
    }

    /// Form A forward reach: `RANGE BETWEEN CURRENT ROW AND INTERVAL '2 hours'
    /// FOLLOWING` derives `Bounded(event_date, 0, 2h)` — the `before`-only B0
    /// walk left `after` at zero; this is the symmetric B2 forward-reach
    /// derivation off the *same* signed walk.
    #[test]
    fn test_range_between_interval_following() {
        let sql = "SELECT id, ts, LEAD(x) OVER (PARTITION BY id ORDER BY ts \
                   RANGE BETWEEN CURRENT ROW AND INTERVAL '2 hours' FOLLOWING) AS next_x \
                   FROM source_table";
        let ctx = BoundContext::new().with_source("silver.events_parsed", "event_date");
        let bounds = derive_model_bounds(sql, &ctx);
        let bound = bounds.get("silver.events_parsed").unwrap();
        match bound {
            BoundResult::Bounded {
                source_partition_col,
                before,
                after,
            } => {
                assert_eq!(source_partition_col, "event_date");
                assert_eq!(*before, Seconds::ZERO, "before must be zero");
                assert_eq!(*after, Seconds::hours(2), "after must be 2 hours");
            }
            other => panic!("Expected Bounded(event_date, 0, 2h), got {:?}", other),
        }
    }

    /// Fail-closed: `RANGE BETWEEN INTERVAL '1 day' PRECEDING AND UNBOUNDED
    /// FOLLOWING` has a genuinely-derivable (nonzero) backward margin, but its
    /// forward reach is unbounded — the whole source must refuse (`Unbounded`),
    /// not silently admit `after = 0`. Mirrors the `UNBOUNDED PRECEDING` reject.
    #[test]
    fn test_unbounded_following_is_unbounded() {
        let sql = "SELECT id, ts, LAG(x) OVER (PARTITION BY id ORDER BY ts \
                   RANGE BETWEEN INTERVAL '1 day' PRECEDING AND UNBOUNDED FOLLOWING) AS x \
                   FROM source_table";
        let ctx = BoundContext::new().with_source("silver.events_parsed", "event_date");
        let bounds = derive_model_bounds(sql, &ctx);
        let bound = bounds.get("silver.events_parsed").unwrap();
        assert_eq!(
            *bound,
            BoundResult::Unbounded,
            "UNBOUNDED FOLLOWING must refuse fail-closed as Unbounded, not derive after=0"
        );
    }

    /// Fail-closed: a `FOLLOWING` bound with no parseable `INTERVAL` (neither
    /// `UNBOUNDED` nor a quoted interval literal) is an unrecognised forward
    /// bound — refuse rather than treat it as zero-margin.
    #[test]
    fn test_following_with_no_interval_is_unbounded() {
        let sql = "SELECT id, ts, LAG(x) OVER (PARTITION BY id ORDER BY ts \
                   RANGE BETWEEN INTERVAL '1 day' PRECEDING AND 5 FOLLOWING) AS x \
                   FROM source_table";
        let ctx = BoundContext::new().with_source("silver.events_parsed", "event_date");
        let bounds = derive_model_bounds(sql, &ctx);
        let bound = bounds.get("silver.events_parsed").unwrap();
        assert_eq!(
            *bound,
            BoundResult::Unbounded,
            "a FOLLOWING bound with no INTERVAL literal must refuse fail-closed"
        );
    }

    /// Form B forward-only reach: `BETWEEN event_ts AND event_ts + INTERVAL
    /// '30 days'` (no backward offset) derives `Bounded(event_ts, 0, 30d)`.
    #[test]
    fn test_form_b_forward_only() {
        let sql = "SELECT * FROM events e \
                   WHERE e.event_ts BETWEEN m.conversion_ts AND m.conversion_ts + INTERVAL '30 days'";
        let ctx = BoundContext::new().with_source("silver.events", "event_ts");
        let bounds = derive_model_bounds(sql, &ctx);
        let bound = bounds.get("silver.events").unwrap();
        match bound {
            BoundResult::Bounded {
                source_partition_col,
                before,
                after,
            } => {
                assert_eq!(source_partition_col, "event_ts");
                assert_eq!(*before, Seconds::ZERO, "before must be zero");
                assert_eq!(*after, Seconds::days(30), "after must be 30 days");
            }
            other => panic!("Expected Bounded(event_ts, 0, 30d), got {:?}", other),
        }
    }

    /// Form B: `WHERE s.event_date BETWEEN m.partition_date - INTERVAL '1 day' AND m.partition_date`
    /// derives `Bounded(event_date, 1d, 0)`.
    #[test]
    fn test_explicit_between_filter() {
        let sql = "SELECT * FROM sessions s \
                   WHERE s.event_date BETWEEN m.partition_date - INTERVAL '1 day' AND m.partition_date";
        let ctx = BoundContext::new().with_source("silver.sessions", "event_date");
        let bounds = derive_model_bounds(sql, &ctx);
        let bound = bounds.get("silver.sessions").unwrap();
        match bound {
            BoundResult::Bounded {
                source_partition_col,
                before,
                after,
            } => {
                assert_eq!(source_partition_col, "event_date");
                assert_eq!(*before, Seconds::days(1), "before must be 1 day");
                assert_eq!(*after, Seconds::ZERO, "after must be zero");
            }
            other => panic!("Expected Bounded, got {:?}", other),
        }
    }

    /// Form B with cross-column rebase:
    /// `WHERE b.event_ts_utc BETWEEN m.event_date_local - INTERVAL '1 day' AND m.event_date_local + INTERVAL '1 day'`
    /// derives `Bounded(event_ts_utc, 1d, 1d)`.
    #[test]
    fn test_cross_column_tz_rebase() {
        let sql = "SELECT * FROM bronze.events b \
                   JOIN users u ON b.user_id = u.user_id \
                   WHERE b.event_ts_utc BETWEEN m.event_date_local - INTERVAL '1 day' \
                     AND m.event_date_local + INTERVAL '1 day'";
        let ctx = BoundContext::new().with_source("bronze.events", "event_ts_utc");
        let bounds = derive_model_bounds(sql, &ctx);
        let bound = bounds.get("bronze.events").unwrap();
        match bound {
            BoundResult::Bounded {
                source_partition_col,
                before,
                after,
            } => {
                assert_eq!(source_partition_col, "event_ts_utc");
                assert_eq!(*before, Seconds::days(1), "before must be 1 day");
                assert_eq!(*after, Seconds::days(1), "after must be 1 day");
            }
            other => panic!("Expected Bounded, got {:?}", other),
        }
    }

    /// Same source referenced twice with different ranges takes the union (max before, max after).
    #[test]
    fn test_aggregation_max() {
        // SQL has two BETWEEN clauses for the same source with different intervals.
        let sql = "SELECT * FROM events e1 \
                   JOIN events e2 ON e1.id = e2.id \
                   WHERE e1.event_date BETWEEN m.date - INTERVAL '1 day' AND m.date \
                   AND e2.event_date BETWEEN m.date - INTERVAL '3 days' AND m.date + INTERVAL '1 day'";
        let ctx = BoundContext::new().with_source("silver.events", "event_date");
        let bounds = derive_model_bounds(sql, &ctx);
        let bound = bounds.get("silver.events").unwrap();
        match bound {
            BoundResult::Bounded { before, after, .. } => {
                assert_eq!(*before, Seconds::days(3), "before must be max(1d, 3d) = 3d");
                assert_eq!(*after, Seconds::days(1), "after must be max(0, 1d) = 1d");
            }
            other => panic!("Expected Bounded, got {:?}", other),
        }
    }

    /// A source without `timeseries:` produces no bound entry.
    #[test]
    fn test_lookup_source_no_bound() {
        let sql = "SELECT * FROM events e JOIN users u ON e.user_id = u.user_id";
        // users is NOT in the context (it's a lookup), events IS
        let ctx = BoundContext::new().with_source("silver.events", "event_date");
        let bounds = derive_model_bounds(sql, &ctx);
        // events gets an entry (fully partition-local since no intervals)
        assert!(bounds.contains_key("silver.events"));
        // users has no entry
        assert!(!bounds.contains_key("bronze.users"));
    }

    /// `LAG(x) OVER (PARTITION BY id ORDER BY ts)` (no RANGE) derives `NotDerivable`.
    #[test]
    fn test_bare_lag_not_derivable() {
        let sql = "SELECT id, ts, LAG(x) OVER (PARTITION BY id ORDER BY ts) AS prev_x \
                   FROM source_table";
        let ctx = BoundContext::new().with_source("silver.events", "event_date");
        let bounds = derive_model_bounds(sql, &ctx);
        let bound = bounds.get("silver.events").unwrap();
        assert_eq!(
            *bound,
            BoundResult::NotDerivable,
            "bare LAG without RANGE must be NotDerivable"
        );
    }

    /// A model calling `smelt.functions.sessionize(...)` whose expanded body has
    /// `RANGE BETWEEN INTERVAL '30 minutes' PRECEDING` derives the bound.
    ///
    /// The "expanded SQL" is the model SQL with the function body inlined (concatenated).
    #[test]
    fn test_function_body_traversal() {
        // Simulate the expanded SQL: the model SQL plus the sessionize function body
        // (which contains RANGE BETWEEN INTERVAL '30 minutes' PRECEDING — note that
        // sessionize actually uses ROWS frames, so we simulate a 30-minute RANGE version).
        let expanded_sql = r#"
            WITH sessionized AS (
                SELECT
                    *,
                    LAG(epoch_us(event_ts)) OVER (PARTITION BY device_id ORDER BY event_ts
                        RANGE BETWEEN INTERVAL '30 minutes' PRECEDING AND CURRENT ROW) AS _smelt_prev_ts,
                    SUM(1) OVER (PARTITION BY device_id ORDER BY event_ts) AS session_seq
                FROM smelt.silver.events_parsed
            )
            SELECT
                device_id,
                session_seq,
                session_start_date,
                COUNT(*) AS event_count
            FROM sessionized
            GROUP BY device_id, session_seq, session_start_date
        "#;

        let ctx = BoundContext::new().with_source("silver.events_parsed", "event_date");
        let bounds = derive_model_bounds(expanded_sql, &ctx);
        let bound = bounds.get("silver.events_parsed").unwrap();
        match bound {
            BoundResult::Bounded {
                source_partition_col,
                before,
                after,
            } => {
                assert_eq!(source_partition_col, "event_date");
                assert_eq!(*before, Seconds::minutes(30), "before must be 30 minutes");
                assert_eq!(*after, Seconds::ZERO, "after must be zero");
            }
            other => panic!("Expected Bounded(event_date, 30min, 0), got {:?}", other),
        }
    }

    /// SC-1 regression (`docs/research/property-discovery/ledger.md` CELL
    /// SC-1): a correlated `EXISTS` predicate on `conversions.conversion_date`
    /// (`c.conversion_date BETWEEN e.event_date AND e.event_date + INTERVAL
    /// '7 days'`) must not also widen the *unrelated* `events.event_date`
    /// source's read — `event_date` never appears to the left of `BETWEEN` in
    /// this SQL, so `events` has no Form-B pattern of its own and must fall
    /// through to the partition-local `Bounded(event_date, 0, 0)` default.
    #[test]
    fn test_form_b_does_not_leak_bound_to_unrelated_source() {
        let sql = "SELECT e.event_date, e.user_id, EXISTS( \
                   SELECT 1 FROM conversions c \
                   WHERE c.user_id = e.user_id \
                     AND c.conversion_date BETWEEN e.event_date AND e.event_date + INTERVAL '7 days' \
                   ) AS converted FROM events e";
        let ctx = BoundContext::new()
            .with_source("silver.events", "event_date")
            .with_source("silver.conversions", "conversion_date");
        let bounds = derive_model_bounds(sql, &ctx);

        // conversions is the source the BETWEEN pattern actually constrains.
        let conversions_bound = bounds.get("silver.conversions").unwrap();
        match conversions_bound {
            BoundResult::Bounded {
                source_partition_col,
                before,
                after,
            } => {
                assert_eq!(source_partition_col, "conversion_date");
                assert_eq!(*before, Seconds::ZERO);
                assert_eq!(*after, Seconds::days(7));
            }
            other => panic!("Expected Bounded(conversion_date, 0, 7d), got {:?}", other),
        }

        // events must NOT inherit the conversions-only bound — it has no
        // Form-B pattern of its own in this SQL.
        let events_bound = bounds.get("silver.events").unwrap();
        assert_eq!(
            *events_bound,
            BoundResult::Bounded {
                source_partition_col: "event_date".to_string(),
                before: Seconds::ZERO,
                after: Seconds::ZERO,
            },
            "events must not be spuriously widened by conversions' BETWEEN pattern"
        );
    }

    /// Expression-delimitation regression (surfaced by the maintenance-plan
    /// tracer, `docs/research/20260705-refresh-as-maintenance-plan/`): the
    /// text scanned for a bound's `± INTERVAL` must stop at the end of the
    /// *expression* — a `BETWEEN`'s zero-margin upper bound must not absorb
    /// a `+ INTERVAL` literal belonging to a *later, unrelated* predicate.
    /// Same SC-1b species as cross-source leakage, but within one source's
    /// window: the spurious widening is safe-not-unsound, yet it silently
    /// inflates every derived scan (here a 2-day session lookback gained a
    /// phantom 48h lookahead from the lateness filter).
    #[test]
    fn test_form_b_between_upper_bound_does_not_absorb_later_intervals() {
        let sql = "SELECT e.event_id, s.session_id FROM events e \
                   LEFT JOIN sessions s \
                     ON s.user_id = e.user_id \
                    AND s.session_start_date BETWEEN CAST(e.event_ts AS DATE) - INTERVAL '2 days' \
                                                 AND CAST(e.event_ts AS DATE) \
                   WHERE e.arrival_ts < e.event_ts + INTERVAL '48 hours'";
        let ctx = BoundContext::new().with_source("sessions", "session_start_date");
        let bounds = derive_model_bounds(sql, &ctx);
        assert_eq!(
            *bounds.get("sessions").unwrap(),
            BoundResult::Bounded {
                source_partition_col: "session_start_date".to_string(),
                before: Seconds::days(2),
                after: Seconds::ZERO,
            },
            "the BETWEEN upper bound must not absorb the later '+ INTERVAL 48 hours'"
        );
    }

    /// Sibling of the test above for the `>= / <=` form: the text after a
    /// comparison operator must be truncated at the expression boundary
    /// before searching for `± INTERVAL`.
    #[test]
    fn test_gte_lte_bound_does_not_absorb_later_intervals() {
        let sql = "SELECT e.event_id, s.session_id FROM events e \
                   LEFT JOIN sessions s \
                     ON s.user_id = e.user_id \
                    AND s.session_start_date >= CAST(e.event_ts AS DATE) - INTERVAL '2 days' \
                    AND s.session_start_date <= CAST(e.event_ts AS DATE) \
                   WHERE e.arrival_ts < e.event_ts + INTERVAL '48 hours'";
        let ctx = BoundContext::new().with_source("sessions", "session_start_date");
        let bounds = derive_model_bounds(sql, &ctx);
        assert_eq!(
            *bounds.get("sessions").unwrap(),
            BoundResult::Bounded {
                source_partition_col: "session_start_date".to_string(),
                before: Seconds::days(2),
                after: Seconds::ZERO,
            },
            "the '<=' zero-margin bound must not absorb the later '+ INTERVAL 48 hours'"
        );
    }

    // ---- Duration serialization tests ----

    #[test]
    fn test_seconds_to_iso8601() {
        assert_eq!(Seconds::ZERO.to_iso8601(), "PT0S");
        assert_eq!(Seconds::minutes(30).to_iso8601(), "PT30M");
        assert_eq!(Seconds::days(1).to_iso8601(), "P1D");
        assert_eq!(Seconds::hours(2).to_iso8601(), "PT2H");
        // 90 seconds = PT1M30S
        assert_eq!(Seconds(90).to_iso8601(), "PT1M30S");
    }

    #[test]
    fn test_bound_result_merge() {
        let b1 = BoundResult::Bounded {
            source_partition_col: "event_date".to_string(),
            before: Seconds::days(1),
            after: Seconds::ZERO,
        };
        let b2 = BoundResult::Bounded {
            source_partition_col: "event_date".to_string(),
            before: Seconds::days(3),
            after: Seconds::days(1),
        };
        let merged = b1.merge(b2);
        match merged {
            BoundResult::Bounded { before, after, .. } => {
                assert_eq!(before, Seconds::days(3));
                assert_eq!(after, Seconds::days(1));
            }
            other => panic!("Expected Bounded, got {:?}", other),
        }
    }

    #[test]
    fn test_not_derivable_wins_merge() {
        let b1 = BoundResult::Bounded {
            source_partition_col: "col".to_string(),
            before: Seconds::days(1),
            after: Seconds::ZERO,
        };
        let merged = b1.merge(BoundResult::NotDerivable);
        assert_eq!(merged, BoundResult::NotDerivable);
    }

    #[test]
    fn test_unbounded_wins_over_bounded() {
        let b1 = BoundResult::Bounded {
            source_partition_col: "col".to_string(),
            before: Seconds::days(1),
            after: Seconds::ZERO,
        };
        let merged = b1.merge(BoundResult::Unbounded);
        assert_eq!(merged, BoundResult::Unbounded);
    }

    // ---- InjectionPoint classification tests (B0) ----

    /// A zero-margin `Bounded` source (the transparent slice) pushes all the
    /// way to the source scan — no outer wrap needed.
    #[test]
    fn test_injection_point_transparent_is_source() {
        let bound = BoundResult::Bounded {
            source_partition_col: "event_date".to_string(),
            before: Seconds::ZERO,
            after: Seconds::ZERO,
        };
        assert_eq!(bound.injection_point(), InjectionPoint::Source);
    }

    /// A `Bounded` source with a nonzero lookback needs the outer clamp too.
    #[test]
    fn test_injection_point_lookback_is_outer_clamp() {
        let bound = BoundResult::Bounded {
            source_partition_col: "event_date".to_string(),
            before: Seconds::days(1),
            after: Seconds::ZERO,
        };
        assert_eq!(bound.injection_point(), InjectionPoint::OuterClamp);
    }

    /// `Unbounded` and `NotDerivable` both conservatively route to the outer
    /// clamp classification (they are handled upstream — refused or sent to
    /// per-partition execution — before this classification matters).
    #[test]
    fn test_injection_point_unbounded_and_not_derivable_are_outer_clamp() {
        assert_eq!(
            BoundResult::Unbounded.injection_point(),
            InjectionPoint::OuterClamp
        );
        assert_eq!(
            BoundResult::NotDerivable.injection_point(),
            InjectionPoint::OuterClamp
        );
    }

    // ---- B1: alias-scoped leaf resolution (join driving-fact) ----

    /// Parse `sql`, return the (single) SELECT statement's `FromClause` and
    /// the first select item's expression.
    fn from_clause_and_first_expr(sql: &str) -> (smelt_parser::FromClause, smelt_parser::Expr) {
        let parse = smelt_parser::parse(sql);
        let root = parse.syntax();
        let file = smelt_parser::File::cast(root).expect("file cast");
        let select = file.select_stmt().expect("select stmt");
        let from_clause = select.from_clause().expect("from clause");
        let select_list = select.select_list().expect("select list");
        let expr = select_list
            .items()
            .next()
            .expect("first item")
            .expression()
            .expect("item expression");
        (from_clause, expr)
    }

    /// Two joined inputs whose partition columns share the bare name
    /// `event_ts` were, before alias-scoped resolution, ambiguous
    /// (`resolve_against_ctx` sees two sources matching the name "event_ts"
    /// and fails closed). A *qualified* reference (`f.event_ts`) now
    /// resolves unambiguously via the FROM/alias scope: `f` names the
    /// `fact` input, so only `fact`'s partition column is consulted.
    #[test]
    fn test_join_driving_fact_resolves_via_alias_despite_shared_column_name() {
        let (from_clause, expr) = from_clause_and_first_expr(
            "SELECT f.event_ts AS event_time FROM smelt.silver.fact f \
             JOIN smelt.silver.lookup g ON f.id = g.id",
        );
        let alias_sources = from_clause_alias_sources(&from_clause);
        let ctx = BoundContext::new()
            .with_source("silver.fact", "event_ts")
            .with_source("silver.lookup", "event_ts");

        match resolve_join_driving_fact(&expr, &alias_sources, &ctx, false) {
            EventTimeTrace::Traceable { source, .. } => {
                assert_eq!(
                    source, "silver.fact",
                    "alias 'f' must resolve to silver.fact"
                );
            }
            other => panic!("expected Traceable, got {other:?}"),
        }
    }

    /// Without a qualifier, two candidate inputs whose partition columns
    /// share the same bare name both independently trace `Traceable` —
    /// ambiguous, fails closed (never guess the driving fact).
    #[test]
    fn test_join_driving_fact_ambiguous_without_qualifier() {
        let (from_clause, expr) = from_clause_and_first_expr(
            "SELECT event_ts AS event_time FROM smelt.silver.fact f \
             JOIN smelt.silver.lookup g ON f.id = g.id",
        );
        let alias_sources = from_clause_alias_sources(&from_clause);
        let ctx = BoundContext::new()
            .with_source("silver.fact", "event_ts")
            .with_source("silver.lookup", "event_ts");

        assert!(matches!(
            resolve_join_driving_fact(&expr, &alias_sources, &ctx, false),
            EventTimeTrace::NotTraceable { .. }
        ));
    }

    /// Exactly one candidate traces (a fact ⋈ lookup join where only the
    /// fact side has a timeseries partition column matching the traced
    /// expression) — resolves to that source as the driving fact.
    #[test]
    fn test_join_driving_fact_single_candidate_resolves() {
        let (from_clause, expr) = from_clause_and_first_expr(
            "SELECT f.event_ts AS event_time FROM smelt.silver.fact f \
             JOIN smelt.silver.lookup g ON f.user_id = g.user_id",
        );
        let alias_sources = from_clause_alias_sources(&from_clause);
        // Only "fact" is a timeseries source; "lookup" carries no partition
        // column at all (absent from ctx — a plain lookup).
        let ctx = BoundContext::new().with_source("silver.fact", "event_ts");

        match resolve_join_driving_fact(&expr, &alias_sources, &ctx, false) {
            EventTimeTrace::Traceable { source, .. } => {
                assert_eq!(source, "silver.fact");
            }
            other => panic!("expected Traceable, got {other:?}"),
        }
    }

    /// `from_clause_alias_sources` resolves both the base table ref and
    /// every JOIN's table ref, keyed by alias.
    #[test]
    fn test_from_clause_alias_sources_covers_base_and_joins() {
        let (from_clause, _expr) = from_clause_and_first_expr(
            "SELECT f.event_ts AS event_time FROM smelt.silver.fact f \
             JOIN smelt.silver.lookup g ON f.id = g.id",
        );
        let alias_sources = from_clause_alias_sources(&from_clause);
        assert_eq!(alias_sources.len(), 2);
        assert!(alias_sources
            .iter()
            .any(|(alias, source)| alias == "f" && source == "silver.fact"));
        assert!(alias_sources
            .iter()
            .any(|(alias, source)| alias == "g" && source == "silver.lookup"));
    }

    // ---- Unified interval-reach walk (F1: fold temporal.rs's EffectiveWindow
    // walk and the divergent parsers into this single `derive_model_bounds`
    // spine) ----

    /// A `DATE_TRUNC('day', …)`-style Form B backfill window — the same
    /// day-granular case `temporal.rs::compute_effective_window` used to
    /// classify independently via its own text scan — now derives through
    /// `derive_model_bounds`, and its seconds-granular `before`/`after`
    /// equal the former `EffectiveWindow.lookback_days` / `.lookahead_days`
    /// span (2 days back, 1 day forward) converted to seconds.
    #[test]
    fn test_day_granular_backfill_window_matches_former_effective_window() {
        let sql = "SELECT * FROM sessions s \
                   WHERE s.event_date BETWEEN m.partition_date - INTERVAL '2 days' \
                     AND m.partition_date + INTERVAL '1 days'";
        let ctx = BoundContext::new().with_source("silver.sessions", "event_date");
        let bounds = derive_model_bounds(sql, &ctx);
        let bound = bounds.get("silver.sessions").unwrap();
        match bound {
            BoundResult::Bounded { before, after, .. } => {
                let former_lookback_days: u64 = 2;
                let former_lookahead_days: u64 = 1;
                assert_eq!(before.0, former_lookback_days * 86400);
                assert_eq!(after.0, former_lookahead_days * 86400);
            }
            other => panic!("Expected Bounded, got {:?}", other),
        }
    }

    /// One interval parser (`parse_interval`), shared by every call site in
    /// this module: `'1 month'` folds to `Offset::Symbolic` (not an
    /// approximate ≈30-day count — see `has_symbolic_interval_in_bound_position`),
    /// while `'120 minutes'` and `'2 hours'` — both routed through the same
    /// parser — fold to the same seconds magnitude (7200s). (Neither the
    /// former nor the unified parser supports a fractional leading number
    /// like `'1.5 hours'` — `parts[0].parse::<u64>()` rejects it — so this
    /// pre-existing limitation is exercised with whole-unit equivalents
    /// instead of invented fractional support.)
    #[test]
    fn test_unified_parser_month_is_symbolic_not_approximate_days() {
        let sql_month = "SELECT id, ts, LAG(x) OVER (PARTITION BY id ORDER BY ts \
                   RANGE BETWEEN INTERVAL '1 month' PRECEDING AND CURRENT ROW) AS x \
                   FROM source_table";
        let ctx = BoundContext::new().with_source("silver.events", "event_date");
        let bounds = derive_model_bounds(sql_month, &ctx);
        assert_eq!(
            *bounds.get("silver.events").unwrap(),
            BoundResult::NotDerivable,
            "a symbolic (month/year) interval must refuse fail-closed, \
             not silently approximate to ~30 days"
        );
    }

    #[test]
    fn test_unified_parser_minutes_and_hours_fold_to_same_seconds_magnitude() {
        let sql_minutes = "SELECT id, ts, LAG(x) OVER (PARTITION BY id ORDER BY ts \
                   RANGE BETWEEN INTERVAL '120 minutes' PRECEDING AND CURRENT ROW) AS x \
                   FROM source_table";
        let sql_hours = "SELECT id, ts, LAG(x) OVER (PARTITION BY id ORDER BY ts \
                   RANGE BETWEEN INTERVAL '2 hours' PRECEDING AND CURRENT ROW) AS x \
                   FROM source_table";
        let ctx = BoundContext::new().with_source("silver.events", "event_date");

        let bounds_minutes = derive_model_bounds(sql_minutes, &ctx);
        let bounds_hours = derive_model_bounds(sql_hours, &ctx);

        let extract_before =
            |bounds: &HashMap<String, BoundResult>| match bounds.get("silver.events").unwrap() {
                BoundResult::Bounded { before, .. } => *before,
                other => panic!("Expected Bounded, got {:?}", other),
            };

        assert_eq!(extract_before(&bounds_minutes), Seconds::hours(2));
        assert_eq!(extract_before(&bounds_hours), Seconds::hours(2));
    }

    /// Fail-closed regression guard (unchanged by the unification): a
    /// `NotDerivable` source (bare LAG/LEAD with no RANGE frame) still
    /// refuses — it is not silently treated as a zero-margin bound.
    #[test]
    fn test_not_derivable_still_fail_closed_after_unification() {
        let sql = "SELECT id, ts, LEAD(x) OVER (PARTITION BY id ORDER BY ts) AS next_x \
                   FROM source_table";
        let ctx = BoundContext::new().with_source("silver.events", "event_date");
        let bounds = derive_model_bounds(sql, &ctx);
        assert_eq!(
            *bounds.get("silver.events").unwrap(),
            BoundResult::NotDerivable
        );
    }

    // ---- BL6: monotone-integer partition keys ----

    /// A monotone integer partition key (`batch_id`, no `INTERVAL` anywhere
    /// in the SQL) referenced via `WHERE e.batch_id >= m.batch_id - 5 AND
    /// e.batch_id < m.batch_id` derives `Bounded(batch_id, 5, 0)` — the
    /// bare-integer sibling of [`test_explicit_between_filter`]'s
    /// `INTERVAL`-literal form. The magnitude `5` is stored directly in the
    /// `Seconds` field (an integer key has no elapsed-seconds meaning; see
    /// `extract_gte_lt_bare_integer_bounds`'s doc for why this is the
    /// smallest-diff representation rather than a parallel `BoundResult`
    /// shape).
    #[test]
    fn test_integer_key_bare_constant_offset_form_b() {
        let sql = "SELECT * FROM events e \
                   WHERE e.batch_id >= m.batch_id - 5 AND e.batch_id < m.batch_id";
        let ctx = BoundContext::new().with_source("silver.events", "batch_id");
        let bounds = derive_model_bounds(sql, &ctx);
        let bound = bounds.get("silver.events").unwrap();
        match bound {
            BoundResult::Bounded {
                source_partition_col,
                before,
                after,
            } => {
                assert_eq!(source_partition_col, "batch_id");
                assert_eq!(before.0, 5, "before must be integer magnitude 5");
                assert_eq!(*after, Seconds::ZERO, "after must be zero");
            }
            other => panic!("Expected Bounded(batch_id, 5, 0), got {:?}", other),
        }
    }

    /// The bare-integer extraction fails closed (inert) when the same SQL
    /// statement also contains an `INTERVAL` literal anywhere — mixed
    /// temporal/non-temporal Form B arithmetic in one statement is not a
    /// supported shape. The WHERE clause's bare-integer shift is `200_000`
    /// (deliberately larger, in raw magnitude, than the unrelated Form A
    /// `RANGE BETWEEN INTERVAL '1 day'` frame's 86 400s): if the
    /// bare-integer path fired despite `INTERVAL` appearing elsewhere in the
    /// statement, the merged bound would pick up `max(86400, 200000) =
    /// 200000`. Since it must stay inert, only the interval-derived 1-day
    /// margin survives.
    #[test]
    fn test_integer_key_bare_offset_inert_when_interval_present_elsewhere() {
        let sql = "SELECT id, ts, LAG(x) OVER (PARTITION BY id ORDER BY ts \
                   RANGE BETWEEN INTERVAL '1 day' PRECEDING AND CURRENT ROW) AS prev_x \
                   FROM events e \
                   WHERE e.batch_id >= m.batch_id - 200000 AND e.batch_id < m.batch_id";
        let ctx = BoundContext::new().with_source("silver.events", "batch_id");
        let bounds = derive_model_bounds(sql, &ctx);
        let bound = bounds.get("silver.events").unwrap();
        assert_eq!(
            *bound,
            BoundResult::Bounded {
                source_partition_col: "batch_id".to_string(),
                before: Seconds::days(1),
                after: Seconds::ZERO,
            },
            "bare-integer extraction must stay inert when INTERVAL appears anywhere in the SQL \
             (only the unrelated Form A interval margin should survive)"
        );
    }

    /// Fail-closed regression guard: `Unbounded` still forbids a pushed
    /// filter — `injection_point()` never resolves to `Source` for it.
    #[test]
    fn test_unbounded_still_forbids_pushed_filter_after_unification() {
        let sql = "SELECT id, ts, SUM(x) OVER (PARTITION BY id ORDER BY ts \
                   RANGE BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW) AS running \
                   FROM source_table";
        let ctx = BoundContext::new().with_source("silver.events", "event_date");
        let bounds = derive_model_bounds(sql, &ctx);
        let bound = bounds.get("silver.events").unwrap();
        assert_eq!(*bound, BoundResult::Unbounded);
        assert_eq!(bound.injection_point(), InjectionPoint::OuterClamp);
    }
}
