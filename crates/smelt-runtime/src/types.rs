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

    /// Run the planner incremental safety check before execution. When
    /// `true` (the default), a model whose SQL fails the safety classifier
    /// or whose temporal bounds are not derivable causes the run to be
    /// refused. Set to `false` to mirror `--allow-downgrade`: models fall
    /// back to full-table refresh with a warning.
    #[serde(default = "default_true")]
    pub enforce_safety: bool,

    /// Permit `ALTER TABLE … DROP COLUMN` during schema evolution. When
    /// `false` (the default), a column-removal diff blocks the run. Set to
    /// `true` to mirror `--allow-column-removal`.
    #[serde(default)]
    pub allow_column_removal: bool,

    /// Permit a full-table refresh when schema evolution requires one (e.g.
    /// a type change that cannot be migrated with ALTER TABLE). When `false`
    /// (the default), such a diff blocks the run. Set to `true` to mirror
    /// `--allow-full-refresh`.
    #[serde(default)]
    pub allow_full_refresh: bool,

    /// Pre-built ephemeral seed CTEs to inject into the ephemeral resolvers.
    ///
    /// Each entry is `(canonical_name, alias_with_cols, cte_body)`:
    /// - `canonical_name`: `address_segments.join("_")` — the lookup key for
    ///   ref resolution (e.g. `"lookup_regions"`).
    /// - `alias_with_cols`: the CTE alias with column list
    ///   (e.g. `"__smelt_lookup_regions(region_id, region_name)"`).
    /// - `cte_body`: the VALUES literal (e.g. `"VALUES\n  (1, 'AMER')"`).
    ///
    /// The CLI populates this from `discover_seed_infos_with_sidecars` when
    /// ephemeral seeds are present; the UI defaults to empty.
    #[serde(default)]
    pub ephemeral_seed_ctes: Vec<(String, String, String)>,
}

fn default_true() -> bool {
    true
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
    /// Resolved execution plan, populated when `ExecuteRequest::dry_run` is
    /// `true`. The plan lists each selected model with its resolved strategy
    /// (full refresh, incremental, cumulative, ephemeral, or skipped). When
    /// `dry_run` is `false` this is `None`.
    pub plan_summary: Option<PlanSummary>,
}

/// Resolved execution plan returned by `execute_project` when `dry_run = true`.
///
/// Summarises which strategy each model will use without invoking any backend.
/// Consumed by `commands/explain.rs` to render `--show-plan` output and by the
/// UI's `/api/run/plan` preview endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanSummary {
    /// Per-model entries in execution (topological) order.
    pub models: Vec<ModelPlanRecord>,
}

/// Plan entry for one model in a `PlanSummary`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelPlanRecord {
    /// Canonical model name (dot-path, e.g. `"silver.events_parsed"`).
    pub name: String,
    /// Resolved execution strategy for this model.
    pub strategy: ModelStrategy,
    /// Materialization type (`Table`, `View`, `Ephemeral`, etc.).
    pub materialization: smelt_core::config::Materialization,
    /// Canonical names of upstream dependencies (in the same canonical-path form).
    pub dependencies: Vec<String>,
}

/// Per-model execution strategy resolved during plan construction.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ModelStrategy {
    /// Drop and rebuild the table/view every run.
    FullRefresh,
    /// Incremental batch processing. Only possible when a time window is provided.
    Incremental {
        /// Column used to partition data (e.g. `"event_date"`).
        partition_column: String,
        /// Granularity label (e.g. `"day"`, `"week"`, `"month"`).
        granularity: String,
    },
    /// Cumulative aggregate merge loop.
    Cumulative,
    /// Inlined as a CTE into downstream models — not materialized directly.
    Ephemeral,
    /// Excluded from execution.
    Skipped {
        /// Human-readable reason (e.g. `"test model"`, `"generator file"`).
        reason: String,
    },
}
