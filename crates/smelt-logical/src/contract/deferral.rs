//! `deferral` — the contract-lattice point declaring that a cell's
//! maintained state may lag its inputs by up to `D`
//! (`docs/specs/incremental_models.md` §"Contract relaxations
//! (`contract:`)"). This module single-owns the complete triple: the
//! clock-admissibility validator, the pure lag oracle (`measure_lag`/
//! `within_deferral`), and the probe comparison (`deferral_violations`) that
//! disproves the oracle at runtime.
//!
//! Unlike `frozen_horizon`, the probe emits no SQL — both frontiers it
//! compares are read from state the run already writes (`IntervalStore`'s
//! per-model latest recorded interval end, `LandedDeltaStore`'s per-source
//! latest covered interval end), so the "probe emitter" leg here is this
//! pure comparison plus its `smelt-runtime::contract_probes` dispatch, not a
//! `MaintenanceStatement`.

/// Validates that `contract.deferral` (model-level or a `contract.cells[]`
/// entry) is declared only where there is a clock to measure lag against.
/// `has_clock` is the caller-resolved admissibility — model-level: does the
/// model carry a `timeseries:` clock; cell-level: does the cell's `on:`
/// trigger address a clocked, interval-representable source (`on: backfill`,
/// an unclocked source, and a `mutable_snapshot` source are each
/// inadmissible). Resolving `has_clock` needs the parsed `ModelMetadata`
/// and/or resolved source facts, unavailable to this pure function — the
/// caller (`smelt-db`'s `check_file_diagnostics`) does that resolution.
///
/// Returns `Err` naming the offender when `has_clock` is `false`.
pub fn validate_deferral(has_clock: bool, offender: &str) -> Result<(), String> {
    if !has_clock {
        return Err(format!(
            "contract.deferral requires an interval-representable clock to measure lag \
             against; {offender} has none"
        ));
    }
    Ok(())
}

/// The measured lag between a cell's maintained frontier and its input
/// frontier, in the caller's unit (days, at the current `smelt-runtime` call
/// site — unit-agnostic like `frozen_horizon`'s day-denominated functions).
/// Positive means the input frontier is ahead of the maintained frontier
/// (the cell owes work); zero or negative means fully caught up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeferralLag {
    pub lag: i64,
}

/// The pure oracle transform: `input_frontier - maintained_frontier`. Both
/// frontiers are event-time, already resolved by the caller from the
/// ledger's own recorded state (`IntervalStore::latest_date`,
/// `LandedDeltaStore`'s per-source latest covered end) — this function does
/// no I/O and no frontier resolution of its own.
pub fn measure_lag(maintained_frontier: i64, input_frontier: i64) -> DeferralLag {
    DeferralLag {
        lag: input_frontier - maintained_frontier,
    }
}

/// Whether `lag` is admitted by the declared deferral window `d`: lag up to
/// and including `d` is within the licensed window; lag exceeding `d`
/// disproves the oracle.
pub fn within_deferral(lag: DeferralLag, d: i64) -> bool {
    lag.lag <= d
}

/// A genuine deferral-exceeded violation: `cell`'s measured lag exceeds its
/// declared `d`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeferralViolation {
    pub cell: String,
    pub lag: i64,
    pub d: i64,
}

/// Compares `cell`'s maintained frontier against its input frontier and
/// reports a [`DeferralViolation`] when the measured lag exceeds `d`. A
/// missing maintained frontier (the cell's first run — nothing recorded yet
/// to compare against) or a missing input frontier (nothing has landed on
/// any clocked input yet) establishes rather than violates, mirroring
/// `frozen_horizon::late_arrivals`'s own "nothing recorded, nothing to
/// disprove" posture for a first observation.
pub fn deferral_violations(
    cell: &str,
    maintained_frontier: Option<i64>,
    input_frontier: Option<i64>,
    d: i64,
) -> Option<DeferralViolation> {
    let maintained = maintained_frontier?;
    let input = input_frontier?;
    let lag = measure_lag(maintained, input);
    if within_deferral(lag, d) {
        None
    } else {
        Some(DeferralViolation {
            cell: cell.to_string(),
            lag: lag.lag,
            d,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deferral_requires_a_clock() {
        let model_level = validate_deferral(false, "model 'daily_revenue'").unwrap_err();
        assert!(
            model_level.contains("daily_revenue"),
            "error must name the offending model, got: {model_level}"
        );

        let cell_level = validate_deferral(false, "cell on 'backfill'").unwrap_err();
        assert!(
            cell_level.contains("backfill"),
            "error must name the offending cell trigger, got: {cell_level}"
        );

        assert!(validate_deferral(true, "model 'daily_revenue'").is_ok());
    }

    #[test]
    fn lag_is_input_frontier_minus_maintained_frontier() {
        assert_eq!(measure_lag(100, 100), DeferralLag { lag: 0 });
        assert_eq!(measure_lag(100, 105), DeferralLag { lag: 5 });
        assert_eq!(measure_lag(105, 100), DeferralLag { lag: -5 });
    }

    #[test]
    fn within_deferral_admits_lag_up_to_d_inclusive() {
        assert!(within_deferral(measure_lag(100, 106), 6));
        assert!(!within_deferral(measure_lag(100, 107), 6));
    }

    #[test]
    fn missing_maintained_frontier_is_not_a_violation() {
        assert_eq!(deferral_violations("cell", None, Some(100), 6), None);
        assert_eq!(deferral_violations("cell", Some(100), None, 6), None);
    }

    #[test]
    fn deferral_violations_names_the_cell_and_measured_lag() {
        let violation = deferral_violations("cell", Some(100), Some(110), 6).unwrap();
        assert_eq!(
            violation,
            DeferralViolation {
                cell: "cell".to_string(),
                lag: 10,
                d: 6,
            }
        );
    }

    #[test]
    fn deferral_violations_holds_within_the_window() {
        assert_eq!(deferral_violations("cell", Some(100), Some(106), 6), None);
    }
}
