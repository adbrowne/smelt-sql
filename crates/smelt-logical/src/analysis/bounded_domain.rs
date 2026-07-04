//! Bounded-domain / space-budget declaration widening + guard.
//!
//! See `docs/specs/model_properties.md` §"Model-scoped declarations" row
//! **"Bounded-domain / space budget"** and §Constraints "Declared escape
//! hatches may only widen". A declared bounded domain asserts that a
//! column's active domain is bounded by an explicit cap, licensing an exact
//! holistic aggregate (`MEDIAN`/`MODE`/exact `COUNT(DISTINCT)`) for multiset
//! maintenance via an explicit per-key multiset (no transform is emitted
//! here — see `docs/plans/20260704-model-updates-l3-declarations.md`'s
//! bounded-domain phase scope note).
//!
//! [`bounded_domain_verdict`] composes the declaration with F4's algebraic
//! discriminants (`analysis::discriminants::combiner_discriminants`): the
//! declaration widens only the *holistic* combiner case (`!is_monoid &&
//! !decomposable`) — a monoid (`SUM`) or decomposable (`AVG`) combiner needs
//! no multiset licence at all, so a declaration attached to one is refused
//! (it cannot license what needs no licence, and cannot narrow).

use crate::analysis::discriminants::Discriminants;

/// The verdict of composing a bounded-domain declaration with F4's algebraic
/// discriminants for the aggregate combiner it is attached to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoundedDomainVerdict {
    /// The combiner is holistic and a declared cap licenses an exact
    /// per-key multiset of at most `max_cardinality` distinct values.
    Licensed { max_cardinality: u64 },
    /// The combiner is holistic but no declaration is present — the caller
    /// stays at the conservative refused verdict (full refresh / an
    /// approximate substitute). Not an error: this is the ordinary
    /// "no declaration" state.
    NotLicensed,
    /// Refused, naming why. Returned when a declaration is present but the
    /// combiner is *not* holistic (a monoid or decomposable combiner needs
    /// no multiset licence) — the declaration cannot be used to license a
    /// construct that was never gated on it, and cannot narrow.
    Refused(String),
}

impl BoundedDomainVerdict {
    pub fn is_licensed(&self) -> bool {
        matches!(self, BoundedDomainVerdict::Licensed { .. })
    }
}

/// Decide whether a declared bounded-domain cap may license an exact
/// holistic aggregate's multiset maintenance.
///
/// `discriminants` is F4's [`Discriminants`] verdict for the aggregate
/// combiner in question. `declared` is the parsed cap (`Some(max_cardinality)`)
/// from the model's `bounded_domain:` declaration, or `None` when absent.
/// Callers must have already run `validate_bounded_domains`
/// (`smelt-core::metadata`), which fails loud on a present-but-malformed
/// declaration (zero/absent cap) before this pure function ever sees it —
/// this function only composes an already-valid `Some(cap)` with the
/// combiner's holistic classification.
///
/// Fail-closed: a declaration attached to a non-holistic combiner
/// (`is_monoid` or `decomposable`) is refused unconditionally — the
/// declaration can widen only the holistic case, never substitute for (or
/// be misapplied to) a combiner that needs no multiset licence at all.
pub fn bounded_domain_verdict(
    discriminants: Discriminants,
    declared: Option<u64>,
) -> BoundedDomainVerdict {
    let holistic = !discriminants.is_monoid && !discriminants.decomposable;

    match (holistic, declared) {
        (true, Some(max_cardinality)) => BoundedDomainVerdict::Licensed { max_cardinality },
        (true, None) => BoundedDomainVerdict::NotLicensed,
        (false, Some(_)) => BoundedDomainVerdict::Refused(
            "a bounded_domain declaration was applied to a non-holistic aggregate combiner \
             (a monoid or decomposable combiner needs no multiset licence at all — the \
             declaration cannot license what needs no licence, and cannot narrow)"
                .to_string(),
        ),
        (false, None) => BoundedDomainVerdict::NotLicensed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::discriminants::{combiner_discriminants, Discriminants};
    use smelt_types::SqlFunction;

    fn holistic() -> Discriminants {
        combiner_discriminants(SqlFunction::Median, false)
    }

    fn monoid() -> Discriminants {
        combiner_discriminants(SqlFunction::Sum, false)
    }

    fn decomposable() -> Discriminants {
        combiner_discriminants(SqlFunction::Avg, false)
    }

    /// Widening test: a holistic combiner (`MEDIAN`) with a declared cap is
    /// admitted for multiset maintenance.
    #[test]
    fn holistic_with_declared_cap_is_licensed() {
        let verdict = bounded_domain_verdict(holistic(), Some(10_000));
        assert_eq!(
            verdict,
            BoundedDomainVerdict::Licensed {
                max_cardinality: 10_000
            }
        );
        assert!(verdict.is_licensed());
    }

    /// Widening only occurs when the cap is present: a holistic combiner
    /// without a declaration stays refused (not maintainable) by default.
    #[test]
    fn holistic_without_declaration_is_not_licensed() {
        let verdict = bounded_domain_verdict(holistic(), None);
        assert_eq!(verdict, BoundedDomainVerdict::NotLicensed);
        assert!(!verdict.is_licensed());
    }

    /// Fail-closed reject test: a monoid combiner (`SUM`) needs no multiset
    /// licence — a declaration attached to it is refused, never silently
    /// honoured.
    #[test]
    fn monoid_with_declared_cap_is_refused() {
        let verdict = bounded_domain_verdict(monoid(), Some(10_000));
        assert!(!verdict.is_licensed());
        assert!(matches!(
            &verdict,
            BoundedDomainVerdict::Refused(reason) if reason.contains("non-holistic")
        ));
    }

    /// Same fail-closed outcome for a decomposable combiner (`AVG`) — it
    /// decomposes into `(sum, count)` and needs no multiset either.
    #[test]
    fn decomposable_with_declared_cap_is_refused() {
        let verdict = bounded_domain_verdict(decomposable(), Some(10_000));
        assert!(!verdict.is_licensed());
        assert!(matches!(verdict, BoundedDomainVerdict::Refused(_)));
    }

    /// A monoid combiner with no declaration is simply not-licensed — not an
    /// error (there was never anything to license).
    #[test]
    fn monoid_without_declaration_is_not_licensed() {
        let verdict = bounded_domain_verdict(monoid(), None);
        assert_eq!(verdict, BoundedDomainVerdict::NotLicensed);
    }
}
