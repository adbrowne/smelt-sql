//! Algebraic discriminants of an aggregate combiner.
//!
//! See `docs/specs/model_properties.md` §"Algebraic discriminants (the raw
//! facts, not the ladder)". These are the raw algebraic facts of a combiner —
//! is-monoid, needs-inverse, decomposable, value-vs-order-monotone —
//! generalising the `cumulative` monoid allowlist into one shared classifier.
//! This module states only the facts; the ordering of those facts into a
//! ladder and the maintainable/delegated cutoff live in `incremental_models.md`
//! and are **not** decided here.

use serde::Serialize;
use smelt_types::SqlFunction;

/// Whether a combiner's presented value moves monotonically, and how.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Monotone {
    /// The presented value itself only ever moves one way (`MIN`/`MAX`).
    Value,
    /// The presented value may switch, but only in step with a monotone
    /// ordering key (`MAX_BY`/`ARG_MAX`) — a semilattice fold.
    Order,
    /// No monotonicity fact is claimed.
    None,
}

/// The raw algebraic facts of a combiner. Not the maintenance ladder — see
/// `incremental_models.md` §"The algebraic maintenance ladder" for how these
/// facts order into maintainable/delegated tiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Discriminants {
    /// The combiner is a commutative monoid (closed, associative, has an
    /// identity) — a fold of contributions is well-defined without replay.
    pub is_monoid: bool,
    /// A contribution cannot be "un-seen": the monoid has no inverse (a
    /// group), so retraction requires replay rather than an inverse combine.
    /// Only meaningful when `is_monoid` is true.
    pub needs_inverse: bool,
    /// The combiner decomposes into a richer monoid element (e.g. `AVG` into
    /// `(sum, count)`) rather than being holistic (requiring the full
    /// multiset to answer, e.g. `MEDIAN`).
    pub decomposable: bool,
    /// Value- vs order-monotonicity of the presented value.
    pub monotone: Monotone,
}

impl Discriminants {
    /// The conservative, fail-closed verdict: not a monoid, not
    /// decomposable, no monotonicity claimed. Returned for holistic
    /// combiners (`MEDIAN`/`MODE`/exact `COUNT(DISTINCT)`) and for any
    /// aggregate this classifier does not (yet) recognise — absence of a
    /// classification is never optimistically treated as a monoid.
    const fn holistic_or_unknown() -> Self {
        Discriminants {
            is_monoid: false,
            needs_inverse: false,
            decomposable: false,
            monotone: Monotone::None,
        }
    }

    /// Whether these are exactly the fail-closed holistic-or-unknown facts:
    /// no monoid, not decomposable, no monotonicity claimed. Used by
    /// `decomposed_state::decompose_to_state` as its entry gate — narrower
    /// than `!decomposable` alone, so an order-monotone combiner
    /// (`ArgMax`/`ArgMin`, `decomposable == false` but `monotone ==
    /// Monotone::Order`) still reaches the state-shape match
    /// (`docs/specs/incremental_shapes.md` §"Decomposed state (rung 2) in
    /// keyed models").
    pub fn is_holistic_or_unknown(&self) -> bool {
        *self == Self::holistic_or_unknown()
    }
}

/// Classify the algebraic discriminants of an aggregate combiner.
///
/// `distinct` is whether the aggregate is applied with a `DISTINCT`
/// modifier (e.g. `COUNT(DISTINCT x)`) — an *exact* distinct aggregate is
/// holistic (the full multiset is needed to de-duplicate), unlike its
/// non-distinct counterpart.
///
/// This classifier is keyed on the resolved `SqlFunction` combiner alone; a
/// join-existence check (`EXISTS`) is not itself an aggregate combiner and is
/// not classified here — its value-monotonicity is a join-contribution fact
/// consumed by the fan-out/join-contribution proof, not this classifier.
///
/// Fail-closed: any combiner not explicitly classified below (a holistic
/// aggregate or an unrecognised/UDF one) returns the conservative
/// [`Discriminants::holistic_or_unknown`] verdict — never an optimistic
/// monoid classification.
pub fn combiner_discriminants(function: SqlFunction, distinct: bool) -> Discriminants {
    use SqlFunction::*;

    if distinct {
        // An exact DISTINCT aggregate needs the full multiset to
        // de-duplicate — holistic regardless of the underlying function.
        return Discriminants::holistic_or_unknown();
    }

    match function {
        // Commutative monoids that are also groups (invertible: a
        // contribution can be un-seen via an inverse combine). `BIT_XOR`
        // belongs here, not with the idempotent bit/bool lattice combiners
        // below: XOR is its own inverse (`x XOR d XOR d == x`), which makes
        // it a group and — critically — NOT idempotent. It is the additive
        // fold family (`docs/specs/incremental_shapes.md` §"The
        // column-family catalogue"), and every consumer that grades
        // re-foldability off these facts must treat it as such.
        Sum | Count | BitXor => Discriminants {
            is_monoid: true,
            needs_inverse: false,
            decomposable: false,
            monotone: Monotone::None,
        },

        // Commutative monoids that are NOT groups: a contribution cannot be
        // un-seen. MIN/MAX are additionally value-monotone.
        Min | Max => Discriminants {
            is_monoid: true,
            needs_inverse: true,
            decomposable: false,
            monotone: Monotone::Value,
        },
        BoolAnd | BoolOr | BitAnd | BitOr => Discriminants {
            is_monoid: true,
            needs_inverse: true,
            decomposable: false,
            monotone: Monotone::None,
        },

        // Decomposable into a richer monoid element (e.g. AVG -> (sum,
        // count); variance/stddev -> a Welford-style triple).
        Avg | Variance | Stddev | StddevPop | StddevSamp | VarPop | VarSamp
        | ApproxCountDistinct => Discriminants {
            is_monoid: false,
            needs_inverse: false,
            decomposable: true,
            monotone: Monotone::None,
        },

        // Order-monotone: the presented value may switch, but only in step
        // with a monotone ordering key (a semilattice fold) — the
        // order-monotone overwrite family (`MAX_BY`/`MIN_BY`,
        // `incremental_shapes.md` §"The column-family catalogue").
        ArgMax | ArgMin => Discriminants {
            is_monoid: false,
            needs_inverse: false,
            decomposable: false,
            monotone: Monotone::Order,
        },

        // Holistic (MEDIAN, MODE, ...) and anything not explicitly
        // classified above: fail-closed.
        _ => Discriminants::holistic_or_unknown(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `BIT_XOR` is a group (self-inverse), so it is graded with `SUM`/
    /// `COUNT`, never with the idempotent bit/bool lattice combiners — the
    /// additive fold family (`docs/specs/incremental_shapes.md` §"The
    /// column-family catalogue").
    #[test]
    fn sum_count_and_bit_xor_are_invertible_monoids() {
        for f in [SqlFunction::Sum, SqlFunction::Count, SqlFunction::BitXor] {
            let d = combiner_discriminants(f, false);
            assert!(d.is_monoid, "{f:?} should be a monoid");
            assert!(!d.needs_inverse, "{f:?} should be invertible (a group)");
            assert!(!d.decomposable);
            assert_eq!(d.monotone, Monotone::None);
        }
    }

    #[test]
    fn min_max_bool_bit_are_monoids_without_inverse() {
        for f in [
            SqlFunction::Min,
            SqlFunction::Max,
            SqlFunction::BoolAnd,
            SqlFunction::BoolOr,
            SqlFunction::BitAnd,
            SqlFunction::BitOr,
        ] {
            let d = combiner_discriminants(f, false);
            assert!(d.is_monoid, "{f:?} should be a monoid");
            assert!(
                d.needs_inverse,
                "{f:?} should need an inverse (not a group)"
            );
        }
    }

    #[test]
    fn min_max_are_value_monotone() {
        assert_eq!(
            combiner_discriminants(SqlFunction::Min, false).monotone,
            Monotone::Value
        );
        assert_eq!(
            combiner_discriminants(SqlFunction::Max, false).monotone,
            Monotone::Value
        );
    }

    #[test]
    fn arg_max_is_order_monotone() {
        assert_eq!(
            combiner_discriminants(SqlFunction::ArgMax, false).monotone,
            Monotone::Order
        );
    }

    #[test]
    fn arg_min_is_order_monotone() {
        assert_eq!(
            combiner_discriminants(SqlFunction::ArgMin, false).monotone,
            Monotone::Order
        );
    }

    #[test]
    fn avg_variance_approx_distinct_are_decomposable() {
        for f in [
            SqlFunction::Avg,
            SqlFunction::Variance,
            SqlFunction::Stddev,
            SqlFunction::ApproxCountDistinct,
        ] {
            let d = combiner_discriminants(f, false);
            assert!(d.decomposable, "{f:?} should be decomposable");
            assert!(!d.is_monoid);
        }
    }

    #[test]
    fn median_and_mode_are_holistic() {
        for f in [SqlFunction::Median, SqlFunction::Mode] {
            let d = combiner_discriminants(f, false);
            assert!(!d.is_monoid);
            assert!(!d.decomposable);
            assert_eq!(d.monotone, Monotone::None);
        }
    }

    #[test]
    fn exact_count_distinct_is_holistic() {
        let d = combiner_discriminants(SqlFunction::Count, true);
        assert!(
            !d.is_monoid,
            "exact COUNT(DISTINCT) is holistic, not a monoid"
        );
        assert!(!d.decomposable);
    }

    #[test]
    fn unrecognised_aggregate_fails_closed() {
        // An aggregate this classifier does not explicitly enumerate (e.g. a
        // holistic percentile function) must not be optimistically treated
        // as a monoid.
        let d = combiner_discriminants(SqlFunction::PercentileCont, false);
        assert!(!d.is_monoid);
        assert!(!d.decomposable);
        assert_eq!(d.monotone, Monotone::None);
    }
}
