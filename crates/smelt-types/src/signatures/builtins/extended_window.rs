//! Built-in registry rows: extended window.
//!
//! Data only — every row is handed to the single `BuiltinRegistry` table
//! constructed in [`super`].

use super::super::{ExprKind, Signature, TypeConstraint, TypeExpr};
use super::{concrete, tp, var};
use crate::DataType;

pub(super) fn register(insert: &mut dyn FnMut(Signature)) {
    // ─── Phase 50: Extended window functions ─────────────────────────────────

    insert(
        Signature::new(
            "NTILE",
            vec![],
            vec![concrete(DataType::BigInt)],
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::BigInt)),
        )
        .with_kind(ExprKind::Window),
    );
    insert(
        Signature::new(
            "FIRST_VALUE",
            vec![tp("T", TypeConstraint::Any)],
            vec![var("T")],
            TypeExpr::Var("T".into()),
        )
        .with_kind(ExprKind::Window),
    );
    insert(
        Signature::new(
            "LAST_VALUE",
            vec![tp("T", TypeConstraint::Any)],
            vec![var("T")],
            TypeExpr::Var("T".into()),
        )
        .with_kind(ExprKind::Window),
    );
    insert(
        Signature::new(
            "NTH_VALUE",
            vec![tp("T", TypeConstraint::Any)],
            vec![var("T"), concrete(DataType::BigInt)],
            TypeExpr::Var("T".into()),
        )
        .with_kind(ExprKind::Window),
    );
    insert(
        Signature::new(
            "CUME_DIST",
            vec![],
            vec![],
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Double)),
        )
        .with_kind(ExprKind::Window),
    );
    insert(
        Signature::new(
            "PERCENT_RANK",
            vec![],
            vec![],
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Double)),
        )
        .with_kind(ExprKind::Window),
    );
}
