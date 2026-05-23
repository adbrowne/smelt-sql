//! Plain-data request and outcome types for the run pipeline.
//!
//! `ExecuteRequest` is the superset of today's CLI and UI run-request
//! shapes — the union of fields each consumer needs. Consumer surfaces
//! convert their native shape (clap args, HTTP body) into this struct and
//! pass it to `execute_project`.
//!
//! `RunOutcome` is the final result of a successful run, mirroring what the
//! CLI prints and what the UI's `/api/run/status` endpoint surfaces.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use smelt_state::ModelRunRecord;
use std::collections::HashMap;

/// Unified run request consumed by `execute_project`. Both `smelt-cli`'s
/// `commands/run.rs` and `smelt-ui`'s `run_manager.rs` convert their native
/// argument shapes into this struct.
///
/// Fields are a superset of what either consumer needs today; consumer
/// surfaces leave unsupported fields at their defaults.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecuteRequest {
    /// Target name from `smelt.yml`. Required.
    pub target: String,

    /// Selector strings (`+model+`, `tag:wip`, plain model names, etc.).
    /// Empty selects all non-test, non-generator models in the project.
    #[serde(default)]
    pub select: Vec<String>,

    /// Exclude selector strings, applied after `select`.
    #[serde(default)]
    pub exclude: Vec<String>,

    /// Run window start, ISO `YYYY-MM-DD`. `None` triggers full-refresh on
    /// incremental models or is ignored for non-temporal models.
    #[serde(default)]
    pub start: Option<String>,

    /// Run window end (exclusive), ISO `YYYY-MM-DD`.
    #[serde(default)]
    pub end: Option<String>,

    /// Override the planner's derived batch size; in days.
    #[serde(default)]
    pub batch_size_days: Option<u32>,

    /// Run one batch per timeseries partition rather than the planner-chosen
    /// chunk size. UI exposes this as a checkbox; CLI as `--per-partition`.
    #[serde(default)]
    pub per_partition: bool,

    /// Drop and rebuild every selected incremental model rather than
    /// applying batches. UI exposes this; CLI as `--full-refresh`.
    #[serde(default)]
    pub full_refresh: bool,

    /// Compute the run plan and report what *would* run without invoking
    /// any backend. Used by the UI's `/api/run/plan` preview endpoint.
    #[serde(default)]
    pub dry_run: bool,
}

/// Outcome of a completed run. The runtime returns this from
/// `execute_project`; the consumer's reporter has already received the
/// fine-grained events.
#[derive(Debug, Clone, Serialize)]
pub struct RunOutcome {
    pub run_id: String,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    /// Per-model records, indexed by model name. Same shape as
    /// `smelt-state`'s `RunManifest::models` so the runtime can pass the
    /// manifest's contents straight through.
    pub models: HashMap<String, ModelRunRecord>,
    /// Sum of row counts across all models. Tests assert on this; the UI
    /// surfaces it as the run total.
    pub total_rows: usize,
}
