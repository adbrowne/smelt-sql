//! Built-in registry rows: extended math.
//!
//! Data only — every row is handed to the single `BuiltinRegistry` table
//! constructed in [`super`].

use super::super::{Signature, TypeConstraint, TypeExpr};
use super::{concrete, tp, var};
use crate::DataType;

pub(super) fn register(insert: &mut dyn FnMut(Signature)) {
    // ─── Phase 50: Extended math scalars ─────────────────────────────────────

    insert(Signature::new(
        "EXP",
        vec![],
        vec![concrete(DataType::Double)],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Double)),
    ));
    insert(Signature::new(
        "LOG10",
        vec![],
        vec![concrete(DataType::Double)],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Double)),
    ));
    insert(Signature::new(
        "LOG2",
        vec![],
        vec![concrete(DataType::Double)],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Double)),
    ));
    insert(Signature::new(
        "MOD",
        vec![tp("T", TypeConstraint::Numeric)],
        vec![var("T"), var("T")],
        TypeExpr::Var("T".into()),
    ));
    insert(Signature::new(
        "SIGN",
        vec![],
        vec![concrete(DataType::Double)],
        // SmallInt to match the hand-written arm (DuckDB `sign` returns a
        // small signed integer, not a float).
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::SmallInt)),
    ));
    insert(Signature::new(
        "SIN",
        vec![],
        vec![concrete(DataType::Double)],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Double)),
    ));
    insert(Signature::new(
        "COS",
        vec![],
        vec![concrete(DataType::Double)],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Double)),
    ));
    insert(Signature::new(
        "TAN",
        vec![],
        vec![concrete(DataType::Double)],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Double)),
    ));
    insert(Signature::new(
        "ATAN",
        vec![],
        vec![concrete(DataType::Double)],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Double)),
    ));
    insert(Signature::new(
        "ATAN2",
        vec![],
        vec![concrete(DataType::Double), concrete(DataType::Double)],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Double)),
    ));
    insert(Signature::new(
        "SINH",
        vec![],
        vec![concrete(DataType::Double)],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Double)),
    ));
    insert(Signature::new(
        "COSH",
        vec![],
        vec![concrete(DataType::Double)],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Double)),
    ));
    insert(Signature::new(
        "TANH",
        vec![],
        vec![concrete(DataType::Double)],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Double)),
    ));
    insert(Signature::new(
        "PI",
        vec![],
        vec![],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Double)),
    ));
}
