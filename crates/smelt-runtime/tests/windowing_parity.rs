//! Phase 1 parity tests for the new bound-aware windowing module.
//!
//! Verifies that `compute_incremental_windows` in `smelt_runtime::windowing` produces
//! correct (partition_start, partition_end, filter_start, filter_end) batch shapes for
//! various model configurations and request options.

use smelt_core::config::TimeseriesConfig;
use smelt_core::{Granularity, PartitionGrainConfig, PartitionGrainSafetyOverrides};
use smelt_runtime::windowing::{
    compute_incremental_windows, compute_incremental_windows_ordered,
    validate_run_window_alignment, PartitionAxis,
};
use smelt_runtime::TimeRange;
use std::collections::HashMap;

fn make_ts(event_col: &str, partition_col: &str, granularity: Granularity) -> TimeseriesConfig {
    TimeseriesConfig {
        event_time_column: event_col.to_string(),
        partition_column: partition_col.to_string(),
        granularity,
        week_start: None,
        assert_monotonic: false,
    }
}

fn make_inc() -> PartitionGrainConfig {
    PartitionGrainConfig {
        unique_key: vec![],
        nondeterministic_columns_retired: (),
        safety_overrides: PartitionGrainSafetyOverrides::default(),
    }
}

fn make_range(start: &str, end: &str) -> TimeRange {
    TimeRange {
        start: start.to_string(),
        end: end.to_string(),
        axis: smelt_logical::PartitionAxis::Calendar,
    }
}

fn no_dep_timeseries() -> HashMap<String, (Vec<String>, String)> {
    HashMap::new()
}

// ── Multi-source bound-aware windows ──────────────────────────────────────────

#[test]
fn test_multi_source_bound_aware_windows() {
    // A model with a LAG dependency (3-period lookback) and additional data
    // latency produces correct (partition_start, partition_end, filter_start, filter_end)
    // shapes. Lateness and computation-reach are independent quantities, so
    // the filter widens by their SUM (a max here encoded the truncated-frame
    // bug — catalog cell SC-5).
    let sql = "SELECT date_trunc('day', event_time) as d, \
               LAG(amount, 3) OVER (ORDER BY d) as prev \
               FROM events";
    let ts = make_ts("event_time", "d", Granularity::Day);
    let inc = make_inc();
    let range = make_range("2026-03-20", "2026-03-22");

    // data_latency_days=2; SQL has 3-period lookback; sum=5
    let windows = compute_incremental_windows(
        &ts,
        &inc,
        sql,
        &no_dep_timeseries(),
        2,
        &range,
        PartitionAxis::Calendar,
        None,
        false,
    )
    .unwrap();

    // FullyBatchSafe batch (bounded 3-day context → ~9–90 day chunks → one batch for 2-day range)
    assert!(!windows.batches.is_empty(), "expected at least one batch");
    let b = &windows.batches[0];
    assert_eq!(b.partition_start.to_string(), "2026-03-20");
    assert_eq!(b.partition_end.to_string(), "2026-03-22");
    // 3 + 2 = 5 days lookback → filter_start = 2026-03-15
    assert_eq!(b.filter_start.to_string(), "2026-03-15");
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

    let windows = compute_incremental_windows(
        &ts,
        &inc,
        sql,
        &no_dep_timeseries(),
        3,
        &range,
        PartitionAxis::Calendar,
        None,
        false,
    )
    .unwrap();

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

    let windows = compute_incremental_windows(
        &ts,
        &inc,
        sql,
        &no_dep_timeseries(),
        0,
        &range,
        PartitionAxis::Calendar,
        None,
        true,
    )
    .unwrap();

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

    let windows = compute_incremental_windows(
        &ts,
        &inc,
        sql,
        &no_dep_timeseries(),
        0,
        &range,
        PartitionAxis::Calendar,
        Some(2),
        false,
    )
    .unwrap();

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

    let windows = compute_incremental_windows(
        &ts,
        &inc,
        sql,
        &no_dep_timeseries(),
        0,
        &range,
        PartitionAxis::Calendar,
        None,
        true,
    )
    .unwrap();

    // Must be exactly 24 batches
    assert_eq!(
        windows.batches.len(),
        24,
        "expected 24 monthly batches, got {}",
        windows.batches.len()
    );

    // Every batch must start and end on the 1st of a month (true calendar alignment)
    use chrono::Datelike;
    fn as_date(p: smelt_runtime::windowing::PartitionPoint) -> chrono::NaiveDate {
        match p {
            smelt_runtime::windowing::PartitionPoint::Date(d) => d,
            other => panic!("expected a calendar PartitionPoint, got {other:?}"),
        }
    }
    for (i, b) in windows.batches.iter().enumerate() {
        assert_eq!(
            as_date(b.partition_start).day(),
            1,
            "batch {} start {} is not the 1st of a month",
            i,
            b.partition_start
        );
        assert_eq!(
            as_date(b.partition_end).day(),
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
        smelt_runtime::windowing::PartitionPoint::Date(
            NaiveDate::from_ymd_opt(2025, 1, 1).unwrap()
        )
    );
    assert_eq!(
        windows.batches[23].partition_end,
        smelt_runtime::windowing::PartitionPoint::Date(
            NaiveDate::from_ymd_opt(2027, 1, 1).unwrap()
        )
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

    let windows = compute_incremental_windows(
        &ts,
        &inc,
        sql,
        &no_dep_timeseries(),
        0,
        &range,
        PartitionAxis::Calendar,
        None,
        true,
    )
    .unwrap();

    assert_eq!(windows.batches.len(), 3);
    use chrono::NaiveDate;
    assert_eq!(
        windows.batches[0].partition_start,
        smelt_runtime::windowing::PartitionPoint::Date(
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap()
        )
    );
    assert_eq!(
        windows.batches[0].partition_end,
        smelt_runtime::windowing::PartitionPoint::Date(
            NaiveDate::from_ymd_opt(2024, 2, 1).unwrap()
        )
    );
    // Feb 1 → Mar 1 (29 days in a leap year)
    assert_eq!(
        windows.batches[1].partition_start,
        smelt_runtime::windowing::PartitionPoint::Date(
            NaiveDate::from_ymd_opt(2024, 2, 1).unwrap()
        )
    );
    assert_eq!(
        windows.batches[1].partition_end,
        smelt_runtime::windowing::PartitionPoint::Date(
            NaiveDate::from_ymd_opt(2024, 3, 1).unwrap()
        )
    );
    // Mar 1 → Apr 1 (31 days)
    assert_eq!(
        windows.batches[2].partition_start,
        smelt_runtime::windowing::PartitionPoint::Date(
            NaiveDate::from_ymd_opt(2024, 3, 1).unwrap()
        )
    );
    assert_eq!(
        windows.batches[2].partition_end,
        smelt_runtime::windowing::PartitionPoint::Date(
            NaiveDate::from_ymd_opt(2024, 4, 1).unwrap()
        )
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

    let windows = compute_incremental_windows(
        &ts,
        &inc,
        sql,
        &no_dep_timeseries(),
        0,
        &range,
        PartitionAxis::Calendar,
        None,
        true,
    )
    .unwrap();

    assert_eq!(
        windows.batches.len(),
        4,
        "expected 4 quarterly batches, got {}",
        windows.batches.len()
    );

    use chrono::NaiveDate;
    let expected_starts = [
        smelt_runtime::windowing::PartitionPoint::Date(
            NaiveDate::from_ymd_opt(2025, 1, 1).unwrap(),
        ),
        smelt_runtime::windowing::PartitionPoint::Date(
            NaiveDate::from_ymd_opt(2025, 4, 1).unwrap(),
        ),
        smelt_runtime::windowing::PartitionPoint::Date(
            NaiveDate::from_ymd_opt(2025, 7, 1).unwrap(),
        ),
        smelt_runtime::windowing::PartitionPoint::Date(
            NaiveDate::from_ymd_opt(2025, 10, 1).unwrap(),
        ),
    ];
    let expected_ends = [
        smelt_runtime::windowing::PartitionPoint::Date(
            NaiveDate::from_ymd_opt(2025, 4, 1).unwrap(),
        ),
        smelt_runtime::windowing::PartitionPoint::Date(
            NaiveDate::from_ymd_opt(2025, 7, 1).unwrap(),
        ),
        smelt_runtime::windowing::PartitionPoint::Date(
            NaiveDate::from_ymd_opt(2025, 10, 1).unwrap(),
        ),
        smelt_runtime::windowing::PartitionPoint::Date(
            NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
        ),
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

    let windows = compute_incremental_windows(
        &ts,
        &inc,
        sql,
        &no_dep_timeseries(),
        0,
        &range,
        PartitionAxis::Calendar,
        None,
        true,
    )
    .unwrap();

    assert_eq!(
        windows.batches.len(),
        3,
        "expected 3 yearly batches, got {}",
        windows.batches.len()
    );

    use chrono::NaiveDate;
    let expected_starts = [
        smelt_runtime::windowing::PartitionPoint::Date(
            NaiveDate::from_ymd_opt(2023, 1, 1).unwrap(),
        ),
        smelt_runtime::windowing::PartitionPoint::Date(
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
        ),
        smelt_runtime::windowing::PartitionPoint::Date(
            NaiveDate::from_ymd_opt(2025, 1, 1).unwrap(),
        ),
    ];
    let expected_ends = [
        smelt_runtime::windowing::PartitionPoint::Date(
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
        ),
        smelt_runtime::windowing::PartitionPoint::Date(
            NaiveDate::from_ymd_opt(2025, 1, 1).unwrap(),
        ),
        smelt_runtime::windowing::PartitionPoint::Date(
            NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
        ),
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
    let windows = compute_incremental_windows(
        &ts_day,
        &inc,
        sql,
        &no_dep_timeseries(),
        0,
        &range,
        PartitionAxis::Calendar,
        None,
        true,
    )
    .unwrap();
    assert_eq!(windows.batches.len(), 3, "expected 3 daily batches");

    // 2 weeks with per_partition → 2 weekly batches
    let ts_week = make_ts("event_time", "week_start", Granularity::Week);
    let range_w = make_range("2026-03-02", "2026-03-16"); // 14 days, 2 Mon→Mon
    let windows_w = compute_incremental_windows(
        &ts_week,
        &inc,
        sql,
        &no_dep_timeseries(),
        0,
        &range_w,
        PartitionAxis::Calendar,
        None,
        true,
    )
    .unwrap();
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

    let windows = compute_incremental_windows(
        &ts,
        &inc,
        sql,
        &no_dep_timeseries(),
        0,
        &range,
        PartitionAxis::Calendar,
        None,
        false,
    )
    .unwrap();

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

    let windows = compute_incremental_windows(
        &ts,
        &inc,
        sql,
        &no_dep_timeseries(),
        0,
        &range,
        PartitionAxis::Calendar,
        None,
        false,
    )
    .unwrap();

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

    let windows = compute_incremental_windows(
        &ts,
        &inc,
        sql,
        &no_dep_timeseries(),
        0,
        &range,
        PartitionAxis::Calendar,
        None,
        true,
    )
    .unwrap();

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

    let windows = compute_incremental_windows(
        &ts,
        &inc,
        sql,
        &no_dep_timeseries(),
        0,
        &range,
        PartitionAxis::Calendar,
        Some(30),
        false,
    )
    .unwrap();

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

    let windows = compute_incremental_windows(
        &ts,
        &inc,
        sql,
        &no_dep_timeseries(),
        0,
        &range,
        PartitionAxis::Calendar,
        None,
        false,
    )
    .unwrap();

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

// ── BL2: F1 bound-based batch-safety drives the chunk shape ──────────────────
//
// These tests exercise `compute_incremental_windows`'s auto (non-per_partition,
// non-batch_size_days-override) chunk-size path, which now derives its
// `BatchSafety` class from `dep_timeseries` via
// `compile::batch_safety_for_model` (`derive_and_classify_bounds`), not the
// legacy `analyze_batch_safety`.

fn one_source_dep(source: &str, partition_col: &str) -> HashMap<String, (Vec<String>, String)> {
    let mut m = HashMap::new();
    m.insert(
        source.to_string(),
        (
            source.split('.').map(String::from).collect(),
            partition_col.to_string(),
        ),
    );
    m
}

#[test]
fn test_fully_batch_safe_single_pair_for_60_day_range() {
    // No temporal dependency on the declared source (zero-margin Bounded) →
    // FullyBatchSafe → one DELETE+INSERT pair for the whole 60-day range.
    let sql = "SELECT d, SUM(amount) FROM smelt.silver.events GROUP BY d";
    let ts = make_ts("event_time", "d", Granularity::Day);
    let inc = make_inc();
    let range = make_range("2026-01-01", "2026-03-02"); // 60 days
    let deps = one_source_dep("silver.events", "d");

    let windows = compute_incremental_windows(
        &ts,
        &inc,
        sql,
        &deps,
        0,
        &range,
        PartitionAxis::Calendar,
        None,
        false,
    )
    .expect("FullyBatchSafe model should not be refused");

    assert_eq!(
        windows.batches.len(),
        1,
        "FullyBatchSafe should yield exactly one batch for the whole range"
    );
    assert_eq!(windows.batches[0].partition_start.to_string(), "2026-01-01");
    assert_eq!(windows.batches[0].partition_end.to_string(), "2026-03-02");
}

#[test]
fn test_bounded_safe_yields_subranges_clamped_to_its_own_max_chunk_days() {
    // A Form-A window frame with a 10-day PRECEDING margin on `d` makes the
    // source BoundedSafe with context_days=10 → max_chunk_days =
    // (10*3).clamp(7,90) = 30. A 100-day range must therefore split into
    // sub-ranges each <= 30 days (batch-safety's own clamp), not one pair.
    let sql = "SELECT d, SUM(amount) OVER (ORDER BY d RANGE BETWEEN INTERVAL '10 day' \
               PRECEDING AND CURRENT ROW) FROM smelt.silver.events";
    let ts = make_ts("event_time", "d", Granularity::Day);
    let inc = make_inc();
    let range = make_range("2026-01-01", "2026-04-11"); // 100 days
    let deps = one_source_dep("silver.events", "d");

    let windows = compute_incremental_windows(
        &ts,
        &inc,
        sql,
        &deps,
        0,
        &range,
        PartitionAxis::Calendar,
        None,
        false,
    )
    .expect("BoundedSafe model should not be refused");

    assert!(
        windows.batches.len() > 1,
        "BoundedSafe should split a 100-day range into multiple sub-ranges, got {}",
        windows.batches.len()
    );
    for b in &windows.batches {
        let span_days = b.partition_start.units_between(&b.partition_end);
        assert!(
            span_days <= 30,
            "each BoundedSafe sub-range must be clamped to its own max_chunk_days (30), got {span_days}"
        );
    }
    // Sub-ranges are contiguous and cover the whole requested range.
    assert_eq!(windows.batches[0].partition_start.to_string(), "2026-01-01");
    assert_eq!(
        windows.batches.last().unwrap().partition_end.to_string(),
        "2026-04-11"
    );
}

#[test]
fn test_per_partition_only_yields_one_partition_per_iteration() {
    // `RANGE BETWEEN UNBOUNDED PRECEDING` on the source's partition column is
    // an unbounded cross-partition dependency → PerPartitionOnly, even
    // without the caller passing `per_partition: true` — the auto chunk-size
    // path must fall back to one partition per iteration on its own.
    let sql = "SELECT d, SUM(amount) OVER (ORDER BY d RANGE BETWEEN UNBOUNDED PRECEDING \
               AND CURRENT ROW) FROM smelt.silver.events";
    let ts = make_ts("event_time", "d", Granularity::Day);
    let inc = make_inc();
    let range = make_range("2026-01-01", "2026-01-06"); // 5 days
    let deps = one_source_dep("silver.events", "d");

    let windows = compute_incremental_windows(
        &ts,
        &inc,
        sql,
        &deps,
        0,
        &range,
        PartitionAxis::Calendar,
        None,
        false,
    )
    .expect("PerPartitionOnly model should not be refused");

    assert_eq!(
        windows.batches.len(),
        5,
        "PerPartitionOnly should yield one partition per iteration (5 daily batches)"
    );
    for (i, b) in windows.batches.iter().enumerate() {
        let span_days = b.partition_start.units_between(&b.partition_end);
        assert_eq!(
            span_days, 1,
            "batch {i} should be exactly one partition wide"
        );
    }
}

#[test]
fn test_not_derivable_source_bound_is_a_hard_refusal_not_an_approximate_chunk() {
    // A bare LAG(...) OVER (...) with no RANGE clause is NotDerivable
    // (fail-closed, `incremental_shapes.md` §"Partition-grain constraints" #10). The auto chunk-size
    // path must surface this as `Err`, never silently fall back to an
    // approximate chunk shape (e.g. FullyBatchSafe or a default BoundedSafe).
    let sql = "SELECT d, LAG(amount) OVER (ORDER BY d) FROM smelt.silver.events";
    let ts = make_ts("event_time", "d", Granularity::Day);
    let inc = make_inc();
    let range = make_range("2026-01-01", "2026-03-02");
    let deps = one_source_dep("silver.events", "d");

    let result = compute_incremental_windows(
        &ts,
        &inc,
        sql,
        &deps,
        0,
        &range,
        PartitionAxis::Calendar,
        None,
        false,
    );

    let err = result.expect_err("a NotDerivable source bound must refuse, not chunk");
    assert!(
        err.contains("cannot derive a temporal bound"),
        "error should name the derivation failure, got: {err}"
    );
    assert!(
        err.contains("silver.events"),
        "error should name the offending source, got: {err}"
    );
}

#[test]
fn test_not_derivable_is_not_masked_by_per_partition_or_batch_size_override() {
    // Explicit `--batch-size`/`--per-partition` bypass the auto chunk-size
    // path entirely (the user made an explicit choice), so they must NOT be
    // used to silently route around a NotDerivable refusal. This test
    // documents current behaviour: an explicit override skips the
    // batch-safety derivation, and therefore skips the refusal too — it is
    // the model author's explicit choice, not an approximation the system
    // invented.
    let sql = "SELECT d, LAG(amount) OVER (ORDER BY d) FROM smelt.silver.events";
    let ts = make_ts("event_time", "d", Granularity::Day);
    let inc = make_inc();
    let range = make_range("2026-01-01", "2026-01-06");
    let deps = one_source_dep("silver.events", "d");

    let windows = compute_incremental_windows(
        &ts,
        &inc,
        sql,
        &deps,
        0,
        &range,
        PartitionAxis::Calendar,
        None,
        true,
    )
    .expect("per_partition=true bypasses batch-safety derivation entirely");
    assert_eq!(windows.batches.len(), 5);
}

// ── BL7: window-independence / ordered-execution composed into the chunker ──
//
// `compute_incremental_windows_ordered` wires F10's `window_independence`
// verdict (`smelt_logical::analysis::window_independence`) into the BL2
// chunker: a self-referential model (`refs` containing its own `model_name`)
// that provably converges partition-by-partition is forced to
// strictly-sequential single-partition batches regardless of its batch-safety
// class or any override; a model with no self-edge is unaffected; a
// non-convergent self-edge refuses fail-closed, naming the model.

const RUNNING_BALANCE_SQL: &str = "SELECT bal.d AS d, bal.balance + t.amount AS balance \
     FROM smelt.marts.running_balance bal \
     JOIN smelt.silver.transactions t ON bal.acct_id = t.acct_id \
     WHERE bal.d >= t.d - INTERVAL '1 day' AND bal.d < t.d";

#[test]
fn test_no_self_edge_is_unaffected_keeps_batch_safety_auto_chunking() {
    // FullyBatchSafe (zero-margin) model with no self-edge: `refs` does not
    // contain `model_name`, so the ordered wrapper must delegate unchanged —
    // one batch for the whole 60-day range, exactly as `compute_incremental_windows`.
    let sql = "SELECT d, SUM(amount) FROM smelt.silver.events GROUP BY d";
    let ts = make_ts("event_time", "d", Granularity::Day);
    let inc = make_inc();
    let range = make_range("2026-01-01", "2026-03-02");
    let deps = one_source_dep("silver.events", "d");

    let windows = compute_incremental_windows_ordered(
        "marts.revenue",
        &["silver.events".to_string()],
        &ts,
        &inc,
        sql,
        &deps,
        0,
        &range,
        PartitionAxis::Calendar,
        None,
        false,
    )
    .expect("window-independent model should not be refused");

    assert_eq!(
        windows.batches.len(),
        1,
        "window-independent model keeps FullyBatchSafe's single-batch auto-chunking"
    );
}

#[test]
fn test_convergent_self_edge_is_ordered_forces_strictly_sequential_single_partition_batches() {
    // `marts.running_balance` reads its own prior partition (backward-bounded
    // by exactly 1 day) — F10 proves `Ordered`. Even though the model has no
    // temporal dependency on any *other* bounded source (which alone would be
    // FullyBatchSafe → one wide batch), the ordered verdict must force one
    // partition per batch over the 5-day range: a wide batch would read
    // `bal`'s not-yet-written rows for later days in the same batch.
    let ts = make_ts("d", "d", Granularity::Day);
    let inc = make_inc();
    let range = make_range("2026-01-01", "2026-01-06"); // 5 days
    let deps = one_source_dep("silver.transactions", "d");
    let refs = vec![
        "marts.running_balance".to_string(),
        "silver.transactions".to_string(),
    ];

    let windows = compute_incremental_windows_ordered(
        "marts.running_balance",
        &refs,
        &ts,
        &inc,
        RUNNING_BALANCE_SQL,
        &deps,
        0,
        &range,
        PartitionAxis::Calendar,
        None,
        false, // no explicit per_partition override — Ordered must force it anyway
    )
    .expect("a convergent self-edge must build, not refuse");

    assert_eq!(
        windows.batches.len(),
        5,
        "Ordered must force one partition per batch, ignoring FullyBatchSafe-style wide chunking"
    );
    for (i, batch) in windows.batches.iter().enumerate() {
        let expected_start = format!("2026-01-0{}", i + 1);
        assert_eq!(
            batch.partition_start.to_string(),
            expected_start,
            "batch {i} must be exactly one day, in strict temporal order"
        );
    }
}

#[test]
fn test_ordered_forces_single_partition_even_with_explicit_batch_size_override() {
    // An `Ordered` model's self-referential dependency is never safe to widen
    // — not even via an explicit `--batch-size` override, since the override
    // would still lump multiple not-yet-committed partitions into one query.
    let ts = make_ts("d", "d", Granularity::Day);
    let inc = make_inc();
    let range = make_range("2026-01-01", "2026-01-06"); // 5 days
    let deps = one_source_dep("silver.transactions", "d");
    let refs = vec![
        "marts.running_balance".to_string(),
        "silver.transactions".to_string(),
    ];

    let windows = compute_incremental_windows_ordered(
        "marts.running_balance",
        &refs,
        &ts,
        &inc,
        RUNNING_BALANCE_SQL,
        &deps,
        0,
        &range,
        PartitionAxis::Calendar,
        Some(5), // explicit override asking for the whole range in one batch
        false,
    )
    .expect("a convergent self-edge must build, not refuse");

    assert_eq!(
        windows.batches.len(),
        5,
        "Ordered overrides an explicit batch_size_days that would widen past one partition"
    );
}

#[test]
fn test_non_convergent_self_edge_refuses_fail_closed_naming_the_model() {
    // A self-reference reading *forward* of the current partition can never
    // be proven to converge partition-by-partition — refused, never silently
    // built as `Ordered` or `WindowIndependent`.
    let forward_sql = "SELECT bal.balance AS balance \
                        FROM smelt.marts.running_balance bal \
                        WHERE bal.d <= m.d + INTERVAL '1 day'";
    let ts = make_ts("d", "d", Granularity::Day);
    let inc = make_inc();
    let range = make_range("2026-01-01", "2026-01-06");
    let refs = vec!["marts.running_balance".to_string()];

    let err = compute_incremental_windows_ordered(
        "marts.running_balance",
        &refs,
        &ts,
        &inc,
        forward_sql,
        &no_dep_timeseries(),
        0,
        &range,
        PartitionAxis::Calendar,
        None,
        false,
    )
    .expect_err("a forward-reading self-edge must refuse, never silently build");

    assert!(
        err.contains("marts.running_balance"),
        "refusal must name the non-convergent self-edge, got: {err}"
    );
}
