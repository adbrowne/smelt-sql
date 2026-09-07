use super::retry::*;

use anyhow::Result;

use smelt_backend::Backend;

use crate::reporter::RunReporter;
use crate::types::ExecuteRequest;

/// Create the EMPTY target table for a **self-referential** model's first
/// run (`docs/specs/incremental_shapes.md` §"First-run and backfill" —
/// "First-run bootstrap for a self-referential model"). Shared by both
/// dispatch arms in `execute_project` — the windowed incremental batch loop
/// (bootstrap-then-DELETE+INSERT) and the unwindowed full-refresh arm
/// (drop-bootstrap-INSERT) — so the schema lookup, the fail-loud guards,
/// and the emitter call can never drift between them.
///
/// Fail-loud guards (`architecture.md` §"Fail-loud discipline") — DDL is
/// authored from `upstream.models`' resolved output schema, so the schema
/// must actually be trustworthy before any statement reaches the backend:
///
/// 1. the model's schema fixpoint must have **converged**
///    (`UpstreamSchemas::unconverged_self_ref_models`) — an unconverged
///    last-iterate is never silently accepted as "the schema";
/// 2. the resolved column list must be non-empty;
/// 3. no output column may still be `DataType::Unknown` — an
///    `UNKNOWN`-typed column would otherwise surface as an opaque engine
///    catalog error (`Type with name UNKNOWN does not exist`) instead of a
///    diagnostic naming the column and the fix.
///
/// The emitted DDL comes from the pure single-owner emitter
/// (`smelt_logical::maintenance::emit::emit_create_empty_table`); this
/// function only resolves inputs, guards, reports, and executes.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn bootstrap_self_ref_empty_target(
    request: &ExecuteRequest,
    backend: &dyn Backend,
    schema: &str,
    model_file: &smelt_core::ModelFile,
    model_display_name: &str,
    upstream: &crate::compile::UpstreamSchemas,
    reporter: &dyn RunReporter,
    run_id: &str,
) -> Result<()> {
    // `UpstreamSchemas.models` is keyed on `ModelFile::name` — the bare
    // leaf name (file stem), not the full dotted graph address — matching
    // the same key every `smelt.ref()` lookup elsewhere uses
    // (`StaticRefSchemaProvider::resolved_columns` is always called with a
    // bare table name).
    if upstream
        .unconverged_self_ref_models
        .contains(&model_file.name)
    {
        anyhow::bail!(
            "model '{model_display_name}' is self-referential and its output-schema \
             fixpoint did not converge — refusing to bootstrap an empty target table \
             from an unconverged schema. Pre-create the table manually (or add explicit \
             CASTs to the model's output columns) and re-run."
        );
    }
    let columns: Vec<(String, smelt_types::DataType)> = upstream
        .models
        .get(&model_file.name)
        .map(|cols| {
            cols.iter()
                .map(|(name, typed)| (name.clone(), typed.data_type.clone()))
                .collect()
        })
        .unwrap_or_default();
    if columns.is_empty() {
        anyhow::bail!(
            "model '{model_display_name}' is self-referential but its output schema could \
             not be resolved — cannot bootstrap an empty target table without a known \
             column list"
        );
    }
    let unknown_columns: Vec<&str> = columns
        .iter()
        .filter(|(_, dt)| matches!(dt, smelt_types::DataType::Unknown(_)))
        .map(|(name, _)| name.as_str())
        .collect();
    if !unknown_columns.is_empty() {
        anyhow::bail!(
            "model '{model_display_name}' is self-referential but the type of output \
             column(s) [{}] could not be inferred — cannot bootstrap an empty target \
             table with unknown column types. Add an explicit CAST to those columns \
             (or pre-create the table manually) and re-run.",
            unknown_columns.join(", ")
        );
    }

    let table_name = format!("{schema}.{}", model_file.db_name_owned());
    let group = smelt_logical::maintenance::emit::emit_create_empty_table(
        &table_name,
        &columns,
        smelt_backend::maintenance_dialect(backend.dialect()),
    );
    reporter.maintenance_statements(run_id, model_display_name, None, &group);
    retry_statement_group(request, run_id, model_display_name, reporter, || {
        backend.execute_statement_group(&group)
    })
    .await
    .map_err(|e| anyhow::anyhow!("{}", e))?;
    Ok(())
}
