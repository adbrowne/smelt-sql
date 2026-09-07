//! Unit tests for the signature/registry modules.

use super::builtins::{tp, var, variadic};
use super::*;
use crate::{DataType, DialectId};
use smelt_parser::{parse, File as AstFile};
use std::collections::BTreeMap;
use std::sync::Arc;

fn parse_file(text: &str) -> (AstFile, String) {
    let clean = smelt_parser::strip_frontmatter(text);
    let parse = parse(&clean);
    let ast = AstFile::cast(parse.syntax()).expect("FILE node");
    (ast, clean)
}

// === Phase 3 tests (still passing) ===

#[test]
fn extracts_minimal_signature() {
    let (file, text) = parse_file("smelt.define foo(x) AS (x + 1)");
    let sigs = extract_function_signatures(&file, &text);
    assert_eq!(sigs.len(), 1);
    assert_eq!(sigs[0].name, "foo");
    assert_eq!(sigs[0].params.len(), 1);
    assert_eq!(sigs[0].params[0].name, "x");
    assert!(sigs[0].params[0].type_ref_text.is_none());
    assert!(sigs[0].params[0].type_ref.is_none());
    assert!(sigs[0].return_type_text.is_none());
    assert!(sigs[0].return_type.is_none());
    assert_eq!(sigs[0].tier, Tier::One);
}

#[test]
fn tier_two_when_params_annotated_return_missing() {
    let (file, text) = parse_file("smelt.define f(x: Expr<Integer>, y: Expr<Integer>) AS (x + y)");
    let sigs = extract_function_signatures(&file, &text);
    assert_eq!(sigs.len(), 1);
    assert_eq!(sigs[0].tier, Tier::Two);
}

#[test]
fn tier_three_when_fully_annotated() {
    let (file, text) = parse_file("smelt.define f(x: Expr<Integer>) -> Expr<Integer> AS (x + 1)");
    let sigs = extract_function_signatures(&file, &text);
    assert_eq!(sigs[0].tier, Tier::Three);
    let ret = sigs[0].return_type_text.as_deref().unwrap();
    assert!(
        ret.contains("Expr<Integer>"),
        "expected return text to contain Expr<Integer>, got {ret:?}"
    );
}

#[test]
fn default_value_flagged() {
    let (file, text) = parse_file("smelt.define f(x: Expr<Integer> = 0) AS (x)");
    let sigs = extract_function_signatures(&file, &text);
    assert!(sigs[0].params[0].has_default);
}

#[test]
fn lookup_by_name() {
    let (file, text) = parse_file("smelt.define a(x) AS (x)\nsmelt.define b(y) AS (y)\n");
    let sig = extract_function_signature_by_name(&file, &text, "b").unwrap();
    assert_eq!(sig.name, "b");
    assert!(extract_function_signature_by_name(&file, &text, "nope").is_none());
}

// === Phase 4 TDD tests ===

#[test]
fn parses_expr_of_concrete_type() {
    assert_eq!(
        parse_smelt_type("Expr<Integer>"),
        Ok(SmeltType::Expr(TypeConstraint::Concrete(DataType::Integer)))
    );
}

#[test]
fn parses_expr_of_boolean_concrete_type() {
    // The plan explicitly calls out Boolean as a required concrete case.
    assert_eq!(
        parse_smelt_type("Expr<Boolean>"),
        Ok(SmeltType::Expr(TypeConstraint::Concrete(DataType::Boolean)))
    );
}

#[test]
fn parses_expr_of_numeric_constraint() {
    assert_eq!(
        parse_smelt_type("Expr<Numeric>"),
        Ok(SmeltType::Expr(TypeConstraint::Numeric))
    );
}

#[test]
fn rejects_unknown_sort() {
    // `TableExpr<T>` — explicitly deferred to Step 3. We expect
    // `UnsupportedSort` with the sort keyword exposed to the user.
    match parse_smelt_type("TableExpr<T>") {
        Err(SmeltTypeParseError::UnsupportedSort { sort, span_text }) => {
            assert_eq!(sort, "TableExpr");
            assert_eq!(span_text, "TableExpr<T>");
        }
        other => panic!("expected UnsupportedSort, got {other:?}"),
    }
}

#[test]
fn rejects_nested_expr() {
    match parse_smelt_type("Expr<Expr<Integer>>") {
        Err(SmeltTypeParseError::NestedExpr { span_text }) => {
            assert_eq!(span_text, "Expr<Expr<Integer>>");
        }
        other => panic!("expected NestedExpr, got {other:?}"),
    }
}

#[test]
fn numeric_constraint_accepts_integer() {
    let c = TypeConstraint::Numeric;
    // Full membership of §16 #9.
    assert!(c.satisfies(&DataType::SmallInt));
    assert!(c.satisfies(&DataType::Integer));
    assert!(c.satisfies(&DataType::BigInt));
    assert!(c.satisfies(&DataType::Float));
    assert!(c.satisfies(&DataType::Double));
    assert!(c.satisfies(&DataType::Decimal {
        precision: 10,
        scale: 2,
    }));
}

#[test]
fn numeric_constraint_rejects_text() {
    let c = TypeConstraint::Numeric;
    assert!(!c.satisfies(&DataType::Text));
    assert!(!c.satisfies(&DataType::Boolean));
    assert!(!c.satisfies(&DataType::Date));
}

#[test]
fn any_constraint_accepts_everything() {
    let c = TypeConstraint::Any;
    assert!(c.satisfies(&DataType::Integer));
    assert!(c.satisfies(&DataType::Text));
    assert!(c.satisfies(&DataType::Boolean));
}

#[test]
fn concrete_constraint_is_exact() {
    let c = TypeConstraint::Concrete(DataType::Integer);
    assert!(c.satisfies(&DataType::Integer));
    assert!(!c.satisfies(&DataType::BigInt));
    assert!(!c.satisfies(&DataType::Text));
}

#[test]
fn parses_expr_with_whitespace() {
    assert_eq!(
        parse_smelt_type("Expr< Integer >"),
        Ok(SmeltType::Expr(TypeConstraint::Concrete(DataType::Integer)))
    );
}

#[test]
fn malformed_missing_angle_brackets() {
    assert!(matches!(
        parse_smelt_type("Expr"),
        Err(SmeltTypeParseError::Malformed { .. })
    ));
}

#[test]
fn malformed_empty_inner() {
    assert!(matches!(
        parse_smelt_type("Expr<>"),
        Err(SmeltTypeParseError::Malformed { .. })
    ));
}

#[test]
fn unknown_inner_type() {
    match parse_smelt_type("Expr<FooBar>") {
        Err(SmeltTypeParseError::UnknownInner { inner, .. }) => {
            assert_eq!(inner, "FooBar");
        }
        other => panic!("expected UnknownInner, got {other:?}"),
    }
}

// === FunctionSig / ParamSpec wiring ===

#[test]
fn function_sig_exposes_parsed_param_types() {
    let (file, text) =
        parse_file("smelt.define f(x: Expr<Integer>, y: Expr<Numeric>) -> Expr<Double> AS (x + y)");
    let sigs = extract_function_signatures(&file, &text);
    assert_eq!(sigs.len(), 1);

    let sig = &sigs[0];
    assert_eq!(
        sig.params[0].type_ref.as_ref().unwrap().as_ref().unwrap(),
        &SmeltType::Expr(TypeConstraint::Concrete(DataType::Integer))
    );
    assert_eq!(
        sig.params[1].type_ref.as_ref().unwrap().as_ref().unwrap(),
        &SmeltType::Expr(TypeConstraint::Numeric)
    );
    assert_eq!(
        sig.return_type.as_ref().unwrap().as_ref().unwrap(),
        &SmeltType::Expr(TypeConstraint::Concrete(DataType::Double))
    );

    // Ranges populated on annotated params and return.
    assert!(sig.params[0].type_ref_range.is_some());
    assert!(sig.params[1].type_ref_range.is_some());
    assert!(sig.return_type_range.is_some());
}

#[test]
fn function_sig_surfaces_bad_annotation_as_error() {
    // `TableExpr<T>` should be parsed into an `Err(UnsupportedSort)` so
    // higher layers can emit a diagnostic. Until the Phase 6 unified
    // harness arrives this is the targeted unit test called out in the
    // plan.
    let (file, text) = parse_file("smelt.define bad(x: TableExpr<T>) AS (x)");
    let sigs = extract_function_signatures(&file, &text);
    assert_eq!(sigs.len(), 1);

    let param = &sigs[0].params[0];
    let err = param
        .type_ref
        .as_ref()
        .expect("annotation present")
        .as_ref()
        .expect_err("should be an error");
    match err {
        SmeltTypeParseError::UnsupportedSort { sort, .. } => {
            assert_eq!(sort, "TableExpr");
        }
        other => panic!("expected UnsupportedSort, got {other:?}"),
    }
    assert!(param.type_ref_range.is_some());
}

#[test]
fn function_sig_surfaces_bad_return_annotation() {
    let (file, text) = parse_file("smelt.define bad(x: Expr<Integer>) -> TableExpr<T> AS (x)");
    let sigs = extract_function_signatures(&file, &text);
    assert_eq!(sigs.len(), 1);

    let err = sigs[0]
        .return_type
        .as_ref()
        .expect("annotation present")
        .as_ref()
        .expect_err("should be an error");
    assert!(matches!(err, SmeltTypeParseError::UnsupportedSort { .. }));
    assert!(sigs[0].return_type_range.is_some());
}

// === Phase 7 TDD tests — Ordered constraint + registry skeleton ===

#[test]
fn ordered_members_match_decision_13() {
    // §16 #13: Numeric ∪ {Text family, temporal family, Boolean, Interval,
    // Blob}. This test enumerates every member exhaustively.
    let c = TypeConstraint::Ordered;

    // Numeric members (also covered by numeric_is_subset_of_ordered, but
    // the research text explicitly enumerates them here).
    assert!(c.satisfies(&DataType::SmallInt));
    assert!(c.satisfies(&DataType::Integer));
    assert!(c.satisfies(&DataType::BigInt));
    assert!(c.satisfies(&DataType::Float));
    assert!(c.satisfies(&DataType::Double));
    assert!(c.satisfies(&DataType::Decimal {
        precision: 10,
        scale: 2,
    }));

    // String family.
    assert!(c.satisfies(&DataType::Text));
    assert!(c.satisfies(&DataType::Varchar { max_length: None }));
    assert!(c.satisfies(&DataType::Varchar {
        max_length: Some(10)
    }));
    assert!(c.satisfies(&DataType::Char { length: 1 }));

    // Temporal family, including both Timestamp tz variants, plus
    // Interval.
    assert!(c.satisfies(&DataType::Date));
    assert!(c.satisfies(&DataType::Time));
    assert!(c.satisfies(&DataType::Timestamp {
        with_timezone: false
    }));
    assert!(c.satisfies(&DataType::Timestamp {
        with_timezone: true
    }));
    assert!(c.satisfies(&DataType::Interval));

    // Remaining singletons.
    assert!(c.satisfies(&DataType::Boolean));
    // "Binary" in §16 #13 is spelt Blob here.
    assert!(c.satisfies(&DataType::Blob));
}

#[test]
fn ordered_excludes_composites() {
    let c = TypeConstraint::Ordered;
    assert!(!c.satisfies(&DataType::Array(Box::new(DataType::Integer))));
    assert!(!c.satisfies(&DataType::Struct(vec![(
        "a".to_string(),
        DataType::Integer,
    )])));
    assert!(!c.satisfies(&DataType::Map(
        Box::new(DataType::Text),
        Box::new(DataType::Integer),
    )));
    // Null and Unknown are explicitly not Ordered members.
    assert!(!c.satisfies(&DataType::Null));
    assert!(!c.satisfies(&DataType::Unknown(crate::UnknownReason::Dynamic)));
}

#[test]
fn numeric_is_subset_of_ordered() {
    // Every type the Numeric constraint accepts must also satisfy the
    // Ordered constraint (§16 #13: Numeric ⊂ Ordered).
    let numerics = [
        DataType::SmallInt,
        DataType::Integer,
        DataType::BigInt,
        DataType::Float,
        DataType::Double,
        DataType::Decimal {
            precision: 10,
            scale: 2,
        },
    ];
    for dt in &numerics {
        assert!(
            TypeConstraint::Numeric.satisfies(dt),
            "expected Numeric to accept {dt:?}"
        );
        assert!(
            TypeConstraint::Ordered.satisfies(dt),
            "expected Ordered to accept numeric {dt:?}"
        );
    }
}

#[test]
fn registry_lookup_by_name() {
    // Phase 8 migrated these entries to the new shape. LOWER/UPPER/LENGTH
    // are still monomorphic (no type params, concrete params + return);
    // ABS moved to `ABS<T: Numeric>(T) → T` per the plan.
    let lower = BuiltinRegistry::resolve("LOWER").expect("LOWER present");
    assert_eq!(lower.name, "LOWER");
    assert!(lower.type_params.is_empty());
    assert_eq!(
        lower.params,
        vec![SigParam::Concrete(TypeConstraint::Concrete(DataType::Text))]
    );
    assert_eq!(
        lower.return_type,
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Text))
    );

    let upper = BuiltinRegistry::resolve("UPPER").expect("UPPER present");
    assert_eq!(
        upper.params,
        vec![SigParam::Concrete(TypeConstraint::Concrete(DataType::Text))]
    );
    assert_eq!(
        upper.return_type,
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Text))
    );

    let length = BuiltinRegistry::resolve("LENGTH").expect("LENGTH present");
    assert_eq!(
        length.params,
        vec![SigParam::Concrete(TypeConstraint::Concrete(DataType::Text))]
    );
    assert_eq!(
        length.return_type,
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::BigInt))
    );

    let abs = BuiltinRegistry::resolve("ABS").expect("ABS present");
    assert_eq!(abs.type_params.len(), 1);
    assert_eq!(abs.type_params[0].name, "T");
    assert_eq!(abs.type_params[0].constraint, TypeConstraint::Numeric);
    assert_eq!(abs.params, vec![SigParam::Var("T".into())]);
    assert_eq!(abs.return_type, TypeExpr::Var("T".into()));
}

#[test]
fn registry_lookup_case_insensitive() {
    let canonical = BuiltinRegistry::resolve("LOWER").expect("LOWER present");
    let lowercase = BuiltinRegistry::resolve("lower").expect("lower present");
    let titlecase = BuiltinRegistry::resolve("Lower").expect("Lower present");
    let mixed = BuiltinRegistry::resolve("LoWeR").expect("LoWeR present");

    // All four lookups must resolve to the same `&'static Signature` —
    // ASCII case folding happens at the lookup boundary, not by inserting
    // multiple entries.
    assert!(std::ptr::eq(canonical, lowercase));
    assert!(std::ptr::eq(canonical, titlecase));
    assert!(std::ptr::eq(canonical, mixed));
}

// === Phase 8 TDD tests — generics + variadics (§16 #14, #15) ===

#[test]
fn min_generic_preserves_input_type() {
    // `MIN<T: Ordered>(T) → T` with Integer must return Integer — the
    // canonical type-preserving case (§16 #14).
    let sig = BuiltinRegistry::resolve("MIN").expect("MIN present");
    let res = unify_call(sig, &[DataType::Integer], &numeric_lub).expect("unification ok");
    assert_eq!(res.return_type, DataType::Integer);
    assert_eq!(res.bindings.get("T"), Some(&DataType::Integer));
}

#[test]
fn coalesce_lub_of_numeric_args() {
    // COALESCE is `<T: Any>(T...) → T`. Any has no promotion chain, so
    // mixing Integer/BigInt/Double would normally fail unification.
    // For the LUB test, we exercise the Numeric-chain path via a bespoke
    // signature (the core behaviour under test is that a Numeric-constrained
    // type variable reduces by LUB across all positions).
    let sig = Signature::new(
        "numeric_coalesce",
        vec![tp("T", TypeConstraint::Numeric)],
        vec![variadic(var("T"))],
        TypeExpr::Var("T".into()),
    );
    let res = unify_call(
        &sig,
        &[DataType::Integer, DataType::BigInt, DataType::Double],
        &numeric_lub,
    )
    .expect("LUB should succeed under Numeric constraint");
    assert_eq!(res.return_type, DataType::Double);
}

#[test]
fn coalesce_text_int_rejects() {
    // COALESCE has an Any-constrained type var. Any has no promotion
    // chain, so mixing Text/Integer must fail with an InconsistentBinding
    // citing position 2 (the Integer that conflicts with T=Text
    // established at position 1).
    let sig = BuiltinRegistry::resolve("COALESCE").expect("COALESCE present");
    let err = unify_call(sig, &[DataType::Text, DataType::Integer], &numeric_lub)
        .expect_err("Text/Integer must not unify under `Any`");
    match err {
        UnificationError::InconsistentBinding {
            var_name,
            positions,
            types,
        } => {
            assert_eq!(var_name, "T");
            // Both positions are cited; position 2 (the mismatch) must
            // appear so the user can pinpoint the offender.
            assert!(
                positions.contains(&2),
                "expected position 2 to be cited, got {positions:?}"
            );
            assert!(
                positions.contains(&1),
                "expected position 1 (the establishing Text) to be cited, got {positions:?}"
            );
            assert!(types.contains(&DataType::Text));
            assert!(types.contains(&DataType::Integer));
        }
        other => panic!("expected InconsistentBinding, got {other:?}"),
    }
}

#[test]
fn greatest_variadic_allows_single_arg() {
    // `GREATEST<T: Ordered>(T...) → T` must accept exactly one arg.
    let sig = BuiltinRegistry::resolve("GREATEST").expect("GREATEST present");
    let res = unify_call(sig, &[DataType::Integer], &numeric_lub)
        .expect("GREATEST should accept a single Integer");
    assert_eq!(res.return_type, DataType::Integer);
}

#[test]
fn concat_zero_args_returns_text() {
    // CONCAT has a concrete Text variadic and a concrete Text return —
    // no type vars to infer. Zero args therefore types cleanly.
    let sig = BuiltinRegistry::resolve("CONCAT").expect("CONCAT present");
    let res = unify_call(sig, &[], &numeric_lub).expect("zero-arity CONCAT ok");
    assert_eq!(res.return_type, DataType::Text);
}

#[test]
fn generic_inference_error_cites_positions() {
    // §16 #14's error-surface contract: messages must cite the positions
    // that forced the inconsistent binding.
    let sig = BuiltinRegistry::resolve("COALESCE").expect("COALESCE present");
    let err = unify_call(
        sig,
        &[DataType::Text, DataType::Integer, DataType::Text],
        &numeric_lub,
    )
    .expect_err("Text/Integer/Text must not unify under Any");
    match &err {
        UnificationError::InconsistentBinding { positions, .. } => {
            // All three positions are cited in the error payload.
            assert_eq!(positions.len(), 3);
            assert!(positions.contains(&1));
            assert!(positions.contains(&2));
            assert!(positions.contains(&3));
        }
        other => panic!("expected InconsistentBinding, got {other:?}"),
    }
    // The Display impl mentions "position N" so users can read the error.
    let rendered = format!("{err}");
    assert!(
        rendered.contains("position 1"),
        "error message should mention position 1, got {rendered:?}"
    );
    assert!(
        rendered.contains("position 2"),
        "error message should mention position 2, got {rendered:?}"
    );
}

// === Phase 8 supplementary tests — signature construction invariants ===

#[test]
fn non_trailing_variadic_rejected() {
    let err = Signature::try_new(
        "bad",
        vec![],
        vec![
            SigParam::Variadic(Box::new(SigParam::Concrete(TypeConstraint::Concrete(
                DataType::Text,
            )))),
            SigParam::Concrete(TypeConstraint::Concrete(DataType::Integer)),
        ],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Text)),
    )
    .expect_err("variadic in non-trailing position must be rejected");
    assert!(matches!(
        err,
        SignatureBuildError::NonTrailingVariadic { .. }
    ));
}

#[test]
fn undeclared_type_var_rejected() {
    let err = Signature::try_new(
        "bad",
        vec![],
        vec![SigParam::Var("T".into())],
        TypeExpr::Var("T".into()),
    )
    .expect_err("undeclared type var must be rejected");
    assert!(matches!(err, SignatureBuildError::UndeclaredTypeVar { .. }));
}

#[test]
fn too_many_args_for_fixed_arity_signature() {
    let sig = BuiltinRegistry::resolve("LOWER").expect("LOWER present");
    let err = unify_call(sig, &[DataType::Text, DataType::Text], &numeric_lub)
        .expect_err("LOWER takes exactly one arg");
    assert!(matches!(err, UnificationError::TooManyArgs { .. }));
}

#[test]
fn missing_args_for_leading_positions() {
    // SUBSTRING takes three args; supplying one triggers MissingArgs.
    let sig = BuiltinRegistry::resolve("SUBSTRING").expect("SUBSTRING present");
    let err = unify_call(sig, &[DataType::Text], &numeric_lub).expect_err("SUBSTRING needs 3 args");
    assert!(matches!(
        err,
        UnificationError::MissingArgs {
            expected: 3,
            got: 1
        }
    ));
}

#[test]
fn constraint_violation_for_wrong_type() {
    // LENGTH(Text) → Integer — passing Integer must violate the Text
    // constraint at position 1.
    let sig = BuiltinRegistry::resolve("LENGTH").expect("LENGTH present");
    let err =
        unify_call(sig, &[DataType::Integer], &numeric_lub).expect_err("LENGTH(Integer) rejects");
    match err {
        UnificationError::ConstraintViolation {
            position, actual, ..
        } => {
            assert_eq!(position, 1);
            assert_eq!(actual, DataType::Integer);
        }
        other => panic!("expected ConstraintViolation, got {other:?}"),
    }
}

#[test]
fn count_accepts_any_returns_bigint() {
    // COUNT(Any) → BigInt is the monomorphic shape that accepts any
    // concrete type without introducing a type variable.
    let sig = BuiltinRegistry::resolve("COUNT").expect("COUNT present");
    for dt in [
        DataType::Integer,
        DataType::Text,
        DataType::Boolean,
        DataType::Date,
    ] {
        let res = unify_call(sig, std::slice::from_ref(&dt), &numeric_lub)
            .unwrap_or_else(|e| panic!("COUNT({dt:?}) should succeed: {e}"));
        assert_eq!(res.return_type, DataType::BigInt);
    }
}

#[test]
fn numeric_lub_matches_promotion_chain() {
    // Spot-check the helper LUB against §16 #9.
    assert_eq!(
        numeric_lub(&DataType::Integer, &DataType::BigInt),
        DataType::BigInt
    );
    assert_eq!(
        numeric_lub(&DataType::Integer, &DataType::Double),
        DataType::Double
    );
    assert_eq!(
        numeric_lub(&DataType::Integer, &DataType::SmallInt),
        DataType::Integer
    );
    // Decimal + integer applies the §15 LUB formula:
    // Integer → Decimal(10,0); s'=max(0,2)=2; p'=max(10,3)+2=12.
    assert_eq!(
        numeric_lub(
            &DataType::Integer,
            &DataType::Decimal {
                precision: 5,
                scale: 2,
            },
        ),
        DataType::Decimal {
            precision: 12,
            scale: 2,
        }
    );
}

// === Phase 4 TDD tests — Decimal LUB formula (§15) ===

#[test]
fn decimal_decimal_lub_coercion_formula() {
    // s' = max(2,3) = 3, p' = max(10-2, 8-3) + 3 = max(8,5) + 3 = 11
    assert_eq!(
        numeric_lub(
            &DataType::Decimal {
                precision: 10,
                scale: 2
            },
            &DataType::Decimal {
                precision: 8,
                scale: 3
            },
        ),
        DataType::Decimal {
            precision: 11,
            scale: 3
        }
    );
}

#[test]
fn decimal_same_params_lub_unchanged() {
    // Same-params: returns unchanged
    assert_eq!(
        numeric_lub(
            &DataType::Decimal {
                precision: 10,
                scale: 2
            },
            &DataType::Decimal {
                precision: 10,
                scale: 2
            },
        ),
        DataType::Decimal {
            precision: 10,
            scale: 2
        }
    );
}

#[test]
fn integer_decimal_lub_lifting() {
    // Integer lifts to Decimal(10,0)
    // s' = max(0,2) = 2, p' = max(10-0, 10-2) + 2 = 10 + 2 = 12
    assert_eq!(
        numeric_lub(
            &DataType::Integer,
            &DataType::Decimal {
                precision: 10,
                scale: 2
            },
        ),
        DataType::Decimal {
            precision: 12,
            scale: 2
        }
    );
}

#[test]
fn bigint_decimal_lub_lifting() {
    // BigInt lifts to Decimal(19,0)
    // s' = max(0,2) = 2, p' = max(19-0, 5-2) + 2 = 19 + 2 = 21
    assert_eq!(
        numeric_lub(
            &DataType::BigInt,
            &DataType::Decimal {
                precision: 5,
                scale: 2
            },
        ),
        DataType::Decimal {
            precision: 21,
            scale: 2
        }
    );
}

#[test]
fn numeric_lub_chain_unaffected() {
    // Non-Decimal cases unchanged
    assert_eq!(
        numeric_lub(&DataType::Integer, &DataType::Double),
        DataType::Double
    );
}

// === Phase 12 TDD tests — multi-level frame rendering + CAST flag ===

#[test]
fn cast_flag_set_when_canonical_differs_from_engine() {
    // Phase 12 TDD test 3 (§16 #9): `SUM` is seeded with
    // canonical = BigInt and engine_native[DuckDb] = DECIMAL(38,0)
    // — the smelt stand-in for DuckDB's HUGEINT return. The
    // `needs_cast_for(DialectId::DuckDb)` hook must flag divergence so
    // Step 7+ can emit a CAST back to BigInt.
    let sum = BuiltinRegistry::resolve("SUM").expect("SUM seeded");
    assert_eq!(sum.canonical_return, Some(DataType::BigInt));
    assert_eq!(
        sum.engine_native.get(&DialectId::DuckDb),
        Some(&DataType::Decimal {
            precision: 38,
            scale: 0,
        })
    );
    assert!(
        sum.needs_cast_for(DialectId::DuckDb),
        "SUM on DuckDB returns HUGEINT (DECIMAL(38,0)) but canonical is BigInt \
             — needs_cast_for must flag the divergence"
    );
    // Dialects that aren't listed default to "native == canonical".
    assert!(
        !sum.needs_cast_for(DialectId::SparkSql),
        "No override for SparkSql → canonical matches native → no cast needed"
    );
}

#[test]
fn cast_flag_false_for_canonical_less_signatures() {
    // The majority of signatures don't declare a canonical — their
    // native return IS their canonical. `needs_cast_for(...)` must
    // return `false` unconditionally for those.
    let lower = BuiltinRegistry::resolve("LOWER").expect("LOWER seeded");
    assert!(lower.canonical_return.is_none());
    assert!(!lower.needs_cast_for(DialectId::DuckDb));
    assert!(!lower.needs_cast_for(DialectId::SparkSql));
    assert!(!lower.needs_cast_for(DialectId::BigQuery));
}

#[test]
fn cast_flag_false_when_native_matches_canonical() {
    // Explicit divergence-negation: a signature that declares a
    // canonical AND a matching native override still reports false.
    let sig = Signature::new(
        "SAME",
        vec![],
        vec![SigParam::Concrete(TypeConstraint::Concrete(
            DataType::Integer,
        ))],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Integer)),
    )
    .with_canonical_return(DataType::Integer)
    .with_engine_native(DialectId::DuckDb, DataType::Integer);
    assert!(!sig.needs_cast_for(DialectId::DuckDb));
}

#[test]
fn frame_info_location_fields_default_none() {
    // Phase 12 added `decl_path`, `decl_range`, `call_site_range`
    // to `FrameInfo`. Constructors that don't populate them must
    // default to `None` so legacy callers (tests, mock harnesses)
    // continue to compile and behave identically.
    let frame = FrameInfo {
        function: "f".into(),
        param: "x".into(),
        bound_type: "INTEGER".into(),
        decl_path: None,
        decl_range: None,
        call_site_range: None,
        fn_id: None,
        element_index: None,
        column_origin: None,
        model_origin: None,
        source_origin: None,
    };
    assert!(frame.decl_path.is_none());
    assert!(frame.decl_range.is_none());
    assert!(frame.call_site_range.is_none());
}

// === Phase 14 TDD tests — ExprKind helpers + registry kind seeding ===

/// `subkind_of` realises the linear `Scalar <= Agg <= Window` chain
/// (§16 #24). Every kind is its own subkind; non-comparable pairs in
/// the *reverse* direction return `false`.
#[test]
fn kind_subtype_chain() {
    // Reflexive.
    assert!(subkind_of(ExprKind::Scalar, ExprKind::Scalar));
    assert!(subkind_of(ExprKind::Agg, ExprKind::Agg));
    assert!(subkind_of(ExprKind::Window, ExprKind::Window));

    // Forward chain: Scalar <= Agg <= Window.
    assert!(subkind_of(ExprKind::Scalar, ExprKind::Agg));
    assert!(subkind_of(ExprKind::Scalar, ExprKind::Window));
    assert!(subkind_of(ExprKind::Agg, ExprKind::Window));

    // Reverse direction is disallowed — Window does NOT fit a Scalar
    // splice point and Agg does NOT fit a Scalar splice point.
    assert!(!subkind_of(ExprKind::Window, ExprKind::Scalar));
    assert!(!subkind_of(ExprKind::Window, ExprKind::Agg));
    assert!(!subkind_of(ExprKind::Agg, ExprKind::Scalar));
}

/// `kind_ceiling` returns the maximum kind in the slice (§16 #24).
/// Empty slice degrades to `Scalar` per the documented invariant.
#[test]
fn selectitems_kind_ceiling() {
    // Empty: Scalar by convention.
    assert_eq!(kind_ceiling(&[]), ExprKind::Scalar);

    // [Scalar] → Scalar.
    assert_eq!(kind_ceiling(&[ExprKind::Scalar]), ExprKind::Scalar);

    // [user_id, COUNT(*)] → Agg (one Agg item lifts the whole list).
    assert_eq!(
        kind_ceiling(&[ExprKind::Scalar, ExprKind::Agg]),
        ExprKind::Agg
    );

    // [COUNT(*) OVER (...)] → Window.
    assert_eq!(kind_ceiling(&[ExprKind::Window]), ExprKind::Window);

    // Window dominates Agg in mixed lists.
    assert_eq!(
        kind_ceiling(&[ExprKind::Agg, ExprKind::Window, ExprKind::Scalar]),
        ExprKind::Window
    );
}

/// Registry seed: aggregates carry [`ExprKind::Agg`].
#[test]
fn registry_aggregates_seeded_with_agg_kind() {
    for name in ["SUM", "AVG", "MIN", "MAX", "COUNT"] {
        let sig =
            BuiltinRegistry::resolve(name).unwrap_or_else(|| panic!("{name} should be registered"));
        assert_eq!(
            sig.kind,
            ExprKind::Agg,
            "{name} should be seeded with ExprKind::Agg"
        );
    }
}

/// Registry seed: window-only built-ins carry [`ExprKind::Window`].
#[test]
fn registry_window_funcs_seeded_with_window_kind() {
    for name in ["ROW_NUMBER", "RANK", "DENSE_RANK", "LAG", "LEAD"] {
        let sig =
            BuiltinRegistry::resolve(name).unwrap_or_else(|| panic!("{name} should be registered"));
        assert_eq!(
            sig.kind,
            ExprKind::Window,
            "{name} should be seeded with ExprKind::Window"
        );
    }
}

/// Registry seed: plain scalar built-ins default to [`ExprKind::Scalar`].
#[test]
fn registry_scalar_defaults() {
    for name in ["LOWER", "UPPER", "ABS", "CONCAT", "POWER", "NOW"] {
        let sig =
            BuiltinRegistry::resolve(name).unwrap_or_else(|| panic!("{name} should be registered"));
        assert_eq!(
            sig.kind,
            ExprKind::Scalar,
            "{name} should default to ExprKind::Scalar"
        );
    }
}

// =================================================================
// Phase 16 — SchemaRequirement / check_schema_requirement tests
// =================================================================

fn req_numeric_rev_cost(tail: RowTail) -> SchemaRequirement {
    SchemaRequirement {
        required: vec![
            (
                "revenue".to_string(),
                DataTypeReq::Constraint(TypeConstraint::Numeric),
                false,
            ),
            (
                "cost".to_string(),
                DataTypeReq::Constraint(TypeConstraint::Numeric),
                false,
            ),
        ],
        tail,
    }
}

#[test]
fn schema_requirement_happy_path_matches_required_columns_exactly() {
    let req = req_numeric_rev_cost(RowTail::None);
    let schema = vec![
        ("revenue".to_string(), DataType::Double),
        (
            "cost".to_string(),
            DataType::Decimal {
                precision: 18,
                scale: 2,
            },
        ),
    ];
    let out = check_schema_requirement(&req, &schema).expect("match");
    // No tail → no binding.
    assert!(out.is_none());
}

#[test]
fn schema_requirement_missing_column_returns_structured_error() {
    // `cost` is absent from the caller's schema.
    let req = req_numeric_rev_cost(RowTail::None);
    let schema = vec![("revenue".to_string(), DataType::Double)];
    let err = check_schema_requirement(&req, &schema).unwrap_err();
    match err {
        SchemaMismatch::MissingColumn { column, required } => {
            assert_eq!(column, "cost");
            assert!(matches!(
                required,
                DataTypeReq::Constraint(TypeConstraint::Numeric)
            ));
        }
        other => panic!("expected MissingColumn, got {other:?}"),
    }
}

#[test]
fn schema_requirement_type_mismatch_returns_structured_error() {
    // `revenue` is Text — not numeric.
    let req = req_numeric_rev_cost(RowTail::None);
    let schema = vec![
        ("revenue".to_string(), DataType::Text),
        ("cost".to_string(), DataType::Double),
    ];
    let err = check_schema_requirement(&req, &schema).unwrap_err();
    match err {
        SchemaMismatch::TypeMismatch {
            column,
            required,
            actual,
        } => {
            assert_eq!(column, "revenue");
            assert!(matches!(
                required,
                DataTypeReq::Constraint(TypeConstraint::Numeric)
            ));
            assert!(actual.contains("TEXT") || actual.contains("Text"));
        }
        other => panic!("expected TypeMismatch, got {other:?}"),
    }
}

#[test]
fn schema_requirement_tail_none_accepts_extras_without_binding() {
    // Phase 16 accepts extras by default — `RowTail::None` still
    // succeeds on a superset schema; the open-record semantics
    // from research §8 mean only `MissingColumn` / `TypeMismatch`
    // produce structural failures. No binding is recorded because
    // the tail is not named.
    let req = req_numeric_rev_cost(RowTail::None);
    let schema = vec![
        ("revenue".to_string(), DataType::Double),
        ("cost".to_string(), DataType::Double),
        ("extra".to_string(), DataType::Text),
    ];
    let out = check_schema_requirement(&req, &schema).expect("extras accepted");
    assert!(
        out.is_none(),
        "tail `None` does not bind extras; got {out:?}"
    );
}

#[test]
fn schema_requirement_tail_anon_accepts_extras_without_binding() {
    let req = req_numeric_rev_cost(RowTail::Anon);
    let schema = vec![
        ("revenue".to_string(), DataType::Double),
        ("cost".to_string(), DataType::Double),
        ("extra".to_string(), DataType::Text),
    ];
    let out = check_schema_requirement(&req, &schema).expect("accept");
    assert!(out.is_none(), "anon tail does not bind; got {out:?}");
}

#[test]
fn schema_requirement_named_tail_binds_extras_in_caller_order() {
    let req = req_numeric_rev_cost(RowTail::Named("r".to_string()));
    let schema = vec![
        ("revenue".to_string(), DataType::Double),
        ("cost".to_string(), DataType::Double),
        ("notes".to_string(), DataType::Text),
        ("extra".to_string(), DataType::BigInt),
    ];
    let binding = check_schema_requirement(&req, &schema)
        .expect("match")
        .expect("named tail produces binding");
    assert_eq!(binding.name, "r");
    assert_eq!(
        binding.extras,
        vec![
            ("notes".to_string(), DataType::Text),
            ("extra".to_string(), DataType::BigInt),
        ]
    );
}

#[test]
fn schema_requirement_concrete_match_accepts_text_varchar() {
    // `notes: Text` required, caller supplies canonical `Varchar`
    // — same family, compatible under our row-requirement rule
    // (Text normalizes to Varchar { max_length: None }).
    let req = SchemaRequirement {
        required: vec![(
            "notes".to_string(),
            DataTypeReq::Concrete(DataType::Text),
            false,
        )],
        tail: RowTail::Anon,
    };
    let schema = vec![("notes".to_string(), DataType::Varchar { max_length: None })];
    assert!(check_schema_requirement(&req, &schema).is_ok());
}

// Phase 18 TDD tests — hover formatter

#[test]
fn lsp_hover_tableexpr_shows_schema() {
    let req = SchemaRequirement {
        required: vec![
            (
                "revenue".to_string(),
                DataTypeReq::Constraint(TypeConstraint::Numeric),
                false,
            ),
            (
                "cost".to_string(),
                DataTypeReq::Constraint(TypeConstraint::Numeric),
                false,
            ),
        ],
        tail: RowTail::None,
    };
    let ty = SmeltType::TableExpr(Some(req));
    let hover = format_smelt_type_hover(&ty);
    assert!(hover.contains("revenue"), "missing 'revenue' in: {hover}");
    assert!(hover.contains("cost"), "missing 'cost' in: {hover}");
    assert!(hover.contains("Numeric"), "missing 'Numeric' in: {hover}");
    assert!(
        hover.starts_with("TableExpr<{"),
        "expected TableExpr<{{..}}>: {hover}"
    );
}

#[test]
fn lsp_hover_bare_tableexpr_shows_type() {
    assert_eq!(
        format_smelt_type_hover(&SmeltType::TableExpr(None)),
        "TableExpr"
    );
}

#[test]
fn lsp_hover_tableexpr_named_tail() {
    let req = SchemaRequirement {
        required: vec![(
            "id".to_string(),
            DataTypeReq::Concrete(DataType::BigInt),
            false,
        )],
        tail: RowTail::Named("r".to_string()),
    };
    let hover = format_smelt_type_hover(&SmeltType::TableExpr(Some(req)));
    assert!(hover.contains("..r"), "expected ..r in: {hover}");
}

#[test]
fn lsp_hover_expr_numeric() {
    let hover = format_smelt_type_hover(&SmeltType::Expr(TypeConstraint::Numeric));
    assert_eq!(hover, "Expr<Numeric>");
}

#[test]
fn lsp_hover_expr_concrete() {
    let hover = format_smelt_type_hover(&SmeltType::Expr(TypeConstraint::Concrete(
        DataType::Integer,
    )));
    assert_eq!(hover, "Expr<INTEGER>");
}

// === Phase 27 TDD tests — bidirectional generics (§16 #14, Decision 14) ===

#[test]
fn coalesce_expected_double_literals_widen() {
    // Decision 14: when context expects Double and the call has Integer
    // args, the expected return type is an additional position for `T`
    // under the Numeric chain.  LUB({Integer, Integer, Double}) = Double.
    let sig = Signature::new(
        "numeric_coalesce",
        vec![tp("T", TypeConstraint::Numeric)],
        vec![variadic(var("T"))],
        TypeExpr::Var("T".into()),
    );
    let res = unify_call_with_expected(
        &sig,
        &[DataType::Integer, DataType::Integer],
        Some(&DataType::Double),
        &numeric_lub,
    )
    .expect("unification ok");
    assert_eq!(res.return_type, DataType::Double);
}

#[test]
fn no_expected_return_positions_unchanged() {
    // Without an expected return, LUB({Integer, Integer}) = Integer.
    let sig = Signature::new(
        "numeric_coalesce",
        vec![tp("T", TypeConstraint::Numeric)],
        vec![variadic(var("T"))],
        TypeExpr::Var("T".into()),
    );
    let res = unify_call_with_expected(
        &sig,
        &[DataType::Integer, DataType::Integer],
        None,
        &numeric_lub,
    )
    .expect("unification ok");
    assert_eq!(res.return_type, DataType::Integer);
}

#[test]
fn expected_return_conflict_local_error() {
    // MIN<T: Ordered>(T) → T with arg=BigInt; expected return=Integer
    // conflicts (Ordered uses exact equality, not LUB).
    // The error must cite both positions: argument position 1 AND return context (0).
    let sig = BuiltinRegistry::resolve("MIN").expect("MIN present");
    let err = unify_call_with_expected(
        sig,
        &[DataType::BigInt],
        Some(&DataType::Integer),
        &numeric_lub,
    )
    .expect_err("BigInt arg vs Integer expected-return must conflict");
    match err {
        UnificationError::InconsistentBinding {
            var_name,
            positions,
            types,
        } => {
            assert_eq!(var_name, "T");
            // Position 0 = return context; position 1 = first argument.
            assert!(
                positions.contains(&0),
                "return context (pos 0) must be cited, got {positions:?}"
            );
            assert!(
                positions.contains(&1),
                "argument position 1 must be cited, got {positions:?}"
            );
            assert!(types.contains(&DataType::BigInt));
            assert!(types.contains(&DataType::Integer));
        }
        other => panic!("expected InconsistentBinding, got {other:?}"),
    }
}

#[test]
fn generics_within_tier2_body() {
    // MIN<T: Ordered>(T) → T with Decimal arg and no expected return
    // must preserve the Decimal type.
    let sig = BuiltinRegistry::resolve("MIN").expect("MIN present");
    let dt = DataType::Decimal {
        precision: 18,
        scale: 6,
    };
    let res = unify_call_with_expected(sig, std::slice::from_ref(&dt), None, &numeric_lub)
        .expect("unification ok");
    assert_eq!(res.return_type, dt);
}

// === Phase A (meta-language) TDD tests: SmeltType::List ===

/// `List<Expr<Integer>>` round-trips through `parse_smelt_type` and
/// `format_smelt_type_hover`.
#[test]
fn list_type_round_trip() {
    let ty = SmeltType::List(Box::new(SmeltType::Expr(TypeConstraint::Concrete(
        DataType::Integer,
    ))));
    // format_smelt_type_hover produces "List<Expr<Integer>>"
    let rendered = format_smelt_type_hover(&ty);
    assert_eq!(rendered, "List<Expr<INTEGER>>");
    // parse_smelt_type parses it back.
    let parsed = parse_smelt_type(&rendered).expect("List<Expr<Integer>> should parse");
    assert_eq!(parsed, ty);
}

/// `List<List<Expr<Varchar>>>` round-trips.
///
/// Note: `DataType::Text` renders as `"TEXT"` via `to_sql()` but `parse_type("TEXT")`
/// returns `Varchar { max_length: None }`, so we use `Varchar` directly for a clean
/// round-trip. The types.md annotation surface uses `Varchar` / `TEXT` interchangeably,
/// and `DataType::Text` normalises to `Varchar`.
#[test]
fn list_type_nested() {
    let inner = SmeltType::Expr(TypeConstraint::Concrete(DataType::Varchar {
        max_length: None,
    }));
    let middle = SmeltType::List(Box::new(inner));
    let outer = SmeltType::List(Box::new(middle));
    let rendered = format_smelt_type_hover(&outer);
    assert_eq!(rendered, "List<List<Expr<VARCHAR>>>");
    let parsed = parse_smelt_type(&rendered).expect("List<List<Expr<Varchar>>> should parse");
    assert_eq!(parsed, outer);
}

/// Covariance: `List<Expr<Integer>> <: List<Expr<Numeric>>` (Integer satisfies Numeric).
/// Anti-covariance: `List<Expr<Numeric>> <: List<Expr<Integer>>` is false.
#[test]
fn list_subtype_covariant() {
    let list_int = SmeltType::List(Box::new(SmeltType::Expr(TypeConstraint::Concrete(
        DataType::Integer,
    ))));
    let list_numeric = SmeltType::List(Box::new(SmeltType::Expr(TypeConstraint::Numeric)));

    // List<Expr<Integer>> <: List<Expr<Numeric>> — Integer satisfies Numeric.
    assert!(
        is_subtype_of(&list_int, &list_numeric),
        "List<Expr<Integer>> must be a subtype of List<Expr<Numeric>>"
    );
    // List<Expr<Numeric>> is NOT <: List<Expr<Integer>>.
    assert!(
        !is_subtype_of(&list_numeric, &list_int),
        "List<Expr<Numeric>> must NOT be a subtype of List<Expr<Integer>>"
    );
}

/// Unrelated element sorts: `List<TableExpr>` is not a subtype of `List<Expr<Numeric>>`.
#[test]
fn list_subtype_invariant_when_element_unrelated() {
    let list_table = SmeltType::List(Box::new(SmeltType::TableExpr(None)));
    let list_numeric = SmeltType::List(Box::new(SmeltType::Expr(TypeConstraint::Numeric)));

    assert!(
        !is_subtype_of(&list_table, &list_numeric),
        "List<TableExpr> must NOT be a subtype of List<Expr<Numeric>>"
    );
    assert!(
        !is_subtype_of(&list_numeric, &list_table),
        "List<Expr<Numeric>> must NOT be a subtype of List<TableExpr>"
    );
}

// === Phase B (meta-language) TDD tests: SmeltType::Lambda ===

/// `Lambda<Expr<Integer>, Expr<Text>>` round-trips through
/// `format_smelt_type_hover` and `parse_smelt_type`.
///
/// Note: `DataType::Text` renders as `"TEXT"` via `to_sql()` but
/// `parse_type("TEXT")` returns `Varchar { max_length: None }`. We use
/// `Varchar` directly for a clean round-trip, consistent with `list_type_nested`.
#[test]
fn lambda_type_round_trip() {
    let ty = SmeltType::Lambda(
        vec![SmeltType::Expr(TypeConstraint::Concrete(DataType::Integer))],
        Box::new(SmeltType::Expr(TypeConstraint::Concrete(
            DataType::Varchar { max_length: None },
        ))),
    );
    let rendered = format_smelt_type_hover(&ty);
    assert_eq!(rendered, "Lambda<Expr<INTEGER>, Expr<VARCHAR>>");
    let parsed =
        parse_smelt_type(&rendered).expect("Lambda<Expr<INTEGER>, Expr<VARCHAR>> should parse");
    assert_eq!(parsed, ty);
}

/// Lambda is invariant: `Lambda<Expr<Integer>, Expr<Text>>` is NOT a subtype of
/// `Lambda<Expr<Numeric>, Expr<Text>>` even though `Expr<Integer> <: Expr<Numeric>`.
#[test]
fn lambda_type_invariant() {
    let lambda_int_text = SmeltType::Lambda(
        vec![SmeltType::Expr(TypeConstraint::Concrete(DataType::Integer))],
        Box::new(SmeltType::Expr(TypeConstraint::Concrete(DataType::Text))),
    );
    let lambda_numeric_text = SmeltType::Lambda(
        vec![SmeltType::Expr(TypeConstraint::Numeric)],
        Box::new(SmeltType::Expr(TypeConstraint::Concrete(DataType::Text))),
    );
    // Lambda is invariant — Integer does NOT widen to Numeric for subtyping.
    assert!(
            !is_subtype_of(&lambda_int_text, &lambda_numeric_text),
            "Lambda<Expr<Integer>, Expr<Text>> must NOT be a subtype of Lambda<Expr<Numeric>, Expr<Text>> (invariant)"
        );
    assert!(
            !is_subtype_of(&lambda_numeric_text, &lambda_int_text),
            "Lambda<Expr<Numeric>, Expr<Text>> must NOT be a subtype of Lambda<Expr<Integer>, Expr<Text>> (invariant)"
        );
}

/// `is_subtype_of(L, L) == true` only for byte-equal `L` (reflexivity).
#[test]
fn lambda_type_equality_only_when_exact() {
    let lambda = SmeltType::Lambda(
        vec![SmeltType::Expr(TypeConstraint::Concrete(DataType::Integer))],
        Box::new(SmeltType::Expr(TypeConstraint::Concrete(DataType::Text))),
    );
    assert!(
        is_subtype_of(&lambda, &lambda),
        "Lambda must be a subtype of itself (reflexivity)"
    );
    let lambda2 = SmeltType::Lambda(
        vec![SmeltType::Expr(TypeConstraint::Concrete(DataType::Integer))],
        Box::new(SmeltType::Expr(TypeConstraint::Concrete(DataType::Boolean))),
    );
    assert!(
        !is_subtype_of(&lambda, &lambda2),
        "Lambda with different body type must NOT be a subtype"
    );
}

// === Phase F (meta-language) TDD tests: multi-arg Lambda ===

/// Multi-arg lambda has distinct equality/arity from single-arg lambda.
#[test]
fn lambda_vec_arity() {
    let lambda_2arg = SmeltType::Lambda(
        vec![
            SmeltType::Expr(TypeConstraint::Concrete(DataType::Integer)),
            SmeltType::Expr(TypeConstraint::Concrete(DataType::Integer)),
        ],
        Box::new(SmeltType::Expr(TypeConstraint::Concrete(DataType::Text))),
    );
    let lambda_1arg = SmeltType::Lambda(
        vec![SmeltType::Expr(TypeConstraint::Concrete(DataType::Integer))],
        Box::new(SmeltType::Expr(TypeConstraint::Concrete(DataType::Text))),
    );
    // Different arities must NOT be equal.
    assert_ne!(
        lambda_2arg, lambda_1arg,
        "Lambda with 2 params must differ from Lambda with 1 param"
    );
    // Subtype must not hold either direction.
    assert!(
        !is_subtype_of(&lambda_2arg, &lambda_1arg),
        "Lambda<(Integer, Integer), Text> must NOT be a subtype of Lambda<Integer, Text>"
    );
    assert!(
        !is_subtype_of(&lambda_1arg, &lambda_2arg),
        "Lambda<Integer, Text> must NOT be a subtype of Lambda<(Integer, Integer), Text>"
    );
}

/// Multi-arg lambda Display renders with tuple syntax; single-arg renders without.
#[test]
fn lambda_vec_display() {
    let lambda_2arg = SmeltType::Lambda(
        vec![
            SmeltType::Expr(TypeConstraint::Concrete(DataType::Integer)),
            SmeltType::Expr(TypeConstraint::Concrete(DataType::Integer)),
        ],
        Box::new(SmeltType::Expr(TypeConstraint::Concrete(DataType::Text))),
    );
    let lambda_1arg = SmeltType::Lambda(
        vec![SmeltType::Expr(TypeConstraint::Concrete(DataType::Integer))],
        Box::new(SmeltType::Expr(TypeConstraint::Concrete(DataType::Text))),
    );
    let display_2 = format!("{}", lambda_2arg);
    let display_1 = format!("{}", lambda_1arg);
    // Multi-arg uses tuple syntax.
    assert!(
        display_2.contains("(") && display_2.contains(")"),
        "Multi-arg lambda must render with tuple parens, got: {}",
        display_2
    );
    assert!(
        display_2.starts_with("Lambda<("),
        "Multi-arg lambda must render as Lambda<(...)>, got: {}",
        display_2
    );
    // Single-arg omits parens.
    assert!(
        display_1.starts_with("Lambda<Expr"),
        "Single-arg lambda must render without tuple parens, got: {}",
        display_1
    );
}

// === Phase C (meta-language) TDD tests — ColumnRef witness + smelt.columns_of ===

#[test]
fn columns_of_signature_returns_list_of_column_ref() {
    // BuiltinRegistry::lookup("smelt.columns_of") must return a SmeltMetaSignature
    // with one positional TableExpr parameter and List<ColumnRef> return.
    let sig = BuiltinRegistry::lookup("smelt.columns_of")
        .expect("smelt.columns_of must be in the smelt meta registry");
    assert_eq!(
        sig.params.len(),
        1,
        "smelt.columns_of takes exactly one param"
    );
    assert!(
        matches!(&sig.params[0], SmeltType::TableExpr(None)),
        "smelt.columns_of param must be TableExpr, got: {:?}",
        sig.params[0]
    );
    assert!(
        matches!(&sig.return_type, SmeltType::List(inner) if matches!(inner.as_ref(), SmeltType::ColumnRef)),
        "smelt.columns_of must return List<ColumnRef>, got: {:?}",
        sig.return_type
    );
}

#[test]
fn column_ref_field_set_is_closed() {
    // COLUMN_REF_FIELDS must expose exactly the 8 closed fields and nothing else.
    let expected = [
        "name",
        "type",
        "is_numeric",
        "is_decimal",
        "is_string",
        "is_temporal",
        "is_integer",
        "is_boolean",
    ];
    for field in &expected {
        assert!(
            column_ref_field(field).is_some(),
            "COLUMN_REF_FIELDS must contain field '{field}'"
        );
    }
    // Any other identifier must return None.
    assert!(
        column_ref_field("foo").is_none(),
        "COLUMN_REF_FIELDS must not contain 'foo'"
    );
    assert!(
        column_ref_field("column_name").is_none(),
        "COLUMN_REF_FIELDS must not contain 'column_name'"
    );
    // Exactly eight fields in the constant.
    assert_eq!(
        COLUMN_REF_FIELDS.len(),
        8,
        "COLUMN_REF_FIELDS must have exactly 8 entries"
    );
    // Verify key field types.
    let name_ty = column_ref_field("name").unwrap();
    assert!(
        matches!(
            name_ty,
            SmeltType::Expr(TypeConstraint::Concrete(DataType::Text))
        ),
        "name field must be Text (Expr<Text>), got: {name_ty:?}"
    );
    let type_ty = column_ref_field("type").unwrap();
    assert!(
        matches!(type_ty, SmeltType::Unknown),
        "c.type maps to SmeltType::Unknown as the forward-compatibility placeholder; got: {:?}",
        type_ty
    );
    // All is_* predicates must be Boolean.
    for pred in &[
        "is_numeric",
        "is_decimal",
        "is_string",
        "is_temporal",
        "is_integer",
        "is_boolean",
    ] {
        let ty = column_ref_field(pred).unwrap();
        assert!(
            matches!(
                ty,
                SmeltType::Expr(TypeConstraint::Concrete(DataType::Boolean))
            ),
            "{pred} field must be Boolean, got: {ty:?}"
        );
    }
}

// === Phase D (meta-language) TDD tests — ModelRef / SourceRef + wide reflection ===

/// `smelt.models.with_tag` resolves to `(Text) -> List<ModelRef>` with one
/// positional parameter; `smelt.models.all` resolves to `() -> List<ModelRef>` with
/// zero parameters; analogous for `smelt.sources.*` returning `List<SourceRef>`.
#[test]
fn wide_reflection_accessor_signatures() {
    // smelt.models.with_tag: (Text) -> List<ModelRef>
    let with_tag_m =
        models_accessor("with_tag").expect("models_accessor(with_tag) must be registered");
    assert_eq!(
        with_tag_m.params.len(),
        1,
        "smelt.models.with_tag must have exactly one positional parameter"
    );
    assert!(
        matches!(
            &with_tag_m.params[0],
            SmeltType::Expr(TypeConstraint::Concrete(DataType::Text))
        ),
        "smelt.models.with_tag param must be Expr<Text>, got: {:?}",
        with_tag_m.params[0]
    );
    assert!(
        matches!(&with_tag_m.return_type, SmeltType::List(inner) if matches!(inner.as_ref(), SmeltType::ModelRef)),
        "smelt.models.with_tag must return List<ModelRef>, got: {:?}",
        with_tag_m.return_type
    );

    // smelt.models.all: () -> List<ModelRef>
    let all_m = models_accessor("all").expect("models_accessor(all) must be registered");
    assert_eq!(
        all_m.params.len(),
        0,
        "smelt.models.all must have zero parameters"
    );
    assert!(
        matches!(&all_m.return_type, SmeltType::List(inner) if matches!(inner.as_ref(), SmeltType::ModelRef)),
        "smelt.models.all must return List<ModelRef>, got: {:?}",
        all_m.return_type
    );

    // smelt.sources.with_tag: (Text) -> List<SourceRef>
    let with_tag_s =
        sources_accessor("with_tag").expect("sources_accessor(with_tag) must be registered");
    assert_eq!(
        with_tag_s.params.len(),
        1,
        "smelt.sources.with_tag must have exactly one positional parameter"
    );
    assert!(
        matches!(
            &with_tag_s.params[0],
            SmeltType::Expr(TypeConstraint::Concrete(DataType::Text))
        ),
        "smelt.sources.with_tag param must be Expr<Text>, got: {:?}",
        with_tag_s.params[0]
    );
    assert!(
        matches!(&with_tag_s.return_type, SmeltType::List(inner) if matches!(inner.as_ref(), SmeltType::SourceRef)),
        "smelt.sources.with_tag must return List<SourceRef>, got: {:?}",
        with_tag_s.return_type
    );

    // smelt.sources.all: () -> List<SourceRef>
    let all_s = sources_accessor("all").expect("sources_accessor(all) must be registered");
    assert_eq!(
        all_s.params.len(),
        0,
        "smelt.sources.all must have zero parameters"
    );
    assert!(
        matches!(&all_s.return_type, SmeltType::List(inner) if matches!(inner.as_ref(), SmeltType::SourceRef)),
        "smelt.sources.all must return List<SourceRef>, got: {:?}",
        all_s.return_type
    );
}

/// `MODEL_REF_FIELDS` exposes exactly `{path: Text, name: Text, tags: List<Text>,
/// columns: List<ColumnRef>}` and no other field; same for `SOURCE_REF_FIELDS`.
#[test]
fn model_ref_field_set_is_closed() {
    let expected = ["path", "name", "tags", "columns"];
    for field in &expected {
        assert!(
            model_ref_field(field).is_some(),
            "MODEL_REF_FIELDS must contain field '{field}'"
        );
        assert!(
            source_ref_field(field).is_some(),
            "SOURCE_REF_FIELDS must contain field '{field}'"
        );
    }
    // Unknown fields must return None.
    assert!(
        model_ref_field("foo").is_none(),
        "MODEL_REF_FIELDS must not contain 'foo'"
    );
    assert!(
        model_ref_field("is_numeric").is_none(),
        "MODEL_REF_FIELDS must not contain 'is_numeric'"
    );
    assert!(
        source_ref_field("foo").is_none(),
        "SOURCE_REF_FIELDS must not contain 'foo'"
    );
    // Exactly four fields in each constant.
    assert_eq!(
        MODEL_REF_FIELDS.len(),
        4,
        "MODEL_REF_FIELDS must have exactly 4 entries"
    );
    assert_eq!(
        SOURCE_REF_FIELDS.len(),
        4,
        "SOURCE_REF_FIELDS must have exactly 4 entries"
    );
    // Verify types: path → Text, name → Text
    let path_ty = model_ref_field("path").unwrap();
    assert!(
        matches!(
            path_ty,
            SmeltType::Expr(TypeConstraint::Concrete(DataType::Text))
        ),
        "path field must be Expr<Text>, got: {path_ty:?}"
    );
    let name_ty = model_ref_field("name").unwrap();
    assert!(
        matches!(
            name_ty,
            SmeltType::Expr(TypeConstraint::Concrete(DataType::Text))
        ),
        "name field must be Expr<Text>, got: {name_ty:?}"
    );
    // tags → List<Expr<Text>>
    let tags_ty = model_ref_field("tags").unwrap();
    assert!(
        matches!(tags_ty, SmeltType::List(inner)
                if matches!(inner.as_ref(), SmeltType::Expr(TypeConstraint::Concrete(DataType::Text)))),
        "tags field must be List<Expr<Text>>, got: {tags_ty:?}"
    );
    // columns → List<ColumnRef>
    let cols_ty = model_ref_field("columns").unwrap();
    assert!(
        matches!(cols_ty, SmeltType::List(inner) if matches!(inner.as_ref(), SmeltType::ColumnRef)),
        "columns field must be List<ColumnRef>, got: {cols_ty:?}"
    );

    // Same checks on source_ref_field
    let s_path_ty = source_ref_field("path").unwrap();
    assert!(
        matches!(
            s_path_ty,
            SmeltType::Expr(TypeConstraint::Concrete(DataType::Text))
        ),
        "SourceRef path field must be Expr<Text>, got: {s_path_ty:?}"
    );
    let s_cols_ty = source_ref_field("columns").unwrap();
    assert!(
        matches!(s_cols_ty, SmeltType::List(inner) if matches!(inner.as_ref(), SmeltType::ColumnRef)),
        "SourceRef columns field must be List<ColumnRef>, got: {s_cols_ty:?}"
    );
}

// === Phase D Phase 2 TDD tests — ModelRef/SourceRef subtype TableExpr ===

/// `ModelRef <: TableExpr` — the subtyping rule fires in the forward direction.
#[test]
fn model_ref_is_subtype_of_table_expr() {
    assert!(
        is_subtype_of(&SmeltType::ModelRef, &SmeltType::TableExpr(None)),
        "ModelRef must be a subtype of TableExpr (forward direction)"
    );
}

/// `SourceRef <: TableExpr` — the subtyping rule fires in the forward direction.
#[test]
fn source_ref_is_subtype_of_table_expr() {
    assert!(
        is_subtype_of(&SmeltType::SourceRef, &SmeltType::TableExpr(None)),
        "SourceRef must be a subtype of TableExpr (forward direction)"
    );
}

/// `TableExpr <: ModelRef` does NOT hold — the rule is one-way.
#[test]
fn table_expr_not_subtype_of_model_ref() {
    assert!(
        !is_subtype_of(&SmeltType::TableExpr(None), &SmeltType::ModelRef),
        "TableExpr must NOT be a subtype of ModelRef (reverse direction forbidden)"
    );
    assert!(
        !is_subtype_of(&SmeltType::TableExpr(None), &SmeltType::SourceRef),
        "TableExpr must NOT be a subtype of SourceRef (reverse direction forbidden)"
    );
}

/// `List<ModelRef> <: List<TableExpr>` — List covariance lifts the element rule
/// automatically.
#[test]
fn list_of_model_ref_is_subtype_of_list_of_table_expr() {
    let list_model_ref = SmeltType::List(Box::new(SmeltType::ModelRef));
    let list_table_expr = SmeltType::List(Box::new(SmeltType::TableExpr(None)));
    assert!(
        is_subtype_of(&list_model_ref, &list_table_expr),
        "List<ModelRef> must be a subtype of List<TableExpr> via List covariance"
    );
    // Reverse does not hold.
    assert!(
        !is_subtype_of(&list_table_expr, &list_model_ref),
        "List<TableExpr> must NOT be a subtype of List<ModelRef>"
    );

    let list_source_ref = SmeltType::List(Box::new(SmeltType::SourceRef));
    assert!(
        is_subtype_of(&list_source_ref, &list_table_expr),
        "List<SourceRef> must be a subtype of List<TableExpr> via List covariance"
    );
}

/// `MODEL_REF_FIELDS` and `SOURCE_REF_FIELDS` have the same field names and
/// types in the same order (uniformity invariant from the design rationale).
#[test]
fn model_ref_and_source_ref_field_sets_are_identical_shape() {
    assert_eq!(
        MODEL_REF_FIELDS.len(),
        SOURCE_REF_FIELDS.len(),
        "MODEL_REF_FIELDS and SOURCE_REF_FIELDS must have the same number of fields"
    );
    for (i, ((model_name, model_ty), (source_name, source_ty))) in MODEL_REF_FIELDS
        .iter()
        .zip(SOURCE_REF_FIELDS.iter())
        .enumerate()
    {
        assert_eq!(
                model_name, source_name,
                "field {i}: MODEL_REF_FIELDS name '{model_name}' != SOURCE_REF_FIELDS name '{source_name}'"
            );
        assert_eq!(
            model_ty, source_ty,
            "field {i} ({model_name}): MODEL_REF_FIELDS type does not match SOURCE_REF_FIELDS type"
        );
    }
}

// =========================================================================
// Phase E1 TDD tests — Record, Map, MAP_API_METHODS, SmeltRecordRegistry
// =========================================================================

/// Helper: build a `SmeltRecordDeclaration` with no source span for testing.
fn make_decl(name: &str, fields: Vec<(&str, SmeltType)>) -> SmeltRecordDeclaration {
    use smelt_parser::TextRange;
    SmeltRecordDeclaration {
        name: name.to_string(),
        fields: fields
            .into_iter()
            .map(|(f, ty)| (f.to_string(), ty, TextRange::new(0.into(), 0.into())))
            .collect(),
        name_span: TextRange::new(0.into(), 0.into()),
        source_path: Arc::from("models/test.sql"),
    }
}

/// Helper: build a `SmeltType::Record` from a slice of `(name, SmeltType)` pairs.
fn record_type(fields: &[(&str, SmeltType)]) -> SmeltType {
    let mut map = BTreeMap::new();
    for (k, v) in fields {
        map.insert(k.to_string(), v.clone());
    }
    SmeltType::Record {
        fields: map,
        name: None,
    }
}

fn named_record_type(name: &str, fields: &[(&str, SmeltType)]) -> SmeltType {
    let mut map = BTreeMap::new();
    for (k, v) in fields {
        map.insert(k.to_string(), v.clone());
    }
    SmeltType::Record {
        fields: map,
        name: Some(name.to_string()),
    }
}

fn expr_text() -> SmeltType {
    SmeltType::Expr(TypeConstraint::Concrete(crate::DataType::Text))
}

fn expr_integer() -> SmeltType {
    SmeltType::Expr(TypeConstraint::Concrete(crate::DataType::Integer))
}

fn expr_number() -> SmeltType {
    SmeltType::Expr(TypeConstraint::Numeric)
}

fn map_text_integer() -> SmeltType {
    SmeltType::Map {
        key: Box::new(expr_text()),
        value: Box::new(expr_integer()),
    }
}

fn map_text_number() -> SmeltType {
    SmeltType::Map {
        key: Box::new(expr_text()),
        value: Box::new(expr_number()),
    }
}

/// Test 1: `record_type_round_trips_field_order_canonicalised`
///
/// `SmeltType::Record { fields: BTreeMap, name: Some("SourceEntry") }` constructed
/// twice with field-insertion in different orders compares equal under `==`.
/// The `Display` impl renders fields in lex order when `name` is `None`;
/// as the type name when `name` is `Some`.
#[test]
fn record_type_round_trips_field_order_canonicalised() {
    // Build in two different insertion orders.
    let mut fields_a = BTreeMap::new();
    fields_a.insert("b".to_string(), expr_integer());
    fields_a.insert("a".to_string(), expr_text());

    let mut fields_b = BTreeMap::new();
    fields_b.insert("a".to_string(), expr_text());
    fields_b.insert("b".to_string(), expr_integer());

    let rec_a = SmeltType::Record {
        fields: fields_a,
        name: Some("SourceEntry".to_string()),
    };
    let rec_b = SmeltType::Record {
        fields: fields_b,
        name: Some("SourceEntry".to_string()),
    };

    assert_eq!(
        rec_a, rec_b,
        "Records with same fields in different insertion orders must be equal"
    );

    // Display: named → renders as type name.
    let display_named = format!("{rec_a}");
    assert_eq!(
        display_named, "SourceEntry",
        "Named record Display must render as the type name"
    );

    // Display: unnamed → renders in lex order.
    let rec_unnamed = record_type(&[("b", expr_integer()), ("a", expr_text())]);
    let display_unnamed = format!("{rec_unnamed}");
    // Lex order: a, b.
    assert!(
        display_unnamed.starts_with("Record<{"),
        "Unnamed record Display must start with Record<{{"
    );
    assert!(
        display_unnamed.contains("a:"),
        "Unnamed record Display must include field 'a'"
    );
    assert!(
        display_unnamed.contains("b:"),
        "Unnamed record Display must include field 'b'"
    );
    // 'a' must appear before 'b' in lex order.
    let a_pos = display_unnamed.find("a:").unwrap();
    let b_pos = display_unnamed.find("b:").unwrap();
    assert!(
        a_pos < b_pos,
        "Unnamed record Display must render fields in lex order (a before b)"
    );
}

/// Test 2: `record_inline_and_named_with_same_field_set_are_structurally_equal`
///
/// Inline and named records with the same fields are structurally equal.
/// The `name` field is accessible and distinguishable for hover.
#[test]
fn record_inline_and_named_with_same_field_set_are_structurally_equal() {
    let inline = record_type(&[("a", expr_text())]);
    let named = named_record_type("X", &[("a", expr_text())]);

    // Structural equality: equal (name ignored).
    assert_eq!(
        inline, named,
        "Inline record and named record with same fields must be structurally equal"
    );

    // The `name` field is accessible and distinguishable.
    let name_val = match &named {
        SmeltType::Record { name, .. } => name.as_deref(),
        _ => panic!("expected Record"),
    };
    assert_eq!(
        name_val,
        Some("X"),
        "Named record must expose its name via the `name` field"
    );

    let inline_name = match &inline {
        SmeltType::Record { name, .. } => name.as_deref(),
        _ => panic!("expected Record"),
    };
    assert_eq!(inline_name, None, "Inline record must have name = None");
}

/// Test 3: `map_type_invariant_both_axes`
///
/// `Map<K, V>` is invariant in both `K` and `V`.
#[test]
fn map_type_invariant_both_axes() {
    // Map<Text, Integer> is NOT a subtype of Map<Text, Number> (covariance forbidden).
    assert!(
        !is_subtype_of(&map_text_integer(), &map_text_number()),
        "Map<Text, Integer> must NOT be subtype of Map<Text, Number> (invariance)"
    );
    // Map<Text, Number> is NOT a subtype of Map<Text, Integer> (contravariance forbidden).
    assert!(
        !is_subtype_of(&map_text_number(), &map_text_integer()),
        "Map<Text, Number> must NOT be subtype of Map<Text, Integer> (invariance)"
    );
    // Map<Text, Integer> IS a subtype of Map<Text, Integer> (reflexivity).
    assert!(
        is_subtype_of(&map_text_integer(), &map_text_integer()),
        "Map<Text, Integer> must be subtype of Map<Text, Integer> (reflexivity)"
    );
}

/// Test 4: `map_api_methods_registry_is_closed_and_exact`
///
/// `MAP_API_METHODS` exposes exactly the five names: `{entries, keys, values, get, has}`.
#[test]
fn map_api_methods_registry_is_closed_and_exact() {
    let expected_names = ["entries", "keys", "values", "get", "has"];
    let actual_names: Vec<&str> = MAP_API_METHODS.iter().map(|m| m.name).collect();

    // Exact five names.
    assert_eq!(
        actual_names.len(),
        5,
        "MAP_API_METHODS must have exactly 5 entries"
    );
    for name in &expected_names {
        assert!(
            actual_names.contains(name),
            "MAP_API_METHODS must contain '{name}'"
        );
    }

    // Lookup of any other identifier returns None.
    assert!(
        lookup_map_api_method("filter").is_none(),
        "lookup of 'filter' must return None"
    );
    assert!(
        lookup_map_api_method("").is_none(),
        "lookup of '' must return None"
    );
    assert!(
        lookup_map_api_method("ENTRIES").is_none(),
        "lookup is case-sensitive; 'ENTRIES' must return None"
    );

    // Arities: entries/keys/values → Exact(0), get/has → Exact(1).
    let entries = lookup_map_api_method("entries").expect("entries must be in MAP_API_METHODS");
    assert_eq!(
        entries.arity,
        Arity::Exact(0),
        "entries arity must be Exact(0)"
    );
    assert!(
        !entries.named_args_allowed,
        "entries must not allow named args"
    );

    let keys = lookup_map_api_method("keys").expect("keys must be in MAP_API_METHODS");
    assert_eq!(keys.arity, Arity::Exact(0), "keys arity must be Exact(0)");

    let values = lookup_map_api_method("values").expect("values must be in MAP_API_METHODS");
    assert_eq!(
        values.arity,
        Arity::Exact(0),
        "values arity must be Exact(0)"
    );

    let get = lookup_map_api_method("get").expect("get must be in MAP_API_METHODS");
    assert_eq!(get.arity, Arity::Exact(1), "get arity must be Exact(1)");

    let has = lookup_map_api_method("has").expect("has must be in MAP_API_METHODS");
    assert_eq!(has.arity, Arity::Exact(1), "has arity must be Exact(1)");

    // Return type of `entries`: List<Record<{key: K, value: V}>>.
    let k = expr_text();
    let v = expr_integer();
    let entries_result = (entries.return_type_formula)(&k, &v);
    match &entries_result {
        SmeltType::List(inner) => match inner.as_ref() {
            SmeltType::Record { fields, .. } => {
                assert_eq!(fields.len(), 2, "entries result record must have 2 fields");
                assert!(
                    fields.contains_key("key"),
                    "entries result must have 'key' field"
                );
                assert!(
                    fields.contains_key("value"),
                    "entries result must have 'value' field"
                );
                assert_eq!(fields["key"], k, "entries 'key' field must be K");
                assert_eq!(fields["value"], v, "entries 'value' field must be V");
            }
            other => panic!("entries result inner must be Record, got: {other:?}"),
        },
        other => panic!("entries result must be List, got: {other:?}"),
    }
}

/// Test 5: `record_width_subtyping_rule`
///
/// Width subtyping: `{a: Text, b: Integer} <: {a: Text}` but not the reverse.
#[test]
fn record_width_subtyping_rule() {
    let wide = record_type(&[("a", expr_text()), ("b", expr_integer())]);
    let narrow = record_type(&[("a", expr_text())]);
    let incompatible = record_type(&[("a", expr_integer())]);

    // Wide <: Narrow (width subtyping).
    assert!(
        is_subtype_of(&wide, &narrow),
        "Record with more fields must be a subtype of record with fewer fields"
    );
    // Narrow is NOT <: Wide (missing field `b`).
    assert!(
        !is_subtype_of(&narrow, &wide),
        "Record with fewer fields must NOT be a subtype of record with more fields"
    );
    // Type mismatch on shared field.
    assert!(
        !is_subtype_of(&narrow, &incompatible),
        "Record with wrong field type must NOT be a subtype"
    );
}

/// Test 6: `record_subtyping_through_nested_field`
///
/// Width subtyping composes through nested record fields.
#[test]
fn record_subtyping_through_nested_field() {
    // sub: Record{a: Record{x: Text, y: Integer}}
    // sup: Record{a: Record{x: Text}}
    let inner_wide = record_type(&[("x", expr_text()), ("y", expr_integer())]);
    let inner_narrow = record_type(&[("x", expr_text())]);

    let sub = record_type(&[("a", inner_wide)]);
    let sup = record_type(&[("a", inner_narrow)]);

    assert!(
        is_subtype_of(&sub, &sup),
        "Width subtyping must compose through nested record fields"
    );
}

/// Test 7: `smelt_record_registry_builder_detects_redefinition`
///
/// Two declarations with the same name produce a redefinition sentinel.
#[test]
fn smelt_record_registry_builder_detects_redefinition() {
    let decl1 = make_decl("Foo", vec![("x", expr_text())]);
    let decl2 = make_decl("Foo", vec![("y", expr_integer())]);

    let (registry, sentinels) = build_record_registry(&[decl1, decl2]);

    // One redefinition sentinel.
    let redef_sentinels: Vec<_> = sentinels
        .iter()
        .filter(|s| s.code == RecordRegistryCode::SmeltRecordRedefinition)
        .collect();
    assert_eq!(
        redef_sentinels.len(),
        1,
        "Expected exactly one SmeltRecordRedefinition sentinel; got: {redef_sentinels:?}"
    );

    // First declaration is authoritative.
    let decl = registry.lookup("Foo").expect("Foo must be in registry");
    assert_eq!(
        decl.fields.len(),
        1,
        "First declaration (x field) must be authoritative"
    );
    assert_eq!(decl.fields[0].0, "x", "First declaration field must be 'x'");
}

/// Test 8: `smelt_record_registry_builder_detects_cycle_self`
///
/// A single self-referential declaration emits one `RecordCyclicDeclaration` sentinel.
#[test]
fn smelt_record_registry_builder_detects_cycle_self() {
    // Node = {child: Node} — self-referential.
    // We model the field type as a named Record with name "Node".
    let node_field_ty = named_record_type("Node", &[]);
    let decl = make_decl("Node", vec![("child", node_field_ty)]);

    let (_, sentinels) = build_record_registry(&[decl]);

    let cycle_sentinels: Vec<_> = sentinels
        .iter()
        .filter(|s| s.code == RecordRegistryCode::RecordCyclicDeclaration)
        .collect();
    assert_eq!(
            cycle_sentinels.len(),
            1,
            "Expected exactly one RecordCyclicDeclaration sentinel for self-cycle; got {cycle_sentinels:?}"
        );
    assert!(
        cycle_sentinels[0].message.contains("Node"),
        "Cycle sentinel message must mention the cycle participant 'Node'"
    );
}

/// Test 9: `smelt_record_registry_builder_detects_cycle_mutual`
///
/// Two mutually referential declarations emit exactly one `RecordCyclicDeclaration` sentinel.
#[test]
fn smelt_record_registry_builder_detects_cycle_mutual() {
    // A = {b: B}, B = {a: A}
    let b_ref = named_record_type("B", &[]);
    let a_ref = named_record_type("A", &[]);

    let decl_a = make_decl("A", vec![("b", b_ref)]);
    let decl_b = make_decl("B", vec![("a", a_ref)]);

    let (_, sentinels) = build_record_registry(&[decl_a, decl_b]);

    let cycle_sentinels: Vec<_> = sentinels
        .iter()
        .filter(|s| s.code == RecordRegistryCode::RecordCyclicDeclaration)
        .collect();
    assert_eq!(
            cycle_sentinels.len(),
            1,
            "Expected exactly one RecordCyclicDeclaration sentinel for mutual cycle; got {cycle_sentinels:?}"
        );
}

/// Test 10: `smelt_record_registry_builder_rejects_reflection_witness_field_types`
///
/// A declaration with `ModelRef`, `ColumnRef`, or `SourceRef` field types emits
/// `RecordFieldTypeForbidden`.
#[test]
fn smelt_record_registry_builder_rejects_reflection_witness_field_types() {
    // Cohort = {model: ModelRef}
    let decl_model = make_decl("Cohort", vec![("model", SmeltType::ModelRef)]);
    let (_, sentinels) = build_record_registry(&[decl_model]);
    let forbidden: Vec<_> = sentinels
        .iter()
        .filter(|s| s.code == RecordRegistryCode::RecordFieldTypeForbidden)
        .collect();
    assert_eq!(
        forbidden.len(),
        1,
        "Expected one RecordFieldTypeForbidden for ModelRef field; got {forbidden:?}"
    );
    assert!(
        forbidden[0].message.contains("ModelRef"),
        "Forbidden sentinel message must mention 'ModelRef'"
    );

    // Same for ColumnRef.
    let decl_col = make_decl("Cohort", vec![("col", SmeltType::ColumnRef)]);
    let (_, sentinels2) = build_record_registry(&[decl_col]);
    assert_eq!(
        sentinels2
            .iter()
            .filter(|s| s.code == RecordRegistryCode::RecordFieldTypeForbidden)
            .count(),
        1,
        "Expected one RecordFieldTypeForbidden for ColumnRef field"
    );

    // Same for SourceRef.
    let decl_src = make_decl("Cohort", vec![("src", SmeltType::SourceRef)]);
    let (_, sentinels3) = build_record_registry(&[decl_src]);
    assert_eq!(
        sentinels3
            .iter()
            .filter(|s| s.code == RecordRegistryCode::RecordFieldTypeForbidden)
            .count(),
        1,
        "Expected one RecordFieldTypeForbidden for SourceRef field"
    );

    // Lambda is also forbidden.
    let lambda_ty = SmeltType::Lambda(vec![expr_text()], Box::new(expr_text()));
    let decl_lambda = make_decl("Cohort", vec![("fn_field", lambda_ty)]);
    let (_, sentinels4) = build_record_registry(&[decl_lambda]);
    assert_eq!(
        sentinels4
            .iter()
            .filter(|s| s.code == RecordRegistryCode::RecordFieldTypeForbidden)
            .count(),
        1,
        "Expected one RecordFieldTypeForbidden for Lambda field"
    );
}

// ── ModelDef type system tests ────────────────────────────────────────────

/// `MODEL_DEF_FIELDS` exposes exactly seven names, in canonical order, and
/// each entry's type matches the spec table.
#[test]
fn model_def_fields_registry_is_closed_and_exact() {
    // Exact seven names in the spec-defined set, canonical order.
    let spec_names = [
        "name",
        "body",
        "materialization",
        "tags",
        "description",
        "timeseries",
        "safety_overrides",
    ];
    assert_eq!(
        MODEL_DEF_FIELDS.len(),
        7,
        "MODEL_DEF_FIELDS must have exactly 7 entries; got {}",
        MODEL_DEF_FIELDS.len()
    );
    let actual_names: Vec<&str> = MODEL_DEF_FIELDS.iter().map(|(n, _)| *n).collect();
    assert_eq!(
        actual_names, spec_names,
        "MODEL_DEF_FIELDS must be in canonical order"
    );
    for name in &spec_names {
        assert!(
            model_def_field(name).is_some(),
            "MODEL_DEF_FIELDS must contain field '{name}'"
        );
    }
    assert!(
        matches!(
            model_def_field("timeseries"),
            Some(SmeltType::Record { .. })
        ),
        "timeseries field must be Record-typed"
    );
    assert!(
        matches!(
            model_def_field("safety_overrides"),
            Some(SmeltType::Record { .. })
        ),
        "safety_overrides field must be Record-typed"
    );
    // Unknown identifiers return None (closed-field invariant).
    assert!(
        model_def_field("incremental").is_none(),
        "MODEL_DEF_FIELDS must NOT contain 'incremental'"
    );
    assert!(
        model_def_field("owner").is_none(),
        "MODEL_DEF_FIELDS must NOT contain 'owner'"
    );

    // name → Expr<Text>
    let name_ty = model_def_field("name").unwrap();
    assert!(
        matches!(
            name_ty,
            SmeltType::Expr(TypeConstraint::Concrete(DataType::Text))
        ),
        "name field must be Expr<Text>, got: {name_ty:?}"
    );
    // body → TableExpr (the single carve-out)
    let body_ty = model_def_field("body").unwrap();
    assert!(
        matches!(body_ty, SmeltType::TableExpr(None)),
        "body field must be TableExpr, got: {body_ty:?}"
    );
    // materialization → Expr<Text>
    let mat_ty = model_def_field("materialization").unwrap();
    assert!(
        matches!(
            mat_ty,
            SmeltType::Expr(TypeConstraint::Concrete(DataType::Text))
        ),
        "materialization field must be Expr<Text>, got: {mat_ty:?}"
    );
    // tags → List<Expr<Text>>
    let tags_ty = model_def_field("tags").unwrap();
    assert!(
        matches!(tags_ty, SmeltType::List(inner)
                if matches!(inner.as_ref(), SmeltType::Expr(TypeConstraint::Concrete(DataType::Text)))),
        "tags field must be List<Expr<Text>>, got: {tags_ty:?}"
    );
    // description → Expr<Text>
    let desc_ty = model_def_field("description").unwrap();
    assert!(
        matches!(
            desc_ty,
            SmeltType::Expr(TypeConstraint::Concrete(DataType::Text))
        ),
        "description field must be Expr<Text>, got: {desc_ty:?}"
    );
}

/// `SmeltType::ModelDef` exposes `field_type(name)` returning the
/// spec-declared type for each of the five names; returns `None` for unknown.
#[test]
fn model_def_smelt_type_round_trips_field_access() {
    let ty = SmeltType::ModelDef;
    // All five spec fields resolve.
    for field in &["name", "body", "materialization", "tags", "description"] {
        let from_static = model_def_field(field);
        assert!(
            from_static.is_some(),
            "model_def_field must return Some for '{field}'"
        );
    }
    // Unknown field returns None.
    assert!(
        model_def_field("unknown_xyz").is_none(),
        "model_def_field must return None for unknown field"
    );
    // Verify ModelDef equality with itself.
    assert_eq!(ty, SmeltType::ModelDef, "ModelDef must equal itself");
}

/// `SmeltType::ModelDef` does not unify with a structurally-identical
/// `SmeltType::Record` in either direction.
#[test]
fn model_def_is_assignment_isolated_from_record() {
    use std::collections::BTreeMap;
    // Build a Record with the same five field names and types as ModelDef.
    let mut fields = BTreeMap::new();
    fields.insert(
        "name".to_string(),
        SmeltType::Expr(TypeConstraint::Concrete(DataType::Text)),
    );
    fields.insert("body".to_string(), SmeltType::TableExpr(None));
    fields.insert(
        "materialization".to_string(),
        SmeltType::Expr(TypeConstraint::Concrete(DataType::Text)),
    );
    fields.insert(
        "tags".to_string(),
        SmeltType::List(Box::new(SmeltType::Expr(TypeConstraint::Concrete(
            DataType::Text,
        )))),
    );
    fields.insert(
        "description".to_string(),
        SmeltType::Expr(TypeConstraint::Concrete(DataType::Text)),
    );
    let record_twin = SmeltType::Record { fields, name: None };

    // ModelDef != Record even with identical fields.
    assert_ne!(
        SmeltType::ModelDef,
        record_twin,
        "ModelDef must not equal a structurally-identical Record"
    );
    // Subtype checks in both directions must fail.
    assert!(
        !is_subtype_of(&SmeltType::ModelDef, &record_twin),
        "ModelDef must NOT be a subtype of a structurally-identical Record"
    );
    assert!(
        !is_subtype_of(&record_twin, &SmeltType::ModelDef),
        "Record must NOT be a subtype of ModelDef"
    );
}

/// The `body` field in `MODEL_DEF_FIELDS` is `TableExpr` — the only
/// carve-out that admits `TableExpr` in a record-like field position.
#[test]
fn model_def_admits_table_expr_in_body_field() {
    let body_ty = model_def_field("body").unwrap();
    assert!(
        matches!(body_ty, SmeltType::TableExpr(None)),
        "body field in MODEL_DEF_FIELDS must be TableExpr(None); got: {body_ty:?}"
    );
}

/// `SmeltType::ModelDef` is meta-only and not a data-world type.
#[test]
fn model_def_is_meta_only_does_not_reach_data_world() {
    assert!(
        is_meta_only_type(&SmeltType::ModelDef),
        "ModelDef must be meta-only"
    );
    assert!(
        !is_data_world_type(&SmeltType::ModelDef),
        "ModelDef must NOT be a data-world type"
    );
}

// === C26 lock-in: signature nullability (bare = nullable, NOT NULL = opt-in) ===

/// A bare type annotation (`Expr<Integer>`) produces `not_null = false` on the
/// parameter — bare annotations are nullable, NOT NULL is the opt-in (C26, §11).
#[test]
fn bare_annotation_is_nullable() {
    let (file, text) = parse_file("smelt.define f(x: Expr<Integer>) -> Expr<Integer> AS (x + 1)");
    let sigs = extract_function_signatures(&file, &text);
    assert_eq!(sigs.len(), 1);
    assert!(
            !sigs[0].params[0].not_null,
            "bare Expr<Integer> annotation must have not_null=false (nullable by default); got not_null=true"
        );
    assert!(
        !sigs[0].return_not_null,
        "bare Expr<Integer> return annotation must have return_not_null=false; got true"
    );
}

/// A `NOT NULL` qualifier on a parameter annotation sets `not_null = true`
/// — the opt-in mechanism for non-nullable signatures (C26, §11).
#[test]
fn not_null_annotation_opts_in() {
    let (file, text) = parse_file(
        "smelt.define f(x: Expr<Integer NOT NULL>) -> Expr<Integer NOT NULL> AS (x + 1)",
    );
    let sigs = extract_function_signatures(&file, &text);
    assert_eq!(sigs.len(), 1);
    assert!(
        sigs[0].params[0].not_null,
        "Expr<Integer NOT NULL> annotation must have not_null=true; got false"
    );
    assert!(
        sigs[0].return_not_null,
        "Expr<Integer NOT NULL> return annotation must have return_not_null=true; got false"
    );
}
