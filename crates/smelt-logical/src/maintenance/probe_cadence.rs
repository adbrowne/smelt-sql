//! Probe dispatch cadence — pure policy over [`ProbeCadence`]
//! (`smelt-core`'s `probes:` config) and a run's ordinal for one model.
//!
//! Probe policy is maintenance-plan data, not runtime ad-hockery: the
//! dispatch *decision* lives here so `smelt-runtime`'s dispatch helper only
//! has to act on it (`docs/specs/model_properties.md` §"Probe cadence").

use smelt_core::config::ProbeCadence;

/// Why a probe dispatch was skipped this run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    /// `probes.cadence: off` — every declaration is trusted and recorded
    /// unverified on the run manifest.
    CadenceOff,
    /// `probes.cadence: periodic` and this run's ordinal is not a multiple
    /// of `every_n_runs`.
    NotThisPeriod,
}

/// The cadence decision for one probe on one run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeDispatch {
    Dispatch,
    Skip(SkipReason),
}

/// Decide whether a probe dispatches this run.
///
/// `run_ordinal` is the model's prior-run count (0 for its first run) —
/// ordinal 0 always dispatches under `periodic`, so a model's first
/// consuming run is always verified regardless of `every_n_runs`.
pub fn should_dispatch(cadence: ProbeCadence, run_ordinal: u64) -> ProbeDispatch {
    match cadence {
        ProbeCadence::PerRun => ProbeDispatch::Dispatch,
        ProbeCadence::Off => ProbeDispatch::Skip(SkipReason::CadenceOff),
        ProbeCadence::Periodic { every_n_runs } => {
            if run_ordinal.is_multiple_of(u64::from(every_n_runs)) {
                ProbeDispatch::Dispatch
            } else {
                ProbeDispatch::Skip(SkipReason::NotThisPeriod)
            }
        }
    }
}
