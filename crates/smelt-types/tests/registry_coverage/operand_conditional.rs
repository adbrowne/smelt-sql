use super::*;

// ─── Operand-conditional verdicts (phase 5) ────────────────────────────────

#[test]
fn operand_class_is_total_over_every_datatype() {
    assert_eq!(OperandClass::of(&DataType::Boolean), OperandClass::Boolean);
    assert_eq!(
        OperandClass::of(&DataType::SmallInt),
        OperandClass::Integral
    );
    assert_eq!(OperandClass::of(&DataType::Integer), OperandClass::Integral);
    assert_eq!(OperandClass::of(&DataType::BigInt), OperandClass::Integral);
    assert_eq!(OperandClass::of(&DataType::Float), OperandClass::Floating);
    assert_eq!(OperandClass::of(&DataType::Double), OperandClass::Floating);
    assert_eq!(
        OperandClass::of(&DataType::Decimal {
            precision: 10,
            scale: 2
        }),
        OperandClass::Decimal
    );
    assert_eq!(
        OperandClass::of(&DataType::Varchar { max_length: None }),
        OperandClass::String
    );
    assert_eq!(
        OperandClass::of(&DataType::Char { length: 1 }),
        OperandClass::String
    );
    assert_eq!(OperandClass::of(&DataType::Text), OperandClass::String);
    assert_eq!(OperandClass::of(&DataType::Blob), OperandClass::Binary);
    assert_eq!(OperandClass::of(&DataType::Date), OperandClass::Temporal);
    assert_eq!(OperandClass::of(&DataType::Time), OperandClass::Temporal);
    assert_eq!(
        OperandClass::of(&DataType::Timestamp {
            with_timezone: false
        }),
        OperandClass::Temporal
    );
    assert_eq!(
        OperandClass::of(&DataType::Interval),
        OperandClass::Interval
    );
    assert_eq!(
        OperandClass::of(&DataType::Array(Box::new(DataType::Integer))),
        OperandClass::Composite
    );
    assert_eq!(
        OperandClass::of(&DataType::Struct(vec![(
            "a".to_string(),
            DataType::Integer
        )])),
        OperandClass::Composite
    );
    assert_eq!(
        OperandClass::of(&DataType::Map(
            Box::new(DataType::Text),
            Box::new(DataType::Integer)
        )),
        OperandClass::Composite
    );
    assert_eq!(OperandClass::of(&DataType::Null), OperandClass::Unresolved);
    assert_eq!(
        OperandClass::of(&DataType::Unknown(smelt_types::UnknownReason::Dynamic)),
        OperandClass::Unresolved
    );
}

const NATIVE_IF_INTEGRAL: ConditionalArm = ConditionalArm {
    arity: None,
    classes: &[(0, OperandClass::Integral)],
    verdict: SettledEmission::Native,
};
const RENAME_IF_STRING: ConditionalArm = ConditionalArm {
    arity: None,
    classes: &[(0, OperandClass::String)],
    verdict: SettledEmission::Rename("STR_ARM"),
};
const OTHERWISE_UNSUPPORTED: ConditionalArm = ConditionalArm {
    arity: None,
    classes: &[],
    verdict: SettledEmission::Unsupported {
        reason: "otherwise",
    },
};

#[test]
fn first_matching_arm_wins() {
    // Both arms match an Integral first argument — `NATIVE_IF_INTEGRAL` is
    // listed first, so its verdict wins even though `RENAME_IF_STRING`'s
    // sibling below would also match a hypothetical wider guard.
    const ARMS: &[ConditionalArm] = &[
        NATIVE_IF_INTEGRAL,
        ConditionalArm {
            arity: None,
            classes: &[],
            verdict: SettledEmission::Rename("SHOULD_NOT_WIN"),
        },
        OTHERWISE_UNSUPPORTED,
    ];
    let sig = one_arg_signature().with_emission(&[(
        DialectId::DuckDb,
        Position::Any,
        Emission::Conditional(ARMS),
    )]);
    let facts = CallFacts::new(vec![OperandClass::Integral]);
    assert_eq!(
        sig.settle_at(DialectId::DuckDb, Position::Any, &facts),
        SettledEmission::Native
    );
}

#[test]
fn an_arity_guard_selects_on_call_arity() {
    const ARMS: &[ConditionalArm] = &[
        ConditionalArm {
            arity: Some(1),
            classes: &[],
            verdict: SettledEmission::Rename("ONE_ARG"),
        },
        ConditionalArm {
            arity: Some(2),
            classes: &[],
            verdict: SettledEmission::Rename("TWO_ARG"),
        },
        OTHERWISE_UNSUPPORTED,
    ];
    // `//`-shaped two-arg signature so both arities are admitted.
    let sig = two_arg_signature().with_emission(&[(
        DialectId::DuckDb,
        Position::Any,
        Emission::Conditional(ARMS),
    )]);
    assert_eq!(
        sig.settle_at(DialectId::DuckDb, Position::Any, &CallFacts::unresolved(1)),
        SettledEmission::Rename("ONE_ARG")
    );
    assert_eq!(
        sig.settle_at(DialectId::DuckDb, Position::Any, &CallFacts::unresolved(2)),
        SettledEmission::Rename("TWO_ARG")
    );
}

#[test]
fn a_class_guard_selects_on_argument_class() {
    const ARMS: &[ConditionalArm] = &[NATIVE_IF_INTEGRAL, RENAME_IF_STRING, OTHERWISE_UNSUPPORTED];
    let sig = one_arg_signature().with_emission(&[(
        DialectId::DuckDb,
        Position::Any,
        Emission::Conditional(ARMS),
    )]);
    assert_eq!(
        sig.settle_at(
            DialectId::DuckDb,
            Position::Any,
            &CallFacts::new(vec![OperandClass::String])
        ),
        SettledEmission::Rename("STR_ARM")
    );
}

#[test]
fn an_unmatched_call_takes_the_otherwise_arm() {
    const ARMS: &[ConditionalArm] = &[NATIVE_IF_INTEGRAL, OTHERWISE_UNSUPPORTED];
    let sig = one_arg_signature().with_emission(&[(
        DialectId::DuckDb,
        Position::Any,
        Emission::Conditional(ARMS),
    )]);
    assert_eq!(
        sig.settle_at(
            DialectId::DuckDb,
            Position::Any,
            &CallFacts::new(vec![OperandClass::Boolean])
        ),
        SettledEmission::Unsupported {
            reason: "otherwise"
        }
    );
    // An unresolved operand lands on `otherwise` too — the fail-safe
    // direction (`docs/specs/multi_backend.md` §"Operand-conditional
    // verdicts").
    assert_eq!(
        sig.settle_at(DialectId::DuckDb, Position::Any, &CallFacts::unresolved(1)),
        SettledEmission::Unsupported {
            reason: "otherwise"
        }
    );
}

#[test]
fn a_conditional_without_an_otherwise_arm_fails_validation() {
    const ARMS: &[ConditionalArm] = &[NATIVE_IF_INTEGRAL];
    let sig = one_arg_signature();
    assert_eq!(
        validate_conditional(ARMS, &sig, Position::Any),
        Err(ConditionalError::MissingOtherwise {
            signature: sig.name.clone()
        })
    );
}

#[test]
fn an_otherwise_arm_not_last_fails_validation() {
    const ARMS: &[ConditionalArm] = &[OTHERWISE_UNSUPPORTED, NATIVE_IF_INTEGRAL];
    let sig = one_arg_signature();
    assert_eq!(
        validate_conditional(ARMS, &sig, Position::Any),
        Err(ConditionalError::MissingOtherwise {
            signature: sig.name.clone()
        })
    );
}

#[test]
fn a_conditional_naming_an_arity_the_signature_does_not_admit_fails_validation() {
    const ARMS: &[ConditionalArm] = &[
        ConditionalArm {
            arity: Some(5),
            classes: &[],
            verdict: SettledEmission::Native,
        },
        OTHERWISE_UNSUPPORTED,
    ];
    let sig = one_arg_signature();
    assert_eq!(
        validate_conditional(ARMS, &sig, Position::Any),
        Err(ConditionalError::ArityNotAdmitted {
            signature: sig.name.clone(),
            arity: 5
        })
    );
}

#[test]
fn a_conditional_naming_an_argument_index_beyond_arity_fails_validation() {
    const ARMS: &[ConditionalArm] = &[
        ConditionalArm {
            arity: None,
            classes: &[(3, OperandClass::Integral)],
            verdict: SettledEmission::Native,
        },
        OTHERWISE_UNSUPPORTED,
    ];
    let sig = one_arg_signature();
    assert_eq!(
        validate_conditional(ARMS, &sig, Position::Any),
        Err(ConditionalError::ArgumentIndexOutOfRange {
            signature: sig.name.clone(),
            index: 3
        })
    );
}

#[test]
fn a_conditional_arm_template_is_validated() {
    // Phase 9: a `Conditional` arm's `Template` verdict is held to the same
    // registry-construction discipline as a top-level `Emission::Template`
    // row — an out-of-range placeholder must fail to build, not misbehave
    // silently at print time.
    const ARMS: &[ConditionalArm] = &[ConditionalArm {
        arity: None,
        classes: &[],
        verdict: SettledEmission::Template("{9}"),
    }];
    let sig = one_arg_signature();
    assert_eq!(
        validate_conditional(ARMS, &sig, Position::Any),
        Err(ConditionalError::InvalidTemplateArm {
            signature: sig.name.clone(),
            arm_index: 0,
            error: TemplateError::IndexOutOfRange {
                signature: sig.name.clone(),
                index: 9,
                arity: 1,
            },
        })
    );
}

#[test]
fn the_full_registry_builds_with_conditional_validation() {
    // Mirrors `the_full_registry_builds`, extended to `Conditional` entries:
    // forcing the registry to build already exercises `validate_conditional`
    // via the seed's `insert` closure (a malformed entry panics at
    // construction). Phase 7 landed the first production `Conditional`
    // entries (`LOG`, `//`, `TRUNC`, `TO_JSON` on Spark), so this loop now
    // exercises real registry data, not just a hypothetical future one.
    let names: Vec<&str> = BuiltinRegistry::names().collect();
    for sig in names.iter().filter_map(|n| BuiltinRegistry::resolve(n)) {
        for (_, position, emission) in sig.emission.iter() {
            if let Emission::Conditional(arms) = emission {
                assert!(
                    validate_conditional(arms, sig, *position).is_ok(),
                    "{}: conditional arms failed validation",
                    sig.name
                );
            }
        }
    }
}

#[test]
fn log_settles_by_arity_on_spark() {
    let sig = BuiltinRegistry::resolve("LOG").expect("LOG is registered");
    assert_eq!(
        sig.settle_at(
            DialectId::SparkSql,
            Position::Any,
            &CallFacts::unresolved(1)
        ),
        SettledEmission::Rename("LOG10")
    );
    assert_eq!(
        sig.settle_at(
            DialectId::SparkSql,
            Position::Any,
            &CallFacts::unresolved(2)
        ),
        SettledEmission::Native
    );
    // DuckDB is unchanged (`Native` at both arities) — it has no emission
    // row for `LOG` at all.
    assert_eq!(
        sig.settle_at(DialectId::DuckDb, Position::Any, &CallFacts::unresolved(1)),
        SettledEmission::Native
    );
    assert_eq!(
        sig.settle_at(DialectId::DuckDb, Position::Any, &CallFacts::unresolved(2)),
        SettledEmission::Native
    );
}

#[test]
fn intdiv_settles_per_operand_class_on_spark() {
    let sig = BuiltinRegistry::resolve("//").expect("// is registered");
    assert_eq!(
        sig.settle_at(
            DialectId::SparkSql,
            Position::Any,
            &CallFacts::new(vec![OperandClass::Integral, OperandClass::Integral])
        ),
        SettledEmission::Template("{0} DIV {1}")
    );
    assert_eq!(
        sig.settle_at(
            DialectId::SparkSql,
            Position::Any,
            &CallFacts::new(vec![OperandClass::Floating, OperandClass::Floating])
        ),
        SettledEmission::Template("{0} / {1}")
    );
    assert_eq!(
        sig.settle_at(
            DialectId::SparkSql,
            Position::Any,
            &CallFacts::new(vec![OperandClass::Decimal, OperandClass::Decimal])
        ),
        SettledEmission::Template("{0} / {1}")
    );
    assert_eq!(
        sig.settle_at(
            DialectId::SparkSql,
            Position::Any,
            &CallFacts::unresolved(2)
        ),
        SettledEmission::Unsupported {
            reason: "Spark SQL has no infix `//`; use a typed FLOOR(a / b) or DIV(a, b)"
        }
    );
}

/// Phase 8 (docs/outcomes/20260904-dialect-emission-vocabulary) closed every
/// remaining #178 Spark schema-gap ledger row with an explicit registry
/// verdict — never falling through to the implicit `Native` default, which
/// would say nothing was ever measured. (`DATE_ADD` is #176, untouched by
/// this phase, and is not in this list.)
#[test]
fn spark_schema_gap_names_have_an_explicit_verdict() {
    let closed_names = [
        "AGE",
        "DATE_SUB",
        "GLOB",
        "JSON_ARRAY",
        "JSON_ARRAY_LENGTH",
        "JSON_CONTAINS",
        "JSON_OBJECT",
        "JSON_OBJECT_KEYS",
        "MAKE_TIME",
        "MAKE_TIMESTAMPTZ",
        "QUOTE_IDENT",
        "QUOTE_LITERAL",
        "TO_SECONDS",
        "TRUNCATE",
        "GROUP_CONCAT",
    ];
    for name in closed_names {
        let sig = BuiltinRegistry::resolve(name).unwrap_or_else(|| panic!("{name} is registered"));
        let has_explicit_spark_row = sig
            .emission
            .iter()
            .any(|(d, _, _)| *d == DialectId::SparkSql);
        assert!(
            has_explicit_spark_row,
            "{name} has no explicit SparkSql emission row — resolves to the implicit Native \
             default, which is not a measured verdict"
        );
    }
}

/// Guards the constraint phase 8 leaned on repeatedly: a signature whose
/// arity is still the permissive variadic `any_args()` shape cannot carry a
/// `Template`, because a fixed `{n}` placeholder can't name a variadic tail
/// (`validate_template`'s `TemplateOnVariadicSignature` error, already
/// exercised directly above by `template_on_a_variadic_signature_is_rejected`).
/// This test pins that the constraint holds for a *real* registry row this
/// phase left `Unsupported` for exactly this reason (`GROUP_CONCAT`), not
/// just the synthetic `variadic_signature()` fixture.
#[test]
fn a_variadic_signature_still_rejects_a_template() {
    let sig = BuiltinRegistry::resolve("GROUP_CONCAT").expect("GROUP_CONCAT is registered");
    let err = validate_template("concat_ws({1}, collect_list({0}))", sig, Position::Any)
        .expect_err("a template over a variadic signature must be rejected");
    assert_eq!(
        err,
        TemplateError::VariadicSignature {
            signature: sig.name.clone(),
        }
    );
}
