//! Per-source landed-delta recording (P10 v1: `docs/specs/sources.md`
//! §"World-facts admission consumes"): append-only landing is recorded as
//! an interval diff on the source's own partition axis; a mutable-snapshot
//! source has no interval representation and its delta is always "the
//! whole table".

use smelt_state::intervals::Interval;
use smelt_state::landed_deltas::{
    record_landing, LandedDelta, LandedDeltaStore, SourceLanding, SourceMutationPosture,
};

/// An append-only clocked source's landing is recorded as the interval
/// diff against prior coverage: a first landing's delta is the whole
/// landed range; a landing overlapping prior coverage diffs to only the
/// genuinely new sub-interval, and prior coverage is merged forward.
#[test]
fn append_only_landing_records_interval_diff() {
    let mut landing = SourceLanding::default();

    let first = landing.record_landing("2026-01-01", "2026-01-10");
    assert_eq!(
        first,
        LandedDelta::Intervals(vec![Interval {
            start: "2026-01-01".to_string(),
            end: "2026-01-10".to_string(),
        }])
    );

    // Landing an overlapping range diffs to only the new portion.
    let second = landing.record_landing("2026-01-05", "2026-01-20");
    assert_eq!(
        second,
        LandedDelta::Intervals(vec![Interval {
            start: "2026-01-10".to_string(),
            end: "2026-01-20".to_string(),
        }])
    );
    assert_eq!(landing.covered_intervals.len(), 1);
    assert_eq!(landing.covered_intervals[0].start, "2026-01-01");
    assert_eq!(landing.covered_intervals[0].end, "2026-01-20");

    // Re-landing an already-fully-covered range diffs to no delta at all —
    // never a silent full-table fallback for an append-only source.
    let repeat = landing.record_landing("2026-01-01", "2026-01-10");
    assert!(repeat.is_empty());

    // The store-level seam (`record_landing`) routes an `AppendOnly`
    // posture through the same interval-diff path, keyed by source address.
    let mut store = LandedDeltaStore::default();
    let delta = record_landing(
        &mut store,
        "sources.orders",
        SourceMutationPosture::AppendOnly,
        "2026-02-01",
        "2026-02-05",
    );
    assert_eq!(
        delta,
        LandedDelta::Intervals(vec![Interval {
            start: "2026-02-01".to_string(),
            end: "2026-02-05".to_string(),
        }])
    );
    assert!(store.get("sources.orders").is_some());
}

/// A `mutable_snapshot` source has no interval representation — its
/// delta is always "the whole table" (`docs/specs/sources.md`: "a
/// `mutable_snapshot` source has no interval representation — its delta is
/// 'the whole table'"), and it is never recorded into the interval store
/// (nothing to diff a future landing against). An unclocked source (no
/// `timeseries:` clock at all) gets the same treatment, never a silent
/// no-op.
#[test]
fn mutable_snapshot_delta_is_whole_table() {
    let mut store = LandedDeltaStore::default();

    let delta = record_landing(
        &mut store,
        "sources.customer_snapshot",
        SourceMutationPosture::MutableSnapshot,
        "2026-01-01",
        "2026-01-10",
    );
    assert_eq!(delta, LandedDelta::WholeTable);
    assert!(
        store.get("sources.customer_snapshot").is_none(),
        "a mutable-snapshot landing must never gain an interval-store entry"
    );

    let unclocked_delta = record_landing(
        &mut store,
        "sources.lookup_table",
        SourceMutationPosture::Unclocked,
        "2026-01-01",
        "2026-01-10",
    );
    assert_eq!(unclocked_delta, LandedDelta::WholeTable);
    assert!(store.get("sources.lookup_table").is_none());
}

/// The per-source watermark (`docs/specs/run_state.md` §"Per-source
/// watermark") serialises alongside the landed-interval coverage and never
/// regresses: an earlier `to` passed to `advance_watermark` is a no-op.
#[test]
fn watermark_field_roundtrips_and_never_regresses() {
    let mut store = LandedDeltaStore::default();
    assert_eq!(store.watermark("sources.orders"), None);

    store.advance_watermark("sources.orders", "2026-01-10");
    assert_eq!(store.watermark("sources.orders"), Some("2026-01-10"));

    store.advance_watermark("sources.orders", "2026-01-05");
    assert_eq!(
        store.watermark("sources.orders"),
        Some("2026-01-10"),
        "an earlier watermark must never regress the recorded one"
    );

    store.advance_watermark("sources.orders", "2026-01-20");
    assert_eq!(store.watermark("sources.orders"), Some("2026-01-20"));

    let json = serde_json::to_string(&store).expect("serialize");
    let reloaded: LandedDeltaStore = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(reloaded.watermark("sources.orders"), Some("2026-01-20"));
}
