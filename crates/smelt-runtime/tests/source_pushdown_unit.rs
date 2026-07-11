//! Unit tests for incremental batch source-filter pushdown (BUG-073 / Phase 3).
//!
//! These tests verify:
//! 1. `build_source_bound_map` + `inject_source_filters` compose correctly
//!    to produce source-read filters using the **run window** (not the write window).
//! 2. Per-source `before_secs`/`after_secs` from the SQL lookback pattern widens
//!    the run window appropriately.
//!
//! RED before Phase 3: the incremental batch loop does not call
//! `inject_source_filters`, so these unit tests verify the building blocks work
//! correctly and that the wiring (tested in the e2e test) is correct.

use smelt_core::config::TimeseriesConfig;
use smelt_core::{BatchedConfig, BatchedSafetyOverrides, Granularity};
use smelt_runtime::windowing::compute_incremental_windows;
use smelt_runtime::{build_source_bound_map, inject_source_filters, SourceBound, TimeRange};
use std::collections::HashMap;

// ── Test A: partition-local source gets exact run-window filter ───────────────

#[test]
fn incremental_batch_source_filter_uses_run_window() {
    let model_sql =
        "SELECT event_date, COUNT(*) AS cnt FROM smelt.sources.events GROUP BY event_date";

    // Simulate source_timeseries: "smelt.sources.events" has partition_column: event_date.
    // dep_timeseries maps full smelt ref → (address_segments, partition_column).
    let mut dep_ts: HashMap<String, (Vec<String>, String)> = HashMap::new();
    dep_ts.insert(
        "smelt.sources.events".to_string(),
        (
            vec!["sources".to_string(), "events".to_string()],
            "event_date".to_string(),
        ),
    );

    let (source_bounds, _warnings) = build_source_bound_map(model_sql, &dep_ts, None);

    // The source must appear in the bound map.
    assert!(
        source_bounds.contains_key("smelt.sources.events"),
        "source must appear in bound map: {:?}",
        source_bounds.keys().collect::<Vec<_>>()
    );

    // Inject source filters using the run window (partition_start / partition_end).
    let run_range = TimeRange {
        start: "2024-01-15".into(),
        end: "2024-01-16".into(),
    };

    let result = inject_source_filters(model_sql, &source_bounds, &run_range);

    // Must contain the source filter on the run window.
    assert!(
        result.contains("WHERE event_date >= '2024-01-15' AND event_date < '2024-01-16'"),
        "source filter must be on the run window; SQL:\n{result}"
    );

    // The subquery wrapper must be present.
    assert!(
        result.contains("(SELECT * FROM smelt.sources.events"),
        "subquery wrapper must be present; SQL:\n{result}"
    );
}

// ── Test B: run window (partition range) is used, not write window ────────────
//
// The write window (filter_start/filter_end) may be wider than the run window
// when a model has context_days (lookback). Source filters must use the run
// window so the source scan tracks the partition being produced, not the
// widened DELETE range.

#[test]
fn source_filter_uses_partition_range_not_filter_range() {
    // Model with a 1-day INTERVAL lookback on the source.
    let model_sql = "SELECT event_date, COUNT(*) AS cnt FROM smelt.sources.events WHERE event_date >= CURRENT_DATE - INTERVAL '1 day' GROUP BY event_date";

    let mut dep_ts: HashMap<String, (Vec<String>, String)> = HashMap::new();
    dep_ts.insert(
        "smelt.sources.events".to_string(),
        (
            vec!["sources".to_string(), "events".to_string()],
            "event_date".to_string(),
        ),
    );

    let (source_bounds, _warnings) = build_source_bound_map(model_sql, &dep_ts, None);

    // With 1-day lookback, before_secs must be 86400.
    let bound = source_bounds
        .get("smelt.sources.events")
        .expect("source must appear in bound map");

    assert_eq!(
        bound.before_secs, 86400,
        "1-day INTERVAL lookback must produce before_secs = 86400; got {}",
        bound.before_secs
    );

    // Run window [2024-01-15, 2024-01-16).
    let run_range = TimeRange {
        start: "2024-01-15".into(),
        end: "2024-01-16".into(),
    };

    let result = inject_source_filters(model_sql, &source_bounds, &run_range);

    // Source filter must extend 1 day before run_start → 2024-01-14.
    assert!(
        result.contains("'2024-01-14'"),
        "source filter must extend 1 day before run_start; expected '2024-01-14' in SQL:\n{result}"
    );
}

// ── Test C: empty dep_ts map produces unchanged SQL ───────────────────────────

#[test]
fn empty_source_timeseries_leaves_sql_unchanged() {
    let model_sql = "SELECT event_date FROM smelt.sources.events";
    let dep_ts: HashMap<String, (Vec<String>, String)> = HashMap::new();

    let (source_bounds, _warnings) = build_source_bound_map(model_sql, &dep_ts, None);
    assert!(
        source_bounds.is_empty(),
        "empty dep_ts must produce empty bounds map"
    );

    let run_range = TimeRange {
        start: "2024-01-15".into(),
        end: "2024-01-16".into(),
    };

    let result = inject_source_filters(model_sql, &source_bounds, &run_range);
    assert_eq!(result, model_sql, "empty bounds map must not modify SQL");
}

// ── Test D: a skewed batch's scan is sized from the DERIVED output window ────
//
// `docs/specs/model_transforms.md` §Constraints "Write window = output
// window; scan window ⊇ output window": every written partition's scan is
// sized from the derived output window's reach, never the run window's. A
// sessions-shaped model (`partition_column` skews ±1 day from the driving
// `event_date` column via a Form B relation) run for `[D, D+1)` derives an
// output window `[D-1, D+2)` (`windowing::compute_incremental_windows`); a
// source declaring its own symmetric 1-day lookback/lookahead on top of that
// must have its scan sized from the OUTPUT window's edges, i.e.
// `[D-2, D+3)` — not from the run window `[D, D+1)`'s edges (which would
// under-read the neighbour partition the skew reaches).
#[test]
fn skewed_batch_scan_sized_from_output_window() {
    let sql = "WITH sessionized AS (\
         SELECT device_id, event_date, session_start_ts, \
                CAST(session_start_ts AS DATE) AS session_start_date \
         FROM smelt.sources.events\
     ) \
     SELECT device_id, session_start_date, COUNT(*) AS event_count \
     FROM sessionized \
     WHERE event_date BETWEEN session_start_date - INTERVAL '1 day' \
         AND session_start_date + INTERVAL '1 day' \
     GROUP BY device_id, session_start_date";

    let ts = TimeseriesConfig {
        event_time_column: "session_start_date".to_string(),
        partition_column: "session_start_date".to_string(),
        granularity: Granularity::Day,
        week_start: None,
        assert_monotonic: false,
    };
    let inc = BatchedConfig {
        unique_key: vec![],
        nondeterministic_columns: vec![],
        safety_overrides: BatchedSafetyOverrides::default(),
    };
    let range = TimeRange {
        start: "2026-04-10".to_string(),
        end: "2026-04-11".to_string(),
    };

    let windows =
        compute_incremental_windows(&ts, &inc, sql, &HashMap::new(), 0, &range, None, false)
            .expect("sessions-shaped model must not be refused");

    assert_eq!(windows.batches.len(), 1, "expected a single batch");
    let batch = &windows.batches[0];
    assert_eq!(
        batch.partition_start.to_string(),
        "2026-04-09",
        "the derived output window must start 1 day before the run window (the skew's `after` reach)"
    );
    assert_eq!(
        batch.partition_end.to_string(),
        "2026-04-12",
        "the derived output window must end 1 day after the run window (the skew's `before` reach)"
    );

    // A source declaring its own symmetric 1-day lookback/lookahead on top
    // of the (already skew-widened) batch — the run window used for
    // pushdown is the batch's own [partition_start, partition_end), the
    // output-window batch itself, never the original [D, D+1) run window.
    let run_range = TimeRange {
        start: batch.partition_start.format("%Y-%m-%d").to_string(),
        end: batch.partition_end.format("%Y-%m-%d").to_string(),
    };
    let mut source_bounds: HashMap<String, SourceBound> = HashMap::new();
    source_bounds.insert(
        "smelt.sources.events".to_string(),
        SourceBound {
            partition_col: "event_date".to_string(),
            before_secs: 86400,
            after_secs: 86400,
        },
    );

    let result = inject_source_filters(sql, &source_bounds, &run_range);
    assert!(
        result.contains("'2026-04-08'"),
        "source scan must extend 1 day before the derived output window's start (2026-04-09 - 1 day); SQL:\n{result}"
    );
    assert!(
        result.contains("'2026-04-13'"),
        "source scan must extend 1 day past the derived output window's end (2026-04-12 + 1 day); SQL:\n{result}"
    );
    assert!(
        !result.contains("2026-04-11'"),
        "source scan must NOT be sized from the un-widened run window's own edges; SQL:\n{result}"
    );
}
