/// Phase 50: registry coverage tests — verify that newly-seeded built-ins
/// are present and carry the correct `ExprKind`.
use smelt_types::{
    signatures::{Emission, ExprKind, RewriteId, SyntaxForm},
    BuiltinRegistry, DialectId,
};

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
    // Transcribed from `remap_function_name` (printer.rs:1881-1925) before its
    // deletion. This test is what makes the printer refactor mechanical.
    let expected: &[(&str, DialectId, &str)] = &[
        ("EXPLODE", DialectId::DuckDb, "UNNEST"),
        ("EXPLODE", DialectId::PostgreSql, "UNNEST"),
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
            sig.emission_for(*dialect),
            Emission::Rename(renamed),
            "{name} on {}",
            dialect.slug()
        );
    }
}

#[test]
fn every_is_native_on_postgresql() {
    // `remap_function_name` deliberately leaves PostgreSQL's EVERY alone while
    // DuckDB rewrites it; snapshots.rs:401 pins the asymmetry.
    let sig = BuiltinRegistry::resolve("EVERY").expect("EVERY");
    assert_eq!(sig.emission_for(DialectId::PostgreSql), Emission::Native);
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
                sig.emission_for(dialect),
                Emission::Rewrite(RewriteId::PowerCall),
                "{op} on {}",
                dialect.slug()
            );
        }
    }
    for op in ["^", "**"] {
        let sig = BuiltinRegistry::resolve(op).expect(op);
        assert_eq!(sig.emission_for(DialectId::DuckDb), Emission::Native);
        assert_eq!(sig.emission_for(DialectId::PostgreSql), Emission::Native);
    }
}

#[test]
fn floor_divide_is_unsupported_everywhere_it_has_no_safe_lowering() {
    let sig = BuiltinRegistry::resolve("//").expect("//");
    assert_eq!(sig.emission_for(DialectId::DuckDb), Emission::Native);
    for dialect in [
        DialectId::SparkSql,
        DialectId::PostgreSql,
        DialectId::BigQuery,
    ] {
        assert!(
            matches!(sig.emission_for(dialect), Emission::Unsupported { .. }),
            "// on {} must be a declared refusal, not a pass-through",
            dialect.slug()
        );
    }
}

#[test]
fn an_unlisted_dialect_defaults_to_native() {
    let sig = BuiltinRegistry::resolve("LOWER").expect("LOWER");
    for d in DialectId::ALL {
        assert_eq!(sig.emission_for(*d), Emission::Native);
    }
}

#[test]
fn every_declared_rewrite_id_is_reachable_from_some_entry() {
    // A RewriteId with no registry row is printer code nothing can call.
    let mut seen: Vec<RewriteId> = BuiltinRegistry::names()
        .filter_map(BuiltinRegistry::resolve)
        .flat_map(|sig| sig.emission.iter())
        .filter_map(|(_, e)| match e {
            Emission::Rewrite(id) => Some(*id),
            _ => None,
        })
        .collect();
    seen.sort();
    seen.dedup();
    assert_eq!(
        seen,
        vec![
            RewriteId::BigQueryMedian,
            RewriteId::ModuloCall,
            RewriteId::PowerCall
        ],
    );
}
