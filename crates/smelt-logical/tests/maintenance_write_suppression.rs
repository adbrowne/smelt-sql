//! Phase C4 (`docs/plans/20260715-composed-axes-conditional-maintenance.md`)
//! — admission of the change-suppressed column-scoped `MERGE` (T1).
//!
//! `smelt_logical::maintenance::choice::resolve_write_suppression` folds the
//! P3 per-column change-comparability verdict
//! (`analysis::walk::ColumnComparability`) and the P2 row-identity verdict
//! (`maintenance::RowIdentityVerdict`) into a fail-closed decision: a fully
//! comparable group over a proven key admits the conditional variant; one
//! Incomparable column, or no proven row identity, refuses it (falling back
//! to the always-safe unconditional matched-arm rewrite) — never a silent
//! downgrade with no reason attached.
//!
//! This is the production-facing conformance check, exercised only through
//! the crate's public API (`smelt_logical::maintenance::choice`,
//! `smelt_logical::analysis::walk`), mirroring `maintenance_choice.rs`'s
//! convention for this module.

use smelt_logical::analysis::walk::{ColumnComparability, Comparability};
use smelt_logical::maintenance::choice::{resolve_write_suppression, WriteSuppression};
use smelt_logical::maintenance::{RowIdentity, RowIdentityVerdict};

fn key_identity() -> RowIdentityVerdict {
    RowIdentityVerdict {
        identity: RowIdentity::Key(vec!["id".to_string()]),
        proven_mismatch: None,
    }
}

/// A fully comparable mutation-sensitive group over a proven key admits the
/// conditional variant, naming exactly the group's own columns as the
/// compared set — nothing added, nothing dropped.
#[test]
fn fully_comparable_group_admits_conditional_variant() {
    let group_columns = vec!["tier".to_string(), "plan_name".to_string()];
    let comparability = vec![
        ColumnComparability {
            output: "tier".to_string(),
            comparability: Comparability::Comparable,
        },
        ColumnComparability {
            output: "plan_name".to_string(),
            comparability: Comparability::Comparable,
        },
    ];

    let resolved = resolve_write_suppression(&group_columns, &comparability, &key_identity());

    assert_eq!(
        resolved,
        WriteSuppression::Suppressed {
            compared_columns: group_columns,
        },
        "a fully comparable group over a proven key must admit the conditional variant"
    );
}

/// A group whose mutation-sensitive set contains one Incomparable column
/// (e.g. a `NOW()`-tainted derived column) refuses the conditional variant,
/// naming that column in the reason, and falls back to the always-safe
/// unconditional variant — never silently dropping the offending column
/// from the write instead.
#[test]
fn one_incomparable_column_refuses_conditional_variant_named() {
    let group_columns = vec!["tier".to_string(), "last_seen_relative".to_string()];
    let comparability = vec![
        ColumnComparability {
            output: "tier".to_string(),
            comparability: Comparability::Comparable,
        },
        ColumnComparability {
            output: "last_seen_relative".to_string(),
            comparability: Comparability::Incomparable,
        },
    ];

    let resolved = resolve_write_suppression(&group_columns, &comparability, &key_identity());

    let why = match resolved {
        WriteSuppression::Unconditional { why } => why,
        other => panic!(
            "one Incomparable column must refuse the conditional variant fail-closed, got {other:?}"
        ),
    };
    assert!(
        why.contains("last_seen_relative"),
        "the refusal reason must name the offending column; got: {why}"
    );
}

/// No proven row identity (P2's `WholeRow` fallback) refuses the conditional
/// variant regardless of how comparable the group's own columns are — a
/// conditional write cannot safely address individual rows to compare
/// without a proven per-row identity.
#[test]
fn whole_row_identity_refuses_conditional_variant_regardless_of_comparability() {
    let group_columns = vec!["tier".to_string()];
    let comparability = vec![ColumnComparability {
        output: "tier".to_string(),
        comparability: Comparability::Comparable,
    }];
    let whole_row = RowIdentityVerdict {
        identity: RowIdentity::WholeRow,
        proven_mismatch: None,
    };

    let resolved = resolve_write_suppression(&group_columns, &comparability, &whole_row);

    assert!(
        matches!(resolved, WriteSuppression::Unconditional { .. }),
        "no proven row identity must refuse the conditional variant even for a fully \
         comparable group, got {resolved:?}"
    );
}
