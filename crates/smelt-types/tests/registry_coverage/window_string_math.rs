use super::*;

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
