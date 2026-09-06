use super::*;
use anyhow::Result;
use smelt_backend::{maintenance_dialect, Backend, BackendError, ExecutionResult, PartitionRange};
use smelt_dialect::SqlDialect;
use smelt_logical::maintenance::emit::emit_staged_candidate_conditional_recompute;
use smelt_state::ddl_duckdb;
use std::time::Instant;

/// Execute a live, membership-sensitive `Technique::DeleteInsert` cell
/// (`resolve_live_membership_recompute_cell` above) via the staged-candidate
/// conditional `DELETE`+`INSERT`, full-recompute variant
/// (`smelt_logical::maintenance::emit::
/// emit_staged_candidate_conditional_recompute`) — the "full-model recompute
/// staged, change-suppressed where comparable" realisation
/// `incremental_models.md` §"The plan matrix" names for a
/// membership-sensitive group. `key` is the cell's own proven
/// `RowIdentity::Key` (never `WholeRow` — the caller only reaches here when
/// the resolver above already proved a real key); `candidate_select` is the
/// model's own FULL (unwindowed) recompiled SQL — the entire current
/// admitted+enriched state, not a time-windowed slice — so a departed OR
/// newly-admitted key is represented correctly, and the recompute variant's
/// own anti-join `DELETE` removes a departed key rather than leaving it
/// stale. `compared_columns` is the already fail-closed-admitted
/// `WriteSuppression::Suppressed` set — this write is always conditional
/// (`resolve_live_membership_recompute_cell` above only ever returns a cell
/// once suppression proved `Suppressed`; an `Unconditional` verdict has no
/// sound lowering and is skipped before reaching here), so its observed
/// output delta is recorded in the SAME backend transaction as the write
/// (T5, `docs/specs/incremental_models.md` §"The graph layer" — "Observed
/// deltas on model edges") unconditionally, matching
/// [`execute_column_scoped_write_with_observed_delta`]'s posture for its
/// own `Suppressed` arm. `window` identifies the run window this write
/// covers — the observed-delta table's own idempotent-replace key.
#[allow(clippy::too_many_arguments)]
pub async fn execute_staged_membership_recompute(
    backend: &dyn Backend,
    schema: &str,
    table: &str,
    key: &[String],
    candidate_select: &str,
    compared_columns: &[String],
    window: &PartitionRange,
    retry: &crate::execute::RetryPolicy<'_>,
) -> Result<ExecutionResult> {
    let start = Instant::now();
    let full_table = format!("{schema}.{table}");
    let dialect = maintenance_dialect(backend.dialect());
    let staged_relation = format!("__smelt_staged_{table}");
    let group = emit_staged_candidate_conditional_recompute(
        &full_table,
        &staged_relation,
        key,
        candidate_select,
        compared_columns,
        dialect,
    );
    if backend.dialect() != SqlDialect::DuckDB {
        return Err(anyhow::anyhow!(
            "{}",
            BackendError::unsupported(
                backend.dialect().name(),
                "observed-delta recording for a staged-candidate membership recompute (T5)",
            )
        ));
    }
    let ensure_sql = ddl_duckdb::generate_observed_delta_table_ddl(schema);
    let partition_column = if window.column.is_empty() {
        None
    } else {
        Some(window.column.as_str())
    };
    let changed_keys_query = staged_candidate_changed_keys_select(
        &full_table,
        key,
        candidate_select,
        compared_columns,
        partition_column,
    );
    let record_sql = ddl_duckdb::generate_observed_delta_upsert_sql(
        schema,
        table,
        &window.start,
        &window.end,
        &changed_keys_query,
    );
    crate::execute::retry_backend_call(retry, || {
        backend.execute_conditional_write_and_record_observed_delta(
            &ensure_sql,
            &group,
            &record_sql,
        )
    })
    .await
    .map_err(|e| {
        anyhow::anyhow!("staged-candidate membership recompute failed for '{full_table}': {e}")
    })?;
    let row_count = backend.get_row_count(schema, table).await.unwrap_or(0);
    Ok(ExecutionResult {
        model_name: table.to_string(),
        duration: start.elapsed(),
        row_count,
        preview: None,
    })
}

/// Execute a live, membership-sensitive `Technique::DeleteInsert` cell whose
/// row identity is `RowIdentity::WholeRow` (`resolve_live_membership_
/// recompute_cell`'s keyless arm, `docs/outcomes/
/// 20260815-definition-delta-migrate/phases/27c-plan.md`) via
/// [`smelt_logical::maintenance::emit::emit_staged_candidate_conditional_keyless`] —
/// the region-grained whole-row conditional `DELETE`+`INSERT`. Mirrors
/// [`execute_staged_membership_recompute`] minus the observed-delta leg: a
/// keyless write has no key columns the observed-delta table (T5) could
/// record against, so this executor never calls
/// `execute_conditional_write_and_record_observed_delta`, only the plain
/// `execute_statement_group`.
pub async fn execute_staged_keyless_recompute(
    backend: &dyn Backend,
    schema: &str,
    table: &str,
    candidate_select: &str,
    retry: &crate::execute::RetryPolicy<'_>,
) -> Result<ExecutionResult> {
    let start = Instant::now();
    let full_table = format!("{schema}.{table}");
    let dialect = maintenance_dialect(backend.dialect());
    let staged_relation = format!("__smelt_staged_{table}");
    let sentinel_relation = format!("__smelt_sentinel_{table}");
    let group = smelt_logical::maintenance::emit::emit_staged_candidate_conditional_keyless(
        &full_table,
        &staged_relation,
        &sentinel_relation,
        None,
        candidate_select,
        dialect,
    );
    crate::execute::retry_backend_call(retry, || backend.execute_statement_group(&group))
        .await
        .map_err(|e| {
            anyhow::anyhow!("staged-candidate keyless recompute failed for '{full_table}': {e}")
        })?;
    let row_count = backend.get_row_count(schema, table).await.unwrap_or(0);
    Ok(ExecutionResult {
        model_name: table.to_string(),
        duration: start.elapsed(),
        row_count,
        preview: None,
    })
}
