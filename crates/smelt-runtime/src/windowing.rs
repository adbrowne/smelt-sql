//! Bound-aware incremental windowing for `execute_project`.
//!
//! Computes (partition_start, partition_end, filter_start, filter_end) for every
//! batch in a run window, incorporating both SQL-inferred temporal dependencies and
//! upstream data latency. This makes the runtime's filter widening identical to the
//! CLI's `compute_incremental_windows` path, closing the gap that previously existed
//! when `execute_project` used only `analyze_batch_safety`'s `context_days`.
//!
//! Also owns `validate_run_window_alignment` (moved from `smelt-cli::temporal`)
//! and `validate_run_window_against_partition_grid` (`g_run >= g_part`, BL5),
//! both called from [`compute_incremental_windows`] so every real
//! `smelt run`/`smelt backfill`/UI run enforces them — see
//! `docs/specs/incremental_shapes.md` §"Run window vs partition granularity".

use std::collections::HashMap;
use std::fmt;

use chrono::{Datelike, Duration, NaiveDate};

use smelt_core::config::TimeseriesConfig;
use smelt_core::{Granularity, PartitionGrainConfig};
use smelt_logical::analysis::source_bounds::{Seconds, Skew};
use smelt_logical::analysis::walk::{model_partition_skew, model_partition_skew_excluding_self};
use smelt_logical::analysis::window_independence::{window_independence, WindowIndependence};
pub use smelt_logical::PartitionAxis;
use smelt_planner::{
    analyze_temporal_dependencies, compute_effective_window, granularity_period_days, BatchSafety,
};

pub use smelt_planner::EffectiveWindow;

use crate::compile::batch_safety_for_model;
use crate::transformer::TimeRange;

/// A single point on a partition axis (`docs/specs/timeseries.md` §Semantics
/// "Partition axis domain") — either a calendar date or a bare integer on a
/// unit-step integer grid. Every [`IncrementalBatch`] bound is one of these;
/// which variant appears is fixed by the model's resolved `partition_column`
/// type, never mixed within one batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PartitionPoint {
    /// A calendar date. `Display` renders exactly `%Y-%m-%d` — byte-identical
    /// to the pre-axis-typing behavior, so calendar-axis output is unchanged.
    Date(NaiveDate),
    /// A bare integer on a unit-step integer grid.
    Integer(i64),
}

impl fmt::Display for PartitionPoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PartitionPoint::Date(d) => write!(f, "{}", d.format("%Y-%m-%d")),
            PartitionPoint::Integer(i) => write!(f, "{i}"),
        }
    }
}

impl PartitionPoint {
    /// This point's axis.
    pub fn axis(&self) -> PartitionAxis {
        match self {
            PartitionPoint::Date(_) => PartitionAxis::Calendar,
            PartitionPoint::Integer(_) => PartitionAxis::Integer,
        }
    }

    /// Render as a SQL literal in this point's domain — quoted for a
    /// calendar date, bare for an integer. No call site consumes this yet
    /// this phase (emission is a later phase's work); it exists so that
    /// later wiring has a single owned rendering to call rather than
    /// re-deriving quoting rules at the call site.
    pub fn sql_literal(&self) -> String {
        match self {
            PartitionPoint::Date(d) => format!("'{}'", d.format("%Y-%m-%d")),
            PartitionPoint::Integer(i) => i.to_string(),
        }
    }

    /// Parse a run-window bound string in the given axis. `Err` (fail-closed)
    /// when the string's form contradicts the axis — a calendar-shaped
    /// string on an integer axis, or vice versa — rather than a silent
    /// coercion between domains (`docs/specs/incremental_shapes.md`
    /// §"The partition grain" rule 8a).
    pub fn parse_in_axis(s: &str, axis: PartitionAxis) -> Result<PartitionPoint, String> {
        match axis {
            PartitionAxis::Calendar => NaiveDate::parse_from_str(s, "%Y-%m-%d")
                .map(PartitionPoint::Date)
                .map_err(|_| {
                    format!(
                        "expected a calendar date (YYYY-MM-DD) for a calendar-axis partition \
                         column, got '{s}'"
                    )
                }),
            PartitionAxis::Integer => s.parse::<i64>().map(PartitionPoint::Integer).map_err(|_| {
                format!("expected a bare integer for a unit-step integer partition axis, got '{s}'")
            }),
        }
    }

    /// Advance to the start of the next partition, one unit-step. On the
    /// calendar axis this is [`calendar_next_partition_start`] (a true
    /// calendar step for Month/Quarter/Year, fixed-day for Day/Week/Hour);
    /// `granularity` is not consulted on the integer axis — the step is
    /// always exactly one unit (`docs/specs/timeseries.md` §"Validation
    /// rules" rule 9).
    pub fn next_partition_start(&self, granularity: &Granularity) -> PartitionPoint {
        match self {
            PartitionPoint::Date(d) => {
                PartitionPoint::Date(calendar_next_partition_start(*d, granularity))
            }
            PartitionPoint::Integer(i) => PartitionPoint::Integer(i + 1),
        }
    }

    /// Advance by `units` partition steps (days for the calendar axis,
    /// integer units for the integer axis).
    pub fn advance_units(&self, units: i64) -> PartitionPoint {
        match self {
            PartitionPoint::Date(d) => PartitionPoint::Date(*d + Duration::days(units)),
            PartitionPoint::Integer(i) => PartitionPoint::Integer(i + units),
        }
    }

    /// The number of partition units between `self` and `other`
    /// (`other - self`; days for the calendar axis, integer difference for
    /// the integer axis). Mismatched variants are unreachable once the axis
    /// is threaded correctly from [`compute_incremental_windows`] — this is
    /// an internal invariant, not a user-facing error path, so it
    /// `debug_assert`s rather than returning `Result`.
    pub fn units_between(&self, other: &PartitionPoint) -> i64 {
        match (self, other) {
            (PartitionPoint::Date(a), PartitionPoint::Date(b)) => (*b - *a).num_days(),
            (PartitionPoint::Integer(a), PartitionPoint::Integer(b)) => b - a,
            _ => {
                debug_assert!(
                    false,
                    "PartitionPoint::units_between called across mismatched axes \
                     ({self:?}, {other:?}) — unreachable once the axis is threaded \
                     correctly from compute_incremental_windows"
                );
                0
            }
        }
    }
}

/// One batch in an incremental run.
///
/// `partition_start/end` is the unwidened batch window — used for manifest
/// recording. `filter_start/end` is widened by the effective lookback/lookahead —
/// used for both the time-filter WHERE clause and the DELETE partition range
/// (DELETE must cover exactly what the INSERT writes). All four fields share
/// one [`PartitionPoint`] variant per batch — the model's resolved
/// `partition_column` axis, never mixed.
#[derive(Debug, Clone, PartialEq)]
pub struct IncrementalBatch {
    pub partition_start: PartitionPoint,
    pub partition_end: PartitionPoint,
    pub filter_start: PartitionPoint,
    pub filter_end: PartitionPoint,
}

/// The full set of incremental batches for a run, plus the effective temporal window.
#[derive(Debug, Clone)]
pub struct IncrementalWindows {
    pub batches: Vec<IncrementalBatch>,
    pub effective_window: EffectiveWindow,
    /// Present when `FullyBatchSafe` causes a single-batch build spanning
    /// many partition periods. The message recommends `--per-partition` or
    /// `--batch-size` to avoid OOM on large backfills.
    pub wide_batch_warning: Option<String>,
    /// The model's own derived partition-column skew bound (`docs/specs/
    /// model_transforms.md` §Semantics "The output window is derived, never
    /// assumed"), as [`model_partition_skew`] reads it off the (expanded)
    /// model SQL — always the value that actually widened `batches` below,
    /// `Skew::ZERO` for an identity model. For an `Ordered` self-referential
    /// model (`docs/specs/incremental_shapes.md` §"Window independence and
    /// self-referential models") this is the skew derived with the
    /// self-edge's own bounding relation excluded as a candidate anchor (see
    /// [`compute_incremental_windows_ordered`]) — never the self-edge's own
    /// bound, which is a distinct, already-proven mechanism, not a partition-
    /// column skew declaration. A consumer that needs to know whether the
    /// transparent-slice fast path (`is_transparent_single_source`) is still
    /// eligible reads this field directly rather than re-deriving it from the
    /// SQL a second time.
    pub skew: Skew,
}

/// Warn when a single FullyBatchSafe batch spans more than this many
/// partition periods. Above this count the single-query footprint can be
/// large enough to OOM a development machine.
const WIDE_BATCH_PERIOD_THRESHOLD: u32 = 30;

/// Compute incremental execution windows for an entire run range.
///
/// Splits `full_range` into batches using the F1 bound-based batch-safety
/// roll-up (`compile::batch_safety_for_model`, unless overridden by
/// `batch_size_days` or `per_partition`), then widens each batch's filter
/// range by the effective temporal window (max of SQL-inferred lookback and
/// `data_latency_days`).
///
/// `dep_timeseries` maps each upstream dependency that carries `timeseries:`
/// to its `(address_segments, partition_column)` — see
/// `compile::build_source_bound_map` for the exact shape/derivation. It
/// drives the batch-safety classification; it does not affect filter
/// widening (that stays SQL/`data_latency_days`-derived, untouched by BL2).
///
/// `data_latency_days` should be resolved by the caller from the model's column
/// metadata (`ColumnMetadata::data_latency` on the event-time column) or a
/// sources configuration.
///
/// Returns `Err` (fail-closed, `incremental_shapes.md` §"Partition-grain constraints" #10) when the
/// batch-safety roll-up cannot classify the model (a `NotDerivable` source
/// bound) — the caller must surface this as a hard refusal, never fall back
/// to an approximate chunk shape.
///
/// Before chunking, `full_range` is itself widened into the **derived output
/// window** (`docs/specs/model_transforms.md` §Semantics "The output window
/// is derived, never assumed"): identity when `timeseries.partition_column`
/// tracks the driving event-time column (`output window = run window`), or
/// skew-inverted `[start − after, end + before)` when a Form B relation
/// anchored on the model's own partition column declares it can skew away
/// from the driving date column ([`model_partition_skew`], `smelt-logical`'s
/// pure leaf classifier — this function only consumes it, per the
/// maintenance-plan-purity rule). The widened range is then aligned outward
/// to `timeseries.granularity` boundaries and chunked exactly as before, so
/// every batch's `partition_start`/`partition_end` — which both the DELETE
/// range and the output clamp key off (`crate::execute`) — already reflect
/// the derived output window; the execute loop needs no window math of its
/// own.
#[allow(clippy::too_many_arguments)]
pub fn compute_incremental_windows(
    timeseries: &TimeseriesConfig,
    inc_config: &PartitionGrainConfig,
    sql: &str,
    dep_timeseries: &HashMap<String, (Vec<String>, String)>,
    data_latency_days: u32,
    full_range: &TimeRange,
    axis: PartitionAxis,
    batch_size_days: Option<u32>,
    per_partition: bool,
) -> Result<IncrementalWindows, String> {
    compute_incremental_windows_impl(
        timeseries,
        inc_config,
        sql,
        dep_timeseries,
        data_latency_days,
        full_range,
        axis,
        batch_size_days,
        per_partition,
        None,
    )
}

/// The shared implementation behind [`compute_incremental_windows`] and
/// [`compute_incremental_windows_ordered`]'s `Ordered` branch.
///
/// `skew_override`, when `Some`, is used in place of deriving
/// [`model_partition_skew`] from `sql` directly — [`compute_incremental_windows_ordered`]'s
/// `Ordered` branch supplies a skew derived with the self-edge's own
/// bounding relation excluded as a candidate anchor (`docs/specs/
/// incremental_shapes.md` §"Window independence and self-referential models": the
/// self-edge is never a skew anchor). Every ordinary call
/// ([`compute_incremental_windows`], and [`compute_incremental_windows_ordered`]'s
/// `WindowIndependent` branch) passes `None`, deriving skew from `sql`
/// unmodified exactly as before. Output-window derivation always runs — a
/// convergent self-edge's own proof establishes correctness for the
/// self-reference itself, but a *separate* genuine Form B relation elsewhere
/// in the same model (anchored on a non-self source) still declares a real
/// partition-column skew, and an `Ordered` model's write window rebases by it
/// exactly like a window-independent model's; ordering then applies over the
/// rebased partitions, strictly sequential either way.
#[allow(clippy::too_many_arguments)]
fn compute_incremental_windows_impl(
    timeseries: &TimeseriesConfig,
    _inc_config: &PartitionGrainConfig,
    sql: &str,
    dep_timeseries: &HashMap<String, (Vec<String>, String)>,
    data_latency_days: u32,
    full_range: &TimeRange,
    axis: PartitionAxis,
    batch_size_days: Option<u32>,
    per_partition: bool,
    skew_override: Option<Skew>,
) -> Result<IncrementalWindows, String> {
    match axis {
        PartitionAxis::Calendar => compute_calendar_windows(
            timeseries,
            sql,
            dep_timeseries,
            data_latency_days,
            full_range,
            batch_size_days,
            per_partition,
            skew_override,
        ),
        PartitionAxis::Integer => compute_integer_windows(
            timeseries,
            sql,
            dep_timeseries,
            data_latency_days,
            full_range,
            batch_size_days,
            per_partition,
            skew_override,
        ),
    }
}

/// The calendar-axis branch of [`compute_incremental_windows_impl`] — the
/// original `chrono::NaiveDate`-based chunker, unchanged in behavior (byte-
/// identical DELETE/scan literals for every existing calendar-axis model;
/// the standing `statement_parity`/`rebuild_dry_run` gates check this).
#[allow(clippy::too_many_arguments)]
fn compute_calendar_windows(
    timeseries: &TimeseriesConfig,
    sql: &str,
    dep_timeseries: &HashMap<String, (Vec<String>, String)>,
    data_latency_days: u32,
    full_range: &TimeRange,
    batch_size_days: Option<u32>,
    per_partition: bool,
    skew_override: Option<Skew>,
) -> Result<IncrementalWindows, String> {
    let start_date = match parse_date(&full_range.start) {
        Ok(d) => d,
        Err(_) => {
            return Ok(IncrementalWindows {
                batches: vec![],
                effective_window: zero_effective_window(),
                wide_batch_warning: None,
                skew: Skew::ZERO,
            })
        }
    };
    let end_date = match parse_date(&full_range.end) {
        Ok(d) => d,
        Err(_) => {
            return Ok(IncrementalWindows {
                batches: vec![],
                effective_window: zero_effective_window(),
                wide_batch_warning: None,
                skew: Skew::ZERO,
            })
        }
    };

    if start_date >= end_date {
        return Ok(IncrementalWindows {
            batches: vec![],
            effective_window: zero_effective_window(),
            wide_batch_warning: None,
            skew: Skew::ZERO,
        });
    }

    // Fail-closed run-window validation (`incremental_shapes.md` §"Run window vs
    // partition granularity"): alignment to `timeseries.granularity`, then
    // `g_run >= g_part` against the partition column's own derived grid unit.
    // Must run before any batching/widening below — a misaligned or
    // sub-`g_part` window is refused outright, never silently coarsened.
    validate_run_window_against_partition_grid(
        sql,
        timeseries,
        PartitionPoint::Date(start_date),
        PartitionPoint::Date(end_date),
    )?;

    // Analyze temporal dependencies to compute effective window.
    let stripped = smelt_parser::strip_frontmatter(sql);
    let temporal_dep = analyze_temporal_dependencies(&stripped);
    let period_days = granularity_period_days(&timeseries.granularity);
    let effective_window = compute_effective_window(&temporal_dep, data_latency_days, period_days);

    // The model's own derived partition-column skew bound — a pure leaf
    // classifier `smelt-logical` owns (`crate::analysis::walk::model_partition_skew`);
    // consumed here, never re-derived (maintenance-plan purity). `skew_override`
    // (set only by `compute_incremental_windows_ordered`'s `Ordered` branch)
    // substitutes a skew derived with the self-edge's own bounding relation
    // excluded as a candidate anchor; every other caller derives directly
    // from `sql` unmodified.
    let skew = skew_override
        .unwrap_or_else(|| model_partition_skew(&stripped, &timeseries.partition_column));

    // Invert the run window through the skew bound
    // (`docs/specs/model_transforms.md` §Semantics "The output window is
    // derived, never assumed"): `after` extends the window earlier (it
    // bounds how far the driving date can sit *after* the partition column,
    // i.e. how far a partition can be dated *before* the data that reaches
    // it), `before` extends it later. Always applied — see
    // `compute_incremental_windows_impl`'s doc comment; `skew` is
    // `Skew::ZERO` (a no-op here) for both an identity model and an `Ordered`
    // model whose only bounding relation is its own self-edge.
    let (output_start, output_end) = {
        let raw_start = start_date - Duration::days(days_ceil(skew.after));
        let raw_end = end_date + Duration::days(days_ceil(skew.before));
        (
            align_output_start(raw_start, &timeseries.granularity),
            align_output_end(raw_end, &timeseries.granularity),
        )
    };

    // Compute filter widening. Unbounded lookback can't be widened; use zero.
    let filter_lookback = if effective_window.is_unbounded {
        0u32
    } else {
        effective_window.lookback_days
    };
    let filter_lookahead = if effective_window.is_unbounded {
        0u32
    } else {
        effective_window.lookahead_days
    };

    let granularity_period = granularity_days(&timeseries.granularity);

    let mut wide_batch_warning: Option<String> = None;

    let batch_days = if per_partition {
        // Calendar granularities use calendar stepping (see tiling loop below).
        // Fixed-day granularities (Day/Week) still use the period in days.
        granularity_period
    } else if let Some(override_days) = batch_size_days {
        override_days.max(1)
    } else {
        // Determine batch chunk size from the F1 bound-based batch-safety
        // roll-up (replaces the legacy text-based `analyze_batch_safety`).
        // Fail-closed: propagate `Err` (a `NotDerivable` source) rather than
        // approximating a chunk shape (`incremental_shapes.md` §"Partition-grain constraints" #10).
        let safety = batch_safety_for_model(&stripped, dep_timeseries)?;
        match &safety {
            BatchSafety::FullyBatchSafe => {
                let total_days = (output_end - output_start).num_days() as u32;
                let period_count = total_days / granularity_period.max(1);
                if period_count > WIDE_BATCH_PERIOD_THRESHOLD {
                    wide_batch_warning = Some(format!(
                        "model spans {} {}{} in a single batch; \
                         consider `--per-partition` or `--batch-size` to reduce memory usage",
                        period_count,
                        granularity_display(&timeseries.granularity),
                        if period_count == 1 { "" } else { "s" },
                    ));
                }
                total_days
            }
            BatchSafety::BoundedSafe { max_chunk_days, .. } => *max_chunk_days,
            BatchSafety::PerPartitionOnly { .. } => granularity_period,
        }
    };

    // Split full_range into partition batches; widen each into a filter batch.
    //
    // When per_partition=true for Month/Quarter/Year we advance by calendar
    // units (1 month, 3 months, 12 months) rather than a fixed day count.
    // A fixed 30-day step drifts off true calendar-month boundaries and grows
    // with the month index — eventually dropping an entire day's worth of rows.
    let use_calendar_stepping = per_partition
        && matches!(
            timeseries.granularity,
            Granularity::Month | Granularity::Quarter | Granularity::Year
        );

    let mut batches = Vec::new();
    let mut batch_start = output_start;
    while batch_start < output_end {
        let batch_end = if use_calendar_stepping {
            calendar_next_partition_start(batch_start, &timeseries.granularity).min(output_end)
        } else {
            (batch_start + Duration::days(batch_days as i64)).min(output_end)
        };
        let filter_start = batch_start - Duration::days(filter_lookback as i64);
        let filter_end = batch_end + Duration::days(filter_lookahead as i64);
        batches.push(IncrementalBatch {
            partition_start: PartitionPoint::Date(batch_start),
            partition_end: PartitionPoint::Date(batch_end),
            filter_start: PartitionPoint::Date(filter_start),
            filter_end: PartitionPoint::Date(filter_end),
        });
        batch_start = batch_end;
    }

    Ok(IncrementalWindows {
        batches,
        effective_window,
        wide_batch_warning,
        skew,
    })
}

/// The integer-axis branch of [`compute_incremental_windows_impl`] — a unit-
/// step integer grid (`docs/specs/timeseries.md` §"Validation rules" rule 9,
/// `docs/specs/incremental_shapes.md` §"The partition grain" rule 8a). One
/// partition is one integer value; the chunk step is one unit (or
/// `--batch-size N` units), never `timeseries.granularity` (that stays the
/// declared propagation grain only). Day-typed widening inputs — a nonzero
/// `data_latency_days`, a nonzero SQL-inferred lookback/lookahead, or a
/// nonzero derived partition-column skew — have no conversion into integer
/// units and are refused fail-closed (`Err`) rather than silently zeroed or
/// coerced 1:1 into "N units".
#[allow(clippy::too_many_arguments)]
fn compute_integer_windows(
    timeseries: &TimeseriesConfig,
    sql: &str,
    dep_timeseries: &HashMap<String, (Vec<String>, String)>,
    data_latency_days: u32,
    full_range: &TimeRange,
    batch_size_days: Option<u32>,
    per_partition: bool,
    skew_override: Option<Skew>,
) -> Result<IncrementalWindows, String> {
    let start_i = match full_range.start.parse::<i64>() {
        Ok(v) => v,
        Err(_) => {
            return Ok(IncrementalWindows {
                batches: vec![],
                effective_window: zero_effective_window(),
                wide_batch_warning: None,
                skew: Skew::ZERO,
            })
        }
    };
    let end_i = match full_range.end.parse::<i64>() {
        Ok(v) => v,
        Err(_) => {
            return Ok(IncrementalWindows {
                batches: vec![],
                effective_window: zero_effective_window(),
                wide_batch_warning: None,
                skew: Skew::ZERO,
            })
        }
    };

    if start_i >= end_i {
        return Ok(IncrementalWindows {
            batches: vec![],
            effective_window: zero_effective_window(),
            wide_batch_warning: None,
            skew: Skew::ZERO,
        });
    }

    // The only run-window validity requirement on an integer axis is a
    // positive span — no granularity-boundary alignment, no `g_run >=
    // g_part` comparison (`derive_partition_grid_unit` is a calendar-only
    // concept and is not consulted here).
    validate_run_window_against_partition_grid(
        sql,
        timeseries,
        PartitionPoint::Integer(start_i),
        PartitionPoint::Integer(end_i),
    )?;

    let stripped = smelt_parser::strip_frontmatter(sql);
    let temporal_dep = analyze_temporal_dependencies(&stripped);
    // `period_days` only scales a day-domain SQL-inferred bound into whole
    // granularity periods — on an integer axis any nonzero result here is
    // refused below regardless of scaling, so the calendar period constant
    // is fine to reuse as-is.
    let period_days = granularity_period_days(&timeseries.granularity);
    let effective_window = compute_effective_window(&temporal_dep, data_latency_days, period_days);

    let skew = skew_override
        .unwrap_or_else(|| model_partition_skew(&stripped, &timeseries.partition_column));

    // Fail-closed day-typed-widening refusal (`docs/specs/incremental_shapes.md`
    // §"The partition grain" rule 8a) — never coerced 1:1 into "N units".
    if data_latency_days != 0 {
        return Err(format!(
            "partition column '{}' resolves to an integer partition axis, but a nonzero \
             data_latency ({data_latency_days} day(s)) is declared on the event-time column; \
             day-typed widening has no conversion into an integer axis and is refused rather \
             than coerced into partition units",
            timeseries.partition_column,
        ));
    }
    if !effective_window.is_unbounded
        && (effective_window.lookback_days != 0 || effective_window.lookahead_days != 0)
    {
        return Err(format!(
            "partition column '{}' resolves to an integer partition axis, but the model's SQL \
             implies a nonzero seconds/day-domain lookback or lookahead ({} lookback day(s), \
             {} lookahead day(s)); day-typed widening has no conversion into an integer axis \
             and is refused rather than coerced into partition units",
            timeseries.partition_column,
            effective_window.lookback_days,
            effective_window.lookahead_days,
        ));
    }
    if skew.before.0 != 0 || skew.after.0 != 0 {
        return Err(format!(
            "partition column '{}' resolves to an integer partition axis, but the model \
             declares a nonzero partition-column skew (before={}s, after={}s); day-typed \
             widening has no conversion into an integer axis and is refused rather than \
             coerced into partition units",
            timeseries.partition_column, skew.before.0, skew.after.0,
        ));
    }

    let mut wide_batch_warning: Option<String> = None;

    let batch_units: i64 = if per_partition {
        1
    } else if let Some(override_units) = batch_size_days {
        override_units.max(1) as i64
    } else {
        // Mirrors the calendar branch's batch-safety-derived chunk sizing —
        // `n` is rendered in the partition column's own unit
        // (`docs/specs/incremental_shapes.md` §"Batch safety classification"),
        // which on this axis is already "partition units", no conversion
        // needed.
        let safety = batch_safety_for_model(&stripped, dep_timeseries)?;
        match &safety {
            BatchSafety::FullyBatchSafe => {
                let total_units = end_i - start_i;
                if total_units > WIDE_BATCH_PERIOD_THRESHOLD as i64 {
                    wide_batch_warning = Some(format!(
                        "model spans {} partition unit{} in a single batch; \
                         consider `--per-partition` or `--batch-size` to reduce memory usage",
                        total_units,
                        if total_units == 1 { "" } else { "s" },
                    ));
                }
                total_units
            }
            BatchSafety::BoundedSafe { max_chunk_days, .. } => *max_chunk_days as i64,
            BatchSafety::PerPartitionOnly { .. } => 1,
        }
    }
    .max(1);

    let mut batches = Vec::new();
    let mut batch_start = start_i;
    while batch_start < end_i {
        let batch_end = (batch_start + batch_units).min(end_i);
        batches.push(IncrementalBatch {
            partition_start: PartitionPoint::Integer(batch_start),
            partition_end: PartitionPoint::Integer(batch_end),
            filter_start: PartitionPoint::Integer(batch_start),
            filter_end: PartitionPoint::Integer(batch_end),
        });
        batch_start = batch_end;
    }

    Ok(IncrementalWindows {
        batches,
        effective_window,
        wide_batch_warning,
        skew,
    })
}

/// Compose F10's window-independence / ordered-execution verdict into the
/// backfill chunker (BL7, `incremental_shapes.md` §"Window independence and
/// self-referential models").
///
/// `model_name` and `refs` identify a self-edge exactly as
/// [`window_independence`] expects (`refs` is this model's own `smelt.ref()`
/// list; a self-edge is `refs` containing `model_name`). Three outcomes:
///
/// - **`WindowIndependent`** (no self-edge, the default) — delegates to
///   [`compute_incremental_windows`] unchanged; the model keeps its ordinary
///   batch-safety-derived auto-chunking (or the caller's `per_partition`/
///   `batch_size_days` override).
/// - **`Ordered`** (a self-edge proven to converge partition-by-partition) —
///   forces strictly-sequential single-partition-per-batch execution
///   regardless of the batch-safety class *or* any `per_partition`/
///   `batch_size_days` override: a self-referential window reads its own
///   immediately-prior partition's committed output, so lumping multiple
///   partitions into one wide batch (`FullyBatchSafe`/`BoundedSafe`'s
///   multi-partition chunks) would read rows that do not exist yet — never
///   safe to widen for an ordered model.
/// - **`Refused`** — the self-edge does not provably converge (a forward
///   read, a same-partition/circular read with no backward reach, an
///   unbounded/whole-history scan, or an underivable bound) — `Err`,
///   fail-closed, naming the non-convergent self-edge; never silently
///   downgraded to `Ordered` or `WindowIndependent`.
#[allow(clippy::too_many_arguments)]
pub fn compute_incremental_windows_ordered(
    model_name: &str,
    refs: &[String],
    timeseries: &TimeseriesConfig,
    inc_config: &PartitionGrainConfig,
    sql: &str,
    dep_timeseries: &HashMap<String, (Vec<String>, String)>,
    data_latency_days: u32,
    full_range: &TimeRange,
    axis: PartitionAxis,
    batch_size_days: Option<u32>,
    per_partition: bool,
) -> Result<IncrementalWindows, String> {
    let stripped = smelt_parser::strip_frontmatter(sql);
    let verdict = window_independence(
        model_name,
        refs,
        Some(&timeseries.partition_column),
        &stripped,
    );

    let forced_per_partition = match &verdict {
        WindowIndependence::Refused { reason } => {
            return Err(format!(
                "model '{model_name}' is not eligible for batched execution: {reason}"
            ));
        }
        WindowIndependence::Ordered => true,
        WindowIndependence::WindowIndependent => per_partition,
    };

    // `Ordered` composes with output-window derivation exactly like
    // `WindowIndependent` (`compute_incremental_windows_impl`'s doc comment)
    // — but the self-edge itself is never a skew anchor: its own bounding
    // relation reads, to the anchor scan, identically to a genuine
    // partition-column skew declaration whenever the self-referenced table's
    // column shares the model's own `partition_column` name. The exclusion
    // is owned by `smelt-logical`'s shared composition walk
    // (`model_partition_skew_excluding_self`, resolved per scope — a genuine
    // Form B relation anchored on a non-self source still contributes);
    // this function only consumes the derived value (maintenance-plan
    // purity, property-composition-walk rule).
    let skew_override = matches!(verdict, WindowIndependence::Ordered).then(|| {
        model_partition_skew_excluding_self(
            &stripped,
            &timeseries.partition_column,
            Some(model_name),
        )
    });

    compute_incremental_windows_impl(
        timeseries,
        inc_config,
        sql,
        dep_timeseries,
        data_latency_days,
        full_range,
        axis,
        batch_size_days,
        forced_per_partition,
        skew_override,
    )
    // Fail-loud refusals from the impl (domain mismatch, day-typed widening
    // on an integer axis) name the offending column/input but not the
    // model — add that context here, the one place in the windowing module
    // that always has a model name to hand.
    .map_err(|e| format!("model '{model_name}': {e}"))
}

/// Validate that a run window `[start, end)` is aligned to the model's granularity.
///
/// The window must satisfy:
/// 1. `end > start` (positive window).
/// 2. Both `start` and `end` fall on granularity boundaries.
/// 3. `(end - start)` is an integer multiple of the granularity period.
///
/// Returns `Err` with a diagnostic message if the window is misaligned.
pub fn validate_run_window_alignment(
    start: NaiveDate,
    end: NaiveDate,
    granularity: &Granularity,
) -> Result<(), String> {
    use chrono::Datelike;

    if end <= start {
        return Err(format!(
            "Run window end ({}) must be after start ({})",
            end, start
        ));
    }

    let total_days = (end - start).num_days();

    // Every misalignment message below names the coarsened `[start, end)` pair
    // that would be accepted (criterion 2), computed through the same
    // `coarsen_window_to`/`align_output_*` rounding this module already uses
    // to derive skew-widened output windows — one rounding rule, never
    // restated per error arm.
    let suggested = || {
        let (s, e) = coarsen_window_to(start, end, granularity);
        suggested_window_flags(s, e)
    };

    match granularity {
        Granularity::Hour | Granularity::Day => Ok(()),
        Granularity::Week => {
            use chrono::Weekday;
            if start.weekday() != Weekday::Mon {
                return Err(format!(
                    "Run window start ({}) is not aligned to weekly granularity: \
                     start must be a Monday, got {:?} — the minimum accepted run window is `{}`",
                    start,
                    start.weekday(),
                    suggested(),
                ));
            }
            if end.weekday() != Weekday::Mon {
                return Err(format!(
                    "Run window end ({}) is not aligned to weekly granularity: \
                     end must be a Monday, got {:?} — the minimum accepted run window is `{}`",
                    end,
                    end.weekday(),
                    suggested(),
                ));
            }
            if total_days % 7 != 0 {
                return Err(format!(
                    "Run window [{}, {}) is not aligned to weekly granularity: \
                     window spans {} days which is not a multiple of 7 — the minimum accepted \
                     run window is `{}`",
                    start,
                    end,
                    total_days,
                    suggested(),
                ));
            }
            Ok(())
        }
        Granularity::Month => {
            use chrono::Datelike;
            if start.day() != 1 {
                return Err(format!(
                    "Run window start ({}) is not aligned to monthly granularity: \
                     start must be the 1st of a month — the minimum accepted run window is `{}`",
                    start,
                    suggested(),
                ));
            }
            if end.day() != 1 {
                return Err(format!(
                    "Run window end ({}) is not aligned to monthly granularity: \
                     end must be the 1st of a month — the minimum accepted run window is `{}`",
                    end,
                    suggested(),
                ));
            }
            Ok(())
        }
        Granularity::Quarter => {
            use chrono::Datelike;
            let quarter_months = [1u32, 4, 7, 10];
            if start.day() != 1 || !quarter_months.contains(&start.month()) {
                return Err(format!(
                    "Run window start ({}) is not aligned to quarterly granularity: \
                     start must be the 1st of a quarter month (Jan, Apr, Jul, Oct) — the minimum \
                     accepted run window is `{}`",
                    start,
                    suggested(),
                ));
            }
            if end.day() != 1 || !quarter_months.contains(&end.month()) {
                return Err(format!(
                    "Run window end ({}) is not aligned to quarterly granularity: \
                     end must be the 1st of a quarter month (Jan, Apr, Jul, Oct) — the minimum \
                     accepted run window is `{}`",
                    end,
                    suggested(),
                ));
            }
            Ok(())
        }
        Granularity::Year => {
            use chrono::Datelike;
            if start.month() != 1 || start.day() != 1 {
                return Err(format!(
                    "Run window start ({}) is not aligned to yearly granularity: \
                     start must be Jan 1st — the minimum accepted run window is `{}`",
                    start,
                    suggested(),
                ));
            }
            if end.month() != 1 || end.day() != 1 {
                return Err(format!(
                    "Run window end ({}) is not aligned to yearly granularity: \
                     end must be Jan 1st — the minimum accepted run window is `{}`",
                    end,
                    suggested(),
                ));
            }
            Ok(())
        }
    }
}

/// Derive the truncation/grid granularity (`g_part`) of `partition_column`'s
/// SELECT-list projection expression in `sql`, if classifiable.
///
/// Locates the projection via `smelt_logical::analyze_select` (the shared
/// select-item classifier — no second parse) and reads its truncation unit
/// off the same structural monotonicity trace `trace_event_time` uses
/// (`smelt_logical::analysis::monotonicity::classify_truncation_grid_unit`).
/// Returns `None` — undecidable, not a positive disproof — when the
/// projection can't be found or its shape doesn't resolve to a known grid
/// unit; callers must fail open (skip the `g_run >= g_part` comparison) in
/// that case, matching the trace's existing `Undecidable` posture.
fn derive_partition_grid_unit(sql: &str, partition_column: &str) -> Option<Granularity> {
    let analysis = smelt_logical::analyze_select(sql)?;
    let expr = analysis.items.into_iter().find_map(|item| match item {
        smelt_logical::SelectItemKind::CountDistinct { alias, expr, .. }
        | smelt_logical::SelectItemKind::OtherAggregate { alias, expr, .. }
        | smelt_logical::SelectItemKind::GroupByKey { alias, expr, .. }
            if alias == partition_column =>
        {
            Some(expr)
        }
        _ => None,
    })?;
    smelt_logical::analysis::monotonicity::classify_truncation_grid_unit(&expr)
}

/// Validate the run window `[start, end)` against the model's derived
/// partition granularity (`g_part`), in addition to the ordinary
/// alignment-to-declared-granularity check ([`validate_run_window_alignment`]).
///
/// Two checks, in order:
/// 1. The window aligns to `timeseries.granularity` boundaries (existing
///    check, unconditional).
/// 2. `timeseries.granularity` (`g_run`) is at least as coarse as the
///    partition column's derived grid unit (`g_part`) — i.e. `g_run >=
///    g_part` under `Granularity`'s increasing-coarseness ordering. When
///    `g_part` can't be derived (an opaque projection, an unrecognised
///    truncation unit), this second check is skipped — fail open, since an
///    undecidable `g_part` is not a positive disproof.
///
/// Ships hard-validation only: a sub-`g_part` run window is rejected with a
/// message naming the minimum window, never silently coarsened to fit
/// (`incremental_shapes.md` §"Run window vs partition granularity"; auto-coarsen
/// is a deferred enhancement, see Known Divergences there).
///
/// Called from [`compute_incremental_windows`] — the single real driver both
/// `smelt-cli` and `smelt-ui` runs go through (`execute_project`) — so both
/// consumers get this refusal for free.
///
/// Branches on the axis carried by `start`/`end` themselves — a
/// [`PartitionPoint::Date`] pair runs the calendar-axis checks above
/// unchanged; a [`PartitionPoint::Integer`] pair runs only the positive-span
/// check (`docs/specs/timeseries.md` §"Validation rules" rule 9): no
/// granularity-boundary alignment, and [`derive_partition_grid_unit`] (a
/// calendar-only concept) is not consulted. A mismatched pair (one `Date`,
/// one `Integer`) is unreachable in production — both call sites always
/// parse both bounds through the same resolved axis — but is refused rather
/// than panicking, naming both domains, as a defensive belt for a caller
/// that constructs `PartitionPoint`s by hand (as the direct unit tests do).
pub fn validate_run_window_against_partition_grid(
    sql: &str,
    timeseries: &TimeseriesConfig,
    start: PartitionPoint,
    end: PartitionPoint,
) -> Result<(), String> {
    match (start, end) {
        (PartitionPoint::Date(start), PartitionPoint::Date(end)) => {
            validate_run_window_alignment(start, end, &timeseries.granularity)?;

            let Some(g_part) = derive_partition_grid_unit(sql, &timeseries.partition_column) else {
                return Ok(());
            };

            if timeseries.granularity < g_part {
                // Config-level refusal (`docs/specs/incremental_shapes.md`
                // §"Run window vs partition granularity"): no run window fixes
                // this, only a `timeseries.granularity` edit does. The
                // covering window at `g_part` is named as context only — it
                // must not read as "re-run with this and it works", since
                // re-running with it still leaves `g_run` unchanged.
                let (cov_start, cov_end) = coarsen_window_to(start, end, &g_part);
                return Err(format!(
                    "run window granularity ({}) is finer than partition column '{}''s derived \
                     granularity ({}); declare `timeseries.granularity: {}` on this model to fix \
                     this. For context only (this alone will not make the run pass), the window \
                     covering this run at {} granularity is `{}`",
                    granularity_display(&timeseries.granularity),
                    timeseries.partition_column,
                    granularity_display(&g_part),
                    granularity_display(&g_part),
                    granularity_display(&g_part),
                    suggested_window_flags(cov_start, cov_end),
                ));
            }

            // Window-level refusal: `g_run >= g_part` (checked above) does not
            // by itself guarantee the *window's own bounds* land on `g_part`
            // boundaries — e.g. a monthly `g_run` window over a weekly
            // `g_part` grid, since a month start is not always a Monday. Named
            // with the coarsened pair that would be accepted (criterion 2);
            // re-running with exactly that pair succeeds.
            if !is_grid_aligned(start, &g_part) || !is_grid_aligned(end, &g_part) {
                let (coarse_start, coarse_end) = coarsen_window_to(start, end, &g_part);
                return Err(format!(
                    "run window [{}, {}) is not aligned to partition column '{}''s derived \
                     granularity ({}); the minimum accepted run window aligned to that grid is \
                     `{}`",
                    start,
                    end,
                    timeseries.partition_column,
                    granularity_display(&g_part),
                    suggested_window_flags(coarse_start, coarse_end),
                ));
            }

            Ok(())
        }
        (PartitionPoint::Integer(start), PartitionPoint::Integer(end)) => {
            if end <= start {
                return Err(format!(
                    "Run window end ({end}) must be after start ({start})"
                ));
            }
            Ok(())
        }
        (start, end) => Err(format!(
            "run window bounds for partition column '{}' must both be in the same domain \
             (a calendar date or a bare integer) — got {} ({:?}) and {} ({:?})",
            timeseries.partition_column,
            start,
            start.axis(),
            end,
            end.axis(),
        )),
    }
}

/// The axis implied by a run-window bound literal's own form — a calendar
/// date (`YYYY-MM-DD`) or a bare integer. Used as a fail-open fallback
/// wherever a model's `partition_column` type can't be resolved (no schema
/// handle to read it from): `execute.rs::build_model_plans` when
/// `resolved_model_schema` can't classify the column, and `smelt explain`
/// (no Salsa handle at all) unconditionally. `Calendar` when `s` is `None`
/// or matches neither form — the historical default, so an absent/malformed
/// bound never flips a calendar-axis model to the integer branch.
pub fn axis_implied_by_literal_form(s: Option<&str>) -> PartitionAxis {
    s.and_then(|s| {
        if NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok() {
            Some(PartitionAxis::Calendar)
        } else if s.parse::<i64>().is_ok() {
            Some(PartitionAxis::Integer)
        } else {
            None
        }
    })
    .unwrap_or(PartitionAxis::Calendar)
}

/// Advance `current` by exactly one partition step for the given granularity.
///
/// For Month/Quarter/Year this is a true calendar step (chrono `Months`), not a
/// fixed-day count — necessary because months have varying lengths.  Day/Week
/// continue to use fixed 1-day / 7-day steps.
fn calendar_next_partition_start(current: NaiveDate, granularity: &Granularity) -> NaiveDate {
    use chrono::Months;
    match granularity {
        Granularity::Month => current + Months::new(1),
        Granularity::Quarter => current + Months::new(3),
        Granularity::Year => current + Months::new(12),
        Granularity::Week => current + Duration::days(7),
        Granularity::Day | Granularity::Hour => current + Duration::days(1),
    }
}

fn parse_date(s: &str) -> Result<NaiveDate, chrono::ParseError> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d")
}

/// Round a [`Seconds`] duration up to a whole number of days — the skew
/// grammar's INTERVAL literals are whole-day (`docs/specs/model_transforms.md`
/// examples are all `INTERVAL '1 day'`-shaped), but rounding up rather than
/// truncating keeps any sub-day remainder from silently narrowing the
/// derived output window.
fn days_ceil(s: Seconds) -> i64 {
    s.0.div_ceil(86400) as i64
}

/// Floor `date` down to the nearest `granularity` boundary at or before it —
/// the "outward" (earlier) side of aligning the skew-derived output window
/// to `timeseries.granularity` (`docs/specs/model_transforms.md` §Semantics
/// "The output window is derived, never assumed"). Mirrors the boundary
/// rules [`validate_run_window_alignment`] enforces for a *declared* window,
/// but computes rather than validates.
fn align_output_start(date: NaiveDate, granularity: &Granularity) -> NaiveDate {
    match granularity {
        Granularity::Hour | Granularity::Day => date,
        Granularity::Week => {
            let days_since_monday = date.weekday().num_days_from_monday();
            date - Duration::days(days_since_monday as i64)
        }
        Granularity::Month => date.with_day(1).unwrap_or(date),
        Granularity::Quarter => {
            let quarter_start_month = ((date.month() - 1) / 3) * 3 + 1;
            NaiveDate::from_ymd_opt(date.year(), quarter_start_month, 1).unwrap_or(date)
        }
        Granularity::Year => NaiveDate::from_ymd_opt(date.year(), 1, 1).unwrap_or(date),
    }
}

/// Ceil `date` up to the nearest `granularity` boundary at or after it — the
/// "outward" (later) side of aligning the skew-derived output window. If
/// `date` already falls exactly on a boundary, it is returned unchanged (the
/// common, zero-skew case never nudges an already-aligned run-window edge).
fn align_output_end(date: NaiveDate, granularity: &Granularity) -> NaiveDate {
    let floor = align_output_start(date, granularity);
    if floor == date {
        date
    } else {
        calendar_next_partition_start(floor, granularity)
    }
}

/// Round `[start, end)` outward to `unit`-aligned boundaries — the coarsened
/// pair a run-window refusal suggests re-running with (`docs/specs/
/// incremental_shapes.md` §"Run window vs partition granularity"). Reuses
/// [`align_output_start`]/[`align_output_end`], the same rounding this module
/// already uses to derive skew-widened output windows, so a suggested run
/// window and a derived output window can never spell different rounding
/// rules for the same granularity. An already-aligned pair is returned
/// unchanged.
fn coarsen_window_to(
    start: NaiveDate,
    end: NaiveDate,
    unit: &Granularity,
) -> (NaiveDate, NaiveDate) {
    (align_output_start(start, unit), align_output_end(end, unit))
}

/// Render a `[start, end)` pair as the exact CLI flags that would reproduce
/// it (`--event-time-start YYYY-MM-DD --event-time-end YYYY-MM-DD`) — the one
/// place a suggested run window is formatted, so every refusal message
/// spells the same flag names.
fn suggested_window_flags(start: NaiveDate, end: NaiveDate) -> String {
    format!(
        "--event-time-start {} --event-time-end {}",
        start.format("%Y-%m-%d"),
        end.format("%Y-%m-%d"),
    )
}

/// Whether `date` itself falls exactly on a `unit` grid boundary — the
/// window-level `g_part`-alignment check's predicate
/// ([`validate_run_window_against_partition_grid`]).
fn is_grid_aligned(date: NaiveDate, unit: &Granularity) -> bool {
    align_output_start(date, unit) == date
}

fn granularity_display(g: &Granularity) -> &'static str {
    match g {
        Granularity::Hour => "hour",
        Granularity::Day => "day",
        Granularity::Week => "week",
        Granularity::Month => "month",
        Granularity::Quarter => "quarter",
        Granularity::Year => "year",
    }
}

fn granularity_days(g: &Granularity) -> u32 {
    match g {
        Granularity::Hour => 1,
        Granularity::Day => 1,
        Granularity::Week => 7,
        Granularity::Month => 30,
        Granularity::Quarter => 91,
        Granularity::Year => 365,
    }
}

fn zero_effective_window() -> EffectiveWindow {
    EffectiveWindow {
        lookback_days: 0,
        lookahead_days: 0,
        is_unbounded: false,
        explanation: String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coarsen_window_to_grid_floors_start_and_ceils_end() {
        let start = NaiveDate::from_ymd_opt(2024, 12, 5).unwrap();
        let end = NaiveDate::from_ymd_opt(2024, 12, 20).unwrap();
        let (coarse_start, coarse_end) = coarsen_window_to(start, end, &Granularity::Month);
        assert_eq!(coarse_start, NaiveDate::from_ymd_opt(2024, 12, 1).unwrap());
        assert_eq!(coarse_end, NaiveDate::from_ymd_opt(2025, 1, 1).unwrap());
    }

    #[test]
    fn coarsen_window_to_grid_leaves_an_already_aligned_pair_unchanged() {
        let start = NaiveDate::from_ymd_opt(2024, 12, 1).unwrap();
        let end = NaiveDate::from_ymd_opt(2025, 1, 1).unwrap();
        let (coarse_start, coarse_end) = coarsen_window_to(start, end, &Granularity::Month);
        assert_eq!(coarse_start, start);
        assert_eq!(coarse_end, end);
    }
}
