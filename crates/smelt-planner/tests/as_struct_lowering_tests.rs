//! Phase 42 — `smelt.as_struct()` lowering tests.
//!
//! These tests exercise the relocated `as_struct_to_sql` helper directly
//! through its new home in `smelt-planner::lowering`. End-to-end SQL
//! emission through `smelt build` is deferred to a later phase: today the
//! body of a `smelt.define` function lowers to `LogicalNode::Raw`
//! placeholders (Phase 41), and threading concrete struct types through
//! the body's projection list requires the schema-aware body lowering
//! that Phase 46 will introduce. Until then, the lowering helper is the
//! production lowering path: any future SQL-emission caller will route
//! through it, so pinning byte-level output here is the contract that
//! matters.
//!
//! Tests 1 and 2 of the Phase 42 plan map directly onto the unit tests
//! below (`as_struct_lowering_emits_duckdb_struct_literal`,
//! `as_struct_lowering_emits_spark_struct_constructor`). Tests 3 and 4
//! live under `crates/smelt-cli/tests/as_struct_capability_tests.rs`
//! because they exercise diagnostic emission against full
//! `Database`/`Workspace` setups. Test 5 is covered by the unchanged
//! Phase 38 `backend_without_struct_literal_errors` test in
//! `crates/smelt-db/tests/as_struct_tests.rs`.

use smelt_planner::lowering::as_struct::{as_struct_to_sql, backend_supports_struct_literal};
use smelt_types::DataType;

#[test]
fn as_struct_lowering_emits_duckdb_struct_literal() {
    // TDD test 1 (plan): `smelt.as_struct(o EXCEPT customer_id)` against the
    // `enrich_order_with_as_struct` fixture's `orders` schema
    // {order_id: BigInt, total: Numeric} (after EXCEPT customer_id) lowers
    // to a DuckDB-style brace struct literal.
    let fields = vec![
        ("order_id".to_string(), DataType::BigInt),
        ("total".to_string(), DataType::Double),
    ];
    let sql = as_struct_to_sql("o", &fields, "duckdb").expect("duckdb supports struct literals");
    assert_eq!(sql, "{'order_id': o.order_id, 'total': o.total}");
}

#[test]
fn as_struct_lowering_emits_spark_struct_constructor() {
    // TDD test 2 (plan): same call shape against the Spark backend lowers
    // to the `struct(... AS ...)` constructor form.
    let fields = vec![
        ("order_id".to_string(), DataType::BigInt),
        ("total".to_string(), DataType::Double),
    ];
    let sql = as_struct_to_sql("o", &fields, "spark").expect("spark supports struct literals");
    assert_eq!(sql, "struct(o.order_id AS order_id, o.total AS total)");
}

#[test]
fn as_struct_lowering_relocated_from_db() {
    // Regression test: `as_struct_to_sql` and `backend_supports_struct_literal`
    // are reachable from `smelt-planner::lowering` and the smelt-db
    // re-exports (used by Phase 38's tests in `crates/smelt-db/tests/`)
    // resolve to the same function pointers.
    assert!(backend_supports_struct_literal("duckdb"));
    assert!(backend_supports_struct_literal("spark"));
    assert!(!backend_supports_struct_literal("no_struct_db"));

    let fields = vec![("x".to_string(), DataType::BigInt)];
    let planner_sql = as_struct_to_sql("t", &fields, "duckdb").unwrap();
    let db_sql = smelt_db::function_body_check::as_struct_to_sql("t", &fields, "duckdb").unwrap();
    assert_eq!(planner_sql, db_sql);
}
