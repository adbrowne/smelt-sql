//! Declarative column-test proof — the resolution order's "consult derived
//! properties first" step (`docs/specs/data_tests.md` §Semantics
//! "Resolution order").
//!
//! Pure data + pure functions; no Salsa dependency (Salsa purity rule,
//! `docs/specs/architecture.md` §"Salsa purity rule (analysis)"). Given
//! already-derived facts about a model (a column's inferred nullability, the
//! model's known key column sets), decides whether a `not_null`/`unique`
//! declarative column test is **proven** (no scan needed) or must fall
//! through to a scan. `accepted_values`/`relationships` have no proof path
//! today — see `docs/specs/data_tests.md` §"Known Divergences".
//!
//! A proof may only remove a scan, never suppress a failure
//! (`docs/specs/data_tests.md` §"Proof is a scan-elimination, never a
//! failure-suppression"): every function here is fail-safe by construction —
//! an undecidable or absent input resolves to [`TestVerdict::NeedsScan`],
//! never to a claimed proof.

use std::collections::BTreeSet;

/// Verdict for a single declarative column test's proof step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestVerdict {
    /// The model's derived properties prove the test true at compile time;
    /// no scan is emitted.
    Proven,
    /// The proof step could not decide the test from derived properties; it
    /// must lower to a failing-rows scan.
    NeedsScan,
}

/// Resolve a `not_null` test's verdict from the tested column's inferred
/// nullability.
///
/// `is_non_nullable` should come from the model's inferred output schema
/// (`docs/specs/model_properties.md`'s nullability analysis) for the tested
/// column — `Some(true)` when the column is proven non-nullable, `Some(false)`
/// when it is known nullable, and `None` when the column's nullability is
/// undecidable (e.g. absent from the schema, or the schema doesn't track a
/// reliable source for it). Only a positive `Some(true)` proves the test;
/// every other input falls through to a scan.
pub fn resolve_not_null_verdict(is_non_nullable: Option<bool>) -> TestVerdict {
    match is_non_nullable {
        Some(true) => TestVerdict::Proven,
        _ => TestVerdict::NeedsScan,
    }
}

/// Resolve a `unique` test's verdict for a (possibly composite) column set.
///
/// Proven when `test_columns` is exactly one of the model's known key sets —
/// order-insensitive, set-equal comparison. `known_key_sets` is the model's
/// declared/proven grain key column sets (today: the declared `unique_key:`
/// fact; a future walk-proven grain/functional-dependency key set may extend
/// this list — `docs/specs/data_tests.md` §Semantics). An empty
/// `test_columns` or no matching key set falls through to a scan.
pub fn resolve_unique_verdict(
    test_columns: &[String],
    known_key_sets: &[Vec<String>],
) -> TestVerdict {
    if test_columns.is_empty() {
        return TestVerdict::NeedsScan;
    }
    let test_set: BTreeSet<&str> = test_columns.iter().map(String::as_str).collect();
    let proven = known_key_sets.iter().any(|key_set| {
        let key_set: BTreeSet<&str> = key_set.iter().map(String::as_str).collect();
        key_set == test_set
    });
    if proven {
        TestVerdict::Proven
    } else {
        TestVerdict::NeedsScan
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_null_proven_when_schema_says_non_nullable() {
        assert_eq!(resolve_not_null_verdict(Some(true)), TestVerdict::Proven);
    }

    #[test]
    fn not_null_needs_scan_when_nullable() {
        assert_eq!(
            resolve_not_null_verdict(Some(false)),
            TestVerdict::NeedsScan
        );
    }

    #[test]
    fn not_null_needs_scan_when_undecidable() {
        assert_eq!(resolve_not_null_verdict(None), TestVerdict::NeedsScan);
    }

    #[test]
    fn unique_proven_when_matches_declared_key() {
        let verdict = resolve_unique_verdict(&["id".to_string()], &[vec!["id".to_string()]]);
        assert_eq!(verdict, TestVerdict::Proven);
    }

    #[test]
    fn unique_proven_for_composite_key_regardless_of_order() {
        let verdict = resolve_unique_verdict(
            &["b".to_string(), "a".to_string()],
            &[vec!["a".to_string(), "b".to_string()]],
        );
        assert_eq!(verdict, TestVerdict::Proven);
    }

    #[test]
    fn unique_needs_scan_when_no_key_set_matches() {
        let verdict = resolve_unique_verdict(&["email".to_string()], &[vec!["id".to_string()]]);
        assert_eq!(verdict, TestVerdict::NeedsScan);
    }

    #[test]
    fn unique_needs_scan_when_no_known_key_sets() {
        let verdict = resolve_unique_verdict(&["id".to_string()], &[]);
        assert_eq!(verdict, TestVerdict::NeedsScan);
    }
}
