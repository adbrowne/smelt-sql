//! Built-in registry rows: temporal.
//!
//! Data only — every row is handed to the single `BuiltinRegistry` table
//! constructed in [`super`].

use super::super::{Emission, Position, Signature, TypeConstraint, TypeExpr};
use super::concrete;
use crate::{DataType, DialectId};

pub(super) fn register(insert: &mut dyn FnMut(Signature)) {
    // ─── Date / time basics.
    insert(Signature::new(
        "DATE_TRUNC",
        vec![],
        vec![
            concrete(DataType::Text),
            concrete(DataType::Timestamp {
                with_timezone: false,
            }),
        ],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Timestamp {
            with_timezone: false,
        })),
    ));
    insert(Signature::new(
        "EXTRACT",
        vec![],
        vec![
            concrete(DataType::Text),
            concrete(DataType::Timestamp {
                with_timezone: false,
            }),
        ],
        // BigInt to match the hand-written arm.
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::BigInt)),
    ));
    insert(Signature::new(
        "DATE",
        vec![],
        vec![concrete(DataType::Timestamp {
            with_timezone: false,
        })],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Date)),
    ));
    insert(
        Signature::new(
            "NOW",
            vec![],
            vec![],
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Timestamp {
                with_timezone: true,
            })),
        )
        .with_emission(&[
            // GoogleSQL has no `now()`; verified live 2026-08-24.
            (
                DialectId::BigQuery,
                Position::Any,
                Emission::Rename("CURRENT_TIMESTAMP"),
            ),
        ]),
    );
    insert(Signature::new(
        "CURRENT_DATE",
        vec![],
        vec![],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Date)),
    ));
    insert(Signature::new(
        "CURRENT_TIMESTAMP",
        vec![],
        vec![],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Timestamp {
            with_timezone: true,
        })),
    ));
}
