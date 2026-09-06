use std::time::Instant;

use anyhow::{bail, Result};

use smelt_backend::{Backend, BackendError, ExecutionResult, SqlDialect};
use smelt_state::ddl_duckdb;
use smelt_types::DataType;

use super::SuccessionCell;
use crate::maintenance_driver::MaintenanceStep;
use crate::reporter::RunReporter;

/// The reconciliation ledger's group key for a succession-patch cell — the
/// whole-row shape every window-forward cell today records under
/// (`driver::LEDGER_WHOLE_ROW_GROUP`'s own precedent; not reused directly
/// since that constant is private to the sibling `driver` module).
const SUCCESSION_LEDGER_GROUP: &str = "{*}";

/// Execute the window-forward succession-patch loop over `steps`: for each
/// step, build the event-delta `SELECT` over the driving source's own
/// arrival-partition window, run the clock-tie probe read-only (refusing
/// before any write on a violation — `SuccessionClockTie`,
/// `docs/specs/incremental_shapes.md` §"Run shape and late events"), then
/// apply the tombstone-ledger DDL, the presented-table bootstrap shell (only
/// when the table does not exist yet), the merge-ledger frontier upsert, and
/// the phase-4 patch group (tombstone insert + presented `MERGE`) all in one
/// backend transaction via [`Backend::execute_write_with_bookkeeping`].
///
/// `columns` is the model's resolved output schema
/// (`UpstreamSchemas.models[<model name>]`, name + `DataType`) — the same
/// fact `execute::bootstrap::bootstrap_self_ref_empty_target` reads, used
/// here to bootstrap the presented table's empty shell and to type the
/// tombstone table's `key_cols ++ [clock_col]` columns.
///
/// Every emitted [`smelt_logical::maintenance::emit::StatementGroup`] is
/// reported via `reporter.maintenance_statements` before it runs — the
/// event-delta `SELECT`, the clock-tie probe, and the patch group — so the
/// executed-vs-emitted statement-parity leg (phase 5c) has a record to check
/// against. The warehouse-resident ledger/tombstone DDL and the ledger
/// upsert are bookkeeping (`CLAUDE.md` §"Maintenance-plan purity" — "ledger
/// DDL/DML in `smelt-state` excluded as bookkeeping"), matching the
/// windowed-keyed driver's own precedent of never reporting those.
#[allow(clippy::too_many_arguments)]
pub async fn execute_succession_maintenance(
    backend: &dyn Backend,
    model_name: &str,
    schema: &str,
    table: &str,
    steps: &[MaintenanceStep],
    cell: &SuccessionCell,
    columns: &[(String, DataType)],
    retry: &crate::execute::RetryPolicy<'_>,
    probe_policy: &crate::probes::ProbePolicy,
    reporter: &dyn RunReporter,
    run_id: &str,
) -> Result<ExecutionResult> {
    if backend.dialect() != SqlDialect::DuckDB {
        bail!(
            "{}",
            BackendError::unsupported(
                backend.dialect().name(),
                "succession-patch technique (window-forward driver)",
            )
        );
    }
    let dialect = smelt_backend::maintenance_dialect(backend.dialect());
    let recipe = &cell.recipe;

    let key_cols_typed: Vec<(String, DataType)> = recipe
        .key_cols
        .iter()
        .map(|k| {
            columns
                .iter()
                .find(|(name, _)| name == k)
                .cloned()
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "succession-patch: model '{model_name}' key column '{k}' has no \
                         resolved output type — cannot build the tombstone ledger table"
                    )
                })
        })
        .collect::<Result<Vec<_>>>()?;
    let (_, clock_type) = columns
        .iter()
        .find(|(name, _)| name == &recipe.clock_col)
        .cloned()
        .ok_or_else(|| {
            anyhow::anyhow!(
                "succession-patch: model '{model_name}' clock column '{}' has no resolved \
                 output type — cannot build the tombstone ledger table",
                recipe.clock_col
            )
        })?;
    let tombstone_table =
        smelt_logical::maintenance::emit::tombstone_table_name(&cell.presented_table);
    let ledger_input = recipe
        .source_table
        .strip_prefix("sources.")
        .unwrap_or(&recipe.source_table)
        .to_string();

    let start = Instant::now();
    let mut total_rows = 0usize;

    for step in steps {
        let window_predicate = format!(
            "{col} >= DATE '{start}' AND {col} < DATE '{end}'",
            col = cell.partition_column,
            start = step.range.start,
            end = step.range.end,
        );
        let event_delta = smelt_logical::maintenance::emit::emit_succession_event_delta(
            &cell.source_table,
            &recipe.row_local_projection,
            recipe.pre_filter.as_deref(),
            &window_predicate,
        );
        reporter.maintenance_statements(
            run_id,
            model_name,
            None,
            &smelt_logical::maintenance::emit::StatementGroup {
                statements: vec![event_delta.clone()],
                transactional: false,
            },
        );

        // Ensure the presented shell and the tombstone ledger exist BEFORE
        // the clock-tie probe reads them (the probe's domain CTE selects
        // FROM both) — idempotent DDL only (`CREATE TABLE IF NOT EXISTS`
        // for the tombstone table; the presented shell only when the table
        // does not already exist), never a data write, so this precedes
        // "no write reaches the backend before the probe holds" without
        // violating it. Uniform every window, including the first — no
        // special-cased branch (design bullet "Uniform patch path for
        // every window, including the first").
        let table_exists = backend.table_exists(schema, table).await.unwrap_or(false);
        let mut ensure_sqls = vec![
            ddl_duckdb::generate_tombstone_table_ddl(
                &tombstone_table,
                &key_cols_typed,
                &recipe.clock_col,
                &clock_type,
            ),
            ddl_duckdb::generate_ledger_table_ddl(schema),
        ];
        if !table_exists {
            let shell = smelt_logical::maintenance::emit::emit_create_empty_table(
                &cell.presented_table,
                columns,
                dialect,
            );
            ensure_sqls.push(shell.statements[0].sql.clone());
        }
        for ensure_sql in &ensure_sqls {
            backend
                .execute_sql(ensure_sql)
                .await
                .map_err(|e| anyhow::anyhow!("succession-patch ensure DDL failed: {e}"))?;
        }

        // The clock-tie probe (`incremental_shapes.md` §"Run shape and late
        // events" — "Clock ties"): read-only, runs before any write this
        // step.
        let probe_stmt = smelt_logical::maintenance::emit::emit_succession_clock_tie_probe(
            &cell.presented_table,
            &recipe.key_cols,
            &recipe.clock_col,
            &recipe.payload_columns,
            recipe.delete_flag_expr.as_deref(),
            &event_delta.sql,
            dialect,
        );
        reporter.maintenance_statements(
            run_id,
            model_name,
            None,
            &smelt_logical::maintenance::emit::StatementGroup {
                statements: vec![probe_stmt.clone()],
                transactional: false,
            },
        );
        let probe_ctx = crate::probes::ProbeContext {
            probe_code: "SuccessionClockTie".to_string(),
            fact: "succession_clock".to_string(),
            model: model_name.to_string(),
            cell: format!("{} succession patch", cell.presented_table),
            remedy: "correct the source data so the same (key, clock) pair never carries two \
                     distinct content/delete-flag combinations, or use a finer clock column"
                .to_string(),
        };
        match crate::probes::dispatch_probe(backend, probe_policy, &probe_ctx, &probe_stmt.sql)
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?
        {
            crate::probes::ProbeVerdict::Skipped(_) | crate::probes::ProbeVerdict::Held => {}
            crate::probes::ProbeVerdict::Violated { count, sample_keys } => {
                bail!(
                    "SuccessionClockTie: model '{}' key column(s) {:?}, clock column '{}': {} \
                     colliding (k, t) pair(s) resolve to more than one distinct \
                     content/delete-flag combination. Sample keys: {}.{}",
                    model_name,
                    recipe.key_cols,
                    recipe.clock_col,
                    count,
                    sample_keys,
                    crate::probes::probe_violation_suffix(&probe_ctx)
                );
            }
        }

        let ledger_upsert = ddl_duckdb::generate_ledger_upsert_sql(
            schema,
            model_name,
            SUCCESSION_LEDGER_GROUP,
            &ledger_input,
            &step.partition_value,
            &step.range.start,
            &step.range.end,
        );

        let patch_group = smelt_logical::maintenance::emit::emit_succession_patch(
            &cell.presented_table,
            &recipe.key_cols,
            &recipe.clock_col,
            &recipe.payload_columns,
            &recipe.lead_derived,
            &recipe.lag_derived,
            recipe.delete_flag_expr.as_deref(),
            &event_delta.sql,
            dialect,
        );
        reporter.maintenance_statements(run_id, model_name, None, &patch_group);

        let pre_write_sqls = vec![ledger_upsert];
        crate::execute::retry_backend_call(retry, || {
            backend.execute_write_with_bookkeeping(&[], &pre_write_sqls, &patch_group)
        })
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "Failed to execute succession-patch model '{}': {}",
                model_name,
                e
            )
        })?;

        total_rows = backend.get_row_count(schema, table).await.unwrap_or(0);
    }

    Ok(ExecutionResult {
        model_name: model_name.to_string(),
        duration: start.elapsed(),
        row_count: total_rows,
        preview: None,
    })
}
