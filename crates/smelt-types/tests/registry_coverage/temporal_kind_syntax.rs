use super::*;

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

#[test]
fn date_add_and_date_sub_are_ordinary_calls() {
    // Phase 9: both names are ordinary two-argument calls on the callable
    // surface — nothing in production consumes their `SyntaxForm::Special`
    // classification (`binary.rs` types the infix interval add/sub itself),
    // so the registry-consistency gate's `SyntaxForm` exemption must no
    // longer cover them.
    for name in ["DATE_ADD", "DATE_SUB"] {
        let sig = BuiltinRegistry::resolve(name).expect(name);
        assert_eq!(
            sig.syntax_form,
            SyntaxForm::Call,
            "{name} must be an ordinary callable, not dedicated syntax"
        );
    }
}
