//! Built-in registry rows: window.
//!
//! Data only — every row is handed to the single `BuiltinRegistry` table
//! constructed in [`super`].

use super::super::{ExprKind, Signature, TypeConstraint, TypeExpr};
use super::{tp, var};
use crate::DataType;

pub(super) fn register(insert: &mut dyn FnMut(Signature)) {
    // ─── Window-only built-ins (Phase 14, §16 #24).
    //
    // These are dispatched only at call sites that carry an `OVER (…)`
    // clause; calling them without `OVER` is a runtime error in every
    // backend. Phase 14 records the kind only — argument-list checks for
    // these signatures land in a later phase. The placeholder `Any` arg
    // lists keep the existing `unify_call` happy without imposing a
    // false constraint.
    insert(
        Signature::new(
            "ROW_NUMBER",
            vec![],
            vec![],
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::BigInt)),
        )
        .with_kind(ExprKind::Window),
    );
    insert(
        Signature::new(
            "RANK",
            vec![],
            vec![],
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::BigInt)),
        )
        .with_kind(ExprKind::Window),
    );
    insert(
        Signature::new(
            "DENSE_RANK",
            vec![],
            vec![],
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::BigInt)),
        )
        .with_kind(ExprKind::Window),
    );
    insert(
        Signature::new(
            "LAG",
            vec![tp("T", TypeConstraint::Any)],
            vec![var("T")],
            TypeExpr::Var("T".into()),
        )
        .with_kind(ExprKind::Window),
    );
    insert(
        Signature::new(
            "LEAD",
            vec![tp("T", TypeConstraint::Any)],
            vec![var("T")],
            TypeExpr::Var("T".into()),
        )
        .with_kind(ExprKind::Window),
    );
}
