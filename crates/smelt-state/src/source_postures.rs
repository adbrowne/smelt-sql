//! Recorded per-source, per-partition append-only posture baseline
//! (`docs/specs/model_properties.md` §"Probe obligation", row
//! `mutation_profile.kind: append_only`; `docs/specs/sources.md` §Semantics
//! 4): the row count and content fingerprint observed the last time a run
//! verified the source's declared append-only posture. This is the
//! persisted state the append-only posture probe compares the source's
//! *current* per-partition state against
//! (`docs/outcomes/20260809-probe-backed-facts/outcome.md` phase 6).
//!
//! This store's row type is intentionally independent of
//! `smelt-logical::maintenance::emit::AppendOnlyBaselinePartition` — this
//! crate sits below `smelt-logical` in the layering (`CLAUDE.md`
//! §"Architectural invariants" — Layered single-ownership), so it cannot
//! name that type. `smelt-runtime` converts between the two.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// One partition's recorded posture baseline for a source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourcePosturePartition {
    /// The partition column's value, as text.
    pub partition_value: String,
    /// The row count recorded for this partition the last time the
    /// posture probe held.
    pub recorded_count: i64,
    /// The row-content fingerprint (hex `sha256`) recorded for this
    /// partition the last time the posture probe held.
    pub recorded_fingerprint: String,
}

/// One source's recorded partitions.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SourcePosture {
    pub partitions: Vec<SourcePosturePartition>,
}

/// The full source-posture store: source address -> recorded per-partition
/// baseline.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SourcePostureStore {
    #[serde(flatten)]
    pub sources: HashMap<String, SourcePosture>,
}

impl SourcePostureStore {
    /// Replace `source`'s recorded partitions wholesale with `partitions` —
    /// a run that dispatched and held the posture probe re-records the
    /// CURRENT per-partition state it just verified as the new baseline
    /// (`outcome.md` phase 6 decision log: "Recording happens only on a run
    /// that actually dispatched and held").
    pub fn record(&mut self, source: &str, partitions: Vec<SourcePosturePartition>) {
        self.sources
            .insert(source.to_string(), SourcePosture { partitions });
    }

    /// The recorded partitions for `source`, or `None` if nothing has been
    /// recorded yet (a source's first run has no baseline to compare
    /// against, so builds no probe).
    pub fn get(&self, source: &str) -> Option<&SourcePosture> {
        self.sources.get(source)
    }

    /// `source`'s recorded baseline with the frontier gate applied
    /// (`outcome.md` phase 6 decision log: "the *count* leg applies to
    /// every recorded partition; the *fingerprint* leg applies only to
    /// partitions strictly below the recorded maximum partition value").
    /// Every recorded partition gets a row; only partitions whose
    /// `partition_value` is strictly less than the recorded maximum (string
    /// comparison, matching how partition values are already compared
    /// elsewhere as strings, e.g. `landed_deltas.rs`'s `Interval`
    /// start/end) get `check_fingerprint: true` — the max partition itself
    /// is still open to legitimate appends, so only its count is checked.
    /// Returns an empty `Vec` if `source` has no recorded baseline.
    pub fn closed_baseline(&self, source: &str) -> Vec<ClosedBaselinePartition> {
        let Some(posture) = self.get(source) else {
            return Vec::new();
        };
        let Some(max_value) = posture.partitions.iter().map(|p| &p.partition_value).max() else {
            return Vec::new();
        };
        posture
            .partitions
            .iter()
            .map(|p| ClosedBaselinePartition {
                partition_value: p.partition_value.clone(),
                recorded_count: p.recorded_count,
                recorded_fingerprint: p.recorded_fingerprint.clone(),
                check_fingerprint: &p.partition_value < max_value,
            })
            .collect()
    }
}

/// [`SourcePostureStore::closed_baseline`]'s output row: a recorded
/// partition plus whether the frontier gate admits its fingerprint into the
/// comparison this run. Deliberately shaped to convert 1:1 into
/// `smelt_logical::maintenance::emit::AppendOnlyBaselinePartition` without
/// this crate depending on `smelt-logical` to name that type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClosedBaselinePartition {
    pub partition_value: String,
    pub recorded_count: i64,
    pub recorded_fingerprint: String,
    pub check_fingerprint: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_round_trips_and_replaces_a_sources_partitions() {
        let mut store = SourcePostureStore::default();
        store.record(
            "sources.events",
            vec![SourcePosturePartition {
                partition_value: "2026-01-01".to_string(),
                recorded_count: 10,
                recorded_fingerprint: "abc".to_string(),
            }],
        );
        assert_eq!(store.get("sources.events").unwrap().partitions.len(), 1);

        // Recording again replaces wholesale, not merges.
        store.record(
            "sources.events",
            vec![SourcePosturePartition {
                partition_value: "2026-01-02".to_string(),
                recorded_count: 20,
                recorded_fingerprint: "def".to_string(),
            }],
        );
        let partitions = &store.get("sources.events").unwrap().partitions;
        assert_eq!(partitions.len(), 1);
        assert_eq!(partitions[0].partition_value, "2026-01-02");

        // Round trip through JSON.
        let json = serde_json::to_string(&store).unwrap();
        let loaded: SourcePostureStore = serde_json::from_str(&json).unwrap();
        assert_eq!(
            loaded.get("sources.events").unwrap().partitions[0].recorded_count,
            20
        );
    }

    #[test]
    fn closed_baseline_marks_every_partition_but_the_max_as_fingerprint_checked() {
        let mut store = SourcePostureStore::default();
        store.record(
            "sources.events",
            vec![
                SourcePosturePartition {
                    partition_value: "2026-01-01".to_string(),
                    recorded_count: 10,
                    recorded_fingerprint: "a".to_string(),
                },
                SourcePosturePartition {
                    partition_value: "2026-01-03".to_string(),
                    recorded_count: 5,
                    recorded_fingerprint: "c".to_string(),
                },
                SourcePosturePartition {
                    partition_value: "2026-01-02".to_string(),
                    recorded_count: 8,
                    recorded_fingerprint: "b".to_string(),
                },
            ],
        );
        let closed = store.closed_baseline("sources.events");
        assert_eq!(closed.len(), 3);
        for row in &closed {
            let expected_checked = row.partition_value != "2026-01-03";
            assert_eq!(
                row.check_fingerprint, expected_checked,
                "partition {} check_fingerprint mismatch",
                row.partition_value
            );
        }
    }

    #[test]
    fn closed_baseline_is_empty_for_a_source_with_no_recorded_baseline() {
        let store = SourcePostureStore::default();
        assert!(store.closed_baseline("sources.unknown").is_empty());
    }
}
