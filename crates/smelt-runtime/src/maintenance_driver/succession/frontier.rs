//! Recording the succession window-forward driver's own maintained-interval
//! and per-source landed-delta frontiers — the write the ordinary
//! `plan.incremental` path already performs after every successful fold
//! (`crates/smelt-runtime/src/execute/project/mod.rs`, the interval-store
//! and landed-delta blocks following the manifest insert), lifted here so
//! `contract_probes::resolve_deferral_frontiers` reads a real maintained
//! frontier for a succession model instead of always `None`
//! (`docs/outcomes/20260906-scd2-keyed-succession/phases/06b-plan.md`,
//! closing the gap phase 7d recorded). The rebuild path (`--full-refresh`/
//! `smelt rebuild`) has no run window and never calls this — only the
//! window-forward branch does.

use smelt_logical::maintenance::{MutationProfile, SourceFacts};
use smelt_state::file_store::FileStore;
use smelt_state::landed_deltas::{record_landing, SourceMutationPosture};
use smelt_state::{ModelRunRecord, RunOutcomeKind, TimeRangeRecord};

/// Mirrors the ordinary incremental path's two whole-store critical
/// sections exactly: an interval-store `get_or_create` + `record_interval`,
/// then a per-source `record_landing` keyed off each source's own
/// [`MutationProfile`]. No new posture logic — lifted, not re-derived. This
/// phase wires the model-level frontier only; whether a succession cell
/// needs its own `contract.cells[].deferral` frontier is left to a future
/// phase (a succession model derives exactly one cell, so there is no
/// per-cell frontier to distinguish from the model-level one today).
pub(crate) async fn record_succession_frontiers(
    file_store: &FileStore,
    state_io_lock: &tokio::sync::Mutex<()>,
    model_name: &str,
    model_hash: &str,
    source_facts: &[SourceFacts],
    start_str: &str,
    end_str: &str,
) {
    {
        let _io_guard = state_io_lock.lock().await;
        if let Ok(mut interval_store) = file_store.load_intervals() {
            interval_store
                .get_or_create(model_name, model_hash)
                .record_interval(start_str, end_str);
            let _ = file_store.save_intervals(&interval_store);
        }
    }

    if !start_str.is_empty() && !end_str.is_empty() {
        let _io_guard = state_io_lock.lock().await;
        if let Ok(mut landed_deltas) = file_store.load_landed_deltas() {
            for sf in source_facts {
                let posture = if sf.partition_col.is_none() {
                    SourceMutationPosture::Unclocked
                } else {
                    match sf.mutation {
                        MutationProfile::AppendOnly => SourceMutationPosture::AppendOnly,
                        MutationProfile::MutableSnapshot | MutationProfile::ChangeFeed => {
                            SourceMutationPosture::MutableSnapshot
                        }
                    }
                };
                record_landing(&mut landed_deltas, &sf.name, posture, start_str, end_str);
            }
            let _ = file_store.save_landed_deltas(&landed_deltas);
        }
    }
}

/// The succession dispatch's `ModelRunRecord` constructor — moved out of
/// `execute/project/mod.rs` (at its large-file baseline) to pay for the
/// [`record_succession_frontiers`] call site added there. `probes` is the
/// run's own accumulated append-only posture dispatch, from
/// [`super::dispatch_succession_source_probes`] — no longer hardcoded empty
/// (`docs/outcomes/20260906-scd2-keyed-succession/phases/06c-plan.md`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_succession_run_record(
    strategy: &str,
    time_range: Option<TimeRangeRecord>,
    row_count: usize,
    duration_ms: u64,
    definition_hash: String,
    probes: Vec<smelt_state::ProbeRecord>,
) -> ModelRunRecord {
    ModelRunRecord {
        strategy: strategy.to_string(),
        time_range,
        partitions_updated: vec![],
        row_count,
        duration_ms,
        batch_safety: Some("succession".to_string()),
        outcome: RunOutcomeKind::Success,
        definition_hash,
        error: None,
        retry_count: 0,
        probes,
        subsumed: None,
        deferred_cells: Vec::new(),
    }
}
