//! Bound-aware incremental windowing for `execute_project`.
//!
//! Computes (partition_start, partition_end, filter_start, filter_end) for every
//! batch in a run window, incorporating both SQL-inferred temporal dependencies and
//! upstream data latency. This makes the runtime's filter widening identical to the
//! CLI's `compute_incremental_windows` path, closing the gap that previously existed
//! when `execute_project` used only `analyze_batch_safety`'s `context_days`.
//!
//! Also owns `validate_run_window_alignment` (moved from `smelt-cli::temporal`).

use chrono::{Duration, NaiveDate};

use smelt_core::config::TimeseriesConfig;
use smelt_core::{Granularity, IncrementalConfig};
use smelt_planner::{
    analyze_batch_safety, analyze_temporal_dependencies, compute_effective_window,
    granularity_period_days, BatchSafety, ModelInfo,
};

pub use smelt_planner::EffectiveWindow;

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
/// Splits `full_range` into batches using `analyze_batch_safety` (unless overridden
/// by `batch_size_days` or `per_partition`), then widens each batch's filter range
/// by the effective temporal window (max of SQL-inferred lookback and `data_latency_days`).
///
/// `data_latency_days` should be resolved by the caller from the model's column
/// metadata (`ColumnMetadata::data_latency` on the event-time column) or a
/// sources configuration.
pub fn compute_incremental_windows(
    timeseries: &TimeseriesConfig,
    inc_config: &IncrementalConfig,
    sql: &str,
    data_latency_days: u32,
    full_range: &TimeRange,
    batch_size_days: Option<u32>,
    per_partition: bool,
) -> IncrementalWindows {
    let start_date = match parse_date(&full_range.start) {
        Ok(d) => d,
        Err(_) => {
            return IncrementalWindows {
                batches: vec![],
                effective_window: zero_effective_window(),
                wide_batch_warning: None,
            }
        }
    };
    let end_date = match parse_date(&full_range.end) {
        Ok(d) => d,
        Err(_) => {
            return IncrementalWindows {
                batches: vec![],
                effective_window: zero_effective_window(),
                wide_batch_warning: None,
            }
        }
    };

    if start_date >= end_date {
        return IncrementalWindows {
            batches: vec![],
            effective_window: zero_effective_window(),
            wide_batch_warning: None,
        };
    }

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

    // Determine batch chunk size using analyze_batch_safety (respects SQL patterns).
    let model_info = ModelInfo {
        name: String::new(),
        sql: sql.to_string(),
        refs: vec![],
        incremental_config: Some(inc_config.clone()),
        timeseries_config: Some(timeseries.clone()),
    };
    let safety = analyze_batch_safety(&model_info);
    let granularity_period = granularity_days(&timeseries.granularity);

    let mut wide_batch_warning: Option<String> = None;

    let batch_days = if per_partition {
        // Calendar granularities use calendar stepping (see tiling loop below).
        // Fixed-day granularities (Day/Week) still use the period in days.
        granularity_period
    } else if let Some(override_days) = batch_size_days {
        override_days.max(1)
    } else {
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

    IncrementalWindows {
        batches,
        effective_window,
        wide_batch_warning,
    }
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
