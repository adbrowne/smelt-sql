use super::*;
use crate::maintenance::RowIdentity;

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
fn fully_comparable_group_admits_suppression() {
    let group = vec!["tier".to_string(), "email".to_string()];
    let comparability = vec![comparable("tier"), comparable("email")];
    let identity = key_identity(&["id"]);

    let resolved = resolve_write_suppression(&group, &comparability, &identity);
    assert_eq!(
        resolved,
        WriteSuppression::Suppressed {
            compared_columns: group.clone()
        }
    );
}

#[test]
fn one_incomparable_column_refuses_named() {
    let group = vec!["tier".to_string(), "notes".to_string()];
    let comparability = vec![comparable("tier"), incomparable("notes")];
    let identity = key_identity(&["id"]);

    let resolved = resolve_write_suppression(&group, &comparability, &identity);
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
fn column_missing_from_comparability_vector_fails_closed() {
    // No proof at all for 'tier' — absence must not be trusted as a pass.
    let group = vec!["tier".to_string()];
    let comparability: Vec<ColumnComparability> = vec![];
    let identity = key_identity(&["id"]);

    let resolved = resolve_write_suppression(&group, &comparability, &identity);
    match resolved {
        WriteSuppression::Unconditional { why } => assert!(why.contains("tier")),
        other => panic!("expected Unconditional refusal, got {other:?}"),
    }
}

#[test]
fn whole_row_identity_refuses_regardless_of_comparability() {
    let group = vec!["tier".to_string()];
    let comparability = vec![comparable("tier")];
    let identity = RowIdentityVerdict {
        identity: RowIdentity::WholeRow,
        proven_mismatch: None,
    };

    let resolved = resolve_write_suppression(&group, &comparability, &identity);
    match resolved {
        WriteSuppression::Unconditional { why } => {
            assert!(why.contains("row identity") || why.contains("WholeRow"));
        }
        other => panic!("expected Unconditional refusal, got {other:?}"),
    }
}
