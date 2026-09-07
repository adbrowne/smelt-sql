use super::*;
use anyhow::Result;
use smelt_backend::{Backend, ExecutionResult};
use smelt_logical::maintenance::KeyDiscovery;

/// Execute a live key-addressed model-edge cell
/// ([`resolve_live_key_addressed_model_edge_cell`]): discover the upstream's
/// changed keys via the group-grain fingerprint sidecar diff over its own
/// output table, project them onto the downstream's own key columns
/// ([`resolve_key_addressed_affected_keys`]), stage the downstream's full
/// SQL semi-joined to that key relation
/// ([`repair_candidate_select`]), and write via the resolved
/// [`RepairWrite`] leg — [`execute_per_group_recompute`] for
/// `TargetedDeleteInsert`, [`execute_diff_patch`] for `DiffPatch`. The
/// upstream sidecar refreshes in the SAME backend transaction as this write
/// ([`RepairSidecarRefresh`]), so a failed write never leaves the sidecar
/// advanced past a change it did not actually consume.
///
/// An empty changed-key set is a legitimate no-op: this returns
/// `Ok(None)` rather than executing an empty-but-real write, matching
/// [`emit_key_addressed_affected_keys_select`]'s own well-typed-empty
/// convention (`docs/outcomes/20260809-output-delta-typing/phases/07-plan.md`
/// task 5).
#[allow(clippy::too_many_arguments)]
pub async fn execute_key_addressed_model_edge_cell(
    backend: &dyn Backend,
    schema: &str,
    table: &str,
    upstream_source_address: &str,
    upstream_table: &str,
    group_key: &[String],
    digest_columns: &[String],
    downstream_keys: &[String],
    discovery: KeyDiscovery,
    clean_model_sql: &str,
    compiled_model_sql: &str,
    write: &RepairWrite,
    retry: &crate::execute::RetryPolicy<'_>,
    consumer_address: &str,
) -> Result<Option<ExecutionResult>> {
    let full_table = format!("{schema}.{table}");
    let (changed_keys, affected_keys_select) = resolve_key_addressed_affected_keys(
        backend,
        schema,
        upstream_source_address,
        upstream_table,
        &full_table,
        group_key,
        digest_columns,
        downstream_keys,
        discovery,
        clean_model_sql,
        consumer_address,
    )
    .await
    .map_err(|e| {
        anyhow::anyhow!(
            "key-addressed affected-key discovery failed over upstream '{upstream_table}' for \
             '{full_table}': {e}"
        )
    })?;
    if changed_keys.is_empty() {
        return Ok(None);
    }
    let candidate_select =
        repair_candidate_select(compiled_model_sql, downstream_keys, &affected_keys_select);
    let sidecar_refresh = RepairSidecarRefresh {
        schema,
        source_address: upstream_source_address,
        source_table: upstream_table,
        group_key,
        digest_columns,
        model_sql: clean_model_sql,
        consumer_address,
    };
    let result = match write {
        RepairWrite::TargetedDeleteInsert => {
            execute_per_group_recompute(
                backend,
                schema,
                table,
                downstream_keys,
                &affected_keys_select,
                &candidate_select,
                retry,
                Some(&sidecar_refresh),
            )
            .await?
        }
        RepairWrite::DiffPatch {
            compared_columns,
            delete_leg,
        } => {
            let slice_predicate =
                repair_slice_predicate(table, downstream_keys, &affected_keys_select);
            execute_diff_patch(
                backend,
                schema,
                table,
                downstream_keys,
                &candidate_select,
                compared_columns,
                &slice_predicate,
                delete_leg,
                retry,
                Some(&sidecar_refresh),
            )
            .await?
        }
    };
    Ok(Some(result))
}
