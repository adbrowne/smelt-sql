//! Live dispatch of the three model-scoped declared-fact probes
//! (`timeseries.assert_monotonic`, `functional_dependencies:`,
//! `bounded_domain:`) at a run's pre-write site
//! (`docs/specs/model_properties.md` §"Probe obligation"). Pure probe-set
//! construction from a model's metadata, then dispatch through the shared
//! `smelt_runtime::probes::dispatch_probe` executor — one owner so the
//! full-refresh and incremental-batch call sites in `execute.rs` share
//! identical probe-building and firing logic.

use smelt_backend::{Backend, BackendError, MaintenanceDialect};
use smelt_core::config::TimeseriesConfig;
use smelt_core::metadata::ModelMetadata;
use smelt_logical::maintenance::emit::{
    emit_bounded_domain_probe, emit_functional_dependency_probe, emit_monotonicity_probe,
};
use smelt_state::{ProbeRecord, ProbeRecordOutcome};

use crate::probes::{
    dispatch_probe, probe_violation_suffix, ProbeContext, ProbePolicy, ProbeVerdict,
};

/// One declared model-scoped probe, built pure from a model's metadata: the
/// registry's diagnostic context and the emitted SQL over the run's own
/// processed rows (`scope_select`).
pub struct DeclaredProbe {
    pub ctx: ProbeContext,
    pub sql: String,
}

/// Build the declared-probe set for `model_name`'s run — pure, no I/O.
/// `cell` names the maintenance cell/technique these probes license (the
/// diagnostic's "Licensed cell:" text). Returns one probe per declaration
/// found: zero, one, or several (multiple `functional_dependencies:`
/// entries each get their own probe) (`docs/specs/model_properties.md`
/// §"Probe obligation").
pub fn declared_model_probes(
    model_name: &str,
    cell: &str,
    metadata: Option<&ModelMetadata>,
    timeseries: Option<&TimeseriesConfig>,
    scope_select: &str,
    dialect: MaintenanceDialect,
) -> Vec<DeclaredProbe> {
    let mut probes = Vec::new();

    if let Some(ts) = timeseries {
        if ts.assert_monotonic {
            let partition_key = metadata
                .and_then(|m| m.unique_key.clone())
                .filter(|k| !k.is_empty())
                .unwrap_or_else(|| vec![ts.partition_column.clone()]);
            let stmt = emit_monotonicity_probe(
                scope_select,
                &partition_key,
                &ts.event_time_column,
                dialect,
            );
            probes.push(DeclaredProbe {
                ctx: ProbeContext {
                    probe_code: "DeclaredMonotonicityViolated".to_string(),
                    fact: "timeseries.assert_monotonic".to_string(),
                    model: model_name.to_string(),
                    cell: cell.to_string(),
                    remedy: "disable `assert_monotonic`, fix the upstream ordering, or `smelt \
                             repair` the affected partition"
                        .to_string(),
                },
                sql: stmt.sql,
            });
        }
    }

    if let Some(meta) = metadata {
        for fd in &meta.functional_dependencies {
            let stmt =
                emit_functional_dependency_probe(scope_select, &fd.key, &fd.determines, dialect);
            probes.push(DeclaredProbe {
                ctx: ProbeContext {
                    probe_code: "DeclaredFunctionalDependencyViolated".to_string(),
                    fact: "functional_dependencies:".to_string(),
                    model: model_name.to_string(),
                    cell: cell.to_string(),
                    remedy: "drop the declaration, correct the source data, or `smelt repair` \
                             the affected keys"
                        .to_string(),
                },
                sql: stmt.sql,
            });
        }

        if let Some(bd) = &meta.bounded_domain {
            let stmt =
                emit_bounded_domain_probe(scope_select, &bd.column, bd.max_cardinality, dialect);
            probes.push(DeclaredProbe {
                ctx: ProbeContext {
                    probe_code: "DeclaredBoundedDomainExceeded".to_string(),
                    fact: "bounded_domain:".to_string(),
                    model: model_name.to_string(),
                    cell: cell.to_string(),
                    remedy: "raise `max_cardinality`, narrow the domain upstream, or `smelt \
                             repair` the affected keys"
                        .to_string(),
                },
                sql: stmt.sql,
            });
        }
    }

    probes
}

/// Dispatch every probe in `probes` through the shared cadence/execution
/// helper. The first violation fails loud with the registry's named
/// diagnostic — carrying the probe code, model, fact, violation count,
/// sample keys, licensed cell, and remedy — before any write; a violation
/// is never downgraded to a warning or a silent continue
/// (`docs/specs/model_properties.md` §"Probe obligation"). Returns one
/// [`ProbeRecord`] per probe that held or was skipped, for a future
/// `ModelRunRecord.probes` population.
pub async fn dispatch_declared_model_probes(
    backend: &dyn Backend,
    policy: &ProbePolicy,
    probes: &[DeclaredProbe],
) -> Result<Vec<ProbeRecord>, BackendError> {
    let mut records = Vec::with_capacity(probes.len());
    for probe in probes {
        match dispatch_probe(backend, policy, &probe.ctx, &probe.sql).await? {
            ProbeVerdict::Skipped(_) => records.push(ProbeRecord {
                fact: probe.ctx.fact.clone(),
                probe: probe.ctx.probe_code.clone(),
                outcome: ProbeRecordOutcome::Skipped,
            }),
            ProbeVerdict::Held => records.push(ProbeRecord {
                fact: probe.ctx.fact.clone(),
                probe: probe.ctx.probe_code.clone(),
                outcome: ProbeRecordOutcome::Dispatched,
            }),
            ProbeVerdict::Violated { count, sample_keys } => {
                return Err(BackendError::ExecutionFailed {
                    model: probe.ctx.model.clone(),
                    message: format!(
                        "{}: model '{}' declares `{}`, but {} row(s) violate it. Sample keys: \
                         {}.{}",
                        probe.ctx.probe_code,
                        probe.ctx.model,
                        probe.ctx.fact,
                        count,
                        sample_keys,
                        probe_violation_suffix(&probe.ctx)
                    ),
                });
            }
        }
    }
    Ok(records)
}
