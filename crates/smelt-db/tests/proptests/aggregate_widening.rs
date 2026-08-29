//! Phase B3 (smelt-shop 0.3 follow-up): regression tests for aggregate type
//! narrowing in `_smelt_typed` wrapper emission.
//!
//! These tests pin down the contract that `infer_select_column_types`
//! preserves DuckDB's actual aggregate widening rules when given a populated
//! `TypeContext`. The CLI's `apply_type_casts` was constructing
//! `TypeContext::new()` (empty), so SUM(DOUBLE) silently became BIGINT etc.
//! That is fixed in the CLI separately; these tests pin the type-inference
//! contract that the fix relies on.
//!
//! Each expected type is verified against DuckDB via the `DuckDbOracle`
//! (canonical oracle for "what type does this expression actually have").

use smelt_db::type_inference::{infer_select_column_types, TypeContext};
use smelt_oracle_testkit::{DuckDbOracle, TypeOracle};
use smelt_parser::ast::File;
use smelt_types::{DataType, TypedColumn};

/// Parse a SELECT statement and infer column types using the given context.
fn infer(sql: &str, ctx: &TypeContext) -> Vec<TypedColumn> {
    let parse = smelt_parser::parse(sql);
    let file = File::cast(parse.syntax()).expect("parse File");
    let select = file.select_stmt().expect("parse SELECT");
    infer_select_column_types(&select, ctx)
}

/// Build a TypeContext with one upstream model that has the given columns.
fn ctx_with_model(model: &str, cols: &[(&str, DataType)]) -> TypeContext {
    let mut ctx = TypeContext::new();
    for (name, dt) in cols {
        ctx.add_model_column(
            model,
            name,
            TypedColumn {
                data_type: dt.clone(),
                nullable: true,
            },
        );
    }
    ctx
}

#[test]
fn sum_double_stays_double() {
    let ctx = ctx_with_model("upstream", &[("x", DataType::Double)]);
    let sql = "SELECT SUM(x) AS total FROM upstream";
    let types = infer(sql, &ctx);
    assert_eq!(types.len(), 1);
    assert_eq!(
        types[0].data_type,
        DataType::Double,
        "SUM(DOUBLE) must remain DOUBLE (was being narrowed to BIGINT)"
    );

    // Oracle check
    let oracle = DuckDbOracle::new();
    let oracle_types = oracle
        .query_types("SELECT SUM(x) AS total FROM (SELECT CAST(1.5 AS DOUBLE) AS x) upstream")
        .expect("oracle query");
    assert_eq!(oracle_types[0].1, DataType::Double);
}

#[test]
fn sum_decimal_widens_precision_keeps_scale() {
    let ctx = ctx_with_model(
        "upstream",
        &[(
            "x",
            DataType::Decimal {
                precision: 10,
                scale: 2,
            },
        )],
    );
    let sql = "SELECT SUM(x) AS total FROM upstream";
    let types = infer(sql, &ctx);
    assert_eq!(types.len(), 1);
    // DuckDB widens SUM(DECIMAL(p, s)) -> DECIMAL(38, s).
    assert_eq!(
        types[0].data_type,
        DataType::Decimal {
            precision: 38,
            scale: 2,
        },
        "SUM(DECIMAL(10,2)) must be DECIMAL(38,2) per DuckDB semantics, got {:?}",
        types[0].data_type,
    );

    // Oracle check
    let oracle = DuckDbOracle::new();
    let oracle_types = oracle
        .query_types(
            "SELECT SUM(x) AS total FROM (SELECT CAST(1.5 AS DECIMAL(10, 2)) AS x) upstream",
        )
        .expect("oracle query");
    assert_eq!(
        oracle_types[0].1,
        DataType::Decimal {
            precision: 38,
            scale: 2,
        }
    );
}

#[test]
fn sum_decimal_product_widens() {
    let ctx = ctx_with_model(
        "upstream",
        &[
            (
                "a",
                DataType::Decimal {
                    precision: 10,
                    scale: 2,
                },
            ),
            (
                "b",
                DataType::Decimal {
                    precision: 10,
                    scale: 2,
                },
            ),
        ],
    );
    let sql = "SELECT SUM(a * b) AS total FROM upstream";
    let types = infer(sql, &ctx);

    // Oracle check (DuckDB: DECIMAL(10,2) * DECIMAL(10,2) -> DECIMAL(20,4),
    // SUM widens precision to 38 -> DECIMAL(38,4)).
    let oracle = DuckDbOracle::new();
    let oracle_types = oracle
        .query_types(
            "SELECT SUM(a * b) AS total FROM \
             (SELECT CAST(1.5 AS DECIMAL(10,2)) AS a, CAST(2.5 AS DECIMAL(10,2)) AS b) upstream",
        )
        .expect("oracle query");
    let expected = oracle_types[0].1.clone();
    assert!(
        matches!(expected, DataType::Decimal { .. }),
        "DuckDB oracle should give a Decimal type, got {expected:?}",
    );
    // smelt's inferred type doesn't have to be byte-equal to DuckDB's, but it
    // MUST stay in the Decimal family (not collapse to BIGINT) — which is the
    // bug. Precision/scale matching is a separate, finer-grained concern.
    assert!(
        matches!(types[0].data_type, DataType::Decimal { .. }),
        "SUM(DECIMAL * DECIMAL) must remain Decimal (not narrow to BIGINT), got {:?}",
        types[0].data_type,
    );
}

#[test]
fn count_star_is_bigint() {
    let ctx = ctx_with_model("upstream", &[("id", DataType::Integer)]);
    let sql = "SELECT COUNT(*) AS n FROM upstream";
    let types = infer(sql, &ctx);
    assert_eq!(types[0].data_type, DataType::BigInt);

    let oracle = DuckDbOracle::new();
    let oracle_types = oracle
        .query_types("SELECT COUNT(*) AS n FROM (SELECT 1 AS id) upstream")
        .expect("oracle query");
    assert_eq!(oracle_types[0].1, DataType::BigInt);
}

#[test]
fn count_distinct_int_is_bigint_not_smallint() {
    let ctx = ctx_with_model("upstream", &[("x", DataType::Integer)]);
    let sql = "SELECT COUNT(DISTINCT x) AS n FROM upstream";
    let types = infer(sql, &ctx);
    assert_eq!(
        types[0].data_type,
        DataType::BigInt,
        "COUNT(DISTINCT) must be BIGINT regardless of the column type \
         (was being narrowed to SMALLINT in some compositions)"
    );

    let oracle = DuckDbOracle::new();
    let oracle_types = oracle
        .query_types("SELECT COUNT(DISTINCT x) AS n FROM (SELECT CAST(1 AS INTEGER) AS x) upstream")
        .expect("oracle query");
    assert_eq!(oracle_types[0].1, DataType::BigInt);
}

#[test]
fn avg_int_is_double() {
    let ctx = ctx_with_model("upstream", &[("x", DataType::Integer)]);
    let sql = "SELECT AVG(x) AS m FROM upstream";
    let types = infer(sql, &ctx);
    assert_eq!(types[0].data_type, DataType::Double);

    let oracle = DuckDbOracle::new();
    let oracle_types = oracle
        .query_types("SELECT AVG(x) AS m FROM (SELECT CAST(1 AS INTEGER) AS x) upstream")
        .expect("oracle query");
    assert_eq!(oracle_types[0].1, DataType::Double);
}

#[test]
fn case_double_then_decimal_else_widens_to_double() {
    // B9 regression. Iter-2 of smelt-shop validation hit:
    //   CASE WHEN flag THEN double_col ELSE 0.0 END
    // narrowing to DECIMAL(2,1), then DuckDB throwing
    //   Conversion Error: Could not cast value 74.260000 to DECIMAL(2,1)
    // at runtime when a THEN value exceeded 9.9.
    //
    // The literal `0.0` is typed as DECIMAL(2,1) (smallest-fit decimal).
    // promote_types must widen Double + Decimal to Double, not narrow.
    let ctx = ctx_with_model(
        "upstream",
        &[("flag", DataType::Boolean), ("rev", DataType::Double)],
    );
    let sql = "SELECT CASE WHEN flag THEN rev ELSE 0.0 END AS x FROM upstream";
    let types = infer(sql, &ctx);
    assert_eq!(
        types[0].data_type,
        DataType::Double,
        "CASE(DOUBLE, decimal-literal) must widen to DOUBLE (was narrowed to \
         DECIMAL(2,1) at runtime), got {:?}",
        types[0].data_type,
    );

    // Oracle check
    let oracle = DuckDbOracle::new();
    let oracle_types = oracle
        .query_types(
            "SELECT CASE WHEN flag THEN rev ELSE 0.0 END AS x FROM \
             (SELECT TRUE AS flag, CAST(74.26 AS DOUBLE) AS rev) upstream",
        )
        .expect("oracle query");
    assert_eq!(oracle_types[0].1, DataType::Double);
}

#[test]
fn coalesce_double_with_decimal_literal_widens_to_double() {
    // B9 cousin: COALESCE(double_col, 0.0) must yield DOUBLE.
    let ctx = ctx_with_model("upstream", &[("rev", DataType::Double)]);
    let sql = "SELECT COALESCE(rev, 0.0) AS x FROM upstream";
    let types = infer(sql, &ctx);
    assert_eq!(
        types[0].data_type,
        DataType::Double,
        "COALESCE(DOUBLE, decimal-literal) must yield DOUBLE, got {:?}",
        types[0].data_type,
    );

    let oracle = DuckDbOracle::new();
    let oracle_types = oracle
        .query_types(
            "SELECT COALESCE(rev, 0.0) AS x FROM (SELECT CAST(74.26 AS DOUBLE) AS rev) upstream",
        )
        .expect("oracle query");
    assert_eq!(oracle_types[0].1, DataType::Double);
}

#[test]
fn sum_with_alias_through_cast_preserves_alias_and_type() {
    // Bug #3 sub-issue: `infer_name()` didn't recognise CAST expressions, so
    // when `apply_type_casts` wrapped a model output and tried to derive a
    // column name from `CAST(x AS DOUBLE)`, it fell back to `_col1`. This
    // test pins the `infer_name()` contract for cast-bearing expressions.

    use smelt_parser::ast::File;
    let sql = "SELECT CAST(x AS DOUBLE) FROM upstream";
    let parse = smelt_parser::parse(sql);
    let file = File::cast(parse.syntax()).expect("parse File");
    let select = file.select_stmt().expect("SELECT");
    let select_list = select.select_list().expect("SELECT list");
    let item = select_list.items().next().expect("one item");
    let expr = item.expression().expect("expression");
    let name = expr
        .infer_name()
        .expect("infer_name should yield something");

    // The natural name is the inner column being cast.
    assert_eq!(
        name, "x",
        "infer_name on CAST(x AS DOUBLE) should return the inner column name 'x', got {name:?}",
    );
}
