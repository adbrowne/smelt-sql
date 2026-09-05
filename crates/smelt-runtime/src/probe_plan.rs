//! Offline, pure descriptor list for `smelt explain`'s probe rendering
//! (`docs/specs/cli.md` §"`smelt explain <model>` maintenance-plan
//! report"). Never executes any SQL — it calls the same dispatch owners a
//! live run consults (`model_probes::declared_model_probes`,
//! `source_probes::append_only_posture_probes`) over a symbolic scope so
//! the mapping from declaration to probe is derived exactly once
//! (`docs/specs/model_properties.md` §"Probe obligation"), never
//! re-derived here.

use smelt_backend::MaintenanceDialect;
use smelt_core::config::TimeseriesConfig;
use smelt_core::metadata::ModelMetadata;
use smelt_core::sources::SourceInfo;
use smelt_core::ModelFile;
use smelt_logical::analysis::skeleton_closure::{RowPreservation, SkeletonSourceClosure};
use smelt_logical::maintenance::locality::LocalitySlice;
use smelt_logical::maintenance::{KeyLocality, PlanCell};
use smelt_state::source_postures::SourcePostureStore;

/// Single-owned in `smelt_logical::analysis::profile` — the struct only;
/// this builder stays here because it needs `smelt-backend`/`smelt-state`,
/// both above `smelt-logical`
/// (`docs/outcomes/20260905-property-diff/phases/02-plan.md` task 5).
/// Re-exported so every existing `smelt_runtime::probe_plan::ProbePlanEntry`
/// import keeps working unchanged.
pub use smelt_logical::analysis::profile::ProbePlanEntry;

/// Every consuming run dispatches at most one extra query per declared
/// probe (`dispatch_probe`/the append-only baseline snapshot are each a
/// single `SELECT`) — a static statement, not a measurement, since explain
/// never connects to a backend.
const PROBE_COST: &str = "+1 query per consuming run";

/// The full probe plan for `model_name`: the three model-scoped probes
/// (`timeseries.assert_monotonic`, `functional_dependencies:`,
/// `bounded_domain:`), the source append-only posture probe for every
/// eligible consumed source, and the two plan-driven registry rows
/// (`key_recurrence`, `referential_integrity`) read directly off the
/// already-derived plan — `key_recurrence` from an admitted route-3
/// declared recurrence bound (`KeyLocality::slice`), `referential_integrity`
/// from any cell whose `skeleton_source_closure` names the declared route.
/// Pure and offline: builds probe SQL only to confirm a declaration is
/// probe-backed, never executes it.
#[allow(clippy::too_many_arguments)]
pub fn probe_plan_for_model(
    model_name: &str,
    schema: &str,
    table: &str,
    metadata: Option<&ModelMetadata>,
    timeseries: Option<&TimeseriesConfig>,
    model_file: &ModelFile,
    source_infos: &[SourceInfo],
    target_name: &str,
    plan_cells: &[PlanCell],
    key_locality: Option<&KeyLocality>,
    dialect: MaintenanceDialect,
) -> Vec<ProbePlanEntry> {
    let cell = format!("{schema}.{table} (declared)");
    let symbolic_scope = format!("SELECT * FROM {schema}.{table}");

    let mut entries: Vec<ProbePlanEntry> = crate::model_probes::declared_model_probes(
        model_name,
        &cell,
        metadata,
        timeseries,
        &symbolic_scope,
        dialect,
    )
    .into_iter()
    .map(|p| ProbePlanEntry {
        fact: p.ctx.fact,
        probe: p.ctx.probe_code,
        cell: p.ctx.cell,
        cost: PROBE_COST.to_string(),
    })
    .collect();

    let empty_baselines = SourcePostureStore::default();
    entries.extend(
        crate::source_probes::append_only_posture_probes(
            model_name,
            &cell,
            model_file,
            source_infos,
            &empty_baselines,
            target_name,
            schema,
            dialect,
        )
        .into_iter()
        .map(|p| ProbePlanEntry {
            fact: p.ctx.fact,
            probe: p.ctx.probe_code,
            cell: p.ctx.cell,
            cost: PROBE_COST.to_string(),
        }),
    );

    if let Some(KeyLocality {
        slice: LocalitySlice::RecurrenceBounded { .. },
        ..
    }) = key_locality
    {
        entries.push(ProbePlanEntry {
            fact: "key_recurrence".to_string(),
            probe: "KeyedRecurrenceBoundViolated".to_string(),
            cell: format!("{schema}.{table} keyed merge"),
            cost: PROBE_COST.to_string(),
        });
    }

    for pc in plan_cells {
        if let Some(SkeletonSourceClosure::Closed {
            row_preservation: RowPreservation::DeclaredReferentialIntegrity { source },
        }) = &pc.skeleton_source_closure
        {
            entries.push(ProbePlanEntry {
                fact: "referential_integrity".to_string(),
                probe: "SourceCountPreservationViolated".to_string(),
                cell: format!("{schema}.{table} declared-route delta restriction ({source})"),
                cost: PROBE_COST.to_string(),
            });
        }
    }

    // The `contract.retain_departed` reconcile anti-join (phase 34,
    // `docs/outcomes/20260815-definition-delta-migrate/phases/34-plan.md`) —
    // dispatched on every snapshot-reconcile write that suppresses the
    // default point's delete, so it is listed whenever the point is
    // declared, independent of `plan_cells`/`key_locality` derivation.
    if metadata
        .and_then(|m| m.contract.as_ref())
        .and_then(|c| c.retain_departed.as_ref())
        .is_some()
    {
        entries.push(ProbePlanEntry {
            fact: "contract.retain_departed".to_string(),
            probe: "ContractDepartedKeyUnmarked".to_string(),
            cell: format!("{schema}.{table} reconcile anti-join"),
            cost: PROBE_COST.to_string(),
        });
    }

    entries
}

#[cfg(test)]
mod tests {
    use super::*;
    use smelt_core::config::{ContractConfig, RetainDeparted};
    use smelt_core::metadata::ModelMetadata;

    fn model_file(name: &str) -> ModelFile {
        let path = std::path::PathBuf::from(format!("models/{name}.sql"));
        ModelFile {
            name: name.to_string(),
            model_id: smelt_core::ModelId::from_path(path.clone()),
            path,
            content: String::new(),
            refs: vec![],
            parse_errors: vec![],
            metadata: None,
            kind: smelt_core::discovery::ModelKind::Sql,
            address_segments: vec![name.to_string()],
        }
    }

    #[test]
    fn probe_plan_lists_declared_retain_departed() {
        let model_file = model_file("device_snapshot");

        let declared = ModelMetadata {
            contract: Some(ContractConfig {
                retain_departed: Some(RetainDeparted::Bool(true)),
                ..Default::default()
            }),
            ..Default::default()
        };
        let entries = probe_plan_for_model(
            "device_snapshot",
            "main",
            "device_snapshot",
            Some(&declared),
            None,
            &model_file,
            &[],
            "dev",
            &[],
            None,
            MaintenanceDialect::DuckDb,
        );
        let entry = entries
            .iter()
            .find(|e| e.fact == "contract.retain_departed")
            .unwrap_or_else(|| {
                panic!("expected a contract.retain_departed entry, got {entries:?}")
            });
        assert_eq!(entry.probe, "ContractDepartedKeyUnmarked");

        let undeclared = ModelMetadata::default();
        let entries = probe_plan_for_model(
            "device_snapshot",
            "main",
            "device_snapshot",
            Some(&undeclared),
            None,
            &model_file,
            &[],
            "dev",
            &[],
            None,
            MaintenanceDialect::DuckDb,
        );
        assert!(
            !entries.iter().any(|e| e.fact == "contract.retain_departed"),
            "an undeclared model must list no contract.retain_departed entry: {entries:?}"
        );
    }
}
