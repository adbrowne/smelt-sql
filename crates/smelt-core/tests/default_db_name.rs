//! Phase 2: default DB-name mapping tests.
//!
//! Per architecture.md §"Default materialization name mapping":
//!   smelt.staging.orders → main.staging_orders
//!   smelt.payments.seeds.lookup.regions → main.payments_seeds_lookup_regions
//!   smelt.users → main.users
//!
//! ## Contract: ephemeral models and `smelt.define` functions have no DB name
//!
//! `default_db_name` is a total function and must NOT be called for:
//! - Models with `materialization: ephemeral` — these are inlined as CTEs
//!   (`__smelt_<name>`) and never materialised.
//! - `smelt.define` function declarations — these are macro-expanded into CTEs,
//!   not physical tables.
//!
//! The call-site enforcement is tested at the compiler level:
//! see `crates/smelt-cli/src/compiler.rs` →
//!   `tests::ephemeral_and_define_have_no_db_name`.

use smelt_core::resolver::default_db_name;

/// join_path_components_with_underscore: path segments joined with '_'
/// and prefixed with the target schema.
#[test]
fn join_path_components_with_underscore() {
    assert_eq!(
        default_db_name(&["users".to_string()], "main"),
        "main.users"
    );
    assert_eq!(
        default_db_name(&["staging".to_string(), "orders".to_string()], "main"),
        "main.staging_orders"
    );
    assert_eq!(
        default_db_name(
            &[
                "payments".to_string(),
                "seeds".to_string(),
                "lookup".to_string(),
                "regions".to_string()
            ],
            "main"
        ),
        "main.payments_seeds_lookup_regions"
    );
}

/// Non-default schema works correctly.
#[test]
fn non_default_schema() {
    assert_eq!(
        default_db_name(&["staging".to_string(), "orders".to_string()], "analytics"),
        "analytics.staging_orders"
    );
}

/// Contract: `default_db_name` is only meaningful for materialised entities.
///
/// Ephemeral models and `smelt.define` function declarations do NOT have a
/// DB name — they are inlined as CTEs (`__smelt_<name>`) and never written to
/// a physical table.  The compiler is responsible for never passing those
/// entities through `default_db_name`.
///
/// This test documents that contract.  The behavioural enforcement (verifying
/// that compiled SQL uses `__smelt_staging_users` and not `main.staging_users`)
/// lives in `crates/smelt-cli/src/compiler.rs` →
/// `tests::ephemeral_and_define_have_no_db_name`.
#[test]
fn ephemeral_and_define_have_no_db_name() {
    // If `default_db_name` *were* called for an ephemeral / define entity it
    // would produce a name — this test asserts what that name would be, so
    // that a reader can see the invariant: calling this for an ephemeral is
    // WRONG.  The returned value should never be used as a table reference.
    let would_be_wrong = default_db_name(&["staging_users".to_string()], "main");
    // The computed name exists but MUST NOT appear in any emitted SQL for an
    // ephemeral model — see the companion compiler test for the enforcement.
    assert_eq!(
        would_be_wrong, "main.staging_users",
        "default_db_name is a total function — callers must exclude ephemeral entities"
    );
}
