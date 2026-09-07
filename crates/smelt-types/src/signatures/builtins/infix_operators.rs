//! Built-in registry rows: infix operators.
//!
//! Data only — every row is handed to the single `BuiltinRegistry` table
//! constructed in [`super`].

use super::super::{
    ConditionalArm, Emission, OperandClass, Position, SettledEmission, Signature, SyntaxForm,
    TypeConstraint, TypeExpr,
};
use super::{concrete, tp, var};
use crate::{DataType, DialectId};

pub(super) fn register(insert: &mut dyn FnMut(Signature)) {
    // ─── Infix operators.
    //
    // These are BINARY_EXPR in the CST, not FUNCTION_CALL, and were absent from
    // the registry entirely. They are registered so per-dialect emission has one
    // owner and so the audit enumeration cannot walk past them — `^` is the
    // silent-divergence case issue #171 was filed about.
    for op in ["%", "^", "**"] {
        let emission: &'static [(DialectId, Position, Emission)] = match op {
            "%" => &[(
                DialectId::BigQuery,
                Position::Any,
                Emission::Template("MOD({0}, {1})"),
            )],
            "^" | "**" => &[
                (
                    DialectId::SparkSql,
                    Position::Any,
                    Emission::Template("POWER({0}, {1})"),
                ),
                (
                    DialectId::BigQuery,
                    Position::Any,
                    Emission::Template("POWER({0}, {1})"),
                ),
            ],
            _ => &[],
        };
        insert(
            Signature::new(
                op,
                vec![tp("T", TypeConstraint::Numeric)],
                vec![var("T"), var("T")],
                TypeExpr::Var("T".into()),
            )
            .with_syntax_form(SyntaxForm::Infix)
            .with_emission(emission),
        );
    }
    insert(
        Signature::new(
            "//",
            vec![tp("T", TypeConstraint::Numeric)],
            vec![var("T"), var("T")],
            TypeExpr::Var("T".into()),
        )
        .with_syntax_form(SyntaxForm::Infix)
        .with_emission(&[
            // DuckDB's `//` truncates toward zero for integer operands and
            // degrades to plain division for floats; settled per operand
            // class since a single spelling on Spark cannot be correct for
            // both. Verified live 2026-09-06: DuckDB `-7 // 2` = `7 // -2` =
            // -3 (truncation toward zero, matched by Spark's `DIV`); DuckDB
            // `7.5 // 2.0` = 3.75 and `7 // 2` over `DECIMAL(10,2)` = 3.5
            // (plain division, matched by Spark's `/`).
            (
                DialectId::SparkSql,
                Position::Any,
                Emission::Conditional(&[
                    ConditionalArm {
                        arity: None,
                        classes: &[(0, OperandClass::Integral), (1, OperandClass::Integral)],
                        verdict: SettledEmission::Template("{0} DIV {1}"),
                    },
                    ConditionalArm {
                        arity: None,
                        classes: &[(0, OperandClass::Floating), (1, OperandClass::Floating)],
                        verdict: SettledEmission::Template("{0} / {1}"),
                    },
                    ConditionalArm {
                        arity: None,
                        classes: &[(0, OperandClass::Decimal), (1, OperandClass::Decimal)],
                        verdict: SettledEmission::Template("{0} / {1}"),
                    },
                    ConditionalArm {
                        arity: None,
                        classes: &[],
                        verdict: SettledEmission::Unsupported {
                            reason: "Spark SQL has no infix `//`; use a typed FLOOR(a / b) or \
                                     DIV(a, b)",
                        },
                    },
                ]),
            ),
            (
                DialectId::BigQuery,
                Position::Any,
                Emission::Unsupported {
                    reason: "GoogleSQL has no infix `//`; use a typed FLOOR(a / b) or DIV(a, b)",
                },
            ),
        ]),
    );
    insert(
        Signature::new(
            "||",
            vec![],
            vec![concrete(DataType::Text), concrete(DataType::Text)],
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Text)),
        )
        .with_syntax_form(SyntaxForm::Infix),
    );
}
