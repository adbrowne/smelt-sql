//! Planner safety check + schema-evolution helpers.
//!
//! These functions are the pre-execute gates the CLI previously owned inline in
//! `commands/run.rs`. Lifting them here means the UI gets identical protection
//! for free, and future CLI refactors become one-line flag pass-throughs.

use anyhow::Result;
use smelt_planner::{derive_model_source_bounds, ModelGraph, Planner, Transformation};

use crate::schema_evolution::SchemaEvolutionResult;

/// Build a `smelt_planner::ModelGraph` from the selected model list.
///
/// The ModelGraph is the planner's view of the project: model names, SQL,
/// refs, and timeseries / incremental configs. Both the planner safety check
/// and bound-derivation pass operate on this value.
///
/// `selected` must be in topological (execution) order so that upstream deps
/// appear before their consumers. Use `DependencyGraph::execution_order()` to
/// obtain this ordering.
pub fn build_model_graph(
    selected: &[String],
    graph: &smelt_core::graph::DependencyGraph,
    config: &smelt_core::config::Config,
) -> ModelGraph {
    let mut model_graph = ModelGraph::new();
    for model_name in selected {
        if let Ok(model) = graph.get_model(model_name) {
            let frontmatter = smelt_planner::Frontmatter::parse(&model.content);
            let metadata = model.metadata.as_deref();
            let timeseries_config = config
                .get_timeseries_with_metadata(model_name, metadata)
                .cloned()
                .or_else(|| metadata.and_then(|m| m.timeseries.clone()));
            let incremental_config = config
                .get_incremental_with_metadata(model_name, metadata)
                .or_else(|| frontmatter.as_ref().and_then(|f| f.batched_config()));
            let refs: Vec<String> = model
                .refs
                .iter()
                .map(|r| r.smelt_ref.to_path().join("."))
                .collect();
            model_graph.add_model(smelt_planner::ModelInfo {
                name: model.name.clone(),
                sql: model.content.clone(),
                refs,
                timeseries_config,
                incremental_config,
            });
        }
    }
    model_graph
}

/// Run the planner incremental safety check against the model graph.
///
/// When `enforce_safety` is `true` (the default), any planner error causes
/// `Err` with the same message the CLI emitted. When `false`, errors are
/// logged as warnings and `Ok(transformations)` is returned — mirroring
/// `--allow-downgrade`.
pub fn check_planner_safety(
    model_graph: &ModelGraph,
    enforce_safety: bool,
) -> Result<Vec<Transformation>> {
    let planner = Planner::new();
    let (transformations, plan_errors) = planner.plan(model_graph);

    if !plan_errors.is_empty() {
        if enforce_safety {
            let mut msg = String::from(
                "Incremental safety check refused the following model(s). \
                 Fix the SQL or use --allow-downgrade to fall back to full-table refresh:\n",
            );
            for err in &plan_errors {
                msg.push_str("  \u{2022} ");
                msg.push_str(err);
                msg.push('\n');
            }
            return Err(anyhow::anyhow!("{}", msg.trim_end()));
        } else {
            for err in &plan_errors {
                tracing::warn!(
                    "Incremental safety check failed (falling back to full-table refresh \
                     because enforce_safety is disabled): {}",
                    err
                );
            }
        }
    }

    Ok(transformations)
}

/// Derive temporal bounds for every incremental model in the graph and refuse
/// if any bound is `NotDerivable`.
///
/// `enforce_safety = true` → `Err` on the first undefinable bound.
/// `enforce_safety = false` → `Ok(())` with a `warn!` per problematic model.
pub fn check_bound_derivation(model_graph: &ModelGraph, enforce_safety: bool) -> Result<()> {
    let mut bound_errors: Vec<String> = Vec::new();
    for model_info in model_graph.models() {
        if model_info.incremental_config.is_some() && model_info.timeseries_config.is_some() {
            if let Err(diag) = derive_model_source_bounds(model_info, model_graph) {
                bound_errors.push(diag);
            }
        }
    }

    if !bound_errors.is_empty() {
        if enforce_safety {
            let mut msg = String::from(
                "Temporal bound derivation refused the following model(s) — \
                 Fix the SQL or use --allow-downgrade to fall back to full-table refresh:\n",
            );
            for err in &bound_errors {
                msg.push_str("  \u{2022} ");
                msg.push_str(err);
                msg.push('\n');
            }
            return Err(anyhow::anyhow!("{}", msg.trim_end()));
        } else {
            for err in &bound_errors {
                tracing::warn!(
                    "Bound derivation failed (falling back to full-table refresh \
                     because enforce_safety is disabled): {}",
                    err
                );
            }
        }
    }

    Ok(())
}

/// Interpret a `SchemaEvolutionResult` in the context of the `--allow-*` flags.
///
/// Returns:
/// - `Ok(true)` — caller should force a full-table refresh before execution.
/// - `Ok(false)` — no special action needed; proceed with the normal strategy.
/// - `Err(...)` — the evolution is blocked; the user must set a flag to allow.
///
/// Error messages match the CLI's existing strings so output is unchanged.
pub fn should_force_full_refresh(
    result: &SchemaEvolutionResult,
    model_name: &str,
    allow_column_removal: bool,
    allow_full_refresh: bool,
) -> Result<bool> {
    match result {
        SchemaEvolutionResult::FirstDeployment
        | SchemaEvolutionResult::NoChange
        | SchemaEvolutionResult::Migrated { .. } => Ok(false),

        SchemaEvolutionResult::FullRefreshRequired { .. } => Ok(true),

        SchemaEvolutionResult::TableRewrite { .. } => Ok(true),

        SchemaEvolutionResult::ColumnRemovalBlocked { columns } => {
            if allow_column_removal {
                Ok(true)
            } else {
                Err(anyhow::anyhow!(
                    "Schema evolution for '{}' would remove columns: {}. \
                     Use --allow-column-removal to permit this.",
                    model_name,
                    columns.join(", ")
                ))
            }
        }

        SchemaEvolutionResult::FullRefreshBlocked { reason } => {
            if allow_full_refresh {
                Ok(true)
            } else {
                Err(anyhow::anyhow!(
                    "Schema evolution for '{}' requires full refresh: {}. \
                     Use --allow-full-refresh to permit this.",
                    model_name,
                    reason
                ))
            }
        }
    }
}
