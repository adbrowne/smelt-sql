//! Phase 58 regression test: ensure `EXTRACT(part FROM expr) AS alias` is
//! handled correctly across the smelt-db semantic-analysis pipeline.
//!
//! Specifically, the proptest harness uses three things:
//!  1. `SelectItem::alias()` to recover the synthesized `expr_N` alias.
//!  2. `infer_select_column_types(...)` to compute per-column types.
//!  3. The outer `FROM data` clause must still resolve correctly even though
//!     the EXTRACT body contains a `FROM` keyword.
//!
//! When this test was first written under Phase 58, all three pieces were
//! actually working — see the report in `docs/plans/20260422-smelt-functions.md`
//! Phase 58. The test is retained as an explicit regression to lock in
//! that invariant before re-enabling `ExprKind::Extract` and
//! `ExprKind::MakeTemporal` in the proptest strategy.

use smelt_db::type_inference::{infer_select_column_types, TypeContext};
use smelt_parser::ast::File;
use smelt_types::{DataType, TypedColumn};

#[test]
fn extract_year_select_item_alias_is_recovered() {
    let sql = "WITH data AS (SELECT CAST('2024-01-01' AS TIMESTAMP) AS dt) \
               SELECT EXTRACT(YEAR FROM dt) AS expr_0 FROM data";
    let parse = smelt_parser::parse(sql);
    assert!(parse.errors.is_empty(), "parse errors: {:?}", parse.errors);

    let file = File::cast(parse.syntax()).expect("should parse as File");
    let select_stmt = file.select_stmt().expect("should have outer SELECT");
    let select_list = select_stmt.select_list().expect("should have select list");
    let items: Vec<_> = select_list.items().collect();
    assert_eq!(items.len(), 1, "expected exactly one select item");

    let alias = items[0].alias();
    assert_eq!(
        alias.as_deref(),
        Some("expr_0"),
        "alias extraction should recover expr_0 from EXTRACT(YEAR FROM dt) AS expr_0; got {:?}",
        alias
    );

    // The outer FROM clause should still be resolved (i.e., not consumed by EXTRACT).
    assert!(
        select_stmt.from_clause().is_some(),
        "outer SELECT should still have a FROM clause"
    );
}

#[test]
fn extract_year_infers_bigint() {
    let sql = "WITH data AS (SELECT CAST('2024-01-01' AS TIMESTAMP) AS dt) \
               SELECT EXTRACT(YEAR FROM dt) AS expr_0 FROM data";
    let parse = smelt_parser::parse(sql);
    assert!(parse.errors.is_empty(), "parse errors: {:?}", parse.errors);

    let file = File::cast(parse.syntax()).expect("should parse as File");
    let select_stmt = file.select_stmt().expect("should have outer SELECT");

    let mut ctx = TypeContext::new();
    ctx.add_cte_column(
        "data",
        "dt",
        TypedColumn::nullable(DataType::Timestamp {
            with_timezone: false,
        }),
    );

    let column_types = infer_select_column_types(&select_stmt, &ctx);
    assert_eq!(column_types.len(), 1);
    assert_eq!(
        column_types[0].data_type,
        DataType::BigInt,
        "EXTRACT(YEAR FROM dt) should infer as BigInt; got {:?}",
        column_types[0].data_type
    );
}

#[test]
fn extract_epoch_infers_double() {
    let sql = "WITH data AS (SELECT CAST('2024-01-01' AS TIMESTAMP) AS dt) \
               SELECT EXTRACT(EPOCH FROM dt) AS expr_0 FROM data";
    let parse = smelt_parser::parse(sql);
    assert!(parse.errors.is_empty(), "parse errors: {:?}", parse.errors);

    let file = File::cast(parse.syntax()).expect("should parse as File");
    let select_stmt = file.select_stmt().expect("should have outer SELECT");

    let mut ctx = TypeContext::new();
    ctx.add_cte_column(
        "data",
        "dt",
        TypedColumn::nullable(DataType::Timestamp {
            with_timezone: false,
        }),
    );

    let column_types = infer_select_column_types(&select_stmt, &ctx);
    assert_eq!(column_types.len(), 1);
    assert_eq!(
        column_types[0].data_type,
        DataType::Double,
        "EXTRACT(EPOCH FROM dt) should infer as Double; got {:?}",
        column_types[0].data_type
    );
}

#[test]
fn make_date_infers_date() {
    let sql = "WITH data AS (SELECT 2024 AS y) \
               SELECT MAKE_DATE(y, 1, 1) AS expr_0 FROM data";
    let parse = smelt_parser::parse(sql);
    assert!(parse.errors.is_empty(), "parse errors: {:?}", parse.errors);

    let file = File::cast(parse.syntax()).expect("should parse as File");
    let select_stmt = file.select_stmt().expect("should have outer SELECT");

    let mut ctx = TypeContext::new();
    ctx.add_cte_column("data", "y", TypedColumn::nullable(DataType::Integer));

    let column_types = infer_select_column_types(&select_stmt, &ctx);
    assert_eq!(column_types.len(), 1);
    assert_eq!(
        column_types[0].data_type,
        DataType::Date,
        "MAKE_DATE(y, 1, 1) should infer as Date; got {:?}",
        column_types[0].data_type
    );
}

#[test]
fn make_timestamp_infers_timestamp() {
    let sql = "WITH data AS (SELECT 2024 AS y) \
               SELECT MAKE_TIMESTAMP(y, 1, 1, 0, 0, 0) AS expr_0 FROM data";
    let parse = smelt_parser::parse(sql);
    assert!(parse.errors.is_empty(), "parse errors: {:?}", parse.errors);

    let file = File::cast(parse.syntax()).expect("should parse as File");
    let select_stmt = file.select_stmt().expect("should have outer SELECT");

    let mut ctx = TypeContext::new();
    ctx.add_cte_column("data", "y", TypedColumn::nullable(DataType::Integer));

    let column_types = infer_select_column_types(&select_stmt, &ctx);
    assert_eq!(column_types.len(), 1);
    assert_eq!(
        column_types[0].data_type,
        DataType::Timestamp {
            with_timezone: false
        },
        "MAKE_TIMESTAMP(y, 1, 1, 0, 0, 0) should infer as Timestamp; got {:?}",
        column_types[0].data_type
    );
}
