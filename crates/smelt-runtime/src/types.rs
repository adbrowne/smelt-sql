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
use smelt_core::discovery::ModelFile;
use smelt_state::ModelRunRecord;
use std::collections::{BTreeMap, HashMap};

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

    /// When `true`, run `smelt.check`s after each model materializes (build
    /// semantics). `false` for `smelt run` and `smelt rebuild` (run
    /// semantics — checks are a build/check concern, not a run concern).
    #[serde(default)]
    pub run_checks: bool,

    /// Check model files to run during the build. Populated by `smelt build`
    /// from discovery; excluded from the dependency graph (supplied separately
    /// so fresh discovery inside `execute_project` is never needed).
    /// Skipped during serde: `ModelFile` is a runtime type, not a wire type.
    #[serde(skip)]
    pub checks: Vec<ModelFile>,

    /// Maximum number of models to execute concurrently. `None` (the
    /// default) resolves to the host's available parallelism at run time;
    /// `Some(1)` forces strictly serial execution, one model at a time, in
    /// `execution_order` — the pre-Phase-5 behaviour. A DAG-independent
    /// (unrelated) pair of models may run concurrently whenever `jobs > 1`;
    /// a dependency edge always keeps the upstream model's completion
    /// strictly before its downstream's start regardless of `jobs`.
    #[serde(default)]
    pub jobs: Option<usize>,

    /// Maximum retry attempts for a transient backend failure
    /// (`BackendError::is_transient`) hit while executing one model's
    /// statement group — the whole group (drop+create, or a batch's
    /// DELETE+INSERT/MERGE) is re-run, never a partial slice of it, so a
    /// retry can never leave a half-applied write behind
    /// (`docs/plans/20260719-prod-w2-operability.md` Phase 6). `None` (the
    /// default) resolves to 3 attempts. `Some(0)` disables retry entirely —
    /// the first transient failure fails the model immediately, matching
    /// pre-Phase-6 behaviour. A deterministic (non-transient) error is never
    /// retried regardless of this value.
    #[serde(default)]
    pub retry_max: Option<u32>,

    /// Base backoff, in milliseconds, for the exponential-backoff delay
    /// between retry attempts (`delay = retry_backoff_ms * 2^(attempt-1) +
    /// jitter`; jitter is derived deterministically from a hash of
    /// `(run_id, model_name, attempt)` — never `Instant::now`/`SystemTime`,
    /// so retry delays are reproducible in tests). `None` (the default)
    /// resolves to 200ms.
    #[serde(default)]
    pub retry_backoff_ms: Option<u64>,

    /// Resume a previously partially-failed run for this target: a selected
    /// model is skipped when it succeeded in the most recent incomplete run
    /// (the latest manifest with `completed_at: null`) with an unchanged
    /// `definition_hash`; everything else — a model that was `failed` or
    /// `skipped`, whose definition changed, or that is downstream of any
    /// such model — re-runs. `false` (the default) is a plain full run.
    /// Refuses (`execute_project` returns `Err`) when there is no
    /// incomplete run to resume from, rather than silently running
    /// everything (`docs/specs/run_state.md` §"`--resume` semantics").
    #[serde(default)]
    pub resume: bool,

    /// Request-scoped hard pins for `smelt bakeoff`'s offline forcing seam
    /// (`docs/plans/20260719-prod-w7-bakeoff.md` Phase 3): each entry forces
    /// one cell's technique for this run only. Entries enter the choice
    /// ladder narrower than any frontmatter `maintenance.cells[]` pin for
    /// the same cell — a request override for a cell also pinned in
    /// frontmatter wins — but admission is still enforced exactly like a
    /// frontmatter pin: an override naming a technique the derived plan did
    /// not admit refuses loud, never silently substitutes. Empty (the
    /// default) leaves resolution byte-identical to a request with no
    /// overrides at all.
    #[serde(default)]
    pub technique_overrides: Vec<CellTechniqueOverride>,

    /// Propagated keyed-restriction channel for `smelt run --since-upstream`
    /// (`docs/specs/incremental_models.md` §"Restrictions compose by
    /// union"): per consumer model name, the resolved key values a prior
    /// `--since-upstream` plan propagated for one of its key-addressed
    /// upstream edges. At dispatch, a key-addressed run unit's read
    /// restriction is the **union** of these values with the cell's own
    /// live affected-key discovery — never an intersection. Populated by
    /// [`crate::propagation::keyed_restrictions_from_plan`]; empty (the
    /// default) leaves resolution byte-identical to a request with no
    /// restrictions at all.
    #[serde(default)]
    pub keyed_restrictions: BTreeMap<String, Vec<KeyedRestriction>>,
}

fn default_true() -> bool {
    true
}

/// One propagated keyed restriction on a key-addressed model edge — see
/// [`ExecuteRequest::keyed_restrictions`]. `upstream` names the edge (the
/// upstream model's bare address) this restriction targets, matching
/// [`smelt_logical::maintenance::KeyScope::from`]; `keys`/`values` are the
/// downstream's own key columns and the resolved key values propagated for
/// them. A plain serde wire type, not a re-export of
/// `smelt_logical::maintenance::propagate::KeyedDirt` (mirrors
/// [`CellTechniqueOverride`]'s own precedent).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyedRestriction {
    pub upstream: String,
    pub keys: Vec<String>,
    pub values: Vec<String>,
}

/// One request-scoped hard pin — see [`ExecuteRequest::technique_overrides`].
/// Mirrors `smelt_core::config::MaintenanceCellConfig`'s `columns`/`on`
/// matching shape but carries only a hard `technique` pin (no soft
/// `prefer`, no `write` addressing pin — those stay frontmatter-only).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CellTechniqueOverride {
    /// Names any member of the derived column group this override
    /// addresses — same matching rule as `MaintenanceCellConfig::columns`.
    pub columns: Vec<String>,
    /// The trigger this override targets: a `<source-address>` or the
    /// literal `backfill`.
    pub on: String,
    /// The hard-pinned technique. Bypasses the cost model, never admission.
    pub technique: smelt_core::config::CellTechnique,
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
    /// (full refresh, incremental, keyed, ephemeral, or skipped). When
    /// `dry_run` is `false` this is `None`.
    pub plan_summary: Option<PlanSummary>,

    /// Check results collected during the build, in model-execution order.
    /// Empty when `ExecuteRequest::run_checks` is `false` (i.e. `smelt run`).
    pub check_results: Vec<crate::check_runner::CheckOutcome>,
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
    /// Keyed merge loop.
    Keyed,
    /// Inlined as a CTE into downstream models — not materialized directly.
    Ephemeral,
    /// Excluded from execution.
    Skipped {
        /// Human-readable reason (e.g. `"test model"`, `"generator file"`).
        reason: String,
    },
}
