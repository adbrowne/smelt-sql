use super::*;
use crate::maintenance::{RowPreservation, SkeletonSourceClosure};

#[test]
fn closed_with_nonempty_delta_restricts() {
    let closure = SkeletonSourceClosure::Closed {
        row_preservation: RowPreservation::JoinShape,
    };
    let delta = vec!["ev-1".to_string(), "ev-2".to_string()];
    let verdict = resolve_recompute_restriction(Some(&closure), Some(&delta));
    assert_eq!(
        verdict,
        RecomputeRestriction::Restricted {
            delta_keys: delta.clone()
        }
    );
}

#[test]
fn open_closure_never_restricts_even_with_a_delta() {
    let closure = SkeletonSourceClosure::Open {
        reason: "test".to_string(),
    };
    let delta = vec!["ev-1".to_string()];
    let verdict = resolve_recompute_restriction(Some(&closure), Some(&delta));
    assert!(matches!(verdict, RecomputeRestriction::Unrestricted { .. }));
}

#[test]
fn absent_closure_fact_never_restricts() {
    let delta = vec!["ev-1".to_string()];
    let verdict = resolve_recompute_restriction(None, Some(&delta));
    assert!(matches!(verdict, RecomputeRestriction::Unrestricted { .. }));
}

#[test]
fn closed_with_absent_delta_falls_back_unrestricted() {
    let closure = SkeletonSourceClosure::Closed {
        row_preservation: RowPreservation::JoinShape,
    };
    let verdict = resolve_recompute_restriction(Some(&closure), None);
    assert!(matches!(verdict, RecomputeRestriction::Unrestricted { .. }));
}

#[test]
fn closed_with_empty_delta_falls_back_unrestricted() {
    let closure = SkeletonSourceClosure::Closed {
        row_preservation: RowPreservation::JoinShape,
    };
    let empty: Vec<String> = vec![];
    let verdict = resolve_recompute_restriction(Some(&closure), Some(&empty));
    assert!(matches!(verdict, RecomputeRestriction::Unrestricted { .. }));
}
