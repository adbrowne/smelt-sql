//! `input_delta_discovery` as a real admission input, not decoration.
//!
//! `input_delta_discovery_dead_code_tripwire.rs` used to fail the build the
//! moment a production caller of `input_delta_discovery` appeared, because
//! the function's `WindowForward` verdict for a clocked `Mutable` source has
//! no branch for an in-place update to an already-processed partition
//! (`docs/research/property-discovery/ledger.md` cell SC-2) — wiring it into
//! a real maintenance path was gated on an explicit `MutationProfile::Mutable`
//! guard and human sign-off (design §8(4)).
//!
//! MP5 (`docs/plans/20260707-maintenance-plan-impl.md`) is that sign-off:
//! `derive_maintenance_plan`'s keyed-fold admission
//! (`crates/smelt-logical/src/maintenance/derive.rs::derive_new_data`) calls
//! `input_delta_discovery` as the source-posture half of the faithful-fold
//! obligation (`incremental_models.md` §"Per-cell admission" obligation 2), and
//! the verdict it returns actually *selects between two distinguishable
//! refusal reasons* — not merely appearing inside an interpolated string.
//!
//! This test constructs two `MutationProfile::MutableSnapshot` sources that
//! differ ONLY in whether they carry a partition clock (`partition_col`):
//! - clocked ⇒ `input_delta_discovery` returns `WindowForward` — the SC-2
//!   blind spot: a window-forward read never revisits an already-processed
//!   partition, so an in-place update there goes entirely unseen, not merely
//!   un-undoable.
//! - unclocked ⇒ `input_delta_discovery` returns `SnapshotDiff` — a full
//!   re-scan at least *sees* the change; it is un-undoable, not invisible.
//!
//! Both refuse the fold (obligation 2 fails either way — `MutableSnapshot` is
//! never append-only), but the *reason text* must differ in a way that is
//! traceable to the discovery verdict, proving the verdict is consulted for
//! more than decoration. A prior version of this test only `rg`-grepped for
//! the string `input_delta_discovery` inside `derive.rs`, which would pass
//! even if the verdict were computed and discarded; this version fails
//! unless the verdict actually changes plan output.

use std::collections::BTreeSet;

use smelt_logical::maintenance::derive::{derive_maintenance_plan, FoldSpec, ModelInputs};
use smelt_logical::maintenance::{
    ColumnGroup, Grain, MutationProfile, OutputSpec, Refusal, SourceFacts, Trigger,
};
use smelt_types::SqlFunction;

fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

/// A `MutableSnapshot` source, varying only whether it is clocked
/// (`partition_col`) — the sole input that changes `input_delta_discovery`'s
/// verdict for a non-append-only source (`WindowForward` vs `SnapshotDiff`;
/// `ChangeFeed` is out of scope for the plan layer per `source_shape`'s doc
/// comment in `derive.rs`).
fn mutable_source(clocked: bool) -> SourceFacts {
    SourceFacts {
        name: "payments".to_string(),
        mutation: MutationProfile::MutableSnapshot,
        partition_col: clocked.then(|| "pay_date".to_string()),
        unique_key: vec![],
        allow_full_scan: false,
    }
}

fn inputs_with(source: SourceFacts) -> (ModelInputs<'static>, Trigger) {
    let inputs = ModelInputs {
        sql: "SELECT user_id, SUM(amount) AS lifetime_spend \
              FROM smelt.sources.payments GROUP BY user_id",
        output: OutputSpec {
            table: "lifetime_spend".to_string(),
            grain: Grain::Key {
                unique_key: strings(&["user_id"]),
            },
            skeleton_columns: set(&["user_id"]),
        },
        sources: vec![source],
        column_groups: vec![ColumnGroup {
            columns: strings(&["lifetime_spend"]),
            mutation_sensitivity: set(&["payments"]),
            membership_sensitivity: BTreeSet::new(),
        }],
        fold: Some(FoldSpec {
            add_columns: vec![("lifetime_spend".to_string(), SqlFunction::Sum)],
        }),
        column_add_proof: None,
    };
    let trigger = Trigger::NewData {
        source: "payments".to_string(),
    };
    (inputs, trigger)
}

fn refusal_why(plan: &smelt_logical::maintenance::MaintenancePlan) -> String {
    assert!(
        plan.cells.is_empty(),
        "expected no admitted cell, got {:?}",
        plan.cells
    );
    assert!(
        matches!(&plan.refusals[..], [Refusal::NoAdmissibleTechnique { .. }]),
        "expected exactly one NoAdmissibleTechnique refusal, got {:?}",
        plan.refusals
    );
    let Refusal::NoAdmissibleTechnique { why, .. } = &plan.refusals[0] else {
        unreachable!()
    };
    why.clone()
}

#[test]
fn window_forward_and_snapshot_diff_verdicts_produce_distinct_refusal_reasons() {
    let (clocked_inputs, clocked_trigger) = inputs_with(mutable_source(true));
    let clocked_plan = derive_maintenance_plan(&clocked_inputs, &[clocked_trigger]);
    let clocked_why = refusal_why(&clocked_plan);

    let (unclocked_inputs, unclocked_trigger) = inputs_with(mutable_source(false));
    let unclocked_plan = derive_maintenance_plan(&unclocked_inputs, &[unclocked_trigger]);
    let unclocked_why = refusal_why(&unclocked_plan);

    // The two refusal reasons must be textually distinct — proving
    // `discovery`'s verdict (not just `facts.mutation`, which is identical
    // in both cases) selected between two code paths.
    assert_ne!(
        clocked_why, unclocked_why,
        "a clocked (WindowForward) and unclocked (SnapshotDiff) MutableSnapshot source \
         must produce distinguishable refusal reasons, proving input_delta_discovery's \
         verdict is consulted rather than merely computed and discarded"
    );

    // The WindowForward (clocked) path names the SC-2 blind spot: the change
    // would go entirely unseen by the next incremental run, not merely
    // un-undoable.
    assert!(
        clocked_why.contains("window-forward") && clocked_why.contains("unseen"),
        "clocked/WindowForward refusal should name the blind-spot distinctly, got: {clocked_why}"
    );
    assert!(
        !unclocked_why.contains("window-forward"),
        "unclocked/SnapshotDiff refusal should not claim window-forward, got: {unclocked_why}"
    );

    // Both must still cite the shared source-posture obligation (obligation
    // 2 fails either way — MutableSnapshot is never append-only).
    for why in [&clocked_why, &unclocked_why] {
        assert!(
            why.contains("append-only") || why.contains("source-posture"),
            "refusal should still cite the source-posture obligation, got: {why}"
        );
    }
}
