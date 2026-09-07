//! Built-in registry rows: extended temporal constructors (`MAKE_DATE`,
//! `MAKE_TIMESTAMP`).
//!
//! Data only — every row is handed to the single `BuiltinRegistry` table
//! constructed in [`super`].

use super::super::{Emission, Position, Signature, TypeConstraint, TypeExpr};
use super::concrete;
use crate::{DataType, DialectId};

pub(super) fn register(insert: &mut dyn FnMut(Signature)) {
    insert(
        Signature::new(
            "MAKE_DATE",
            vec![],
            vec![
                concrete(DataType::BigInt),
                concrete(DataType::BigInt),
                concrete(DataType::BigInt),
            ],
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Date)),
        )
        .with_emission(&[
            // GoogleSQL's three-argument `DATE(y, m, d)` is the same constructor.
            // Verified live 2026-08-24.
            (DialectId::BigQuery, Position::Any, Emission::Rename("DATE")),
        ]),
    );
    insert(
        Signature::new(
            "MAKE_TIMESTAMP",
            vec![],
            vec![
                concrete(DataType::BigInt),
                concrete(DataType::BigInt),
                concrete(DataType::BigInt),
                concrete(DataType::BigInt),
                concrete(DataType::BigInt),
                concrete(DataType::Double),
            ],
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Timestamp {
                with_timezone: false,
            })),
        )
        .with_emission(&[
            // GoogleSQL's `DATETIME(y, m, d, h, mi, s)`. Verified live 2026-08-24.
            (
                DialectId::BigQuery,
                Position::Any,
                Emission::Rename("DATETIME"),
            ),
        ]),
    );
}
