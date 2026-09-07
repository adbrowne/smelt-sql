//! Built-in registry rows: operator stubs.
//!
//! Data only — every row is handed to the single `BuiltinRegistry` table
//! constructed in [`super`].

use super::super::{Emission, Position, Signature, SyntaxForm, TypeConstraint, TypeExpr};
use super::{concrete, tp, var};
use crate::{DataType, DialectId};

pub(super) fn register(insert: &mut dyn FnMut(Signature)) {
    // ─── Phase 50: Operator stubs ────────────────────────────────────────────
    //
    // These are not dispatched through `infer_function_type`'s normal path
    // (they use dedicated SQL syntax), but having registry entries enables
    // hover, completion, and future lint rules.

    insert(
        Signature::new(
            "LIKE",
            vec![],
            vec![concrete(DataType::Text), concrete(DataType::Text)],
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Boolean)),
        )
        .with_syntax_form(SyntaxForm::Infix),
    );
    insert(
        Signature::new(
            "ILIKE",
            vec![],
            vec![concrete(DataType::Text), concrete(DataType::Text)],
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Boolean)),
        )
        .with_syntax_form(SyntaxForm::Infix),
    );
    insert(
        Signature::new(
            "GLOB",
            vec![],
            vec![concrete(DataType::Text), concrete(DataType::Text)],
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Boolean)),
        )
        .with_syntax_form(SyntaxForm::Infix)
        // Verified live 2026-09-06 (phase 8): `'abc' GLOB 'a*'` is a parse
        // error on Spark — no `GLOB` operator. `LIKE`/`RLIKE` exist, but
        // translating a shell-style glob pattern into either pattern
        // language is a text transform on the pattern argument's own
        // literal, not a placeholder substitution a template can express.
        .with_emission(&[(
            DialectId::SparkSql,
            Position::Any,
            Emission::Unsupported {
                reason: "no `GLOB` operator; Spark's `LIKE`/`RLIKE` use a different pattern \
                         language, and translating a glob pattern into either isn't a \
                         placeholder substitution",
            },
        )]),
    );
    insert(
        Signature::new(
            "IS_NULL",
            vec![tp("T", TypeConstraint::Any)],
            vec![var("T")],
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Boolean)),
        )
        .with_syntax_form(SyntaxForm::Postfix),
    );
    insert(
        Signature::new(
            "IS_NOT_NULL",
            vec![tp("T", TypeConstraint::Any)],
            vec![var("T")],
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Boolean)),
        )
        .with_syntax_form(SyntaxForm::Postfix),
    );
    insert(
        Signature::new(
            "BETWEEN",
            vec![tp("T", TypeConstraint::Ordered)],
            vec![var("T"), var("T"), var("T")],
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Boolean)),
        )
        .with_syntax_form(SyntaxForm::Special),
    );
    insert(
        Signature::new(
            "IN",
            vec![tp("T", TypeConstraint::Any)],
            vec![var("T"), var("T")],
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Boolean)),
        )
        .with_syntax_form(SyntaxForm::Special),
    );
    insert(
        Signature::new(
            "EXISTS",
            vec![tp("T", TypeConstraint::Any)],
            vec![var("T")],
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Boolean)),
        )
        .with_syntax_form(SyntaxForm::Special),
    );
    insert(
        Signature::new(
            "CAST",
            vec![tp("T", TypeConstraint::Any)],
            vec![var("T")],
            TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Unknown(
                crate::UnknownReason::Dynamic,
            ))),
        )
        .with_syntax_form(SyntaxForm::Special),
    );
}
