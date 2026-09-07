/// Phase 50: registry coverage tests — verify that newly-seeded built-ins
/// are present and carry the correct `ExprKind`.
pub(crate) use smelt_types::{
    signatures::{
        ConditionalArm, Emission, ExprKind, Position, RewriteId, SigParam, Signature, SyntaxForm,
        TemplateError,
    },
    validate_conditional, validate_template, BuiltinRegistry, CallFacts, ConditionalError,
    DataType, DialectId, OperandClass, SettledEmission, TypeConstraint, TypeExpr,
};

mod emission;
mod operand_conditional;
mod operators_aggregates;
mod position_retired;
mod temporal_kind_syntax;
mod window_string_math;

/// A two-parameter integer signature — only its params/kind matter to the
/// `validate_template` tests below.
pub(crate) fn two_arg_signature() -> Signature {
    Signature::new(
        "TEST_TWO_ARG",
        vec![],
        vec![
            SigParam::Concrete(TypeConstraint::Concrete(DataType::Integer)),
            SigParam::Concrete(TypeConstraint::Concrete(DataType::Integer)),
        ],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Integer)),
    )
}

pub(crate) fn one_arg_signature() -> Signature {
    Signature::new(
        "TEST_ONE_ARG",
        vec![],
        vec![SigParam::Concrete(TypeConstraint::Concrete(
            DataType::Integer,
        ))],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Integer)),
    )
}

pub(crate) fn variadic_signature() -> Signature {
    Signature::new(
        "TEST_VARIADIC",
        vec![],
        vec![SigParam::Variadic(Box::new(SigParam::Concrete(
            TypeConstraint::Concrete(DataType::Integer),
        )))],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Integer)),
    )
}

// ─── `Emission::Template` validation ───────────────────────────────────────

#[test]
fn template_index_beyond_arity_is_rejected() {
    let sig = two_arg_signature();
    let err = validate_template("MOD({0}, {2})", &sig, Position::Any).unwrap_err();
    assert_eq!(
        err,
        TemplateError::IndexOutOfRange {
            signature: sig.name.clone(),
            index: 2,
            arity: 2,
        }
    );
}

#[test]
fn template_dropping_an_argument_is_rejected() {
    let sig = two_arg_signature();
    let err = validate_template("MOD({0})", &sig, Position::Any).unwrap_err();
    assert_eq!(
        err,
        TemplateError::ArgumentUnreferenced {
            signature: sig.name.clone(),
            index: 1,
        }
    );
}

#[test]
fn template_with_unbalanced_parens_is_rejected() {
    let sig = two_arg_signature();
    let err = validate_template("(MOD({0}, {1})", &sig, Position::Any).unwrap_err();
    assert_eq!(
        err,
        TemplateError::UnbalancedParens {
            signature: sig.name.clone()
        }
    );
}

#[test]
fn non_call_template_at_a_window_position_is_rejected() {
    let sig = one_arg_signature();
    let err = validate_template("{0} - 1", &sig, Position::WholePartitionWindow).unwrap_err();
    assert_eq!(
        err,
        TemplateError::NonCallAtWindowPosition {
            signature: sig.name.clone()
        }
    );
    // A call-shaped template at the same position is fine.
    assert!(validate_template("SUM({0})", &sig, Position::WholePartitionWindow).is_ok());
}

#[test]
fn template_on_a_variadic_signature_is_rejected() {
    let sig = variadic_signature();
    let err = validate_template("{0}", &sig, Position::Any).unwrap_err();
    assert_eq!(
        err,
        TemplateError::VariadicSignature {
            signature: sig.name.clone()
        }
    );
}

#[test]
fn the_full_registry_builds() {
    // Forces `REGISTRY`'s `LazyLock` to build — a malformed `Emission::Template`
    // row panics at construction (the registry seed's `insert` closure), so
    // simply resolving anything already exercises the build-time gate.
    let names: Vec<&str> = BuiltinRegistry::names().collect();
    assert!(!names.is_empty());
    for sig in names.iter().filter_map(|n| BuiltinRegistry::resolve(n)) {
        for (_, position, emission) in sig.emission.iter() {
            if let Emission::Template(t) = emission {
                assert!(
                    validate_template(t, sig, *position).is_ok(),
                    "{}: template {t:?} failed validation",
                    sig.name
                );
            }
        }
    }
}

/// A bare test signature with no parameters — only its `emission` table
/// matters to the tests below.
pub(crate) fn test_signature(name: &str) -> Signature {
    Signature::new(
        name,
        vec![],
        vec![],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Integer)),
    )
}
