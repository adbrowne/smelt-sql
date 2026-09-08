//! Interior-chunk forward reach for Form-B models (`docs/specs/incremental_shapes.md`
//! §"Execution model (DuckDB)", `docs/specs/model_transforms.md` §Semantics "The output
//! window is derived, never assumed"): `compute_calendar_windows` folds the model's own
//! declared partition-column skew into every batch's **scan** range, not just the
//! invocation's outer write-side widening — so an interior chunk boundary no longer
//! truncates the driving-date rows a Form-B partition's own declared relation admits.
//!
//! `SESSION_SQL` is the same `driving BETWEEN partition_col AND partition_col + INTERVAL
//! '1 day'` shape as `examples/github_activity/models/silver/actor_sessions.sql`
//! (`Skew { before: 0, after: 1 day }`) — deep-interior batches (far enough from both
//! edges of the derived output window) get the model's full forward reach; batches within
//! `after` days of the output window's own end stay clamped to the invocation's existing
//! outer scan envelope, by the phase 6 decision log's deliberate choice not to widen past
//! the run window's own trailing edge.

use smelt_core::config::TimeseriesConfig;
use smelt_core::{Granularity, PartitionGrainConfig, PartitionGrainSafetyOverrides};
use smelt_runtime::windowing::{compute_incremental_windows, PartitionAxis, PartitionPoint};
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

fn date_of(p: PartitionPoint) -> chrono::NaiveDate {
    match p {
        PartitionPoint::Date(d) => d,
        PartitionPoint::Integer(_) => unreachable!("calendar-axis test"),
    }
}

/// Form-B: `d` (the partition column) is the driving event's own day, and the
/// declared reach lets a partition dated `d` be assembled from driving rows
/// on `d` through one day after — `Skew { before: 0, after: 1 day }`, the
/// same shape `examples/github_activity/models/silver/actor_sessions.sql`
/// declares.
const SESSION_SQL: &str = "SELECT d, SUM(amount) \
     FROM events \
     WHERE driving BETWEEN d AND d + INTERVAL '1 day' \
     GROUP BY 1";

/// Form A (identity): no partition-column skew at all.
const IDENTITY_SQL: &str = "SELECT date_trunc('day', event_time) as d, SUM(amount) \
     FROM events GROUP BY 1";

#[test]
fn interior_chunk_scan_carries_the_form_b_forward_reach() {
    let ts = make_ts("driving", "d", Granularity::Day);
    let inc = make_inc();
    // 8-day run, forced into 2-day batches -> deep-interior batches exist
    // (more than `after` = 1 day away from the derived output window's end).
    let range = make_range("2026-04-01", "2026-04-09");
    // Derived output window for this model: identity on the start side
    // (before = 0), skew-inverted by `after` = 1 day on the end side.
    let output_start = chrono::NaiveDate::from_ymd_opt(2026, 3, 31).unwrap();
    let output_end = chrono::NaiveDate::from_ymd_opt(2026, 4, 9).unwrap();

    let windows = compute_incremental_windows(
        &ts,
        &inc,
        SESSION_SQL,
        &no_dep_timeseries(),
        &range,
        PartitionAxis::Calendar,
        Some(2),
        false,
    )
    .unwrap();

    assert!(
        windows.batches.len() >= 4,
        "expected at least 4 batches from a 2-day chunk size, got {}",
        windows.batches.len()
    );
    assert_eq!(
        date_of(windows.batches.first().unwrap().partition_start),
        output_start
    );
    assert_eq!(
        date_of(windows.batches.last().unwrap().partition_end),
        output_end
    );

    // A batch whose own reach (`partition_end + after`) does not exceed
    // `output_end` gets the full forward reach; RED today (pre-fix,
    // scan_end never exceeds partition_end at all).
    let mid = &windows.batches[windows.batches.len() / 2];
    let partition_end = date_of(mid.partition_end);
    assert!(
        partition_end + chrono::Duration::days(1) <= output_end,
        "test fixture bug: chosen mid-batch is not deep-interior"
    );
    assert_eq!(
        date_of(mid.scan_end),
        partition_end + chrono::Duration::days(1),
        "a deep-interior batch's scan_end must reach partition_end + after"
    );
}

#[test]
fn outer_scan_envelope_is_unchanged_by_the_interior_widening() {
    let ts = make_ts("driving", "d", Granularity::Day);
    let inc = make_inc();
    let range = make_range("2026-04-01", "2026-04-09");

    let widened = compute_incremental_windows(
        &ts,
        &inc,
        SESSION_SQL,
        &no_dep_timeseries(),
        &range,
        PartitionAxis::Calendar,
        Some(2),
        false,
    )
    .unwrap();

    // A single-chunk invocation of the same run window: its one batch's
    // filter bounds are exactly today's (pre-fix) outer envelope, since the
    // write-side widening alone covers a single chunk incidentally.
    let single_chunk = compute_incremental_windows(
        &ts,
        &inc,
        SESSION_SQL,
        &no_dep_timeseries(),
        &range,
        PartitionAxis::Calendar,
        None,
        false,
    )
    .unwrap();
    assert_eq!(single_chunk.batches.len(), 1);
    let outer_scan_start = single_chunk.batches[0].scan_start;
    let outer_scan_end = single_chunk.batches[0].scan_end;

    let first = widened.batches.first().unwrap();
    let last = widened.batches.last().unwrap();
    assert_eq!(
        first.scan_start, outer_scan_start,
        "first batch's scan_start must equal the outer scan envelope (the clamp)"
    );
    assert_eq!(
        last.scan_end, outer_scan_end,
        "last batch's scan_end must equal the outer scan envelope (the clamp)"
    );
}

#[test]
fn chunking_is_invariant_for_a_form_b_model() {
    let ts = make_ts("driving", "d", Granularity::Day);
    let inc = make_inc();
    let range = make_range("2026-04-01", "2026-04-12");

    // The outer scan envelope (the skew bound applied once to the whole
    // invocation) is independent of `batch_size_days` — read `output_start`/
    // `output_end` once from a single-chunk invocation.
    let single_chunk = compute_incremental_windows(
        &ts,
        &inc,
        SESSION_SQL,
        &no_dep_timeseries(),
        &range,
        PartitionAxis::Calendar,
        None,
        false,
    )
    .unwrap();
    assert_eq!(single_chunk.batches.len(), 1);
    let output_start = date_of(single_chunk.batches[0].partition_start);
    let output_end = date_of(single_chunk.batches[0].partition_end);
    // The skew this fixture declares (`after` = 1 day, `before` = 0).
    let after_days = chrono::Duration::days(1);
    let before_days = chrono::Duration::days(0);

    let mut partition_covers = Vec::new();
    for batch_size in [Some(1u32), Some(3u32), None] {
        let windows = compute_incremental_windows(
            &ts,
            &inc,
            SESSION_SQL,
            &no_dep_timeseries(),
            &range,
            PartitionAxis::Calendar,
            batch_size,
            false,
        )
        .unwrap();

        let cover: Vec<(String, String)> = windows
            .batches
            .iter()
            .map(|b| (b.partition_start.to_string(), b.partition_end.to_string()))
            .collect();
        partition_covers.push(cover);

        // Every batch's scan bounds match the documented formula exactly:
        // scan_start = max(bs - before, output_start)
        // scan_end   = min(be + after, output_end)
        for b in &windows.batches {
            let bs = date_of(b.partition_start);
            let be = date_of(b.partition_end);
            let expected_scan_start = (bs - before_days).max(output_start);
            let expected_scan_end = (be + after_days).min(output_end);
            assert_eq!(date_of(b.scan_start), expected_scan_start);
            assert_eq!(date_of(b.scan_end), expected_scan_end);
        }
    }

    // Every sizing partitions the same overall span identically end-to-end
    // (first start / last end match) even though the interior chunk count
    // differs.
    for cover in &partition_covers {
        assert_eq!(
            cover.first().unwrap().0,
            partition_covers[0].first().unwrap().0
        );
        assert_eq!(
            cover.last().unwrap().1,
            partition_covers[0].last().unwrap().1
        );
    }
}

#[test]
fn zero_skew_model_batches_are_byte_identical() {
    let ts = make_ts("event_time", "d", Granularity::Day);
    let inc = make_inc();
    let range = make_range("2026-04-01", "2026-04-07");

    let windows = compute_incremental_windows(
        &ts,
        &inc,
        IDENTITY_SQL,
        &no_dep_timeseries(),
        &range,
        PartitionAxis::Calendar,
        Some(2),
        false,
    )
    .unwrap();

    for b in &windows.batches {
        assert_eq!(
            b.scan_start, b.partition_start,
            "an identity (Form-A) model has no skew: scan_start must equal partition_start"
        );
        assert_eq!(
            b.scan_end, b.partition_end,
            "an identity (Form-A) model has no skew: scan_end must equal partition_end"
        );
    }
}

/// A model with a nonzero SQL-inferred lookback but zero partition-column
/// skew — the shape `examples/web_analytics/tutorial_stages/03_late_data/
/// models/silver/events_parsed.sql` has — must keep `filter_start`/
/// `filter_end` (the lookback-only field) and `scan_start`/`scan_end` (the
/// skew-only field) numerically distinct sources of widening, never summed
/// into either field: `derive_batch_filtered_sql`
/// (`crate::execute::sources`) widens each bounded source's own scan by its
/// independently-derived lookback margin on top of `scan_start`/`scan_end`,
/// so folding the lookback into `scan_start`/`scan_end` too would double it
/// (`docs/outcomes/20260906-bigquery-correctness/phases/06-summary.md`).
const LOOKBACK_NO_SKEW_SQL: &str = "SELECT d, SUM(amount) \
     FROM events \
     WHERE d BETWEEN arrival - INTERVAL '3 days' AND arrival \
     GROUP BY 1";

#[test]
fn lookback_and_skew_widen_independently_never_summed() {
    let ts = make_ts("event_time", "d", Granularity::Day);
    let inc = make_inc();
    let range = make_range("2026-04-01", "2026-04-19");

    let windows = compute_incremental_windows(
        &ts,
        &inc,
        LOOKBACK_NO_SKEW_SQL,
        &no_dep_timeseries(),
        &range,
        PartitionAxis::Calendar,
        Some(9),
        false,
    )
    .unwrap();
    assert_eq!(windows.effective_window.lookback_days, 3);
    assert_eq!(
        windows.skew,
        smelt_logical::analysis::source_bounds::Skew::ZERO
    );
    assert!(
        windows.batches.len() >= 2,
        "expected at least 2 batches, got {}",
        windows.batches.len()
    );

    for b in &windows.batches {
        // filter_start carries the 3-day lookback alone.
        assert_eq!(
            date_of(b.filter_start),
            date_of(b.partition_start) - chrono::Duration::days(3),
            "filter_start must carry only the SQL-inferred lookback"
        );
        // scan_start carries no widening at all — zero skew.
        assert_eq!(
            b.scan_start, b.partition_start,
            "scan_start must equal partition_start when skew is zero, \
             regardless of a nonzero lookback"
        );
        assert_eq!(
            b.scan_end, b.partition_end,
            "scan_end must equal partition_end when skew is zero, \
             regardless of a nonzero lookback"
        );
    }
}
