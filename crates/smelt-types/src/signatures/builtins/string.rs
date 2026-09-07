//! Built-in registry rows: string.
//!
//! Data only — every row is handed to the single `BuiltinRegistry` table
//! constructed in [`super`].

use super::super::{Signature, TypeConstraint, TypeExpr};
use super::{concrete, variadic};
use crate::DataType;

pub(super) fn register(insert: &mut dyn FnMut(Signature)) {
    // ─── Text / string scalars.
    insert(Signature::new(
        "LOWER",
        vec![],
        vec![concrete(DataType::Text)],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Text)),
    ));
    insert(Signature::new(
        "UPPER",
        vec![],
        vec![concrete(DataType::Text)],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Text)),
    ));
    insert(Signature::new(
        "MD5",
        vec![],
        vec![concrete(DataType::Text)],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Text)),
    ));
    insert(Signature::new(
        "LENGTH",
        vec![],
        vec![concrete(DataType::Text)],
        // BigInt (not Integer) to match the hand-written arm — DuckDB returns
        // a 64-bit length and the migrated typing path must reproduce it.
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::BigInt)),
    ));
    insert(Signature::new(
        "SUBSTRING",
        vec![],
        vec![
            concrete(DataType::Text),
            concrete(DataType::Integer),
            concrete(DataType::Integer),
        ],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Text)),
    ));
    insert(Signature::new(
        "TRIM",
        vec![],
        vec![concrete(DataType::Text)],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Text)),
    ));
    insert(Signature::new(
        "CONCAT",
        vec![],
        vec![variadic(concrete(DataType::Text))],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Text)),
    ));
}
