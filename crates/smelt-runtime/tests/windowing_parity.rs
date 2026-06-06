//! Phase 1 parity tests for the new bound-aware windowing module.
//!
//! Verifies that `compute_incremental_windows` in `smelt_runtime::windowing` produces
//! correct (partition_start, partition_end, filter_start, filter_end) batch shapes for
//! various model configurations and request options.

use smelt_core::config::TimeseriesConfig;
use smelt_core::{Granularity, IncrementalConfig, IncrementalSafetyOverrides};
use smelt_runtime::windowing::{compute_incremental_windows, validate_run_window_alignment};
use smelt_runtime::TimeRange;

fn make_ts(event_col: &str, partition_col: &str, granularity: Granularity) -> TimeseriesConfig {
    TimeseriesConfig {
        event_time_column: event_col.to_string(),
        partition_column: partition_col.to_string(),
        granularity,
        week_start: None,
    }
}

fn make_inc() -> IncrementalConfig {
    IncrementalConfig {
        enabled: true,
        unique_key: vec![],
        safety_overrides: IncrementalSafetyOverrides::default(),
    }
}

fn make_range(start: &str, end: &str) -> TimeRange {
    TimeRange {
        start: start.to_string(),
        end: end.to_string(),
    }
}

// ── Multi-source bound-aware windows ──────────────────────────────────────────

#[test]
fn test_multi_source_bound_aware_windows() {
    // A model with a LAG dependency (3-period lookback) and additional data
    // latency produces correct (partition_start, partition_end, filter_start, filter_end)
    // shapes. The filter widens by max(SQL-lookback, data_latency_days).
    let sql = "SELECT date_trunc('day', event_time) as d, \
               LAG(amount, 3) OVER (ORDER BY d) as prev \
               FROM events";
    let ts = make_ts("event_time", "d", Granularity::Day);
    let inc = make_inc();
    let range = make_range("2026-03-20", "2026-03-22");

    // data_latency_days=2; SQL has 3-period lookback; max=3
    let windows = compute_incremental_windows(&ts, &inc, sql, 2, &range, None, false);

    // FullyBatchSafe batch (bounded 3-day context → ~9–90 day chunks → one batch for 2-day range)
    assert!(!windows.batches.is_empty(), "expected at least one batch");
    let b = &windows.batches[0];
    assert_eq!(b.partition_start.to_string(), "2026-03-20");
    assert_eq!(b.partition_end.to_string(), "2026-03-22");
    // max(3, 2) = 3 days lookback → filter_start = 2026-03-17
    assert_eq!(b.filter_start.to_string(), "2026-03-17");
    assert_eq!(b.filter_end.to_string(), "2026-03-22");
}

// ── Lookback widens filter window ─────────────────────────────────────────────

#[test]
fn test_lookback_widens_filter_window() {
    // A data_latency_days=3 source widens filter_start by 3 days while
    // partition_start stays at the partition boundary.
    let sql = "SELECT date_trunc('day', event_time) as d, SUM(amount) \
               FROM events GROUP BY 1";
    let ts = make_ts("event_time", "d", Granularity::Day);
    let inc = make_inc();
    let range = make_range("2026-03-20", "2026-03-22");

    let windows = compute_incremental_windows(&ts, &inc, sql, 3, &range, None, false);

    assert!(!windows.batches.is_empty(), "expected at least one batch");
    let b = &windows.batches[0];
    // Partition boundary is unaffected
    assert_eq!(b.partition_start.to_string(), "2026-03-20");
    assert_eq!(b.partition_end.to_string(), "2026-03-22");
    // filter_start widened by 3 days
    assert_eq!(b.filter_start.to_string(), "2026-03-17");
    assert_eq!(b.filter_end.to_string(), "2026-03-22");

    // Effective window captured correctly
    assert_eq!(windows.effective_window.lookback_days, 3);
}

// ── per_partition override ─────────────────────────────────────────────────────

#[test]
fn test_per_partition_override() {
    // per_partition=true produces one batch per granularity period,
    // ignoring BatchSafety::FullyBatchSafe.
    let sql = "SELECT date_trunc('day', event_time) as d, SUM(amount) \
               FROM events GROUP BY 1";
    let ts = make_ts("event_time", "d", Granularity::Day);
    let inc = make_inc();
    let range = make_range("2026-03-20", "2026-03-22");

    let windows = compute_incremental_windows(&ts, &inc, sql, 0, &range, None, true);

    // 2 days → 2 per-day batches
    assert_eq!(windows.batches.len(), 2, "expected 2 per-day batches");

    assert_eq!(windows.batches[0].partition_start.to_string(), "2026-03-20");
    assert_eq!(windows.batches[0].partition_end.to_string(), "2026-03-21");

    assert_eq!(windows.batches[1].partition_start.to_string(), "2026-03-21");
    assert_eq!(windows.batches[1].partition_end.to_string(), "2026-03-22");
}

// ── batch_size_days override ───────────────────────────────────────────────────

#[test]
fn test_batch_size_days_override() {
    // batch_size_days=2 produces 2-day batches regardless of batch safety.
    let sql = "SELECT date_trunc('day', event_time) as d, SUM(amount) \
               FROM events GROUP BY 1";
    let ts = make_ts("event_time", "d", Granularity::Day);
    let inc = make_inc();
    // 7-day range → 3 full 2-day batches + 1 partial 1-day batch
    let range = make_range("2026-03-20", "2026-03-27");

    let windows = compute_incremental_windows(&ts, &inc, sql, 0, &range, Some(2), false);

    assert_eq!(windows.batches.len(), 4, "expected 4 batches (3×2d + 1×1d)");

    assert_eq!(windows.batches[0].partition_start.to_string(), "2026-03-20");
    assert_eq!(windows.batches[0].partition_end.to_string(), "2026-03-22");

    assert_eq!(windows.batches[3].partition_start.to_string(), "2026-03-26");
    assert_eq!(windows.batches[3].partition_end.to_string(), "2026-03-27");
}

// ── validate_run_window_alignment ─────────────────────────────────────────────

#[test]
fn test_validate_run_window_alignment_misalignment() {
    use chrono::NaiveDate;

    // Monthly granularity — start must be 1st of month.
    let start = NaiveDate::from_ymd_opt(2026, 3, 15).unwrap();
    let end = NaiveDate::from_ymd_opt(2026, 4, 1).unwrap();

    let result = validate_run_window_alignment(start, end, &Granularity::Month);
    assert!(result.is_err());
    let msg = result.unwrap_err();
    assert!(
        msg.contains("not aligned to monthly granularity"),
        "unexpected error message: {msg}"
    );
}

#[test]
fn test_validate_run_window_alignment_ok_monthly() {
    use chrono::NaiveDate;

    let start = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
    let end = NaiveDate::from_ymd_opt(2026, 4, 1).unwrap();

    assert!(validate_run_window_alignment(start, end, &Granularity::Month).is_ok());
}

#[test]
fn test_validate_run_window_alignment_weekly() {
    use chrono::NaiveDate;

    // Weekly — both endpoints must be Mondays.
    let mon_start = NaiveDate::from_ymd_opt(2026, 3, 16).unwrap(); // Monday
    let mon_end = NaiveDate::from_ymd_opt(2026, 3, 23).unwrap(); // Monday
    assert!(validate_run_window_alignment(mon_start, mon_end, &Granularity::Week).is_ok());

    let tue_start = NaiveDate::from_ymd_opt(2026, 3, 17).unwrap(); // Tuesday
    let result = validate_run_window_alignment(tue_start, mon_end, &Granularity::Week);
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .contains("not aligned to weekly granularity"));
}

// ── No-latency baseline ───────────────────────────────────────────────────────

#[test]
fn test_no_lookback_no_widening() {
    // A simple GROUP BY query with no temporal deps and no data latency
    // should produce filter=partition ranges.
    let sql = "SELECT date_trunc('day', event_time) as d, COUNT(*) FROM events GROUP BY 1";
    let ts = make_ts("event_time", "d", Granularity::Day);
    let inc = make_inc();
    let range = make_range("2026-03-20", "2026-03-22");

    let windows = compute_incremental_windows(&ts, &inc, sql, 0, &range, None, false);

    let b = &windows.batches[0];
    assert_eq!(
        b.filter_start, b.partition_start,
        "filter_start should equal partition_start when no lookback"
    );
    assert_eq!(
        b.filter_end, b.partition_end,
        "filter_end should equal partition_end when no lookahead"
    );
}
