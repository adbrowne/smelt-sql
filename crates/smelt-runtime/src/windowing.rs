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
//! `docs/specs/batched_models.md` §"Run window vs partition granularity".

use std::collections::HashMap;

use chrono::{Duration, NaiveDate};

use smelt_core::config::TimeseriesConfig;
use smelt_core::{BatchedConfig, Granularity};
use smelt_logical::analysis::window_independence::{window_independence, WindowIndependence};
use smelt_planner::{
    analyze_temporal_dependencies, compute_effective_window, granularity_period_days, BatchSafety,
};

pub use smelt_planner::EffectiveWindow;

use crate::compile::batch_safety_for_model;
use crate::transformer::TimeRange;

/// One batch in an incremental run.
///
/// `partition_start/end` is the unwidened batch window — used for manifest
/// recording. `filter_start/end` is widened by the effective lookback/lookahead —
/// used for both the time-filter WHERE clause and the DELETE partition range
/// (DELETE must cover exactly what the INSERT writes).
#[derive(Debug, Clone, PartialEq)]
pub struct IncrementalBatch {
    pub partition_start: NaiveDate,
    pub partition_end: NaiveDate,
    pub filter_start: NaiveDate,
    pub filter_end: NaiveDate,
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
/// Returns `Err` (fail-closed, `batched_models.md` Constraint 10) when the
/// batch-safety roll-up cannot classify the model (a `NotDerivable` source
/// bound) — the caller must surface this as a hard refusal, never fall back
/// to an approximate chunk shape.
#[allow(clippy::too_many_arguments)]
pub fn compute_incremental_windows(
    timeseries: &TimeseriesConfig,
    _inc_config: &BatchedConfig,
    sql: &str,
    dep_timeseries: &HashMap<String, (Vec<String>, String)>,
    data_latency_days: u32,
    full_range: &TimeRange,
    batch_size_days: Option<u32>,
    per_partition: bool,
) -> Result<IncrementalWindows, String> {
    let start_date = match parse_date(&full_range.start) {
        Ok(d) => d,
        Err(_) => {
            return Ok(IncrementalWindows {
                batches: vec![],
                effective_window: zero_effective_window(),
                wide_batch_warning: None,
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
            })
        }
    };

    if start_date >= end_date {
        return Ok(IncrementalWindows {
            batches: vec![],
            effective_window: zero_effective_window(),
            wide_batch_warning: None,
        });
    }

    // Fail-closed run-window validation (`batched_models.md` §"Run window vs
    // partition granularity"): alignment to `timeseries.granularity`, then
    // `g_run >= g_part` against the partition column's own derived grid unit.
    // Must run before any batching/widening below — a misaligned or
    // sub-`g_part` window is refused outright, never silently coarsened.
    validate_run_window_against_partition_grid(sql, timeseries, start_date, end_date)?;

    // Analyze temporal dependencies to compute effective window.
    let stripped = smelt_parser::strip_frontmatter(sql);
    let temporal_dep = analyze_temporal_dependencies(&stripped);
    let period_days = granularity_period_days(&timeseries.granularity);
    let effective_window = compute_effective_window(&temporal_dep, data_latency_days, period_days);

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
        // approximating a chunk shape (`batched_models.md` Constraint 10).
        let safety = batch_safety_for_model(&stripped, dep_timeseries)?;
        match &safety {
            BatchSafety::FullyBatchSafe => {
                let total_days = (end_date - start_date).num_days() as u32;
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
    let mut batch_start = start_date;
    while batch_start < end_date {
        let batch_end = if use_calendar_stepping {
            calendar_next_partition_start(batch_start, &timeseries.granularity).min(end_date)
        } else {
            (batch_start + Duration::days(batch_days as i64)).min(end_date)
        };
        let filter_start = batch_start - Duration::days(filter_lookback as i64);
        let filter_end = batch_end + Duration::days(filter_lookahead as i64);
        batches.push(IncrementalBatch {
            partition_start: batch_start,
            partition_end: batch_end,
            filter_start,
            filter_end,
        });
        batch_start = batch_end;
    }

    Ok(IncrementalWindows {
        batches,
        effective_window,
        wide_batch_warning,
    })
}

/// Compose F10's window-independence / ordered-execution verdict into the
/// backfill chunker (BL7, `batched_models.md` §"Window independence and
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
///   read, an unbounded/whole-history scan, or an underivable bound) — `Err`,
///   fail-closed, naming the non-convergent self-edge; never silently
///   downgraded to `Ordered` or `WindowIndependent`.
#[allow(clippy::too_many_arguments)]
pub fn compute_incremental_windows_ordered(
    model_name: &str,
    refs: &[String],
    timeseries: &TimeseriesConfig,
    inc_config: &BatchedConfig,
    sql: &str,
    dep_timeseries: &HashMap<String, (Vec<String>, String)>,
    data_latency_days: u32,
    full_range: &TimeRange,
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

    let forced_per_partition = match verdict {
        WindowIndependence::Refused { reason } => {
            return Err(format!(
                "model '{model_name}' is not eligible for batched execution: {reason}"
            ));
        }
        WindowIndependence::Ordered => true,
        WindowIndependence::WindowIndependent => per_partition,
    };

    compute_incremental_windows(
        timeseries,
        inc_config,
        sql,
        dep_timeseries,
        data_latency_days,
        full_range,
        batch_size_days,
        forced_per_partition,
    )
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

    match granularity {
        Granularity::Hour | Granularity::Day => Ok(()),
        Granularity::Week => {
            use chrono::Weekday;
            if start.weekday() != Weekday::Mon {
                return Err(format!(
                    "Run window start ({}) is not aligned to weekly granularity: \
                     start must be a Monday, got {:?}",
                    start,
                    start.weekday()
                ));
            }
            if end.weekday() != Weekday::Mon {
                return Err(format!(
                    "Run window end ({}) is not aligned to weekly granularity: \
                     end must be a Monday, got {:?}",
                    end,
                    end.weekday()
                ));
            }
            if total_days % 7 != 0 {
                return Err(format!(
                    "Run window [{}, {}) is not aligned to weekly granularity: \
                     window spans {} days which is not a multiple of 7",
                    start, end, total_days
                ));
            }
            Ok(())
        }
        Granularity::Month => {
            use chrono::Datelike;
            if start.day() != 1 {
                return Err(format!(
                    "Run window start ({}) is not aligned to monthly granularity: \
                     start must be the 1st of a month",
                    start
                ));
            }
            if end.day() != 1 {
                return Err(format!(
                    "Run window end ({}) is not aligned to monthly granularity: \
                     end must be the 1st of a month",
                    end
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
                     start must be the 1st of a quarter month (Jan, Apr, Jul, Oct)",
                    start
                ));
            }
            if end.day() != 1 || !quarter_months.contains(&end.month()) {
                return Err(format!(
                    "Run window end ({}) is not aligned to quarterly granularity: \
                     end must be the 1st of a quarter month (Jan, Apr, Jul, Oct)",
                    end
                ));
            }
            Ok(())
        }
        Granularity::Year => {
            use chrono::Datelike;
            if start.month() != 1 || start.day() != 1 {
                return Err(format!(
                    "Run window start ({}) is not aligned to yearly granularity: \
                     start must be Jan 1st",
                    start
                ));
            }
            if end.month() != 1 || end.day() != 1 {
                return Err(format!(
                    "Run window end ({}) is not aligned to yearly granularity: \
                     end must be Jan 1st",
                    end
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
/// (`batched_models.md` §"Run window vs partition granularity"; auto-coarsen
/// is a deferred enhancement, see Known Divergences there).
///
/// Called from [`compute_incremental_windows`] — the single real driver both
/// `smelt-cli` and `smelt-ui` runs go through (`execute_project`) — so both
/// consumers get this refusal for free.
pub fn validate_run_window_against_partition_grid(
    sql: &str,
    timeseries: &TimeseriesConfig,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<(), String> {
    validate_run_window_alignment(start, end, &timeseries.granularity)?;

    let Some(g_part) = derive_partition_grid_unit(sql, &timeseries.partition_column) else {
        return Ok(());
    };

    if timeseries.granularity < g_part {
        return Err(format!(
            "run window granularity ({}) is finer than partition column '{}''s derived \
             granularity ({}); the minimum run window for this model is one {}",
            granularity_display(&timeseries.granularity),
            timeseries.partition_column,
            granularity_display(&g_part),
            granularity_display(&g_part),
        ));
    }

    Ok(())
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
