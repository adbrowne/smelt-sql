//! Live dispatch of the source append-only posture probe
//! (`docs/specs/model_properties.md` §"Probe obligation", row
//! `mutation_profile.kind: append_only`) at a run's pre-write site. Unlike
//! the three model-scoped probes (`crate::model_probes`), this probe's
//! scope is not the run's own compiled SQL — it is a source's recorded
//! per-partition baseline (`smelt_state::source_postures`) compared against
//! that source's current per-partition state, so this module also owns
//! persisting the refreshed baseline after a held probe.
//!
//! An eligible source with **no** recorded baseline yet has nothing to
//! verify — its first observation is unconditionally established as the
//! baseline (there is no prior state it could disprove), gated by the same
//! cadence policy as an ordinary probe dispatch. A source with a recorded
//! baseline is verified: the current per-partition state is compared
//! against it, and only a held verification refreshes the baseline.

use std::collections::HashSet;

use smelt_backend::{Backend, BackendError, MaintenanceDialect};
use smelt_core::sources::{MutationProfile, SourceInfo};
use smelt_core::ModelFile;
use smelt_logical::maintenance::emit::{
    emit_append_only_baseline_snapshot, emit_append_only_posture_probe, AppendOnlyBaselinePartition,
};
use smelt_logical::maintenance::{should_dispatch, ProbeDispatch};
use smelt_state::source_postures::{SourcePosturePartition, SourcePostureStore};
use smelt_state::{ProbeRecord, ProbeRecordOutcome};

use crate::probes::{
    dispatch_probe, probe_violation_suffix, ProbeContext, ProbePolicy, ProbeVerdict,
};

/// Which action a declared append-only source probe performs this run.
pub enum SourcePostureAction {
    /// A recorded baseline exists — verify the source's current
    /// per-partition state against it; refresh the baseline only if the
    /// verification holds.
    Verify { sql: String, snapshot_sql: String },
    /// No recorded baseline exists yet — nothing to verify against, so
    /// this source's current per-partition state is unconditionally
    /// established as the baseline (subject to the same cadence gate as a
    /// verification would be).
    Establish { snapshot_sql: String },
}

/// One declared append-only posture probe, built pure from a source's
/// metadata and its recorded baseline (or lack of one): the registry's
/// diagnostic context plus which action this run performs.
pub struct DeclaredSourceProbe {
    pub source_address: String,
    pub ctx: ProbeContext,
    pub action: SourcePostureAction,
}

/// The bare source-address convention `build_maint_source_facts` and the
/// landed-delta recording already use: `sources.raw.events` addresses
/// become `raw.events`, everything else joins its segments verbatim. One
/// spelling for a source's identity across every state store this run
/// touches (`landed_deltas.json` and `source_postures.json` alike).
fn bare_source_address(segments: &[String]) -> String {
    match segments.split_first() {
        Some((first, rest)) if first == "sources" => rest.join("."),
        _ => segments.join("."),
    }
}

/// Resolve `model_file`'s refs against `source_infos`, pairing each
/// resolved source with its bare address — the same resolution
/// `build_maint_source_facts` performs, kept local to this module so the
/// append-only posture probe owns its own source resolution rather than
/// depending on a `smelt-logical`-shaped intermediate.
fn resolve_model_source_infos<'a>(
    model_file: &ModelFile,
    source_infos: &'a [SourceInfo],
) -> Vec<(String, &'a SourceInfo)> {
    let mut resolved = Vec::new();
    let mut seen = HashSet::new();
    for r in &model_file.refs {
        let segs = r.smelt_ref.to_path();
        let Some(info) = source_infos.iter().find(|s| s.address_segments == segs) else {
            continue;
        };
        let bare = bare_source_address(&segs);
        if seen.insert(bare.clone()) {
            resolved.push((bare, info));
        }
    }
    resolved
}

/// Build the declared append-only posture probe set for `model_name`'s
/// run — pure, no I/O. `cell` names the maintenance cell/technique these
/// probes license. Only a source with ALL of: declared
/// `mutation_profile.kind == append_only`, a `timeseries:` block (for its
/// `partition_column`), and a non-empty declared `columns:` (the digest
/// columns) is eligible at all — a source missing any of these has nothing
/// this probe can check (`docs/outcomes/20260809-probe-backed-facts/
/// outcome.md` phase 6). An eligible source with a non-empty recorded
/// baseline gets a [`SourcePostureAction::Verify`]; one with no recorded
/// baseline gets a [`SourcePostureAction::Establish`].
#[allow(clippy::too_many_arguments)]
pub fn append_only_posture_probes(
    model_name: &str,
    cell: &str,
    model_file: &ModelFile,
    source_infos: &[SourceInfo],
    baselines: &SourcePostureStore,
    target_name: &str,
    target_schema: &str,
    dialect: MaintenanceDialect,
) -> Vec<DeclaredSourceProbe> {
    let mut probes = Vec::new();
    for (source_address, info) in resolve_model_source_infos(model_file, source_infos) {
        let Some(profile) = &info.mutation_profile else {
            continue;
        };
        if profile.kind != MutationProfile::AppendOnly {
            continue;
        }
        let Some(ts) = &info.timeseries else {
            continue;
        };
        if info.columns.is_empty() {
            continue;
        }

        let digest_columns: Vec<String> = info.columns.iter().map(|c| c.name.clone()).collect();
        let table = info.db_name_for_target(target_name, target_schema);
        let snapshot = emit_append_only_baseline_snapshot(
            &table,
            &ts.partition_column,
            &digest_columns,
            dialect,
        );

        let closed_baseline = baselines.closed_baseline(&source_address);
        let action = if closed_baseline.is_empty() {
            SourcePostureAction::Establish {
                snapshot_sql: snapshot.sql,
            }
        } else {
            let baseline: Vec<AppendOnlyBaselinePartition> = closed_baseline
                .into_iter()
                .map(|p| AppendOnlyBaselinePartition {
                    partition_value: p.partition_value,
                    recorded_count: p.recorded_count,
                    recorded_fingerprint: p.recorded_fingerprint,
                    check_fingerprint: p.check_fingerprint,
                })
                .collect();
            let stmt = emit_append_only_posture_probe(
                &table,
                &ts.partition_column,
                &digest_columns,
                &baseline,
                dialect,
            );
            SourcePostureAction::Verify {
                sql: stmt.sql,
                snapshot_sql: snapshot.sql,
            }
        };

        probes.push(DeclaredSourceProbe {
            source_address,
            ctx: ProbeContext {
                probe_code: "SourceMutationProfileViolated".to_string(),
                fact: "mutation_profile.kind: append_only".to_string(),
                model: model_name.to_string(),
                cell: cell.to_string(),
                remedy: "correct the source (undo the delete/reload), or `smelt repair` the \
                         affected partitions"
                    .to_string(),
            },
            action,
        });
    }
    probes
}

/// One source's refreshed baseline, ready for the caller to persist under
/// `state_io_lock` (the same critical-section shape as the landed-delta
/// recording in `execute.rs`).
#[derive(Debug, Clone)]
pub struct RefreshedBaseline {
    pub source_address: String,
    pub partitions: Vec<SourcePosturePartition>,
}

/// Execute `snapshot_sql` and parse it into refreshed baseline partitions
/// (`emit_append_only_baseline_snapshot`'s row shape: `partition_value`,
/// `current_count`, `current_fingerprint`).
async fn read_snapshot(
    backend: &dyn Backend,
    ctx: &ProbeContext,
    source_address: &str,
    snapshot_sql: &str,
) -> Result<Vec<SourcePosturePartition>, BackendError> {
    let batches =
        backend
            .execute_sql(snapshot_sql)
            .await
            .map_err(|e| BackendError::ExecutionFailed {
                model: ctx.model.clone(),
                message: format!(
                    "Failed to execute append-only baseline snapshot for source '{}' (model \
                 '{}'):\n  SQL: {}\n  Error: {}",
                    source_address, ctx.model, snapshot_sql, e
                ),
            })?;
    let rows = crate::check_runner::batches_to_rows(&batches);
    Ok(rows
        .iter()
        .map(|row| SourcePosturePartition {
            partition_value: row.get("partition_value").cloned().unwrap_or_default(),
            recorded_count: row
                .get("current_count")
                .and_then(|v| v.parse().ok())
                .unwrap_or(0),
            recorded_fingerprint: row.get("current_fingerprint").cloned().unwrap_or_default(),
        })
        .collect())
}

/// Dispatch every probe in `probes` through the shared cadence policy. On
/// `Violated` (a [`SourcePostureAction::Verify`] whose comparison fires),
/// fail loud immediately — before touching any baseline — with the
/// registry's named diagnostic (`SourceMutationProfileViolated`, carrying
/// the fact, violation count, sample keys, licensed cell, and remedy). On
/// `Held`, execute the matching baseline-snapshot SQL to read the source's
/// CURRENT per-partition state and return it as the refreshed baseline for
/// that source. A [`SourcePostureAction::Establish`] follows the same
/// cadence gate but never compares — a dispatched establish always "holds"
/// (there is nothing yet to violate) and unconditionally records the
/// snapshot as the source's first baseline. On cadence `Skip`, the baseline
/// is left untouched — no refreshed-baseline entry is returned for that
/// source (`docs/outcomes/20260809-probe-backed-facts/outcome.md` phase 6:
/// "a cadence-skipped run does NOT refresh the baseline").
///
/// Returns the refreshed baselines (for the caller to persist) and one
/// [`ProbeRecord`] per probe that held/established or was skipped, for a
/// future `ModelRunRecord.probes` population.
pub async fn dispatch_and_record_append_only_postures(
    backend: &dyn Backend,
    policy: &ProbePolicy,
    probes: &[DeclaredSourceProbe],
) -> Result<(Vec<RefreshedBaseline>, Vec<ProbeRecord>), BackendError> {
    let mut refreshed = Vec::new();
    let mut records = Vec::with_capacity(probes.len());
    for probe in probes {
        match &probe.action {
            SourcePostureAction::Verify { sql, snapshot_sql } => {
                match dispatch_probe(backend, policy, &probe.ctx, sql).await? {
                    ProbeVerdict::Skipped(_) => {
                        records.push(ProbeRecord {
                            fact: probe.ctx.fact.clone(),
                            probe: probe.ctx.probe_code.clone(),
                            outcome: ProbeRecordOutcome::Skipped,
                        });
                    }
                    ProbeVerdict::Held => {
                        let partitions =
                            read_snapshot(backend, &probe.ctx, &probe.source_address, snapshot_sql)
                                .await?;
                        refreshed.push(RefreshedBaseline {
                            source_address: probe.source_address.clone(),
                            partitions,
                        });
                        records.push(ProbeRecord {
                            fact: probe.ctx.fact.clone(),
                            probe: probe.ctx.probe_code.clone(),
                            outcome: ProbeRecordOutcome::Dispatched,
                        });
                    }
                    ProbeVerdict::Violated { count, sample_keys } => {
                        return Err(BackendError::ExecutionFailed {
                            model: probe.ctx.model.clone(),
                            message: format!(
                                "{}: source '{}' (model '{}') declares `{}`, but {} \
                                 partition(s) violate it. Sample keys: {}.{}",
                                probe.ctx.probe_code,
                                probe.source_address,
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
            SourcePostureAction::Establish { snapshot_sql } => {
                match should_dispatch(policy.cadence, policy.run_ordinal) {
                    ProbeDispatch::Skip(_) => {
                        records.push(ProbeRecord {
                            fact: probe.ctx.fact.clone(),
                            probe: probe.ctx.probe_code.clone(),
                            outcome: ProbeRecordOutcome::Skipped,
                        });
                    }
                    ProbeDispatch::Dispatch => {
                        let partitions =
                            read_snapshot(backend, &probe.ctx, &probe.source_address, snapshot_sql)
                                .await?;
                        refreshed.push(RefreshedBaseline {
                            source_address: probe.source_address.clone(),
                            partitions,
                        });
                        records.push(ProbeRecord {
                            fact: probe.ctx.fact.clone(),
                            probe: probe.ctx.probe_code.clone(),
                            outcome: ProbeRecordOutcome::Dispatched,
                        });
                    }
                }
            }
        }
    }
    Ok((refreshed, records))
}
