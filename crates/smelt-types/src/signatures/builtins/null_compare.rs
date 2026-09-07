//! Built-in registry rows: null compare.
//!
//! Data only — every row is handed to the single `BuiltinRegistry` table
//! constructed in [`super`].

use super::super::{Signature, TypeConstraint, TypeExpr};
use super::{tp, var, variadic};

pub(super) fn register(insert: &mut dyn FnMut(Signature)) {
    // ─── Null / coalesce / comparison family.
    insert(Signature::new(
        "COALESCE",
        vec![tp("T", TypeConstraint::Any)],
        vec![variadic(var("T"))],
        TypeExpr::Var("T".into()),
    ));
    insert(Signature::new(
        "GREATEST",
        vec![tp("T", TypeConstraint::Ordered)],
        vec![variadic(var("T"))],
        TypeExpr::Var("T".into()),
    ));
    insert(Signature::new(
        "LEAST",
        vec![tp("T", TypeConstraint::Ordered)],
        vec![variadic(var("T"))],
        TypeExpr::Var("T".into()),
    ));
    insert(Signature::new(
        "NULLIF",
        vec![tp("T", TypeConstraint::Any)],
        vec![var("T"), var("T")],
        TypeExpr::Var("T".into()),
    ));
    insert(
        Signature::new(
            "IFNULL",
            vec![tp("T", TypeConstraint::Any)],
            vec![var("T"), var("T")],
            TypeExpr::Var("T".into()),
        )
        // Null-handling alias (Oracle/Snowflake/DuckDB dialect).
        .with_aliases(&["NVL"]),
    );
}
