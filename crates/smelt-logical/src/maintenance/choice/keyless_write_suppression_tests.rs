use super::*;
use crate::maintenance::RowIdentity;

fn whole_row_identity() -> RowIdentityVerdict {
    RowIdentityVerdict {
        identity: RowIdentity::WholeRow,
        proven_mismatch: None,
    }
}

fn key_identity(cols: &[&str]) -> RowIdentityVerdict {
    RowIdentityVerdict {
        identity: RowIdentity::Key(cols.iter().map(|s| s.to_string()).collect()),
        proven_mismatch: None,
    }
}

fn comparable(col: &str) -> ColumnComparability {
    ColumnComparability {
        output: col.to_string(),
        comparability: Comparability::Comparable,
    }
}

fn incomparable(col: &str) -> ColumnComparability {
    ColumnComparability {
        output: col.to_string(),
        comparability: Comparability::Incomparable,
    }
}

#[test]
fn whole_row_identity_admits_keyless_staged_suppression_when_every_column_is_comparable() {
    let columns = vec!["event_id".to_string(), "payload".to_string()];
    let comparability = vec![comparable("event_id"), comparable("payload")];

    let resolved =
        resolve_keyless_staged_suppression(&columns, &comparability, &whole_row_identity());
    assert_eq!(
        resolved,
        WriteSuppression::Suppressed {
            compared_columns: columns
        }
    );
}

#[test]
fn whole_row_identity_with_an_incomparable_column_refuses_keyless_staged_suppression() {
    let columns = vec!["event_id".to_string(), "notes".to_string()];
    let comparability = vec![comparable("event_id"), incomparable("notes")];

    let resolved =
        resolve_keyless_staged_suppression(&columns, &comparability, &whole_row_identity());
    match resolved {
        WriteSuppression::Unconditional { why } => {
            assert!(
                why.contains("notes"),
                "refusal reason must name the incomparable column; got: {why}"
            );
        }
        other => panic!("expected Unconditional refusal, got {other:?}"),
    }
}

#[test]
fn key_identity_never_resolves_the_keyless_mechanism() {
    let columns = vec!["event_id".to_string()];
    let comparability = vec![comparable("event_id")];

    let resolved =
        resolve_keyless_staged_suppression(&columns, &comparability, &key_identity(&["id"]));
    match resolved {
        WriteSuppression::Unconditional { why } => {
            assert!(why.contains("keyed"), "got: {why}");
        }
        other => {
            panic!("expected Unconditional refusal (falls through to keyed), got {other:?}")
        }
    }
}
