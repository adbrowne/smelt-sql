//! Built-in registry rows: table functions.
//!
//! Data only — every row is handed to the single `BuiltinRegistry` table
//! constructed in [`super`].

use super::super::{Emission, Position, Signature, SyntaxForm, TypeConstraint, TypeExpr};
use super::{tp, var};
use crate::DialectId;

pub(super) fn register(insert: &mut dyn FnMut(Signature)) {
    // ─── Table functions.
    insert(
        Signature::new(
            "EXPLODE",
            vec![tp("T", TypeConstraint::Any)],
            vec![var("T")],
            TypeExpr::Var("T".into()),
        )
        .with_syntax_form(SyntaxForm::TableFn)
        .with_emission(&[
            (DialectId::DuckDb, Position::Any, Emission::Rename("UNNEST")),
            (
                DialectId::BigQuery,
                Position::Any,
                Emission::Rename("UNNEST"),
            ),
        ]),
    );
    insert(
        Signature::new(
            "UNNEST",
            vec![tp("T", TypeConstraint::Any)],
            vec![var("T")],
            TypeExpr::Var("T".into()),
        )
        .with_syntax_form(SyntaxForm::TableFn)
        .with_emission(&[(
            DialectId::SparkSql,
            Position::Any,
            Emission::Rename("EXPLODE"),
        )]),
    );
}
