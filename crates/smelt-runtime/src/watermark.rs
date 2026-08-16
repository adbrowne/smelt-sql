//! Per-source watermark advancement (`docs/specs/run_state.md` §"Per-source
//! watermark"). Pure: consults only the consumer sets and completed-model
//! set a run already computed, plus the recorded watermark; performs no I/O
//! itself. The caller (`execute.rs`) is the single seam that reads/writes
//! `LandedDeltaStore` through `FileStore`, exactly as every other
//! `state.mode`-aware family does.

use std::collections::{BTreeMap, HashSet};

use smelt_state::landed_deltas::LandedDeltaStore;

/// Which sources' watermarks a just-completed run's window end (`window_end`,
/// ISO `YYYY-MM-DD`) advances, and to what.
///
/// A source advances only when **every** model in its consumer set
/// (`consumers_by_source`, direct downstream refs from the propagation
/// graph) is in `completed_models` — a selective run (any consumer excluded
/// by selection, or failed) never advances it, per `run_state.md`: "It
/// advances only on a run that completed every model consuming that source
/// within the propagation graph". A source not present in
/// `consumers_by_source` (nothing in this workspace refs it) is never
/// touched. The advance is also monotone — `window_end` must be strictly
/// past the recorded watermark, on the source's own ISO axis (string
/// compare, same convention `advance_watermark` itself uses).
pub fn watermark_advances(
    consumers_by_source: &BTreeMap<String, HashSet<String>>,
    completed_models: &HashSet<String>,
    window_end: &str,
    store: &LandedDeltaStore,
) -> Vec<(String, String)> {
    consumers_by_source
        .iter()
        .filter(|(_, consumers)| {
            !consumers.is_empty() && consumers.iter().all(|c| completed_models.contains(c))
        })
        .filter(|(source, _)| {
            store
                .watermark(source)
                .is_none_or(|current| window_end > current)
        })
        .map(|(source, _)| (source.clone(), window_end.to_string()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn consumers(pairs: &[(&str, &[&str])]) -> BTreeMap<String, HashSet<String>> {
        pairs
            .iter()
            .map(|(source, models)| {
                (
                    source.to_string(),
                    models.iter().map(|m| m.to_string()).collect(),
                )
            })
            .collect()
    }

    fn set(models: &[&str]) -> HashSet<String> {
        models.iter().map(|m| m.to_string()).collect()
    }

    #[test]
    fn watermark_advances_only_when_every_consumer_completed() {
        let by_source = consumers(&[("orders", &["staging_orders", "marts_orders"])]);
        let store = LandedDeltaStore::default();

        // Every consumer completed -> advances.
        let advances = watermark_advances(
            &by_source,
            &set(&["staging_orders", "marts_orders"]),
            "2026-02-01",
            &store,
        );
        assert_eq!(
            advances,
            vec![("orders".to_string(), "2026-02-01".to_string())]
        );

        // One consumer missing (selective run) -> no advance.
        let advances =
            watermark_advances(&by_source, &set(&["staging_orders"]), "2026-02-01", &store);
        assert!(advances.is_empty());

        // A failed/absent consumer -> no advance either.
        let advances = watermark_advances(&by_source, &set(&[]), "2026-02-01", &store);
        assert!(advances.is_empty());
    }

    #[test]
    fn already_past_watermark_does_not_advance() {
        let by_source = consumers(&[("orders", &["staging_orders"])]);
        let mut store = LandedDeltaStore::default();
        store.advance_watermark("orders", "2026-02-01");

        let advances =
            watermark_advances(&by_source, &set(&["staging_orders"]), "2026-01-15", &store);
        assert!(
            advances.is_empty(),
            "a window end at or before the recorded watermark must not advance it"
        );
    }
}
