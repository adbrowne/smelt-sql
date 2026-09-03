//! Recorded per-source, whole-source content-fingerprint baseline
//! (`docs/specs/incremental_models.md` §"When a mutation cell dispatches"):
//! the row count and content fingerprint observed the last time a run
//! dispatched an `UpstreamMutation` cell for that source. This is the
//! persisted state a run's mutation-happened discrimination compares the
//! source's *current* whole-source state against, before deciding whether
//! the cell genuinely needs to dispatch this run.
//!
//! This store's row type is intentionally independent of
//! `smelt-logical::maintenance::emit`'s statement construction — this crate
//! sits below `smelt-logical` in the layering (`CLAUDE.md`
//! §"Architectural invariants" — Layered single-ownership), so it cannot
//! name that layer's types. `smelt-runtime` converts between the two.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// One source's recorded mutation-fingerprint baseline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceMutationBaseline {
    /// The whole-source row count recorded the last time this source's
    /// `UpstreamMutation` cell dispatched.
    pub recorded_count: i64,
    /// The whole-source row-content fingerprint (hex `sha256`) recorded the
    /// last time this source's `UpstreamMutation` cell dispatched.
    pub recorded_fingerprint: String,
    /// The digest column set the recorded fingerprint was computed over. A
    /// baseline recorded under a different digest-column set is
    /// incomparable to a current fingerprint computed under today's set —
    /// never a silent skip (`docs/specs/incremental_models.md` §"When a
    /// mutation cell dispatches").
    pub digest_columns: Vec<String>,
}

/// The full source-mutation store: source address -> recorded baseline.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SourceMutationStore {
    #[serde(flatten)]
    sources: HashMap<String, SourceMutationBaseline>,
}

impl SourceMutationStore {
    /// Replace `source`'s recorded baseline wholesale with `baseline` — a
    /// run that dispatched the mutation cell re-records the CURRENT
    /// whole-source state it just observed as the new baseline (same
    /// discipline as `SourcePostureStore::record`: recording happens only
    /// on a run that actually dispatched).
    pub fn record(&mut self, source: &str, baseline: SourceMutationBaseline) {
        self.sources.insert(source.to_string(), baseline);
    }

    /// The recorded baseline for `source`, or `None` if nothing has been
    /// recorded yet (a source's first eligible run has no baseline to
    /// compare against, so unconditionally dispatches).
    pub fn get(&self, source: &str) -> Option<&SourceMutationBaseline> {
        self.sources.get(source)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_round_trips_and_replaces_a_sources_baseline() {
        let mut store = SourceMutationStore::default();
        assert!(store.get("sources.events").is_none());

        store.record(
            "sources.events",
            SourceMutationBaseline {
                recorded_count: 10,
                recorded_fingerprint: "abc".to_string(),
                digest_columns: vec!["event_id".to_string()],
            },
        );
        assert_eq!(store.get("sources.events").unwrap().recorded_count, 10);

        // Recording again replaces wholesale, not merges.
        store.record(
            "sources.events",
            SourceMutationBaseline {
                recorded_count: 20,
                recorded_fingerprint: "def".to_string(),
                digest_columns: vec!["event_id".to_string()],
            },
        );
        let baseline = store.get("sources.events").unwrap();
        assert_eq!(baseline.recorded_count, 20);
        assert_eq!(baseline.recorded_fingerprint, "def");

        // Round trip through JSON.
        let json = serde_json::to_string(&store).unwrap();
        let loaded: SourceMutationStore = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded.get("sources.events").unwrap().recorded_count, 20);
    }

    #[test]
    fn get_on_an_unrecorded_source_is_none() {
        let store = SourceMutationStore::default();
        assert!(store.get("sources.unknown").is_none());
    }
}
