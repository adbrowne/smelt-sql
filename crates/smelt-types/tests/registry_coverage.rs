/// Phase 50: registry coverage tests — verify that newly-seeded built-ins
/// are present and carry the correct `ExprKind`.
use smelt_types::{
    signatures::{
        ConditionalArm, Emission, ExprKind, Position, RewriteId, SigParam, Signature, SyntaxForm,
        TemplateError,
    },
    validate_conditional, validate_template, BuiltinRegistry, CallFacts, ConditionalError,
    DataType, DialectId, OperandClass, SettledEmission, TypeConstraint, TypeExpr,
};

/// A two-parameter integer signature — only its params/kind matter to the
/// `validate_template` tests below.
fn two_arg_signature() -> Signature {
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

fn one_arg_signature() -> Signature {
    Signature::new(
        "TEST_ONE_ARG",
        vec![],
        vec![SigParam::Concrete(TypeConstraint::Concrete(
            DataType::Integer,
        ))],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Integer)),
    )
}

fn variadic_signature() -> Signature {
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
fn test_signature(name: &str) -> Signature {
    Signature::new(
        name,
        vec![],
        vec![],
        TypeExpr::Concrete(TypeConstraint::Concrete(DataType::Integer)),
    )
}

// ─── Operators ──────────────────────────────────────────────────────────────

#[test]
fn operator_like_registered() {
    assert!(
        BuiltinRegistry::resolve("LIKE").is_some(),
        "LIKE not in registry"
    );
}

#[test]
fn operator_ilike_registered() {
    assert!(
        BuiltinRegistry::resolve("ILIKE").is_some(),
        "ILIKE not in registry"
    );
}

#[test]
fn operator_glob_registered() {
    assert!(
        BuiltinRegistry::resolve("GLOB").is_some(),
        "GLOB not in registry"
    );
}

#[test]
fn operator_is_null_registered() {
    assert!(
        BuiltinRegistry::resolve("IS_NULL").is_some(),
        "IS_NULL not in registry"
    );
}

#[test]
fn operator_is_not_null_registered() {
    assert!(
        BuiltinRegistry::resolve("IS_NOT_NULL").is_some(),
        "IS_NOT_NULL not in registry"
    );
}

#[test]
fn operator_between_registered() {
    assert!(
        BuiltinRegistry::resolve("BETWEEN").is_some(),
        "BETWEEN not in registry"
    );
}

#[test]
fn operator_in_registered() {
    assert!(
        BuiltinRegistry::resolve("IN").is_some(),
        "IN not in registry"
    );
}

#[test]
fn operator_exists_registered() {
    assert!(
        BuiltinRegistry::resolve("EXISTS").is_some(),
        "EXISTS not in registry"
    );
}

#[test]
fn operator_cast_registered() {
    assert!(
        BuiltinRegistry::resolve("CAST").is_some(),
        "CAST not in registry"
    );
}

// ─── Aggregates ─────────────────────────────────────────────────────────────

#[test]
fn agg_string_agg_registered() {
    assert!(
        BuiltinRegistry::resolve("STRING_AGG").is_some(),
        "STRING_AGG not in registry"
    );
}

#[test]
fn agg_listagg_registered() {
    assert!(
        BuiltinRegistry::resolve("LISTAGG").is_some(),
        "LISTAGG not in registry"
    );
}

#[test]
fn agg_array_agg_registered() {
    assert!(
        BuiltinRegistry::resolve("ARRAY_AGG").is_some(),
        "ARRAY_AGG not in registry"
    );
}

#[test]
fn agg_median_registered() {
    assert!(
        BuiltinRegistry::resolve("MEDIAN").is_some(),
        "MEDIAN not in registry"
    );
}

#[test]
fn agg_stddev_registered() {
    assert!(
        BuiltinRegistry::resolve("STDDEV").is_some(),
        "STDDEV not in registry"
    );
}

#[test]
fn agg_stddev_pop_registered() {
    assert!(
        BuiltinRegistry::resolve("STDDEV_POP").is_some(),
        "STDDEV_POP not in registry"
    );
}

#[test]
fn agg_stddev_samp_registered() {
    assert!(
        BuiltinRegistry::resolve("STDDEV_SAMP").is_some(),
        "STDDEV_SAMP not in registry"
    );
}

#[test]
fn agg_variance_registered() {
    assert!(
        BuiltinRegistry::resolve("VARIANCE").is_some(),
        "VARIANCE not in registry"
    );
}

#[test]
fn agg_var_pop_registered() {
    assert!(
        BuiltinRegistry::resolve("VAR_POP").is_some(),
        "VAR_POP not in registry"
    );
}

#[test]
fn agg_var_samp_registered() {
    assert!(
        BuiltinRegistry::resolve("VAR_SAMP").is_some(),
        "VAR_SAMP not in registry"
    );
}

#[test]
fn agg_bool_and_registered() {
    assert!(
        BuiltinRegistry::resolve("BOOL_AND").is_some(),
        "BOOL_AND not in registry"
    );
}

#[test]
fn agg_bool_or_registered() {
    assert!(
        BuiltinRegistry::resolve("BOOL_OR").is_some(),
        "BOOL_OR not in registry"
    );
}

#[test]
fn agg_bit_and_registered() {
    assert!(
        BuiltinRegistry::resolve("BIT_AND").is_some(),
        "BIT_AND not in registry"
    );
}

#[test]
fn agg_bit_or_registered() {
    assert!(
        BuiltinRegistry::resolve("BIT_OR").is_some(),
        "BIT_OR not in registry"
    );
}

#[test]
fn agg_bit_xor_registered() {
    assert!(
        BuiltinRegistry::resolve("BIT_XOR").is_some(),
        "BIT_XOR not in registry"
    );
}

#[test]
fn agg_any_value_registered() {
    assert!(
        BuiltinRegistry::resolve("ANY_VALUE").is_some(),
        "ANY_VALUE not in registry"
    );
}

#[test]
fn agg_arg_max_registered() {
    assert!(
        BuiltinRegistry::resolve("ARG_MAX").is_some(),
        "ARG_MAX not in registry"
    );
}

#[test]
fn agg_approx_count_distinct_registered() {
    assert!(
        BuiltinRegistry::resolve("APPROX_COUNT_DISTINCT").is_some(),
        "APPROX_COUNT_DISTINCT not in registry"
    );
}

// ─── Window functions ────────────────────────────────────────────────────────

#[test]
fn window_ntile_registered() {
    assert!(
        BuiltinRegistry::resolve("NTILE").is_some(),
        "NTILE not in registry"
    );
}

#[test]
fn window_first_value_registered() {
    assert!(
        BuiltinRegistry::resolve("FIRST_VALUE").is_some(),
        "FIRST_VALUE not in registry"
    );
}

#[test]
fn window_last_value_registered() {
    assert!(
        BuiltinRegistry::resolve("LAST_VALUE").is_some(),
        "LAST_VALUE not in registry"
    );
}

#[test]
fn window_nth_value_registered() {
    assert!(
        BuiltinRegistry::resolve("NTH_VALUE").is_some(),
        "NTH_VALUE not in registry"
    );
}

#[test]
fn window_cume_dist_registered() {
    assert!(
        BuiltinRegistry::resolve("CUME_DIST").is_some(),
        "CUME_DIST not in registry"
    );
}

#[test]
fn window_percent_rank_registered() {
    assert!(
        BuiltinRegistry::resolve("PERCENT_RANK").is_some(),
        "PERCENT_RANK not in registry"
    );
}

// ─── String scalars ──────────────────────────────────────────────────────────

#[test]
fn string_ltrim_registered() {
    assert!(
        BuiltinRegistry::resolve("LTRIM").is_some(),
        "LTRIM not in registry"
    );
}

#[test]
fn string_rtrim_registered() {
    assert!(
        BuiltinRegistry::resolve("RTRIM").is_some(),
        "RTRIM not in registry"
    );
}

#[test]
fn string_char_length_registered() {
    assert!(
        BuiltinRegistry::resolve("CHAR_LENGTH").is_some(),
        "CHAR_LENGTH not in registry"
    );
}

#[test]
fn string_character_length_registered() {
    assert!(
        BuiltinRegistry::resolve("CHARACTER_LENGTH").is_some(),
        "CHARACTER_LENGTH not in registry"
    );
}

#[test]
fn string_replace_registered() {
    assert!(
        BuiltinRegistry::resolve("REPLACE").is_some(),
        "REPLACE not in registry"
    );
}

#[test]
fn string_lpad_registered() {
    assert!(
        BuiltinRegistry::resolve("LPAD").is_some(),
        "LPAD not in registry"
    );
}

#[test]
fn string_rpad_registered() {
    assert!(
        BuiltinRegistry::resolve("RPAD").is_some(),
        "RPAD not in registry"
    );
}

#[test]
fn string_repeat_registered() {
    assert!(
        BuiltinRegistry::resolve("REPEAT").is_some(),
        "REPEAT not in registry"
    );
}

#[test]
fn string_substr_registered() {
    assert!(
        BuiltinRegistry::resolve("SUBSTR").is_some(),
        "SUBSTR not in registry"
    );
}

#[test]
fn string_split_part_registered() {
    assert!(
        BuiltinRegistry::resolve("SPLIT_PART").is_some(),
        "SPLIT_PART not in registry"
    );
}

#[test]
fn string_strpos_registered() {
    assert!(
        BuiltinRegistry::resolve("STRPOS").is_some(),
        "STRPOS not in registry"
    );
}

#[test]
fn string_left_registered() {
    assert!(
        BuiltinRegistry::resolve("LEFT").is_some(),
        "LEFT not in registry"
    );
}

#[test]
fn string_right_registered() {
    assert!(
        BuiltinRegistry::resolve("RIGHT").is_some(),
        "RIGHT not in registry"
    );
}

// ─── Math scalars ────────────────────────────────────────────────────────────

#[test]
fn math_exp_registered() {
    assert!(
        BuiltinRegistry::resolve("EXP").is_some(),
        "EXP not in registry"
    );
}

#[test]
fn math_log10_registered() {
    assert!(
        BuiltinRegistry::resolve("LOG10").is_some(),
        "LOG10 not in registry"
    );
}

#[test]
fn math_log2_registered() {
    assert!(
        BuiltinRegistry::resolve("LOG2").is_some(),
        "LOG2 not in registry"
    );
}

#[test]
fn math_mod_registered() {
    assert!(
        BuiltinRegistry::resolve("MOD").is_some(),
        "MOD not in registry"
    );
}

#[test]
fn math_sign_registered() {
    assert!(
        BuiltinRegistry::resolve("SIGN").is_some(),
        "SIGN not in registry"
    );
}

#[test]
fn math_sin_registered() {
    assert!(
        BuiltinRegistry::resolve("SIN").is_some(),
        "SIN not in registry"
    );
}

#[test]
fn math_cos_registered() {
    assert!(
        BuiltinRegistry::resolve("COS").is_some(),
        "COS not in registry"
    );
}

#[test]
fn math_tan_registered() {
    assert!(
        BuiltinRegistry::resolve("TAN").is_some(),
        "TAN not in registry"
    );
}

#[test]
fn math_atan_registered() {
    assert!(
        BuiltinRegistry::resolve("ATAN").is_some(),
        "ATAN not in registry"
    );
}

#[test]
fn math_atan2_registered() {
    assert!(
        BuiltinRegistry::resolve("ATAN2").is_some(),
        "ATAN2 not in registry"
    );
}

#[test]
fn math_sinh_registered() {
    assert!(
        BuiltinRegistry::resolve("SINH").is_some(),
        "SINH not in registry"
    );
}

#[test]
fn math_cosh_registered() {
    assert!(
        BuiltinRegistry::resolve("COSH").is_some(),
        "COSH not in registry"
    );
}

#[test]
fn math_tanh_registered() {
    assert!(
        BuiltinRegistry::resolve("TANH").is_some(),
        "TANH not in registry"
    );
}

#[test]
fn math_pi_registered() {
    assert!(
        BuiltinRegistry::resolve("PI").is_some(),
        "PI not in registry"
    );
}

// ─── Temporal scalars ────────────────────────────────────────────────────────

#[test]
fn temporal_date_part_registered() {
    assert!(
        BuiltinRegistry::resolve("DATE_PART").is_some(),
        "DATE_PART not in registry"
    );
}

#[test]
fn temporal_date_add_registered() {
    assert!(
        BuiltinRegistry::resolve("DATE_ADD").is_some(),
        "DATE_ADD not in registry"
    );
}

#[test]
fn temporal_date_sub_registered() {
    assert!(
        BuiltinRegistry::resolve("DATE_SUB").is_some(),
        "DATE_SUB not in registry"
    );
}

#[test]
fn temporal_make_date_registered() {
    assert!(
        BuiltinRegistry::resolve("MAKE_DATE").is_some(),
        "MAKE_DATE not in registry"
    );
}

#[test]
fn temporal_make_timestamp_registered() {
    assert!(
        BuiltinRegistry::resolve("MAKE_TIMESTAMP").is_some(),
        "MAKE_TIMESTAMP not in registry"
    );
}

#[test]
fn temporal_age_registered() {
    assert!(
        BuiltinRegistry::resolve("AGE").is_some(),
        "AGE not in registry"
    );
}

// ─── Kind checks ─────────────────────────────────────────────────────────────

#[test]
fn agg_kinds_correct() {
    let agg_names = [
        "STRING_AGG",
        "LISTAGG",
        "ARRAY_AGG",
        "MEDIAN",
        "STDDEV",
        "STDDEV_POP",
        "STDDEV_SAMP",
        "VARIANCE",
        "VAR_POP",
        "VAR_SAMP",
        "BOOL_AND",
        "BOOL_OR",
        "BIT_AND",
        "BIT_OR",
        "BIT_XOR",
        "ANY_VALUE",
        "ARG_MAX",
        "APPROX_COUNT_DISTINCT",
    ];
    for name in agg_names {
        let sig =
            BuiltinRegistry::resolve(name).unwrap_or_else(|| panic!("{name} not in registry"));
        assert_eq!(
            sig.kind,
            ExprKind::Agg,
            "{name} should have kind Agg, got {:?}",
            sig.kind
        );
    }
}

#[test]
fn window_kinds_correct() {
    let window_names = [
        "NTILE",
        "FIRST_VALUE",
        "LAST_VALUE",
        "NTH_VALUE",
        "CUME_DIST",
        "PERCENT_RANK",
    ];
    for name in window_names {
        let sig =
            BuiltinRegistry::resolve(name).unwrap_or_else(|| panic!("{name} not in registry"));
        assert_eq!(
            sig.kind,
            ExprKind::Window,
            "{name} should have kind Window, got {:?}",
            sig.kind
        );
    }
}

// ─── Syntax forms and the operator surface

#[test]
fn infix_operators_are_registered_with_the_infix_form() {
    for op in ["%", "^", "**", "//", "||"] {
        let sig =
            BuiltinRegistry::resolve(op).unwrap_or_else(|| panic!("operator {op} not in registry"));
        assert_eq!(
            sig.syntax_form,
            SyntaxForm::Infix,
            "{op} must be Infix so the audit enumerates it as an operator"
        );
    }
}

#[test]
fn table_functions_are_registered_with_the_tablefn_form() {
    for name in ["EXPLODE", "UNNEST"] {
        let sig =
            BuiltinRegistry::resolve(name).unwrap_or_else(|| panic!("{name} not in registry"));
        assert_eq!(sig.syntax_form, SyntaxForm::TableFn);
    }
}

#[test]
fn ordinary_functions_default_to_the_call_form() {
    for name in ["SUM", "LOWER", "ROW_NUMBER", "DATE_TRUNC"] {
        let sig = BuiltinRegistry::resolve(name).expect(name);
        assert_eq!(sig.syntax_form, SyntaxForm::Call);
    }
}

#[test]
fn dedicated_syntax_entries_are_not_call_form() {
    // The exemption the registry-consistency gate derives. Each of these is a
    // registry entry for hover/completion but not a callable function.
    for name in [
        "LIKE",
        "ILIKE",
        "GLOB",
        "IS_NULL",
        "IS_NOT_NULL",
        "BETWEEN",
        "IN",
        "EXISTS",
        "CAST",
        "DATE_ADD",
        "DATE_SUB",
    ] {
        let sig = BuiltinRegistry::resolve(name).expect(name);
        assert_ne!(
            sig.syntax_form,
            SyntaxForm::Call,
            "{name} is dedicated syntax; leaving it Call re-enters it into the \
             callable-function consistency gate"
        );
    }
}

// ─── Emission

#[test]
fn the_rename_matrix_matches_the_printer_it_replaces() {
    // These rows were transcribed from the printer's hand-written rename chain
    // before that chain was deleted, so the registry is provably a faithful
    // replacement for it rather than a re-derivation.
    let expected: &[(&str, DialectId, &str)] = &[
        ("EXPLODE", DialectId::DuckDb, "UNNEST"),
        ("EXPLODE", DialectId::BigQuery, "UNNEST"),
        ("UNNEST", DialectId::SparkSql, "EXPLODE"),
        ("EVERY", DialectId::DuckDb, "BOOL_AND"),
        ("EVERY", DialectId::BigQuery, "LOGICAL_AND"),
        ("BOOL_AND", DialectId::SparkSql, "EVERY"),
        ("BOOL_AND", DialectId::BigQuery, "LOGICAL_AND"),
        ("BOOL_OR", DialectId::SparkSql, "SOME"),
        ("BOOL_OR", DialectId::BigQuery, "LOGICAL_OR"),
    ];
    for (name, dialect, renamed) in expected {
        let sig = BuiltinRegistry::resolve(name).expect(name);
        assert_eq!(
            sig.emission_at(*dialect, Position::Any),
            Emission::Rename(renamed),
            "{name} on {}",
            dialect.slug()
        );
    }
}

#[test]
fn caret_is_rewritten_wherever_infix_caret_means_xor() {
    // GoogleSQL and Spark SQL both define infix `^` as bitwise XOR while smelt's
    // grammar reads it as power. Emitting it verbatim returns a different number
    // rather than failing — the silent-divergence class this work exists to close.
    for dialect in [DialectId::SparkSql, DialectId::BigQuery] {
        for op in ["^", "**"] {
            let sig = BuiltinRegistry::resolve(op).expect(op);
            assert_eq!(
                sig.emission_at(dialect, Position::Any),
                Emission::Template("POWER({0}, {1})"),
                "{op} on {}",
                dialect.slug()
            );
        }
    }
    for op in ["^", "**"] {
        let sig = BuiltinRegistry::resolve(op).expect(op);
        assert_eq!(
            sig.emission_at(DialectId::DuckDb, Position::Any),
            Emission::Native
        );
    }
}

#[test]
fn floor_divide_is_unsupported_everywhere_it_has_no_safe_lowering() {
    let sig = BuiltinRegistry::resolve("//").expect("//");
    assert_eq!(
        sig.emission_at(DialectId::DuckDb, Position::Any),
        Emission::Native
    );
    // BigQuery has no per-class lowering at all — flatly unsupported.
    assert!(
        matches!(
            sig.emission_at(DialectId::BigQuery, Position::Any),
            Emission::Unsupported { .. }
        ),
        "// on bigquery must be a declared refusal, not a pass-through"
    );
    // Spark settles per operand class (phase 7); an operand whose class
    // cannot be resolved must still land on a declared refusal, never a
    // pass-through.
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

#[test]
fn an_unlisted_dialect_defaults_to_native() {
    let sig = BuiltinRegistry::resolve("LOWER").expect("LOWER");
    for d in DialectId::ALL {
        assert_eq!(sig.emission_at(*d, Position::Any), Emission::Native);
    }
}

#[test]
fn every_declared_rewrite_id_is_reachable_from_some_entry() {
    // A RewriteId with no registry row is printer code nothing can call.
    let mut seen: Vec<RewriteId> = BuiltinRegistry::names()
        .filter_map(BuiltinRegistry::resolve)
        .flat_map(|sig| sig.emission.iter())
        .filter_map(|(_, _, e)| match e {
            Emission::Rewrite(id) => Some(*id),
            _ => None,
        })
        .collect();
    seen.sort();
    seen.dedup();
    assert_eq!(
        seen,
        vec![RewriteId::BigQueryMedian, RewriteId::WithinGroupToAnalytic],
    );
}

#[test]
fn every_declared_template_is_reachable_from_some_entry() {
    // The `%`/`^`/`**` templates migrated off `RewriteId` in this phase; this
    // is their equivalent reachability check for `Emission::Template`.
    let templates: Vec<&'static str> = BuiltinRegistry::names()
        .filter_map(BuiltinRegistry::resolve)
        .flat_map(|sig| sig.emission.iter())
        .filter_map(|(_, _, e)| match e {
            Emission::Template(t) => Some(*t),
            _ => None,
        })
        .collect();
    assert!(
        templates.contains(&"MOD({0}, {1})"),
        "expected the `%` modulo template to be registered: {templates:?}"
    );
    assert!(
        templates.contains(&"POWER({0}, {1})"),
        "expected the `^`/`**` power template to be registered: {templates:?}"
    );
}

// ─── Position ───────────────────────────────────────────────────────────────

/// `Position` has exactly the five documented variants, and `Any` is a
/// lookup wildcard no classifier ever returns — it exists only so a
/// registry entry can state one verdict that applies at every call
/// position. This match is exhaustive: adding or removing a variant is a
/// compile error here, forcing this test (and its doc comment) to be
/// updated alongside the enum.
#[test]
fn position_variants_are_exhaustive() {
    let variants = [
        Position::Any,
        Position::Scalar,
        Position::Aggregate,
        Position::WholePartitionWindow,
        Position::Window,
    ];
    for v in variants {
        match v {
            Position::Any
            | Position::Scalar
            | Position::Aggregate
            | Position::WholePartitionWindow
            | Position::Window => {}
        }
    }
    assert_eq!(
        variants.len(),
        5,
        "Position must have exactly five variants"
    );
}

/// A verdict stated at the call's own position wins over one stated only at
/// the `Any` wildcard for the same dialect — the wildcard is a default, not a
/// veto.
#[test]
fn emission_at_prefers_exact_position_over_any() {
    let sig = test_signature("TEST_PREFERS_EXACT").with_emission(&[
        (
            DialectId::BigQuery,
            Position::Any,
            Emission::Rename("ANY_SPELLING"),
        ),
        (
            DialectId::BigQuery,
            Position::Aggregate,
            Emission::Rename("AGG_SPELLING"),
        ),
    ]);
    assert_eq!(
        sig.emission_at(DialectId::BigQuery, Position::Aggregate),
        Emission::Rename("AGG_SPELLING"),
        "the exact-position entry must win over Any"
    );
    // A position with no entry of its own still falls through to Any.
    assert_eq!(
        sig.emission_at(DialectId::BigQuery, Position::Scalar),
        Emission::Rename("ANY_SPELLING"),
        "a position with no dedicated entry falls back to Any"
    );
}

/// Lookup falls from the exact position to `Any`, and from `Any` to
/// `Native` when the dialect has no entry at all — and stops there.
#[test]
fn emission_at_falls_back_to_any_then_native() {
    let sig = test_signature("TEST_FALLS_BACK").with_emission(&[(
        DialectId::BigQuery,
        Position::Any,
        Emission::Rename("X"),
    )]);
    assert_eq!(
        sig.emission_at(DialectId::BigQuery, Position::Scalar),
        Emission::Rename("X"),
        "no Scalar entry, but an Any entry exists for this dialect"
    );
    assert_eq!(
        sig.emission_at(DialectId::DuckDb, Position::Scalar),
        Emission::Native,
        "no entry at all for this dialect: Native"
    );
}

/// The finding that motivated position-scoped emission: the two window
/// positions must never answer for each other. Falling from
/// `WholePartitionWindow` to `Window` would refuse a whole-partition call
/// the restructure exists to serve; falling from `Window` to `Any` would let
/// a running window reach the backend as `Native` and fail at the warehouse.
#[test]
fn window_positions_never_fall_back_to_each_other() {
    let sig = test_signature("TEST_WINDOW_POSITIONS").with_emission(&[
        (
            DialectId::BigQuery,
            Position::WholePartitionWindow,
            Emission::Native,
        ),
        (
            DialectId::BigQuery,
            Position::Window,
            Emission::Unsupported {
                reason: "no analytic form for a running window",
            },
        ),
    ]);
    assert_eq!(
        sig.emission_at(DialectId::BigQuery, Position::WholePartitionWindow),
        Emission::Native,
        "the whole-partition verdict must not be shadowed by the running-window one"
    );
    assert!(
        matches!(
            sig.emission_at(DialectId::BigQuery, Position::Window),
            Emission::Unsupported { .. }
        ),
        "the running-window verdict must not be shadowed by the whole-partition one"
    );
}

/// Coverage-totality gate: an entry declaring a verdict at one window
/// position must declare one at the other, because there is no fallback
/// between them for the lookup to fall back on. Checked against the real
/// registry data, not a hypothetical signature, so the gate fires the moment
/// a real entry violates it.
#[test]
fn window_verdict_totality() {
    let mut violations: Vec<String> = Vec::new();
    for name in BuiltinRegistry::names() {
        let Some(sig) = BuiltinRegistry::resolve(name) else {
            continue;
        };
        for dialect in DialectId::ALL {
            let has_whole_partition = sig
                .emission
                .iter()
                .any(|(d, p, _)| *d == *dialect && *p == Position::WholePartitionWindow);
            let has_window = sig
                .emission
                .iter()
                .any(|(d, p, _)| *d == *dialect && *p == Position::Window);
            if has_whole_partition != has_window {
                let (present, missing) = if has_whole_partition {
                    ("WholePartitionWindow", "Window")
                } else {
                    ("Window", "WholePartitionWindow")
                };
                violations.push(format!(
                    "{name} on {}: declares {present} but not {missing}",
                    dialect.slug()
                ));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "window-verdict totality violated (an entry stating one window \
         position must state the other, since lookup never falls between \
         them):\n{}",
        violations.join("\n")
    );
}

// ─── Retired dialects

/// The whole `signatures` module source, concatenated — read at test time
/// rather than `include_str!`-ed one file at a time, so a new submodule
/// dropped into `src/signatures/` (or `src/signatures/builtins/`) is scanned
/// the moment it exists, never only once someone remembers to list it here.
fn signatures_src() -> String {
    fn read_dir_recursive(dir: &std::path::Path, out: &mut Vec<(std::path::PathBuf, String)>) {
        let mut entries: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
            .expect("readable signatures module directory")
            .map(|e| e.expect("readable dir entry").path())
            .collect();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                read_dir_recursive(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let src = std::fs::read_to_string(&path).expect("readable source file");
                out.push((path, src));
            }
        }
    }
    let dir = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/src/signatures"));
    let mut files = Vec::new();
    read_dir_recursive(dir, &mut files);
    assert!(
        !files.is_empty(),
        "no signatures sources found — the gate would pass vacuously"
    );
    files
        .into_iter()
        .map(|(_, src)| src)
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn no_registry_row_names_a_retired_dialect() {
    // The PostgreSQL emission dialect was retired (#181): no backend crate
    // exists to verify its verdicts, so a template or conditional-verdict
    // arm reintroducing it would carry an unverifiable claim. This scans the
    // registry source directly rather than `DialectId::ALL` (which would
    // trivially be silent about a variant that no longer compiles).
    let src = signatures_src();
    for spelling in ["PostgreSql", "PostgreSQL"] {
        assert!(
            !src.contains(spelling),
            "the signatures module names the retired dialect spelling {spelling:?}"
        );
    }
}

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
        validate_conditional(ARMS, &sig),
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
        validate_conditional(ARMS, &sig),
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
        validate_conditional(ARMS, &sig),
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
        validate_conditional(ARMS, &sig),
        Err(ConditionalError::ArgumentIndexOutOfRange {
            signature: sig.name.clone(),
            index: 3
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
        for (_, _, emission) in sig.emission.iter() {
            if let Emission::Conditional(arms) = emission {
                assert!(
                    validate_conditional(arms, sig).is_ok(),
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
