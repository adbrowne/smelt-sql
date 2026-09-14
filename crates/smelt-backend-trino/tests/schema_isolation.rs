//! Offline unit tests for `common::unique_schema`'s naming rule
//! (`docs/outcomes/20260913-trino-incremental/phases/03e-plan.md`): every
//! call within one process yields a distinct, legal Trino identifier. No
//! live tier needed — deliberately free of every live-gate marker
//! (`live_env_or_skip(`, `trino_env(`, `targets_to_run_with_trino(`, a raw
//! `SMELT_TRINO_URL` read) so `trino_ci_wiring.rs`'s derived census
//! classifies this binary as offline and never demands a CI tier entry for
//! it.

mod common;

use std::collections::HashSet;

/// Trino has no documented hard identifier-length cap, but every connector
/// this repo targets (Iceberg via Hive Metastore/REST) is comfortable well
/// under 128 bytes; this is the budget the generator is held to.
const MAX_IDENTIFIER_LEN: usize = 128;

/// Shared by [`unique_schema_names_never_repeat_for_one_suffix`]'s sibling
/// assertion and duplicated verbatim in
/// `crates/smelt-cli/tests/trino_ci_wiring.rs` so both naming rules
/// (`trino_schema` and `unique_schema`) are held to the identical check and
/// cannot drift apart.
fn assert_legal_trino_identifier(name: &str) {
    assert!(
        !name.is_empty() && name.len() <= MAX_IDENTIFIER_LEN,
        "schema name {name:?} must be 1..={MAX_IDENTIFIER_LEN} bytes long"
    );
    let first = name.chars().next().unwrap();
    assert!(
        first.is_ascii_alphabetic(),
        "schema name {name:?} must start with an ASCII letter, found {first:?}"
    );
    assert!(
        name.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
        "schema name {name:?} must contain only [a-z0-9_]"
    );
}

#[test]
fn unique_schema_names_never_repeat_for_one_suffix() {
    let names: Vec<String> = (0..1000).map(|_| common::unique_schema("cap")).collect();
    let unique: HashSet<&String> = names.iter().collect();
    assert_eq!(
        unique.len(),
        names.len(),
        "1000 calls to unique_schema(\"cap\") must yield 1000 distinct names"
    );
}

#[test]
fn a_generated_schema_name_is_a_legal_lowercase_identifier() {
    for _ in 0..1000 {
        assert_legal_trino_identifier(&common::unique_schema("cap"));
    }
}
