use super::window::*;

use std::collections::HashMap;

use anyhow::Result;
use chrono::{Datelike, NaiveDate};
use tracing::{info, warn};

use smelt_core::config::Config;
use smelt_core::graph::DependencyGraph;
use smelt_planner::Frontmatter;

use crate::expand_function_calls;
use crate::transformer::TimeRange;
use crate::types::ExecuteRequest;
use crate::windowing::{compute_incremental_windows_ordered, IncrementalBatch};

/// Plan for one model's execution. Internal to `execute_project` — the
/// public API is `ExecuteRequest` in / `RunOutcome` out.
pub(crate) struct ModelPlan {
    pub(crate) name: String,
    pub(crate) sql: String,
    pub(crate) materialization: smelt_core::config::Materialization,
    pub(crate) incremental: Option<IncrementalPlan>,
    pub(crate) model_file: smelt_core::ModelFile,
    /// Resolved `refresh:` strategy (SQL frontmatter > `smelt.yml` > `Full`,
    /// via `Config::get_refresh_with_metadata`), resolved once here rather
    /// than re-read deep in the executor. `refresh: materialized_view`
    /// models have no `grain:`/timeseries and so always land in the `None`
    /// (full-refresh) arm of the `plan.incremental` match; this field is
    /// what that arm consults to route to
    /// `Backend::create_materialized_view_as` instead of
    /// `Backend::execute_model` (`docs/specs/materialized_view.md`).
    pub(crate) refresh: smelt_core::config::RefreshStrategy,
}

pub(crate) struct IncrementalPlan {
    pub(crate) config: smelt_core::PartitionGrainConfig,
    pub(crate) timeseries: smelt_core::config::TimeseriesConfig,
    /// Batches with separate partition and filter ranges (bound-aware windowing).
    pub(crate) batches: Vec<IncrementalBatch>,
    /// The model's own derived partition-column skew bound
    /// (`docs/specs/model_transforms.md` §Semantics "The output window is
    /// derived, never assumed"), carried alongside `batches` (whose
    /// `partition_start`/`partition_end` already reflect it) so
    /// `derive_batch_filtered_sql` can additionally gate the transparent
    /// fast path on it without re-deriving it from the SQL a second time.
    pub(crate) skew: smelt_logical::analysis::source_bounds::Skew,
}

/// Build the per-model execution plans (batch/chunk windows via the
/// bound-aware windowing) for the selected models. Pure with respect to the
/// backend — it touches only the graph, config, function bodies, and the
/// project-wide source-timeseries map — so both the dry-run statement-emission
/// branch and the real run share the identical chunk decomposition
/// (`docs/specs/cli.md` §"`--dry-run` prints the maintenance statements":
/// rebuild's per-chunk boundaries under `--dry-run` are the real chunks).
/// Returns the plans plus the total batch count (for `run_started`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_model_plans(
    selected: &[String],
    graph_lock: &DependencyGraph,
    config: &Config,
    fn_bodies: &crate::FnBodyMap,
    source_timeseries: &smelt_planner::SourceTimeseriesMap,
    start_date: Option<NaiveDate>,
    end_date: Option<NaiveDate>,
    request: &ExecuteRequest,
    partition_axes: &HashMap<String, smelt_logical::PartitionAxis>,
) -> Result<(Vec<ModelPlan>, usize)> {
    let mut model_plans: Vec<ModelPlan> = Vec::new();
    let mut total_batches: usize = 0;

    for model_name in selected {
        let model = graph_lock.get_model(model_name)?;
        let metadata = model.metadata.as_deref();
        let frontmatter = Frontmatter::parse(&model.content);

        let inc_config = config
            .get_incremental_with_metadata(model_name, metadata)
            .or_else(|| frontmatter.as_ref().and_then(|f| f.batched_config()));

        let ts_config = config
            .get_timeseries_with_metadata(model_name, metadata)
            .cloned()
            .or_else(|| metadata.and_then(|m| m.timeseries.clone()));

        let refresh = config.get_refresh_with_metadata(model_name, metadata);

        // Resolve this model's partition axis (`docs/specs/timeseries.md`
        // §"Validation rules" rule 9) and, in that axis's domain, the
        // effective run-window bounds — `None` when no run window was
        // supplied, or the run-window literal doesn't parse in this
        // model's resolved axis (`window_for_axis`, which itself is `Err`
        // for a malformed-but-present window).
        type AxisWindow<'a> = (
            smelt_logical::PartitionAxis,
            &'a smelt_core::config::TimeseriesConfig,
            Option<(
                crate::windowing::PartitionPoint,
                crate::windowing::PartitionPoint,
            )>,
        );
        let axis_and_window: Option<AxisWindow> = match &ts_config {
            None => None,
            Some(ts) => {
                let axis = partition_axes.get(model_name).copied().unwrap_or_else(|| {
                    // Undecidable type — not a positive disproof of either
                    // domain (same fail-open posture as
                    // `derive_partition_grid_unit`). Fall back to the axis
                    // implied by the run-window literal's own form so a
                    // first-run without a resolvable schema still works for
                    // the common calendar case.
                    let implied =
                        crate::windowing::axis_implied_by_literal_form(request.start.as_deref());
                    warn!(
                        "model '{model_name}': partition column '{}' type could not be \
                         resolved from the output schema; falling back to the axis implied \
                         by the run-window literal's form ({:?})",
                        ts.partition_column, implied
                    );
                    implied
                });
                let window = window_for_axis(axis, start_date, end_date, request)?;
                Some((axis, ts, window))
            }
        };

        match (inc_config, axis_and_window) {
            (Some(inc), Some((axis, ts, Some((window_start, window_end))))) => {
                let ts = ts.clone();

                // Contract-lattice `frozen_horizon` write-eligibility clamp
                // (`docs/specs/incremental_models.md` §"Contract relaxations
                // (`contract:`)"): narrows the requested range's start to
                // `end - H`, never widens. The pure transform is single-owned
                // in `smelt-logical`; this call site only converts dates to
                // the day-count unit it operates on. Calendar axis only — a
                // `frozen_horizon` declared on an integer-axis model is a
                // hard refusal below (`docs/specs/incremental_shapes.md`
                // §"The partition grain" rule 8a: its horizon is a day count
                // with no conversion into partition units).
                if axis == smelt_logical::PartitionAxis::Integer
                    && metadata
                        .and_then(|m| m.contract.as_ref())
                        .and_then(|c| c.frozen_horizon.as_ref())
                        .is_some()
                {
                    return Err(anyhow::anyhow!(
                        "model '{model_name}': contract.frozen_horizon is declared, but \
                         partition column '{}' resolves to an integer partition axis; a \
                         frozen_horizon (a day count) has no conversion into partition units \
                         and is refused rather than silently unclamped",
                        ts.partition_column,
                    ));
                }
                let full_range = match (window_start, window_end) {
                    (
                        crate::windowing::PartitionPoint::Date(start_date),
                        crate::windowing::PartitionPoint::Date(end_date),
                    ) => {
                        let clamped_start_date = metadata
                            .and_then(|m| m.contract.as_ref())
                            .and_then(|c| c.frozen_horizon.as_ref())
                            .and_then(|fh| {
                                let h_days = fh.to_days() as i64;
                                let start_days = start_date.num_days_from_ce() as i64;
                                let end_days = end_date.num_days_from_ce() as i64;
                                let clamped_days = smelt_logical::clamp_frozen_horizon_write_range(
                                    start_days, end_days, h_days,
                                );
                                if clamped_days > start_days {
                                    info!(
                                        "model '{model_name}': frozen_horizon ({} days) narrows the \
                                         requested write range start to {}",
                                        h_days,
                                        NaiveDate::from_num_days_from_ce_opt(clamped_days as i32)
                                            .map(|d| d.format("%Y-%m-%d").to_string())
                                            .unwrap_or_default()
                                    );
                                }
                                NaiveDate::from_num_days_from_ce_opt(clamped_days as i32)
                            })
                            .unwrap_or(start_date);

                        TimeRange {
                            start: clamped_start_date.format("%Y-%m-%d").to_string(),
                            end: end_date.format("%Y-%m-%d").to_string(),
                            axis: smelt_logical::PartitionAxis::Calendar,
                        }
                    }
                    (start, end) => TimeRange {
                        start: start.to_string(),
                        end: end.to_string(),
                        axis: start.axis(),
                    },
                };

                // Use bound-aware windowing: SQL temporal dependencies + data latency
                // determine filter widening (not just analyze_batch_safety context_days).
                let expanded_sql = expand_function_calls(&model.content, fn_bodies);

                // Dependency timeseries map for this model — mirrors the
                // restriction to `model.refs` used later for
                // `build_source_bound_map` (see the comment at that call
                // site): `source_timeseries` also carries this model's own
                // frontmatter `timeseries:` entry, which must be excluded or
                // it inflates the bound map with a spurious self-entry.
                let model_ref_paths: std::collections::HashSet<String> = model
                    .refs
                    .iter()
                    .map(|r| format!("smelt.{}", r.smelt_ref.to_path().join(".")))
                    .collect();
                let dep_ts: HashMap<String, (Vec<String>, String)> = source_timeseries
                    .iter()
                    .filter(|(smelt_ref, _)| model_ref_paths.contains(*smelt_ref))
                    .filter_map(|(smelt_ref, ts_cfg)| {
                        let path = smelt_ref.strip_prefix("smelt.")?;
                        let segs: Vec<String> = path.split('.').map(String::from).collect();
                        Some((smelt_ref.clone(), (segs, ts_cfg.partition_column.clone())))
                    })
                    .collect();

                // Own `smelt.ref()` list, unfiltered — a self-edge (BL7,
                // `window_independence`) is `refs` containing `model_name`
                // itself, which `model_ref_paths`/`dep_ts` above deliberately
                // excludes (that map is upstream-*source* timeseries only).
                let refs: Vec<String> = model
                    .refs
                    .iter()
                    .map(|r| r.smelt_ref.to_path().join("."))
                    .collect();

                let inc_windows = compute_incremental_windows_ordered(
                    model_name,
                    &refs,
                    &ts,
                    &inc,
                    &expanded_sql,
                    &dep_ts,
                    &full_range,
                    axis,
                    request.batch_size_days,
                    request.per_partition,
                )
                .map_err(|diag| {
                    // Fail-closed last line of defense (`incremental_shapes.md` §"Partition-grain constraints" #10):
                    // even under `--allow-downgrade` (which only warns at the earlier
                    // `check_bound_derivation` gate), the batch-safety roll-up here must
                    // still refuse rather than silently approximate a chunk shape —
                    // there is no flag that makes an unsafe chunk shape safe.
                    anyhow::anyhow!(
                        "Backfill chunk-size derivation refused model '{}':\n  \u{2022} {}",
                        model_name,
                        diag
                    )
                })?;

                if let Some(ref warning) = inc_windows.wide_batch_warning {
                    warn!("model '{model_name}': {warning}");
                }

                let batches = inc_windows.batches;
                let skew = inc_windows.skew;
                total_batches += batches.len();
                model_plans.push(ModelPlan {
                    name: model_name.clone(),
                    sql: model.content.clone(),
                    materialization: config.get_materialization_with_metadata(model_name, metadata),
                    incremental: Some(IncrementalPlan {
                        config: inc,
                        timeseries: ts,
                        batches,
                        skew,
                    }),
                    model_file: model.clone(),
                    refresh: refresh.clone(),
                });
            }
            (Some(_inc), Some((_axis, _ts, None))) => {
                // Incremental config present but no time window resolved for
                // this model's axis. Fall back to full refresh; the model
                // still compiles and executes.
                model_plans.push(ModelPlan {
                    name: model_name.clone(),
                    sql: model.content.clone(),
                    materialization: config.get_materialization_with_metadata(model_name, metadata),
                    incremental: None,
                    model_file: model.clone(),
                    refresh: refresh.clone(),
                });
            }
            (Some(_inc), None) => {
                warn!(
                    "model '{model_name}' has incremental: but no timeseries: — skipping incremental execution"
                );
                model_plans.push(ModelPlan {
                    name: model_name.clone(),
                    sql: model.content.clone(),
                    materialization: config.get_materialization_with_metadata(model_name, metadata),
                    incremental: None,
                    model_file: model.clone(),
                    refresh: refresh.clone(),
                });
            }
            (None, _) => {
                model_plans.push(ModelPlan {
                    name: model_name.clone(),
                    sql: model.content.clone(),
                    materialization: config.get_materialization_with_metadata(model_name, metadata),
                    incremental: None,
                    model_file: model.clone(),
                    refresh: refresh.clone(),
                });
            }
        }
    }

    Ok((model_plans, total_batches))
}
