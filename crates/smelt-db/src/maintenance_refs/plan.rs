use std::collections::HashMap;
use std::sync::Arc;

use smelt_core::metadata::{extract_file_metadata, FileMetadata};

use crate::*;

use super::edges::model_edges_for;
use super::refs::{ref_model_source_facts, ref_source_info, ref_timeseries_config};

/// Thin Salsa wrapper around
/// `smelt_logical::maintenance::derive::derive_maintenance_plan`
/// (`incremental_models.md` §Surface "The plan (derived, reported)"): gathers
/// `file`'s referenced sources and declared `maintenance:`/`grain:`
/// frontmatter, then calls
/// [`crate::queries::maintenance::maintenance_plan_diagnostics`] (pure) to
/// derive the plan and map its admission refusals onto a Salsa-safe
/// return shape. Returns the default (empty) result for a model with no
/// maintenance plan (not `refresh: incremental`, or no frontmatter at all).
#[salsa::tracked]
pub fn maintenance_plan(
    db: &dyn salsa::Database,
    workspace: Workspace,
    file: SourceFile,
) -> Arc<crate::queries::maintenance::MaintenancePlanDiagnostics> {
    let text = file.text(db);
    let Ok(FileMetadata::Single {
        metadata,
        sql_offset,
    }) = extract_file_metadata(text)
    else {
        return Arc::new(Default::default());
    };
    let resolved_grain = metadata.resolved_grain();
    if metadata.refresh != Some(smelt_core::config::RefreshStrategy::Incremental)
        || resolved_grain.is_none()
    {
        return Arc::new(Default::default());
    }
    let path = file.path(db);
    let project_root = file.project_root(db).clone();
    let project = find_project(db, workspace, &project_root);

    let sql_body = &text[sql_offset..];
    let refs = smelt_logical::collect_path_refs(sql_body);
    let source_refs: Vec<(String, Option<smelt_core::SourceInfo>)> = refs
        .iter()
        .filter_map(|r| {
            let info = ref_source_info(db, workspace, project, r)?;
            // `SourceFacts::name` is the *bare* source name — the address
            // with the leading `sources` breadcrumb stripped
            // (`crate::maintenance::grouping` resolves a FROM alias's
            // `smelt.<path>` the same way, stripping `sources.` before
            // matching against `SourceFacts.name`; see
            // `maintenance_plan_admission.rs`'s fixtures, which name
            // sources bare — e.g. `"payments"` for `FROM
            // smelt.sources.payments`). Keeping this stripping in one place
            // (here) keeps the trigger/`scan_bounds.per_source` keys and the
            // grouping-derived `mutation_sensitivity` keys in agreement.
            let stripped = r.strip_prefix("smelt.")?;
            let bare = stripped.strip_prefix("sources.").unwrap_or(stripped);
            Some((bare.to_string(), Some(info)))
        })
        .collect();

    let project_scan_bounds = project
        .and_then(|p| (*crate::queries::project::project_maintenance_config(db, p)).clone())
        .and_then(|m| m.scan_bounds);

    let table = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();

    // Mirrors `maintenance_plan_report`'s own composed-driving-source
    // wiring below: a `grain: key` model's driving source may be another
    // maintained model's locality-admitted composed output, not just a
    // declared `sources:` entry.
    let model_scan_bounds = metadata
        .maintenance
        .as_ref()
        .and_then(|m| m.scan_bounds.as_ref());
    let extra_model_sources: Vec<(
        smelt_logical::maintenance::SourceFacts,
        smelt_core::config::Granularity,
    )> = if resolved_grain == Some(smelt_core::config::Grain::Key) {
        refs.iter()
            .filter_map(|r| {
                ref_model_source_facts(
                    db,
                    workspace,
                    r,
                    model_scan_bounds,
                    project_scan_bounds.as_ref(),
                )
            })
            .collect()
    } else {
        Vec::new()
    };

    // `maintenance.cells[].write` pins are validated against every one of
    // the project's declared target backends (`write_pin_diagnostics`'s own
    // doc comment) — reuses the same `project_active_backends` query the
    // `smelt.as_struct()` backend check already threads through
    // `file_diagnostics` (`as_struct_backend_diagnostics_for_file`).
    let active_backends = project
        .and_then(|p| project_active_backends(db, p))
        .unwrap_or_default();

    // `state.warehouse_tables` (`docs/specs/state.md` §"Opting out of
    // warehouse bookkeeping") — the other availability-resolution input,
    // threaded alongside `active_backends` above. Absent/unparseable config
    // resolves to the default posture (`Allowed`), same as an absent
    // `state:` block.
    let warehouse_tables = project
        .and_then(|p| project_warehouse_tables(db, p))
        .unwrap_or_default();

    // The deployed-schema snapshot (`docs/specs/definition_deltas.md`
    // §"Detection"): a Salsa world-fact input the CLI and LSP both register
    // at workspace load (`workspace_ingest::register_deployed_schemas_from_disk`).
    // `deployed_column_names` now threads the snapshot's real column names —
    // a non-skeleton `Trigger::ColumnAdded` cell that cannot be backfilled in
    // place reports `MaintenanceColumnAddNotBackfillable` as a Warning rather
    // than blocking the plan (`definition_deltas.md` §"Detection" posture
    // rules 1-3), matching what `smelt-runtime`'s own run gate already
    // admits. A model declaring `schema_evolution: strategy: full_refresh`
    // derives no definition-change trigger at all (rule 3): the runtime
    // rebuilds the whole table, so there is no in-place backfill obligation
    // to report ahead of time — implemented here, at fact assembly, rather
    // than as a new branch inside the pure derivation.
    let deployed_schema = find_deployed_schema(db, workspace, &project_root, &table);
    let deployed_model_sql: Option<String> = deployed_schema.and_then(|s| {
        s.model_sql(db)
            .as_ref()
            .map(|sql: &Arc<str>| sql.to_string())
    });
    let deployed_partition_column: Option<String> = deployed_schema.and_then(|s| {
        s.partition_column(db)
            .as_ref()
            .map(|col: &Arc<str>| col.to_string())
    });
    let full_refresh_schema_evolution = metadata.schema_evolution.as_ref().is_some_and(|se| {
        se.strategy == smelt_core::metadata::SchemaEvolutionStrategy::FullRefresh
    });
    let deployed_column_names: Vec<String> = if full_refresh_schema_evolution {
        Vec::new()
    } else {
        deployed_schema
            .map(|s| s.columns(db).iter().map(|c| c.to_string()).collect())
            .unwrap_or_default()
    };

    Arc::new(crate::queries::maintenance::maintenance_plan_diagnostics(
        sql_body,
        &table,
        &metadata,
        &source_refs,
        project_scan_bounds.as_ref(),
        &extra_model_sources,
        &active_backends,
        warehouse_tables,
        &deployed_column_names,
        deployed_model_sql.as_deref(),
        deployed_partition_column.as_deref(),
    ))
}

/// Plain (non-Salsa-tracked) counterpart of [`maintenance_plan`] that returns
/// the *full* derived plan — cells, clamps, locality verdicts — rather than
/// the Salsa-safe refusals-only projection. Used by `smelt explain <model>`
/// (`incremental_models.md` §Surface "CLI"), a one-shot CLI report that has no
/// need for Salsa's incremental caching and cannot use the tracked query
/// because [`smelt_logical::maintenance::MaintenancePlan`] does not implement
/// `PartialEq`/`Eq` (the Salsa tracked-return-value requirement the
/// refusals-only [`crate::queries::maintenance::MaintenancePlanDiagnostics`]
/// projection exists to satisfy instead).
///
/// Mirrors the exact input-assembly `maintenance_plan` performs above, but
/// calls [`crate::queries::maintenance::derive_model_maintenance_plan`]
/// directly. Still a Salsa-purity-respecting function: it only assembles
/// inputs from Salsa accessors and calls pure derivation code — it never
/// re-implements admission, locality, or ledger logic. Returns `None` for a
/// model with no maintenance plan (not `refresh: incremental`, or no
/// shape-defining fact declared and no `grain:` to resolve).
pub fn maintenance_plan_report(
    db: &dyn salsa::Database,
    workspace: Workspace,
    file: SourceFile,
) -> Option<crate::queries::maintenance::MaintenancePlanResult> {
    let text = file.text(db);
    let Ok(FileMetadata::Single {
        metadata,
        sql_offset,
    }) = extract_file_metadata(text)
    else {
        return None;
    };
    let resolved_grain = metadata.resolved_grain();
    if metadata.refresh != Some(smelt_core::config::RefreshStrategy::Incremental)
        || resolved_grain.is_none()
    {
        return None;
    }
    let path = file.path(db);
    let project_root = file.project_root(db).clone();
    let project = find_project(db, workspace, &project_root);

    let sql_body = &text[sql_offset..];
    let refs = smelt_logical::collect_path_refs(sql_body);
    let source_refs: Vec<(String, Option<smelt_core::SourceInfo>)> = refs
        .iter()
        .filter_map(|r| {
            let info = ref_source_info(db, workspace, project, r)?;
            let stripped = r.strip_prefix("smelt.")?;
            let bare = stripped.strip_prefix("sources.").unwrap_or(stripped);
            Some((bare.to_string(), Some(info)))
        })
        .collect();

    // Upstream maintained-model edges (`incremental_models.md` §"Upstream model
    // edges"): the model refs that resolve to another maintained model in
    // this project, each carrying that upstream's own validated clock and
    // derived output-delta shape.
    let model_edges = model_edges_for(db, workspace, file);

    let project_scan_bounds = project
        .and_then(|p| (*crate::queries::project::project_maintenance_config(db, p)).clone())
        .and_then(|m| m.scan_bounds);

    let table = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();

    let model_scan_bounds = metadata
        .maintenance
        .as_ref()
        .and_then(|m| m.scan_bounds.as_ref());
    let (mut sources, _scan_bounds_warnings) = crate::queries::maintenance::build_source_facts(
        &source_refs,
        model_scan_bounds,
        project_scan_bounds.as_ref(),
    );
    // A `grain: key` model's driving source may itself be another
    // maintained model's locality-admitted composed output, not just a
    // declared `sources:` entry — `resolve_driving_source` (consulted
    // below via `derive_model_maintenance_plan`) is already agnostic to
    // provenance, so publish every referenced upstream model that clears
    // the locality gate into the same `SourceFacts` candidate list a
    // declared source populates (`incremental_shapes.md` §"Key temporal
    // locality (the time-partitioned output)" — "The output as a clocked
    // source"). Scoped to `grain: key` models only — a `grain: partition`
    // downstream's pushdown against a composed upstream is already derived
    // through `smelt-logical`'s own model-graph registry, not this path.
    let mut model_source_granularities: Vec<smelt_core::config::Granularity> = Vec::new();
    if resolved_grain == Some(smelt_core::config::Grain::Key) {
        for r in &refs {
            if let Some((facts, granularity)) = ref_model_source_facts(
                db,
                workspace,
                r,
                model_scan_bounds,
                project_scan_bounds.as_ref(),
            ) {
                if !sources.iter().any(|s| s.name == facts.name) {
                    sources.push(facts);
                    model_source_granularities.push(granularity);
                }
            }
        }
    }
    let key_recurrences = crate::queries::maintenance::build_key_recurrences(&source_refs);
    let explicitly_mutable: std::collections::HashSet<String> = source_refs
        .iter()
        .filter(|(_, info)| {
            info.as_ref().is_some_and(|i| {
                i.mutation_profile
                    .as_ref()
                    .is_some_and(|m| m.kind == smelt_core::sources::MutationProfile::Mutable)
            })
        })
        .map(|(name, _)| name.clone())
        .collect();

    // The locality gate's granularity-equality structural precondition
    // needs the driving source's granularity regardless of whether it is a
    // declared source or a composed upstream model's own output — combine
    // both candidate pools and pass the union through the single shared
    // "exactly one clocked candidate, else undecided" rule
    // (`smelt_logical::maintenance::locality::single_clocked_granularity`),
    // the same rule `single_clocked_source_granularity` applies over
    // declared sources alone.
    let mut clocked_granularities: Vec<smelt_core::config::Granularity> = source_refs
        .iter()
        .filter_map(|(_, info)| info.as_ref().and_then(|i| i.timeseries.as_ref()))
        .map(|t| t.granularity)
        .collect();
    clocked_granularities.extend(model_source_granularities);
    let driving_source_granularity =
        smelt_logical::maintenance::locality::single_clocked_granularity(clocked_granularities);
    let source_referential_integrity =
        crate::queries::maintenance::build_source_referential_integrity(&source_refs);
    // The deployed-schema snapshot world-fact — see `maintenance_plan`'s own
    // call site for the full rationale: `deployed_column_names` threads the
    // snapshot's real column names (gated to empty under `schema_evolution:
    // strategy: full_refresh`, rule 3), and `model_sql` feeds the
    // skeleton-clause check; `smelt explain`'s report path reads the same
    // registered Salsa input `maintenance_plan` does.
    let deployed_schema = find_deployed_schema(db, workspace, &project_root, &table);
    let deployed_model_sql: Option<String> = deployed_schema.and_then(|s| {
        s.model_sql(db)
            .as_ref()
            .map(|sql: &Arc<str>| sql.to_string())
    });
    let deployed_partition_column: Option<String> = deployed_schema.and_then(|s| {
        s.partition_column(db)
            .as_ref()
            .map(|col: &Arc<str>| col.to_string())
    });
    let full_refresh_schema_evolution = metadata.schema_evolution.as_ref().is_some_and(|se| {
        se.strategy == smelt_core::metadata::SchemaEvolutionStrategy::FullRefresh
    });
    let deployed_column_names: Vec<String> = if full_refresh_schema_evolution {
        Vec::new()
    } else {
        deployed_schema
            .map(|s| s.columns(db).iter().map(|c| c.to_string()).collect())
            .unwrap_or_default()
    };
    let mut result = crate::queries::maintenance::derive_model_maintenance_plan_with_edges(
        sql_body,
        &table,
        &metadata,
        &sources,
        &explicitly_mutable,
        &model_edges,
        driving_source_granularity,
        &key_recurrences,
        &deployed_column_names,
        &source_referential_integrity,
        deployed_model_sql.as_deref(),
        deployed_partition_column.as_deref(),
        &source_refs,
    )?;

    // Decomposed-state summary (`docs/outcomes/20260809-rung2-state-shapes`
    // row 9): only a `grain: key` model can carry state-bearing columns
    // (`rules::cumulative::classify_cumulative` is the keyed classifier),
    // and only when it actually admits — an unadmitted model contributes an
    // empty summary rather than a guess. `classify_cumulative` is the single
    // owner of which spellings are state-bearing; this call derives nothing
    // beyond assembling its inputs from the resolved `SourceInfo`s already
    // gathered above, per the Salsa purity rule.
    if metadata.is_keyed() {
        let mut source_timeseries: smelt_logical::SourceTimeseriesMap = HashMap::new();
        for r in &refs {
            if let Some(ts) = ref_timeseries_config(db, workspace, project, r) {
                source_timeseries.insert(r.clone(), ts);
            }
        }
        if let Ok(classification) = smelt_logical::classify_cumulative(
            sql_body,
            &refs,
            &source_timeseries,
            metadata.timeseries.is_some(),
            &metadata.functional_dependencies,
        ) {
            result.state_columns = smelt_logical::state_column_summary(&classification);
            result.execution_postures = Some(classification.execution_postures());
            result.is_snapshot_reconcile = Some(classification.is_snapshot_reconcile());
        }
    }

    Some(result)
}
