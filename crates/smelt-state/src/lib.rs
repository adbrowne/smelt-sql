pub mod ddl_bigquery;
pub mod ddl_duckdb;
pub mod ddl_spark;
pub mod file_store;
pub mod frozen_band_baselines;
pub mod history;
pub mod intervals;
pub mod landed_deltas;
pub mod migration_approvals;
pub mod reconciliation;
pub mod schema_tracking;
pub mod snapshot_store;
pub mod source_postures;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A single run manifest recording what smelt did during an execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunManifest {
    pub run_id: String,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub models: HashMap<String, ModelRunRecord>,
}

/// Record of a single model's execution within a run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelRunRecord {
    pub strategy: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_range: Option<TimeRangeRecord>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub partitions_updated: Vec<String>,
    pub row_count: usize,
    pub duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub batch_safety: Option<String>,
    /// Whether this model completed, errored, or was never attempted this
    /// run (`docs/specs/run_state.md` §"Run manifest"). Defaults to
    /// `Success` when reading a manifest written before this field existed —
    /// every entry the pre-Phase-7 writer ever produced really was a
    /// completed success or an explicit check-skip (never a silently
    /// dropped failure, since a failed run never reached `save_run` at all),
    /// so this default is exact, not a guess.
    #[serde(default = "default_outcome")]
    pub outcome: RunOutcomeKind,
    /// Hash of the model's compiled definition at run time
    /// (`smelt_state::intervals::compute_model_hash`), recorded for every
    /// entry regardless of outcome. `--resume` compares this against the
    /// model's current hash to decide whether a prior `success` still
    /// applies (`docs/specs/run_state.md` §"`--resume` semantics"). Defaults
    /// to empty when reading a pre-Phase-7 manifest that never recorded
    /// one — an empty hash never matches a real hash, so `--resume` always
    /// re-runs a model whose only history predates this field, which is the
    /// safe (never skip incorrectly) direction.
    #[serde(default)]
    pub definition_hash: String,
    /// Error text captured when `outcome` is `Failed`. `None` for
    /// `Success`/`Skipped` entries and for pre-Phase-8 manifests, which
    /// never recorded per-model error text.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub error: Option<String>,
    /// Number of retry attempts made for this model before its final
    /// outcome (0 if it succeeded/failed on the first attempt, or for
    /// pre-Phase-8 manifests that never recorded retries).
    #[serde(default)]
    pub retry_count: u32,
    /// Per-declaration probe cadence outcomes for this model's run
    /// (`docs/specs/run_state.md` §"Run manifest"; `docs/specs/
    /// model_properties.md` §"Probe cadence"). Defaults to empty for
    /// manifests written before probe dispatch was wired in.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub probes: Vec<ProbeRecord>,
    /// The `contract.deferral` pending window this run's write range proved
    /// it folded, ledger-proven work subsumption
    /// (`docs/specs/run_state.md` §"Run manifest",
    /// `docs/specs/incremental_models.md` §"The contract lattice"). `None`
    /// for every model without a declared `contract.deferral`, and for a
    /// run that folds work on schedule without ever having deferred it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subsumed: Option<SubsumedWindow>,
}

/// The dated bounds of a `contract.deferral` pending window a run's own
/// write range covered, recorded on the covering run's manifest entry
/// (`docs/specs/run_state.md` §"Run manifest").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubsumedWindow {
    /// The maintained frontier the pending window starts just after
    /// (exclusive), `YYYY-MM-DD`.
    pub maintained_exclusive: String,
    /// The input frontier the pending window ends at (inclusive),
    /// `YYYY-MM-DD`.
    pub input_inclusive: String,
}

/// One declared-fact probe's cadence outcome on this run
/// (`docs/specs/run_state.md` §"Run manifest").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProbeRecord {
    /// The declared fact this probe verifies, e.g. `key_recurrence`.
    pub fact: String,
    /// The registry's named diagnostic code, e.g. `KeyedRecurrenceBoundViolated`.
    pub probe: String,
    pub outcome: ProbeRecordOutcome,
}

/// Whether a probe actually ran this run, or was skipped by cadence policy
/// (`docs/specs/model_properties.md` §"Probe cadence": a policy skip trusts
/// the declaration and records it unverified, distinct from a probe that
/// cannot be built, which stays fail-closed).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProbeRecordOutcome {
    Dispatched,
    Skipped,
}

fn default_outcome() -> RunOutcomeKind {
    RunOutcomeKind::Success
}

/// Outcome of one model's attempted execution within a run
/// (`docs/specs/run_state.md` §"Run manifest").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunOutcomeKind {
    /// Completed without error.
    Success,
    /// The model's execution raised an error.
    Failed,
    /// Not attempted this run — upstream failure, selector exclusion, or a
    /// `--resume` short-circuit.
    Skipped,
}

/// Human/tooling-facing summary of one run, written alongside the run
/// manifest at `.smelt/targets/<target>/reports/<run_id>.json`
/// (`docs/specs/run_state.md` §"Run report"). Derived entirely from the
/// manifest — carries no information the manifest lacks — so it is always
/// reconstructible via [`RunReport::from_manifest`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunReport {
    pub run_id: String,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    /// `0` for an incomplete run (`completed_at` is `None`) — a cancelled or
    /// aborted run has no well-defined total duration to report.
    pub duration_ms: u64,
    pub outcome_counts: OutcomeCounts,
    /// One entry per `failed` model, carrying its manifest-recorded error
    /// text and retry count. Empty when nothing failed.
    pub failures: Vec<ModelFailure>,
}

/// Count of models by outcome (`docs/specs/run_state.md` §"Run report").
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct OutcomeCounts {
    pub success: usize,
    pub failed: usize,
    pub skipped: usize,
}

/// One `failed` model's recorded error, surfaced in the report so an
/// orchestrator or human doesn't need to reopen the manifest to see why a
/// run failed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelFailure {
    pub model: String,
    /// The manifest entry's `error` text, or a fixed placeholder for a
    /// `failed` entry recorded before `error` existed (pre-Phase-8
    /// manifests) — never a missing/null field, since a report naming a
    /// failed model with no explanation at all is worse than a placeholder
    /// that says so.
    pub error: String,
    pub retry_count: u32,
}

impl RunReport {
    /// Derive a report from a (typically just-finalized) manifest. Pure and
    /// total — never fails, since every field is either copied or summed
    /// from data the manifest already carries.
    pub fn from_manifest(manifest: &RunManifest) -> Self {
        let mut counts = OutcomeCounts::default();
        let mut failures = Vec::new();
        for (name, record) in &manifest.models {
            match record.outcome {
                RunOutcomeKind::Success => counts.success += 1,
                RunOutcomeKind::Skipped => counts.skipped += 1,
                RunOutcomeKind::Failed => {
                    counts.failed += 1;
                    failures.push(ModelFailure {
                        model: name.clone(),
                        error: record
                            .error
                            .clone()
                            .unwrap_or_else(|| "(no error text recorded)".to_string()),
                        retry_count: record.retry_count,
                    });
                }
            }
        }
        failures.sort_by(|a, b| a.model.cmp(&b.model));
        let duration_ms = match manifest.completed_at {
            Some(completed_at) => (completed_at - manifest.started_at)
                .num_milliseconds()
                .max(0) as u64,
            None => 0,
        };
        RunReport {
            run_id: manifest.run_id.clone(),
            started_at: manifest.started_at,
            completed_at: manifest.completed_at,
            duration_ms,
            outcome_counts: counts,
            failures,
        }
    }
}

/// A time range with start (inclusive) and end (exclusive) dates.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TimeRangeRecord {
    pub start: String,
    pub end: String,
}

/// Generate a run ID from the current time and a random suffix.
pub fn generate_run_id() -> String {
    let now = Utc::now();
    let timestamp = now.format("%Y%m%d-%H%M%S");
    let suffix: u32 = rand_suffix();
    format!("{}-{:06x}", timestamp, suffix)
}

fn rand_suffix() -> u32 {
    use std::time::SystemTime;
    // Use sub-microsecond time bits + process id for sufficient uniqueness
    // in non-concurrent use (only one smelt process runs at a time).
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    (nanos ^ std::process::id()) & 0xFFFFFF
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A manifest written before probe dispatch existed carries no
    /// `probes` key at all. Reading it must default to an empty list, not
    /// fail to deserialize (`docs/specs/run_state.md` §"Manifest evolution
    /// is backward-compatible").
    #[test]
    fn probe_records_default_empty_on_legacy_manifest() {
        let legacy_json = r#"{
            "strategy": "full_refresh",
            "row_count": 42,
            "duration_ms": 100,
            "outcome": "success",
            "definition_hash": "abc123",
            "retry_count": 0
        }"#;
        let record: ModelRunRecord =
            serde_json::from_str(legacy_json).expect("legacy manifest entry must still parse");
        assert!(record.probes.is_empty());
    }
}
