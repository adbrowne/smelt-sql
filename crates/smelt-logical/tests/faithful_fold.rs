//! TDD tests for the faithful-fold conditions proof
//! (`model_properties.md` §"Faithful-fold conditions"): a typed
//! [`FaithfulFold`] verdict composing two INDEPENDENT conditions —
//! (1) the delta stream partitions the input (declared source posture,
//! never derived from the model's SQL) and (2) the combiner's fold over any
//! sub-multiset of a partition equals the batch aggregate (the algebraic
//! discriminants). Either failing alone refuses; each is reported
//! independently, never collapsed into one boolean.

use smelt_logical::analysis::faithful_fold::{faithful_fold, ConditionVerdict, FaithfulFold};
use smelt_logical::analysis::input_delta::{InputDeltaKind, MutationProfile};
use smelt_types::SqlFunction;

/// A monoid combiner (`SUM`) over an append-only posture satisfies both
/// conditions — the verdict is `Holds`, with nothing partially reported.
#[test]
fn monoid_over_append_only_passes_both_conditions() {
    let verdict = faithful_fold(
        SqlFunction::Sum,
        false,
        &MutationProfile::AppendOnly,
        InputDeltaKind::WindowForward,
    );
    assert!(
        matches!(verdict, FaithfulFold::Holds),
        "SUM over an append-only source should hold both conditions, got {verdict:?}"
    );
}

/// The spec's canonical example ("a retraction-carrying feed fails (1)
/// even when the combiner is invertible"): a retraction-carrying posture
/// folded through `MIN` (a monoid, so condition 2 passes) fails condition (1)
/// ONLY — and the verdict names condition (1) as the failing one, with
/// condition (2) independently reported as holding.
#[test]
fn retraction_carrying_feed_into_noninvertible_fails_condition_one_only() {
    let verdict = faithful_fold(
        SqlFunction::Min,
        false,
        &MutationProfile::Mutable,
        InputDeltaKind::SnapshotDiff,
    );
    let FaithfulFold::Fails {
        partitioned_input,
        submultiset_fold,
    } = verdict
    else {
        panic!("retractions into MIN must fail the faithful fold, got Holds");
    };
    assert!(
        matches!(partitioned_input, ConditionVerdict::Fails { .. }),
        "condition (1) — partitioned input — must be the failing condition, \
         got {partitioned_input:?}"
    );
    assert!(
        matches!(submultiset_fold, ConditionVerdict::Holds),
        "condition (2) — sub-multiset fold — must independently report as \
         holding (MIN is a monoid), got {submultiset_fold:?}"
    );
}

/// A holistic combiner (`MEDIAN`) fails condition (2) regardless of how
/// clean the source posture is — an append-only source does not rescue it.
/// Condition (1) is independently reported as holding.
#[test]
fn holistic_combiner_fails_condition_two_regardless_of_posture() {
    let verdict = faithful_fold(
        SqlFunction::Median,
        false,
        &MutationProfile::AppendOnly,
        InputDeltaKind::WindowForward,
    );
    let FaithfulFold::Fails {
        partitioned_input,
        submultiset_fold,
    } = verdict
    else {
        panic!("MEDIAN must fail the faithful fold even over append-only, got Holds");
    };
    assert!(
        matches!(partitioned_input, ConditionVerdict::Holds),
        "condition (1) must independently report as holding over an \
         append-only posture, got {partitioned_input:?}"
    );
    assert!(
        matches!(submultiset_fold, ConditionVerdict::Fails { .. }),
        "condition (2) must be the failing condition for a holistic \
         combiner, got {submultiset_fold:?}"
    );
}

/// The rendered refusal reason for each failing condition preserves the
/// vocabulary the existing admission tests assert on
/// (`maintenance_plan_admission.rs`, `input_delta_consumed.rs`):
/// condition (1) failures cite the append-only/retraction posture and name
/// the two conditions as independent; condition (2) failures name the
/// combiner as holistic / not a monoid and the recompute family as what
/// remains.
#[test]
fn verdict_reasons_preserve_admission_refusal_vocabulary() {
    // Condition (1) failure — retracting posture, monoid combiner.
    let posture_fail = faithful_fold(
        SqlFunction::Sum,
        false,
        &MutationProfile::Mutable,
        InputDeltaKind::SnapshotDiff,
    );
    let FaithfulFold::Fails {
        partitioned_input: ConditionVerdict::Fails { reason },
        ..
    } = posture_fail
    else {
        panic!("expected a condition-(1) failure, got {posture_fail:?}");
    };
    assert!(
        reason.contains("append-only") || reason.contains("retract"),
        "condition-(1) reason should cite the source posture, got: {reason}"
    );
    assert!(
        reason.contains("independent"),
        "condition-(1) reason should name the two conditions as independent, got: {reason}"
    );

    // Condition (1) failure, clocked variant — the WindowForward blind spot
    // must be named distinctly (the change goes unseen, not merely
    // un-undoable), matching `input_delta_consumed.rs`'s distinctness gate.
    let clocked_fail = faithful_fold(
        SqlFunction::Sum,
        false,
        &MutationProfile::Mutable,
        InputDeltaKind::WindowForward,
    );
    let FaithfulFold::Fails {
        partitioned_input: ConditionVerdict::Fails { reason: clocked },
        ..
    } = clocked_fail
    else {
        panic!("expected a condition-(1) failure, got {clocked_fail:?}");
    };
    assert!(
        clocked.contains("window-forward") && clocked.contains("unseen"),
        "clocked condition-(1) reason should name the window-forward blind spot, got: {clocked}"
    );
    assert_ne!(
        clocked, reason,
        "clocked and unclocked condition-(1) reasons must be distinct"
    );
    assert!(
        !reason.contains("window-forward"),
        "unclocked condition-(1) reason should not claim window-forward, got: {reason}"
    );

    // Condition (2) failure — holistic combiner.
    let algebra_fail = faithful_fold(
        SqlFunction::Median,
        false,
        &MutationProfile::AppendOnly,
        InputDeltaKind::WindowForward,
    );
    let FaithfulFold::Fails {
        submultiset_fold: ConditionVerdict::Fails { reason },
        ..
    } = algebra_fail
    else {
        panic!("expected a condition-(2) failure, got {algebra_fail:?}");
    };
    assert!(
        reason.contains("holistic") || reason.contains("not a monoid"),
        "condition-(2) reason should name the combiner class, got: {reason}"
    );
    assert!(
        reason.to_lowercase().contains("recompute"),
        "condition-(2) reason should name the recompute family as what remains, got: {reason}"
    );
}
