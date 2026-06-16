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

// ── Calendar-aligned per-partition tiling (Month/Quarter/Year) ───────────────

#[test]
fn test_per_partition_monthly_calendar_aligned() {
    // per_partition=true with monthly granularity must produce calendar-month
    // batches (28/29/30/31 days) instead of fixed 30-day steps.  Over 24 months
    // the fixed-day path drifts several days off the true month start; this test
    // verifies every batch lands exactly on the 1st of each month.
    let sql = "SELECT event_time, amount FROM events";
    let ts = make_ts("event_time", "month_start", Granularity::Month);
    let inc = make_inc();
    // 24 months: 2025-01-01 to 2027-01-01
    let range = make_range("2025-01-01", "2027-01-01");

    let windows = compute_incremental_windows(&ts, &inc, sql, 0, &range, None, true);

    // Must be exactly 24 batches
    assert_eq!(
        windows.batches.len(),
        24,
        "expected 24 monthly batches, got {}",
        windows.batches.len()
    );

    // Every batch must start and end on the 1st of a month (true calendar alignment)
    use chrono::Datelike;
    for (i, b) in windows.batches.iter().enumerate() {
        assert_eq!(
            b.partition_start.day(),
            1,
            "batch {} start {} is not the 1st of a month",
            i,
            b.partition_start
        );
        assert_eq!(
            b.partition_end.day(),
            1,
            "batch {} end {} is not the 1st of a month",
            i,
            b.partition_end
        );
    }

    // Span is contiguous: each batch's end equals the next batch's start
    for i in 0..windows.batches.len() - 1 {
        assert_eq!(
            windows.batches[i].partition_end,
            windows.batches[i + 1].partition_start,
            "gap between batch {} and {}",
            i,
            i + 1
        );
    }

    // First and last boundaries correct
    use chrono::NaiveDate;
    assert_eq!(
        windows.batches[0].partition_start,
        NaiveDate::from_ymd_opt(2025, 1, 1).unwrap()
    );
    assert_eq!(
        windows.batches[23].partition_end,
        NaiveDate::from_ymd_opt(2027, 1, 1).unwrap()
    );
}

#[test]
fn test_per_partition_monthly_feb_boundary() {
    // Crossing a February validates the calendar logic against 28/29-day months.
    let sql = "SELECT event_time, amount FROM events";
    let ts = make_ts("event_time", "month_start", Granularity::Month);
    let inc = make_inc();
    // Jan-Mar 2024 (Feb has 29 days — leap year)
    let range = make_range("2024-01-01", "2024-04-01");

    let windows = compute_incremental_windows(&ts, &inc, sql, 0, &range, None, true);

    assert_eq!(windows.batches.len(), 3);
    use chrono::NaiveDate;
    assert_eq!(
        windows.batches[0].partition_start,
        NaiveDate::from_ymd_opt(2024, 1, 1).unwrap()
    );
    assert_eq!(
        windows.batches[0].partition_end,
        NaiveDate::from_ymd_opt(2024, 2, 1).unwrap()
    );
    // Feb 1 → Mar 1 (29 days in a leap year)
    assert_eq!(
        windows.batches[1].partition_start,
        NaiveDate::from_ymd_opt(2024, 2, 1).unwrap()
    );
    assert_eq!(
        windows.batches[1].partition_end,
        NaiveDate::from_ymd_opt(2024, 3, 1).unwrap()
    );
    // Mar 1 → Apr 1 (31 days)
    assert_eq!(
        windows.batches[2].partition_start,
        NaiveDate::from_ymd_opt(2024, 3, 1).unwrap()
    );
    assert_eq!(
        windows.batches[2].partition_end,
        NaiveDate::from_ymd_opt(2024, 4, 1).unwrap()
    );
}

#[test]
fn test_per_partition_quarterly_calendar_aligned() {
    // Quarterly per-partition batches must step by true calendar quarters.
    let sql = "SELECT event_time, amount FROM events";
    let ts = make_ts("event_time", "quarter_start", Granularity::Quarter);
    let inc = make_inc();
    // 4 quarters: 2025-01-01 to 2026-01-01
    let range = make_range("2025-01-01", "2026-01-01");

    let windows = compute_incremental_windows(&ts, &inc, sql, 0, &range, None, true);

    assert_eq!(
        windows.batches.len(),
        4,
        "expected 4 quarterly batches, got {}",
        windows.batches.len()
    );

    use chrono::NaiveDate;
    let expected_starts = [
        NaiveDate::from_ymd_opt(2025, 1, 1).unwrap(),
        NaiveDate::from_ymd_opt(2025, 4, 1).unwrap(),
        NaiveDate::from_ymd_opt(2025, 7, 1).unwrap(),
        NaiveDate::from_ymd_opt(2025, 10, 1).unwrap(),
    ];
    let expected_ends = [
        NaiveDate::from_ymd_opt(2025, 4, 1).unwrap(),
        NaiveDate::from_ymd_opt(2025, 7, 1).unwrap(),
        NaiveDate::from_ymd_opt(2025, 10, 1).unwrap(),
        NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
    ];
    for (i, b) in windows.batches.iter().enumerate() {
        assert_eq!(b.partition_start, expected_starts[i], "batch {} start", i);
        assert_eq!(b.partition_end, expected_ends[i], "batch {} end", i);
    }
}

#[test]
fn test_per_partition_yearly_calendar_aligned() {
    // Yearly per-partition batches must step by true calendar years.
    let sql = "SELECT event_time, amount FROM events";
    let ts = make_ts("event_time", "year_start", Granularity::Year);
    let inc = make_inc();
    // 3 years: 2023-01-01 to 2026-01-01
    let range = make_range("2023-01-01", "2026-01-01");

    let windows = compute_incremental_windows(&ts, &inc, sql, 0, &range, None, true);

    assert_eq!(
        windows.batches.len(),
        3,
        "expected 3 yearly batches, got {}",
        windows.batches.len()
    );

    use chrono::NaiveDate;
    let expected_starts = [
        NaiveDate::from_ymd_opt(2023, 1, 1).unwrap(),
        NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
        NaiveDate::from_ymd_opt(2025, 1, 1).unwrap(),
    ];
    let expected_ends = [
        NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
        NaiveDate::from_ymd_opt(2025, 1, 1).unwrap(),
        NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
    ];
    for (i, b) in windows.batches.iter().enumerate() {
        assert_eq!(b.partition_start, expected_starts[i], "batch {} start", i);
        assert_eq!(b.partition_end, expected_ends[i], "batch {} end", i);
    }
}

#[test]
fn test_per_partition_daily_and_weekly_unchanged() {
    // Day/Week granularity still use fixed-step tiling (unchanged by the calendar fix).
    let sql = "SELECT event_time, amount FROM events";

    // 3 days with per_partition → 3 day batches
    let ts_day = make_ts("event_time", "day", Granularity::Day);
    let inc = make_inc();
    let range = make_range("2026-03-01", "2026-03-04");
    let windows = compute_incremental_windows(&ts_day, &inc, sql, 0, &range, None, true);
    assert_eq!(windows.batches.len(), 3, "expected 3 daily batches");

    // 2 weeks with per_partition → 2 weekly batches
    let ts_week = make_ts("event_time", "week_start", Granularity::Week);
    let range_w = make_range("2026-03-02", "2026-03-16"); // 14 days, 2 Mon→Mon
    let windows_w = compute_incremental_windows(&ts_week, &inc, sql, 0, &range_w, None, true);
    assert_eq!(windows_w.batches.len(), 2, "expected 2 weekly batches");
}

// ── Wide single-batch warning ─────────────────────────────────────────────────

#[test]
fn test_wide_single_batch_warns() {
    // A FullyBatchSafe model (simple GROUP BY, no temporal deps) over 90 days
    // should warn because it creates a single query covering 90 partition periods.
    let sql = "SELECT date_trunc('day', event_time) as d, COUNT(*) FROM events GROUP BY 1";
    let ts = make_ts("event_time", "d", Granularity::Day);
    let inc = make_inc();
    // 90 days > 30-period threshold
    let range = make_range("2026-01-01", "2026-04-01");

    let windows = compute_incremental_windows(&ts, &inc, sql, 0, &range, None, false);

    // FullyBatchSafe → single batch
    assert_eq!(
        windows.batches.len(),
        1,
        "expected single batch for FullyBatchSafe"
    );
    // Warning should be present for wide range
    assert!(
        windows.wide_batch_warning.is_some(),
        "expected a wide-batch warning for 90-day single batch"
    );
    let msg = windows.wide_batch_warning.unwrap();
    assert!(
        msg.contains("--per-partition") || msg.contains("--batch-size"),
        "warning should recommend --per-partition or --batch-size, got: {msg}"
    );
}

#[test]
fn test_narrow_single_batch_no_warn() {
    // A FullyBatchSafe model over 7 days should NOT warn (well within threshold).
    let sql = "SELECT date_trunc('day', event_time) as d, COUNT(*) FROM events GROUP BY 1";
    let ts = make_ts("event_time", "d", Granularity::Day);
    let inc = make_inc();
    let range = make_range("2026-01-01", "2026-01-08"); // 7 days

    let windows = compute_incremental_windows(&ts, &inc, sql, 0, &range, None, false);

    assert!(
        windows.wide_batch_warning.is_none(),
        "no warning expected for 7-day single batch"
    );
}

#[test]
fn test_per_partition_no_wide_batch_warn() {
    // per_partition=true means the user already opted into safe batching;
    // no warning should fire even for a wide range.
    let sql = "SELECT date_trunc('day', event_time) as d, COUNT(*) FROM events GROUP BY 1";
    let ts = make_ts("event_time", "d", Granularity::Day);
    let inc = make_inc();
    let range = make_range("2026-01-01", "2026-04-01"); // 90 days

    let windows = compute_incremental_windows(&ts, &inc, sql, 0, &range, None, true);

    assert!(
        windows.wide_batch_warning.is_none(),
        "no warning expected when per_partition=true"
    );
}

#[test]
fn test_batch_size_override_no_wide_batch_warn() {
    // If the user supplied --batch-size, they already made an explicit choice;
    // no additional warning is needed.
    let sql = "SELECT date_trunc('day', event_time) as d, COUNT(*) FROM events GROUP BY 1";
    let ts = make_ts("event_time", "d", Granularity::Day);
    let inc = make_inc();
    let range = make_range("2026-01-01", "2026-04-01"); // 90 days

    let windows = compute_incremental_windows(&ts, &inc, sql, 0, &range, Some(30), false);

    assert!(
        windows.wide_batch_warning.is_none(),
        "no warning expected when batch_size_days is set"
    );
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
