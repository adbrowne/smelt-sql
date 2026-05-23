//! Temporal window computation for incremental execution.
//!
//! Integrates AST-based temporal dependency analysis with upstream data latency
//! to compute the effective filter window for incremental model execution.

use smelt_core::config::TimeseriesConfig;
use smelt_core::{DataLatency, Granularity, IncrementalConfig, SourcesConfig};
use smelt_planner::{
    analyze_temporal_dependencies, compute_effective_window, granularity_period_days,
    EffectiveWindow,
};

use smelt_runtime::TimeRange;

/// The result of computing temporal windows for an incremental model.
#[derive(Debug, Clone)]
pub struct IncrementalWindows {
    /// The filter range — wider than requested to capture context rows.
    /// Used for inject_time_filter() WHERE clause.
    pub filter_range: TimeRange,
    /// The partition range — the originally requested range.
    /// Used for partition DELETE/overwrite.
    pub partition_range: TimeRange,
    /// The computed effective window (for explain output).
    pub effective_window: EffectiveWindow,
}

/// Compute the incremental windows for a model.
///
/// Analyzes the SQL for temporal dependencies and resolves data latency
/// from sources config and model metadata to determine the effective window.
///
/// Returns wider filter range and original partition range.
pub fn compute_incremental_windows(
    sql: &str,
    _config: &IncrementalConfig,
    timeseries: &TimeseriesConfig,
    sources: Option<&SourcesConfig>,
    model_metadata_latency: Option<&DataLatency>,
    requested_range: &TimeRange,
) -> IncrementalWindows {
    // 1. Analyze SQL for temporal dependencies
    let temporal_dep = analyze_temporal_dependencies(sql);

    // 2. Resolve data latency
    let data_latency_days = resolve_data_latency(
        &timeseries.event_time_column,
        sources,
        model_metadata_latency,
    );

    // 3. Compute effective window
    let period_days = granularity_period_days(&timeseries.granularity);
    let effective_window = compute_effective_window(&temporal_dep, data_latency_days, period_days);

    // 4. Compute filter range by adjusting the requested range
    let filter_range = if effective_window.is_unbounded {
        // Unbounded: can't widen meaningfully; the safety check should have caught this.
        // Use the original range and let the safety override handle it.
        requested_range.clone()
    } else {
        adjust_range(
            requested_range,
            effective_window.lookback_days,
            effective_window.lookahead_days,
            &timeseries.granularity,
        )
    };

    IncrementalWindows {
        filter_range,
        partition_range: requested_range.clone(),
        effective_window,
    }
}

/// Resolve data latency for an event_time_column.
///
/// Checks model metadata first (explicit declaration), then falls back to
/// source definitions. Returns 0 if no latency information found.
fn resolve_data_latency(
    event_time_column: &str,
    sources: Option<&SourcesConfig>,
    model_metadata_latency: Option<&DataLatency>,
) -> u32 {
    // Model metadata latency takes priority
    if let Some(latency) = model_metadata_latency {
        return latency.to_days();
    }

    // Search sources for a column matching event_time_column
    if let Some(sources_config) = sources {
        for source in &sources_config.sources {
            for table in &source.tables {
                for col in &table.columns {
                    if col.name == event_time_column {
                        if let Some(ref latency) = col.data_latency {
                            return latency.to_days();
                        }
                    }
                }
            }
        }
    }

    0 // No latency information
}

/// Adjust a time range by subtracting lookback days and adding lookahead days.
fn adjust_range(
    range: &TimeRange,
    lookback_days: u32,
    lookahead_days: u32,
    _granularity: &Granularity,
) -> TimeRange {
    if lookback_days == 0 && lookahead_days == 0 {
        return range.clone();
    }

    use chrono::{Duration, NaiveDate};

    let start = NaiveDate::parse_from_str(&range.start, "%Y-%m-%d").unwrap_or_else(|_| {
        NaiveDate::from_ymd_opt(2000, 1, 1).expect("2000-01-01 is always valid")
    });
    let end = NaiveDate::parse_from_str(&range.end, "%Y-%m-%d").unwrap_or_else(|_| {
        NaiveDate::from_ymd_opt(2099, 12, 31).expect("2099-12-31 is always valid")
    });

    let adjusted_start = start - Duration::days(lookback_days as i64);
    let adjusted_end = end + Duration::days(lookahead_days as i64);

    TimeRange {
        start: adjusted_start.format("%Y-%m-%d").to_string(),
        end: adjusted_end.format("%Y-%m-%d").to_string(),
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
///
/// # Alignment rules by granularity
///
/// | Granularity | Period | Required boundary |
/// |-------------|--------|-------------------|
/// | Day         | 1 day  | any date          |
/// | Week        | 7 days | week_start day (Mon by default) |
/// | Month       | 28–31d | 1st of the month  |
/// | Quarter     | 91–92d | 1st of Jan/Apr/Jul/Oct |
/// | Year        | 365–366d | Jan 1st          |
///
/// For `Day` granularity, any integer-day window is aligned. For `Week`,
/// both endpoints must be a Monday (ISO week start) and the window must be
/// a multiple of 7 days. For `Month`, both endpoints must be the first day
/// of a month. For `Quarter`, both endpoints must be the first day of a
/// quarter. For `Year`, both endpoints must be Jan 1st.
pub fn validate_run_window_alignment(
    start: chrono::NaiveDate,
    end: chrono::NaiveDate,
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
        Granularity::Hour | Granularity::Day => {
            // Any positive-day window is aligned for daily (or sub-daily) granularity.
            Ok(())
        }
        Granularity::Week => {
            // Both endpoints must be Mondays and the window must be a multiple of 7.
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
            // Both endpoints must be the 1st of a month.
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
            // Both endpoints must be the 1st of a quarter (Jan, Apr, Jul, Oct).
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
            // Both endpoints must be Jan 1st.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_adjust_range_no_offset() {
        let range = TimeRange {
            start: "2026-03-20".into(),
            end: "2026-03-22".into(),
        };
        let result = adjust_range(&range, 0, 0, &Granularity::Day);
        assert_eq!(result.start, "2026-03-20");
        assert_eq!(result.end, "2026-03-22");
    }

    #[test]
    fn test_adjust_range_lookback() {
        let range = TimeRange {
            start: "2026-03-20".into(),
            end: "2026-03-22".into(),
        };
        let result = adjust_range(&range, 6, 0, &Granularity::Day);
        assert_eq!(result.start, "2026-03-14");
        assert_eq!(result.end, "2026-03-22");
    }

    #[test]
    fn test_adjust_range_lookahead() {
        let range = TimeRange {
            start: "2026-03-20".into(),
            end: "2026-03-22".into(),
        };
        let result = adjust_range(&range, 0, 1, &Granularity::Day);
        assert_eq!(result.start, "2026-03-20");
        assert_eq!(result.end, "2026-03-23");
    }

    #[test]
    fn test_adjust_range_both() {
        let range = TimeRange {
            start: "2026-03-20".into(),
            end: "2026-03-22".into(),
        };
        let result = adjust_range(&range, 6, 1, &Granularity::Day);
        assert_eq!(result.start, "2026-03-14");
        assert_eq!(result.end, "2026-03-23");
    }

    fn make_ts(event_time_column: &str, partition_column: &str) -> TimeseriesConfig {
        TimeseriesConfig {
            event_time_column: event_time_column.into(),
            partition_column: partition_column.into(),
            granularity: Granularity::Day,
            week_start: None,
        }
    }

    fn make_inc() -> IncrementalConfig {
        IncrementalConfig {
            enabled: true,
            unique_key: vec![],
            safety_overrides: Default::default(),
        }
    }

    #[test]
    fn test_compute_windows_simple_group_by() {
        let sql = "SELECT date_trunc('day', event_time) as d, SUM(amount) FROM events GROUP BY 1";
        let config = make_inc();
        let ts = make_ts("event_time", "d");
        let range = TimeRange {
            start: "2026-03-20".into(),
            end: "2026-03-22".into(),
        };

        let windows = compute_incremental_windows(sql, &config, &ts, None, None, &range);

        // No temporal dependency, no latency → filter range = partition range
        assert_eq!(windows.filter_range.start, "2026-03-20");
        assert_eq!(windows.filter_range.end, "2026-03-22");
        assert_eq!(windows.partition_range.start, "2026-03-20");
        assert_eq!(windows.partition_range.end, "2026-03-22");
    }

    #[test]
    fn test_compute_windows_with_lag() {
        let sql = "SELECT user_id, day, LAG(amount, 3) OVER (ORDER BY day) as prev FROM events";
        let config = make_inc();
        let ts = make_ts("event_time", "day");
        let range = TimeRange {
            start: "2026-03-20".into(),
            end: "2026-03-22".into(),
        };

        let windows = compute_incremental_windows(sql, &config, &ts, None, None, &range);

        // LAG(col, 3) → 3 periods lookback → filter starts 3 days earlier
        assert_eq!(windows.filter_range.start, "2026-03-17");
        assert_eq!(windows.filter_range.end, "2026-03-22");
        // Partition range stays the same
        assert_eq!(windows.partition_range.start, "2026-03-20");
        assert_eq!(windows.partition_range.end, "2026-03-22");
    }

    #[test]
    fn test_compute_windows_with_data_latency() {
        let sql = "SELECT date_trunc('day', event_time) as d, SUM(amount) FROM events GROUP BY 1";
        let config = make_inc();
        let ts = make_ts("event_time", "d");
        let range = TimeRange {
            start: "2026-03-20".into(),
            end: "2026-03-22".into(),
        };
        let latency = DataLatency::parse("3 days").unwrap();

        let windows = compute_incremental_windows(sql, &config, &ts, None, Some(&latency), &range);

        // No temporal dep, but 3-day latency → filter starts 3 days earlier
        assert_eq!(windows.filter_range.start, "2026-03-17");
        assert_eq!(windows.filter_range.end, "2026-03-22");
        assert_eq!(windows.partition_range.start, "2026-03-20");
    }
}
