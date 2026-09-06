use super::*;

/// The open write-pattern registry's [`smelt_logical::maintenance::
/// BackendWriteCapabilities`] for a declared backend name (`smelt.yml`
/// `targets.*.type`, lower-cased — the same vocabulary
/// `smelt_logical::lowering::backend_supports_struct_literal` and
/// `project_active_backends` already use). The single owner of the
/// name→struct mapping stays `smelt_dialect::BackendCapabilities`'s own
/// constructors — this only narrows the two booleans the write-pattern
/// registry needs (`CLAUDE.md` §"Layered single-ownership": `smelt-logical`
/// stays below `smelt-dialect`, so it cannot hold this mapping itself).
/// An unrecognised backend name conservatively reports no capability at
/// all — a `write:` pin naming a capability-gated pattern is refused rather
/// than silently assumed available.
pub fn backend_write_capabilities_for(
    backend_name: &str,
) -> smelt_logical::maintenance::BackendWriteCapabilities {
    let caps = match backend_name.to_ascii_lowercase().as_str() {
        "duckdb" => smelt_dialect::BackendCapabilities::duckdb(),
        "spark" | "databricks" => smelt_dialect::BackendCapabilities::spark(),
        _ => {
            return smelt_logical::maintenance::BackendWriteCapabilities::default();
        }
    };
    smelt_logical::maintenance::BackendWriteCapabilities {
        supports_merge: caps.supports_merge,
        supports_column_scoped_merge: caps.supports_column_scoped_merge,
    }
}

/// The [`smelt_dialect::SqlDialect`] a declared backend name (`smelt.yml`
/// `targets.*.type`, lower-cased) prints as — the availability-resolution
/// input `maintenance_plan_diagnostics` feeds
/// [`smelt_logical::maintenance::availability::realisable_state_structures`],
/// mirroring [`backend_write_capabilities_for`]'s own name vocabulary. An
/// unrecognised backend name resolves to `None`, which callers treat as no
/// state structure realisable at all — the same conservative-refusal
/// posture `backend_write_capabilities_for` takes for an unrecognised name,
/// never a silently-assumed dialect.
pub fn backend_dialect_for(backend_name: &str) -> Option<smelt_dialect::SqlDialect> {
    match backend_name.to_ascii_lowercase().as_str() {
        "duckdb" => Some(smelt_dialect::SqlDialect::DuckDB),
        "spark" | "databricks" => Some(smelt_dialect::SqlDialect::SparkSQL),
        "bigquery" => Some(smelt_dialect::SqlDialect::BigQuery),
        _ => None,
    }
}

/// The `on:` address a derived [`Trigger`] resolves to, for matching against
/// a `maintenance.cells[].on` frontmatter entry — mirrors the vocabulary
/// `cells[].on` already writes (`incremental_models.md` §Surface
/// "Frontmatter": "`on: <source-address> | backfill`"). `ColumnAdded` (the
/// definition-change trigger) has no `on:` address of its own — `write:`
/// pins do not address it in this phase.
fn trigger_on_address(trigger: &Trigger) -> Option<String> {
    match trigger {
        Trigger::NewData { source } | Trigger::UpstreamMutation { source } => Some(source.clone()),
        Trigger::Backfill => Some("backfill".to_string()),
        Trigger::ColumnAdded { .. } => None,
    }
}

/// The `maintenance.cells[].write` pin (if any) that addresses `plan_cell`,
/// per the same trigger/column-group matching
/// [`write_pin_diagnostics`] uses — read-only presentation lookup for
/// `smelt explain` (`smelt-cli/src/explain.rs`'s admissible-set + active-pin
/// rows). Never re-derives admission or the registry's admissible set
/// itself (`CLAUDE.md` §"Maintenance-plan purity") — just answers "does a
/// `cells[]` entry name this cell, and if so, what pin did it write".
fn write_pin_matching(
    on_address: &str,
    group: &str,
    column_groups: &[ColumnGroup],
    cells_cfg: &[smelt_core::config::MaintenanceCellConfig],
) -> Option<String> {
    cells_cfg.iter().find_map(|cell_cfg| {
        let pin = cell_cfg.write.as_deref()?;
        if cell_cfg.on != on_address {
            return None;
        }
        let matched_group_name = column_groups
            .iter()
            .find(|g| {
                g.columns
                    .iter()
                    .any(|c| cell_cfg.columns.iter().any(|cc| cc == c))
            })
            .map(|g| g.name());
        let group_matches = group == "{*}" || Some(group.to_string()) == matched_group_name;
        group_matches.then(|| pin.to_string())
    })
}

pub fn matching_write_pin(
    plan_cell: &smelt_logical::maintenance::PlanCell,
    column_groups: &[ColumnGroup],
    cells_cfg: &[smelt_core::config::MaintenanceCellConfig],
) -> Option<String> {
    let on_address = trigger_on_address(&plan_cell.trigger)?;
    write_pin_matching(&on_address, &plan_cell.group, column_groups, cells_cfg)
}

/// The `maintenance.cells[].write` pin (if any) addressing a `refresh: keyed`
/// model's window-forward keyed-fold write (`docs/outcomes/
/// 20260815-definition-delta-migrate/phases/27g-plan.md`). Unlike
/// [`matching_write_pin`], there is no derived [`smelt_logical::maintenance::
/// PlanCell`] to read here — `keyed`'s classifier (`smelt-planner`) runs
/// outside the `MaintenancePlan`/`derive_model_maintenance_plan` machinery
/// entirely — but the keyed fold's cell is always whole-row (`group: "{*}"`),
/// so it matches a `cells[]` entry by its `on:` address alone, using the
/// exact same predicate [`matching_write_pin`] uses (`write_pin_matching`
/// above, with `group` fixed to `"{*}"` and no column groups to consult).
/// Never re-derives admission — a read-only lookup for the runtime write
/// path to resolve the mechanism through
/// [`smelt_logical::maintenance::choice::resolve_keyed_write_mechanism`].
pub fn keyed_fold_write_pin(metadata: &ModelMetadata, driving_source: &str) -> Option<String> {
    let cells_cfg: &[smelt_core::config::MaintenanceCellConfig] = metadata
        .maintenance
        .as_ref()
        .map(|m| m.cells.as_slice())
        .unwrap_or(&[]);
    write_pin_matching(driving_source, "{*}", &[], cells_cfg)
}

/// The `maintenance.defaults`/`maintenance.cells[].prefer`/`cells[].technique`
/// override ladder's effective value for a `refresh: keyed` model's
/// whole-row keyed-fold write-suppression dimension (`docs/outcomes/
/// 20260815-definition-delta-migrate/phases/33-plan.md`). Mirrors
/// [`keyed_fold_write_pin`]'s own reasoning: there is no derived `PlanCell`
/// to consult here, and the keyed fold's cell is always whole-row
/// (`group: "{*}"`), so a `cells[]` entry matches by its `on:` address
/// alone — the same address-only rule [`write_pin_matching`]'s `group ==
/// "{*}"` arm already applies, not [`smelt_logical::maintenance::choice::
/// effective_override`]'s per-column-group `matching_cell`, which would
/// never match a whole-row cell's (typically empty) `columns`.
/// Never re-derives admission — a read-only lookup the runtime write path
/// folds into [`smelt_logical::maintenance::choice::resolve_write_variant`].
pub fn keyed_fold_effective_override(
    metadata: &ModelMetadata,
    driving_source: &str,
) -> smelt_logical::maintenance::choice::EffectiveOverride {
    let maintenance = metadata.maintenance.as_ref();
    let cells_cfg: &[smelt_core::config::MaintenanceCellConfig] =
        maintenance.map(|m| m.cells.as_slice()).unwrap_or(&[]);
    let broad_prefer = maintenance
        .and_then(|m| m.defaults.as_ref())
        .and_then(|d| d.prefer);
    let narrow = cells_cfg.iter().find(|c| c.on == driving_source);
    smelt_logical::maintenance::choice::EffectiveOverride {
        prefer: narrow.and_then(|c| c.prefer).or(broad_prefer),
        technique: narrow.and_then(|c| c.technique),
    }
}

/// Validate every `maintenance.cells[].write` pin against the open
/// write-pattern registry (`incremental_models.md` §"Per-cell write
/// addressing" → "User pins"): an unrecognised name, or one the target
/// backend(s) cannot execute, is `MaintenanceWritePatternUnavailable`; a
/// name the registry and backend admit but whose cell declares none of the
/// pattern's required contract facts (e.g. `write: keyed` on an
/// identity-free cell) is `MaintenanceWriteAddressingRefused`. Checked
/// against every one of the project's `active_backends` — a pin unavailable
/// on any declared target backend refuses, naming that backend, rather than
/// silently passing because a *different* target happens to support it.
///
/// A compare-based pin (`diff_patch`/`keyed_conditional`/`staged_candidate`)
/// is additionally checked against `comparability` — the model's derived
/// P3 column-comparability (`MaintenancePlanResult::comparability`) — via
/// [`smelt_logical::maintenance::cell_equivalence_proof`], so an
/// incomparable compared column or a `WholeRow` cell refuses
/// `MaintenanceWriteAddressingRefused` here too, not just the structural
/// contract-fact check.
///
/// Pure function — the caller ([`maintenance_plan_diagnostics`]) gathers
/// `metadata`/`plan`/`column_groups`/`active_backends`/`comparability` and
/// calls this; it never re-derives the plan itself (Salsa purity rule).
pub fn write_pin_diagnostics(
    metadata: &ModelMetadata,
    plan: &MaintenancePlan,
    column_groups: &[ColumnGroup],
    active_backends: &[String],
    comparability: &[smelt_logical::analysis::walk::ColumnComparability],
) -> Vec<WritePinDiagnostic> {
    use smelt_logical::maintenance::{
        cell_equivalence_proof, resolve_write_pin, OutputContractFacts, RowIdentity,
        WritePinRefusal,
    };

    let Some(maintenance) = metadata.maintenance.as_ref() else {
        return Vec::new();
    };
    let has_partition_axis = metadata.timeseries.is_some();
    let backends: Vec<String> = if active_backends.is_empty() {
        vec!["duckdb".to_string()]
    } else {
        active_backends.to_vec()
    };

    let mut out = Vec::new();
    for cell_cfg in &maintenance.cells {
        let Some(pin) = cell_cfg.write.as_deref() else {
            continue;
        };
        // A whole-row trigger's cell (`NewData`/`Backfill`) carries the
        // `{*}` wildcard group name (`PlanCell::group`'s own doc comment),
        // not a derived `ColumnGroup::name()` — it matches any `cells[]`
        // entry on the same `on:` trigger regardless of `columns`. A
        // per-column-group trigger (`UpstreamMutation`/`ColumnAdded`) only
        // matches a `cells[]` entry whose `columns` land in that same
        // derived group.
        let matched_group_name = column_groups
            .iter()
            .find(|g| {
                g.columns
                    .iter()
                    .any(|c| cell_cfg.columns.iter().any(|cc| cc == c))
            })
            .map(|g| g.name());
        let Some(plan_cell) = plan.cells.iter().find(|c| {
            trigger_on_address(&c.trigger).as_deref() == Some(cell_cfg.on.as_str())
                && (c.group == "{*}" || Some(c.group.clone()) == matched_group_name)
        }) else {
            continue;
        };
        let has_identity = matches!(plan_cell.row_identity.identity, RowIdentity::Key(_));
        let facts = OutputContractFacts {
            has_identity,
            has_partition_axis,
        };
        let cell_label = format!("{:?}", plan_cell.trigger);
        let group_columns: Vec<String> = column_groups
            .iter()
            .find(|g| g.name() == plan_cell.group)
            .map(|g| g.columns.clone())
            .unwrap_or_default();

        for backend_name in &backends {
            let backend_caps = backend_write_capabilities_for(backend_name);
            if let Err(refusal) = resolve_write_pin(
                &cell_label,
                pin,
                backend_name,
                facts,
                backend_caps,
                |pattern| {
                    cell_equivalence_proof(
                        pattern,
                        &group_columns,
                        comparability,
                        &plan_cell.row_identity,
                    )
                },
            ) {
                out.push(match refusal {
                    WritePinRefusal::PatternUnavailable { pattern, backend } => {
                        WritePinDiagnostic::PatternUnavailable { pattern, backend }
                    }
                    WritePinRefusal::AddressingRefused { cell, pattern, why } => {
                        WritePinDiagnostic::AddressingRefused { cell, pattern, why }
                    }
                });
                // One diagnostic per cell is enough — the pin either
                // resolves against every declared backend or it doesn't;
                // reporting per-backend duplicates would just be noise.
                break;
            }
        }
    }
    out
}
