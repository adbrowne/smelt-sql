//! Temporal window computation for incremental execution.
//!
//! Integrates AST-based temporal dependency analysis with upstream data latency
//! to compute the effective filter window for incremental model execution.

use smelt_core::{DataLatency, Granularity, IncrementalConfig, SourcesConfig};
use smelt_planner::{
    analyze_temporal_dependencies, compute_effective_window, granularity_period_days,
    EffectiveWindow,
};

use crate::transformer::TimeRange;

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
    config: &IncrementalConfig,
    sources: Option<&SourcesConfig>,
    model_metadata_latency: Option<&DataLatency>,
    requested_range: &TimeRange,
) -> IncrementalWindows {
    // 1. Analyze SQL for temporal dependencies
    let temporal_dep = analyze_temporal_dependencies(sql);

    // 2. Resolve data latency
    let data_latency_days =
        resolve_data_latency(&config.event_time_column, sources, model_metadata_latency);

    // 3. Compute effective window
    let period_days = granularity_period_days(&config.granularity);
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
            &config.granularity,
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

    #[test]
    fn test_compute_windows_simple_group_by() {
        let sql = "SELECT date_trunc('day', event_time) as d, SUM(amount) FROM events GROUP BY 1";
        let config = IncrementalConfig {
            enabled: true,
            event_time_column: "event_time".into(),
            partition_column: "d".into(),
            granularity: Granularity::Day,
            unique_key: vec![],
            safety_overrides: Default::default(),
        };
        let range = TimeRange {
            start: "2026-03-20".into(),
            end: "2026-03-22".into(),
        };

        let windows = compute_incremental_windows(sql, &config, None, None, &range);

        // No temporal dependency, no latency → filter range = partition range
        assert_eq!(windows.filter_range.start, "2026-03-20");
        assert_eq!(windows.filter_range.end, "2026-03-22");
        assert_eq!(windows.partition_range.start, "2026-03-20");
        assert_eq!(windows.partition_range.end, "2026-03-22");
    }

    #[test]
    fn test_compute_windows_with_lag() {
        let sql = "SELECT user_id, day, LAG(amount, 3) OVER (ORDER BY day) as prev FROM events";
        let config = IncrementalConfig {
            enabled: true,
            event_time_column: "event_time".into(),
            partition_column: "day".into(),
            granularity: Granularity::Day,
            unique_key: vec![],
            safety_overrides: Default::default(),
        };
        let range = TimeRange {
            start: "2026-03-20".into(),
            end: "2026-03-22".into(),
        };

        let windows = compute_incremental_windows(sql, &config, None, None, &range);

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
        let config = IncrementalConfig {
            enabled: true,
            event_time_column: "event_time".into(),
            partition_column: "d".into(),
            granularity: Granularity::Day,
            unique_key: vec![],
            safety_overrides: Default::default(),
        };
        let range = TimeRange {
            start: "2026-03-20".into(),
            end: "2026-03-22".into(),
        };
        let latency = DataLatency::parse("3 days").unwrap();

        let windows = compute_incremental_windows(sql, &config, None, Some(&latency), &range);

        // No temporal dep, but 3-day latency → filter starts 3 days earlier
        assert_eq!(windows.filter_range.start, "2026-03-17");
        assert_eq!(windows.filter_range.end, "2026-03-22");
        assert_eq!(windows.partition_range.start, "2026-03-20");
    }
}
