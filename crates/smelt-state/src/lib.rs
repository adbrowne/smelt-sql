pub mod ddl_duckdb;
pub mod ddl_spark;
pub mod file_store;
pub mod history;
pub mod intervals;
pub mod landed_deltas;
pub mod reconciliation;
pub mod schema_tracking;
pub mod snapshot_store;

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
