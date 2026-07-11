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
use crate::analysis::walk::PropertyVector;

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

/// Compose a declared `key → determines` functional dependency with the
/// model's whole-model [`PropertyVector`] (the walk-derived grain, FD, and
/// structural facts). This is the key-aware verdict that reads the declared
/// `key` columns — closing the "parsed but never read" gap
/// (`20260707-property-per-key-constancy.md` §7 gap 5a) and the FD-over-union
/// widening bug (§3.8, catalog cell SC-6):
///
/// - the declaration is **consulted against the model's columns** — a `key`
///   (or `determines`) column the model does not project cannot be widened
///   (`NotProven`);
/// - a proven grain key that is a subset of the declared `key` already
///   establishes the FD by construction (`Constant`, no declaration needed —
///   the `GROUP BY`/`DISTINCT`/discriminated-union factory);
/// - a `determines` column crossing a proven fan-out join, or an
///   undiscriminated set operation whose branches are not proven key-disjoint,
///   is a **structural disproof** the declaration cannot override (`Refused`);
/// - only a genuinely undecidable single-branch origin is widened by the
///   declaration (`Constant` when declared, else `NotProven`).
pub fn functional_dependency_verdict_over_vector(
    key: &[String],
    determines: &str,
    vector: &PropertyVector,
    declared: bool,
) -> FunctionalDependencyVerdict {
    // Consult the declaration against what the model actually produces. A
    // determined or key column absent from the output relation is a
    // declaration the analysis cannot honour — never widened.
    if !vector
        .columns
        .iter()
        .any(|c| c.eq_ignore_ascii_case(determines))
    {
        return FunctionalDependencyVerdict::NotProven;
    }
    if !key
        .iter()
        .all(|k| vector.columns.iter().any(|c| c.eq_ignore_ascii_case(k)))
    {
        return FunctionalDependencyVerdict::NotProven;
    }

    // Query-derived: a proven key that is a subset of the declared key
    // determines every output column — the FD holds by construction.
    let declared_key: std::collections::BTreeSet<String> =
        key.iter().map(|c| c.to_ascii_lowercase()).collect();
    if vector.grain.has_subset_key(&declared_key) {
        return FunctionalDependencyVerdict::Constant;
    }

    // Structural disproofs the declaration cannot override.
    if vector.has_fan_out_join {
        return FunctionalDependencyVerdict::Refused(
            "the determined column is sourced from a join proven to fan out (OneToMany); a \
             declared functional dependency cannot substitute for that proof of per-key variance"
                .to_string(),
        );
    }
    if vector.has_set_op_barrier {
        return FunctionalDependencyVerdict::Refused(
            "the determined column crosses a UNION ALL / set operation whose branches are not \
             proven key-disjoint (no literal discriminator covering the declared key); an FD \
             holding in each branch does not hold in the union, so a declared functional \
             dependency cannot be assumed to survive it"
                .to_string(),
        );
    }

    // Genuinely undecidable single-branch origin — the case a declaration
    // widens.
    if declared {
        FunctionalDependencyVerdict::Constant
    } else {
        FunctionalDependencyVerdict::NotProven
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::join_shape::JoinContext;
    use crate::analysis::walk::model_property_vector;

    /// The declared `key` field is consulted (previously parsed but never
    /// read): a declaration whose key column the model does not produce is not
    /// widened, whereas a plausible single-branch declaration still is.
    #[test]
    fn fd_key_field_is_consulted() {
        let vector = model_property_vector(
            "SELECT customer_id, region FROM orders",
            &JoinContext::new(),
        )
        .expect("parses");

        // A declared key column absent from the model must not widen.
        let bogus = functional_dependency_verdict_over_vector(
            &["not_a_real_column".to_string()],
            "region",
            &vector,
            true,
        );
        assert!(
            !bogus.is_constant(),
            "a declared key absent from the model must not widen to Constant; got {bogus:?}"
        );

        // A plausible single-branch declaration (key projected, undecidable
        // origin) still widens.
        let plausible = functional_dependency_verdict_over_vector(
            &["customer_id".to_string()],
            "region",
            &vector,
            true,
        );
        assert!(
            plausible.is_constant(),
            "a plausible single-branch declaration still widens; got {plausible:?}"
        );
    }

    /// SC-6 at the unit layer: a declared FD over a bare `UNION ALL` body is a
    /// structural disproof, not a widenable undecidable origin.
    #[test]
    fn declared_fd_over_union_all_does_not_widen() {
        let vector = model_property_vector(
            "SELECT customer_id, region FROM crm_a \
             UNION ALL \
             SELECT customer_id, region FROM crm_b",
            &JoinContext::new(),
        )
        .expect("parses");

        let verdict = functional_dependency_verdict_over_vector(
            &["customer_id".to_string()],
            "region",
            &vector,
            true,
        );
        assert!(
            !verdict.is_constant(),
            "a declared FD over a bare UNION ALL must not widen to Constant; got {verdict:?}"
        );
    }

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
