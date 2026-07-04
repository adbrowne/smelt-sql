//! Temporal window computation — shim over `smelt_runtime::windowing`.
//!
//! The shared logic (batch production, filter widening, window alignment) lives in
//! `smelt-runtime` so both CLI and UI consume the same implementation. This module
//! re-exports the canonical types and provides a backward-compat adapter for the
//! backfill path that still calls the old single-window API.

pub use smelt_runtime::windowing::{
    compute_incremental_windows, validate_run_window_alignment, EffectiveWindow, IncrementalBatch,
    IncrementalWindows,
};

use smelt_core::config::TimeseriesConfig;
use smelt_core::{BatchedConfig, DataLatency, SourcesConfig};
use smelt_planner::{
    analyze_temporal_dependencies, compute_effective_window, granularity_period_days,
};
use smelt_runtime::TimeRange;

/// Single-window adapter used by the backfill path.
///
/// Computes the filter widening for a single (already-batched) range by analyzing
/// temporal dependencies and resolving data latency from sources or model metadata.
/// The backfill command iterates batches externally and calls this once per batch.
///
/// TODO (Phase 4): Remove when backfill.rs is migrated to `execute_project`.
pub struct SingleIncrementalWindow {
    pub filter_range: TimeRange,
    pub partition_range: TimeRange,
    pub effective_window: EffectiveWindow,
}

/// Compute the filter window for a **single** pre-batched range.
///
/// Used by the backfill path which iterates batch windows externally.
/// Resolves data latency from sources config and model metadata, then widens
/// the requested range by the derived lookback/lookahead.
pub fn compute_single_window(
    sql: &str,
    _config: &BatchedConfig,
    timeseries: &TimeseriesConfig,
    sources: Option<&SourcesConfig>,
    model_metadata_latency: Option<&DataLatency>,
    requested_range: &TimeRange,
) -> SingleIncrementalWindow {
    let temporal_dep = analyze_temporal_dependencies(sql);
    let data_latency_days = resolve_data_latency(
        &timeseries.event_time_column,
        sources,
        model_metadata_latency,
    );
    let period_days = granularity_period_days(&timeseries.granularity);
    let effective_window = compute_effective_window(&temporal_dep, data_latency_days, period_days);

    let filter_range = if effective_window.is_unbounded {
        requested_range.clone()
    } else {
        adjust_range(
            requested_range,
            effective_window.lookback_days,
            effective_window.lookahead_days,
        )
    };

    SingleIncrementalWindow {
        filter_range,
        partition_range: requested_range.clone(),
        effective_window,
    }
}

fn resolve_data_latency(
    event_time_column: &str,
    sources: Option<&SourcesConfig>,
    model_metadata_latency: Option<&DataLatency>,
) -> u32 {
    if let Some(latency) = model_metadata_latency {
        return latency.to_days();
    }
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
    0
}

fn adjust_range(range: &TimeRange, lookback_days: u32, lookahead_days: u32) -> TimeRange {
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
    TimeRange {
        start: (start - Duration::days(lookback_days as i64))
            .format("%Y-%m-%d")
            .to_string(),
        end: (end + Duration::days(lookahead_days as i64))
            .format("%Y-%m-%d")
            .to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smelt_core::Granularity;

    fn make_ts(event_time_column: &str, partition_column: &str) -> TimeseriesConfig {
        TimeseriesConfig {
            event_time_column: event_time_column.into(),
            partition_column: partition_column.into(),
            granularity: Granularity::Day,
            week_start: None,
        }
    }

    fn make_inc() -> BatchedConfig {
        use smelt_core::BatchedSafetyOverrides;
        BatchedConfig {
            unique_key: vec![],
            safety_overrides: BatchedSafetyOverrides::default(),
        }
    }

    #[test]
    fn test_single_window_no_lookback() {
        let ts = make_ts("event_time", "d");
        let inc = make_inc();
        let range = TimeRange {
            start: "2026-03-20".into(),
            end: "2026-03-22".into(),
        };
        let w = compute_single_window(
            "SELECT date_trunc('day', event_time) as d FROM events",
            &inc,
            &ts,
            None,
            None,
            &range,
        );
        assert_eq!(w.filter_range.start, "2026-03-20");
        assert_eq!(w.filter_range.end, "2026-03-22");
    }

    #[test]
    fn test_single_window_lag_lookback() {
        let ts = make_ts("event_time", "day");
        let inc = make_inc();
        let range = TimeRange {
            start: "2026-03-20".into(),
            end: "2026-03-22".into(),
        };
        let w = compute_single_window(
            "SELECT user_id, day, LAG(amount, 3) OVER (ORDER BY day) as prev FROM events",
            &inc,
            &ts,
            None,
            None,
            &range,
        );
        assert_eq!(w.filter_range.start, "2026-03-17");
        assert_eq!(w.filter_range.end, "2026-03-22");
        assert_eq!(w.partition_range.start, "2026-03-20");
        assert_eq!(w.partition_range.end, "2026-03-22");
    }

    #[test]
    fn test_single_window_with_data_latency() {
        let ts = make_ts("event_time", "d");
        let inc = make_inc();
        let range = TimeRange {
            start: "2026-03-20".into(),
            end: "2026-03-22".into(),
        };
        let latency = smelt_core::DataLatency::parse("3 days").unwrap();
        let w = compute_single_window(
            "SELECT date_trunc('day', event_time) as d, SUM(amount) FROM events GROUP BY 1",
            &inc,
            &ts,
            None,
            Some(&latency),
            &range,
        );
        assert_eq!(w.filter_range.start, "2026-03-17");
        assert_eq!(w.filter_range.end, "2026-03-22");
    }
}
