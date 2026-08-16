//! Live dispatch of the `contract.frozen_horizon` late-arrival probe
//! (`docs/specs/incremental_models.md` §"The contract lattice", frozen
//! horizon) at a run's pre-write site. Like
//! `crate::source_probes`, this probe's scope is not the run's own compiled
//! SQL — it is a source's recorded frozen-band baseline
//! (`smelt_state::frozen_band_baselines`) compared against that source's
//! current frozen-band state, so this module also owns persisting the
//! refreshed baseline.
//!
//! Unlike the append-only posture probe, a genuine late arrival does not
//! leave the baseline untouched: the baseline is refreshed after a held
//! verification **and** after a reported violation, so the same late rows
//! are reported once, never re-reported on every subsequent run
//! (`docs/outcomes/20260809-contract-lattice-v1/phases/03-plan.md`). The
//! comparison itself is pure Rust
//! (`smelt_logical::contract::frozen_horizon::late_arrivals`), not a SQL
//! `violation_count` contract, so this module cannot reuse
//! `crate::probes::dispatch_probe` — it consults `should_dispatch` and
//! executes the snapshot SQL directly.

use std::collections::{HashMap, HashSet};

use chrono::{Datelike, NaiveDate};
use smelt_backend::{Backend, BackendError, MaintenanceDialect};
use smelt_core::metadata::ModelMetadata;
use smelt_core::sources::SourceInfo;
use smelt_core::ModelFile;
use smelt_logical::contract::deferral::{
    deferral_violations, pending_window, run_license, subsumption, PendingWindow, RunLicense,
};
use smelt_logical::contract::frozen_horizon::{
    emit_frozen_band_snapshot, frozen_band_before, late_arrivals, PartitionCount,
};
use smelt_logical::maintenance::{should_dispatch, ProbeDispatch};
use smelt_state::frozen_band_baselines::{FrozenBandBaselineStore, FrozenBandPartition};
use smelt_state::intervals::IntervalStore;
use smelt_state::landed_deltas::LandedDeltaStore;
use smelt_state::{ProbeRecord, ProbeRecordOutcome, SubsumedWindow};

use crate::probes::{probe_violation_suffix, ProbeContext, ProbePolicy};

/// The bare source-address convention `crate::source_probes` already uses:
/// `sources.raw.events` becomes `raw.events`. Kept local to this module —
/// the same duplication `source_probes.rs` documents for its own copy,
/// avoiding a dependency on a `smelt-logical`-shaped intermediate for one
/// small pure helper.
fn bare_source_address(segments: &[String]) -> String {
    match segments.split_first() {
        Some((first, rest)) if first == "sources" => rest.join("."),
        _ => segments.join("."),
    }
}

/// Resolve `model_file`'s refs against `source_infos`, pairing each
/// resolved source with its bare address — mirrors
/// `source_probes::resolve_model_source_infos`.
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

/// One declared frozen-horizon late-arrival probe, built pure from a
/// model's metadata and its consumed clocked sources.
pub struct DeclaredContractProbe {
    pub source_address: String,
    pub ctx: ProbeContext,
    pub frozen_before: String,
    pub h_days: i64,
    pub snapshot_sql: String,
}

/// Build the declared frozen-horizon late-arrival probe set for
/// `model_name`'s run — pure, no I/O. Returns an empty set when the model
/// has no `contract.frozen_horizon` declaration (the probe is opt-in only)
/// or when none of its refs resolve to a source with a `timeseries:` clock
/// ("clocked sources" — an unclocked source has no partition axis the
/// frozen band is measured over).
#[allow(clippy::too_many_arguments)]
pub fn frozen_horizon_probes(
    model_name: &str,
    cell: &str,
    model_file: &ModelFile,
    metadata: Option<&ModelMetadata>,
    source_infos: &[SourceInfo],
    end_date: NaiveDate,
    target_name: &str,
    target_schema: &str,
    dialect: MaintenanceDialect,
) -> Vec<DeclaredContractProbe> {
    let Some(h_days) = metadata
        .and_then(|m| m.contract.as_ref())
        .and_then(|c| c.frozen_horizon.as_ref())
        .map(|fh| fh.to_days() as i64)
    else {
        return Vec::new();
    };

    let end_days = end_date.num_days_from_ce() as i64;
    let frozen_before_days = frozen_band_before(end_days, h_days);
    let Some(frozen_before_date) = NaiveDate::from_num_days_from_ce_opt(frozen_before_days as i32)
    else {
        return Vec::new();
    };
    let frozen_before = frozen_before_date.format("%Y-%m-%d").to_string();

    let mut probes = Vec::new();
    for (source_address, info) in resolve_model_source_infos(model_file, source_infos) {
        let Some(ts) = &info.timeseries else {
            continue;
        };
        let table = info.db_name_for_target(target_name, target_schema);
        let snapshot =
            emit_frozen_band_snapshot(&table, &ts.partition_column, &frozen_before, dialect);
        probes.push(DeclaredContractProbe {
            source_address,
            ctx: ProbeContext {
                probe_code: "ContractLateArrivalOutsideHorizon".to_string(),
                fact: "contract.frozen_horizon".to_string(),
                model: model_name.to_string(),
                cell: cell.to_string(),
                remedy: "the arrival is genuine — `smelt repair` the affected frozen partition, \
                         or widen `frozen_horizon` if this lateness is expected"
                    .to_string(),
            },
            frozen_before: frozen_before.clone(),
            h_days,
            snapshot_sql: snapshot.sql,
        });
    }
    probes
}

/// One source's refreshed frozen-band baseline, ready for the caller to
/// persist under `state_io_lock`.
#[derive(Debug, Clone)]
pub struct RefreshedFrozenBandBaseline {
    pub source_address: String,
    pub partitions: Vec<FrozenBandPartition>,
}

/// One genuine late arrival this dispatch observed, carrying the message a
/// caller should fail the run with.
#[derive(Debug, Clone)]
pub struct LateArrivalViolation {
    pub source_address: String,
    pub message: String,
}

/// The result of dispatching a probe set: the refreshed baselines (to
/// persist regardless of whether a violation fired — see the module doc's
/// "report once" rule), the probe records, and any genuine late-arrival
/// violations observed. Callers must persist `refreshed` before consulting
/// `violations`; a non-empty `violations` fails the run, but only after the
/// baseline reflecting what was just observed has been saved.
pub struct FrozenHorizonDispatchResult {
    pub refreshed: Vec<RefreshedFrozenBandBaseline>,
    pub records: Vec<ProbeRecord>,
    pub violations: Vec<LateArrivalViolation>,
}

/// Dispatch every probe in `probes` through the shared cadence policy: no
/// recorded baseline unconditionally establishes; a recorded baseline is
/// compared via [`late_arrivals`] and, if it disproves the frozen-band
/// oracle, recorded as a [`LateArrivalViolation`] — the run's write is not
/// aborted here (the caller decides, after persisting the refreshed
/// baseline for every probe this dispatch touched, held or violated alike).
pub async fn dispatch_and_record_frozen_horizon_probes(
    backend: &dyn Backend,
    policy: &ProbePolicy,
    probes: &[DeclaredContractProbe],
    baselines: &FrozenBandBaselineStore,
) -> Result<FrozenHorizonDispatchResult, BackendError> {
    let mut refreshed = Vec::new();
    let mut records = Vec::with_capacity(probes.len());
    let mut violations = Vec::new();

    for probe in probes {
        match should_dispatch(policy.cadence, policy.run_ordinal) {
            ProbeDispatch::Skip(_) => {
                records.push(ProbeRecord {
                    fact: probe.ctx.fact.clone(),
                    probe: probe.ctx.probe_code.clone(),
                    outcome: ProbeRecordOutcome::Skipped,
                });
                continue;
            }
            ProbeDispatch::Dispatch => {}
        }

        let batches = backend
            .execute_sql(&probe.snapshot_sql)
            .await
            .map_err(|e| BackendError::ExecutionFailed {
                model: probe.ctx.model.clone(),
                message: format!(
                    "Failed to execute frozen-band snapshot for source '{}' (model '{}'):\n  \
                     SQL: {}\n  Error: {}",
                    probe.source_address, probe.ctx.model, probe.snapshot_sql, e
                ),
            })?;
        let rows = crate::check_runner::batches_to_rows(&batches);
        let current: Vec<PartitionCount> = rows
            .iter()
            .map(|row| PartitionCount {
                partition_value: row.get("partition_value").cloned().unwrap_or_default(),
                row_count: row
                    .get("row_count")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0),
            })
            .collect();

        let recorded: Vec<PartitionCount> = baselines
            .get(&probe.source_address)
            .map(|s| {
                s.partitions
                    .iter()
                    .map(|p| PartitionCount {
                        partition_value: p.partition_value.clone(),
                        row_count: p.recorded_count,
                    })
                    .collect()
            })
            .unwrap_or_default();

        let new_baseline = || RefreshedFrozenBandBaseline {
            source_address: probe.source_address.clone(),
            partitions: current
                .iter()
                .map(|p| FrozenBandPartition {
                    partition_value: p.partition_value.clone(),
                    recorded_count: p.row_count,
                })
                .collect(),
        };

        if recorded.is_empty() {
            // First observation: nothing to disprove, unconditionally
            // establish.
            refreshed.push(new_baseline());
            records.push(ProbeRecord {
                fact: probe.ctx.fact.clone(),
                probe: probe.ctx.probe_code.clone(),
                outcome: ProbeRecordOutcome::Dispatched,
            });
            continue;
        }

        let arrivals = late_arrivals(&recorded, &current, &probe.frozen_before);
        refreshed.push(new_baseline());
        records.push(ProbeRecord {
            fact: probe.ctx.fact.clone(),
            probe: probe.ctx.probe_code.clone(),
            outcome: ProbeRecordOutcome::Dispatched,
        });

        if !arrivals.is_empty() {
            let sample: Vec<String> = arrivals
                .iter()
                .take(5)
                .map(|a| format!("{} (+{} row(s))", a.partition, a.added_rows))
                .collect();
            violations.push(LateArrivalViolation {
                source_address: probe.source_address.clone(),
                message: format!(
                    "{}: source '{}' (model '{}') declares `{}`, but {} partition(s) received \
                     rows after being frozen (H = {} day(s)). Sample: {}.{}",
                    probe.ctx.probe_code,
                    probe.source_address,
                    probe.ctx.model,
                    probe.ctx.fact,
                    arrivals.len(),
                    probe.h_days,
                    sample.join(", "),
                    probe_violation_suffix(&probe.ctx)
                ),
            });
        }
    }

    Ok(FrozenHorizonDispatchResult {
        refreshed,
        records,
        violations,
    })
}

/// One declared `contract.deferral` probe, built pure from a model's
/// metadata — model-level granularity only this phase (the two capabilities
/// `deferral` licenses, run skipping and work subsumption, are per-cell
/// scheduling work deferred to
/// `docs/outcomes/20260809-contract-lattice-v1/outcome.md` phase 5; the
/// triple this phase lands operates at the same model granularity
/// `frozen_horizon`'s probe already does).
pub struct DeclaredDeferralProbe {
    pub ctx: ProbeContext,
    pub d_days: i64,
}

/// Build the declared `deferral` probe set for `model_name`'s run — pure, no
/// I/O. Returns an empty set when the model has no `contract.deferral`
/// declaration (the probe is opt-in only, mirroring
/// [`frozen_horizon_probes`]).
pub fn deferral_probes(
    model_name: &str,
    cell: &str,
    metadata: Option<&ModelMetadata>,
) -> Vec<DeclaredDeferralProbe> {
    let Some(d_days) = metadata
        .and_then(|m| m.contract.as_ref())
        .and_then(|c| c.deferral.as_ref())
        .map(|d| d.to_days() as i64)
    else {
        return Vec::new();
    };

    vec![DeclaredDeferralProbe {
        ctx: ProbeContext {
            probe_code: "ContractDeferralExceeded".to_string(),
            fact: "contract.deferral".to_string(),
            model: model_name.to_string(),
            cell: cell.to_string(),
            remedy: "the lag is genuine — let the run catch up, or widen `deferral` if this lag \
                     is expected"
                .to_string(),
        },
        d_days,
    }]
}

/// One genuine deferral-exceeded violation, carrying the message a caller
/// should fail the run with.
#[derive(Debug, Clone)]
pub struct DeferralExceededViolation {
    pub message: String,
}

/// Evaluate every probe in `probes` against the ledger's own recorded
/// frontiers — `interval_store`'s per-model latest recorded interval end
/// (the maintained frontier) and the latest covered interval end across
/// `clocked_source_addresses` in `landed_deltas` (the input frontier). Pure
/// beyond reading the two already-in-memory stores the caller loaded; no
/// backend I/O, unlike `frozen_horizon`'s probe (`contract::deferral`'s
/// module doc: the probe emits no SQL, both frontiers are already-recorded
/// ledger state). Cadence-gated the same way as every other declared-fact
/// probe.
pub fn evaluate_deferral(
    policy: &ProbePolicy,
    probes: &[DeclaredDeferralProbe],
    model_name: &str,
    clocked_source_addresses: &[String],
    interval_store: &IntervalStore,
    landed_deltas: &LandedDeltaStore,
) -> (Vec<ProbeRecord>, Vec<DeferralExceededViolation>) {
    let mut records = Vec::with_capacity(probes.len());
    let mut violations = Vec::new();

    let (maintained_frontier, input_frontier) = resolve_deferral_frontiers(
        model_name,
        clocked_source_addresses,
        interval_store,
        landed_deltas,
    );

    for probe in probes {
        match should_dispatch(policy.cadence, policy.run_ordinal) {
            ProbeDispatch::Skip(_) => {
                records.push(ProbeRecord {
                    fact: probe.ctx.fact.clone(),
                    probe: probe.ctx.probe_code.clone(),
                    outcome: ProbeRecordOutcome::Skipped,
                });
                continue;
            }
            ProbeDispatch::Dispatch => {}
        }

        records.push(ProbeRecord {
            fact: probe.ctx.fact.clone(),
            probe: probe.ctx.probe_code.clone(),
            outcome: ProbeRecordOutcome::Dispatched,
        });

        if let Some(violation) = deferral_violations(
            &probe.ctx.cell,
            maintained_frontier,
            input_frontier,
            probe.d_days,
        ) {
            violations.push(DeferralExceededViolation {
                message: format!(
                    "{}: cell '{}' (model '{}') declares `{}`, but the measured lag ({} day(s)) \
                     exceeds the declared D ({} day(s)).{}",
                    probe.ctx.probe_code,
                    violation.cell,
                    probe.ctx.model,
                    probe.ctx.fact,
                    violation.lag,
                    violation.d,
                    probe_violation_suffix(&probe.ctx)
                ),
            });
        }
    }

    (records, violations)
}

/// Both ledger frontiers `contract.deferral`'s licensing decisions compare
/// — `model_name`'s maintained frontier (`IntervalStore`'s per-model latest
/// recorded interval end) and the input frontier (the latest covered
/// interval end across `clocked_source_addresses` in `LandedDeltaStore`).
/// Shared by [`evaluate_deferral`] (the probe) and [`deferral_decision`]
/// (the scheduler) so both read the same two frontiers the same way.
fn resolve_deferral_frontiers(
    model_name: &str,
    clocked_source_addresses: &[String],
    interval_store: &IntervalStore,
    landed_deltas: &LandedDeltaStore,
) -> (Option<i64>, Option<i64>) {
    let maintained_frontier = interval_store
        .get(model_name)
        .and_then(|mi| mi.latest_date())
        .map(|d| d.num_days_from_ce() as i64);
    let input_frontier = clocked_source_addresses
        .iter()
        .filter_map(|addr| landed_deltas.get(addr))
        .filter_map(|landing| landing.covered_intervals.last())
        .filter_map(|iv| NaiveDate::parse_from_str(&iv.end, "%Y-%m-%d").ok())
        .map(|d| d.num_days_from_ce() as i64)
        .max();
    (maintained_frontier, input_frontier)
}

/// The run-skip license and pending window `contract.deferral` grants one
/// model for this run, computed once before the wavefront scheduler starts
/// (`docs/outcomes/20260809-contract-lattice-v1/phases/05-plan.md`). Thin
/// over `smelt_logical::contract::deferral`'s pure functions — this module
/// contributes no independent lag-vs-window comparison of its own.
#[derive(Debug, Clone, Copy)]
pub struct DeferralDecision {
    pub license: RunLicense,
    pub pending: Option<PendingWindow>,
}

/// Build `model_name`'s [`DeferralDecision`], or `None` when it declares no
/// `contract.deferral` (the scheduling decision is opt-in only, mirroring
/// [`deferral_probes`]).
pub fn deferral_decision(
    model_name: &str,
    metadata: Option<&ModelMetadata>,
    clocked_source_addresses: &[String],
    interval_store: &IntervalStore,
    landed_deltas: &LandedDeltaStore,
) -> Option<DeferralDecision> {
    let d_days = metadata
        .and_then(|m| m.contract.as_ref())
        .and_then(|c| c.deferral.as_ref())
        .map(|d| d.to_days() as i64)?;
    let (maintained_frontier, input_frontier) = resolve_deferral_frontiers(
        model_name,
        clocked_source_addresses,
        interval_store,
        landed_deltas,
    );
    Some(DeferralDecision {
        license: run_license(maintained_frontier, input_frontier, d_days),
        pending: pending_window(maintained_frontier, input_frontier),
    })
}

/// Closes `own_skip` (models `contract.deferral` itself licensed to skip)
/// over `upstream_map` (every selected model's transitive upstream set) to
/// the fixpoint: a dependent of a deferral-skipped model is skipped with
/// it, since a dependent that ran while its upstream was deferred would
/// record interval coverage for a window its upstream never folded, and
/// would never revisit it — the silent hole the default point forbids
/// (`docs/outcomes/20260809-contract-lattice-v1/outcome.md` phase 5
/// decision log). Mirrors `execute_project`'s own `--resume` downstream
/// closure.
pub fn propagate_deferral_skip(
    own_skip: &HashSet<String>,
    upstream_map: &HashMap<String, HashSet<String>>,
    all_models: &[String],
) -> HashSet<String> {
    let mut skip = own_skip.clone();
    loop {
        let mut added = false;
        for name in all_models {
            if skip.contains(name) {
                continue;
            }
            if let Some(ups) = upstream_map.get(name) {
                if ups.iter().any(|u| skip.contains(u)) {
                    skip.insert(name.clone());
                    added = true;
                }
            }
        }
        if !added {
            break;
        }
    }
    skip
}

/// Whether `pending` (this model's `contract.deferral` pending window,
/// already computed by [`deferral_decision`]) is subsumed by a run whose
/// own write range is `[scheduled_start, scheduled_end]`, given whether a
/// prior run manifest recorded a `skipped_deferral` for this model. Thin
/// date-formatting wrapper over `smelt_logical::contract::deferral::
/// subsumption` — the licensing decision itself is not reimplemented here.
pub fn subsumed_window(
    pending: Option<PendingWindow>,
    prior_run_recorded_skip: bool,
    scheduled_start: NaiveDate,
    scheduled_end: NaiveDate,
) -> Option<SubsumedWindow> {
    let work = subsumption(
        pending,
        prior_run_recorded_skip,
        scheduled_start.num_days_from_ce() as i64,
        scheduled_end.num_days_from_ce() as i64,
    )?;
    Some(SubsumedWindow {
        maintained_exclusive: NaiveDate::from_num_days_from_ce_opt(
            work.pending.maintained_exclusive as i32,
        )
        .map(|d| d.format("%Y-%m-%d").to_string())
        .unwrap_or_default(),
        input_inclusive: NaiveDate::from_num_days_from_ce_opt(work.pending.input_inclusive as i32)
            .map(|d| d.format("%Y-%m-%d").to_string())
            .unwrap_or_default(),
    })
}
