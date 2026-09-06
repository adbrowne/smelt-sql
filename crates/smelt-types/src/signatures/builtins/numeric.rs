//! Built-in registry rows: numeric.
//!
//! Data only — every row is handed to the single `BuiltinRegistry` table
//! constructed in [`super`].

use super::super::{
    ConditionalArm, Emission, Position, SettledEmission, Signature, TypeConstraint, TypeExpr,
};
use super::{concrete, tp, var, variadic};
use crate::{DataType, DialectId};

pub(super) fn register(insert: &mut dyn FnMut(Signature)) {
    // ─── Arithmetic / numeric scalars.
    insert(Signature::new(
        "ABS",
        vec![tp("T", TypeConstraint::Numeric)],
        vec![var("T")],
        TypeExpr::Var("T".into()),
    ));
    insert(Signature::new(
        "POWER",
        vec![],
        vec![concrete(DataType::Double), concrete(DataType::Double)],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Double)),
    ));
    insert(Signature::new(
        "SQRT",
        vec![],
        vec![concrete(DataType::Double)],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Double)),
    ));
    insert(
        Signature::new(
            "LOG",
            vec![],
            vec![variadic(concrete(DataType::Double))],
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Double)),
        )
        // `LOG(x)` is base-10 on DuckDB but natural on Spark, so the one-argument
        // form needs a rename; `LOG(base, x)` is native on Spark at both
        // engines. Verified live 2026-09-06: DuckDB `log(100)` = `log10(100)` =
        // 2.0 while Spark's `log(100)` = `ln(100)` = 4.605...; both agree
        // `log(2, 8)` = 3.0.
        .with_emission(&[(
            DialectId::SparkSql,
            Position::Any,
            Emission::Conditional(&[
                ConditionalArm {
                    arity: Some(1),
                    classes: &[],
                    verdict: SettledEmission::Rename("LOG10"),
                },
                // `otherwise` covers every other arity — in practice, the
                // two-argument `LOG(base, x)` form, native on both engines.
                ConditionalArm {
                    arity: None,
                    classes: &[],
                    verdict: SettledEmission::Native,
                },
            ]),
        )]),
    );
    insert(Signature::new(
        "LN",
        vec![],
        vec![concrete(DataType::Double)],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Double)),
    ));
    insert(Signature::new(
        "ROUND",
        vec![],
        vec![concrete(DataType::Double)],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Double)),
    ));
    insert(Signature::new(
        "CEIL",
        vec![],
        vec![concrete(DataType::Double)],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Double)),
    ));
    insert(Signature::new(
        "FLOOR",
        vec![],
        vec![concrete(DataType::Double)],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Double)),
    ));
}
