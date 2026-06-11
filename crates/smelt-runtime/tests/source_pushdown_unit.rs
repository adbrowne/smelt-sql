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

use smelt_runtime::{build_source_bound_map, inject_source_filters, TimeRange};
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

    let source_bounds = build_source_bound_map(model_sql, &dep_ts);

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

    let source_bounds = build_source_bound_map(model_sql, &dep_ts);

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

    let source_bounds = build_source_bound_map(model_sql, &dep_ts);
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
