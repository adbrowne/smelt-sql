//! Built-in registry rows: aggregates.
//!
//! Data only — every row is handed to the single `BuiltinRegistry` table
//! constructed in [`super`].

use super::super::{
    ExprKind, NullabilityPropagation, SigParam, Signature, TypeConstraint, TypeExpr,
};
use super::{tp, var};
use crate::{DataType, DialectId};

pub(super) fn register(insert: &mut dyn FnMut(Signature)) {
    // ─── Aggregates (treated like scalars here; aggregate-ness is Phase 3+
    //     of the broader roadmap and has no bearing on unification).
    //
    // Phase 12: SUM is the canonical example of §16 #9 engine-divergence.
    // `SUM(INTEGER)` returns `BigInt` in the smelt type system (the
    // "canonical" widening), but DuckDB natively returns `HUGEINT` — a
    // 128-bit type smelt models as `Decimal(38, 0)` in v1 (no dedicated
    // `Hugeint` variant until we have a concrete consumer). The
    // divergence flag feeds Step 7+'s CAST emitter; Phase 12 records
    // only.
    insert(
        Signature::new(
            "SUM",
            vec![tp("T", TypeConstraint::Numeric)],
            vec![var("T")],
            TypeExpr::Var("T".into()),
        )
        .with_canonical_return(DataType::BigInt)
        .with_engine_native(
            DialectId::DuckDb,
            DataType::Decimal {
                precision: 38,
                scale: 0,
            },
        )
        .with_kind(ExprKind::Agg),
    );
    insert(
        Signature::new(
            "AVG",
            vec![tp("T", TypeConstraint::Numeric)],
            vec![var("T")],
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Double)),
        )
        .with_kind(ExprKind::Agg),
    );
    insert(
        Signature::new(
            "MIN",
            vec![tp("T", TypeConstraint::Ordered)],
            vec![var("T")],
            TypeExpr::Var("T".into()),
        )
        .with_kind(ExprKind::Agg)
        .with_nullability(NullabilityPropagation::GroupedExtremal),
    );
    insert(
        Signature::new(
            "MAX",
            vec![tp("T", TypeConstraint::Ordered)],
            vec![var("T")],
            TypeExpr::Var("T".into()),
        )
        .with_kind(ExprKind::Agg)
        .with_nullability(NullabilityPropagation::GroupedExtremal),
    );
    insert(
        Signature::new(
            "COUNT",
            vec![],
            vec![SigParam::Concrete(TypeConstraint::Any)],
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::BigInt)),
        )
        .with_kind(ExprKind::Agg),
    );
}
