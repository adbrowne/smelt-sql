//! DuckDB leg tests and the pure leg-decision unit tests. DuckDB needs no
//! live warehouse, so these run per-PR.

use crate::legs::{
    classify_accepted, first_row_difference, run_schema_leg, run_value_leg, AcceptedVerdict,
    PROBE_COVERAGE_FLOOR,
};
use smelt_oracle_testkit::{Cell, DuckDbOracle};
use smelt_types::DialectId;

#[test]
fn schema_leg_duckdb() {
    let oracle = DuckDbOracle::new();
    let outcome = run_schema_leg(DialectId::DuckDb, &oracle);
    assert!(outcome.failures.is_empty(), "{}", outcome.report());
    assert!(
        outcome.probes_compared >= PROBE_COVERAGE_FLOOR,
        "schema leg compared only {} probes — the enumeration collapsed",
        outcome.probes_compared
    );
    eprintln!(
        "COVERAGE[duckdb schema] probes_compared={}",
        outcome.probes_compared
    );
}

#[test]
fn value_leg_duckdb_is_self_consistent() {
    // DuckDB against itself: proves the harness, the fixture and the comparator
    // agree before any cross-engine claim is made.
    let oracle = DuckDbOracle::new();
    let outcome = run_value_leg(DialectId::DuckDb, &oracle, &oracle);
    assert!(outcome.failures.is_empty(), "{}", outcome.report());
    assert!(
        outcome.probes_compared >= PROBE_COVERAGE_FLOOR,
        "value leg compared only {} probes — the enumeration collapsed",
        outcome.probes_compared
    );
    eprintln!(
        "COVERAGE[duckdb value] probes_compared={} schema_only={}",
        outcome.probes_compared,
        outcome.schema_only.len()
    );
}

/// The leg's comparator actually reports a difference — a self-consistent
/// green run proves the plumbing, not the detection.
///
/// The planted values are the real case: DuckDB's `2 ^ 3` is 8, and a dialect
/// reading `^` as bitwise XOR answers 1. Both are the same type, so no schema
/// comparison can tell them apart.
#[test]
fn the_value_leg_reports_a_planted_divergence() {
    let reference = vec![vec![Cell::Int(8)]];
    let xor_reading = vec![vec![Cell::Int(1)]];
    let detail =
        first_row_difference(&reference, &xor_reading).expect("8 and 1 are not the same number");
    assert!(detail.contains("row 0 column 0"), "{detail}");

    // …and a shorter result set is a difference too, not a silently truncated
    // comparison.
    assert!(first_row_difference(&reference, &[]).is_some());
    assert!(first_row_difference(&reference, &reference).is_none());
}

/// A ledger row that the engine has started accepting is reported as stale
/// rather than left standing — the same two-sidedness the hardening baseline
/// has. Proven directly against `classify_accepted` (the pure decision
/// `probe_schema_once` delegates to) rather than through a live ledger row +
/// oracle pair: no real DuckDB Schema-leg gap survives phase 4 of
/// `docs/outcomes/20260904-dialect-emission-vocabulary` to borrow for this,
/// and mixing dialects (printing for one, querying with another's oracle)
/// produces a real syntax mismatch rather than the scenario under test.
#[test]
fn a_ledger_row_the_engine_now_accepts_is_reported_stale() {
    assert!(matches!(
        classify_accepted(false, true),
        AcceptedVerdict::StaleLedgerRow
    ));
}

#[test]
fn an_unsupported_verdict_the_engine_now_accepts_is_reported() {
    assert!(matches!(
        classify_accepted(true, false),
        AcceptedVerdict::UnsupportedButAccepted
    ));
    // Unsupported takes priority: a pair that is somehow both a declared
    // Unsupported verdict and a ledger row still reports the Unsupported
    // finding, never silently prefers the other.
    assert!(matches!(
        classify_accepted(true, true),
        AcceptedVerdict::UnsupportedButAccepted
    ));
}

#[test]
fn a_pair_the_engine_accepts_with_no_gap_is_type_checked() {
    assert!(matches!(
        classify_accepted(false, false),
        AcceptedVerdict::CheckType
    ));
}
