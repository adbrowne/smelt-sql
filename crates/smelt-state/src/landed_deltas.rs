//! Per-source landed-delta recording — P10 v1 (`docs/specs/sources.md`
//! §"World-facts admission consumes"): what landed, per source, as
//! partition intervals on that source's own axis. This is the input to
//! cross-model forward propagation (`docs/specs/maintenance_plan.md`
//! §"The graph layer": "Runs are driven by **what landed**, per source, as
//! partition intervals on that source's own axis").
//!
//! **Derived-and-recorded, never declared.** A caller announces "this
//! source's own partition axis now covers `[start, end)`" (an append-only
//! clocked source's own landing event); the store computes the **interval
//! diff** against prior coverage — the sub-intervals genuinely new to this
//! landing — merges them into recorded coverage, and returns them as the
//! delta. A `mutable_snapshot` source (or an unclocked source with no
//! `timeseries:` clock at all) has no interval representation to diff
//! against — rows can change anywhere, not just at a frontier — so its
//! delta is always [`LandedDelta::WholeTable`], which propagates as
//! whole-model dirt downstream (`sources.md` ibid) rather than a silent
//! no-op.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::intervals::{find_gaps_in, merge_intervals_in, Interval};

/// One landing event's delta for a source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LandedDelta {
    /// The sub-intervals of the landed range not already reflected in
    /// prior coverage, on the source's own partition axis.
    Intervals(Vec<Interval>),
    /// No interval representation for this source — the whole table is the
    /// delta (a `mutable_snapshot` or unclocked source).
    WholeTable,
}

impl LandedDelta {
    /// A landing event that added nothing new (every sub-interval was
    /// already covered). Always `false` for `WholeTable` — a mutable
    /// snapshot's delta is never treated as empty, since it is a declared
    /// cost of the conservative posture, not an optimisation to skip
    /// (`maintenance_plan.md` §"Forward propagation": "A delta on an
    /// unclocked source dirties the whole model ... never a silent
    /// no-op").
    pub fn is_empty(&self) -> bool {
        matches!(self, LandedDelta::Intervals(v) if v.is_empty())
    }
}

/// Recorded landed-interval coverage for one append-only clocked source.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SourceLanding {
    /// Sorted, non-overlapping covered intervals — same shape and merge
    /// convention as `intervals::ModelIntervals::covered_intervals`.
    pub covered_intervals: Vec<Interval>,
}

impl SourceLanding {
    /// Record that `[start, end)` has landed on this source's own
    /// partition axis. Returns the interval diff against prior coverage
    /// (the genuinely new sub-intervals — possibly more than one, possibly
    /// none if `[start, end)` was already fully covered) and merges
    /// `[start, end)` into recorded coverage.
    pub fn record_landing(&mut self, start: &str, end: &str) -> LandedDelta {
        let new_intervals: Vec<Interval> = find_gaps_in(&self.covered_intervals, start, end)
            .into_iter()
            .map(|g| Interval {
                start: g.start.to_string(),
                end: g.end.to_string(),
            })
            .collect();

        self.covered_intervals.push(Interval {
            start: start.to_string(),
            end: end.to_string(),
        });
        merge_intervals_in(&mut self.covered_intervals);

        LandedDelta::Intervals(new_intervals)
    }
}

/// Whether a source's mutation posture has an interval representation at
/// all (`sources.md`: "a `mutable_snapshot` source has no interval
/// representation — its delta is 'the whole table'"). A structural mirror
/// of `smelt_core::sources::MutationProfile`'s discriminants (plus the
/// unclocked case) without `smelt-state` taking a dependency on
/// `smelt-core` for it — the same convention `reconciliation.rs`'s `group:
/// String` keying already uses for a cross-layer shape it stays
/// structurally compatible with rather than depends on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceMutationPosture {
    /// Rows are only ever added; recording is interval diffing.
    AppendOnly,
    /// The table is a mutable snapshot: rows may be updated or deleted in
    /// place with no change history — no interval representation.
    MutableSnapshot,
    /// No `timeseries:` clock declared at all — read in full on every
    /// recompute, same as a mutable snapshot for delta-recording purposes.
    Unclocked,
}

/// The full landed-delta store: source address -> recorded interval
/// coverage. Only `AppendOnly` sources ever gain an entry here — a
/// `mutable_snapshot`/`Unclocked` source's delta is always
/// [`LandedDelta::WholeTable`], computed directly by [`record_landing`]
/// without ever consulting or mutating this map, since there is nothing
/// for it to interval-diff against.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LandedDeltaStore {
    #[serde(flatten)]
    pub sources: HashMap<String, SourceLanding>,
}

impl LandedDeltaStore {
    /// Get or create the recorded coverage for `source_address`.
    pub fn get_or_create(&mut self, source_address: &str) -> &mut SourceLanding {
        self.sources.entry(source_address.to_string()).or_default()
    }

    /// Get the recorded coverage for `source_address`, read-only.
    pub fn get(&self, source_address: &str) -> Option<&SourceLanding> {
        self.sources.get(source_address)
    }
}

/// Record a landing event for `source_address` under `posture` — the single
/// seam a caller (`smelt-runtime`'s execute loop) goes through so an
/// append-only source's landing is always interval-diffed and a
/// mutable-snapshot/unclocked source's landing always returns
/// `LandedDelta::WholeTable`, never the other way around by omission.
pub fn record_landing(
    store: &mut LandedDeltaStore,
    source_address: &str,
    posture: SourceMutationPosture,
    start: &str,
    end: &str,
) -> LandedDelta {
    match posture {
        SourceMutationPosture::AppendOnly => store
            .get_or_create(source_address)
            .record_landing(start, end),
        SourceMutationPosture::MutableSnapshot | SourceMutationPosture::Unclocked => {
            LandedDelta::WholeTable
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_only_first_landing_covers_whole_range() {
        let mut landing = SourceLanding::default();
        let delta = landing.record_landing("2026-01-01", "2026-01-10");
        assert_eq!(
            delta,
            LandedDelta::Intervals(vec![Interval {
                start: "2026-01-01".to_string(),
                end: "2026-01-10".to_string(),
            }])
        );
        assert_eq!(landing.covered_intervals.len(), 1);
    }

    #[test]
    fn append_only_repeat_landing_diffs_to_empty() {
        let mut landing = SourceLanding::default();
        landing.record_landing("2026-01-01", "2026-01-10");
        let delta = landing.record_landing("2026-01-01", "2026-01-10");
        assert!(delta.is_empty(), "repeat landing should diff to no delta");
    }

    #[test]
    fn append_only_overlapping_landing_diffs_to_new_portion_only() {
        let mut landing = SourceLanding::default();
        landing.record_landing("2026-01-01", "2026-01-10");
        let delta = landing.record_landing("2026-01-05", "2026-01-15");
        assert_eq!(
            delta,
            LandedDelta::Intervals(vec![Interval {
                start: "2026-01-10".to_string(),
                end: "2026-01-15".to_string(),
            }])
        );
        // Covered range is now the merged union.
        assert_eq!(landing.covered_intervals.len(), 1);
        assert_eq!(landing.covered_intervals[0].end, "2026-01-15");
    }

    #[test]
    fn mutable_snapshot_never_interval_diffs() {
        let mut store = LandedDeltaStore::default();
        let delta = record_landing(
            &mut store,
            "sources.enrichment",
            SourceMutationPosture::MutableSnapshot,
            "2026-01-01",
            "2026-01-10",
        );
        assert_eq!(delta, LandedDelta::WholeTable);
        assert!(
            store.get("sources.enrichment").is_none(),
            "a mutable-snapshot landing must never be recorded into the interval store"
        );
    }

    #[test]
    fn unclocked_never_interval_diffs() {
        let mut store = LandedDeltaStore::default();
        let delta = record_landing(
            &mut store,
            "sources.lookup",
            SourceMutationPosture::Unclocked,
            "2026-01-01",
            "2026-01-10",
        );
        assert_eq!(delta, LandedDelta::WholeTable);
        assert!(store.get("sources.lookup").is_none());
    }
}
