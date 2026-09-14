//! The emission coverage classifier — `docs/outcomes/20260913-trino-emission`
//! phases 2 and 6.
//!
//! Refines the three-way `passing`/`gap`/`unverified` vocabulary
//! `docs/specs/multi_backend.md` §"Cross-engine emission audit" states:
//! `passing` splits into [`Coverage::Stated`] (an explicit verdict exists) and
//! [`Coverage::Verified`] (the implicit `Native` default is backed by both a
//! live schema leg and a live value leg — see `BOTH_LEGS_LIVE` below).
//! [`Coverage::Unverified`] is the hole the implicit-`Native` default opens:
//! no explicit verdict, and no both-legs-live audit to have observed the
//! default correct. `no_dialect_has_unverified_pairs` is the standing gate:
//! every dialect this audit covers now has both legs live, so no pair may
//! classify `Unverified`. A dialect introduced before both its legs are
//! built may record its outstanding `Unverified` pairs in a shrink-only
//! census file while the gap is closed (`docs/specs/multi_backend.md`
//! §"Cross-engine emission audit") — Trino's own census
//! (`.claude/trino-emission-census.txt`) served that role through phases 3-5
//! and was deleted here once phase 6's value leg drove its count to zero.

use smelt_types::signatures::{Position, Signature};
use smelt_types::{BuiltinRegistry, DialectId};

use crate::ledger::{LedgerRow, Verdict};
use crate::report::applicable_positions;

/// Dialects with **both** the schema and value legs live — the narrower set
/// `classify` consults for `Coverage::Verified`, distinct from
/// [`crate::AUDITED_DIALECTS`] (which drives the offline totality gates: the
/// fixture gate and the print-for-every-dialect gate). A dialect can join
/// `AUDITED_DIALECTS` with only a schema leg — as Trino did in
/// `20260913-trino-emission` phase 5 — without every one of its unstated
/// pairs silently flipping to `Verified` on the strength of that leg alone.
/// Trino joined this set in phase 6, once its value leg landed.
const BOTH_LEGS_LIVE: &[DialectId] = &[
    DialectId::DuckDb,
    DialectId::SparkSql,
    DialectId::BigQuery,
    DialectId::Trino,
];

/// The four-way refinement of `passing`/`gap`/`unverified`
/// (`docs/specs/multi_backend.md` §"Cross-engine emission audit") a pair
/// classifies into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Coverage {
    /// An explicit `(dialect, position)` verdict exists in the registry.
    Stated,
    /// No explicit verdict, but the dialect has both legs live — the implicit
    /// `Native` default has been observed correct by a live schema leg AND a
    /// live value leg.
    Verified,
    /// A ledger row (`Gap` or `Divergent`) covers this pair — a live finding,
    /// checked ahead of the registry's own verdict because a pair the
    /// registry states `Native` for, that a sweep found does not actually
    /// work, is a finding, not a pass.
    Gap,
    /// No explicit verdict, no ledger row, and the dialect carries no
    /// both-legs-live audit to back the implicit `Native` default. The hole
    /// this classification exists to enumerate.
    Unverified,
}

/// Whether `ledger` carries a `Gap` or `Divergent` row for `(name, dialect,
/// position)`. A row with no `position` of its own (`None`) exempts every
/// position — the same convention every other ledger consumer in this crate
/// follows.
fn has_ledger_row(
    ledger: &[LedgerRow],
    name: &str,
    dialect: DialectId,
    position: Position,
) -> bool {
    ledger.iter().any(|row| {
        row.name == name
            && row.dialect == dialect
            && (row.position.is_none() || row.position == Some(position))
            && matches!(row.verdict, Verdict::Gap { .. } | Verdict::Divergent { .. })
    })
}

/// Classify one `(dialect, entry, position)` pair. `both_legs_live` is the
/// dialect set consulted for `Coverage::Verified` — an explicit parameter
/// (rather than always reading the real `BOTH_LEGS_LIVE`) so a test can prove
/// a dialect absent from it still classifies `Unverified` without needing a
/// real dialect that has not yet reached both legs.
pub fn classify(
    dialect: DialectId,
    sig: &Signature,
    position: Position,
    ledger: &[LedgerRow],
    both_legs_live: &[DialectId],
) -> Coverage {
    if has_ledger_row(ledger, &sig.name, dialect, position) {
        return Coverage::Gap;
    }
    if sig.stated_emission_at(dialect, position).is_some() {
        return Coverage::Stated;
    }
    if both_legs_live.contains(&dialect) {
        Coverage::Verified
    } else {
        Coverage::Unverified
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use smelt_types::signatures::SigParam;
    use smelt_types::{DataType, Emission, TypeConstraint, TypeExpr};

    fn test_signature(name: &str) -> Signature {
        Signature::new(
            name,
            vec![],
            vec![SigParam::Concrete(TypeConstraint::Concrete(
                DataType::Integer,
            ))],
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Integer)),
        )
    }

    #[test]
    fn census_classifies_a_stated_verdict_as_stated() {
        let sig = test_signature("TEST_STATED").with_emission(&[(
            DialectId::Trino,
            Position::Scalar,
            Emission::Rename("SOME_TRINO_NAME"),
        )]);
        assert_eq!(
            classify(
                DialectId::Trino,
                &sig,
                Position::Scalar,
                &[],
                BOTH_LEGS_LIVE
            ),
            Coverage::Stated
        );
    }

    #[test]
    fn census_classifies_an_audited_dialect_as_verified() {
        let sig = test_signature("TEST_UNSTATED");
        assert_eq!(
            classify(
                DialectId::DuckDb,
                &sig,
                Position::Scalar,
                &[],
                BOTH_LEGS_LIVE
            ),
            Coverage::Verified,
            "DuckDb has both legs live"
        );
    }

    #[test]
    fn census_classifies_a_ledger_row_as_gap() {
        let sig = test_signature("TEST_GAPPED");
        let ledger = [LedgerRow {
            name: "TEST_GAPPED",
            dialect: DialectId::Trino,
            position: None,
            arm: None,
            leg: crate::ledger::Leg::Schema,
            verdict: Verdict::Gap {
                issue: "#0",
                detail: "test row",
            },
        }];
        assert_eq!(
            classify(
                DialectId::Trino,
                &sig,
                Position::Scalar,
                &ledger,
                BOTH_LEGS_LIVE
            ),
            Coverage::Gap
        );
        let divergent = [LedgerRow {
            name: "TEST_GAPPED",
            dialect: DialectId::Trino,
            position: None,
            arm: None,
            leg: crate::ledger::Leg::Value,
            verdict: Verdict::Divergent { reason: "test" },
        }];
        assert_eq!(
            classify(
                DialectId::Trino,
                &sig,
                Position::Scalar,
                &divergent,
                BOTH_LEGS_LIVE
            ),
            Coverage::Gap
        );
    }

    /// The red-proof self-test kept alive now every real dialect is
    /// verified: `classify` takes the both-legs set as an explicit
    /// parameter, so a synthetic set can still prove an unstated pair
    /// classifies `Unverified` when its dialect is absent from it.
    #[test]
    fn classify_reports_unverified_without_both_legs() {
        let sig = test_signature("TEST_SCHEMA_ONLY_UNSTATED");
        const SYNTHETIC_BOTH_LEGS_LIVE: &[DialectId] = &[DialectId::DuckDb];
        assert_eq!(
            classify(
                DialectId::Trino,
                &sig,
                Position::Scalar,
                &[],
                SYNTHETIC_BOTH_LEGS_LIVE
            ),
            Coverage::Unverified,
            "a dialect absent from the both-legs-live set must not classify Verified"
        );
        assert_eq!(
            classify(
                DialectId::DuckDb,
                &sig,
                Position::Scalar,
                &[],
                SYNTHETIC_BOTH_LEGS_LIVE
            ),
            Coverage::Verified
        );
    }

    /// The standing criterion-1 gate: every registry entry, at every
    /// applicable position, for every audited dialect, classifies as
    /// something other than `Unverified`. This is what a newly-added
    /// built-in with no Trino (or any dialect's) verdict would fail —
    /// the audit's own coverage-totality and two-sided ledger gates carry
    /// the "cannot silently acquire a claim" property from here on, now
    /// that every dialect has both legs live and there is no census to
    /// fall back on.
    #[test]
    fn no_dialect_has_unverified_pairs() {
        let entries: Vec<&Signature> = BuiltinRegistry::names()
            .filter_map(BuiltinRegistry::resolve)
            .collect();
        let ledger = crate::ledger::dialect_divergences();
        let mut unverified = Vec::new();
        for dialect in crate::AUDITED_DIALECTS {
            for sig in &entries {
                for &position in applicable_positions(sig.kind) {
                    if classify(*dialect, sig, position, ledger, BOTH_LEGS_LIVE)
                        == Coverage::Unverified
                    {
                        unverified.push(format!("{} {} {:?}", dialect.slug(), sig.name, position));
                    }
                }
            }
        }
        assert!(
            unverified.is_empty(),
            "{} Unverified pair(s) — a newly-added built-in with no verdict for one of \
             AUDITED_DIALECTS. Give it an explicit Signature::emission verdict, or register a \
             ledger row if a live sweep found a gap:\n{}",
            unverified.len(),
            unverified.join("\n")
        );
    }
}
