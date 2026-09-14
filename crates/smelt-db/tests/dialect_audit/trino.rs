//! Trino. Schema leg only — value-leg comparison is
//! `20260913-trino-emission` phase 6's subject. Skips green when
//! `SMELT_TRINO_URL` is unset; the tier is `scripts/trino-up.sh` +
//! `scripts/trino-env.sh`, a per-PR/nightly cost, never a bare-Docker
//! ambient assumption.

use crate::legs::run_schema_leg;
use smelt_oracle_testkit::TrinoOracle;
use smelt_types::DialectId;
use std::sync::LazyLock;

static TRINO: LazyLock<Option<TrinoOracle>> = LazyLock::new(TrinoOracle::from_env);

#[test]
fn schema_leg_trino() {
    let Some(oracle) = TRINO.as_ref() else {
        eprintln!("SMELT_TRINO_URL unset — skipping schema_leg_trino");
        return;
    };
    let outcome = run_schema_leg(DialectId::Trino, oracle);
    assert!(outcome.failures.is_empty(), "{}", outcome.report());
    assert!(
        outcome.probes_compared >= crate::legs::PROBE_COVERAGE_FLOOR,
        "too few probes compared: {}",
        outcome.probes_compared
    );
    eprintln!(
        "COVERAGE[trino schema] probes_compared={}",
        outcome.probes_compared
    );
}
