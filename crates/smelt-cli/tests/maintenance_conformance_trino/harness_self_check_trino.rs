//! Trino twin of `maintenance_conformance/harness_self_check.rs` plus the
//! same "prove the oracle catches a real divergence on THIS backend before
//! any `<family>_on_trino` wrapper is trusted" ordering discipline the
//! BigQuery leg's own doc comment states
//! (`docs/outcomes/20260913-trino-incremental/phases/08-plan.md` test 7):
//! this is the first Trino-arm test written and must be observed FAILING the
//! oracle (i.e. the corruption is actually caught) before any
//! `<family>_on_trino` wrapper is trusted — otherwise a standing green leg
//! could just mean the oracle is vacuously true on Trino too. Corruption
//! goes through `Backend::execute_sql` (`TrinoConformanceBackend::corrupt_sql`),
//! never a raw write.

use smelt_maintenance_testkit::families::{harness_self_check, ConformanceBackend};

use crate::backend::TrinoConformanceBackend;

/// `oracle_flags_a_seeded_divergence_on_trino`.
#[test]
fn oracle_flags_a_seeded_divergence_on_trino() {
    let b = TrinoConformanceBackend::new("harness_self_check");
    if let Some(reason) = b.skip_reason() {
        eprintln!("{reason} — skipping oracle_flags_a_seeded_divergence_on_trino");
        return;
    }
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(harness_self_check::run_oracle_flags_a_seeded_divergence(&b))
        .expect("harness self-check failed on Trino");
}
