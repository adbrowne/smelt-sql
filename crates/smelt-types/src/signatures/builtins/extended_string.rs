//! Built-in registry rows: extended string.
//!
//! Data only — every row is handed to the single `BuiltinRegistry` table
//! constructed in [`super`].

use super::super::{Emission, Position, Signature, TypeConstraint, TypeExpr};
use super::concrete;
use crate::{DataType, DialectId};

pub(super) fn register(insert: &mut dyn FnMut(Signature)) {
    // ─── Phase 50: Extended string scalars ───────────────────────────────────

    insert(Signature::new(
        "LTRIM",
        vec![],
        vec![concrete(DataType::Text)],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Text)),
    ));
    insert(Signature::new(
        "RTRIM",
        vec![],
        vec![concrete(DataType::Text)],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Text)),
    ));
    insert(Signature::new(
        "CHAR_LENGTH",
        vec![],
        vec![concrete(DataType::Text)],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::BigInt)),
    ));
    insert(Signature::new(
        "CHARACTER_LENGTH",
        vec![],
        vec![concrete(DataType::Text)],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::BigInt)),
    ));
    insert(Signature::new(
        "REPLACE",
        vec![],
        vec![
            concrete(DataType::Text),
            concrete(DataType::Text),
            concrete(DataType::Text),
        ],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Text)),
    ));
    insert(Signature::new(
        "LPAD",
        vec![],
        vec![
            concrete(DataType::Text),
            concrete(DataType::BigInt),
            concrete(DataType::Text),
        ],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Text)),
    ));
    insert(Signature::new(
        "RPAD",
        vec![],
        vec![
            concrete(DataType::Text),
            concrete(DataType::BigInt),
            concrete(DataType::Text),
        ],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Text)),
    ));
    insert(Signature::new(
        "REPEAT",
        vec![],
        vec![concrete(DataType::Text), concrete(DataType::BigInt)],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Text)),
    ));
    insert(Signature::new(
        "SUBSTR",
        vec![],
        vec![concrete(DataType::Text), concrete(DataType::BigInt)],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Text)),
    ));
    insert(Signature::new(
        "SPLIT_PART",
        vec![],
        vec![
            concrete(DataType::Text),
            concrete(DataType::Text),
            concrete(DataType::BigInt),
        ],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Text)),
    ));
    insert(
        Signature::new(
            "STRPOS",
            vec![],
            vec![concrete(DataType::Text), concrete(DataType::Text)],
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::BigInt)),
        )
        .with_emission(&[
            // Spark spells it `INSTR`, with the same (haystack, needle) argument order.
            // Verified live 2026-08-24: `instr('abc','b')` -> 2.
            (
                DialectId::SparkSql,
                Position::Any,
                Emission::Rename("INSTR"),
            ),
        ]),
    );
    insert(Signature::new(
        "LEFT",
        vec![],
        vec![concrete(DataType::Text), concrete(DataType::BigInt)],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Text)),
    ));
    insert(Signature::new(
        "RIGHT",
        vec![],
        vec![concrete(DataType::Text), concrete(DataType::BigInt)],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Text)),
    ));
}
