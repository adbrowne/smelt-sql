//! Run-level dispatch of an **enrichment-keyed** `Technique::ColumnScopedMerge`
//! model-edge cell (`docs/specs/incremental_models.md` §"Upstream model
//! edges") — phase 5, `docs/outcomes/20260906-bigquery-correctness`.
//!
//! `append_model_edge_cells` (phase 4, `smelt-logical`) derives this cell for
//! a clockless keyed upstream model read in value-enrichment position by a
//! partition-addressed downstream (e.g. `gold.events_enriched` reading
//! `gold.repo_dim` to carry a mutable `current_repo_name`). Its write is
//! addressed by the join key the downstream's own output carries
//! (`key_scope.keys`), not by a partition interval — `admit_enrichment_
//! keyed_merge` says so explicitly (`PartitionLocal::No { why: "addressed by
//! its own join key, not a partition interval" }`). The ordinary per-batch
//! `ColumnMergeDispatch::Full` arm MERGEs `compiled.sql` already filtered to
//! the batch's `[start, end)` window, which would heal only rows this run's
//! window rewrote — a day-N replay run would never revisit day N−3's rows,
//! leaving the stale count non-zero forever. `decide_column_merge_dispatch`
//! therefore excludes an `EnrichmentKeyed` cell from the window-scoped
//! dispatch entirely; this function is its only run path — dispatched once
//! per run, after the model's own creation-trigger writes, over the model's
//! **unwindowed** compiled SQL, updating only the cell's own group columns.
//! The edge's declared `allow_full_scan: true` (checked by
//! `append_model_edge_cells` before this cell was ever derived) is what
//! licenses that full read.
//!
//! A model-edge `UpstreamMutation` trigger has no `SourceInfo` (edges are
//! keyed on the upstream MODEL's bare address, never a declared source), so
//! `resolve_upstream_mutation_gate`'s lookup into `source_infos` already
//! returns `None` for it — the declared behaviour is that this **fails open
//! to dispatch** every run, at the cost of a full-table merge every run,
//! rather than silently never healing because no baseline could be compared
//! (`docs/specs/incremental_models.md` §"When a mutation cell dispatches").
//! This function does not itself call `resolve_upstream_mutation_gate` or
//! record a baseline — there is nothing to gate on or record.

use anyhow::Result;
use smelt_backend::{Backend, ExecutionResult, PartitionRange};
use smelt_logical::maintenance::availability::StateAvailability;
use smelt_logical::maintenance::choice::WriteSuppression;
use smelt_logical::maintenance::derive::{ModelEdge, SourceReferentialIntegrity};
use smelt_logical::maintenance::{KeyDiscovery, PlanCell, SourceFacts};
use std::collections::HashSet;

use crate::compile::{EphemeralResolver, SqlCompiler};
use crate::execute::RetryPolicy;
use crate::maintenance_driver::execute_column_scoped_merge_full;

/// Dispatch the run-level heal for `column_scoped_cell`, when (and only
/// when) it is a live `EnrichmentKeyed` cell and the target table already
/// existed before this run (never on the creation run — there is nothing to
/// heal yet). `sql` must be the model's clean, **unwindowed** SQL (frontmatter
/// stripped, no `[start, end)` time filter injected) — the same text every
/// other resolver in this ladder derives the plan from.
///
/// `unique_key` is the model's own declared write key (`inc_plan.config.
/// unique_key` / the keyed branch's own `unique_key`, which `merge_key:`
/// folds into) — an empty key keeps `decide_column_merge_dispatch`'s
/// documented no-error posture (a model with no declared key cannot MERGE at
/// all): this falls back to a no-op rather than erroring, but names the
/// model and edge in a `tracing::warn!` rather than vanishing silently.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn execute_enrichment_keyed_heal(
    backend: &dyn Backend,
    schema: &str,
    model_name: &str,
    table: &str,
    table_existed_before_run: bool,
    sql: &str,
    metadata: &smelt_core::ModelMetadata,
    sources: &[SourceFacts],
    explicitly_mutable: &HashSet<String>,
    model_edges: &[ModelEdge],
    availability: &StateAvailability,
    column_scoped_cell: Option<&(String, PlanCell, WriteSuppression)>,
    unique_key: &[String],
    model_file: &smelt_core::ModelFile,
    compiler: &SqlCompiler,
    resolver: &EphemeralResolver,
    window: &PartitionRange,
    retry: &RetryPolicy<'_>,
) -> Result<Option<ExecutionResult>> {
    let Some((source, cell, suppression)) = column_scoped_cell else {
        return Ok(None);
    };
    let is_enrichment_keyed = cell
        .key_scope
        .as_ref()
        .is_some_and(|scope| scope.discovery == KeyDiscovery::EnrichmentKeyed);
    if !is_enrichment_keyed || !table_existed_before_run {
        return Ok(None);
    }
    if unique_key.is_empty() {
        tracing::warn!(
            "model '{model_name}': enrichment-keyed heal for upstream model edge '{source}' \
             has no declared write key (unique_key/merge_key) — skipping the run-level heal \
             rather than erroring; '{source}''s mutations of this model will stay unhealed \
             until a write key is declared"
        );
        return Ok(None);
    }
    // Re-derive the same plan `resolve_live_column_scoped_cell` already
    // resolved `cell` from, purely to read `column_groups` (maintenance-plan
    // purity: this reads already-derivable data the same way every other
    // per-cell resolver in this ladder does — `repair/resolve_cell.rs`,
    // `membership/mod.rs`, `key_addressed/mod.rs` — never a second admission
    // pass).
    let Some(result) = crate::maintenance_availability::derive_resolved_with_edges(
        sql,
        table,
        metadata,
        sources,
        explicitly_mutable,
        model_edges,
        None,
        &[],
        &[],
        &SourceReferentialIntegrity::new(),
        None,
        None,
        availability,
        &[],
    ) else {
        return Ok(None);
    };
    let Some(group_columns) = result
        .column_groups
        .iter()
        .find(|g| g.name() == cell.group)
        .map(|g| g.columns.clone())
    else {
        tracing::warn!(
            "model '{model_name}': enrichment-keyed heal for upstream model edge '{source}' \
             resolved a cell for group '{}' but the re-derived plan carries no matching column \
             group — internal inconsistency, skipping the run-level heal",
            cell.group
        );
        return Ok(None);
    };
    let compiled = compiler.compile_with_sql_and_ephemerals(model_file, schema, sql, resolver)?;
    let exec_result = execute_column_scoped_merge_full(
        backend,
        schema,
        table,
        unique_key,
        &compiled.sql,
        &group_columns,
        suppression,
        window,
        retry,
    )
    .await?;
    Ok(Some(exec_result))
}

/// Thin run-level wrapper around [`execute_enrichment_keyed_heal`]: builds
/// the whole-run `PartitionRange` (`start_date`/`end_date` formatted, matching
/// every other run-level window in this module) and a fresh
/// [`RetryPolicy`](crate::execute::RetryPolicy), so both `execute_project`
/// call sites (the keyed and non-keyed branches) pass only their own already-
/// resolved facts rather than repeating this boilerplate — kept out of
/// `execute/project/mod.rs` to hold that file at its large-file baseline.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn dispatch_enrichment_keyed_heal_for_run(
    backend: &dyn Backend,
    schema: &str,
    model_name: &str,
    table: &str,
    table_existed_before_run: bool,
    sql: &str,
    metadata: &smelt_core::ModelMetadata,
    sources: &[SourceFacts],
    explicitly_mutable: &HashSet<String>,
    model_edges: &[ModelEdge],
    availability: &StateAvailability,
    column_scoped_cell: Option<&(String, PlanCell, WriteSuppression)>,
    unique_key: &[String],
    model_file: &smelt_core::ModelFile,
    compiler: &SqlCompiler,
    resolver: &EphemeralResolver,
    start_date: Option<chrono::NaiveDate>,
    end_date: Option<chrono::NaiveDate>,
    request: &crate::types::ExecuteRequest,
    run_id: &str,
    reporter: &dyn crate::reporter::RunReporter,
) -> Result<Option<ExecutionResult>> {
    let (window_start, window_end) = match (start_date, end_date) {
        (Some(s), Some(e)) => (
            s.format("%Y-%m-%d").to_string(),
            e.format("%Y-%m-%d").to_string(),
        ),
        _ => (String::new(), String::new()),
    };
    let window = PartitionRange {
        column: String::new(),
        start: window_start,
        end: window_end,
        axis: smelt_backend::PartitionAxis::Calendar,
    };
    let retry = RetryPolicy::from_request(request, run_id, model_name, reporter);
    execute_enrichment_keyed_heal(
        backend,
        schema,
        model_name,
        table,
        table_existed_before_run,
        sql,
        metadata,
        sources,
        explicitly_mutable,
        model_edges,
        availability,
        column_scoped_cell,
        unique_key,
        model_file,
        compiler,
        resolver,
        &window,
        &retry,
    )
    .await
}
