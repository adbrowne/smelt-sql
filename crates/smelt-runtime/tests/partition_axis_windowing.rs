//! Phase 5a tests for the integer partition axis (`docs/specs/timeseries.md`
//! §"Validation rules" rule 9, `docs/specs/incremental_shapes.md` §"The
//! partition grain" rule 8a) — the unit-step integer grid, its
//! `PartitionPoint` arithmetic, run-window validation, and the day-typed
//! widening refusal. Calendar-axis coverage (unchanged behavior) lives in
//! `windowing_parity.rs`; this file only covers the new integer-axis branch
//! and the `PartitionPoint` type itself.

use std::collections::HashMap;

use smelt_core::config::TimeseriesConfig;
use smelt_core::{Granularity, PartitionGrainConfig, PartitionGrainSafetyOverrides};
use smelt_runtime::windowing::{
    compute_incremental_windows, validate_run_window_against_partition_grid, PartitionAxis,
    PartitionPoint,
};
use smelt_runtime::TimeRange;

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

// ── PartitionPoint::Display / sql_literal ──────────────────────────────────

#[test]
fn partition_point_display_and_sql_literal() {
    let d = PartitionPoint::Date(chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap());
    assert_eq!(d.to_string(), "2026-01-01");
    assert_eq!(d.sql_literal(), "'2026-01-01'");

    let i = PartitionPoint::Integer(7);
    assert_eq!(i.to_string(), "7");
    assert_eq!(i.sql_literal(), "7");
}

// ── Integer-axis chunking ───────────────────────────────────────────────────

#[test]
fn integer_axis_chunks_by_unit_steps() {
    let sql = "SELECT batch_id, id FROM events";
    let ts = make_ts("event_ts", "batch_id", Granularity::Day);
    let inc = make_inc();
    let range = make_range("1", "4");

    let windows = compute_incremental_windows(
        &ts,
        &inc,
        sql,
        &no_dep_timeseries(),
        0,
        &range,
        PartitionAxis::Integer,
        None,
        true, // per_partition: one unit per batch
    )
    .expect("integer axis must not be refused");

    assert_eq!(windows.batches.len(), 3, "expected [1,2) [2,3) [3,4)");
    assert_eq!(
        windows.batches[0].partition_start,
        PartitionPoint::Integer(1)
    );
    assert_eq!(windows.batches[0].partition_end, PartitionPoint::Integer(2));
    assert_eq!(
        windows.batches[1].partition_start,
        PartitionPoint::Integer(2)
    );
    assert_eq!(windows.batches[1].partition_end, PartitionPoint::Integer(3));
    assert_eq!(
        windows.batches[2].partition_start,
        PartitionPoint::Integer(3)
    );
    assert_eq!(windows.batches[2].partition_end, PartitionPoint::Integer(4));
}

#[test]
fn integer_axis_batch_size_counts_units() {
    let sql = "SELECT batch_id, id FROM events";
    let ts = make_ts("event_ts", "batch_id", Granularity::Day);
    let inc = make_inc();
    let range = make_range("1", "6");

    let windows = compute_incremental_windows(
        &ts,
        &inc,
        sql,
        &no_dep_timeseries(),
        0,
        &range,
        PartitionAxis::Integer,
        Some(2),
        false,
    )
    .expect("integer axis must not be refused");

    assert_eq!(windows.batches.len(), 3, "expected [1,3) [3,5) [5,6)");
    assert_eq!(
        windows.batches[0].partition_start,
        PartitionPoint::Integer(1)
    );
    assert_eq!(windows.batches[0].partition_end, PartitionPoint::Integer(3));
    assert_eq!(
        windows.batches[1].partition_start,
        PartitionPoint::Integer(3)
    );
    assert_eq!(windows.batches[1].partition_end, PartitionPoint::Integer(5));
    assert_eq!(
        windows.batches[2].partition_start,
        PartitionPoint::Integer(5)
    );
    assert_eq!(windows.batches[2].partition_end, PartitionPoint::Integer(6));
}

// ── validate_run_window_against_partition_grid on the integer axis ────────

#[test]
fn integer_axis_run_window_requires_positive_span_only() {
    let ts = make_ts("event_ts", "batch_id", Granularity::Day);
    let sql = "SELECT batch_id, id FROM events";

    validate_run_window_against_partition_grid(
        sql,
        &ts,
        PartitionPoint::Integer(3),
        PartitionPoint::Integer(4),
    )
    .expect("a positive-span integer window is accepted, no boundary/g_part check applies");

    let err = validate_run_window_against_partition_grid(
        sql,
        &ts,
        PartitionPoint::Integer(4),
        PartitionPoint::Integer(4),
    )
    .expect_err("a zero-span integer window must be rejected");
    assert!(
        err.contains("after"),
        "error must explain end must be after start, got: {err}"
    );
}

// ── Domain-mismatch refusals ────────────────────────────────────────────────

#[test]
fn integer_axis_refuses_date_bounds() {
    let err = PartitionPoint::parse_in_axis("2026-01-01", PartitionAxis::Integer)
        .expect_err("a calendar-shaped bound must be refused on an integer axis");
    assert!(err.contains("integer"), "got: {err}");
}

#[test]
fn calendar_axis_refuses_integer_bounds() {
    let err = PartitionPoint::parse_in_axis("7", PartitionAxis::Calendar)
        .expect_err("a bare-integer bound must be refused on a calendar axis");
    assert!(err.contains("calendar"), "got: {err}");
}

// ── Day-typed widening is refused fail-closed on the integer axis ─────────

#[test]
fn integer_axis_refuses_day_typed_widening() {
    let ts = make_ts("event_ts", "batch_id", Granularity::Day);
    let inc = make_inc();
    let sql = "SELECT batch_id, id FROM events";
    let range = make_range("1", "4");

    // Nonzero data_latency_days.
    let err = compute_incremental_windows(
        &ts,
        &inc,
        sql,
        &no_dep_timeseries(),
        3,
        &range,
        PartitionAxis::Integer,
        None,
        true,
    )
    .expect_err("nonzero data_latency_days must be refused on an integer axis, never coerced");
    assert!(
        err.contains("data_latency"),
        "error must name the offending input, got: {err}"
    );

    // Separately: a nonzero seconds/day-domain SQL-inferred lookback.
    let lag_sql = "SELECT batch_id, LAG(amount, 3) OVER (ORDER BY batch_id) as prev FROM events";
    let err = compute_incremental_windows(
        &ts,
        &inc,
        lag_sql,
        &no_dep_timeseries(),
        0,
        &range,
        PartitionAxis::Integer,
        None,
        true,
    )
    .expect_err("a nonzero SQL-inferred lookback must be refused on an integer axis");
    assert!(
        err.contains("lookback") || err.contains("lookahead"),
        "error must name the offending input, got: {err}"
    );
}
