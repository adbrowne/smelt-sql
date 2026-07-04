//! Functional-dependency (`key → determines`) declaration widening + guard.
//!
//! See `docs/specs/model_properties.md` §"Model-scoped declarations" row
//! **"Functional dependency (`key → column`)"** and §Constraints "Declared
//! escape hatches may only widen". A declared functional dependency asserts
//! that `determines` is a per-key constant, licensing once-write
//! `COALESCE`/1:1-after-dedup enrichment (no transform is emitted here — see
//! `docs/plans/20260704-model-updates-l3-declarations.md` DC2's scope note).
//!
//! [`functional_dependency_verdict`] composes the declaration with F6's
//! fan-out/cardinality proof (`analysis::join_shape::fan_out`): a
//! `determines` column sourced from a join F6 proves `OneToMany` (row-
//! multiplying) is refused regardless of the declaration — the declaration
//! can widen only the *undecidable* case (no traceable join at all, or one F6
//! cannot resolve), never substitute for F6's positive proof of variance.

use crate::analysis::join_shape::Cardinality;

/// The verdict of composing a functional-dependency declaration with F6's
/// fan-out proof for the `determines` column's origin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FunctionalDependencyVerdict {
    /// `determines` is proven (or, absent a proof, declared and not
    /// positively disproven) a per-key constant — once-write enrichment may
    /// be licensed.
    Constant,
    /// Not proven a per-key constant and not declared — the caller stays at
    /// the conservative rebuild/re-derive verdict. Not an error: this is the
    /// ordinary "no declaration, no proof" state.
    NotProven,
    /// Refused, naming why. Never returned optimistically — a positive
    /// fan-out proof always wins, declared or not, and a declaration can
    /// never be used to narrow (skip a dedup the proof requires).
    Refused(String),
}

impl FunctionalDependencyVerdict {
    pub fn is_constant(&self) -> bool {
        matches!(self, FunctionalDependencyVerdict::Constant)
    }
}

/// Decide whether a declared `key → determines` functional dependency may be
/// honoured for once-write enrichment.
///
/// `determines_fan_out` is F6's [`Cardinality`] verdict for the join (if any)
/// that sources the `determines` column — `None` when `determines` has no
/// traceable join origin at all (e.g. a plain pass-through column), which is
/// the undecidable case the declaration is meant to widen.
///
/// Fail-closed: a `OneToMany` fan-out into `determines` is refused
/// unconditionally, whether or not the FD is declared — F6's positive proof
/// of variance can never be overridden. Without a `OneToMany` proof, the
/// declaration widens an otherwise-`NotProven` verdict to `Constant`; absent
/// both the declaration and a `OneToOne` proof, the conservative `NotProven`
/// verdict stands.
pub fn functional_dependency_verdict(
    determines_fan_out: Option<Cardinality>,
    declared: bool,
) -> FunctionalDependencyVerdict {
    match determines_fan_out {
        Some(Cardinality::OneToMany) => FunctionalDependencyVerdict::Refused(
            "the determined column is sourced from a join proven to fan out (OneToMany per F6's \
             fan-out/cardinality proof); a declared functional dependency cannot substitute for \
             that proof of per-key variance"
                .to_string(),
        ),
        Some(Cardinality::OneToOne) => FunctionalDependencyVerdict::Constant,
        None if declared => FunctionalDependencyVerdict::Constant,
        None => FunctionalDependencyVerdict::NotProven,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undeclared_and_unproven_stays_not_proven() {
        let verdict = functional_dependency_verdict(None, false);
        assert_eq!(verdict, FunctionalDependencyVerdict::NotProven);
    }

    /// Widening test: a `determines` column with no traceable join origin at
    /// all (the SQL alone cannot decide per-key constancy) is admitted only
    /// once the FD is declared.
    #[test]
    fn declared_fd_widens_undecidable_origin_to_constant() {
        let verdict = functional_dependency_verdict(None, true);
        assert!(verdict.is_constant());
    }

    /// A join F6 proves `OneToOne` already establishes per-key constancy —
    /// no declaration needed.
    #[test]
    fn one_to_one_join_is_constant_without_declaration() {
        let verdict = functional_dependency_verdict(Some(Cardinality::OneToOne), false);
        assert!(verdict.is_constant());
    }

    /// Fail-closed reject test: a join F6 proves `OneToMany` into the
    /// determined column is refused even when the FD is declared — the
    /// declaration cannot substitute for, or narrow past, the positive
    /// disproof.
    #[test]
    fn declared_fd_is_refused_when_f6_proves_fan_out() {
        let verdict = functional_dependency_verdict(Some(Cardinality::OneToMany), true);
        assert!(!verdict.is_constant());
        assert!(matches!(
            &verdict,
            FunctionalDependencyVerdict::Refused(reason) if reason.contains("fan out")
        ));
    }

    /// Same fail-closed outcome without a declaration at all — the proof
    /// alone is already conclusive against per-key constancy.
    #[test]
    fn undeclared_fd_is_refused_when_f6_proves_fan_out() {
        let verdict = functional_dependency_verdict(Some(Cardinality::OneToMany), false);
        assert!(!verdict.is_constant());
        assert!(matches!(verdict, FunctionalDependencyVerdict::Refused(_)));
    }
}
