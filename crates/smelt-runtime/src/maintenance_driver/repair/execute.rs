use super::*;
use anyhow::Result;
use smelt_backend::{maintenance_dialect, Backend, ExecutionResult};
use smelt_logical::maintenance::emit::emit_per_group_recompute;
use std::time::Instant;

/// The affected-key relation a repair reads: the distinct group keys present
/// in the mutated source over the cell's own bounded slice, projected as a
/// single canonical `delta_key` column — the same expression
/// ([`smelt_logical::maintenance::emit::key_expr_for_columns`])
/// [`emit_fingerprint_sidecar_diff`]/[`emit_repair_group_sidecar_diff`]
/// project, so the append-only clamped-scan path and the `mutable_snapshot`
/// group-grain sidecar-diff path ([`diff_repair_group_sidecar_changed_keys`] +
/// [`repair_keys_literal_select`])
/// yield the SAME one-column relation shape — [`emit_per_group_recompute`]
/// joins against it by key EXPRESSION, never by raw key columns, because a
/// deleted group's typed column values are unrecoverable by construction.
///
/// Pure string builder, per this module's "callers resolve strings, emitters
/// assemble" contract — [`emit_per_group_recompute`] consumes this as opaque
/// `SELECT` text. The clamp is pushed into **this** read (and only this
/// one): the widened source band is what bounds how many groups a repair
/// touches, while the recompute of each touched group must stay unbounded
/// so the group is recomputed whole (see
/// [`repair_candidate_select`]).
///
/// `clamp: None` yields the unpredicated read. That branch is only reachable
/// where admission already proved the slice by another route —
/// `repair::admit_per_group_recompute` refuses `RepairSliceUnbounded` rather
/// than admitting a clamp-less cell, so this is not a silent widening path.
pub fn repair_affected_keys_select(
    source_table: &str,
    key: &[String],
    clamp: Option<&ScanClamp>,
    region: &Region,
) -> String {
    let key_expr = smelt_logical::maintenance::emit::key_expr_for_columns(key);
    match clamp {
        Some(clamp) => format!(
            "SELECT DISTINCT {key_expr} AS delta_key FROM {source_table} WHERE {}",
            widened_scan_predicate(clamp, region)
        ),
        None => format!("SELECT DISTINCT {key_expr} AS delta_key FROM {source_table}"),
    }
}

/// The candidate relation a repair stages: the model's **full** (unwindowed)
/// recompiled SQL, semi-joined to `affected_keys_select`'s single-column
/// `delta_key` relation.
///
/// Full, not windowed, because the repair family's promise is that an
/// affected group's stored value equals a full refresh of that group — a
/// non-invertible combiner (`MAX`) over a retracted contribution cannot be
/// fixed from a window's rows alone. The semi-join is what keeps the
/// recompute *bounded*: only the groups the affected-keys read named are
/// recomputed. `EXISTS` rather than a row-value `IN`, so a composite key
/// lowers identically across dialects. The join compares
/// [`repair_affected_keys_select`]'s own canonical key expression over the
/// candidate's key columns against `delta_key`, mirroring
/// [`repair_slice_predicate`]'s and `emit_per_group_recompute`'s identical
/// shape.
/// Widen `clean_sql` (the model's own raw, pre-compile SELECT) with one
/// `, <per_partition_expr> AS <name>` per `state_columns` — a named wrapper
/// over [`smelt_logical::maintenance::emit::state_augmented_projection`]
/// with the repair path's own error text, so the widening is independently
/// unit-testable rather than inlined at each call site
/// (`docs/outcomes/20260809-repair-family/phases/10-plan.md`). Mirrors
/// `smelt-runtime::cumulative::execute_windowed_keyed`/
/// `execute_snapshot_reconcile`'s own use of the same emitter: the fold's
/// create/merge path already carries a decomposed combiner's hidden state
/// columns in the physical table, so a repair's own candidate/insert must
/// supply them too, or the `INSERT`'s implicit column list mismatches the
/// table. `state_columns.is_empty()` returns `clean_sql` unchanged.
pub fn repair_augmented_model_sql(
    clean_sql: &str,
    state_columns: &[smelt_logical::analysis::decomposed_state::StateColumn],
) -> Result<String> {
    smelt_logical::maintenance::emit::state_augmented_projection(clean_sql, state_columns).map_err(
        |_| {
            anyhow::anyhow!(
                "Failed to append decomposed-state columns to a repair candidate: the model's \
                 SELECT could not be parsed"
            )
        },
    )
}

pub fn repair_candidate_select(
    full_model_sql: &str,
    key: &[String],
    affected_keys_select: &str,
) -> String {
    let candidate_key_columns: Vec<String> = key
        .iter()
        .map(|k| format!("__smelt_repair_candidate.{k}"))
        .collect();
    let candidate_key_expr =
        smelt_logical::maintenance::emit::key_expr_for_columns(&candidate_key_columns);
    format!(
        "SELECT __smelt_repair_candidate.* FROM ({full_model_sql}) AS __smelt_repair_candidate \
         WHERE EXISTS (SELECT 1 FROM ({affected_keys_select}) AS __smelt_repair_keys WHERE \
         {candidate_key_expr} = __smelt_repair_keys.delta_key)"
    )
}

/// Inputs to refresh the group-grain fingerprint sidecar transactionally
/// with a repair write (P9, `docs/outcomes/20260809-repair-family/phases/
/// 09-plan.md` task 6) — passed to [`execute_per_group_recompute`]/
/// [`execute_diff_patch`] only when the live cell's discovery is
/// [`RepairDiscovery::SidecarDiff`]; `None` for the ordinary clamped-scan
/// path, which has no sidecar partition to refresh.
pub struct RepairSidecarRefresh<'a> {
    pub schema: &'a str,
    pub source_address: &'a str,
    pub source_table: &'a str,
    pub group_key: &'a [String],
    pub digest_columns: &'a [String],
    pub model_sql: &'a str,
    pub consumer_address: &'a str,
}

/// Execute a live `Technique::PerGroupRecompute` cell
/// ([`resolve_live_per_group_recompute_cell`]) via the repair family's
/// targeted `DELETE`+`INSERT` over the affected-key relation
/// ([`emit_per_group_recompute`]) — the same emitter → `retry_backend_call`
/// → [`Backend::execute_statement_group`] shape
/// [`execute_staged_membership_recompute`] uses, so the executed text is
/// exactly the single owner's output (`docs/specs/incremental_models.md`
/// §"Statement emission (single owner)").
///
/// `sidecar_refresh: Some(..)` (a [`RepairDiscovery::SidecarDiff`] cell)
/// routes the SAME emitted [`StatementGroup`] through
/// [`refresh_repair_group_sidecar`] instead of a bare
/// [`Backend::execute_statement_group`] call — the group-grain sidecar
/// partition refreshes in the SAME backend transaction as this write
/// (mirroring [`refresh_fingerprint_sidecar`]'s own transactional shape),
/// so a failed write leaves the sidecar untouched rather than
/// half-committed.
#[allow(clippy::too_many_arguments)]
pub async fn execute_per_group_recompute(
    backend: &dyn Backend,
    schema: &str,
    table: &str,
    key: &[String],
    affected_keys_select: &str,
    candidate_select: &str,
    retry: &crate::execute::RetryPolicy<'_>,
    sidecar_refresh: Option<&RepairSidecarRefresh<'_>>,
) -> Result<ExecutionResult> {
    let start = Instant::now();
    let full_table = format!("{schema}.{table}");
    let dialect = maintenance_dialect(backend.dialect());
    let staged_relation = repair_staged_relation(table);
    let group = emit_per_group_recompute(
        &full_table,
        &staged_relation,
        key,
        affected_keys_select,
        candidate_select,
        dialect,
    );
    match sidecar_refresh {
        None => {
            crate::execute::retry_backend_call(retry, || backend.execute_statement_group(&group))
                .await
                .map_err(|e| {
                    anyhow::anyhow!("per-group recompute failed for '{full_table}': {e}")
                })?;
        }
        Some(refresh) => {
            crate::execute::retry_backend_call(retry, || {
                refresh_repair_group_sidecar(
                    backend,
                    refresh.schema,
                    refresh.source_address,
                    refresh.source_table,
                    refresh.group_key,
                    refresh.digest_columns,
                    refresh.model_sql,
                    refresh.consumer_address,
                    &group,
                )
            })
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "per-group recompute (with group-grain sidecar refresh) failed for \
                     '{full_table}': {e}"
                )
            })?;
        }
    }
    let row_count = backend.get_row_count(schema, table).await.unwrap_or(0);
    Ok(ExecutionResult {
        model_name: table.to_string(),
        duration: start.elapsed(),
        row_count,
        preview: None,
    })
}

/// The staged temp relation name a repair uses for `table` — one derivation,
/// so a parity test (and the technique preview) can name the same relation
/// the live run does without guessing.
pub fn repair_staged_relation(table: &str) -> String {
    format!("__smelt_repair_{table}")
}

/// The staged temp relation name a `diff_patch` write over a repair cell
/// uses for `table` — a distinct prefix from [`repair_staged_relation`] so a
/// parity test can name each group's own relation without ambiguity.
pub fn diff_patch_staged_relation(table: &str) -> String {
    format!("__smelt_diff_patch_{table}")
}

/// The `diff_patch` slice restriction for a repair cell: the candidate's own
/// slice is the affected-key set, not a partition region
/// ([`emit_diff_patch`]'s doc comment) — an `EXISTS` over the affected-keys
/// read's single-column `delta_key` relation, `table`-qualified on every key
/// column (via the same canonical key expression
/// [`repair_affected_keys_select`]/[`repair_candidate_select`] use) so it
/// composes unambiguously into both the update-leg and delete-leg `DELETE`s
/// `emit_diff_patch` builds.
pub fn repair_slice_predicate(table: &str, key: &[String], affected_keys_select: &str) -> String {
    let table_key_columns: Vec<String> = key.iter().map(|k| format!("{table}.{k}")).collect();
    let table_key_expr = smelt_logical::maintenance::emit::key_expr_for_columns(&table_key_columns);
    format!(
        "EXISTS (SELECT 1 FROM ({affected_keys_select}) AS __smelt_repair_keys WHERE \
         {table_key_expr} = __smelt_repair_keys.delta_key)"
    )
}

/// Execute a `write: diff_patch` pin over a live `Technique::PerGroupRecompute`
/// cell ([`resolve_live_per_group_recompute_cell`]) via [`emit_diff_patch`] —
/// same emitter → `retry_backend_call` → [`Backend::execute_statement_group`]
/// shape [`execute_per_group_recompute`] uses, so the executed text is
/// exactly the single owner's output (`docs/specs/incremental_models.md`
/// §"Statement emission (single owner)"). `sidecar_refresh` carries the same
/// meaning as [`execute_per_group_recompute`]'s own parameter.
#[allow(clippy::too_many_arguments)]
pub async fn execute_diff_patch(
    backend: &dyn Backend,
    schema: &str,
    table: &str,
    key: &[String],
    candidate_select: &str,
    compared_columns: &[String],
    slice_predicate: &str,
    delete_leg: &smelt_logical::maintenance::diff_patch::DeleteLeg,
    retry: &crate::execute::RetryPolicy<'_>,
    sidecar_refresh: Option<&RepairSidecarRefresh<'_>>,
) -> Result<ExecutionResult> {
    let start = Instant::now();
    let full_table = format!("{schema}.{table}");
    let dialect = maintenance_dialect(backend.dialect());
    let staged_relation = diff_patch_staged_relation(table);
    let group = smelt_logical::maintenance::emit::emit_diff_patch(
        &full_table,
        &staged_relation,
        key,
        candidate_select,
        compared_columns,
        slice_predicate,
        delete_leg,
        dialect,
    );
    match sidecar_refresh {
        None => {
            crate::execute::retry_backend_call(retry, || backend.execute_statement_group(&group))
                .await
                .map_err(|e| anyhow::anyhow!("diff_patch write failed for '{full_table}': {e}"))?;
        }
        Some(refresh) => {
            crate::execute::retry_backend_call(retry, || {
                refresh_repair_group_sidecar(
                    backend,
                    refresh.schema,
                    refresh.source_address,
                    refresh.source_table,
                    refresh.group_key,
                    refresh.digest_columns,
                    refresh.model_sql,
                    refresh.consumer_address,
                    &group,
                )
            })
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "diff_patch write (with group-grain sidecar refresh) failed for \
                     '{full_table}': {e}"
                )
            })?;
        }
    }
    let row_count = backend.get_row_count(schema, table).await.unwrap_or(0);
    Ok(ExecutionResult {
        model_name: table.to_string(),
        duration: start.elapsed(),
        row_count,
        preview: None,
    })
}
