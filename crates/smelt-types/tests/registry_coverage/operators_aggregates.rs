use super::*;

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
