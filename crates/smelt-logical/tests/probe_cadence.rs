//! Integration test for `smelt_logical::maintenance::probe_cadence` — the
//! pure probe dispatch cadence decision (`docs/specs/model_properties.md`
//! §"Probe cadence").

use smelt_core::config::ProbeCadence;
use smelt_logical::maintenance::{should_dispatch, ProbeDispatch, SkipReason};

#[test]
fn per_run_dispatches_every_run() {
    for run_ordinal in 0..5 {
        assert_eq!(
            should_dispatch(ProbeCadence::PerRun, run_ordinal),
            ProbeDispatch::Dispatch
        );
    }
}

#[test]
fn off_never_dispatches() {
    for run_ordinal in 0..5 {
        assert_eq!(
            should_dispatch(ProbeCadence::Off, run_ordinal),
            ProbeDispatch::Skip(SkipReason::CadenceOff)
        );
    }
}

#[test]
fn periodic_dispatches_on_the_first_run_then_every_nth() {
    let cadence = ProbeCadence::Periodic { every_n_runs: 3 };
    assert_eq!(should_dispatch(cadence, 0), ProbeDispatch::Dispatch);
    assert_eq!(
        should_dispatch(cadence, 1),
        ProbeDispatch::Skip(SkipReason::NotThisPeriod)
    );
    assert_eq!(
        should_dispatch(cadence, 2),
        ProbeDispatch::Skip(SkipReason::NotThisPeriod)
    );
    assert_eq!(should_dispatch(cadence, 3), ProbeDispatch::Dispatch);
    assert_eq!(should_dispatch(cadence, 6), ProbeDispatch::Dispatch);
}
