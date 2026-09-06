//! The append-only posture probe's two-verdict split (`docs/specs/
//! model_properties.md` §Constraints "Declared lateness is
//! orchestration-only"): a late append into a closed partition is an
//! observation, never a violation. `late_appends` is the pure classifier
//! owning the late-append half; `emit_append_only_posture_probe`'s narrowed
//! SQL predicate (exercised in `emit_statements.rs`/`probe_execution.rs`)
//! owns the violation half.

use smelt_logical::maintenance::emit::{
    late_appends, AppendOnlyBaselinePartition, CurrentPartitionState,
};

fn baseline(
    partition_value: &str,
    recorded_count: i64,
    check_fingerprint: bool,
) -> AppendOnlyBaselinePartition {
    AppendOnlyBaselinePartition {
        partition_value: partition_value.to_string(),
        recorded_count,
        recorded_fingerprint: "fp0".to_string(),
        check_fingerprint,
    }
}

fn current(partition_value: &str, current_count: i64) -> CurrentPartitionState {
    CurrentPartitionState {
        partition_value: partition_value.to_string(),
        current_count,
    }
}

#[test]
fn count_increase_in_closed_partition_is_a_late_append() {
    let baseline = vec![baseline("2026-01-01", 10, true)];
    let current = vec![current("2026-01-01", 13)];

    let appends = late_appends(&baseline, &current);
    assert_eq!(
        appends.len(),
        1,
        "expected exactly one late append: {appends:?}"
    );
    assert_eq!(appends[0].partition_value, "2026-01-01");
    assert_eq!(appends[0].added_rows, 3);
}

#[test]
fn count_decrease_is_a_violation() {
    let baseline = vec![baseline("2026-01-01", 10, true)];
    let current = vec![current("2026-01-01", 7)];

    let appends = late_appends(&baseline, &current);
    assert!(
        appends.is_empty(),
        "a count decrease must never be classified as a late append: {appends:?}"
    );
}

#[test]
fn changed_fingerprint_at_equal_count_is_a_violation() {
    let baseline = vec![baseline("2026-01-01", 10, true)];
    let current = vec![current("2026-01-01", 10)];

    let appends = late_appends(&baseline, &current);
    assert!(
        appends.is_empty(),
        "an unchanged count must never be classified as a late append, even if the fingerprint \
         differs (that is the violation predicate's concern): {appends:?}"
    );
}

#[test]
fn increase_in_the_open_frontier_partition_is_neither() {
    let baseline = vec![baseline("2026-01-03", 10, false)];
    let current = vec![current("2026-01-03", 15)];

    let appends = late_appends(&baseline, &current);
    assert!(
        appends.is_empty(),
        "the still-open frontier partition legitimately gains rows every run: {appends:?}"
    );
}

#[test]
fn partition_absent_from_baseline_is_neither() {
    let baseline = vec![baseline("2026-01-01", 10, true)];
    let current = vec![current("2026-01-01", 10), current("2026-01-02", 5)];

    let appends = late_appends(&baseline, &current);
    assert!(
        appends.is_empty(),
        "a brand-new partition with no recorded baseline is an ordinary append: {appends:?}"
    );
}
