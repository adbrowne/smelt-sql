//! Recorded per-source, per-partition frozen-band row-count baseline
//! (`docs/specs/incremental_models.md` §"The contract lattice", frozen
//! horizon): the row count observed, over a `frozen_horizon` model's clocked
//! sources restricted to their frozen band (partitions strictly before
//! `end - H`), the last time a run snapshotted it. This is the persisted
//! state `smelt_logical::contract::frozen_horizon::late_arrivals` compares
//! a source's *current* frozen-band state against
//! (`docs/outcomes/20260809-contract-lattice-v1/phases/03-plan.md`).
//!
//! Deliberately a dedicated store, not a reuse of
//! `smelt_state::source_postures::SourcePostureStore` — the two have
//! different refresh rules (posture refreshes only on a held verification;
//! this store refreshes after a held verification **and** after a reported
//! violation, so a genuine late arrival is reported once, not every
//! subsequent run) and must not cross-talk, even though their row shape is
//! the same (`docs/outcomes/20260809-contract-lattice-v1/phases/03-plan.md`
//! §"Store isolation").

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// One partition's recorded frozen-band row count for a source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrozenBandPartition {
    /// The partition column's value, as text.
    pub partition_value: String,
    /// The row count recorded for this partition the last time the
    /// frozen-band probe ran (established, held, or reported a violation).
    pub recorded_count: i64,
}

/// One source's recorded frozen-band partitions.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FrozenBandSource {
    pub partitions: Vec<FrozenBandPartition>,
}

/// The full frozen-band baseline store: source address -> recorded
/// per-partition frozen-band row counts.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FrozenBandBaselineStore {
    #[serde(flatten)]
    pub sources: HashMap<String, FrozenBandSource>,
}

impl FrozenBandBaselineStore {
    /// Replace `source`'s recorded frozen-band partitions wholesale with
    /// `partitions` — every run that dispatches the probe (established,
    /// held, or reported) re-records the CURRENT frozen-band state it just
    /// observed as the new baseline.
    pub fn record(&mut self, source: &str, partitions: Vec<FrozenBandPartition>) {
        self.sources
            .insert(source.to_string(), FrozenBandSource { partitions });
    }

    /// The recorded partitions for `source`, or `None` if nothing has been
    /// recorded yet (a source's first run has no baseline to compare
    /// against, so its first observation is unconditionally established).
    pub fn get(&self, source: &str) -> Option<&FrozenBandSource> {
        self.sources.get(source)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_round_trips_and_replaces_a_sources_partitions() {
        let mut store = FrozenBandBaselineStore::default();
        store.record(
            "raw.events",
            vec![FrozenBandPartition {
                partition_value: "2026-01-01".to_string(),
                recorded_count: 10,
            }],
        );
        assert_eq!(store.get("raw.events").unwrap().partitions.len(), 1);

        store.record(
            "raw.events",
            vec![FrozenBandPartition {
                partition_value: "2026-01-02".to_string(),
                recorded_count: 20,
            }],
        );
        let partitions = &store.get("raw.events").unwrap().partitions;
        assert_eq!(partitions.len(), 1);
        assert_eq!(partitions[0].partition_value, "2026-01-02");

        let json = serde_json::to_string(&store).unwrap();
        let loaded: FrozenBandBaselineStore = serde_json::from_str(&json).unwrap();
        assert_eq!(
            loaded.get("raw.events").unwrap().partitions[0].recorded_count,
            20
        );
    }

    #[test]
    fn get_is_none_for_a_source_with_no_recorded_baseline() {
        let store = FrozenBandBaselineStore::default();
        assert!(store.get("raw.unknown").is_none());
    }
}
