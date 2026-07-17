//! `key_per_partition` fail-loud refusal
//! (`docs/plans/20260715-composed-axes-conditional-maintenance.md` Phase A0).
//!
//! `crates/smelt-db/src/queries/maintenance.rs` maps a declared `grain:
//! key_per_partition` to [`unsupported_grain_plan`] instead of silently
//! deriving a keyed plan with an empty `unique_key` — there is no trajectory/
//! backfill machinery yet to back a real `key_per_partition` plan, so the
//! honest behaviour is a named refusal, not a plan that looks like an
//! ordinary (if degenerate) keyed one.

use smelt_logical::maintenance::{unsupported_grain_plan, Refusal};

#[test]
fn key_per_partition_refuses_not_silently_collapses() {
    let plan = unsupported_grain_plan("key_per_partition");

    // Negative: no cell in this plan ever carries a keyed grain (in
    // particular, never the empty-`unique_key` collapse this phase
    // eliminates). This plan derives no cells at all — the only outcome is
    // the refusal below.
    assert!(
        plan.cells.is_empty(),
        "a key_per_partition plan must derive no cells (no keyed plan with an \
         empty unique_key), got {:?}",
        plan.cells
    );

    assert_eq!(
        plan.refusals.len(),
        1,
        "expected exactly one refusal, got {:?}",
        plan.refusals
    );
    match &plan.refusals[0] {
        Refusal::UnsupportedGrain {
            grain,
            tracking_plan,
        } => {
            assert_eq!(grain, "key_per_partition");
            assert_eq!(
                tracking_plan,
                "docs/plans/20260715-composed-axes-conditional-maintenance.md"
            );
        }
        other => panic!("expected Refusal::UnsupportedGrain, got {other:?}"),
    }
}
