//! S1: a declared top-level `unique_key:` (`docs/specs/models.md` §"Refresh
//! axis") must agree with the keyed classifier's GROUP-BY-derived key.
//!
//! Spec oracle: `docs/specs/models.md` §"Constraint violations" — "For
//! aggregated key bodies: `unique_key` ≠ the `GROUP BY` column set → hard
//! error (checked restatement)".
//!
//! `declared_unique_key_matches` (`smelt_logical::rules::cumulative`) is the
//! leaf comparison the maintenance-plan derivation
//! (`smelt-db::queries::maintenance::derive_model_maintenance_plan`) calls to
//! turn a disagreement into a refused plan; this test exercises the
//! comparison itself against the classifier's real derivation.

use smelt_logical::rules::cumulative::{declared_unique_key_matches, group_by_unique_key};

const SQL: &str =
    "SELECT device_id, user_id, COUNT(*) AS n FROM smelt.events GROUP BY device_id, user_id";

/// The classifier derives `[device_id, user_id]` from the GROUP BY.
#[test]
fn group_by_unique_key_derives_the_group_by_columns() {
    assert_eq!(
        group_by_unique_key(SQL),
        vec!["device_id".to_string(), "user_id".to_string()]
    );
}

/// A declared `unique_key` naming exactly the GROUP BY columns (in any
/// order) agrees with the derived key.
#[test]
fn declared_unique_key_agreeing_with_group_by_passes() {
    declared_unique_key_matches(&["device_id".to_string(), "user_id".to_string()], SQL)
        .expect("declared unique_key matching the GROUP BY columns must agree");

    // Order-independent: declaring the same columns in the opposite order is
    // still agreement, not a mismatch.
    declared_unique_key_matches(&["user_id".to_string(), "device_id".to_string()], SQL)
        .expect("declared unique_key in a different order must still agree");
}

/// A declared `unique_key` that disagrees with the GROUP BY-derived key
/// errors, naming both the declared and derived lists.
#[test]
fn declared_unique_key_disagreeing_with_group_by_errors_naming_both_lists() {
    let declared = vec!["device_id".to_string()];
    let err = declared_unique_key_matches(&declared, SQL)
        .expect_err("a declared unique_key narrower than the GROUP BY columns must disagree");
    let (declared_out, derived_out) = err;
    assert_eq!(declared_out, declared);
    assert_eq!(
        derived_out,
        vec!["device_id".to_string(), "user_id".to_string()]
    );
}

/// A declared `unique_key` naming a column absent from the GROUP BY entirely
/// also disagrees.
#[test]
fn declared_unique_key_naming_a_foreign_column_errors() {
    let declared = vec!["device_id".to_string(), "session_id".to_string()];
    let err = declared_unique_key_matches(&declared, SQL)
        .expect_err("a declared unique_key naming a non-GROUP-BY column must disagree");
    assert_eq!(err.0, declared);
    assert_eq!(err.1, vec!["device_id".to_string(), "user_id".to_string()]);
}
