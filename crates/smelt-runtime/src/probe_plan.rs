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

/// One declared-fact probe this model would dispatch on a consuming run —
/// the fact, its named diagnostic, the maintenance cell it licenses, and
/// its static per-run cost. Never carries executable SQL: `smelt explain`
/// stays offline (`docs/specs/cli.md` §"`smelt explain <model>`
/// maintenance-plan report").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbePlanEntry {
    pub fact: String,
    pub probe: String,
    pub cell: String,
    pub cost: String,
}

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

    entries
}
