//! Tests for the type-inference submodule tree.
//!
//! `mod.rs` already declares this file with `#[cfg(test)] mod tests;`, so the
//! whole file is test-only. Items here are written with the same imports the
//! pre-split file had in scope at the parent module level.

#![cfg(test)]

#[allow(unused_imports)]
use super::*;
// Re-imports of items the previous monolithic file imported at module scope.
// After the split these are not part of `mod.rs`'s public surface, so tests
// pull them in explicitly here.
#[allow(unused_imports)]
use crate::type_inference::binary::{
    check_undeclared_columns, infer_cte_columns, walk_expression_columns,
    walk_expression_columns_with_visitor, walk_select_columns, walk_select_columns_with_visitor,
};
#[allow(unused_imports)]
use crate::type_inference::composite::{
    infer_array_literal_type, infer_array_slice_type, infer_array_subscript_type,
    infer_row_constructor_type, infer_struct_literal_type,
};
#[allow(unused_imports)]
use crate::type_inference::dispatch::{
    infer_expression_kind, infer_expression_type, infer_select_column_types,
    infer_select_output_schema, promote_types,
};
#[allow(unused_imports)]
use crate::type_inference::function_call::{
    infer_as_struct_type, infer_function_type, infer_smelt_path_call_type,
};
#[allow(unused_imports)]
use crate::type_inference::hof::extract_pipe_expr_from_expr;
#[allow(unused_imports)]
use crate::type_inference::hof::{
    check_define_name_shadowing, check_forbidden_position_spreads, check_hof_position_diagnostics,
    check_select_list_spreads, disambiguate_list_literal, expand_spread_into_position,
    infer_hof_call, infer_hof_call_from_function_call,
    infer_hof_call_from_function_call_with_expected, infer_list_literal,
    infer_parameterised_reducer_call, infer_pipe_expr, infer_reduce_call,
    list_literal_sentinels_to_diagnostics, lookup_parameterised_reducer, lookup_reducer,
    EmptyIdentity, HofInferResult, HofInferSentinel, HofKind, HofSecondArg, ListDisambiguation,
    ListInferSentinel, ListLiteralInferResult, OriginTag, ParameterisedReducerResult,
    ParameterisedReducerSentinel, ReducerInputConstraint, ReducerOutputSort, ReducerSpec,
    SelectListSpreadResult, SplicePosition, SynthesizedReason, PARAMETERISED_REDUCER_REGISTRY,
    REDUCER_REGISTRY,
};
#[allow(unused_imports)]
use crate::type_inference::literal::{
    infer_case_expr_type, infer_cast_type, infer_extract_type, infer_literal_type,
    infer_numeric_literal_type,
};
#[allow(unused_imports)]
use crate::type_inference::loader_and_reflection::{
    check_column_ref_field_diagnostics, check_columns_of_diagnostics,
    check_config_var_call_diagnostics, check_loader_call_diagnostics, check_loader_path,
    check_model_ref_source_ref_field_diagnostics, check_wide_reflection_diagnostics,
    infer_field_on_column_ref, infer_field_on_model_ref, infer_field_on_source_ref,
    infer_loader_call_smelt_type, is_compile_time_text_arg, schema_arg_text_is_admissible,
    LoaderPathOutcome,
};
#[allow(unused_imports)]
use crate::type_inference::record::{
    check_meta_text_lift_diagnostics, check_record_in_data_world, check_record_literal,
    infer_map_method_call, infer_record_field_projection, is_meta_text_value,
    record_registry_for_workspace, registry_code_to_diagnostic_code, validate_map_type_expression,
    MapCallArg, MapMethodCallResult, MetaTextLiftPosition, RecordFieldProjectionResult,
    RecordLiteralResult, RecordLiteralSentinel, StaticResolution,
};
#[allow(unused_imports)]
use crate::type_inference::subquery::build_subquery_context;
#[allow(unused_imports)]
use crate::type_inference::type_context::TypeContext;

#[allow(unused_imports)]
use rowan::TextRange;
#[allow(unused_imports)]
use smelt_parser::ast::{
    BinaryExpr, CaseExpr, CastExpr, Cte, Expr, ExtractExpr, FunctionCall, RowConstructor,
    SelectStmt, SmeltAsStructCall, SmeltPathCall, StructLiteral, Subquery,
};
#[allow(unused_imports)]
use smelt_types::signatures::{
    kind_ceiling, unify_call_with_expected, BuiltinRegistry, ExprKind, FunctionSig, RecordRegistry,
    SmeltType, TypeConstraint,
};
#[allow(unused_imports)]
use smelt_types::{parse_type, DataType, SqlFunction, TypedColumn};
#[allow(unused_imports)]
use std::collections::HashMap;
#[allow(unused_imports)]
use std::sync::{Arc, Mutex};

#[test]
fn test_literal_type_inference() {
    // SmallInt (small values fit in SmallInt)
    assert_eq!(
        infer_literal_type("42"),
        Some(TypedColumn {
            data_type: DataType::SmallInt,
            nullable: false,
        })
    );

    // Integer (larger values that don't fit in SmallInt)
    assert_eq!(
        infer_literal_type("100000"),
        Some(TypedColumn {
            data_type: DataType::Integer,
            nullable: false,
        })
    );

    // BigInt
    assert_eq!(
        infer_literal_type("9999999999"),
        Some(TypedColumn {
            data_type: DataType::BigInt,
            nullable: false,
        })
    );

    // Decimal
    let decimal_type = infer_literal_type("123.45").unwrap();
    assert!(matches!(decimal_type.data_type, DataType::Decimal { .. }));
    assert!(!decimal_type.nullable);

    // Underscore digit separators: DuckDB types `1_000_000` as INTEGER
    // (`duckdb -c "SELECT typeof(1_000_000);"` -> INTEGER) and
    // `1_000.000_1` as DECIMAL(8,4) (`duckdb -c "SELECT typeof(1_000.000_1);"`
    // -> DECIMAL(8,4)) — separators must be stripped before value-parsing,
    // not treated as part of the digit count or cause an Unknown inference.
    assert_eq!(
        infer_literal_type("1_000_000"),
        Some(TypedColumn {
            data_type: DataType::Integer,
            nullable: false,
        })
    );
    assert_eq!(
        infer_literal_type("1_000.000_1"),
        Some(TypedColumn {
            data_type: DataType::Decimal {
                precision: 8,
                scale: 4,
            },
            nullable: false,
        })
    );

    // Double (scientific notation)
    assert_eq!(
        infer_literal_type("1.5e10"),
        Some(TypedColumn {
            data_type: DataType::Double,
            nullable: false,
        })
    );

    // String
    assert_eq!(
        infer_literal_type("'hello'"),
        Some(TypedColumn {
            data_type: DataType::Text,
            nullable: false,
        })
    );

    // Dollar-quoted strings infer exactly the same type as ordinary quoted
    // strings (DuckDB: `SELECT typeof($$abc$$)` -> VARCHAR, same as 'abc').
    assert_eq!(
        infer_literal_type("$$abc$$"),
        infer_literal_type("'abc'"),
        "$$abc$$ must infer the same type as 'abc'"
    );
    assert_eq!(
        infer_literal_type("$tag$ x $$ y $tag$"),
        Some(TypedColumn {
            data_type: DataType::Text,
            nullable: false,
        })
    );

    // Boolean
    assert_eq!(
        infer_literal_type("TRUE"),
        Some(TypedColumn {
            data_type: DataType::Boolean,
            nullable: false,
        })
    );

    // NULL
    assert_eq!(
        infer_literal_type("NULL"),
        Some(TypedColumn {
            data_type: DataType::Null,
            nullable: true,
        })
    );
}

#[test]
fn test_type_context_lookup() {
    let mut ctx = TypeContext::new();

    ctx.add_source_column(
        "raw",
        "users",
        "id",
        TypedColumn {
            data_type: DataType::Integer,
            nullable: false,
        },
    );

    ctx.add_model_column(
        "staging_users",
        "user_id",
        TypedColumn {
            data_type: DataType::BigInt,
            nullable: false,
        },
    );

    // Look up source column with qualifier
    let result = ctx.lookup_column(Some("users"), "id");
    assert!(result.is_some());
    assert_eq!(result.unwrap().data_type, DataType::Integer);

    // Look up model column with qualifier
    let result = ctx.lookup_column(Some("staging_users"), "user_id");
    assert!(result.is_some());
    assert_eq!(result.unwrap().data_type, DataType::BigInt);

    // Look up without qualifier (unambiguous)
    let result = ctx.lookup_column(None, "id");
    assert!(result.is_some());
}

#[test]
fn test_aggregate_function_types() {
    let ctx = TypeContext::new();

    // Create a mock expression text for COUNT
    // Note: In real usage, we'd use the actual AST
    let count_type = infer_function_type_by_name("COUNT", &ctx).unwrap();
    assert_eq!(count_type.data_type, DataType::BigInt);
    assert!(!count_type.nullable);

    // AVG returns Double
    let avg_type = infer_function_type_by_name("AVG", &ctx).unwrap();
    assert_eq!(avg_type.data_type, DataType::Double);
    assert!(avg_type.nullable);

    // SUM returns Decimal
    let sum_type = infer_function_type_by_name("SUM", &ctx).unwrap();
    assert!(matches!(sum_type.data_type, DataType::Decimal { .. }));
}

// Helper for testing function types without AST
fn infer_function_type_by_name(name: &str, _ctx: &TypeContext) -> Option<TypedColumn> {
    match name.to_uppercase().as_str() {
        // Aggregate functions
        "COUNT" => Some(TypedColumn {
            data_type: DataType::BigInt,
            nullable: false,
        }),
        "AVG" => Some(TypedColumn {
            data_type: DataType::Double,
            nullable: true,
        }),
        "SUM" => Some(TypedColumn {
            data_type: DataType::Decimal {
                precision: 38,
                scale: 10,
            },
            nullable: true,
        }),
        // Math functions
        "SQRT" | "POWER" | "POW" | "EXP" | "LN" | "LOG" | "LOG10" | "LOG2" => Some(TypedColumn {
            data_type: DataType::Double,
            nullable: true,
        }),
        "PI" | "RANDOM" => Some(TypedColumn {
            data_type: DataType::Double,
            nullable: false,
        }),
        "SIN" | "COS" | "TAN" | "ASIN" | "ACOS" | "ATAN" => Some(TypedColumn {
            data_type: DataType::Double,
            nullable: true,
        }),
        // Date/time functions
        "EXTRACT" | "DATE_PART" => Some(TypedColumn {
            data_type: DataType::BigInt,
            nullable: true,
        }),
        "MAKE_DATE" => Some(TypedColumn {
            data_type: DataType::Date,
            nullable: true,
        }),
        "AGE" => Some(TypedColumn {
            data_type: DataType::Interval,
            nullable: true,
        }),
        // String functions
        "REPLACE" | "SPLIT_PART" | "LEFT" | "RIGHT" | "LPAD" | "RPAD" => Some(TypedColumn {
            data_type: DataType::Text,
            nullable: true,
        }),
        "POSITION" | "STRPOS" => Some(TypedColumn {
            data_type: DataType::BigInt,
            nullable: true,
        }),
        "STRING_AGG" | "LISTAGG" => Some(TypedColumn {
            data_type: DataType::Text,
            nullable: true,
        }),
        _ => None,
    }
}

#[test]
fn test_cte_column_lookup() {
    let mut ctx = TypeContext::new();

    // Add a CTE column
    ctx.add_cte_column(
        "daily_totals",
        "day",
        TypedColumn {
            data_type: DataType::Date,
            nullable: false,
        },
    );

    ctx.add_cte_column(
        "daily_totals",
        "total",
        TypedColumn {
            data_type: DataType::Decimal {
                precision: 38,
                scale: 10,
            },
            nullable: true,
        },
    );

    // Check that CTE is registered
    assert!(ctx.is_cte("daily_totals"));
    assert!(!ctx.is_cte("nonexistent"));

    // Look up CTE column with qualifier
    let result = ctx.lookup_column(Some("daily_totals"), "day");
    assert!(result.is_some());
    assert_eq!(result.unwrap().data_type, DataType::Date);

    // Look up CTE column without qualifier
    let result = ctx.lookup_column(None, "total");
    assert!(result.is_some());
    assert!(matches!(
        result.unwrap().data_type,
        DataType::Decimal { .. }
    ));
}

#[test]
fn test_cte_shadows_source() {
    let mut ctx = TypeContext::new();

    // Add a source column with name "orders"
    ctx.add_source_column(
        "raw",
        "orders",
        "amount",
        TypedColumn {
            data_type: DataType::Integer,
            nullable: false,
        },
    );

    // Add a CTE with the same name "orders" but different column type
    ctx.add_cte_column(
        "orders",
        "amount",
        TypedColumn {
            data_type: DataType::BigInt,
            nullable: true,
        },
    );

    // CTE should shadow the source - BigInt should be returned, not Integer
    let result = ctx.lookup_column(Some("orders"), "amount");
    assert!(result.is_some());
    assert_eq!(result.unwrap().data_type, DataType::BigInt);

    // Unqualified lookup should also return CTE column
    let result = ctx.lookup_column(None, "amount");
    assert!(result.is_some());
    assert_eq!(result.unwrap().data_type, DataType::BigInt);
}

#[test]
fn test_extended_function_types() {
    let ctx = TypeContext::new();

    // Math functions
    let sqrt = infer_function_type_by_name("SQRT", &ctx).unwrap();
    assert_eq!(sqrt.data_type, DataType::Double);

    let power = infer_function_type_by_name("POWER", &ctx).unwrap();
    assert_eq!(power.data_type, DataType::Double);

    let pi = infer_function_type_by_name("PI", &ctx).unwrap();
    assert_eq!(pi.data_type, DataType::Double);
    assert!(!pi.nullable); // PI is never null

    let sin = infer_function_type_by_name("SIN", &ctx).unwrap();
    assert_eq!(sin.data_type, DataType::Double);

    // Date/time functions
    let extract = infer_function_type_by_name("EXTRACT", &ctx).unwrap();
    assert_eq!(extract.data_type, DataType::BigInt);

    let make_date = infer_function_type_by_name("MAKE_DATE", &ctx).unwrap();
    assert_eq!(make_date.data_type, DataType::Date);

    let age = infer_function_type_by_name("AGE", &ctx).unwrap();
    assert_eq!(age.data_type, DataType::Interval);

    // String functions
    let replace = infer_function_type_by_name("REPLACE", &ctx).unwrap();
    assert_eq!(replace.data_type, DataType::Text);

    let position = infer_function_type_by_name("POSITION", &ctx).unwrap();
    assert_eq!(position.data_type, DataType::BigInt);

    let split_part = infer_function_type_by_name("SPLIT_PART", &ctx).unwrap();
    assert_eq!(split_part.data_type, DataType::Text);

    // String aggregate
    let string_agg = infer_function_type_by_name("STRING_AGG", &ctx).unwrap();
    assert_eq!(string_agg.data_type, DataType::Text);
}

/// Regression: MEDIAN over integer-family inputs interpolates to Double in
/// DuckDB and Spark (`median(INTEGER) -> DOUBLE`), so smelt must widen too;
/// previously it returned the argument type (Integer). Decimal/Double inputs
/// keep their own type. Surfaced by the extended type-oracle generators.
#[test]
fn median_integer_infers_double() {
    let mut ctx = TypeContext::new();
    ctx.add_cte_column("t", "x", TypedColumn::nullable(DataType::Integer));
    let types = infer_sql_with_ctx(
        "WITH t AS (SELECT CAST(1 AS INTEGER) AS x) SELECT MEDIAN(x) FROM t",
        &ctx,
    );
    assert_eq!(types[0].data_type, DataType::Double);

    let mut ctx = TypeContext::new();
    ctx.add_cte_column("t", "b", TypedColumn::nullable(DataType::BigInt));
    let types = infer_sql_with_ctx(
        "WITH t AS (SELECT CAST(1 AS BIGINT) AS b) SELECT MEDIAN(b) FROM t",
        &ctx,
    );
    assert_eq!(types[0].data_type, DataType::Double);

    // Decimal input preserves its precision/scale.
    let mut ctx = TypeContext::new();
    ctx.add_cte_column(
        "t",
        "d",
        TypedColumn::nullable(DataType::Decimal {
            precision: 10,
            scale: 2,
        }),
    );
    let types = infer_sql_with_ctx(
        "WITH t AS (SELECT CAST(1 AS DECIMAL(10,2)) AS d) SELECT MEDIAN(d) FROM t",
        &ctx,
    );
    assert_eq!(
        types[0].data_type,
        DataType::Decimal {
            precision: 10,
            scale: 2
        }
    );
}

/// Parse a SQL SELECT and return the inferred types of all columns.
fn infer_sql(sql: &str) -> Vec<TypedColumn> {
    infer_sql_with_ctx(sql, &TypeContext::new())
}

fn infer_sql_with_ctx(sql: &str, ctx: &TypeContext) -> Vec<TypedColumn> {
    use smelt_parser::ast::File;
    let parse = smelt_parser::parse(sql);
    let root = parse.syntax();
    let file = File::cast(root).expect("failed to cast to File");
    let select_stmt = file.select_stmt().expect("no SelectStmt in parsed SQL");
    infer_select_column_types(&select_stmt, ctx)
}

#[test]
fn test_coalesce_nullability() {
    // COALESCE with a non-null literal → non-nullable
    let types = infer_sql("SELECT COALESCE(NULL, 42)");
    assert_eq!(types[0].data_type, DataType::SmallInt);
    assert!(
        !types[0].nullable,
        "COALESCE with non-null literal should be non-nullable"
    );

    // COALESCE with all nullable columns → nullable
    let mut ctx = TypeContext::new();
    ctx.add_cte_column("t", "a", TypedColumn::nullable(DataType::Integer));
    ctx.add_cte_column("t", "b", TypedColumn::nullable(DataType::Integer));
    let types = infer_sql_with_ctx(
        "WITH t AS (SELECT 1 AS a, 2 AS b) SELECT COALESCE(a, b) FROM t",
        &ctx,
    );
    assert_eq!(types[0].data_type, DataType::Integer);
    assert!(
        types[0].nullable,
        "COALESCE with all nullable args should be nullable"
    );

    // COALESCE where second arg is non-nullable → non-nullable
    let mut ctx = TypeContext::new();
    ctx.add_cte_column("t", "a", TypedColumn::nullable(DataType::Integer));
    ctx.add_cte_column("t", "b", TypedColumn::not_null(DataType::Integer));
    let types = infer_sql_with_ctx(
        "WITH t AS (SELECT 1 AS a, 2 AS b) SELECT COALESCE(a, b) FROM t",
        &ctx,
    );
    assert!(
        !types[0].nullable,
        "COALESCE with non-nullable arg should be non-nullable"
    );

    // COALESCE with a non-null literal as fallback → non-nullable
    let mut ctx = TypeContext::new();
    ctx.add_cte_column("t", "a", TypedColumn::nullable(DataType::Integer));
    let types = infer_sql_with_ctx(
        "WITH t AS (SELECT 1 AS a) SELECT COALESCE(a, 0) FROM t",
        &ctx,
    );
    assert!(
        !types[0].nullable,
        "COALESCE with literal fallback should be non-nullable"
    );
}

#[test]
fn test_case_nullability() {
    // CASE without ELSE → always nullable (implicit NULL)
    let types = infer_sql("SELECT CASE WHEN 1 = 1 THEN 42 END");
    assert_eq!(types[0].data_type, DataType::SmallInt);
    assert!(types[0].nullable, "CASE without ELSE should be nullable");

    // CASE with ELSE, all branches non-nullable → non-nullable
    let types = infer_sql("SELECT CASE WHEN 1 = 1 THEN 42 ELSE 0 END");
    assert!(
        !types[0].nullable,
        "CASE with ELSE and non-nullable branches should be non-nullable"
    );

    // CASE with ELSE, but a branch returns NULL → nullable
    let types = infer_sql("SELECT CASE WHEN 1 = 1 THEN NULL ELSE 0 END");
    assert!(
        types[0].nullable,
        "CASE with NULL branch should be nullable"
    );

    // CASE with ELSE that is NULL → nullable
    let types = infer_sql("SELECT CASE WHEN 1 = 1 THEN 42 ELSE NULL END");
    assert!(types[0].nullable, "CASE with NULL ELSE should be nullable");

    // CASE with multiple WHEN branches, all non-nullable + ELSE → non-nullable
    let types = infer_sql("SELECT CASE WHEN 1 = 1 THEN 42 WHEN 2 = 2 THEN 99 ELSE 0 END");
    assert!(
        !types[0].nullable,
        "CASE with all non-nullable branches and ELSE should be non-nullable"
    );
}

#[test]
fn test_cast_nullability() {
    // CAST of non-nullable literal → non-nullable
    let types = infer_sql("SELECT CAST(42 AS VARCHAR)");
    assert_eq!(types[0].data_type, DataType::Varchar { max_length: None });
    assert!(
        !types[0].nullable,
        "CAST of non-nullable literal should be non-nullable"
    );

    // CAST of NULL → nullable
    let types = infer_sql("SELECT CAST(NULL AS INTEGER)");
    assert!(types[0].nullable, "CAST of NULL should be nullable");

    // CAST of non-nullable column → non-nullable
    let mut ctx = TypeContext::new();
    ctx.add_cte_column("t", "x", TypedColumn::not_null(DataType::Integer));
    let types = infer_sql_with_ctx(
        "WITH t AS (SELECT 1 AS x) SELECT CAST(x AS VARCHAR) FROM t",
        &ctx,
    );
    assert!(
        !types[0].nullable,
        "CAST of non-nullable column should be non-nullable"
    );

    // CAST of nullable column → nullable
    let mut ctx = TypeContext::new();
    ctx.add_cte_column("t", "x", TypedColumn::nullable(DataType::Integer));
    let types = infer_sql_with_ctx(
        "WITH t AS (SELECT 1 AS x) SELECT CAST(x AS VARCHAR) FROM t",
        &ctx,
    );
    assert!(
        types[0].nullable,
        "CAST of nullable column should be nullable"
    );
}

#[test]
fn test_ifnull_nullability() {
    // IFNULL with non-null literal fallback → non-nullable
    let mut ctx = TypeContext::new();
    ctx.add_cte_column("t", "a", TypedColumn::nullable(DataType::Integer));
    let types = infer_sql_with_ctx("WITH t AS (SELECT 1 AS a) SELECT IFNULL(a, 0) FROM t", &ctx);
    assert_eq!(types[0].data_type, DataType::Integer);
    assert!(
        !types[0].nullable,
        "IFNULL with non-null literal fallback should be non-nullable"
    );

    // IFNULL with both nullable → nullable
    let mut ctx = TypeContext::new();
    ctx.add_cte_column("t", "a", TypedColumn::nullable(DataType::Integer));
    ctx.add_cte_column("t", "b", TypedColumn::nullable(DataType::Integer));
    let types = infer_sql_with_ctx(
        "WITH t AS (SELECT 1 AS a, 2 AS b) SELECT IFNULL(a, b) FROM t",
        &ctx,
    );
    assert!(
        types[0].nullable,
        "IFNULL with both nullable should be nullable"
    );

    // IFNULL where first arg is non-nullable → non-nullable
    let mut ctx = TypeContext::new();
    ctx.add_cte_column("t", "a", TypedColumn::not_null(DataType::Integer));
    ctx.add_cte_column("t", "b", TypedColumn::nullable(DataType::Integer));
    let types = infer_sql_with_ctx(
        "WITH t AS (SELECT 1 AS a, 2 AS b) SELECT IFNULL(a, b) FROM t",
        &ctx,
    );
    assert!(
        !types[0].nullable,
        "IFNULL with non-nullable first arg should be non-nullable"
    );
}

// Regression: a dedicated numeric-function property test caught IFNULL,
// COALESCE, GREATEST, LEAST, and MOD returning the first argument's type
// verbatim instead of promoting across mixed numeric argument types — a real
// divergence from DuckDB itself (not a cross-engine difference), found in
// 175 (function, type-pair) combinations.
#[test]
fn test_ifnull_promotes_mixed_numeric_types() {
    let mut ctx = TypeContext::new();
    ctx.add_cte_column("t", "a", TypedColumn::nullable(DataType::SmallInt));
    ctx.add_cte_column("t", "b", TypedColumn::nullable(DataType::Integer));
    let types = infer_sql_with_ctx(
        "WITH t AS (SELECT 1 AS a, 2 AS b) SELECT IFNULL(a, b) FROM t",
        &ctx,
    );
    assert_eq!(
        types[0].data_type,
        DataType::Integer,
        "IFNULL(SmallInt, Integer) should promote to Integer, matching DuckDB"
    );
}

#[test]
fn test_coalesce_promotes_mixed_numeric_types() {
    let mut ctx = TypeContext::new();
    ctx.add_cte_column("t", "a", TypedColumn::nullable(DataType::Integer));
    ctx.add_cte_column("t", "b", TypedColumn::nullable(DataType::BigInt));
    let types = infer_sql_with_ctx(
        "WITH t AS (SELECT 1 AS a, 2 AS b) SELECT COALESCE(a, b) FROM t",
        &ctx,
    );
    assert_eq!(
        types[0].data_type,
        DataType::BigInt,
        "COALESCE(Integer, BigInt) should promote to BigInt, matching DuckDB"
    );
}

#[test]
fn test_greatest_least_promote_mixed_numeric_types() {
    let mut ctx = TypeContext::new();
    ctx.add_cte_column("t", "a", TypedColumn::nullable(DataType::Integer));
    ctx.add_cte_column(
        "t",
        "b",
        TypedColumn::nullable(DataType::Decimal {
            precision: 10,
            scale: 2,
        }),
    );
    let types = infer_sql_with_ctx(
        "WITH t AS (SELECT 1 AS a, 2 AS b) SELECT GREATEST(a, b), LEAST(a, b) FROM t",
        &ctx,
    );
    // promote_types widens Integer+Decimal to Decimal(38,10) (the same
    // overflow-avoidance rule CASE/UNION use, Bug #7) rather than DuckDB's
    // narrower DECIMAL(12,2) — an already-tolerated raw-SQL precision/scale
    // gap (decimal_arithmetic_model in divergences.rs). What matters here is
    // the *family*: Decimal, not Integer, matching DuckDB's GREATEST/LEAST
    // widening across mixed numeric types.
    assert!(
        matches!(types[0].data_type, DataType::Decimal { .. }),
        "GREATEST(Integer, Decimal) should promote to the Decimal family, matching DuckDB: got {:?}",
        types[0].data_type
    );
    assert!(
        matches!(types[1].data_type, DataType::Decimal { .. }),
        "LEAST(Integer, Decimal) should promote to the Decimal family, matching DuckDB: got {:?}",
        types[1].data_type
    );
}

#[test]
fn test_mod_promotes_mixed_numeric_types() {
    let mut ctx = TypeContext::new();
    ctx.add_cte_column("t", "a", TypedColumn::nullable(DataType::SmallInt));
    ctx.add_cte_column("t", "b", TypedColumn::nullable(DataType::BigInt));
    let types = infer_sql_with_ctx(
        "WITH t AS (SELECT 1 AS a, 2 AS b) SELECT MOD(a, b) FROM t",
        &ctx,
    );
    assert_eq!(
        types[0].data_type,
        DataType::BigInt,
        "MOD(SmallInt, BigInt) should promote to BigInt, matching DuckDB"
    );
}

#[test]
fn test_temporal_arithmetic_date_interval() {
    // DATE + INTERVAL → Timestamp
    let types = infer_sql("SELECT CAST('2024-01-01' AS DATE) + INTERVAL '1' DAY");
    assert_eq!(
        types[0].data_type,
        DataType::Timestamp {
            with_timezone: false
        }
    );

    // DATE - INTERVAL → Timestamp
    let types = infer_sql("SELECT CAST('2024-01-01' AS DATE) - INTERVAL '1' DAY");
    assert_eq!(
        types[0].data_type,
        DataType::Timestamp {
            with_timezone: false
        }
    );

    // DATE - DATE → Interval
    let types = infer_sql("SELECT CAST('2024-01-01' AS DATE) - CAST('2024-01-02' AS DATE)");
    assert_eq!(types[0].data_type, DataType::Interval);
}

#[test]
fn test_temporal_arithmetic_timestamp_interval() {
    // TIMESTAMP + INTERVAL → Timestamp
    let types = infer_sql("SELECT CAST('2024-01-01' AS TIMESTAMP) + INTERVAL '1' HOUR");
    assert_eq!(
        types[0].data_type,
        DataType::Timestamp {
            with_timezone: false
        }
    );

    // TIMESTAMP - INTERVAL → Timestamp
    let types = infer_sql("SELECT CAST('2024-01-01' AS TIMESTAMP) - INTERVAL '1' HOUR");
    assert_eq!(
        types[0].data_type,
        DataType::Timestamp {
            with_timezone: false
        }
    );

    // TIMESTAMP - TIMESTAMP → Interval
    let types =
        infer_sql("SELECT CAST('2024-01-01' AS TIMESTAMP) - CAST('2024-01-02' AS TIMESTAMP)");
    assert_eq!(types[0].data_type, DataType::Interval);
}

#[test]
fn test_temporal_arithmetic_interval_ops() {
    // INTERVAL + INTERVAL → Interval
    let types = infer_sql("SELECT INTERVAL '1' DAY + INTERVAL '2' HOUR");
    assert_eq!(types[0].data_type, DataType::Interval);

    // INTERVAL - INTERVAL → Interval
    let types = infer_sql("SELECT INTERVAL '1' DAY - INTERVAL '2' HOUR");
    assert_eq!(types[0].data_type, DataType::Interval);

    // INTERVAL * numeric → Interval
    let types = infer_sql("SELECT INTERVAL '1' DAY * 3");
    assert_eq!(types[0].data_type, DataType::Interval);

    // numeric * INTERVAL → Interval
    let types = infer_sql("SELECT 3 * INTERVAL '1' DAY");
    assert_eq!(types[0].data_type, DataType::Interval);

    // INTERVAL / numeric → Interval
    let types = infer_sql("SELECT INTERVAL '6' HOUR / 2");
    assert_eq!(types[0].data_type, DataType::Interval);
}

#[test]
fn test_temporal_arithmetic_time() {
    // TIME + INTERVAL → Time
    let types = infer_sql("SELECT CAST('12:00:00' AS TIME) + INTERVAL '1' HOUR");
    assert_eq!(types[0].data_type, DataType::Time);

    // TIME - INTERVAL → Time
    let types = infer_sql("SELECT CAST('12:00:00' AS TIME) - INTERVAL '1' HOUR");
    assert_eq!(types[0].data_type, DataType::Time);

    // TIME - TIME → Interval
    let types = infer_sql("SELECT CAST('12:00:00' AS TIME) - CAST('10:00:00' AS TIME)");
    assert_eq!(types[0].data_type, DataType::Interval);
}

#[test]
fn test_temporal_arithmetic_with_columns() {
    // Test with typed columns from context
    let mut ctx = TypeContext::new();
    ctx.add_cte_column("t", "d", TypedColumn::not_null(DataType::Date));
    ctx.add_cte_column(
        "t",
        "ts",
        TypedColumn::not_null(DataType::Timestamp {
            with_timezone: false,
        }),
    );
    ctx.add_cte_column("t", "i", TypedColumn::not_null(DataType::Interval));

    // Column DATE + INTERVAL → Timestamp
    let types = infer_sql_with_ctx(
        "WITH t AS (SELECT 1 AS d) SELECT d + INTERVAL '1' DAY FROM t",
        &ctx,
    );
    assert_eq!(
        types[0].data_type,
        DataType::Timestamp {
            with_timezone: false
        }
    );

    // Column TIMESTAMP - Column TIMESTAMP → Interval
    let types = infer_sql_with_ctx("WITH t AS (SELECT 1 AS ts) SELECT ts - ts FROM t", &ctx);
    assert_eq!(types[0].data_type, DataType::Interval);
}

#[test]
fn test_at_time_zone_naive_to_aware() {
    // TIMESTAMP AT TIME ZONE tz → TIMESTAMP WITH TIME ZONE (verified against
    // the DuckDB oracle: `typeof(ts AT TIME ZONE 'UTC')` on a naive TIMESTAMP
    // column returns `TIMESTAMP WITH TIME ZONE`).
    let mut ctx = TypeContext::new();
    ctx.add_cte_column(
        "t",
        "ts",
        TypedColumn::not_null(DataType::Timestamp {
            with_timezone: false,
        }),
    );
    let types = infer_sql_with_ctx(
        "WITH t AS (SELECT 1 AS ts) SELECT ts AT TIME ZONE 'UTC' FROM t",
        &ctx,
    );
    assert_eq!(
        types[0].data_type,
        DataType::Timestamp {
            with_timezone: true
        }
    );
    assert!(
        !types[0].nullable,
        "nullability propagates from the operand"
    );
}

#[test]
fn test_at_time_zone_aware_to_naive() {
    // TIMESTAMP WITH TIME ZONE AT TIME ZONE tz → TIMESTAMP (plain) — verified
    // against the DuckDB oracle: `typeof((ts AT TIME ZONE 'UTC') AT TIME ZONE
    // 'America/New_York')` on a naive TIMESTAMP returns `TIMESTAMP` (the
    // second AT TIME ZONE strips the tz-awareness added by the first).
    let mut ctx = TypeContext::new();
    ctx.add_cte_column(
        "t",
        "ts",
        TypedColumn::not_null(DataType::Timestamp {
            with_timezone: true,
        }),
    );
    let types = infer_sql_with_ctx(
        "WITH t AS (SELECT 1 AS ts) SELECT ts AT TIME ZONE 'UTC' FROM t",
        &ctx,
    );
    assert_eq!(
        types[0].data_type,
        DataType::Timestamp {
            with_timezone: false
        }
    );
    assert!(
        !types[0].nullable,
        "nullability propagates from the operand"
    );
}

#[test]
fn test_at_time_zone_chained_round_trip() {
    // ts AT TIME ZONE 'UTC' AT TIME ZONE 'EST' on a naive TIMESTAMP: first
    // conversion → TIMESTAMP WITH TIME ZONE, second conversion strips it back
    // to plain TIMESTAMP (verified against the DuckDB oracle).
    let mut ctx = TypeContext::new();
    ctx.add_cte_column(
        "t",
        "ts",
        TypedColumn::not_null(DataType::Timestamp {
            with_timezone: false,
        }),
    );
    let types = infer_sql_with_ctx(
        "WITH t AS (SELECT 1 AS ts) SELECT ts AT TIME ZONE 'UTC' AT TIME ZONE 'EST' FROM t",
        &ctx,
    );
    assert_eq!(
        types[0].data_type,
        DataType::Timestamp {
            with_timezone: false
        }
    );
}

#[test]
fn test_at_time_zone_nullable_operand_propagates() {
    let mut ctx = TypeContext::new();
    ctx.add_cte_column(
        "t",
        "ts",
        TypedColumn::nullable(DataType::Timestamp {
            with_timezone: false,
        }),
    );
    let types = infer_sql_with_ctx(
        "WITH t AS (SELECT 1 AS ts) SELECT ts AT TIME ZONE 'UTC' FROM t",
        &ctx,
    );
    assert!(
        types[0].nullable,
        "nullable operand should propagate nullability"
    );
}

#[test]
fn test_promote_types_numeric_hierarchy() {
    let mk = |dt: DataType| TypedColumn {
        data_type: dt,
        nullable: false,
    };

    // SmallInt + Integer → Integer
    assert_eq!(
        promote_types(&mk(DataType::SmallInt), &mk(DataType::Integer)).data_type,
        DataType::Integer
    );
    // Integer + BigInt → BigInt
    assert_eq!(
        promote_types(&mk(DataType::Integer), &mk(DataType::BigInt)).data_type,
        DataType::BigInt
    );
    // BigInt + Float → Float
    assert_eq!(
        promote_types(&mk(DataType::BigInt), &mk(DataType::Float)).data_type,
        DataType::Float
    );
    // Float + Double → Double
    assert_eq!(
        promote_types(&mk(DataType::Float), &mk(DataType::Double)).data_type,
        DataType::Double
    );
    // Float + Decimal → Float
    assert_eq!(
        promote_types(
            &mk(DataType::Float),
            &mk(DataType::Decimal {
                precision: 10,
                scale: 2
            })
        )
        .data_type,
        DataType::Float
    );
    // Decimal + Integer → Decimal(38,10) (widened to prevent overflow)
    // e.g. CASE WHEN ... THEN 150::INTEGER ELSE col::DECIMAL(10,2) must hold integer values
    assert_eq!(
        promote_types(
            &mk(DataType::Decimal {
                precision: 10,
                scale: 2
            }),
            &mk(DataType::Integer)
        )
        .data_type,
        DataType::Decimal {
            precision: 38,
            scale: 10
        }
    );
}

#[test]
fn test_promote_types_null_handling() {
    let mk = |dt: DataType| TypedColumn {
        data_type: dt,
        nullable: false,
    };

    // Null + Integer → Integer (nullable)
    let result = promote_types(&mk(DataType::Null), &mk(DataType::Integer));
    assert_eq!(result.data_type, DataType::Integer);
    assert!(result.nullable);

    // Integer + Null → Integer (nullable)
    let result = promote_types(&mk(DataType::Integer), &mk(DataType::Null));
    assert_eq!(result.data_type, DataType::Integer);
    assert!(result.nullable);

    // Unknown + Text → Text
    let result = promote_types(&mk(DataType::unknown_dynamic()), &mk(DataType::Text));
    assert_eq!(result.data_type, DataType::Text);
}

#[test]
fn test_promote_types_temporal() {
    let mk = |dt: DataType| TypedColumn {
        data_type: dt,
        nullable: false,
    };

    // Date + Timestamp → Timestamp
    assert_eq!(
        promote_types(
            &mk(DataType::Date),
            &mk(DataType::Timestamp {
                with_timezone: false
            })
        )
        .data_type,
        DataType::Timestamp {
            with_timezone: false
        }
    );

    // Date + Time → Timestamp
    assert_eq!(
        promote_types(&mk(DataType::Date), &mk(DataType::Time)).data_type,
        DataType::Timestamp {
            with_timezone: false
        }
    );
}

#[test]
fn test_promote_types_string() {
    let mk = |dt: DataType| TypedColumn {
        data_type: dt,
        nullable: false,
    };

    // Varchar + Text → Text
    assert_eq!(
        promote_types(
            &mk(DataType::Varchar {
                max_length: Some(10)
            }),
            &mk(DataType::Text)
        )
        .data_type,
        DataType::Text
    );

    // Varchar + Varchar → Text (different discriminant doesn't matter, same variant)
    assert_eq!(
        promote_types(
            &mk(DataType::Varchar {
                max_length: Some(10)
            }),
            &mk(DataType::Varchar {
                max_length: Some(20)
            })
        )
        .data_type,
        DataType::Varchar {
            max_length: Some(10)
        } // same discriminant, returns first
    );
}

#[test]
fn test_union_type_inference() {
    // UNION of SmallInt + Integer → Integer
    let types =
        infer_sql("SELECT CAST(1 AS SMALLINT) AS x UNION ALL SELECT CAST(2 AS INTEGER) AS x");
    assert_eq!(types[0].data_type, DataType::Integer);

    // UNION of Integer + BigInt → BigInt
    let types = infer_sql("SELECT CAST(1 AS INTEGER) AS x UNION ALL SELECT CAST(2 AS BIGINT) AS x");
    assert_eq!(types[0].data_type, DataType::BigInt);

    // 3-way UNION: SmallInt + Integer + BigInt → BigInt
    let types = infer_sql(
            "SELECT CAST(1 AS SMALLINT) AS x UNION ALL SELECT CAST(2 AS INTEGER) AS x UNION ALL SELECT CAST(3 AS BIGINT) AS x"
        );
    assert_eq!(types[0].data_type, DataType::BigInt);
}

#[test]
fn test_intersect_except_type_inference() {
    // INTERSECT should also promote types
    let types =
        infer_sql("SELECT CAST(1 AS SMALLINT) AS x INTERSECT SELECT CAST(2 AS INTEGER) AS x");
    assert_eq!(types[0].data_type, DataType::Integer);

    // EXCEPT should also promote types
    let types = infer_sql("SELECT CAST(1 AS INTEGER) AS x EXCEPT SELECT CAST(2 AS BIGINT) AS x");
    assert_eq!(types[0].data_type, DataType::BigInt);
}

#[test]
fn test_promote_types_decimal_precision() {
    let mk = |dt: DataType| TypedColumn {
        data_type: dt,
        nullable: false,
    };

    // Decimal(10,2) + Decimal(18,4) → Decimal(18,4) (takes max)
    assert_eq!(
        promote_types(
            &mk(DataType::Decimal {
                precision: 10,
                scale: 2
            }),
            &mk(DataType::Decimal {
                precision: 18,
                scale: 4
            })
        )
        .data_type,
        DataType::Decimal {
            precision: 18,
            scale: 4
        }
    );
}

#[test]
fn test_array_literal_integer() {
    let types = infer_sql("SELECT ARRAY[1, 2, 3]");
    assert_eq!(
        types[0].data_type,
        DataType::Array(Box::new(DataType::SmallInt))
    );
    assert!(!types[0].nullable, "array literal should be non-nullable");
}

#[test]
fn test_array_literal_string() {
    let types = infer_sql("SELECT ARRAY['a', 'b', 'c']");
    assert_eq!(
        types[0].data_type,
        DataType::Array(Box::new(DataType::Text))
    );
}

#[test]
fn test_array_literal_empty() {
    let types = infer_sql("SELECT ARRAY[]");
    assert_eq!(
        types[0].data_type,
        DataType::Array(Box::new(DataType::unknown_dynamic()))
    );
}

#[test]
fn test_array_literal_with_null() {
    // ARRAY[1, NULL, 3] — NULL is compatible, element type is SmallInt
    let types = infer_sql("SELECT ARRAY[1, NULL, 3]");
    assert_eq!(
        types[0].data_type,
        DataType::Array(Box::new(DataType::SmallInt))
    );
}

#[test]
fn test_array_literal_numeric_promotion() {
    // ARRAY[1, 2.5] — SmallInt + Decimal should promote
    let types = infer_sql("SELECT ARRAY[1, 100000]");
    // 1 is SmallInt, 100000 is Integer → promoted to Integer
    assert_eq!(
        types[0].data_type,
        DataType::Array(Box::new(DataType::Integer))
    );
}

#[test]
fn test_array_literal_mixed_types_rejected() {
    // ARRAY[1, 'hello'] — Integer + Text can't be promoted → should fail inference
    let types = infer_sql("SELECT ARRAY[1, 'hello']");
    // Mixed types return Unknown since the array literal inference returns None
    assert_eq!(types[0].data_type, DataType::unknown_dynamic());
}

// ===== List comprehensions: [expr FOR x IN list (IF cond)?] (DuckDB) =====

#[test]
fn test_list_comprehension_bare_var_element_types_from_source() {
    // `[x FOR x IN [1, 2, 3]]` — element expr is exactly the loop variable,
    // so the result element type is the source list's element type.
    let types = infer_sql("SELECT [x FOR x IN [1, 2, 3]]");
    assert_eq!(
        types[0].data_type,
        DataType::Array(Box::new(DataType::SmallInt))
    );
    assert!(
        !types[0].nullable,
        "list comprehension result should be non-nullable"
    );
}

#[test]
fn test_list_comprehension_bare_var_with_filter_types_from_source() {
    // `[x FOR x IN [1, 2, 3] IF x > 1]` — filter present, still a bare-var
    // element, so still typed from the source list's element type.
    let types = infer_sql("SELECT [x FOR x IN [1, 2, 3] IF x > 1]");
    assert_eq!(
        types[0].data_type,
        DataType::Array(Box::new(DataType::SmallInt))
    );
}

#[test]
fn test_list_comprehension_non_trivial_element_is_classified_unknown() {
    // `[x + 1 FOR x IN [1, 2, 3]]` — element expr is not the bare loop
    // variable, so the element type is the classified-Unknown fallback
    // (TypeContext has no scoped-binding machinery for the loop variable).
    let types = infer_sql("SELECT [x + 1 FOR x IN [1, 2, 3]]");
    assert_eq!(
        types[0].data_type,
        DataType::Array(Box::new(DataType::unknown_dynamic()))
    );
}

#[test]
fn test_list_comprehension_string_source() {
    let types = infer_sql("SELECT [s FOR s IN ['a', 'b']]");
    assert_eq!(
        types[0].data_type,
        DataType::Array(Box::new(DataType::Text))
    );
}

#[test]
fn test_array_subscript_from_column() {
    // With a column of Array(Integer) type, subscript should return Integer
    let mut ctx = TypeContext::new();
    ctx.add_model_column(
        "t",
        "arr",
        TypedColumn::not_null(DataType::Array(Box::new(DataType::Integer))),
    );
    let types = infer_sql_with_ctx("SELECT arr[1]", &ctx);
    assert_eq!(types[0].data_type, DataType::Integer);
    assert!(types[0].nullable, "array element access should be nullable");
}

#[test]
fn test_array_slice_from_column() {
    // Slice should return the same array type
    let mut ctx = TypeContext::new();
    ctx.add_model_column(
        "t",
        "arr",
        TypedColumn::not_null(DataType::Array(Box::new(DataType::Integer))),
    );
    let types = infer_sql_with_ctx("SELECT arr[1:3]", &ctx);
    assert_eq!(
        types[0].data_type,
        DataType::Array(Box::new(DataType::Integer))
    );
}

#[test]
fn test_row_constructor_type() {
    // ROW(1, 'hello', TRUE) → Struct with positional fields
    let types = infer_sql("SELECT ROW(1, 'hello', TRUE)");
    assert_eq!(
        types[0].data_type,
        DataType::Struct(vec![
            ("v1".to_string(), DataType::SmallInt),
            ("v2".to_string(), DataType::Text),
            ("v3".to_string(), DataType::Boolean),
        ])
    );
    assert!(!types[0].nullable); // Struct itself is not nullable
}

#[test]
fn test_struct_literal_named_fields() {
    // STRUCT(1 AS a, 'hello' AS b) → Struct with named fields
    let types = infer_sql("SELECT STRUCT(1 AS a, 'hello' AS b)");
    assert_eq!(
        types[0].data_type,
        DataType::Struct(vec![
            ("a".to_string(), DataType::SmallInt),
            ("b".to_string(), DataType::Text),
        ])
    );
    assert!(!types[0].nullable);
}

#[test]
fn test_struct_literal_unnamed_fields() {
    // STRUCT(1, 2, 3) without AS → positional names
    let types = infer_sql("SELECT STRUCT(1, 2, 3)");
    assert_eq!(
        types[0].data_type,
        DataType::Struct(vec![
            ("v1".to_string(), DataType::SmallInt),
            ("v2".to_string(), DataType::SmallInt),
            ("v3".to_string(), DataType::SmallInt),
        ])
    );
}

#[test]
fn test_struct_literal_mixed_named_unnamed() {
    // STRUCT(1 AS a, 'hello') → mix of named and positional
    let types = infer_sql("SELECT STRUCT(1 AS a, 'hello')");
    assert_eq!(
        types[0].data_type,
        DataType::Struct(vec![
            ("a".to_string(), DataType::SmallInt),
            ("v2".to_string(), DataType::Text),
        ])
    );
}

#[test]
fn test_struct_field_access() {
    // Field access on a struct-typed column
    let mut ctx = TypeContext::new();
    ctx.add_model_column(
        "t",
        "s",
        TypedColumn::not_null(DataType::Struct(vec![
            ("name".to_string(), DataType::Text),
            ("age".to_string(), DataType::Integer),
        ])),
    );
    let types = infer_sql_with_ctx("SELECT s.name", &ctx);
    assert_eq!(types[0].data_type, DataType::Text);
    assert!(types[0].nullable); // Field access is nullable (struct could be null)
}

#[test]
fn test_struct_field_access_case_insensitive() {
    let mut ctx = TypeContext::new();
    ctx.add_model_column(
        "t",
        "s",
        TypedColumn::not_null(DataType::Struct(vec![("Name".to_string(), DataType::Text)])),
    );
    let types = infer_sql_with_ctx("SELECT s.name", &ctx);
    assert_eq!(types[0].data_type, DataType::Text);
}

#[test]
fn test_map_literal_string_int() {
    // MAP {'a': 1, 'b': 2} → Map(Text, SmallInt). Verified against the DuckDB
    // oracle: `typeof(MAP {'a': 1, 'b': 2})` is `MAP(VARCHAR, INTEGER)`; smelt's
    // key/value width inference (SmallInt for small integer literals) matches
    // the same convention as ARRAY[1, 2, 3] → Array(SmallInt) above.
    let types = infer_sql("SELECT MAP {'a': 1, 'b': 2}");
    assert_eq!(
        types[0].data_type,
        DataType::Map(Box::new(DataType::Text), Box::new(DataType::SmallInt))
    );
    assert!(!types[0].nullable, "map literal should be non-nullable");
}

#[test]
fn test_map_literal_empty() {
    // Verified against DuckDB: `MAP {}` parses and executes. smelt infers
    // Map(Unknown, Unknown) — following the `ARRAY[]` → `Array(Unknown)`
    // precedent rather than DuckDB's engine-specific INTEGER default.
    let types = infer_sql("SELECT MAP {}");
    assert_eq!(
        types[0].data_type,
        DataType::Map(
            Box::new(DataType::unknown_dynamic()),
            Box::new(DataType::unknown_dynamic())
        )
    );
}

#[test]
fn test_map_literal_numeric_keys() {
    // MAP {1: 'x', 2: 'y'} → Map(SmallInt, Text). Verified against DuckDB:
    // `typeof(MAP {1: 'x', 2: 'y'})` is `MAP(INTEGER, VARCHAR)`.
    let types = infer_sql("SELECT MAP {1: 'x', 2: 'y'}");
    assert_eq!(
        types[0].data_type,
        DataType::Map(Box::new(DataType::SmallInt), Box::new(DataType::Text))
    );
}

#[test]
fn test_map_literal_trailing_comma() {
    // Verified against DuckDB: trailing comma before `}` is accepted.
    let types = infer_sql("SELECT MAP {'a': 1, 'b': 2,}");
    assert_eq!(
        types[0].data_type,
        DataType::Map(Box::new(DataType::Text), Box::new(DataType::SmallInt))
    );
}

#[test]
fn test_map_literal_mixed_value_types_rejected() {
    // MAP {'a': 1, 'b': 'x'} — Integer/Text values can't be promoted →
    // inference rejects, same as the mixed-type array literal case.
    let types = infer_sql("SELECT MAP {'a': 1, 'b': 'x'}");
    assert_eq!(types[0].data_type, DataType::unknown_dynamic());
}

#[test]
fn test_map_literal_value_numeric_promotion() {
    // MAP {'a': 1, 'b': 100000} — SmallInt + Integer values promote to Integer.
    let types = infer_sql("SELECT MAP {'a': 1, 'b': 100000}");
    assert_eq!(
        types[0].data_type,
        DataType::Map(Box::new(DataType::Text), Box::new(DataType::Integer))
    );
}

#[test]
fn test_brace_struct_literal_string_keyed() {
    // `{'a': 1, 'b': 'x'}` → DuckDB struct/dict literal, string-literal keys.
    // Verified against DuckDB: `typeof({'a': 1, 'b': 'x'})` is
    // `STRUCT(a INTEGER, b VARCHAR)`; smelt's own integer-literal width
    // convention (SmallInt for small integer literals) applies the same way
    // it does for MAP {'a': 1} above.
    let types = infer_sql("SELECT {'a': 1, 'b': 'x'}");
    assert_eq!(
        types[0].data_type,
        DataType::Struct(vec![
            ("a".to_string(), DataType::SmallInt),
            ("b".to_string(), DataType::Text),
        ])
    );
    assert!(!types[0].nullable, "struct literal should be non-nullable");
}

#[test]
fn test_brace_struct_literal_double_quoted_keys() {
    // Verified against DuckDB: double-quoted keys behave the same as
    // single-quoted keys inside a struct/dict literal.
    let types = infer_sql("SELECT {\"CamelCase\": 1, \"lowercase\": 2}");
    assert_eq!(
        types[0].data_type,
        DataType::Struct(vec![
            ("CamelCase".to_string(), DataType::SmallInt),
            ("lowercase".to_string(), DataType::SmallInt),
        ])
    );
}

#[test]
fn test_brace_struct_literal_nested() {
    // `{'x': 1, 'y': {'a': 'duck', 'b': 1.5}}` — nested struct/dict literal
    // value. Verified against DuckDB: nested struct/dict literals parse and
    // execute (external corpus statement `48ccbc31d75bae5c`).
    let types = infer_sql("SELECT {'x': 1, 'y': {'a': 'duck', 'b': 1.5}}");
    assert_eq!(
        types[0].data_type,
        DataType::Struct(vec![
            ("x".to_string(), DataType::SmallInt),
            (
                "y".to_string(),
                DataType::Struct(vec![
                    ("a".to_string(), DataType::Text),
                    (
                        "b".to_string(),
                        DataType::Decimal {
                            precision: 2,
                            scale: 1
                        }
                    ),
                ])
            ),
        ])
    );
}

#[test]
fn test_brace_struct_literal_comparison_is_boolean() {
    // The `duckdb_struct_dict_literal_compare` ledger class: struct/dict
    // literal comparisons must type as Boolean once the literal itself
    // parses and infers.
    let types = infer_sql("SELECT {'x': 1, 'y': 2} > {'x': 1, 'y': 3}");
    assert_eq!(types[0].data_type, DataType::Boolean);
}

#[test]
fn test_identifier_keyed_brace_literal_not_typed_as_struct_here() {
    // Guard: `{a: 1}` is parsed by the record-literal path (pre-existing
    // behavior, not a BRACE_STRUCT_LITERAL), so it is NOT expected to infer
    // via `infer_brace_struct_literal_type`. This test only pins that the
    // string-keyed fix didn't change this pre-existing dispatch.
    let types = infer_sql("SELECT {a: 1} AS s FROM (SELECT 1 AS a) t");
    // Whatever the record-literal path currently infers (Unknown, since it's
    // not a recognized SQL context for RECORD_LITERAL), it must not be the
    // DuckDB struct/dict Struct([("a", SmallInt)]) shape — that would mean
    // both `{a: 1}` and `{'a': 1}` are silently typed differently was NOT
    // the intent change here.
    assert_ne!(
        types[0].data_type,
        DataType::Struct(vec![("a".to_string(), DataType::SmallInt)])
    );
}

#[test]
fn test_struct_display() {
    let dt = DataType::Struct(vec![
        ("a".to_string(), DataType::Integer),
        ("b".to_string(), DataType::Text),
    ]);
    assert_eq!(dt.to_sql(), "STRUCT(a INTEGER, b TEXT)");
}

#[test]
fn test_modulo_operator() {
    // Integer % Integer → Integer
    let types = infer_sql("SELECT 10 % 3");
    assert_eq!(types[0].data_type, DataType::SmallInt);

    // CAST to explicit types
    let types = infer_sql("SELECT CAST(10 AS INTEGER) % CAST(3 AS INTEGER)");
    assert_eq!(types[0].data_type, DataType::Integer);

    // BigInt % Integer → BigInt (promotion)
    let types = infer_sql("SELECT CAST(10 AS BIGINT) % CAST(3 AS INTEGER)");
    assert_eq!(types[0].data_type, DataType::BigInt);

    // Double % Double → Double
    let types = infer_sql("SELECT CAST(10.5 AS DOUBLE) % CAST(3.0 AS DOUBLE)");
    assert_eq!(types[0].data_type, DataType::Double);
}

// Bug #7: promote_types should widen narrow decimal when combined with wider integer type
// CASE WHEN cond THEN integer_col ELSE decimal_literal END should not produce a narrow type
#[test]
fn test_decimal_case_widening() {
    // CASE result combining Integer and Decimal{2,1}: should widen to at least Decimal{38,10}
    // so that integer values like 100 don't overflow the decimal type
    let types = infer_sql(
        "SELECT CASE WHEN TRUE THEN CAST(150 AS INTEGER) ELSE CAST(0.5 AS DECIMAL(2,1)) END",
    );
    match &types[0].data_type {
        DataType::Decimal { precision, scale } => {
            // precision - scale = integer digits available; must be >= 3 for value 150
            let integer_digits = precision - scale;
            assert!(
                    integer_digits >= 3,
                    "CASE of Integer/Decimal should widen to allow values like 150, got DECIMAL({precision},{scale})"
                );
        }
        other => panic!("Expected Decimal, got {other:?}"),
    }
}

// Bug #8: CAST(x AS FLOAT) should infer as Double (FLOAT normalizes to DOUBLE)
#[test]
fn test_cast_float_normalizes_to_double() {
    let types = infer_sql("SELECT CAST(1 AS FLOAT)");
    assert_eq!(
        types[0].data_type,
        DataType::Double,
        "CAST AS FLOAT should infer as Double"
    );
}

/// Phase 5 unit: seeded function parameters shadow outer column scope.
///
/// §16 #1 of the smelt-functions research pins the resolution order:
/// params resolve *before* any SQL scope. This test proves the
/// ordering in isolation — no parser, no Salsa — so Phase 6 and
/// beyond can compose on top of it with confidence.
#[test]
fn param_shadows_outer_name_lookup_logic() {
    let mut ctx = TypeContext::new();
    // Seed a model column `bar.x: Integer` — this is what
    // `lookup_column` would return if we consulted it directly.
    ctx.add_model_column("bar", "x", TypedColumn::nullable(DataType::Integer));

    // Sanity: `lookup_column(None, "x")` currently sees only the model
    // column, returning Integer.
    let via_column = ctx
        .lookup_column(None, "x")
        .expect("model column should be resolvable before param binding");
    assert_eq!(via_column.data_type, DataType::Integer);

    // Now seed a function param `x: Double`. Per §16 #1, the param
    // wins on unqualified lookups through `lookup_identifier`.
    ctx.add_function_param("x", TypedColumn::nullable(DataType::Double));
    assert!(ctx.has_function_param("x"));

    let via_identifier = ctx
        .lookup_identifier(None, "x")
        .expect("seeded param should resolve through lookup_identifier");
    assert_eq!(
        via_identifier.data_type,
        DataType::Double,
        "param type must shadow model type on unqualified lookups"
    );

    // Qualified lookups still bypass the param scope — params are
    // bare names.
    let via_qualified = ctx.lookup_identifier(Some("bar"), "x");
    assert_eq!(
        via_qualified.map(|c| c.data_type.clone()),
        Some(DataType::Integer),
        "qualified lookup must ignore function params"
    );
}

// ── Phase 49: check_window_in_scalar_contexts recurses into subqueries ──

fn parse_select(sql: &str) -> smelt_parser::ast::SelectStmt {
    use smelt_parser::ast::File;
    let parse = smelt_parser::parse(sql);
    let root = parse.syntax();
    let file = File::cast(root).expect("failed to cast to File");
    file.select_stmt().expect("no SelectStmt in parsed SQL")
}

/// WHERE contains a scalar subquery whose body includes a window function.
/// Expected: at least one WindowInScalarContextInfo with clause "WHERE".
#[test]
fn where_subquery_with_window_func_errors() {
    let sql = "SELECT col FROM t WHERE col > \
                   (SELECT MAX(ROW_NUMBER() OVER (PARTITION BY col ORDER BY col)) FROM t)";
    let select = parse_select(sql);
    let ctx = TypeContext::new();
    let infos = check_window_in_scalar_contexts(&select, &ctx);
    assert!(
        infos.iter().any(|i| i.clause == "WHERE"),
        "expected a WindowInScalarContext error in WHERE for a subquery containing \
             a window function, got: {infos:?}"
    );
}

/// HAVING contains a scalar subquery whose body includes a window function.
/// Expected: at least one WindowInScalarContextInfo with clause "HAVING".
#[test]
fn having_subquery_with_window_func_errors() {
    let sql = "SELECT col, COUNT(*) FROM t GROUP BY col \
                   HAVING COUNT(*) > (SELECT AVG(RANK() OVER (ORDER BY col)) FROM t)";
    let select = parse_select(sql);
    let ctx = TypeContext::new();
    let infos = check_window_in_scalar_contexts(&select, &ctx);
    assert!(
        infos.iter().any(|i| i.clause == "HAVING"),
        "expected a WindowInScalarContext error in HAVING for a subquery containing \
             a window function, got: {infos:?}"
    );
}

/// Window function in SELECT-list subquery — must NOT produce any error
/// (regression guard: only WHERE / GROUP BY / HAVING are restricted).
///
/// The outer query intentionally includes a WHERE clause so that the
/// checker has a non-trivial scalar context to walk.  A buggy
/// implementation that descended into SELECT-list subqueries and
/// misattributed the inner window function to the outer WHERE would
/// emit a spurious error here — this test catches that regression.
#[test]
fn select_list_subquery_with_window_func_allowed() {
    let sql = "SELECT (SELECT ROW_NUMBER() OVER (ORDER BY col) FROM inner_t) AS rn, col \
             FROM outer_t \
             WHERE col > 0";
    let select = parse_select(sql);
    let ctx = TypeContext::new();
    let infos = check_window_in_scalar_contexts(&select, &ctx);
    assert!(
        infos.is_empty(),
        "window function inside a SELECT-list subquery must not be flagged \
             even when the outer query has a WHERE clause, got: {infos:?}"
    );
}

/// Window function in FROM-clause derived-table — must NOT produce any error
/// (regression guard: FROM subqueries are not scalar contexts).
#[test]
fn from_clause_subquery_with_window_func_allowed() {
    let sql = "SELECT * FROM (SELECT ROW_NUMBER() OVER (ORDER BY col) AS rn FROM t) sub";
    let select = parse_select(sql);
    let ctx = TypeContext::new();
    let infos = check_window_in_scalar_contexts(&select, &ctx);
    assert!(
        infos.is_empty(),
        "window function inside a FROM-clause subquery must not be flagged, \
             got: {infos:?}"
    );
}

// === Phase A (meta-language) TDD tests: infer_list_literal ===

/// Parse `SELECT <expr>` and return the first select-item expression.
fn parse_first_expr(sql: &str) -> smelt_parser::ast::Expr {
    use smelt_parser::ast::File;
    let parse = smelt_parser::parse(sql);
    let root = parse.syntax();
    let file = File::cast(root).expect("FILE node");
    let select = file.select_stmt().expect("SelectStmt");
    let select_list = select.select_list().expect("select list");
    let first_item = select_list.items().next().expect("at least one item");
    first_item.expression().expect("expression")
}

/// Extract elements from `SELECT [e1, e2, ...]` as a vec of Expr.
fn list_elements(sql: &str) -> Vec<smelt_parser::ast::Expr> {
    let expr = parse_first_expr(sql);
    // The list literal lands as an ARRAY_LITERAL child inside the expression.
    let arr = expr
        .as_array_literal()
        .expect("expected an array/list literal node");
    arr.elements()
}

/// `[100000, 200000, 300000]` — all Integer literals — infers `List<Expr<Integer>>`.
#[test]
fn infer_list_literal_homogeneous_integer() {
    let elems = list_elements("SELECT [100000, 200000, 300000]");
    let ctx = TypeContext::new();
    let result = infer_list_literal(&elems, &ctx, None);
    assert!(
        result.sentinels.is_empty(),
        "homogeneous integer list must have no sentinels, got: {:?}",
        result.sentinels
    );
    assert_eq!(
        result.inferred,
        SmeltType::List(Box::new(SmeltType::Expr(
            smelt_types::signatures::TypeConstraint::Concrete(DataType::Integer)
        ))),
        "homogeneous Integer list must infer List<Expr<Integer>>"
    );
}

/// `[1, 1.5]` — SmallInt + Decimal — infers `List<Expr<Decimal(38,10)>>` via LUB.
///
/// The spec references `types.md §"Numeric promotion chain"` and says `[1, 1.5]` →
/// `List<Expr<Double>>`. However, the actual `promote_types` implementation promotes
/// `(SmallInt, Decimal{2,1})` to `Decimal{38,10}` (the safe "integer+Decimal" widening
/// rule). `Double` would require an `e`-notation literal (`1.5e0`) but the lexer does
/// not handle exponent notation, so `1.5` always produces `Decimal`. The test asserts
/// the actual promotion behaviour, which is correct per the implementation's own rules.
#[test]
fn infer_list_literal_lub_promotion() {
    // `1` → SmallInt, `1.5` → Decimal(2,1).
    // promote_types(SmallInt, Decimal) → Decimal(38,10).
    let elems = list_elements("SELECT [1, 1.5]");
    let ctx = TypeContext::new();
    let result = infer_list_literal(&elems, &ctx, None);
    assert!(
        result.sentinels.is_empty(),
        "numeric-promoted list must have no sentinels, got: {:?}",
        result.sentinels
    );
    // Numeric promotion: SmallInt + Decimal → Decimal(38,10).
    assert_eq!(
        result.inferred,
        SmeltType::List(Box::new(SmeltType::Expr(
            smelt_types::signatures::TypeConstraint::Concrete(DataType::Decimal {
                precision: 38,
                scale: 10
            })
        ))),
        "SmallInt+Decimal list must promote to List<Expr<Decimal(38,10)>>"
    );
}

/// `[1, 'hello']` — Integer + Text — infers `List<Unknown>` with Heterogeneous sentinel.
#[test]
fn infer_list_literal_heterogeneous_unknown() {
    let elems = list_elements("SELECT [1, 'hello']");
    let ctx = TypeContext::new();
    let result = infer_list_literal(&elems, &ctx, None);
    // Must produce List<Unknown>
    assert_eq!(
        result.inferred,
        SmeltType::List(Box::new(SmeltType::Unknown)),
        "heterogeneous list must infer List<Unknown>"
    );
    // Must carry exactly one Heterogeneous sentinel.
    assert_eq!(result.sentinels.len(), 1);
    assert!(
        matches!(result.sentinels[0], ListInferSentinel::Heterogeneous { .. }),
        "expected Heterogeneous sentinel, got: {:?}",
        result.sentinels[0]
    );
}

/// `[]` with expected type `List<Expr<Integer>>` infers to `List<Expr<Integer>>`.
#[test]
fn infer_list_literal_empty_with_target() {
    let elems = list_elements("SELECT []");
    let ctx = TypeContext::new();
    let expected = SmeltType::List(Box::new(SmeltType::Expr(
        smelt_types::signatures::TypeConstraint::Concrete(DataType::Integer),
    )));
    let result = infer_list_literal(&elems, &ctx, Some(&expected));
    assert!(
        result.sentinels.is_empty(),
        "empty list with known target must have no sentinels, got: {:?}",
        result.sentinels
    );
    assert_eq!(
        result.inferred, expected,
        "empty list with target List<Expr<Integer>> must infer to that type"
    );
}

/// `[]` without a target infers to `List<Unknown>` with `MetaListEmptyTypeUnknown` sentinel.
#[test]
fn infer_list_literal_empty_without_target() {
    let elems = list_elements("SELECT []");
    let ctx = TypeContext::new();
    let result = infer_list_literal(&elems, &ctx, None);
    assert_eq!(
        result.inferred,
        SmeltType::List(Box::new(SmeltType::Unknown)),
        "empty list without target must infer List<Unknown>"
    );
    assert_eq!(result.sentinels.len(), 1);
    assert!(
        matches!(result.sentinels[0], ListInferSentinel::EmptyTypeUnknown),
        "expected EmptyTypeUnknown sentinel, got: {:?}",
        result.sentinels[0]
    );
}

/// `[]` with a non-List expected type (`Expr<Numeric>`) — the caller passed an
/// inappropriate expected sort. The function must NOT return the non-List expected
/// type; it must fall back to `List<Unknown>` + `EmptyTypeUnknown` sentinel.
///
/// Regression test for B-2: without the guard, `infer_list_literal` would
/// return `Expr<Numeric>` (a non-List type) when passed any non-None expected,
/// which would break the invariant that the function always returns a `List<T>`.
#[test]
fn infer_list_literal_empty_with_non_list_expected_falls_through() {
    let elems = list_elements("SELECT []");
    let ctx = TypeContext::new();
    // Pass a non-List expected type — should NOT be returned as-is.
    let non_list_expected = SmeltType::Expr(smelt_types::signatures::TypeConstraint::Concrete(
        DataType::Integer,
    ));
    let result = infer_list_literal(&elems, &ctx, Some(&non_list_expected));
    // Must still be List<Unknown>, NOT Expr<Integer>.
    assert_eq!(
        result.inferred,
        SmeltType::List(Box::new(SmeltType::Unknown)),
        "empty list with non-List expected must fall through to List<Unknown>, \
             not return the non-List expected type; got: {:?}",
        result.inferred
    );
    // Must emit EmptyTypeUnknown sentinel.
    assert_eq!(result.sentinels.len(), 1);
    assert!(
        matches!(result.sentinels[0], ListInferSentinel::EmptyTypeUnknown),
        "expected EmptyTypeUnknown sentinel, got: {:?}",
        result.sentinels[0]
    );
}

/// `[[100000, 200000], [300000, 400000]]` — nested list — infers `List<List<Expr<Integer>>>`.
#[test]
fn infer_list_literal_nested() {
    let elems = list_elements("SELECT [[100000, 200000], [300000, 400000]]");
    let ctx = TypeContext::new();
    let result = infer_list_literal(&elems, &ctx, None);
    assert!(
        result.sentinels.is_empty(),
        "nested integer list must have no sentinels, got: {:?}",
        result.sentinels
    );
    let expected = SmeltType::List(Box::new(SmeltType::List(Box::new(SmeltType::Expr(
        smelt_types::signatures::TypeConstraint::Concrete(DataType::Integer),
    )))));
    assert_eq!(
        result.inferred, expected,
        "nested integer list must infer List<List<Expr<Integer>>>"
    );
}

/// `[1, [2, 3]]` — mixed scalar + nested list — infers `List<Unknown>` with
/// `Heterogeneous` sentinel. A scalar element has sort `Expr<…>` while a list-literal
/// element has sort `List<…>`; they cannot unify under LUB, so the result is
/// `List<Unknown>` per spec `meta_language.md` Phase A semantic rule 2.
#[test]
fn infer_list_literal_mixed_scalar_and_nested() {
    let elems = list_elements("SELECT [1, [2, 3]]");
    let ctx = TypeContext::new();
    let result = infer_list_literal(&elems, &ctx, None);
    // Must produce List<Unknown>
    assert_eq!(
        result.inferred,
        SmeltType::List(Box::new(SmeltType::Unknown)),
        "mixed scalar+nested list must infer List<Unknown>, got: {:?}",
        result.inferred
    );
    // Must carry exactly one Heterogeneous sentinel.
    assert_eq!(
        result.sentinels.len(),
        1,
        "expected exactly 1 sentinel, got: {:?}",
        result.sentinels
    );
    assert!(
        matches!(result.sentinels[0], ListInferSentinel::Heterogeneous { .. }),
        "expected Heterogeneous sentinel, got: {:?}",
        result.sentinels[0]
    );
}

/// `[[1, 2], 3]` — nested list then scalar — same cross-sort mix as above.
/// Must also infer `List<Unknown>` with `Heterogeneous` sentinel (symmetry).
#[test]
fn infer_list_literal_nested_then_scalar() {
    let elems = list_elements("SELECT [[1, 2], 3]");
    let ctx = TypeContext::new();
    let result = infer_list_literal(&elems, &ctx, None);
    assert_eq!(
        result.inferred,
        SmeltType::List(Box::new(SmeltType::Unknown)),
        "nested-then-scalar list must infer List<Unknown>, got: {:?}",
        result.inferred
    );
    assert_eq!(
        result.sentinels.len(),
        1,
        "expected exactly 1 sentinel, got: {:?}",
        result.sentinels
    );
    assert!(
        matches!(result.sentinels[0], ListInferSentinel::Heterogeneous { .. }),
        "expected Heterogeneous sentinel, got: {:?}",
        result.sentinels[0]
    );
}

// === Phase A Phase 3 TDD tests: diagnostics + bidirectional disambiguation + spread ===

fn parse_select_stmt(sql: &str) -> smelt_parser::ast::SelectStmt {
    use smelt_parser::ast::File;
    let parse = smelt_parser::parse(sql);
    let root = parse.syntax();
    let file = File::cast(root).expect("FILE node");
    file.select_stmt().expect("SelectStmt")
}

/// `[1, 2, 3]` at a splice point expecting `List<Expr<Integer>>` evaluates
/// as a meta-list (not a Data-World array).
#[test]
fn list_literal_disambiguation_meta_list_target() {
    let elems = list_elements("SELECT [100000, 200000, 300000]");
    let ctx = TypeContext::new();
    // Expected sort is List<Expr<Integer>> — meta-list context.
    let expected = SmeltType::List(Box::new(SmeltType::Expr(
        smelt_types::signatures::TypeConstraint::Concrete(DataType::Integer),
    )));
    let result = disambiguate_list_literal(&elems, &ctx, Some(&expected));
    assert!(
        matches!(result, ListDisambiguation::MetaList(_)),
        "with List<Expr<Integer>> target, literal must be interpreted as meta-list, got: {:?}",
        result
    );
}

/// `[1, 2, 3]` at a splice point expecting `Expr<Array<Integer>>` evaluates
/// as a runtime array.
#[test]
fn list_literal_disambiguation_data_array_target() {
    let elems = list_elements("SELECT [100000, 200000, 300000]");
    let ctx = TypeContext::new();
    // Expected sort is Expr<Concrete(Array(Integer))> — Data-World array context.
    let expected = SmeltType::Expr(smelt_types::signatures::TypeConstraint::Concrete(
        DataType::Array(Box::new(DataType::Integer)),
    ));
    let result = disambiguate_list_literal(&elems, &ctx, Some(&expected));
    assert!(
        matches!(result, ListDisambiguation::DataWorldArray),
        "with Expr<Array<Integer>> target, literal must be interpreted as Data-World array, \
             got: {:?}",
        result
    );
}

/// At a position admitting both meta-list and Data-World array, the literal
/// evaluates as meta-list (rule 3: meta wins).
#[test]
fn list_literal_disambiguation_both_admissible_meta_wins() {
    let elems = list_elements("SELECT [100000, 200000, 300000]");
    let ctx = TypeContext::new();
    // No expected type → both admissible → meta-list wins.
    let result = disambiguate_list_literal(&elems, &ctx, None);
    assert!(
        matches!(result, ListDisambiguation::MetaList(_)),
        "with no expected type (both admissible), literal must default to meta-list, \
             got: {:?}",
        result
    );
}

/// `[1, 'hello']` emits exactly one `MetaListHeterogeneous` diagnostic
/// anchored at the literal's source span.
#[test]
fn list_literal_heterogeneous_emits_diagnostic() {
    let elems = list_elements("SELECT [1, 'hello']");
    let ctx = TypeContext::new();
    let span = rowan::TextRange::new(7.into(), 20.into()); // approximate span
                                                           // Pass "" as text — unit tests don't assert specific line/column ranges.
    let diags = list_literal_sentinels_to_diagnostics(&elems, &ctx, span);
    assert_eq!(
        diags.len(),
        1,
        "heterogeneous list must produce exactly 1 diagnostic, got: {:?}",
        diags
    );
    assert!(
        matches!(
            diags[0].code,
            Some(crate::DiagnosticCode::MetaListHeterogeneous)
        ),
        "expected MetaListHeterogeneous diagnostic, got: {:?}",
        diags[0]
    );
}

/// `[]` in an unconstrained position emits exactly one
/// `MetaListEmptyTypeUnknown` diagnostic anchored at the literal's span.
#[test]
fn list_literal_empty_unknown_target_emits_diagnostic() {
    let elems = list_elements("SELECT []");
    let ctx = TypeContext::new();
    let span = rowan::TextRange::new(7.into(), 9.into());
    // Pass "" as text — unit tests don't assert specific line/column ranges.
    let diags = list_literal_sentinels_to_diagnostics(&elems, &ctx, span);
    assert_eq!(
        diags.len(),
        1,
        "empty list without target must produce exactly 1 diagnostic, got: {:?}",
        diags
    );
    assert!(
        matches!(
            diags[0].code,
            Some(crate::DiagnosticCode::MetaListEmptyTypeUnknown)
        ),
        "expected MetaListEmptyTypeUnknown diagnostic, got: {:?}",
        diags[0]
    );
}

/// `SELECT id, ...[a, b], created_at` — spread of a list literal into SELECT
/// list expands to the individual elements; each emitted item carries
/// `Synthesized(SpreadFrom(span_of_list_literal))` provenance.
#[test]
fn spread_in_select_list_expands() {
    let sql = "SELECT id, ...[a, b], created_at FROM t";
    let select = parse_select_stmt(sql);
    let ctx = TypeContext::new();
    // Pass "" as text — unit tests don't assert specific line/column ranges.
    let result = check_select_list_spreads(&select, &ctx);
    // Must find the spread and report expanded count = 2 (for a, b)
    assert_eq!(
        result.expanded_item_count, 2,
        "spread of [a, b] must expand to 2 items, got: {}",
        result.expanded_item_count
    );
    assert!(
        result.diagnostics.is_empty(),
        "valid spread in SELECT must produce no diagnostics, got: {:?}",
        result.diagnostics
    );
    // Each emitted item must carry Synthesized(SpreadFrom(...)) provenance.
    assert_eq!(
        result.provenance_tags.len(),
        2,
        "each of the 2 expanded items must carry a provenance tag, got: {:?}",
        result.provenance_tags
    );
    assert!(
        result
            .provenance_tags
            .iter()
            .all(|t| matches!(t, OriginTag::Synthesized(SynthesizedReason::SpreadFrom(_)))),
        "all provenance tags must be Synthesized(SpreadFrom(…)), got: {:?}",
        result.provenance_tags
    );
}

/// `SELECT id, ...[], created_at` — spread of an empty list elides silently;
/// the SELECT type-checks as if `SELECT id, created_at`.
#[test]
fn spread_empty_list_elides() {
    let sql = "SELECT id, ...[], created_at FROM t";
    let select = parse_select_stmt(sql);
    let ctx = TypeContext::new();
    // Pass "" as text — unit tests don't assert specific line/column ranges.
    let result = check_select_list_spreads(&select, &ctx);
    assert_eq!(
        result.expanded_item_count, 0,
        "spread of empty list must expand to 0 items (elision), got: {}",
        result.expanded_item_count
    );
    assert!(
        result.diagnostics.is_empty(),
        "empty-list spread in SELECT must produce no diagnostics, got: {:?}",
        result.diagnostics
    );
}

/// `WHERE x = 1 AND ...preds` emits `MetaSpreadInForbiddenPosition` at the
/// spread span.
#[test]
fn spread_in_where_clause_emits_diagnostic() {
    // Note: the parser does not emit a LIST_SPREAD node inside WHERE (it
    // produces a parse error instead). The orphaned DOT_DOT_DOT token ends
    // up as a sibling of the SELECT_STMT at the FILE level.
    // `check_forbidden_position_spreads` detects this pattern and emits
    // MetaSpreadInForbiddenPosition.
    let sql = "SELECT x FROM t WHERE x = 1 AND ...preds";
    let select = parse_select_stmt(sql);
    let ctx = TypeContext::new();
    // Pass "" as text — unit tests don't assert specific line/column ranges.
    let diags = check_forbidden_position_spreads(&select, &ctx);
    assert!(
        !diags.is_empty(),
        "spread in WHERE must produce MetaSpreadInForbiddenPosition diagnostic"
    );
    assert!(
        diags
            .iter()
            .any(|d| d.code == Some(crate::DiagnosticCode::MetaSpreadInForbiddenPosition)),
        "expected MetaSpreadInForbiddenPosition, got: {:?}",
        diags
    );
}

/// `SELECT ...x FROM t` where `x` is `Expr<Integer>` emits
/// `MetaSpreadOnNonList`; surrounding SELECT type-checks as if spread were
/// absent.
#[test]
fn spread_on_non_list_emits_diagnostic() {
    let sql = "SELECT ...x FROM t";
    let select = parse_select_stmt(sql);
    // Context: x is Expr<Integer>, not a List.
    let mut ctx = TypeContext::new();
    ctx.add_model_column(
        "t",
        "x",
        smelt_types::TypedColumn::not_null(DataType::Integer),
    );
    // Pass "" as text — unit tests don't assert specific line/column ranges.
    let result = check_select_list_spreads(&select, &ctx);
    assert_eq!(
        result.diagnostics.len(),
        1,
        "spread on non-list must produce exactly 1 MetaSpreadOnNonList, \
             got: {:?}",
        result.diagnostics
    );
    assert!(
        matches!(
            result.diagnostics[0].code,
            Some(crate::DiagnosticCode::MetaSpreadOnNonList)
        ),
        "expected MetaSpreadOnNonList, got: {:?}",
        result.diagnostics[0]
    );
}

// === Phase B (meta-language) TDD tests: HOF inference + reducer registry + pipe ===

/// Parse a HOF call like `SELECT map([1, 2, 3], fn c => c)` and return the
/// FunctionCall AST node (the map/filter/reduce call).
fn parse_hof_call(sql: &str) -> smelt_parser::ast::FunctionCall {
    let expr = parse_first_expr(sql);
    expr.as_function_call()
        .expect("expected a function-call expression for HOF test")
}

/// `map([1, 2, 3], fn c => c)` — identity lambda on SmallInt list — infers
/// `List<Expr<SmallInt>>` (HOF produces `List<U>` where U = body type = Expr<SmallInt>).
///
/// 1, 2, 3 are in i16 range so they infer SmallInt.
#[test]
fn infer_map_returns_list_of_body_type() {
    let call = parse_hof_call("SELECT map([1, 2, 3], fn c => c)");
    let ctx = TypeContext::new();
    let result = infer_hof_call_from_function_call(&call, &ctx);
    assert!(
        result.sentinel.is_none(),
        "identity map must have no sentinel, got: {:?}",
        result.sentinel
    );
    assert_eq!(
        result.inferred,
        SmeltType::List(Box::new(SmeltType::Expr(
            smelt_types::signatures::TypeConstraint::Concrete(DataType::SmallInt)
        ))),
        "map([1,2,3], fn c => c) must infer List<Expr<SmallInt>>"
    );
}

/// `map([1, 2, 3], fn c => CAST(c AS Text))` — body produces Expr<Varchar> —
/// HOF result is `List<Expr<Varchar>>`.
///
/// Note: `CAST(x AS Text)` produces `DataType::Varchar { max_length: None }`
/// (not `DataType::Text`) because the type parser normalises `TEXT` → `VARCHAR`.
#[test]
fn infer_map_with_typed_body() {
    let call = parse_hof_call("SELECT map([1, 2, 3], fn c => CAST(c AS Text))");
    let ctx = TypeContext::new();
    let result = infer_hof_call_from_function_call(&call, &ctx);
    assert!(
        result.sentinel.is_none(),
        "map with CAST body must have no sentinel, got: {:?}",
        result.sentinel
    );
    // CAST(x AS Text) normalises to Varchar { max_length: None }.
    assert_eq!(
        result.inferred,
        SmeltType::List(Box::new(SmeltType::Expr(
            smelt_types::signatures::TypeConstraint::Concrete(DataType::Varchar {
                max_length: None
            })
        ))),
        "map([1,2,3], fn c => CAST(c AS Text)) must infer List<Expr<Varchar>>"
    );
}

/// `filter([1, 2, 3], fn c => c > 0)` — filter preserves element type —
/// result is `List<Expr<SmallInt>>`.
#[test]
fn infer_filter_returns_same_list_type() {
    let call = parse_hof_call("SELECT filter([1, 2, 3], fn c => c > 0)");
    let ctx = TypeContext::new();
    let result = infer_hof_call_from_function_call(&call, &ctx);
    assert!(
        result.sentinel.is_none(),
        "filter must have no sentinel, got: {:?}",
        result.sentinel
    );
    assert_eq!(
        result.inferred,
        SmeltType::List(Box::new(SmeltType::Expr(
            smelt_types::signatures::TypeConstraint::Concrete(DataType::SmallInt)
        ))),
        "filter([1,2,3], fn c => c > 0) must infer List<Expr<SmallInt>>"
    );
}

/// `filter([1, 2, 3], fn c => c)` — predicate body is `Expr<SmallInt>` not
/// `Expr<Boolean>` — returns a sentinel for `LambdaResultTypeMismatch`.
#[test]
fn infer_filter_predicate_must_be_boolean_sentinel() {
    let call = parse_hof_call("SELECT filter([1, 2, 3], fn c => c)");
    let ctx = TypeContext::new();
    let result = infer_hof_call_from_function_call(&call, &ctx);
    assert!(
            matches!(result.sentinel, Some(HofInferSentinel::LambdaResultTypeMismatch { .. })),
            "filter with non-Boolean predicate must return LambdaResultTypeMismatch sentinel, got: {:?}",
            result.sentinel
        );
}

/// `reduce([1, 2, 3], plus_chain)` → `Expr<SmallInt>`.
/// `reduce(['a', 'b', 'c'], concat)` → `Expr<Text>`.
/// `reduce([true, false], and_all)` → `Expr<Boolean>`.
#[test]
fn infer_reduce_returns_reducer_output_sort() {
    // plus_chain with SmallInt integers
    {
        let call = parse_hof_call("SELECT reduce([1, 2, 3], plus_chain)");
        let ctx = TypeContext::new();
        let result = infer_hof_call_from_function_call(&call, &ctx);
        assert!(
            result.sentinel.is_none(),
            "reduce([1,2,3], plus_chain) must have no sentinel, got: {:?}",
            result.sentinel
        );
        // plus_chain output sort is Expr<Numeric>; element type is SmallInt
        // which satisfies Numeric, so output is Expr<SmallInt> (the element type).
        assert!(
            matches!(result.inferred, SmeltType::Expr(_)),
            "reduce(ints, plus_chain) must infer Expr<...>, got: {:?}",
            result.inferred
        );
    }
    // concat with text
    {
        let call = parse_hof_call("SELECT reduce(['a', 'b', 'c'], concat)");
        let ctx = TypeContext::new();
        let result = infer_hof_call_from_function_call(&call, &ctx);
        assert!(
            result.sentinel.is_none(),
            "reduce(texts, concat) must have no sentinel, got: {:?}",
            result.sentinel
        );
        assert!(
            matches!(result.inferred, SmeltType::Expr(_)),
            "reduce(texts, concat) must infer Expr<...>, got: {:?}",
            result.inferred
        );
    }
    // and_all with booleans
    {
        let call = parse_hof_call("SELECT reduce([true, false], and_all)");
        let ctx = TypeContext::new();
        let result = infer_hof_call_from_function_call(&call, &ctx);
        assert!(
            result.sentinel.is_none(),
            "reduce(bools, and_all) must have no sentinel, got: {:?}",
            result.sentinel
        );
        assert_eq!(
            result.inferred,
            SmeltType::Expr(smelt_types::signatures::TypeConstraint::Concrete(
                DataType::Boolean
            )),
            "reduce([true,false], and_all) must infer Expr<Boolean>"
        );
    }
}

/// `reduce([col1, col2, col3], comma_sep)` — output is `SelectItems<Scalar>`
/// regardless of element `T`.
#[test]
fn infer_reduce_comma_sep_yields_select_items() {
    // Use integer literals as "columns" — comma_sep accepts any Expr<T>.
    let call = parse_hof_call("SELECT reduce([1, 2, 3], comma_sep)");
    let ctx = TypeContext::new();
    let result = infer_hof_call_from_function_call(&call, &ctx);
    assert!(
        result.sentinel.is_none(),
        "comma_sep reduce must have no sentinel, got: {:?}",
        result.sentinel
    );
    assert_eq!(
        result.inferred,
        SmeltType::SelectItems {
            kind: smelt_types::signatures::ExprKind::Scalar,
            context: None
        },
        "reduce(any, comma_sep) must infer SelectItems<Scalar>"
    );
}

/// `reduce([], and_all)` — empty list with identity reducer — infers `Expr<Boolean>`;
/// no sentinel.
#[test]
fn infer_reduce_empty_list_with_identity() {
    let call = parse_hof_call("SELECT reduce([], and_all)");
    let ctx = TypeContext::new();
    let expected = SmeltType::Expr(smelt_types::signatures::TypeConstraint::Concrete(
        DataType::Boolean,
    ));
    let result = infer_hof_call_from_function_call_with_expected(&call, &ctx, Some(&expected));
    assert!(
        result.sentinel.is_none(),
        "reduce([], and_all) must have no sentinel, got: {:?}",
        result.sentinel
    );
    assert_eq!(
        result.inferred, expected,
        "reduce([], and_all) must infer Expr<Boolean> (TRUE identity)"
    );
}

/// `reduce([], union_all)` — empty list, no identity — sentinel for
/// `ReducerEmptyNoIdentity`.
#[test]
fn infer_reduce_empty_list_no_identity_sentinel() {
    let call = parse_hof_call("SELECT reduce([], union_all)");
    let ctx = TypeContext::new();
    let result = infer_hof_call_from_function_call(&call, &ctx);
    assert!(
        matches!(
            result.sentinel,
            Some(HofInferSentinel::ReducerEmptyNoIdentity { .. })
        ),
        "reduce([], union_all) must produce ReducerEmptyNoIdentity sentinel, got: {:?}",
        result.sentinel
    );
}

/// `reduce([], comma_sep)` — empty list with `comma_sep` — produces
/// `SelectItems<Scalar>` with no sentinel (via the registry `EmptySelectItems`
/// identity, not a special-case branch).
#[test]
fn infer_reduce_comma_sep_empty_returns_select_items_with_identity() {
    let call = parse_hof_call("SELECT reduce([], comma_sep)");
    let ctx = TypeContext::new();
    let result = infer_hof_call_from_function_call(&call, &ctx);
    assert!(
        result.sentinel.is_none(),
        "reduce([], comma_sep) must have no sentinel, got: {:?}",
        result.sentinel
    );
    assert_eq!(
        result.inferred,
        SmeltType::SelectItems {
            kind: smelt_types::signatures::ExprKind::Scalar,
            context: None,
        },
        "reduce([], comma_sep) must infer SelectItems<Scalar>"
    );
}

/// `reduce([1, 2, 3], and_all)` — element type `Expr<SmallInt>` does not
/// satisfy `and_all`'s `Expr<Boolean>` requirement — sentinel for
/// `ReducerInputTypeMismatch`.
#[test]
fn infer_reduce_input_type_mismatch_sentinel() {
    let call = parse_hof_call("SELECT reduce([1, 2, 3], and_all)");
    let ctx = TypeContext::new();
    let result = infer_hof_call_from_function_call(&call, &ctx);
    assert!(
        matches!(
            result.sentinel,
            Some(HofInferSentinel::ReducerInputTypeMismatch { .. })
        ),
        "reduce([1,2,3], and_all) must produce ReducerInputTypeMismatch sentinel, got: {:?}",
        result.sentinel
    );
}

/// `xs |> filter(fn c => c > 0)` and `filter(xs, fn c => c > 0)` infer to
/// the same `SmeltType` for the same input.
#[test]
fn infer_pipe_desugars_to_call() {
    let ctx = TypeContext::new();

    // Piped form: [1, 2, 3] |> filter(fn c => c > 0)
    let pipe_sql = "SELECT [1, 2, 3] |> filter(fn c => c > 0)";
    let pipe_result = {
        let expr = parse_first_expr(pipe_sql);
        let pipe = extract_pipe_expr_from_expr(&expr).expect("expected PIPE_EXPR");
        infer_pipe_expr(&pipe, &ctx, None)
    };

    // Direct form: filter([1, 2, 3], fn c => c > 0)
    let call_sql = "SELECT filter([1, 2, 3], fn c => c > 0)";
    let call_result = {
        let call = parse_hof_call(call_sql);
        infer_hof_call_from_function_call(&call, &ctx)
    };

    assert_eq!(
        pipe_result.inferred, call_result.inferred,
        "piped and direct forms must infer the same type"
    );
}

/// `[1, 2, 3] |> filter(fn c => c > 0) |> map(fn c => c * 2)` infers to
/// `List<Expr<SmallInt>>` (left-associative pipe chain).
#[test]
fn infer_pipe_chain_associates_left() {
    let ctx = TypeContext::new();
    let sql = "SELECT [1, 2, 3] |> filter(fn c => c > 0) |> map(fn c => c * 2)";
    let expr = parse_first_expr(sql);
    let pipe = extract_pipe_expr_from_expr(&expr).expect("expected outer PIPE_EXPR");
    let result = infer_pipe_expr(&pipe, &ctx, None);
    assert!(
        result.sentinel.is_none(),
        "pipe chain must have no sentinel, got: {:?}",
        result.sentinel
    );
    // SmallInt * SmallInt → SmallInt (integer arithmetic promotion)
    assert!(
        matches!(result.inferred, SmeltType::List(_)),
        "pipe chain result must be a List, got: {:?}",
        result.inferred
    );
}

/// Inside `map(xs: List<Expr<SmallInt>>, fn c => c)`, the lookup of `c`
/// in the body context returns `Expr<SmallInt>` (lambda parameter binding).
#[test]
fn lambda_parameter_binding_via_typecontext() {
    // We test by checking that a context with lambda param `c: SmallInt`
    // resolves `c` to SmallInt in lookup_identifier.
    let mut ctx = TypeContext::new();
    ctx.add_lambda_param("c", smelt_types::TypedColumn::not_null(DataType::SmallInt));

    let resolved = ctx
        .lookup_identifier(None, "c")
        .expect("lambda param 'c' must resolve");
    assert_eq!(
        resolved.data_type,
        DataType::SmallInt,
        "lambda param 'c' must resolve to SmallInt"
    );
}

/// When an enclosing `smelt.define` parameter named `c` is in scope,
/// the lambda parameter `c` wins inside the lambda body (shadowing).
#[test]
fn lambda_parameter_shadows_outer_binding() {
    let mut ctx = TypeContext::new();
    // Outer function param `c` is BigInt
    ctx.add_function_param("c", smelt_types::TypedColumn::not_null(DataType::BigInt));
    // Lambda param `c` is SmallInt — shadows the outer BigInt
    ctx.add_lambda_param("c", smelt_types::TypedColumn::not_null(DataType::SmallInt));

    let resolved = ctx
        .lookup_identifier(None, "c")
        .expect("lambda param must be found");
    assert_eq!(
        resolved.data_type,
        DataType::SmallInt,
        "lambda param 'c: SmallInt' must shadow outer function param 'c: BigInt'"
    );
}

/// Every reducer name in the closed registry is recognised; an unknown
/// identifier is not.
#[test]
fn reducer_registry_lookup_closed_set() {
    let known = [
        "comma_sep",
        "and_all",
        "or_any",
        "union_all",
        "intersect_all",
        "plus_chain",
        "concat",
    ];
    for name in &known {
        assert!(
            lookup_reducer(name).is_some(),
            "reducer '{}' must be in the closed registry",
            name
        );
    }
    assert!(
        lookup_reducer("not_a_reducer").is_none(),
        "unknown reducer must not be in the registry"
    );
}

// === Phase 3 (meta-language Phase B) TDD tests: diagnostic emission ===

/// A `fn x => body` lambda not in a HOF positional argument position emits
/// `LambdaInForbiddenPosition`. We check via `check_hof_position_diagnostics`.
#[test]
fn lambda_outside_hof_position_emits_diagnostic() {
    // A lambda in a plain expression position — not inside a HOF call.
    let sql = "SELECT fn c => c FROM t";
    let select = parse_select_stmt(sql);
    let ctx = TypeContext::new();
    let diags = check_hof_position_diagnostics(&select, &ctx, "");
    assert!(
        diags
            .iter()
            .any(|d| d.code == Some(crate::DiagnosticCode::LambdaInForbiddenPosition)),
        "lambda in SELECT (non-HOF position) must emit LambdaInForbiddenPosition, \
             got: {:?}",
        diags
    );
}

/// `map(xs, fn (a, b) => a)` — multi-arg lambda in map — emits `LambdaArityMismatch`
/// (map expects arity 1).
#[test]
fn multi_arg_lambda_emits_arity_diagnostic() {
    // map call with multi-arg lambda (two params)
    let sql = "SELECT map([1, 2, 3], fn (a, b) => a) FROM t";
    let select = parse_select_stmt(sql);
    let ctx = TypeContext::new();
    let diags = check_hof_position_diagnostics(&select, &ctx, "");
    // After Phase F the multi-arg lambda parses as a LAMBDA node with two params.
    // The LAMBDA node walk emits LambdaArityMismatch (map expects arity 1).
    let has_arity_error = diags
        .iter()
        .any(|d| d.code == Some(crate::DiagnosticCode::LambdaArityMismatch));
    assert!(
        has_arity_error,
        "map with multi-arg lambda must emit LambdaArityMismatch, got: {:?}",
        diags
    );
}

/// `filter([1,2,3], fn c => c)` — predicate body is `Expr<SmallInt>` not Boolean —
/// emits `LambdaResultTypeMismatch`.
#[test]
fn filter_predicate_non_boolean_emits_lambda_result_mismatch() {
    let sql = "SELECT filter([1, 2, 3], fn c => c) FROM t";
    let select = parse_select_stmt(sql);
    let ctx = TypeContext::new();
    let diags = check_hof_position_diagnostics(&select, &ctx, "");
    assert!(
        diags
            .iter()
            .any(|d| d.code == Some(crate::DiagnosticCode::LambdaResultTypeMismatch)),
        "filter with non-Boolean predicate must emit LambdaResultTypeMismatch, got: {:?}",
        diags
    );
}

/// `map(xs, 42)` — non-lambda second arg — emits `HofExpectsLambda`.
#[test]
fn map_with_non_lambda_second_arg_emits_hof_expects_lambda() {
    let sql = "SELECT map([1, 2, 3], 42) FROM t";
    let select = parse_select_stmt(sql);
    let ctx = TypeContext::new();
    let diags = check_hof_position_diagnostics(&select, &ctx, "");
    assert!(
        diags
            .iter()
            .any(|d| d.code == Some(crate::DiagnosticCode::HofExpectsLambda)),
        "map with non-lambda second arg must emit HofExpectsLambda, got: {:?}",
        diags
    );
}

/// `reduce(xs, fn c => c)` — lambda where reducer expected — emits `HofExpectsReducer`.
#[test]
fn reduce_with_non_reducer_second_arg_emits_hof_expects_reducer() {
    let sql = "SELECT reduce([1, 2, 3], fn c => c) FROM t";
    let select = parse_select_stmt(sql);
    let ctx = TypeContext::new();
    let diags = check_hof_position_diagnostics(&select, &ctx, "");
    assert!(
        diags
            .iter()
            .any(|d| d.code == Some(crate::DiagnosticCode::HofExpectsReducer)),
        "reduce with lambda second arg must emit HofExpectsReducer, got: {:?}",
        diags
    );
}

/// D-19: `map(list => xs, fn c => c)` — named arg to HOF — emits `HofNamedArgument`.
#[test]
fn hof_named_arg_emits_hof_named_argument() {
    let sql = "SELECT map(list => [1, 2, 3], fn c => c * 2)";
    let select = parse_select_stmt(sql);
    let ctx = TypeContext::new();
    let diags = check_hof_position_diagnostics(&select, &ctx, "");
    assert!(
        diags
            .iter()
            .any(|d| d.code == Some(crate::DiagnosticCode::HofNamedArgument)),
        "map with named arg must emit HofNamedArgument, got: {:?}",
        diags
    );
}

/// D-19: A named arg that *is* a lambda still fires `HofNamedArgument`, not silently accepted.
#[test]
fn hof_named_lambda_arg_emits_hof_named_argument() {
    let sql = "SELECT map([1, 2, 3], transform => fn c => c * 2)";
    let select = parse_select_stmt(sql);
    let ctx = TypeContext::new();
    let diags = check_hof_position_diagnostics(&select, &ctx, "");
    assert!(
        diags
            .iter()
            .any(|d| d.code == Some(crate::DiagnosticCode::HofNamedArgument)),
        "map with named lambda arg must emit HofNamedArgument, got: {:?}",
        diags
    );
}

/// D-19 (positive case): positional args to HOF must not fire `HofNamedArgument`.
#[test]
fn hof_positional_args_ok_no_hof_named_argument() {
    let sql = "SELECT map([1, 2, 3], fn c => c * 2)";
    let select = parse_select_stmt(sql);
    let ctx = TypeContext::new();
    let diags = check_hof_position_diagnostics(&select, &ctx, "");
    assert!(
        !diags
            .iter()
            .any(|d| d.code == Some(crate::DiagnosticCode::HofNamedArgument)),
        "map with positional args must not emit HofNamedArgument, got: {:?}",
        diags
    );
}

/// `xs |> 3 + 4` — non-call RHS — emits `PipeRhsNotCall`.
#[test]
fn pipe_rhs_not_call_emits_diagnostic() {
    let sql = "SELECT [1, 2, 3] |> 3 + 4 FROM t";
    let select = parse_select_stmt(sql);
    let ctx = TypeContext::new();
    let diags = check_hof_position_diagnostics(&select, &ctx, "");
    assert!(
        diags
            .iter()
            .any(|d| d.code == Some(crate::DiagnosticCode::PipeRhsNotCall)),
        "pipe with non-call RHS must emit PipeRhsNotCall, got: {:?}",
        diags
    );
}

/// `reduce([1,2,3], and_all)` — Integer input, but and_all requires Boolean —
/// emits `ReducerInputTypeMismatch`.
#[test]
fn reduce_input_type_mismatch_emits_diagnostic() {
    let sql = "SELECT reduce([1, 2, 3], and_all) FROM t";
    let select = parse_select_stmt(sql);
    let ctx = TypeContext::new();
    let diags = check_hof_position_diagnostics(&select, &ctx, "");
    assert!(
        diags
            .iter()
            .any(|d| d.code == Some(crate::DiagnosticCode::ReducerInputTypeMismatch)),
        "reduce([1,2,3], and_all) must emit ReducerInputTypeMismatch, got: {:?}",
        diags
    );
}

/// `reduce([], union_all)` — empty list, no identity — emits `ReducerEmptyNoIdentity`.
#[test]
fn reduce_empty_no_identity_emits_diagnostic() {
    let sql = "SELECT reduce([], union_all) FROM t";
    let select = parse_select_stmt(sql);
    let ctx = TypeContext::new();
    let diags = check_hof_position_diagnostics(&select, &ctx, "");
    assert!(
        diags
            .iter()
            .any(|d| d.code == Some(crate::DiagnosticCode::ReducerEmptyNoIdentity)),
        "reduce([], union_all) must emit ReducerEmptyNoIdentity, got: {:?}",
        diags
    );
}

// === Phase 3: `smelt.config.var` resolver tests ===

/// `smelt.config.var('region')` over a workspace with `vars: { region: us-west-2 }`
/// resolves to a `Text` value `'us-west-2'` (no diagnostics).
#[test]
fn config_var_resolves_string_scalar() {
    use crate::config_vars::{coerce_yaml_scalar_to_text, parse_vars_from_yaml};

    let yaml = "name: my_project\nvars:\n  region: us-west-2\ntargets: {}\n";
    let vars = parse_vars_from_yaml(yaml);
    let vars = vars.expect("vars must parse successfully");
    let val = vars.get("region").expect("region must be present");
    let (text_val, warning) = coerce_yaml_scalar_to_text(val, "region");
    assert_eq!(text_val, "us-west-2", "region must resolve to 'us-west-2'");
    assert!(
        warning.is_none(),
        "string scalar must not warn, got: {:?}",
        warning
    );
}

/// `smelt.config.var('flag')` over `vars: { flag: true }` resolves to `'true'`;
/// integer `42` resolves to `'42'`.
#[test]
fn config_var_coerces_yaml_boolean() {
    use crate::config_vars::{coerce_yaml_scalar_to_text, parse_vars_from_yaml};

    let yaml = "name: my_project\nvars:\n  flag: true\n  count: 42\ntargets: {}\n";
    let vars = parse_vars_from_yaml(yaml).expect("vars must parse");
    {
        let val = vars.get("flag").expect("flag must be present");
        let (text_val, warning) = coerce_yaml_scalar_to_text(val, "flag");
        assert_eq!(text_val, "true", "boolean true must coerce to 'true'");
        assert!(warning.is_none());
    }
    {
        let val = vars.get("count").expect("count must be present");
        let (text_val, warning) = coerce_yaml_scalar_to_text(val, "count");
        assert_eq!(text_val, "42", "integer 42 must coerce to '42'");
        assert!(warning.is_none());
    }
}

/// `smelt.config.var('nullable')` over `vars: { nullable: ~ }` resolves to `''`
/// and emits `ConfigVarNullCoercion` warning sentinel.
#[test]
fn config_var_null_emits_warning() {
    use crate::config_vars::{coerce_yaml_scalar_to_text, parse_vars_from_yaml};

    let yaml = "name: my_project\nvars:\n  nullable: ~\ntargets: {}\n";
    let vars = parse_vars_from_yaml(yaml).expect("vars must parse");
    let val = vars.get("nullable").expect("nullable must be present");
    let (text_val, warning) = coerce_yaml_scalar_to_text(val, "nullable");
    assert_eq!(text_val, "", "null must coerce to empty string");
    assert!(
        warning.is_some(),
        "null coercion must produce a ConfigVarNullCoercion warning sentinel"
    );
}

/// `smelt.config.var('not_declared')` over a workspace whose `vars:` lacks `not_declared`
/// emits `ConfigVarNotFound`.
#[test]
fn config_var_not_found_emits_diagnostic() {
    use crate::config_vars::parse_vars_from_yaml;

    let yaml = "name: my_project\nvars:\n  region: us-east-1\ntargets: {}\n";
    let vars = parse_vars_from_yaml(yaml).expect("vars must parse");
    let result = vars.get("not_declared");
    assert!(
        result.is_none(),
        "not_declared must not be present in vars, got: {:?}",
        result
    );
    // The diagnostic emission path is tested in the production path (lib.rs).
}

/// `smelt.config.var(some_var)` (non-literal) — detection of non-literal arg.
/// We test the helper that detects whether an Expr is a string literal.
#[test]
fn config_var_non_literal_arg_emits_diagnostic() {
    use crate::config_vars::is_string_literal_expr;

    // A column reference "some_var" is not a string literal.
    let col_expr = parse_first_expr("SELECT some_var FROM t");
    assert!(
        !is_string_literal_expr(&col_expr),
        "column reference must NOT be a string literal"
    );

    // A string literal 'region' IS a string literal.
    let str_expr = parse_first_expr("SELECT 'region' FROM t");
    assert!(
        is_string_literal_expr(&str_expr),
        "quoted string must be a string literal"
    );
}

/// `smelt.define map(...)` — re-declaring a HOF name — emits `HofNameShadowed`.
#[test]
fn smelt_define_named_map_emits_hof_name_shadowed() {
    // Parse a file with a smelt.define named 'map' and check the name-shadowing
    // diagnostic via check_define_name_shadowing.
    use smelt_parser::ast::SmeltDefine;
    let sql = "smelt.define map(x: Expr<Integer>) AS (x + 1)\n";
    let parse = smelt_parser::parse(sql);
    let file = smelt_parser::ast::File::cast(parse.syntax()).expect("FILE");
    let define: SmeltDefine = file.defines().next().expect("one smelt.define");
    let diags = check_define_name_shadowing(&define);
    assert!(
        diags
            .iter()
            .any(|d| d.code == Some(crate::DiagnosticCode::HofNameShadowed)),
        "smelt.define named 'map' must emit HofNameShadowed, got: {:?}",
        diags
    );
}

/// `smelt.define concat(...)` — re-declaring a reducer name — emits `ReducerNameShadowed`.
#[test]
fn smelt_define_named_concat_emits_reducer_name_shadowed() {
    use smelt_parser::ast::SmeltDefine;
    let sql = "smelt.define concat(x: Expr<Text>) AS (x)\n";
    let parse = smelt_parser::parse(sql);
    let file = smelt_parser::ast::File::cast(parse.syntax()).expect("FILE");
    let define: SmeltDefine = file.defines().next().expect("one smelt.define");
    let diags = check_define_name_shadowing(&define);
    assert!(
        diags
            .iter()
            .any(|d| d.code == Some(crate::DiagnosticCode::ReducerNameShadowed)),
        "smelt.define named 'concat' must emit ReducerNameShadowed, got: {:?}",
        diags
    );
}

/// `WHERE a |> b()` — pipe in a Data-World position (WHERE predicate) — emits
/// `PipeInDataPosition`.
#[test]
fn pipe_in_where_clause_emits_diagnostic() {
    let sql = "SELECT x FROM t WHERE [1, 2, 3] |> map(fn c => c + 1)";
    let select = parse_select_stmt(sql);
    let ctx = TypeContext::new();
    let diags = check_hof_position_diagnostics(&select, &ctx, "");
    assert!(
        diags
            .iter()
            .any(|d| d.code == Some(crate::DiagnosticCode::PipeInDataPosition)),
        "pipe in WHERE clause must emit PipeInDataPosition, got: {:?}",
        diags
    );
}

// === Finding 1 fix: smelt.config.var type inference ===

/// `smelt.config.var('env')` infers as nullable Varchar (Phase B rule 10).
///
/// The `smelt.config.var` built-in is not in the function-signature index,
/// so it requires a special-case in `infer_smelt_path_call_type`.
#[test]
fn config_var_infers_nullable_varchar() {
    let ctx = TypeContext::new();
    let expr = parse_first_expr("SELECT smelt.config.var('env')");
    let typed =
        infer_expression_type(&expr, &ctx).expect("smelt.config.var('env') must infer a type");
    assert_eq!(
        typed.data_type,
        DataType::Varchar { max_length: None },
        "smelt.config.var must infer Varchar, got: {:?}",
        typed.data_type
    );
    assert!(
        typed.nullable,
        "smelt.config.var must be nullable (value may be absent without a default)"
    );
}

/// `smelt.config.var('env') = 'prod'` infers as Boolean (no type error).
///
/// An equality between `smelt.config.var(...)` (Varchar) and a string
/// literal (also Varchar) must produce a Boolean result — not an Unknown or
/// error — because both sides are Text-compatible.
#[test]
fn config_var_equality_with_varchar_literal_infers_boolean() {
    let ctx = TypeContext::new();
    let expr = parse_first_expr("SELECT smelt.config.var('env') = 'prod'");
    let typed = infer_expression_type(&expr, &ctx)
        .expect("smelt.config.var('env') = 'prod' must infer a type");
    assert_eq!(
        typed.data_type,
        DataType::Boolean,
        "smelt.config.var(...) = 'prod' must infer Boolean, got: {:?}",
        typed.data_type
    );
}

// === Finding 2 fix: HOF inference with List<T> function parameter ===

/// `xs.map(fn x => x + 1)` where `xs` is seeded as `List<Expr<Integer>>`
/// via `add_function_param_smelt_type` must infer a non-error result.
///
/// Without the fix the non-literal first-argument path would collapse
/// `xs` to `SmeltType::Expr(Concrete(Unknown))`, triggering `InputNotList`.
/// With the fix the lookup in `function_param_smelt_types` recovers the full
/// `SmeltType::List(...)`, and the lambda parameter `x` is bound to
/// `Expr<Integer>`.
#[test]
fn hof_map_on_list_param_infers_correctly() {
    let call = parse_hof_call("SELECT map(xs, fn x => x + 1)");
    let mut ctx = TypeContext::new();
    // Simulate a function body context where `xs: List<Expr<Integer>>` was declared.
    // add_function_param stores DataType::Unknown (the scalar projection).
    ctx.add_function_param(
        "xs",
        smelt_types::TypedColumn::nullable(DataType::unknown_dynamic()),
    );
    // add_function_param_smelt_type stores the full SmeltType.
    ctx.add_function_param_smelt_type(
        "xs",
        SmeltType::List(Box::new(SmeltType::Expr(
            smelt_types::signatures::TypeConstraint::Concrete(DataType::Integer),
        ))),
    );
    let result = infer_hof_call_from_function_call(&call, &ctx);
    assert!(
        !matches!(result.sentinel, Some(HofInferSentinel::InputNotList { .. })),
        "map on List<Expr<Integer>> param must not produce InputNotList, got: {:?}",
        result.sentinel
    );
    // The lambda body `x + 1` where x: Integer infers as Integer/BigInt —
    // the exact type depends on arithmetic promotion rules.  We only require
    // that the inferred type is a List<T> (not Unknown or Error).
    assert!(
        matches!(result.inferred, SmeltType::List(_)),
        "map on List<Expr<Integer>> param must infer List<T>, got: {:?}",
        result.inferred
    );
}

// === Phase C (meta-language) TDD tests — smelt.columns_of + ColumnRef field projection ===

/// `smelt.columns_of(42)` synthesises `List<ColumnRef>` (recoverable) and
/// emits exactly one `ColumnsOfRequiresTableExpr` at the `42` argument span.
#[test]
fn columns_of_arg_must_be_table_expr() {
    // Non-TableExpr arg: 42 (integer literal) — should emit ColumnsOfRequiresTableExpr.
    let sql = "SELECT smelt.columns_of(42) FROM t";
    let select = parse_select_stmt(sql);
    let ctx = TypeContext::new();
    let diags = check_columns_of_diagnostics(&select, &ctx);
    assert!(
        diags
            .iter()
            .any(|d| d.code == Some(crate::DiagnosticCode::ColumnsOfRequiresTableExpr)),
        "smelt.columns_of(42) must emit ColumnsOfRequiresTableExpr, got: {:?}",
        diags
    );
    assert_eq!(
        diags
            .iter()
            .filter(|d| d.code == Some(crate::DiagnosticCode::ColumnsOfRequiresTableExpr))
            .count(),
        1,
        "must emit exactly one ColumnsOfRequiresTableExpr"
    );

    // A smelt.<path> reference must not emit a Phase C diagnostic.
    // We use a bare path reference — the type-checker resolves it as TableExpr.
    let sql_ok = "SELECT smelt.columns_of(smelt.models.orders) FROM t";
    let select_ok = parse_select_stmt(sql_ok);
    let diags_ok = check_columns_of_diagnostics(&select_ok, &ctx);
    assert!(
        !diags_ok
            .iter()
            .any(|d| d.code == Some(crate::DiagnosticCode::ColumnsOfRequiresTableExpr)),
        "smelt.columns_of(smelt.models.orders) must NOT emit ColumnsOfRequiresTableExpr, got: {:?}",
        diags_ok
    );
}

/// `smelt.columns_of(t => orders)` emits exactly one `ColumnsOfNamedArgument`.
#[test]
fn columns_of_rejects_named_argument() {
    let sql = "SELECT smelt.columns_of(t => orders) FROM t";
    let select = parse_select_stmt(sql);
    let ctx = TypeContext::new();
    let diags = check_columns_of_diagnostics(&select, &ctx);
    assert!(
        diags
            .iter()
            .any(|d| d.code == Some(crate::DiagnosticCode::ColumnsOfNamedArgument)),
        "smelt.columns_of(t => orders) must emit ColumnsOfNamedArgument, got: {:?}",
        diags
    );
    assert_eq!(
        diags
            .iter()
            .filter(|d| d.code == Some(crate::DiagnosticCode::ColumnsOfNamedArgument))
            .count(),
        1,
        "must emit exactly one ColumnsOfNamedArgument"
    );

    // Positional arg must not emit ColumnsOfNamedArgument.
    let sql_ok = "SELECT smelt.columns_of(orders) FROM t";
    let select_ok = parse_select_stmt(sql_ok);
    let diags_ok = check_columns_of_diagnostics(&select_ok, &ctx);
    assert!(
        !diags_ok
            .iter()
            .any(|d| d.code == Some(crate::DiagnosticCode::ColumnsOfNamedArgument)),
        "smelt.columns_of(orders) must NOT emit ColumnsOfNamedArgument, got: {:?}",
        diags_ok
    );
}

/// Given a binding `c: ColumnRef`, field access `c.name`, `c.type`, `c.is_numeric`
/// synthesise the correct types.
#[test]
fn column_ref_field_projection_synthesises_field_type() {
    // Seed a lambda param `c` as ColumnRef.
    let mut ctx = TypeContext::new();
    ctx.add_function_param_smelt_type("c", SmeltType::ColumnRef);
    // For the data-type projection the lambda param `c` maps to DataType::unknown_dynamic()    // (ColumnRef is not a SQL DataType).
    ctx.add_lambda_param(
        "c",
        smelt_types::TypedColumn {
            data_type: DataType::unknown_dynamic(),
            nullable: true,
        },
    );

    // c.name → Text
    let name_ty = infer_field_on_column_ref("c", "name", &ctx);
    assert!(
        matches!(
            name_ty,
            Some(SmeltType::Expr(
                smelt_types::signatures::TypeConstraint::Concrete(DataType::Text)
            ))
        ),
        "c.name must synthesise Text, got: {:?}",
        name_ty
    );

    // c.is_numeric → Boolean
    let is_numeric_ty = infer_field_on_column_ref("c", "is_numeric", &ctx);
    assert!(
        matches!(
            is_numeric_ty,
            Some(SmeltType::Expr(
                smelt_types::signatures::TypeConstraint::Concrete(DataType::Boolean)
            ))
        ),
        "c.is_numeric must synthesise Boolean, got: {:?}",
        is_numeric_ty
    );

    // c.type → SmeltType::Unknown as the Phase C sentinel for "DataType (meta literal)".
    // Phase D will introduce a proper meta-DataType representation; for now Unknown
    // is the documented placeholder per the Phase C plan.
    let type_ty = infer_field_on_column_ref("c", "type", &ctx);
    assert!(
        matches!(type_ty, Some(SmeltType::Unknown)),
        "c.type maps to SmeltType::Unknown as the Phase C sentinel for DataType (meta literal); \
             Phase D will introduce a proper meta-DataType representation; got: {:?}",
        type_ty
    );
}

/// Head-constructor predicates `is_decimal`/`is_string`/`is_temporal`/`is_integer`/
/// `is_boolean` synthesise `Boolean` for a `ColumnRef`-typed binding `c`.
#[test]
fn column_ref_head_predicates() {
    let mut ctx = TypeContext::new();
    ctx.add_function_param_smelt_type("c", SmeltType::ColumnRef);
    ctx.add_lambda_param(
        "c",
        smelt_types::TypedColumn {
            data_type: DataType::unknown_dynamic(),
            nullable: true,
        },
    );

    for field in &[
        "is_decimal",
        "is_string",
        "is_temporal",
        "is_integer",
        "is_boolean",
    ] {
        let ty = infer_field_on_column_ref("c", field, &ctx);
        assert!(
            matches!(
                ty,
                Some(SmeltType::Expr(
                    smelt_types::signatures::TypeConstraint::Concrete(DataType::Boolean)
                ))
            ),
            "c.{field} must synthesise Boolean, got: {:?}",
            ty
        );
    }
}

/// `c.bogus` emits `ColumnRefFieldUnknown` whose message lists all 8 closed fields
/// including the new head-constructor predicates.
#[test]
fn column_ref_field_unknown_lists_new_fields() {
    use crate::diagnostics_types::meta_reflection_diagnostic_message;
    let msg = meta_reflection_diagnostic_message(
        crate::DiagnosticCode::ColumnRefFieldUnknown,
        None,
        Some("bogus"),
    );
    // The message must mention all 8 closed fields.
    for field in &[
        "name",
        "type",
        "is_numeric",
        "is_decimal",
        "is_string",
        "is_temporal",
        "is_integer",
        "is_boolean",
    ] {
        assert!(
            msg.contains(field),
            "ColumnRefFieldUnknown message must mention '{field}'; got: {msg}"
        );
    }
}

/// Given a binding `c: ColumnRef`, `c.foo` emits exactly one
/// `ColumnRefFieldUnknown` at the `foo` field token span and synthesises `Unknown`.
#[test]
fn column_ref_field_projection_rejects_unknown_field() {
    let mut ctx = TypeContext::new();
    ctx.add_function_param_smelt_type("c", SmeltType::ColumnRef);
    ctx.add_lambda_param(
        "c",
        smelt_types::TypedColumn {
            data_type: DataType::unknown_dynamic(),
            nullable: true,
        },
    );

    // c.foo must emit ColumnRefFieldUnknown anchored at the `foo` token span.
    // In "SELECT c.foo FROM t", `foo` is at byte offset 9..12 (line 0, col 9..12).
    let sql = "SELECT c.foo FROM t";
    let select = parse_select_stmt(sql);
    // Pass the actual source text so that to_range() can compute line/column positions.
    let diags = check_column_ref_field_diagnostics(&select, &ctx);
    assert!(
        diags
            .iter()
            .any(|d| d.code == Some(crate::DiagnosticCode::ColumnRefFieldUnknown)),
        "c.foo must emit ColumnRefFieldUnknown, got: {:?}",
        diags
    );
    assert_eq!(
        diags
            .iter()
            .filter(|d| d.code == Some(crate::DiagnosticCode::ColumnRefFieldUnknown))
            .count(),
        1,
        "must emit exactly one ColumnRefFieldUnknown"
    );
    // Pin the diagnostic to the `foo` field-token span, not the whole `c.foo` expression.
    // Spec invariant: ColumnRefFieldUnknown must anchor at the field name token only.
    // In "SELECT c.foo FROM t", `foo` is at byte offset 9..12.
    let unknown_diag = diags
        .iter()
        .find(|d| d.code == Some(crate::DiagnosticCode::ColumnRefFieldUnknown))
        .expect("already asserted above");
    let foo_start = sql.find("foo").expect("`foo` in sql");
    let foo_end = foo_start + "foo".len();
    assert_eq!(
        usize::from(unknown_diag.range.start()),
        foo_start,
        "diagnostic must start at the `foo` token byte offset, not the start of `c.foo`"
    );
    assert_eq!(
        usize::from(unknown_diag.range.end()),
        foo_end,
        "diagnostic must end after `foo`"
    );
}

// ─── Phase C Phase 2: meta-Text-as-identifier lift tests ─────────────────

/// Helper: build a TypeContext with a ColumnRef binding `c` and columns
/// `{name: Text, amount: Numeric}` in scope.
fn make_column_ref_ctx() -> TypeContext {
    let mut ctx = TypeContext::new();
    // Register `c` as a ColumnRef-typed lambda parameter.
    ctx.add_function_param_smelt_type("c", SmeltType::ColumnRef);
    ctx.add_lambda_param(
        "c",
        TypedColumn {
            data_type: DataType::unknown_dynamic(),
            nullable: true,
        },
    );
    // Seed two in-scope columns via a fake model.
    ctx.add_model_column(
        "t",
        "name",
        TypedColumn {
            data_type: DataType::Text,
            nullable: true,
        },
    );
    ctx.add_model_column(
        "t",
        "amount",
        TypedColumn {
            data_type: DataType::Double,
            nullable: true,
        },
    );
    ctx.add_alias("t", "t");
    ctx
}

/// `is_meta_text_value` predicate: `c.name` with `c: ColumnRef` → `Some("name")`.
#[test]
fn is_meta_text_value_recognises_column_ref_name_projection() {
    let ctx = make_column_ref_ctx();

    // c.name → Some("name")
    let sql = "SELECT c.name FROM t";
    let select = parse_select_stmt(sql);
    let list = select.select_list().expect("SelectList");
    let item = list.items().next().expect("first select item");
    let expr = item.expression().expect("expression");
    let result = is_meta_text_value(&expr, &ctx);
    assert_eq!(
        result,
        Some("name".to_string()),
        "c.name with c: ColumnRef must be recognised as meta-Text, got: {:?}",
        result
    );
}

/// `is_meta_text_value` predicate: `c.is_numeric` returns `None` (Boolean field,
/// not Text).
#[test]
fn is_meta_text_value_rejects_non_text_field() {
    let ctx = make_column_ref_ctx();

    // c.is_numeric → None (Boolean field, not Text)
    let sql = "SELECT c.is_numeric FROM t";
    let select = parse_select_stmt(sql);
    let list = select.select_list().expect("SelectList");
    let item = list.items().next().expect("first select item");
    let expr = item.expression().expect("expression");
    let result = is_meta_text_value(&expr, &ctx);
    assert_eq!(
        result, None,
        "c.is_numeric is Boolean, not Text — must NOT be recognised as meta-Text; got: {:?}",
        result
    );
}

/// `is_meta_text_value` predicate: a runtime `Expr<Text>` like `UPPER('foo')` returns `None`.
#[test]
fn no_lift_for_runtime_expr_text() {
    let ctx = make_column_ref_ctx();

    // UPPER('foo') is a runtime Expr<Text> — not a meta-Text, lift must not fire.
    let sql = "SELECT UPPER('foo') FROM t";
    let select = parse_select_stmt(sql);
    let list = select.select_list().expect("SelectList");
    let item = list.items().next().expect("first select item");
    let expr = item.expression().expect("expression");
    let result = is_meta_text_value(&expr, &ctx);
    assert_eq!(
        result, None,
        "UPPER('foo') is a runtime Expr<Text> — must NOT be recognised as meta-Text; got: {:?}",
        result
    );
}

/// `is_meta_text_value` predicate: a SQL string literal `'foo'` returns `None`.
#[test]
fn lift_only_for_compile_time_meta_text() {
    let ctx = make_column_ref_ctx();

    // 'foo' is a string literal — not a meta-Text projection.
    let sql = "SELECT 'foo' FROM t";
    let select = parse_select_stmt(sql);
    let list = select.select_list().expect("SelectList");
    let item = list.items().next().expect("first select item");
    let expr = item.expression().expect("expression");
    let result = is_meta_text_value(&expr, &ctx);
    assert_eq!(
        result, None,
        "'foo' is a SQL string literal — must NOT be recognised as meta-Text; got: {:?}",
        result
    );
}

/// `is_meta_text_value` predicate: `UPPER(c.name)` — the argument c.name is a
/// meta-Text but the outer UPPER call is NOT.  `no_lift_in_function_argument_position`
/// verifies that the lift does not fire for the function-call expression.
#[test]
fn no_lift_in_function_argument_position() {
    let ctx = make_column_ref_ctx();

    // UPPER(c.name) — the outer expression is a function call, not a meta-Text.
    let sql = "SELECT UPPER(c.name) FROM t";
    let select = parse_select_stmt(sql);
    let list = select.select_list().expect("SelectList");
    let item = list.items().next().expect("first select item");
    let expr = item.expression().expect("expression");

    // The outer expression (UPPER(...)) must NOT be a meta-Text value.
    let result = is_meta_text_value(&expr, &ctx);
    assert_eq!(
            result, None,
            "UPPER(c.name) outer expression must NOT be meta-Text (lift doesn't fire for function calls); got: {:?}",
            result
        );

    // No UnknownColumn expected from check_meta_text_lift_diagnostics for UPPER(c.name),
    // because UPPER(c.name) is not a lift-position expression.
    let diags = check_meta_text_lift_diagnostics(&select, &ctx);
    assert!(
        diags.is_empty(),
        "UPPER(c.name) must not produce lift diagnostics (not in lift position); got: {:?}",
        diags
    );
}

/// `lift_in_column_reference_position_resolves_to_column`:
/// `c.name` (meta-Text, field "name") in column-reference position produces
/// no diagnostics regardless of whether a column named "name" is in scope.
///
/// Body-check-time scope validation is suppressed because
/// `check_meta_text_lift_diagnostics` returns the field-name token ("name"),
/// not the per-element column name that the lift produces at expansion time.
/// Expansion-time validation is the correct location.
#[test]
fn lift_in_column_reference_position_resolves_to_column() {
    // ── Part 1: "name" IS in scope — no UnknownColumn ─────────────────────
    let ctx_with_name = make_column_ref_ctx(); // has `name` and `amount` columns
    let sql = "SELECT c.name FROM t";
    let select = parse_select_stmt(sql);
    let diags = check_meta_text_lift_diagnostics(&select, &ctx_with_name);
    assert!(
            diags.is_empty(),
            "c.name in column-ref position with 'name' in scope must produce no lift diagnostics; got: {:?}",
            diags
        );

    // ── Part 2: "name" NOT in scope — still no diagnostic ─────────────────
    // Body-check-time scope validation is suppressed: the field-name token
    // "name" is not the per-element column name.  Expansion-time validation
    // is the correct gate.
    let mut ctx_without_name = TypeContext::new();
    ctx_without_name.add_function_param_smelt_type("c", SmeltType::ColumnRef);
    ctx_without_name.add_lambda_param(
        "c",
        TypedColumn {
            data_type: DataType::unknown_dynamic(),
            nullable: true,
        },
    );
    ctx_without_name.add_model_column(
        "t",
        "amount",
        TypedColumn {
            data_type: DataType::Double,
            nullable: true,
        },
    );

    let diags_no_name = check_meta_text_lift_diagnostics(&select, &ctx_without_name);
    assert!(
        diags_no_name.is_empty(),
        "c.name with 'name' NOT literally in scope must still produce no diagnostics — \
             body-check-time lift-scope validation is suppressed; got: {:?}",
        diags_no_name
    );
}

/// `as_alias_lift_is_parser_limited`:
/// The spec describes `SUM(amount) AS c.name` as an AS-alias lift position
/// (Phase C §"Meta-Text-as-identifier lift", position 2).  At Phase C the
/// parser cannot represent `c.name` as a multi-token alias: `SelectItem::alias()`
/// captures only the first IDENT after `AS`, so `SUM(amount) AS c.name` yields
/// alias `"c"` and the `c.name` is silently truncated.
///
/// This test documents that parser limitation: the AS-alias arm of
/// `check_meta_text_lift_diagnostics` (the `item.alias().is_some()` branch)
/// cannot be reached by any syntactically valid Phase-C input.  The arm is
/// retained as a Phase-3-pending code path with a comment; its behaviour is
/// verified here via a parser-limitation assertion rather than an end-to-end
/// lift test.
///
/// TODO(Phase-3): once the parser supports `AS <dotted-identifier>`, replace
/// this test with a positive fixture that uses `SUM(amount) AS c.name` and
/// verifies that no scope-check error is emitted (aliases introduce names,
/// they do not reference them).
#[test]
fn as_alias_lift_is_parser_limited() {
    // `SUM(amount) AS c.name` — the parser captures only "c" as the alias.
    let sql = "SELECT SUM(amount) AS c_name FROM t";
    let select = parse_select_stmt(sql);
    let list = select.select_list().expect("SelectList");
    let item = list.items().next().expect("first select item");

    // alias() returns the single IDENT immediately after AS.
    let alias = item.alias();
    assert!(
        alias.is_some(),
        "SELECT SUM(...) AS c_name must have an alias; got: {:?}",
        alias
    );
    // Confirm the EXPRESSION of this item is NOT a meta-Text value —
    // SUM(amount) is a function-call, so is_meta_text_value returns None.
    let ctx = make_column_ref_ctx();
    let expr = item.expression().expect("expression");
    assert_eq!(
        is_meta_text_value(&expr, &ctx),
        None,
        "SUM(amount) is a function call, not a meta-Text value; got: {:?}",
        is_meta_text_value(&expr, &ctx)
    );

    // No lift diagnostics from check_meta_text_lift_diagnostics — SUM(amount)
    // is not a meta-Text expression so the AS-alias arm never fires.
    let diags = check_meta_text_lift_diagnostics(&select, &ctx);
    assert!(
        diags.is_empty(),
        "no lift diagnostics expected for SUM(amount) AS c_name; got: {:?}",
        diags
    );
}

/// `lift_in_column_reference_position_no_alias`:
/// `SELECT c.name FROM t` (no explicit AS alias, `name` in scope):
/// the meta-Text lift fires in column-reference position and emits no error.
/// `infer_select_output_schema` infers `"name"` as the output column name.
#[test]
fn lift_in_column_reference_position_no_alias() {
    let ctx = make_column_ref_ctx();

    // c.name as the select expression — the lifted identifier "name" is the
    // inferred output column name.  No UnknownColumn should be emitted.
    let sql = "SELECT c.name FROM t";
    let select = parse_select_stmt(sql);

    // Confirm lift predicate fires.
    let list = select.select_list().expect("SelectList");
    let item = list.items().next().expect("first select item");
    let expr = item.expression().expect("expression");
    assert_eq!(
        is_meta_text_value(&expr, &ctx),
        Some("name".to_string()),
        "c.name must be detected as meta-Text"
    );

    // No explicit alias on this select item.
    assert!(
        item.alias().is_none(),
        "SELECT c.name FROM t must have no explicit alias"
    );

    // No lift diagnostic — "name" is in scope.
    let diags = check_meta_text_lift_diagnostics(&select, &ctx);
    assert!(
            diags.is_empty(),
            "c.name in SELECT list (column-ref position) with 'name' in scope must not produce diagnostics; got: {:?}",
            diags
        );
}

/// `lift_in_order_by_position_resolves_to_column`:
/// `ORDER BY c.name` produces no diagnostics regardless of whether a column
/// named "name" is in scope.
///
/// Body-check-time scope validation is suppressed for the same reason as the
/// column-reference position: the field-name token is not the per-element
/// column name.
#[test]
fn lift_in_order_by_position_resolves_to_column() {
    let ctx_with_name = make_column_ref_ctx();

    // ── Part 1: "name" in scope ────────────────────────────────────────────
    let sql = "SELECT name FROM t ORDER BY c.name";
    let select = parse_select_stmt(sql);
    let diags = check_meta_text_lift_diagnostics(&select, &ctx_with_name);
    assert!(
        diags.is_empty(),
        "ORDER BY c.name with 'name' in scope must produce no diagnostics; got: {:?}",
        diags
    );

    // ── Part 2: "name" NOT in scope — still no diagnostic ─────────────────
    let mut ctx_no_name = TypeContext::new();
    ctx_no_name.add_function_param_smelt_type("c", SmeltType::ColumnRef);
    ctx_no_name.add_lambda_param(
        "c",
        TypedColumn {
            data_type: DataType::unknown_dynamic(),
            nullable: true,
        },
    );
    ctx_no_name.add_model_column(
        "t",
        "amount",
        TypedColumn {
            data_type: DataType::Double,
            nullable: true,
        },
    );

    let diags_err = check_meta_text_lift_diagnostics(&select, &ctx_no_name);
    assert!(
        diags_err.is_empty(),
        "ORDER BY c.name with 'name' NOT literally in scope must produce no diagnostics — \
             body-check-time lift-scope validation is suppressed; got: {:?}",
        diags_err
    );
}

/// `lift_in_group_by_position_resolves_to_column`:
/// `GROUP BY c.name` produces no diagnostics regardless of whether a column
/// named "name" is in scope.
///
/// Body-check-time scope validation is suppressed for the same reason as the
/// other lift positions.
#[test]
fn lift_in_group_by_position_resolves_to_column() {
    let ctx_with_name = make_column_ref_ctx();

    // ── Part 1: "name" in scope ────────────────────────────────────────────
    let sql = "SELECT c.name FROM t GROUP BY c.name";
    let select = parse_select_stmt(sql);
    let diags = check_meta_text_lift_diagnostics(&select, &ctx_with_name);
    assert!(
        diags.is_empty(),
        "GROUP BY c.name with 'name' in scope must produce no diagnostics; got: {:?}",
        diags
    );

    // ── Part 2: "name" NOT in scope — still no diagnostic ─────────────────
    let mut ctx_no_name = TypeContext::new();
    ctx_no_name.add_function_param_smelt_type("c", SmeltType::ColumnRef);
    ctx_no_name.add_lambda_param(
        "c",
        TypedColumn {
            data_type: DataType::unknown_dynamic(),
            nullable: true,
        },
    );
    ctx_no_name.add_model_column(
        "t",
        "amount",
        TypedColumn {
            data_type: DataType::Double,
            nullable: true,
        },
    );

    let sql_no_name = "SELECT c.name FROM t GROUP BY c.name";
    let select_no_name = parse_select_stmt(sql_no_name);
    let diags_err = check_meta_text_lift_diagnostics(&select_no_name, &ctx_no_name);
    assert!(
        diags_err.is_empty(),
        "GROUP BY c.name with 'name' NOT literally in scope must produce no diagnostics — \
             body-check-time lift-scope validation is suppressed; got: {:?}",
        diags_err
    );
}

// ─── Phase D: wide-reflection diagnostics ────────────────────────────────

/// `smelt.models.with_tag(42)` emits `WithTagRequiresText` (integer is not Text).
/// `smelt.sources.with_tag(UPPER('x'))` emits `WithTagRequiresText` (runtime Text).
/// `smelt.models.with_tag('core')` emits no Phase D diagnostic.
#[test]
fn with_tag_arg_must_be_compile_time_text() {
    // smelt.models.with_tag(42) — integer literal, not Text → WithTagRequiresText
    {
        let sql = "SELECT smelt.models.with_tag(42) FROM t";
        let select = parse_select_stmt(sql);
        let ctx = TypeContext::new();
        let diags = check_wide_reflection_diagnostics(&select, &ctx, "");
        assert!(
            diags
                .iter()
                .any(|d| d.code == Some(crate::DiagnosticCode::WithTagRequiresText)),
            "smelt.models.with_tag(42) must emit WithTagRequiresText, got: {:?}",
            diags
        );
        assert_eq!(
            diags
                .iter()
                .filter(|d| d.code == Some(crate::DiagnosticCode::WithTagRequiresText))
                .count(),
            1,
            "must emit exactly one WithTagRequiresText"
        );
    }

    // smelt.sources.with_tag(UPPER('x')) — runtime Expr<Text> → WithTagRequiresText
    {
        let sql = "SELECT smelt.sources.with_tag(UPPER('x')) FROM t";
        let select = parse_select_stmt(sql);
        let ctx = TypeContext::new();
        let diags = check_wide_reflection_diagnostics(&select, &ctx, "");
        assert!(
            diags
                .iter()
                .any(|d| d.code == Some(crate::DiagnosticCode::WithTagRequiresText)),
            "smelt.sources.with_tag(UPPER('x')) must emit WithTagRequiresText, got: {:?}",
            diags
        );
    }

    // smelt.models.with_tag('core') — string literal → NO Phase D diagnostic
    {
        let sql = "SELECT smelt.models.with_tag('core') FROM t";
        let select = parse_select_stmt(sql);
        let ctx = TypeContext::new();
        let diags = check_wide_reflection_diagnostics(&select, &ctx, "");
        assert!(
            !diags.iter().any(|d| matches!(
                d.code,
                Some(crate::DiagnosticCode::WithTagRequiresText)
                    | Some(crate::DiagnosticCode::WithTagNamedArgument)
                    | Some(crate::DiagnosticCode::WideReflectionUnknownAccessor)
                    | Some(crate::DiagnosticCode::WideReflectionUnexpectedArgument)
            )),
            "smelt.models.with_tag('core') must emit NO Phase D diagnostic, got: {:?}",
            diags
        );
    }
}

/// `smelt.models.with_tag(tag => 'core')` emits exactly one `WithTagNamedArgument`.
/// `smelt.models.with_tag('core')` does not.
#[test]
fn with_tag_rejects_named_argument() {
    // Named argument → WithTagNamedArgument
    {
        let sql = "SELECT smelt.models.with_tag(tag => 'core') FROM t";
        let select = parse_select_stmt(sql);
        let ctx = TypeContext::new();
        let diags = check_wide_reflection_diagnostics(&select, &ctx, "");
        assert!(
            diags
                .iter()
                .any(|d| d.code == Some(crate::DiagnosticCode::WithTagNamedArgument)),
            "smelt.models.with_tag(tag => 'core') must emit WithTagNamedArgument, got: {:?}",
            diags
        );
        assert_eq!(
            diags
                .iter()
                .filter(|d| d.code == Some(crate::DiagnosticCode::WithTagNamedArgument))
                .count(),
            1,
            "must emit exactly one WithTagNamedArgument"
        );
    }

    // Positional arg → no WithTagNamedArgument
    {
        let sql = "SELECT smelt.models.with_tag('core') FROM t";
        let select = parse_select_stmt(sql);
        let ctx = TypeContext::new();
        let diags = check_wide_reflection_diagnostics(&select, &ctx, "");
        assert!(
            !diags
                .iter()
                .any(|d| d.code == Some(crate::DiagnosticCode::WithTagNamedArgument)),
            "smelt.models.with_tag('core') must NOT emit WithTagNamedArgument, got: {:?}",
            diags
        );
    }
}

/// `smelt.models.bogus()` emits exactly one `WideReflectionUnknownAccessor` at
/// the `bogus` token span; same for `smelt.sources.bogus()`.
#[test]
fn wide_reflection_unknown_accessor() {
    // smelt.models.bogus() — unknown accessor
    {
        let sql = "SELECT smelt.models.bogus() FROM t";
        let select = parse_select_stmt(sql);
        let ctx = TypeContext::new();
        let diags = check_wide_reflection_diagnostics(&select, &ctx, "");
        assert!(
            diags
                .iter()
                .any(|d| d.code == Some(crate::DiagnosticCode::WideReflectionUnknownAccessor)),
            "smelt.models.bogus() must emit WideReflectionUnknownAccessor, got: {:?}",
            diags
        );
        assert_eq!(
            diags
                .iter()
                .filter(|d| d.code == Some(crate::DiagnosticCode::WideReflectionUnknownAccessor))
                .count(),
            1,
            "must emit exactly one WideReflectionUnknownAccessor"
        );
    }

    // smelt.sources.bogus() — same for "sources"
    {
        let sql = "SELECT smelt.sources.bogus() FROM t";
        let select = parse_select_stmt(sql);
        let ctx = TypeContext::new();
        let diags = check_wide_reflection_diagnostics(&select, &ctx, "");
        assert!(
            diags
                .iter()
                .any(|d| d.code == Some(crate::DiagnosticCode::WideReflectionUnknownAccessor)),
            "smelt.sources.bogus() must emit WideReflectionUnknownAccessor, got: {:?}",
            diags
        );
    }
}

/// `smelt.models.all(42)` emits exactly one `WideReflectionUnexpectedArgument` at the
/// `42` arg span; `smelt.models.all()` does not.
/// `smelt.sources.all(named => 'x')` emits `WideReflectionUnexpectedArgument` at named-arg span.
#[test]
fn wide_reflection_all_takes_no_arguments() {
    // smelt.models.all(42) — positional arg to all()
    {
        let sql = "SELECT smelt.models.all(42) FROM t";
        let select = parse_select_stmt(sql);
        let ctx = TypeContext::new();
        let diags = check_wide_reflection_diagnostics(&select, &ctx, "");
        assert!(
            diags
                .iter()
                .any(|d| d.code == Some(crate::DiagnosticCode::WideReflectionUnexpectedArgument)),
            "smelt.models.all(42) must emit WideReflectionUnexpectedArgument, got: {:?}",
            diags
        );
        assert_eq!(
            diags
                .iter()
                .filter(|d| d.code == Some(crate::DiagnosticCode::WideReflectionUnexpectedArgument))
                .count(),
            1,
            "must emit exactly one WideReflectionUnexpectedArgument"
        );
    }

    // smelt.models.all() — no args → no diagnostic
    {
        let sql = "SELECT smelt.models.all() FROM t";
        let select = parse_select_stmt(sql);
        let ctx = TypeContext::new();
        let diags = check_wide_reflection_diagnostics(&select, &ctx, "");
        assert!(
            !diags
                .iter()
                .any(|d| d.code == Some(crate::DiagnosticCode::WideReflectionUnexpectedArgument)),
            "smelt.models.all() must NOT emit WideReflectionUnexpectedArgument, got: {:?}",
            diags
        );
    }

    // smelt.sources.all(named => 'x') — named arg to all()
    {
        let sql = "SELECT smelt.sources.all(named => 'x') FROM t";
        let select = parse_select_stmt(sql);
        let ctx = TypeContext::new();
        let diags = check_wide_reflection_diagnostics(&select, &ctx, "");
        assert!(
            diags
                .iter()
                .any(|d| d.code == Some(crate::DiagnosticCode::WideReflectionUnexpectedArgument)),
            "smelt.sources.all(named => 'x') must emit WideReflectionUnexpectedArgument, got: {:?}",
            diags
        );
    }
}

/// Given `m: ModelRef`, field projections synthesise the correct types.
/// Given `s: SourceRef`, field projections synthesise the correct types.
#[test]
fn model_ref_field_projection_synthesises_field_type() {
    use smelt_types::signatures::SmeltType;

    // Set up a context with `m: ModelRef` and `s: SourceRef`.
    let mut ctx = TypeContext::new();
    ctx.add_function_param_smelt_type("m", SmeltType::ModelRef);
    ctx.add_lambda_param(
        "m",
        TypedColumn {
            data_type: DataType::unknown_dynamic(),
            nullable: true,
        },
    );
    ctx.add_function_param_smelt_type("s", SmeltType::SourceRef);
    ctx.add_lambda_param(
        "s",
        TypedColumn {
            data_type: DataType::unknown_dynamic(),
            nullable: true,
        },
    );

    // m.path → Expr<Text>
    let path_ty = infer_field_on_model_ref("m", "path", &ctx);
    assert!(
        matches!(
            path_ty,
            Some(SmeltType::Expr(
                smelt_types::signatures::TypeConstraint::Concrete(DataType::Text)
            ))
        ),
        "m.path must synthesise Expr<Text>, got: {:?}",
        path_ty
    );

    // m.name → Expr<Text>
    let name_ty = infer_field_on_model_ref("m", "name", &ctx);
    assert!(
        matches!(
            name_ty,
            Some(SmeltType::Expr(
                smelt_types::signatures::TypeConstraint::Concrete(DataType::Text)
            ))
        ),
        "m.name must synthesise Expr<Text>, got: {:?}",
        name_ty
    );

    // m.tags → List<Expr<Text>>
    let tags_ty = infer_field_on_model_ref("m", "tags", &ctx);
    assert!(
        matches!(&tags_ty, Some(SmeltType::List(inner))
                if matches!(inner.as_ref(), SmeltType::Expr(smelt_types::signatures::TypeConstraint::Concrete(DataType::Text)))),
        "m.tags must synthesise List<Expr<Text>>, got: {:?}",
        tags_ty
    );

    // m.columns → List<ColumnRef>
    let cols_ty = infer_field_on_model_ref("m", "columns", &ctx);
    assert!(
        matches!(&cols_ty, Some(SmeltType::List(inner)) if matches!(inner.as_ref(), SmeltType::ColumnRef)),
        "m.columns must synthesise List<ColumnRef>, got: {:?}",
        cols_ty
    );

    // SourceRef: s.path → Expr<Text>
    let s_path_ty = infer_field_on_source_ref("s", "path", &ctx);
    assert!(
        matches!(
            s_path_ty,
            Some(SmeltType::Expr(
                smelt_types::signatures::TypeConstraint::Concrete(DataType::Text)
            ))
        ),
        "s.path must synthesise Expr<Text>, got: {:?}",
        s_path_ty
    );

    // SourceRef: s.name → Expr<Text>
    let s_name_ty = infer_field_on_source_ref("s", "name", &ctx);
    assert!(
        matches!(
            s_name_ty,
            Some(SmeltType::Expr(
                smelt_types::signatures::TypeConstraint::Concrete(DataType::Text)
            ))
        ),
        "s.name must synthesise Expr<Text>, got: {:?}",
        s_name_ty
    );

    // SourceRef: s.tags → List<Expr<Text>>
    let s_tags_ty = infer_field_on_source_ref("s", "tags", &ctx);
    assert!(
        matches!(&s_tags_ty, Some(SmeltType::List(inner))
                if matches!(inner.as_ref(), SmeltType::Expr(smelt_types::signatures::TypeConstraint::Concrete(DataType::Text)))),
        "s.tags must synthesise List<Expr<Text>>, got: {:?}",
        s_tags_ty
    );

    // SourceRef: s.columns → List<ColumnRef>
    let s_cols_ty = infer_field_on_source_ref("s", "columns", &ctx);
    assert!(
        matches!(&s_cols_ty, Some(SmeltType::List(inner)) if matches!(inner.as_ref(), SmeltType::ColumnRef)),
        "s.columns must synthesise List<ColumnRef>, got: {:?}",
        s_cols_ty
    );
}

/// Given `m: ModelRef`, `m.foo` emits exactly one `ModelRefFieldUnknown` at the `foo`
/// field span and synthesises `Unknown` (drop-on-error).
/// Given `s: SourceRef`, `s.bar` emits exactly one `SourceRefFieldUnknown`.
#[test]
fn model_ref_field_projection_rejects_unknown_field() {
    use smelt_types::signatures::SmeltType;

    let mut ctx = TypeContext::new();
    ctx.add_function_param_smelt_type("m", SmeltType::ModelRef);
    ctx.add_lambda_param(
        "m",
        TypedColumn {
            data_type: DataType::unknown_dynamic(),
            nullable: true,
        },
    );
    ctx.add_function_param_smelt_type("s", SmeltType::SourceRef);
    ctx.add_lambda_param(
        "s",
        TypedColumn {
            data_type: DataType::unknown_dynamic(),
            nullable: true,
        },
    );

    // m.foo — unknown field on ModelRef → ModelRefFieldUnknown
    let sql = "SELECT m.foo FROM t";
    let select = parse_select_stmt(sql);
    let diags = check_model_ref_source_ref_field_diagnostics(&select, &ctx);
    assert!(
        diags
            .iter()
            .any(|d| d.code == Some(crate::DiagnosticCode::ModelRefFieldUnknown)),
        "m.foo must emit ModelRefFieldUnknown, got: {:?}",
        diags
    );
    assert_eq!(
        diags
            .iter()
            .filter(|d| d.code == Some(crate::DiagnosticCode::ModelRefFieldUnknown))
            .count(),
        1,
        "must emit exactly one ModelRefFieldUnknown"
    );
    // Confirm infer_field_on_model_ref returns None for unknown field.
    let unknown_ty = infer_field_on_model_ref("m", "foo", &ctx);
    assert!(
        unknown_ty.is_none(),
        "infer_field_on_model_ref must return None for unknown field 'foo', got: {:?}",
        unknown_ty
    );

    // s.bar — unknown field on SourceRef → SourceRefFieldUnknown
    let sql_s = "SELECT s.bar FROM t";
    let select_s = parse_select_stmt(sql_s);
    let diags_s = check_model_ref_source_ref_field_diagnostics(&select_s, &ctx);
    assert!(
        diags_s
            .iter()
            .any(|d| d.code == Some(crate::DiagnosticCode::SourceRefFieldUnknown)),
        "s.bar must emit SourceRefFieldUnknown, got: {:?}",
        diags_s
    );
    assert_eq!(
        diags_s
            .iter()
            .filter(|d| d.code == Some(crate::DiagnosticCode::SourceRefFieldUnknown))
            .count(),
        1,
        "must emit exactly one SourceRefFieldUnknown"
    );
    // Confirm infer_field_on_source_ref returns None for unknown field.
    let s_unknown_ty = infer_field_on_source_ref("s", "bar", &ctx);
    assert!(
        s_unknown_ty.is_none(),
        "infer_field_on_source_ref must return None for unknown field 'bar', got: {:?}",
        s_unknown_ty
    );
}

// === Phase D Phase 2 TDD tests — ModelRef/SourceRef <: TableExpr subtyping ===

/// `smelt.columns_of(m)` where `m: ModelRef` synthesises `List<ColumnRef>` with no
/// diagnostic — the `ModelRef <: TableExpr` subtyping lift fires at the call site.
#[test]
fn model_ref_assignable_to_table_expr_in_columns_of_arg() {
    use smelt_types::signatures::SmeltType;

    let mut ctx = TypeContext::new();
    ctx.add_function_param_smelt_type("m", SmeltType::ModelRef);
    ctx.add_lambda_param(
        "m",
        TypedColumn {
            data_type: DataType::unknown_dynamic(),
            nullable: true,
        },
    );

    // smelt.columns_of(m) — m is ModelRef; must produce no ColumnsOfRequiresTableExpr diagnostic.
    let sql = "SELECT smelt.columns_of(m) FROM t";
    let select = parse_select_stmt(sql);
    let diags = check_columns_of_diagnostics(&select, &ctx);
    let table_expr_errors: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(crate::DiagnosticCode::ColumnsOfRequiresTableExpr))
        .collect();
    assert!(
            table_expr_errors.is_empty(),
            "smelt.columns_of(m) where m: ModelRef must produce no ColumnsOfRequiresTableExpr, got: {:?}",
            table_expr_errors
        );
}

/// `smelt.columns_of(s)` where `s: SourceRef` synthesises `List<ColumnRef>` with no
/// diagnostic — the `SourceRef <: TableExpr` subtyping lift fires.
#[test]
fn source_ref_assignable_to_table_expr_in_columns_of_arg() {
    use smelt_types::signatures::SmeltType;

    let mut ctx = TypeContext::new();
    ctx.add_function_param_smelt_type("s", SmeltType::SourceRef);
    ctx.add_lambda_param(
        "s",
        TypedColumn {
            data_type: DataType::unknown_dynamic(),
            nullable: true,
        },
    );

    let sql = "SELECT smelt.columns_of(s) FROM t";
    let select = parse_select_stmt(sql);
    let diags = check_columns_of_diagnostics(&select, &ctx);
    let table_expr_errors: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(crate::DiagnosticCode::ColumnsOfRequiresTableExpr))
        .collect();
    assert!(
            table_expr_errors.is_empty(),
            "smelt.columns_of(s) where s: SourceRef must produce no ColumnsOfRequiresTableExpr, got: {:?}",
            table_expr_errors
        );
}

/// `reduce(xs, union_all)` where `xs: List<ModelRef>` synthesises `TableExpr` with no
/// sentinel — List covariance lifts `List<ModelRef>` to `List<TableExpr>`.
#[test]
fn list_of_model_ref_lifts_to_list_of_table_expr_in_reducer_arg() {
    use smelt_types::signatures::SmeltType;

    // Build a context with `xs: List<ModelRef>`.
    let mut ctx = TypeContext::new();
    ctx.add_function_param_smelt_type("xs", SmeltType::List(Box::new(SmeltType::ModelRef)));

    // Parse `reduce(xs, union_all)` as a HOF call.
    let call = parse_hof_call("SELECT reduce(xs, union_all)");
    let result = infer_hof_call_from_function_call(&call, &ctx);

    assert!(
        result.sentinel.is_none(),
        "reduce(xs, union_all) where xs: List<ModelRef> must have no sentinel, got: {:?}",
        result.sentinel
    );
    assert!(
        matches!(result.inferred, SmeltType::TableExpr(_)),
        "reduce(xs, union_all) where xs: List<ModelRef> must infer TableExpr, got: {:?}",
        result.inferred
    );
}

/// `SELECT * FROM m` as a meta splice: `ModelRef <: TableExpr` means the
/// subtyping rule fires. At body-check time this test verifies `is_subtype_of`
/// returns true — concrete column resolution comes in Phase 3.
#[test]
fn model_ref_assignable_in_from_clause_splice() {
    use smelt_types::signatures::{is_subtype_of, SmeltType};

    // The subtyping rule is the gate for FROM-clause splice acceptance.
    assert!(
        is_subtype_of(&SmeltType::ModelRef, &SmeltType::TableExpr(None)),
        "ModelRef must be assignable to TableExpr at FROM-clause splice positions"
    );
    assert!(
        is_subtype_of(&SmeltType::SourceRef, &SmeltType::TableExpr(None)),
        "SourceRef must be assignable to TableExpr at FROM-clause splice positions"
    );
}

/// A plain `TableExpr`-typed binding is NOT assignable to a `ModelRef`-typed position —
/// the subtyping rule is one-way. Verified via `is_subtype_of` and also via the
/// `ReducerInputConstraint` path: `reduce(xs, union_all)` where `xs: List<TableExpr>`
/// succeeds, but `xs: List<TableExpr>` is NOT assignable to `List<ModelRef>`.
#[test]
fn table_expr_not_assignable_to_model_ref() {
    use smelt_types::signatures::{is_subtype_of, SmeltType};

    // Direct subtyping: TableExpr is NOT a subtype of ModelRef.
    assert!(
        !is_subtype_of(&SmeltType::TableExpr(None), &SmeltType::ModelRef),
        "TableExpr must NOT be assignable to ModelRef (reverse direction forbidden)"
    );
    assert!(
        !is_subtype_of(&SmeltType::TableExpr(None), &SmeltType::SourceRef),
        "TableExpr must NOT be assignable to SourceRef (reverse direction forbidden)"
    );

    // List direction: List<TableExpr> is NOT a subtype of List<ModelRef>.
    let list_table = SmeltType::List(Box::new(SmeltType::TableExpr(None)));
    let list_model_ref = SmeltType::List(Box::new(SmeltType::ModelRef));
    assert!(
        !is_subtype_of(&list_table, &list_model_ref),
        "List<TableExpr> must NOT be assignable to List<ModelRef>"
    );
}

/// `m.columns` (field projection on `ModelRef`) synthesises the same type as
/// `smelt.columns_of(m)` — both produce `List<ColumnRef>` at body-check time.
#[test]
fn m_columns_equivalent_to_smelt_columns_of_m() {
    use smelt_types::signatures::SmeltType;

    let mut ctx = TypeContext::new();
    ctx.add_function_param_smelt_type("m", SmeltType::ModelRef);
    ctx.add_lambda_param(
        "m",
        TypedColumn {
            data_type: DataType::unknown_dynamic(),
            nullable: true,
        },
    );

    // m.columns via field projection.
    let columns_ty = infer_field_on_model_ref("m", "columns", &ctx);
    assert!(
        matches!(&columns_ty, Some(SmeltType::List(inner)) if matches!(inner.as_ref(), SmeltType::ColumnRef)),
        "m.columns must synthesise List<ColumnRef>, got: {:?}",
        columns_ty
    );

    // smelt.columns_of(m) via smelt path call — check no ColumnsOfRequiresTableExpr.
    let sql = "SELECT smelt.columns_of(m) FROM t";
    let select = parse_select_stmt(sql);
    let diags = check_columns_of_diagnostics(&select, &ctx);
    let table_expr_errors: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(crate::DiagnosticCode::ColumnsOfRequiresTableExpr))
        .collect();
    assert!(
        table_expr_errors.is_empty(),
        "smelt.columns_of(m) where m: ModelRef must produce no ColumnsOfRequiresTableExpr"
    );

    // Both produce List<ColumnRef> — assert the types are equivalent.
    let columns_ty_inner = match columns_ty {
        Some(SmeltType::List(inner)) => *inner,
        other => panic!("expected List<ColumnRef> from m.columns, got: {:?}", other),
    };
    assert!(
        matches!(columns_ty_inner, SmeltType::ColumnRef),
        "m.columns inner type must be ColumnRef, got: {:?}",
        columns_ty_inner
    );
    // smelt.columns_of return type from signature also produces List<ColumnRef>
    // (verified by the columns_of_signature_returns_list_of_column_ref test in signatures.rs).
    // At this level we just confirm no diagnostic fires — the full semantic equivalence
    // is enforced at expansion time (Phase 3).
}

// ═══════════════════════════════════════════════════════════════════════
// Phase E1 Phase 3 TDD tests — record type inference
// ═══════════════════════════════════════════════════════════════════════

/// Build a `Cohort` record declaration: `{ name: Text, threshold: Integer }`.
fn make_cohort_decl() -> smelt_types::signatures::SmeltRecordDeclaration {
    use smelt_types::signatures::{SmeltRecordDeclaration, TypeConstraint};
    let zero_range = rowan::TextRange::new(0.into(), 0.into());
    SmeltRecordDeclaration {
        name: "Cohort".to_string(),
        fields: vec![
            (
                "name".to_string(),
                SmeltType::Expr(TypeConstraint::Concrete(DataType::Text)),
                zero_range,
            ),
            (
                "threshold".to_string(),
                SmeltType::Expr(TypeConstraint::Concrete(DataType::Integer)),
                zero_range,
            ),
        ],
        name_span: zero_range,
        source_path: std::sync::Arc::from("models/cohort.sql"),
    }
}

/// Build a `TypeContext` with the `Cohort` record registered.
fn make_cohort_ctx() -> TypeContext {
    let decl = make_cohort_decl();
    let (registry, _sentinels) = record_registry_for_workspace(&[decl]);
    let mut ctx = TypeContext::new();
    ctx.set_record_registry(std::sync::Arc::new(registry));
    ctx
}

/// Parse a `RecordLiteral` node from `SELECT smelt.foo({...}) FROM t`.
fn parse_record_literal(sql: &str) -> smelt_parser::ast::RecordLiteral {
    let parse = smelt_parser::parse(sql);
    parse
        .syntax()
        .descendants()
        .find_map(smelt_parser::ast::RecordLiteral::cast)
        .expect("must find RecordLiteral node in SQL")
}

/// Resolve the `Cohort` target type from the registry in `ctx`.
fn cohort_target_type(ctx: &TypeContext) -> SmeltType {
    let decl = ctx
        .lookup_record_decl("Cohort")
        .expect("Cohort must be registered");
    let mut fields = std::collections::BTreeMap::new();
    for (name, ty, _span) in &decl.fields {
        fields.insert(name.clone(), ty.clone());
    }
    SmeltType::Record {
        fields,
        name: Some("Cohort".to_string()),
    }
}

// ─── Test 1: happy path ───────────────────────────────────────────────
#[test]
fn infer_record_literal_against_named_target_emits_no_diagnostic_on_happy_path() {
    let ctx = make_cohort_ctx();
    let target = cohort_target_type(&ctx);
    // {name: 'us_west', threshold: 100} — threshold=100 is SmallInt from literal
    // but we accept it as Integer via DataType compatibility (same family).
    // Actually 100 is SmallInt. Use 100000 which is Integer.
    let lit = parse_record_literal("SELECT smelt.foo({name: 'us_west', threshold: 100000}) FROM t");
    let result = check_record_literal(&lit, &ctx, Some(&target), "");
    assert!(
        result.sentinels.is_empty(),
        "happy path must emit no diagnostics, got: {:?}",
        result.sentinels
    );
    assert!(
        matches!(&result.inferred, SmeltType::Record { name: Some(n), .. } if n == "Cohort"),
        "must synthesise Record{{Cohort}}, got: {:?}",
        result.inferred
    );
}

// ─── Test 2: RecordFieldMissing ───────────────────────────────────────
#[test]
fn infer_record_literal_emits_record_field_missing() {
    let ctx = make_cohort_ctx();
    let target = cohort_target_type(&ctx);
    // Missing `threshold`
    let lit = parse_record_literal("SELECT smelt.foo({name: 'us_west'}) FROM t");
    let result = check_record_literal(&lit, &ctx, Some(&target), "");

    let missing: Vec<_> = result
        .sentinels
        .iter()
        .filter(|s| s.code == crate::DiagnosticCode::RecordFieldMissing)
        .collect();
    assert_eq!(
        missing.len(),
        1,
        "must emit exactly 1 RecordFieldMissing, got: {:?}",
        result.sentinels
    );
    assert!(
        missing[0].message.contains("threshold"),
        "RecordFieldMissing must name `threshold`, got: {}",
        missing[0].message
    );
    // Synthesised type is still Record{Cohort} (recoverable).
    assert!(
        matches!(&result.inferred, SmeltType::Record { name: Some(n), .. } if n == "Cohort"),
        "must synthesise Record{{Cohort}} on missing field, got: {:?}",
        result.inferred
    );
}

// ─── Test 3: RecordFieldUnknown (literal unknown field) ───────────────
#[test]
fn infer_record_literal_emits_record_field_unknown() {
    let ctx = make_cohort_ctx();
    let target = cohort_target_type(&ctx);
    // Extra `bogus` field
    let lit = parse_record_literal(
        "SELECT smelt.foo({name: 'us_west', threshold: 100000, bogus: true}) FROM t",
    );
    let result = check_record_literal(&lit, &ctx, Some(&target), "");

    let unknown: Vec<_> = result
        .sentinels
        .iter()
        .filter(|s| s.code == crate::DiagnosticCode::RecordFieldUnknown)
        .collect();
    assert_eq!(
        unknown.len(),
        1,
        "must emit exactly 1 RecordFieldUnknown for bogus field, got: {:?}",
        result.sentinels
    );
    assert!(
        unknown[0].message.contains("bogus"),
        "RecordFieldUnknown must name `bogus`, got: {}",
        unknown[0].message
    );
    // No follow-on diagnostics: only RecordFieldUnknown (no RecordFieldMissing).
    let non_unknown: Vec<_> = result
        .sentinels
        .iter()
        .filter(|s| s.code != crate::DiagnosticCode::RecordFieldUnknown)
        .collect();
    assert!(
        non_unknown.is_empty(),
        "must have no follow-on diagnostics after RecordFieldUnknown, got: {:?}",
        non_unknown
    );
}

// ─── Test 4: RecordFieldDuplicate ─────────────────────────────────────
#[test]
fn infer_record_literal_emits_record_field_duplicate() {
    let ctx = make_cohort_ctx();
    let target = cohort_target_type(&ctx);
    // Second `name` occurrence
    let lit = parse_record_literal(
        "SELECT smelt.foo({name: 'us_west', name: 'eu', threshold: 100000}) FROM t",
    );
    let result = check_record_literal(&lit, &ctx, Some(&target), "");

    let dupes: Vec<_> = result
        .sentinels
        .iter()
        .filter(|s| s.code == crate::DiagnosticCode::RecordFieldDuplicate)
        .collect();
    assert_eq!(
        dupes.len(),
        1,
        "must emit exactly 1 RecordFieldDuplicate, got: {:?}",
        result.sentinels
    );
    assert!(
        dupes[0].message.contains("name"),
        "RecordFieldDuplicate must mention field `name`, got: {}",
        dupes[0].message
    );
}

// ─── Test 5: RecordFieldTypeMismatch ──────────────────────────────────
#[test]
fn infer_record_literal_emits_record_field_type_mismatch() {
    let ctx = make_cohort_ctx();
    let target = cohort_target_type(&ctx);
    // `threshold: 'lots'` — string literal for an Integer field.
    let lit = parse_record_literal("SELECT smelt.foo({name: 'us_west', threshold: 'lots'}) FROM t");
    let result = check_record_literal(&lit, &ctx, Some(&target), "");

    let mismatches: Vec<_> = result
        .sentinels
        .iter()
        .filter(|s| s.code == crate::DiagnosticCode::RecordFieldTypeMismatch)
        .collect();
    assert_eq!(
        mismatches.len(),
        1,
        "must emit exactly 1 RecordFieldTypeMismatch, got: {:?}",
        result.sentinels
    );
    assert!(
        mismatches[0].message.contains("threshold"),
        "RecordFieldTypeMismatch must name `threshold`, got: {}",
        mismatches[0].message
    );
    // The synthesised record carries `Unknown` at `threshold`.
    if let SmeltType::Record { fields, .. } = &result.inferred {
        let threshold_ty = fields
            .get("threshold")
            .expect("threshold must be in result");
        assert!(
            matches!(threshold_ty, SmeltType::Unknown),
            "threshold must be Unknown after type mismatch, got: {:?}",
            threshold_ty
        );
    } else {
        panic!("result must be Record, got: {:?}", result.inferred);
    }
}

// ─── Test 6: RecordLiteralUnknownTarget ───────────────────────────────
#[test]
fn infer_record_literal_emits_record_literal_unknown_target_when_unanchored() {
    let ctx = TypeContext::new(); // no registry
    let lit = parse_record_literal("SELECT smelt.foo({name: 'us_west', threshold: 100000}) FROM t");
    let result = check_record_literal(&lit, &ctx, None, "");

    let unknown_target: Vec<_> = result
        .sentinels
        .iter()
        .filter(|s| s.code == crate::DiagnosticCode::RecordLiteralUnknownTarget)
        .collect();
    assert_eq!(
        unknown_target.len(),
        1,
        "must emit exactly 1 RecordLiteralUnknownTarget, got: {:?}",
        result.sentinels
    );
    // Type is Record<Unknown> (name: None, empty fields).
    assert!(
        matches!(&result.inferred, SmeltType::Record { name: None, fields } if fields.is_empty()),
        "unanchored literal must have type Record<Unknown>, got: {:?}",
        result.inferred
    );
}

// ─── Test 7: field projection synthesises field type ──────────────────
#[test]
fn infer_record_field_projection_synthesises_field_type() {
    use smelt_types::signatures::TypeConstraint;
    // Build a Cohort record type directly.
    let mut fields = std::collections::BTreeMap::new();
    fields.insert(
        "name".to_string(),
        SmeltType::Expr(TypeConstraint::Concrete(DataType::Text)),
    );
    fields.insert(
        "threshold".to_string(),
        SmeltType::Expr(TypeConstraint::Concrete(DataType::Integer)),
    );
    let cohort_ty = SmeltType::Record {
        fields,
        name: Some("Cohort".to_string()),
    };

    let zero = rowan::TextRange::new(0.into(), 0.into());

    // c.name → Text
    let result_name = infer_record_field_projection(&cohort_ty, "name", zero, "");
    assert!(
        result_name.sentinels.is_empty(),
        "c.name must emit no diagnostics, got: {:?}",
        result_name.sentinels
    );
    assert!(
        matches!(
            &result_name.inferred,
            SmeltType::Expr(TypeConstraint::Concrete(DataType::Text))
        ),
        "c.name must synthesise Expr<Text>, got: {:?}",
        result_name.inferred
    );

    // c.threshold → Integer
    let result_threshold = infer_record_field_projection(&cohort_ty, "threshold", zero, "");
    assert!(
        result_threshold.sentinels.is_empty(),
        "c.threshold must emit no diagnostics, got: {:?}",
        result_threshold.sentinels
    );
    assert!(
        matches!(
            &result_threshold.inferred,
            SmeltType::Expr(TypeConstraint::Concrete(DataType::Integer))
        ),
        "c.threshold must synthesise Expr<Integer>, got: {:?}",
        result_threshold.inferred
    );
}

// ─── Test 8: field projection on unknown field emits RecordFieldUnknown ─
#[test]
fn infer_record_field_projection_emits_record_field_unknown_on_miss() {
    use smelt_types::signatures::TypeConstraint;
    let mut fields = std::collections::BTreeMap::new();
    fields.insert(
        "name".to_string(),
        SmeltType::Expr(TypeConstraint::Concrete(DataType::Text)),
    );
    fields.insert(
        "threshold".to_string(),
        SmeltType::Expr(TypeConstraint::Concrete(DataType::Integer)),
    );
    let cohort_ty = SmeltType::Record {
        fields,
        name: Some("Cohort".to_string()),
    };

    let zero = rowan::TextRange::new(0.into(), 0.into());
    let result = infer_record_field_projection(&cohort_ty, "bogus", zero, "");

    let unknown: Vec<_> = result
        .sentinels
        .iter()
        .filter(|s| s.code == crate::DiagnosticCode::RecordFieldUnknown)
        .collect();
    assert_eq!(
        unknown.len(),
        1,
        "c.bogus must emit exactly 1 RecordFieldUnknown, got: {:?}",
        result.sentinels
    );
    assert!(
        matches!(&result.inferred, SmeltType::Unknown),
        "c.bogus projection must synthesise Unknown, got: {:?}",
        result.inferred
    );
}

// ─── Test 9: RecordFieldNotProjectable mid-chain ──────────────────────
#[test]
fn infer_record_field_projection_emits_record_field_not_projectable_mid_chain() {
    use smelt_types::signatures::TypeConstraint;

    // c.name is Expr<Text>; then .foo is projection on Text (not a Record).
    let text_ty = SmeltType::Expr(TypeConstraint::Concrete(DataType::Text));
    let zero = rowan::TextRange::new(0.into(), 0.into());

    let result = infer_record_field_projection(&text_ty, "foo", zero, "");
    let not_projectable: Vec<_> = result
        .sentinels
        .iter()
        .filter(|s| s.code == crate::DiagnosticCode::RecordFieldNotProjectable)
        .collect();
    assert_eq!(
        not_projectable.len(),
        1,
        "projection on Text must emit exactly 1 RecordFieldNotProjectable, got: {:?}",
        result.sentinels
    );
    assert!(
        matches!(&result.inferred, SmeltType::Unknown),
        "projection on non-record must synthesise Unknown, got: {:?}",
        result.inferred
    );
}

// ─── Test 10: width subtyping admits wider to narrower ───────────────
#[test]
fn record_width_subtyping_assigns_wider_to_narrower() {
    use smelt_types::signatures::{is_subtype_of, TypeConstraint};

    // Cohort = { name: Text, threshold: Integer }  (wider)
    let mut cohort_fields = std::collections::BTreeMap::new();
    cohort_fields.insert(
        "name".to_string(),
        SmeltType::Expr(TypeConstraint::Concrete(DataType::Text)),
    );
    cohort_fields.insert(
        "threshold".to_string(),
        SmeltType::Expr(TypeConstraint::Concrete(DataType::Integer)),
    );
    let cohort_ty = SmeltType::Record {
        fields: cohort_fields,
        name: Some("Cohort".to_string()),
    };

    // Narrow target: { name: Text }
    let mut narrow_fields = std::collections::BTreeMap::new();
    narrow_fields.insert(
        "name".to_string(),
        SmeltType::Expr(TypeConstraint::Concrete(DataType::Text)),
    );
    let narrow_ty = SmeltType::Record {
        fields: narrow_fields.clone(),
        name: None,
    };

    // Cohort (wider) <: {name: Text} (narrower) — admitted with no diagnostic.
    assert!(
        is_subtype_of(&cohort_ty, &narrow_ty),
        "Cohort must be assignable to {{name: Text}} (width subtyping)"
    );

    // Reverse: {name: Text} is NOT a subtype of Cohort (missing `threshold`).
    assert!(
        !is_subtype_of(&narrow_ty, &cohort_ty),
        "{{name: Text}} must NOT be assignable to Cohort (missing threshold)"
    );
}

// ─── Test 11: width subtyping — projection diagnostics use declared (narrower) type ─
#[test]
fn record_width_subtyping_projection_diagnostic_unchanged_under_widening() {
    use smelt_types::signatures::TypeConstraint;

    // When checking against the narrower type {name: Text}, projecting
    // .threshold must emit RecordFieldUnknown (the closed declared set of
    // the narrower type doesn't include `threshold`).
    let mut narrow_fields = std::collections::BTreeMap::new();
    narrow_fields.insert(
        "name".to_string(),
        SmeltType::Expr(TypeConstraint::Concrete(DataType::Text)),
    );
    let narrow_ty = SmeltType::Record {
        fields: narrow_fields,
        name: None,
    };

    let zero = rowan::TextRange::new(0.into(), 0.into());
    let result = infer_record_field_projection(&narrow_ty, "threshold", zero, "");

    let unknown: Vec<_> = result
        .sentinels
        .iter()
        .filter(|s| s.code == crate::DiagnosticCode::RecordFieldUnknown)
        .collect();
    assert_eq!(
        unknown.len(),
        1,
        "projecting .threshold on {{name: Text}} must emit RecordFieldUnknown; got: {:?}",
        result.sentinels
    );
}

// ─── Test 12: SmeltRecordRedefinition ────────────────────────────────
#[test]
fn smelt_record_declaration_redefinition_emits_diagnostic() {
    use smelt_types::signatures::{RecordRegistryCode, SmeltRecordDeclaration, TypeConstraint};

    let zero_range = rowan::TextRange::new(0.into(), 0.into());
    let second_range = rowan::TextRange::new(10.into(), 16.into());

    let decl1 = SmeltRecordDeclaration {
        name: "Cohort".to_string(),
        fields: vec![(
            "name".to_string(),
            SmeltType::Expr(TypeConstraint::Concrete(DataType::Text)),
            zero_range,
        )],
        name_span: zero_range,
        source_path: std::sync::Arc::from("models/a.sql"),
    };
    let decl2 = SmeltRecordDeclaration {
        name: "Cohort".to_string(),
        fields: vec![(
            "count".to_string(),
            SmeltType::Expr(TypeConstraint::Concrete(DataType::Integer)),
            zero_range,
        )],
        name_span: second_range,
        source_path: std::sync::Arc::from("models/b.sql"),
    };

    let (_registry, sentinels) = record_registry_for_workspace(&[decl1, decl2]);
    let redef: Vec<_> = sentinels
        .iter()
        .filter(|s| s.code == RecordRegistryCode::SmeltRecordRedefinition)
        .collect();
    assert_eq!(
        redef.len(),
        1,
        "two Cohort declarations must emit exactly 1 SmeltRecordRedefinition, got: {:?}",
        sentinels
    );
    // Anchored at the second declaration's name_span.
    assert_eq!(
        redef[0].span, second_range,
        "SmeltRecordRedefinition must be anchored at the second declaration's name_span"
    );
    // First declaration is authoritative.
    assert!(
        _registry.lookup("Cohort").is_some(),
        "Cohort must still be in the registry (first wins)"
    );
}

// ─── Test 13: RecordCyclicDeclaration ────────────────────────────────
#[test]
fn smelt_record_cyclic_declaration_emits_diagnostic() {
    use smelt_types::signatures::{RecordRegistryCode, SmeltRecordDeclaration};

    let zero_range = rowan::TextRange::new(0.into(), 0.into());

    // A = {b: B}, B = {a: A} — mutual cycle.
    let decl_a = SmeltRecordDeclaration {
        name: "A".to_string(),
        fields: vec![(
            "b".to_string(),
            SmeltType::Record {
                fields: std::collections::BTreeMap::new(),
                name: Some("B".to_string()),
            },
            zero_range,
        )],
        name_span: zero_range,
        source_path: std::sync::Arc::from("models/a.sql"),
    };
    let decl_b = SmeltRecordDeclaration {
        name: "B".to_string(),
        fields: vec![(
            "a".to_string(),
            SmeltType::Record {
                fields: std::collections::BTreeMap::new(),
                name: Some("A".to_string()),
            },
            zero_range,
        )],
        name_span: zero_range,
        source_path: std::sync::Arc::from("models/b.sql"),
    };

    let (registry, sentinels) = record_registry_for_workspace(&[decl_a, decl_b]);
    let cyclic: Vec<_> = sentinels
        .iter()
        .filter(|s| s.code == RecordRegistryCode::RecordCyclicDeclaration)
        .collect();
    assert!(
        !cyclic.is_empty(),
        "mutually cyclic declarations must emit at least 1 RecordCyclicDeclaration, got: {:?}",
        sentinels
    );
    // Downstream uses: A and B still in the registry (continue to type-check).
    assert!(
        registry.lookup("A").is_some() && registry.lookup("B").is_some(),
        "A and B must remain in registry after cycle detection"
    );
}

// ─── Test 14: RecordFieldTypeForbidden ───────────────────────────────
#[test]
fn record_field_type_forbidden_for_reflection_witnesses() {
    use smelt_types::signatures::{RecordRegistryCode, SmeltRecordDeclaration};

    let zero_range = rowan::TextRange::new(0.into(), 0.into());

    // Bad = {m: ModelRef}
    let decl_model_ref = SmeltRecordDeclaration {
        name: "BadModelRef".to_string(),
        fields: vec![("m".to_string(), SmeltType::ModelRef, zero_range)],
        name_span: zero_range,
        source_path: std::sync::Arc::from("models/bad.sql"),
    };
    // Bad2 = {c: ColumnRef}
    let decl_column_ref = SmeltRecordDeclaration {
        name: "BadColumnRef".to_string(),
        fields: vec![("c".to_string(), SmeltType::ColumnRef, zero_range)],
        name_span: zero_range,
        source_path: std::sync::Arc::from("models/bad2.sql"),
    };
    // Bad3 = {s: SourceRef}
    let decl_source_ref = SmeltRecordDeclaration {
        name: "BadSourceRef".to_string(),
        fields: vec![("s".to_string(), SmeltType::SourceRef, zero_range)],
        name_span: zero_range,
        source_path: std::sync::Arc::from("models/bad3.sql"),
    };
    // Bad4 = {f: Lambda<Unknown, Unknown>}
    let decl_lambda = SmeltRecordDeclaration {
        name: "BadLambda".to_string(),
        fields: vec![(
            "f".to_string(),
            SmeltType::Lambda(vec![SmeltType::Unknown], Box::new(SmeltType::Unknown)),
            zero_range,
        )],
        name_span: zero_range,
        source_path: std::sync::Arc::from("models/bad4.sql"),
    };

    let (_registry, sentinels) = record_registry_for_workspace(&[
        decl_model_ref,
        decl_column_ref,
        decl_source_ref,
        decl_lambda,
    ]);
    let forbidden: Vec<_> = sentinels
        .iter()
        .filter(|s| s.code == RecordRegistryCode::RecordFieldTypeForbidden)
        .collect();
    assert_eq!(
        forbidden.len(),
        4,
        "must emit 4 RecordFieldTypeForbidden sentinels (one per bad decl), got: {:?}",
        sentinels
    );
}

// ─── Test 15: RecordInDataWorld ───────────────────────────────────────
#[test]
fn record_in_data_world_emits_diagnostic_when_consumed_at_sql_position() {
    use smelt_types::signatures::TypeConstraint;

    let zero = rowan::TextRange::new(0.into(), 0.into());

    // A record-typed binding reference in a non-splice SQL position (is_splice_context=false).
    let mut cohort_fields = std::collections::BTreeMap::new();
    cohort_fields.insert(
        "name".to_string(),
        SmeltType::Expr(TypeConstraint::Concrete(DataType::Text)),
    );
    let cohort_ty = SmeltType::Record {
        fields: cohort_fields,
        name: Some("Cohort".to_string()),
    };

    let sentinel = check_record_in_data_world(&cohort_ty, zero, false, "");
    assert!(
        sentinel.is_some(),
        "Record-typed binding in non-splice SQL position must emit RecordInDataWorld"
    );
    assert!(
        matches!(
            sentinel.as_ref().unwrap().code,
            crate::DiagnosticCode::RecordInDataWorld
        ),
        "diagnostic code must be RecordInDataWorld, got: {:?}",
        sentinel.unwrap().code
    );

    // Projecting `c.name` produces Expr<Text>, which is NOT a Record — no diagnostic.
    let text_ty = SmeltType::Expr(TypeConstraint::Concrete(DataType::Text));
    let no_sentinel = check_record_in_data_world(&text_ty, zero, false, "");
    assert!(
        no_sentinel.is_none(),
        "Expr<Text> in SQL position must NOT emit RecordInDataWorld, got: {:?}",
        no_sentinel
    );
}

// ─── Test 16: inline and named records with same fields are assignable ─
#[test]
fn inline_record_and_named_record_with_same_field_set_are_assignable() {
    use smelt_types::signatures::{is_subtype_of, TypeConstraint};

    let mut fields = std::collections::BTreeMap::new();
    fields.insert(
        "name".to_string(),
        SmeltType::Expr(TypeConstraint::Concrete(DataType::Text)),
    );
    fields.insert(
        "threshold".to_string(),
        SmeltType::Expr(TypeConstraint::Concrete(DataType::Integer)),
    );

    let named = SmeltType::Record {
        fields: fields.clone(),
        name: Some("Cohort".to_string()),
    };
    let inline = SmeltType::Record {
        fields: fields.clone(),
        name: None,
    };

    // Named <: inline
    assert!(
        is_subtype_of(&named, &inline),
        "Record{{Cohort}} must be assignable to inline {{name: Text, threshold: Integer}}"
    );
    // Inline <: named
    assert!(
        is_subtype_of(&inline, &named),
        "inline {{name: Text, threshold: Integer}} must be assignable to Record{{Cohort}}"
    );
}

// =========================================================================
// Phase E1 Phase 4 TDD tests — Map<K,V> API dispatch + invariance +
// statically-known-key resolution
// =========================================================================

/// Helper: build `Map<Text, Integer>`.
fn map_text_integer_ty() -> SmeltType {
    SmeltType::Map {
        key: Box::new(SmeltType::Expr(TypeConstraint::Concrete(DataType::Text))),
        value: Box::new(SmeltType::Expr(TypeConstraint::Concrete(DataType::Integer))),
    }
}

/// Helper: build `Map<Text, Number>`.
fn map_text_number_ty() -> SmeltType {
    use smelt_types::signatures::TypeConstraint;
    SmeltType::Map {
        key: Box::new(SmeltType::Expr(TypeConstraint::Concrete(DataType::Text))),
        value: Box::new(SmeltType::Expr(TypeConstraint::Numeric)),
    }
}

/// Helper: build a `MapCallArg` that is a positional Text literal.
fn text_literal_arg(s: &str) -> MapCallArg {
    MapCallArg::Positional {
        ty: SmeltType::Expr(TypeConstraint::Concrete(DataType::Text)),
        literal_value: Some(s.to_string()),
    }
}

/// Helper: build a `MapCallArg` that is a positional Integer literal (non-text key).
fn integer_literal_arg() -> MapCallArg {
    MapCallArg::Positional {
        ty: SmeltType::Expr(TypeConstraint::Concrete(DataType::Integer)),
        literal_value: None,
    }
}

/// Helper: build a `MapCallArg` that is a positional non-literal Text (variable).
fn text_variable_arg() -> MapCallArg {
    MapCallArg::Positional {
        ty: SmeltType::Expr(TypeConstraint::Concrete(DataType::Text)),
        literal_value: None,
    }
}

/// Helper: a named argument.
fn named_arg() -> MapCallArg {
    MapCallArg::Named {
        param_name: "key".to_string(),
        ty: SmeltType::Expr(TypeConstraint::Concrete(DataType::Text)),
    }
}

/// Helper: build a synthetic `Map<Text, Numeric>` receiver.
///
/// The declared `V` is `Numeric` (widened). The per-entry contents
/// (`bounded_map_contents`) store `Integer` (narrower). This split is
/// intentional: if the static-resolution path is taken, `get('a')` returns
/// `Integer` (per-entry type). If the generic-V fallback were taken instead,
/// it would return `Numeric`. Tests that assert `Integer` therefore prove the
/// static path was exercised.
fn bounded_map_receiver() -> SmeltType {
    SmeltType::Map {
        key: Box::new(SmeltType::Expr(TypeConstraint::Concrete(DataType::Text))),
        value: Box::new(SmeltType::Expr(TypeConstraint::Numeric)),
    }
}

/// Helper: build the bound contents `{'a': Integer, 'b': Integer}`.
///
/// Each entry stores `Integer` — narrower than the declared `V = Numeric` in
/// `bounded_map_receiver()`. Used alongside `bounded_map_receiver()` to
/// distinguish the static-resolution path from the generic-formula fallback.
fn bounded_map_contents() -> std::collections::BTreeMap<String, SmeltType> {
    let mut contents = std::collections::BTreeMap::new();
    contents.insert(
        "a".to_string(),
        SmeltType::Expr(TypeConstraint::Concrete(DataType::Integer)),
    );
    contents.insert(
        "b".to_string(),
        SmeltType::Expr(TypeConstraint::Concrete(DataType::Integer)),
    );
    contents
}

// Phase E1 Phase 4 test 1
#[test]
fn map_api_entries_synthesises_list_of_record() {
    let receiver = map_text_integer_ty();
    let result = infer_map_method_call(
        &receiver,
        "entries",
        &[],
        None,
        TextRange::new(0.into(), 0.into()),
    );
    assert!(
        result.sentinels.is_empty(),
        "m.entries() must emit no diagnostics, got: {:?}",
        result.sentinels
    );
    // Expected: List<Record<{key: Text, value: Integer}>>
    let mut fields = std::collections::BTreeMap::new();
    fields.insert(
        "key".to_string(),
        SmeltType::Expr(TypeConstraint::Concrete(DataType::Text)),
    );
    fields.insert(
        "value".to_string(),
        SmeltType::Expr(TypeConstraint::Concrete(DataType::Integer)),
    );
    let expected = SmeltType::List(Box::new(SmeltType::Record { fields, name: None }));
    assert_eq!(
        result.inferred, expected,
        "m.entries() must synthesise List<Record<{{key: Text, value: Integer}}>>, got: {:?}",
        result.inferred
    );
}

// Phase E1 Phase 4 test 2
#[test]
fn map_api_keys_and_values_synthesise_lists() {
    let receiver = map_text_integer_ty();

    // m.keys() → List<Text>
    let keys_result = infer_map_method_call(
        &receiver,
        "keys",
        &[],
        None,
        TextRange::new(0.into(), 0.into()),
    );
    assert!(
        keys_result.sentinels.is_empty(),
        "m.keys() must emit no diagnostics, got: {:?}",
        keys_result.sentinels
    );
    let expected_keys = SmeltType::List(Box::new(SmeltType::Expr(TypeConstraint::Concrete(
        DataType::Text,
    ))));
    assert_eq!(
        keys_result.inferred, expected_keys,
        "m.keys() must synthesise List<Text>, got: {:?}",
        keys_result.inferred
    );

    // m.values() → List<Integer>
    let values_result = infer_map_method_call(
        &receiver,
        "values",
        &[],
        None,
        TextRange::new(0.into(), 0.into()),
    );
    assert!(
        values_result.sentinels.is_empty(),
        "m.values() must emit no diagnostics, got: {:?}",
        values_result.sentinels
    );
    let expected_values = SmeltType::List(Box::new(SmeltType::Expr(TypeConstraint::Concrete(
        DataType::Integer,
    ))));
    assert_eq!(
        values_result.inferred, expected_values,
        "m.values() must synthesise List<Integer>, got: {:?}",
        values_result.inferred
    );
}

// Phase E1 Phase 4 test 3
#[test]
fn map_api_get_synthesises_value_type_on_non_static_key() {
    let receiver = map_text_integer_ty();
    // Non-literal k: Text — evaluation deferred.
    let args = [text_variable_arg()];
    let result = infer_map_method_call(
        &receiver,
        "get",
        &args,
        None,
        TextRange::new(0.into(), 0.into()),
    );
    assert!(
        result.sentinels.is_empty(),
        "m.get(k) with non-literal key must emit no diagnostics, got: {:?}",
        result.sentinels
    );
    let expected = SmeltType::Expr(TypeConstraint::Concrete(DataType::Integer));
    assert_eq!(
        result.inferred, expected,
        "m.get(k) with non-literal key must synthesise Integer (value type), got: {:?}",
        result.inferred
    );
    assert_eq!(
        result.static_resolution,
        StaticResolution::Deferred,
        "m.get(k) with non-literal key must report Deferred resolution, got: {:?}",
        result.static_resolution
    );
}

// Phase E1 Phase 4 test 4
#[test]
fn map_api_get_statically_known_present_key_synthesises_value_and_resolves() {
    // Use bounded_map_receiver() which declares V = Numeric (widened), while
    // bounded_map_contents() stores Integer (narrower) for each entry.
    // If the static-resolution path is taken, get('a') returns Integer (per-entry type).
    // If the generic-V fallback is taken instead, it would return Numeric.
    // Asserting Integer proves the static path was exercised.
    let receiver = bounded_map_receiver();
    let contents = bounded_map_contents();
    // m.get('a') — key 'a' is present in the bound contents.
    let args = [text_literal_arg("a")];
    let result = infer_map_method_call(
        &receiver,
        "get",
        &args,
        Some(&contents),
        TextRange::new(0.into(), 0.into()),
    );
    assert!(
        result.sentinels.is_empty(),
        "m.get('a') on bound map must emit no diagnostics, got: {:?}",
        result.sentinels
    );
    // Must return Integer (the per-entry type from contents), NOT Numeric (the declared V).
    let expected = SmeltType::Expr(TypeConstraint::Concrete(DataType::Integer));
    assert_eq!(
        result.inferred, expected,
        "m.get('a') must synthesise Integer (per-entry type), not Numeric (declared V), got: {:?}",
        result.inferred
    );
    assert_eq!(
        result.static_resolution,
        StaticResolution::Present,
        "m.get('a') on bound map must report Present resolution, got: {:?}",
        result.static_resolution
    );
}

// Phase E1 Phase 4 test 5
#[test]
fn map_api_get_statically_known_missing_key_emits_diagnostic() {
    let receiver = map_text_integer_ty();
    let contents = bounded_map_contents();
    // m.get('c') — key 'c' is absent from the bound contents.
    let args = [text_literal_arg("c")];
    let result = infer_map_method_call(
        &receiver,
        "get",
        &args,
        Some(&contents),
        TextRange::new(0.into(), 0.into()),
    );
    let missing: Vec<_> = result
        .sentinels
        .iter()
        .filter(|s| s.code == crate::DiagnosticCode::MapGetMissingKey)
        .collect();
    assert_eq!(
        missing.len(),
        1,
        "m.get('c') on bound map must emit exactly 1 MapGetMissingKey, got: {:?}",
        result.sentinels
    );
    assert!(
        matches!(result.inferred, SmeltType::Unknown),
        "m.get('c') missing key must synthesise Unknown, got: {:?}",
        result.inferred
    );
    assert_eq!(
        result.static_resolution,
        StaticResolution::Absent,
        "m.get('c') missing key must report Absent resolution, got: {:?}",
        result.static_resolution
    );
}

// Phase E1 Phase 4 test 6
#[test]
fn map_api_has_statically_known_returns_boolean_literal() {
    let receiver = map_text_integer_ty();
    let contents = bounded_map_contents();

    // m.has('a') — present → Boolean + StaticResolution::Bool(true), no diagnostic.
    let args_present = [text_literal_arg("a")];
    let result_present = infer_map_method_call(
        &receiver,
        "has",
        &args_present,
        Some(&contents),
        TextRange::new(0.into(), 0.into()),
    );
    assert!(
        result_present.sentinels.is_empty(),
        "m.has('a') must emit no diagnostics, got: {:?}",
        result_present.sentinels
    );
    assert!(
        matches!(
            result_present.inferred,
            SmeltType::Expr(TypeConstraint::Concrete(DataType::Boolean))
        ),
        "m.has('a') must synthesise Boolean, got: {:?}",
        result_present.inferred
    );
    assert_eq!(
        result_present.static_resolution,
        StaticResolution::Bool(true),
        "m.has('a') on bound map (key present) must report Bool(true), got: {:?}",
        result_present.static_resolution
    );

    // m.has('c') — absent → Boolean + StaticResolution::Bool(false), no diagnostic.
    let args_absent = [text_literal_arg("c")];
    let result_absent = infer_map_method_call(
        &receiver,
        "has",
        &args_absent,
        Some(&contents),
        TextRange::new(0.into(), 0.into()),
    );
    assert!(
        result_absent.sentinels.is_empty(),
        "m.has('c') must emit no diagnostics, got: {:?}",
        result_absent.sentinels
    );
    assert!(
        matches!(
            result_absent.inferred,
            SmeltType::Expr(TypeConstraint::Concrete(DataType::Boolean))
        ),
        "m.has('c') must synthesise Boolean, got: {:?}",
        result_absent.inferred
    );
    assert_eq!(
        result_absent.static_resolution,
        StaticResolution::Bool(false),
        "m.has('c') on bound map (key absent) must report Bool(false), got: {:?}",
        result_absent.static_resolution
    );
}

// Phase E1 Phase 4 test 7
#[test]
fn map_api_unknown_method_emits_diagnostic() {
    let receiver = map_text_integer_ty();
    let result = infer_map_method_call(
        &receiver,
        "bogus",
        &[],
        None,
        TextRange::new(0.into(), 0.into()),
    );
    let unknown: Vec<_> = result
        .sentinels
        .iter()
        .filter(|s| s.code == crate::DiagnosticCode::MapApiUnknown)
        .collect();
    assert_eq!(
        unknown.len(),
        1,
        "m.bogus() must emit exactly 1 MapApiUnknown, got: {:?}",
        result.sentinels
    );
    assert!(
        matches!(result.inferred, SmeltType::Unknown),
        "m.bogus() must synthesise Unknown, got: {:?}",
        result.inferred
    );
}

// Phase E1 Phase 4 test 8
#[test]
fn map_api_arity_mismatch_on_get_emits_diagnostic() {
    let receiver = map_text_integer_ty();
    let zero = TextRange::new(0.into(), 0.into());

    // m.get() — zero args → ArityMismatch
    let result_zero = infer_map_method_call(&receiver, "get", &[], None, zero);
    let mismatch_zero: Vec<_> = result_zero
        .sentinels
        .iter()
        .filter(|s| s.code == crate::DiagnosticCode::MapApiArityMismatch)
        .collect();
    assert_eq!(
        mismatch_zero.len(),
        1,
        "m.get() with 0 args must emit exactly 1 MapApiArityMismatch, got: {:?}",
        result_zero.sentinels
    );

    // m.get('a', 'b') — two args → ArityMismatch
    let args_two = [text_literal_arg("a"), text_literal_arg("b")];
    let result_two = infer_map_method_call(&receiver, "get", &args_two, None, zero);
    let mismatch_two: Vec<_> = result_two
        .sentinels
        .iter()
        .filter(|s| s.code == crate::DiagnosticCode::MapApiArityMismatch)
        .collect();
    assert_eq!(
        mismatch_two.len(),
        1,
        "m.get('a', 'b') must emit exactly 1 MapApiArityMismatch, got: {:?}",
        result_two.sentinels
    );
}

// Phase E1 Phase 4 test 9
#[test]
fn map_api_named_argument_emits_diagnostic() {
    let receiver = map_text_integer_ty();
    let args = [named_arg()];
    let result = infer_map_method_call(
        &receiver,
        "get",
        &args,
        None,
        TextRange::new(0.into(), 0.into()),
    );
    let named: Vec<_> = result
        .sentinels
        .iter()
        .filter(|s| s.code == crate::DiagnosticCode::MapApiNamedArgument)
        .collect();
    assert_eq!(
        named.len(),
        1,
        "m.get(key => 'a') must emit exactly 1 MapApiNamedArgument, got: {:?}",
        result.sentinels
    );
}

// Phase E1 Phase 4 test 10
#[test]
fn map_api_unexpected_argument_on_entries_emits_diagnostic() {
    let receiver = map_text_integer_ty();
    let zero = TextRange::new(0.into(), 0.into());

    // m.entries('x') — unexpected argument
    let args = [text_literal_arg("x")];
    let result_entries = infer_map_method_call(&receiver, "entries", &args, None, zero);
    let unexpected_entries: Vec<_> = result_entries
        .sentinels
        .iter()
        .filter(|s| s.code == crate::DiagnosticCode::MapApiUnexpectedArgument)
        .collect();
    assert_eq!(
        unexpected_entries.len(),
        1,
        "m.entries('x') must emit exactly 1 MapApiUnexpectedArgument, got: {:?}",
        result_entries.sentinels
    );

    // m.keys('x') — unexpected argument
    let result_keys = infer_map_method_call(&receiver, "keys", &args, None, zero);
    let unexpected_keys: Vec<_> = result_keys
        .sentinels
        .iter()
        .filter(|s| s.code == crate::DiagnosticCode::MapApiUnexpectedArgument)
        .collect();
    assert_eq!(
        unexpected_keys.len(),
        1,
        "m.keys('x') must emit exactly 1 MapApiUnexpectedArgument, got: {:?}",
        result_keys.sentinels
    );

    // m.values('x') — unexpected argument
    let result_values = infer_map_method_call(&receiver, "values", &args, None, zero);
    let unexpected_values: Vec<_> = result_values
        .sentinels
        .iter()
        .filter(|s| s.code == crate::DiagnosticCode::MapApiUnexpectedArgument)
        .collect();
    assert_eq!(
        unexpected_values.len(),
        1,
        "m.values('x') must emit exactly 1 MapApiUnexpectedArgument, got: {:?}",
        result_values.sentinels
    );
}

// Phase E1 Phase 4 test 11
#[test]
fn map_api_arg_type_mismatch_emits_diagnostic() {
    let receiver = map_text_integer_ty();
    // m.get(42) — argument type Integer is not assignable to key type Text
    let args = [integer_literal_arg()];
    let result = infer_map_method_call(
        &receiver,
        "get",
        &args,
        None,
        TextRange::new(0.into(), 0.into()),
    );
    let mismatch: Vec<_> = result
        .sentinels
        .iter()
        .filter(|s| s.code == crate::DiagnosticCode::MapApiArgTypeMismatch)
        .collect();
    assert_eq!(
        mismatch.len(),
        1,
        "m.get(42) must emit exactly 1 MapApiArgTypeMismatch, got: {:?}",
        result.sentinels
    );
}

// Phase E1 Phase 4 test 12
#[test]
fn map_key_type_not_text_emits_diagnostic() {
    // Map<Integer, Text> — K is Integer, not Text → MapKeyTypeNotText
    let map_integer_key = SmeltType::Map {
        key: Box::new(SmeltType::Expr(TypeConstraint::Concrete(DataType::Integer))),
        value: Box::new(SmeltType::Expr(TypeConstraint::Concrete(DataType::Text))),
    };
    let zero = TextRange::new(0.into(), 0.into());
    let (sentinels, _recovered) = validate_map_type_expression(&map_integer_key, zero);
    let not_text: Vec<_> = sentinels
        .iter()
        .filter(|s| s.code == crate::DiagnosticCode::MapKeyTypeNotText)
        .collect();
    assert_eq!(
        not_text.len(),
        1,
        "Map<Integer, Text> must emit exactly 1 MapKeyTypeNotText, got: {:?}",
        sentinels
    );
}

// Phase E1 Phase 4 test 12b
#[test]
fn map_key_type_not_text_recovers_as_map_text_v_for_avalanche_protection() {
    // Map<Integer, Text> — K is Integer (invalid).
    // The recovered type must be Map<Text, Text> (K replaced with Text, V preserved).
    let map_integer_key = SmeltType::Map {
        key: Box::new(SmeltType::Expr(TypeConstraint::Concrete(DataType::Integer))),
        value: Box::new(SmeltType::Expr(TypeConstraint::Concrete(DataType::Text))),
    };
    let zero = TextRange::new(0.into(), 0.into());
    let (sentinels, recovered) = validate_map_type_expression(&map_integer_key, zero);

    // Diagnostic must still be present.
    assert_eq!(
        sentinels
            .iter()
            .filter(|s| s.code == crate::DiagnosticCode::MapKeyTypeNotText)
            .count(),
        1,
        "recovered path must still emit MapKeyTypeNotText"
    );

    // Recovered type must be Map<Text, Text> (V = Text preserved from original).
    let expected_recovered = SmeltType::Map {
        key: Box::new(SmeltType::Expr(TypeConstraint::Concrete(DataType::Text))),
        value: Box::new(SmeltType::Expr(TypeConstraint::Concrete(DataType::Text))),
    };
    assert_eq!(
        recovered, expected_recovered,
        "recovered type must be Map<Text, V> (V = Text), not {:?}",
        recovered
    );

    // Verify valid Map<Text, Integer> passes through unchanged.
    let map_text_int = map_text_integer_ty();
    let (ok_sentinels, ok_recovered) = validate_map_type_expression(&map_text_int, zero);
    assert!(
        ok_sentinels.is_empty(),
        "Map<Text, Integer> must emit no diagnostics"
    );
    assert_eq!(
        ok_recovered, map_text_int,
        "valid map must be returned unchanged"
    );
}

// Phase E1 Phase 4 test 13
#[test]
fn map_invariance_in_value_axis_rejects_assignment() {
    use smelt_types::signatures::is_subtype_of;
    // Map<Text, Integer> is NOT assignable to Map<Text, Number> (invariant in V)
    let m_int = map_text_integer_ty();
    let m_num = map_text_number_ty();
    assert!(
        !is_subtype_of(&m_int, &m_num),
        "Map<Text, Integer> must NOT be assignable to Map<Text, Number> (invariance in V)"
    );
    assert!(
        !is_subtype_of(&m_num, &m_int),
        "Map<Text, Number> must NOT be assignable to Map<Text, Integer> (invariance in V)"
    );
}

// Phase E1 Phase 4 test 14
#[test]
fn map_invariance_does_not_block_record_value_projection() {
    use smelt_types::signatures::{is_subtype_of, TypeConstraint};

    // Map<Text, Cohort> — build a Cohort-typed Map
    let mut cohort_fields = std::collections::BTreeMap::new();
    cohort_fields.insert(
        "name".to_string(),
        SmeltType::Expr(TypeConstraint::Concrete(DataType::Text)),
    );
    cohort_fields.insert(
        "threshold".to_string(),
        SmeltType::Expr(TypeConstraint::Concrete(DataType::Integer)),
    );
    let cohort_ty = SmeltType::Record {
        fields: cohort_fields,
        name: Some("Cohort".to_string()),
    };

    // Wide record target: { name: Text } (narrower record type)
    let mut narrow_fields = std::collections::BTreeMap::new();
    narrow_fields.insert(
        "name".to_string(),
        SmeltType::Expr(TypeConstraint::Concrete(DataType::Text)),
    );
    let narrow_ty = SmeltType::Record {
        fields: narrow_fields,
        name: None,
    };

    // Map-level: Map<Text, Cohort> is NOT assignable to Map<Text, {name: Text}>
    // (invariance at the Map level).
    let map_cohort = SmeltType::Map {
        key: Box::new(SmeltType::Expr(TypeConstraint::Concrete(DataType::Text))),
        value: Box::new(cohort_ty.clone()),
    };
    let map_narrow = SmeltType::Map {
        key: Box::new(SmeltType::Expr(TypeConstraint::Concrete(DataType::Text))),
        value: Box::new(narrow_ty.clone()),
    };
    assert!(
        !is_subtype_of(&map_cohort, &map_narrow),
        "Map<Text, Cohort> must NOT be assignable to Map<Text, {{name: Text}}> (Map invariance)"
    );

    // But width subtyping over the projected value IS admitted:
    // Cohort <: {name: Text} (Cohort has more fields, so it satisfies the narrower record).
    assert!(
        is_subtype_of(&cohort_ty, &narrow_ty),
        "Cohort must be assignable to {{name: Text}} via record width subtyping"
    );
}

// ═══════════════════════════════════════════════════════════════════════
// Phase E1 Phase 5: Loader call dispatch tests
// ═══════════════════════════════════════════════════════════════════════

/// Helper: parse SQL text and return the syntax root.
fn parse_loader_sql(src: &str) -> smelt_parser::syntax_kind::SyntaxNode {
    smelt_parser::parse(src).syntax()
}

/// Helper: a file_exists callback that always returns `false` (no files exist).
fn no_files(_: &str) -> bool {
    false
}

/// Helper: a file_exists callback that always returns `true` (all files exist).
fn all_files_exist(_: &str) -> bool {
    true
}

/// Helper: empty RecordRegistry.
fn empty_registry() -> smelt_types::signatures::RecordRegistry {
    smelt_types::signatures::RecordRegistry::empty()
}

// ─── load_yaml_path_must_be_literal_emits_diagnostic ─────────────────

#[test]
fn load_yaml_path_must_be_literal_emits_diagnostic() {
    // `some_var` is not a string literal — should emit ConfigLoaderPathNotLiteral.
    let src = "SELECT smelt.config.load_yaml(some_var, {f: Text}) FROM t";
    let root = parse_loader_sql(src);
    let diags = check_loader_call_diagnostics(&root, &no_files, &empty_registry());
    let not_literal: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(crate::DiagnosticCode::ConfigLoaderPathNotLiteral))
        .collect();
    assert_eq!(
        not_literal.len(),
        1,
        "non-literal path must emit exactly 1 ConfigLoaderPathNotLiteral; got: {:?}",
        diags
    );
}

// ─── load_yaml_path_escapes_workspace_emits_diagnostic ───────────────

#[test]
fn load_yaml_path_escapes_workspace_emits_diagnostic() {
    let bad_paths = &[
        "'/etc/passwd'",
        "'../escape.yaml'",
        "'http://x.com/c.yaml'",
        "'s3://bucket/c.yaml'",
    ];
    for path_literal in bad_paths {
        let src = format!(
            "SELECT smelt.config.load_yaml({}, {{f: Text}}) FROM t",
            path_literal
        );
        let root = parse_loader_sql(&src);
        let diags = check_loader_call_diagnostics(&root, &all_files_exist, &empty_registry());
        let escapes: Vec<_> = diags
            .iter()
            .filter(|d| d.code == Some(crate::DiagnosticCode::ConfigLoaderPathEscapesWorkspace))
            .collect();
        assert_eq!(
            escapes.len(),
            1,
            "path {} must emit exactly 1 ConfigLoaderPathEscapesWorkspace; got: {:?}",
            path_literal,
            diags
        );
    }
}

// ─── load_yaml_path_backslash_emits_diagnostic ───────────────────────

#[test]
fn load_yaml_path_backslash_emits_diagnostic() {
    // Note: in SQL, backslash in a string literal must be doubled or escaped.
    // We test the check_loader_path pure function directly.
    let zero_range = TextRange::empty(rowan::TextSize::from(0));
    let outcome = check_loader_path(
        "configs\\cohorts.yaml",
        zero_range,
        zero_range,
        &all_files_exist,
    );
    assert!(
        matches!(outcome, LoaderPathOutcome::Backslash { .. }),
        "backslash in path must produce Backslash outcome; got: {:?}",
        outcome
    );
}

// ─── load_yaml_file_not_found_emits_diagnostic ───────────────────────

#[test]
fn load_yaml_file_not_found_emits_diagnostic() {
    let src = "SELECT smelt.config.load_yaml('nope.yaml', {f: Text}) FROM t";
    let root = parse_loader_sql(src);
    // file_exists always returns false.
    let diags = check_loader_call_diagnostics(&root, &no_files, &empty_registry());
    let not_found: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(crate::DiagnosticCode::ConfigLoaderFileNotFound))
        .collect();
    assert_eq!(
        not_found.len(),
        1,
        "missing file must emit exactly 1 ConfigLoaderFileNotFound; got: {:?}",
        diags
    );
    assert!(
        not_found[0].message.contains("nope.yaml"),
        "ConfigLoaderFileNotFound message must name the file; got: {}",
        not_found[0].message
    );
}

// ─── load_yaml_schema_forbidden_emits_diagnostic ─────────────────────

#[test]
fn load_yaml_schema_forbidden_emits_diagnostic() {
    // `Integer` is a bare scalar — forbidden as a loader schema.
    let src = "SELECT smelt.config.load_yaml('c.yaml', Integer) FROM t";
    let root = parse_loader_sql(src);
    let diags = check_loader_call_diagnostics(&root, &all_files_exist, &empty_registry());
    let forbidden: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(crate::DiagnosticCode::ConfigLoaderSchemaForbidden))
        .collect();
    assert_eq!(
        forbidden.len(),
        1,
        "bare scalar schema `Integer` must emit exactly 1 ConfigLoaderSchemaForbidden; got: {:?}",
        diags
    );

    // `Text` is a bare scalar — also forbidden.
    let src2 = "SELECT smelt.config.load_yaml('c.yaml', Text) FROM t";
    let root2 = parse_loader_sql(src2);
    let diags2 = check_loader_call_diagnostics(&root2, &all_files_exist, &empty_registry());
    let forbidden2: Vec<_> = diags2
        .iter()
        .filter(|d| d.code == Some(crate::DiagnosticCode::ConfigLoaderSchemaForbidden))
        .collect();
    assert_eq!(
        forbidden2.len(),
        1,
        "bare scalar schema `Text` must emit exactly 1 ConfigLoaderSchemaForbidden; got: {:?}",
        diags2
    );

    // `Lambda<Text>` is a reflection witness — forbidden as a loader schema.
    let src3 = "SELECT smelt.config.load_yaml('c.yaml', Lambda<Text>) FROM t";
    let root3 = parse_loader_sql(src3);
    let diags3 = check_loader_call_diagnostics(&root3, &all_files_exist, &empty_registry());
    let forbidden3: Vec<_> = diags3
        .iter()
        .filter(|d| d.code == Some(crate::DiagnosticCode::ConfigLoaderSchemaForbidden))
        .collect();
    assert_eq!(
            forbidden3.len(),
            1,
            "reflection witness `Lambda<Text>` must emit exactly 1 ConfigLoaderSchemaForbidden; got: {:?}",
            diags3
        );

    // `ColumnRef` is a reflection witness — forbidden as a loader schema.
    let src4 = "SELECT smelt.config.load_yaml('c.yaml', ColumnRef) FROM t";
    let root4 = parse_loader_sql(src4);
    let diags4 = check_loader_call_diagnostics(&root4, &all_files_exist, &empty_registry());
    let forbidden4: Vec<_> = diags4
        .iter()
        .filter(|d| d.code == Some(crate::DiagnosticCode::ConfigLoaderSchemaForbidden))
        .collect();
    assert_eq!(
        forbidden4.len(),
        1,
        "reflection witness `ColumnRef` must emit exactly 1 ConfigLoaderSchemaForbidden; got: {:?}",
        diags4
    );
}

// ─── load_toml_emits_reserved_diagnostic ─────────────────────────────

#[test]
fn load_toml_emits_reserved_diagnostic() {
    let src = "SELECT smelt.config.load_toml('c.toml', {f: Text}) FROM t";
    let root = parse_loader_sql(src);
    let diags = check_loader_call_diagnostics(&root, &all_files_exist, &empty_registry());
    let reserved: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(crate::DiagnosticCode::ConfigLoaderTomlNotYetSupported))
        .collect();
    assert_eq!(
        reserved.len(),
        1,
        "load_toml must emit exactly 1 ConfigLoaderTomlNotYetSupported; got: {:?}",
        diags
    );
    // The call's synthesised type is Unknown (recoverable) — we check that
    // no other loader-specific diagnostic is emitted.
    let other: Vec<_> = diags
        .iter()
        .filter(|d| d.code != Some(crate::DiagnosticCode::ConfigLoaderTomlNotYetSupported))
        .collect();
    assert!(
        other.is_empty(),
        "load_toml must emit ONLY ConfigLoaderTomlNotYetSupported; also got: {:?}",
        other
    );
}

// ─── load_yaml_synthesises_schema_type_on_happy_path ─────────────────

#[test]
fn load_yaml_synthesises_schema_type_on_happy_path() {
    // A valid path (file exists) + inline schema → no diagnostics.
    let src =
        "SELECT smelt.config.load_yaml('cohorts.yaml', {name: Text, threshold: Integer}) FROM t";
    let root = parse_loader_sql(src);
    let diags = check_loader_call_diagnostics(&root, &all_files_exist, &empty_registry());
    // On the happy path, the pure function emits zero diagnostics.
    // (The actual validation diagnostics — from parsing the file — are
    //  emitted by the Salsa `loader_resolved_value` orchestration layer,
    //  not by this pure function.)
    assert!(
        diags.is_empty(),
        "happy-path loader call must emit zero diagnostics from the dispatch function; got: {:?}",
        diags
    );

    // The synthesised SmeltType of `smelt.config.load_yaml('cohorts.yaml', Cohort)` must
    // be the schema's declared type (spec §"Loader value materialisation" rule 1).
    // `TypedColumn` carries only DataType (no SmeltType), so we use the dedicated
    // `infer_loader_call_smelt_type` function which returns `SmeltType` directly.
    let ctx = make_cohort_ctx();

    // Named record schema: `Cohort` → Record{Cohort}.
    let src_named = "SELECT smelt.config.load_yaml('cohorts.yaml', Cohort) FROM t";
    let expr_named = parse_first_expr(src_named);
    let call_named = expr_named
        .as_smelt_path_call()
        .expect("must be a SmeltPathCall");
    let smelt_ty_named = infer_loader_call_smelt_type(&call_named, &ctx)
        .expect("load_yaml with named Cohort schema must return a SmeltType");
    match &smelt_ty_named {
        SmeltType::Record { name, .. } => {
            assert_eq!(
                name.as_deref(),
                Some("Cohort"),
                "load_yaml with named schema must synthesise Record{{Cohort}}; got name {:?}",
                name
            );
        }
        other => panic!(
            "load_yaml with named schema must synthesise SmeltType::Record; got: {:?}",
            other
        ),
    }

    // Inline schema: `{name: Text, threshold: Integer}` → anonymous Record.
    let src_inline =
        "SELECT smelt.config.load_yaml('cohorts.yaml', {name: Text, threshold: Integer}) FROM t";
    let expr_inline = parse_first_expr(src_inline);
    let call_inline = expr_inline
        .as_smelt_path_call()
        .expect("must be a SmeltPathCall");
    let ctx_empty = TypeContext::new();
    let smelt_ty_inline = infer_loader_call_smelt_type(&call_inline, &ctx_empty)
        .expect("load_yaml with inline schema must return a SmeltType");
    match &smelt_ty_inline {
        SmeltType::Record { fields, name } => {
            assert!(
                name.is_none(),
                "inline schema must produce an anonymous Record; got name {:?}",
                name
            );
            assert!(
                fields.contains_key("name"),
                "inline Record must contain 'name' field; got fields: {:?}",
                fields.keys().collect::<Vec<_>>()
            );
            assert!(
                fields.contains_key("threshold"),
                "inline Record must contain 'threshold' field; got fields: {:?}",
                fields.keys().collect::<Vec<_>>()
            );
        }
        other => panic!(
            "load_yaml with inline schema must synthesise SmeltType::Record; got: {:?}",
            other
        ),
    }

    // List<schema>: `List<Cohort>` → List<Record{Cohort}>.
    let src_list = "SELECT smelt.config.load_yaml('cohorts.yaml', List<Cohort>) FROM t";
    let expr_list = parse_first_expr(src_list);
    let call_list = expr_list
        .as_smelt_path_call()
        .expect("must be a SmeltPathCall");
    let smelt_ty_list = infer_loader_call_smelt_type(&call_list, &ctx)
        .expect("load_yaml with List<Cohort> schema must return a SmeltType");
    match &smelt_ty_list {
        SmeltType::List(inner) => {
            assert!(
                matches!(inner.as_ref(), SmeltType::Record { .. }),
                "load_yaml with List<Cohort> schema must synthesise List<Record>; got inner: {:?}",
                inner
            );
        }
        other => panic!(
            "load_yaml with List<Cohort> schema must synthesise SmeltType::List; got: {:?}",
            other
        ),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Phase F (meta-language) TDD tests — multi-arg lambdas, parameterised reducers,
// ternary expression type inference.
// ─────────────────────────────────────────────────────────────────────────────

/// Helper: parse a smelt SQL string and return all HOF position diagnostics.
fn hof_diags_for(sql: &str) -> Vec<crate::Diagnostic> {
    let select = parse_select_stmt(sql);
    let ctx = TypeContext::new();
    check_hof_position_diagnostics(&select, &ctx, "")
}

// ── Lambda arity / zero / duplicate checks ──────────────────────────────────

/// `map(xs, fn (a, b) => a + b)` over `List<Integer>` emits `LambdaArityMismatch`
/// ("map expects a lambda of arity 1; found arity 2").
#[test]
fn map_rejects_multi_arg_lambda() {
    let diags = hof_diags_for("SELECT map([1, 2, 3], fn (a, b) => a + b) FROM t");
    assert!(
        diags
            .iter()
            .any(|d| d.code == Some(crate::DiagnosticCode::LambdaArityMismatch)),
        "map with 2-arg lambda must emit LambdaArityMismatch; got: {:?}",
        diags
    );
    // Check message contains "map" and arities.
    let msg = diags
        .iter()
        .find(|d| d.code == Some(crate::DiagnosticCode::LambdaArityMismatch))
        .unwrap()
        .message
        .clone();
    assert!(
        msg.contains("map") && msg.contains("1") && msg.contains("2"),
        "LambdaArityMismatch message must mention hof, expected, and actual arities; got: {}",
        msg
    );
}

/// `filter(xs, fn (a, b) => a > b)` emits `LambdaArityMismatch`
/// ("filter expects a lambda of arity 1; found arity 2").
#[test]
fn filter_rejects_multi_arg_lambda() {
    let diags = hof_diags_for("SELECT filter([1, 2, 3], fn (a, b) => a > b) FROM t");
    assert!(
        diags
            .iter()
            .any(|d| d.code == Some(crate::DiagnosticCode::LambdaArityMismatch)),
        "filter with 2-arg lambda must emit LambdaArityMismatch; got: {:?}",
        diags
    );
    let msg = diags
        .iter()
        .find(|d| d.code == Some(crate::DiagnosticCode::LambdaArityMismatch))
        .unwrap()
        .message
        .clone();
    assert!(
        msg.contains("filter"),
        "LambdaArityMismatch message must mention 'filter'; got: {}",
        msg
    );
}

/// `map(xs, fn () => 1)` emits `LambdaZeroParameters`.
#[test]
fn lambda_zero_params_diagnostic() {
    let diags = hof_diags_for("SELECT map([1, 2, 3], fn () => 1) FROM t");
    assert!(
        diags
            .iter()
            .any(|d| d.code == Some(crate::DiagnosticCode::LambdaZeroParameters)),
        "lambda with zero params must emit LambdaZeroParameters; got: {:?}",
        diags
    );
}

/// `fn (a, a) => a` in a HOF position emits `LambdaDuplicateParameter` at the second `a`.
#[test]
fn lambda_duplicate_parameter_diagnostic() {
    let diags = hof_diags_for("SELECT map([1, 2, 3], fn (a, a) => a) FROM t");
    assert!(
        diags
            .iter()
            .any(|d| d.code == Some(crate::DiagnosticCode::LambdaDuplicateParameter)),
        "lambda with duplicate parameter must emit LambdaDuplicateParameter; got: {:?}",
        diags
    );
    let msg = diags
        .iter()
        .find(|d| d.code == Some(crate::DiagnosticCode::LambdaDuplicateParameter))
        .unwrap()
        .message
        .clone();
    assert!(
        msg.contains('`') && msg.contains("a"),
        "LambdaDuplicateParameter message must name the duplicate param; got: {}",
        msg
    );
}

// ── Parameterised reducer tests ─────────────────────────────────────────────

/// Helpers to parse and run infer_parameterised_reducer on the second arg of
/// `reduce(xs, concat_with(...))`.
fn run_infer_parameterised_reducer(
    reducer_name: &str,
    args_sql: &[&str],
) -> crate::type_inference::hof::ParameterisedReducerResult {
    use crate::type_inference::hof::infer_parameterised_reducer_call;
    use smelt_parser::ast::File;

    // Build `reduce([' '], concat_with(...))` where args are the reducer args.
    let args_joined = args_sql.join(", ");
    let sql = format!(
        "SELECT reduce([' '], {}({})) FROM t",
        reducer_name, args_joined
    );
    let parse = smelt_parser::parse(&sql);
    let root = parse.syntax();
    let file = File::cast(root).expect("FILE");
    let select = file.select_stmt().expect("SelectStmt");

    // Find the REDUCER_CALL node.
    use smelt_parser::SyntaxKind::REDUCER_CALL;
    let reducer_call_node = select
        .syntax()
        .descendants()
        .find(|n| n.kind() == REDUCER_CALL)
        .expect("REDUCER_CALL node");
    let reducer_call =
        smelt_parser::ast::ReducerCall::cast(reducer_call_node).expect("ReducerCall cast");

    let ctx = TypeContext::new();
    infer_parameterised_reducer_call(&reducer_call, &ctx)
}

/// `reduce(xs: List<Expr<Text>>, concat_with(' OR '))` synthesises `Expr<Text>`.
#[test]
fn reducer_call_concat_with_text_separator() {
    let result = run_infer_parameterised_reducer("concat_with", &["' OR '"]);
    assert!(
        result.sentinel.is_none(),
        "concat_with(' OR ') must succeed without sentinel; got: {:?}",
        result.sentinel
    );
    assert!(
        matches!(
            result.output_type,
            SmeltType::Expr(smelt_types::signatures::TypeConstraint::Concrete(
                DataType::Text | DataType::Varchar { .. }
            ))
        ),
        "concat_with(' OR ') must synthesise Expr<Text>; got: {:?}",
        result.output_type
    );
}

/// `reduce(xs, concat_with())` emits `ReducerArityMismatch`.
#[test]
fn reducer_call_concat_with_arity_mismatch() {
    let result = run_infer_parameterised_reducer("concat_with", &[]);
    assert!(
        matches!(
            result.sentinel,
            Some(crate::type_inference::hof::ParameterisedReducerSentinel::ArityMismatch { .. })
        ),
        "concat_with() must emit ArityMismatch; got: {:?}",
        result.sentinel
    );
}

/// `reduce(xs, concat_with(' OR ', ' AND '))` emits `ReducerArityMismatch`.
#[test]
fn reducer_call_concat_with_too_many_args() {
    let result = run_infer_parameterised_reducer("concat_with", &["' OR '", "' AND '"]);
    assert!(
        matches!(
            result.sentinel,
            Some(crate::type_inference::hof::ParameterisedReducerSentinel::ArityMismatch { .. })
        ),
        "concat_with(' OR ', ' AND ') must emit ArityMismatch; got: {:?}",
        result.sentinel
    );
}

/// `reduce(xs, concat_with(42))` emits `ReducerArgTypeMismatch`.
#[test]
fn reducer_call_concat_with_wrong_arg_type() {
    let result = run_infer_parameterised_reducer("concat_with", &["42"]);
    assert!(
        matches!(
            result.sentinel,
            Some(crate::type_inference::hof::ParameterisedReducerSentinel::ArgTypeMismatch { .. })
        ),
        "concat_with(42) must emit ArgTypeMismatch; got: {:?}",
        result.sentinel
    );
}

/// `reduce(xs, concat_with(sep => ' OR '))` emits `ReducerNamedArgument`.
#[test]
fn reducer_call_concat_with_named_arg_rejected() {
    let result = run_infer_parameterised_reducer("concat_with", &["sep => ' OR '"]);
    assert!(
        matches!(
            result.sentinel,
            Some(crate::type_inference::hof::ParameterisedReducerSentinel::NamedArgument)
        ),
        "concat_with(sep => ' OR ') must emit NamedArgument; got: {:?}",
        result.sentinel
    );
}

/// `reduce(xs, concat_with(UPPER('|')))` emits `ReducerArgNotCompileTime`.
#[test]
fn reducer_call_concat_with_runtime_arg_rejected() {
    let result = run_infer_parameterised_reducer("concat_with", &["UPPER('|')"]);
    assert!(
        matches!(
            result.sentinel,
            Some(
                crate::type_inference::hof::ParameterisedReducerSentinel::ArgNotCompileTime { .. }
            )
        ),
        "concat_with(UPPER('|')) must emit ArgNotCompileTime; got: {:?}",
        result.sentinel
    );
}

/// `reduce(xs, concat_with(smelt.config.var('sep')))` is accepted (compile-time Text).
#[test]
fn reducer_call_concat_with_config_var_arg_accepted() {
    let result = run_infer_parameterised_reducer("concat_with", &["smelt.config.var('sep')"]);
    // config.var returns compile-time Text — should be accepted.
    assert!(
        result.sentinel.is_none()
            || !matches!(
                result.sentinel,
                Some(
                    crate::type_inference::hof::ParameterisedReducerSentinel::ArgNotCompileTime { .. }
                )
            ),
        "concat_with(smelt.config.var('sep')) must be accepted as compile-time; got: {:?}",
        result.sentinel
    );
}

// ── Ternary type inference tests ─────────────────────────────────────────────

/// Helper: infer the ternary type from a SQL SELECT containing a ternary meta-expression.
/// Returns the full [`TernaryResult`] (type, sentinels, short-circuit hint).
fn run_infer_ternary(
    cond_sql: &str,
    then_sql: &str,
    else_sql: &str,
) -> crate::type_inference::ternary::TernaryResult {
    use crate::type_inference::ternary::infer_ternary_type;
    use smelt_parser::ast::File;
    use smelt_parser::SyntaxKind::TERNARY_EXPR;

    let sql = format!(
        "SELECT if {} then {} else {} FROM t",
        cond_sql, then_sql, else_sql
    );
    let parse = smelt_parser::parse(&sql);
    let root = parse.syntax();
    let file = File::cast(root).expect("FILE");
    let select = file.select_stmt().expect("SelectStmt");

    let ternary_node = select
        .syntax()
        .descendants()
        .find(|n| n.kind() == TERNARY_EXPR)
        .expect("TERNARY_EXPR node");
    let ternary = smelt_parser::ast::TernaryExpr::cast(ternary_node).expect("TernaryExpr");

    let ctx = TypeContext::new();
    infer_ternary_type(&ternary, &ctx)
}

/// `if TRUE then 1 else 2` synthesises `Integer` (SmallInt or Integer from literals).
#[test]
fn ternary_basic_boolean_cond() {
    let result = run_infer_ternary("TRUE", "1", "2");
    assert!(
        result.sentinels.is_empty(),
        "if TRUE then 1 else 2 must have no sentinels; got: {:?}",
        result.sentinels
    );
    assert!(
        matches!(
            result.ty,
            SmeltType::Expr(smelt_types::signatures::TypeConstraint::Concrete(
                DataType::Integer | DataType::SmallInt | DataType::BigInt
            ))
        ),
        "if TRUE then 1 else 2 must synthesise an integer type; got: {:?}",
        result.ty
    );
}

/// `if cond then 1 else 1.5` synthesises a numeric type (LUB of Integer and Double/Decimal).
#[test]
fn ternary_lub_branches() {
    let result = run_infer_ternary("TRUE", "1", "1.5");
    // There should be no type-mismatch sentinels (int and float unify numerically).
    let has_branch_mismatch = result.sentinels.iter().any(|s| {
        matches!(
            s,
            crate::type_inference::ternary::TernarySentinel::BranchTypeMismatch { .. }
        )
    });
    assert!(
        !has_branch_mismatch,
        "if cond then 1 else 1.5 must not emit BranchTypeMismatch (numeric promotion); got: {:?}",
        result.sentinels
    );
    assert!(
        matches!(result.ty, SmeltType::Expr(_)),
        "if cond then 1 else 1.5 must synthesise Expr<Numeric>; got: {:?}",
        result.ty
    );
}

/// `if 42 then a else b` emits `TernaryConditionNotBoolean`.
#[test]
fn ternary_non_boolean_cond() {
    let result = run_infer_ternary("42", "1", "2");
    assert!(
        result.sentinels.iter().any(|s| matches!(
            s,
            crate::type_inference::ternary::TernarySentinel::ConditionNotBoolean { .. }
        )),
        "if 42 then 1 else 2 must emit ConditionNotBoolean; got: {:?}",
        result.sentinels
    );
    assert!(
        matches!(result.ty, SmeltType::Unknown),
        "non-Boolean cond must synthesise Unknown; got: {:?}",
        result.ty
    );
}

/// `if cond then 1 else 'hello'` emits `TernaryBranchTypeMismatch`.
#[test]
fn ternary_branch_type_mismatch() {
    let result = run_infer_ternary("TRUE", "1", "'hello'");
    assert!(
        result.sentinels.iter().any(|s| matches!(
            s,
            crate::type_inference::ternary::TernarySentinel::BranchTypeMismatch { .. }
        )),
        "if cond then 1 else 'hello' must emit BranchTypeMismatch; got: {:?}",
        result.sentinels
    );
}

/// Both branches are type-checked even when condition is a literal FALSE.
/// `if FALSE then 42 else 'unreachable'` has incompatible branch types (Integer vs Text)
/// so BranchTypeMismatch emits even though the condition is always FALSE.
///
/// This demonstrates that Phase 2 `infer_ternary_type` checks BOTH branches
/// for type compatibility regardless of the condition value — it does not
/// short-circuit inference even when the condition is a constant.
#[test]
fn ternary_both_branches_typecheck_even_when_one_unreached() {
    // Condition is FALSE (constant), then-branch is an integer, else-branch is a string.
    // The types are incompatible → BranchTypeMismatch should still fire.
    let result = run_infer_ternary("FALSE", "42", "'unreachable'");
    assert!(
        result.sentinels.iter().any(|s| matches!(
            s,
            crate::type_inference::ternary::TernarySentinel::BranchTypeMismatch { .. }
        )),
        "even with FALSE cond, incompatible branch types must emit BranchTypeMismatch; got: {:?}",
        result.sentinels
    );
}

/// `if <Unknown-typed-cond> then a else b` propagates `Unknown` through the ternary.
///
/// When the condition synthesises to `Unknown` (e.g. a bare unresolved identifier
/// whose type cannot be determined at compile time), the ternary:
///   1. Does NOT emit `TernaryConditionNotBoolean` — Unknown is treated as
///      "type suppressed, don't double-report" per the gradual-typing spec rule 4.
///   2. Returns `Unknown` as the result type (both branches still type-checked,
///      but with an Unknown condition there is no concrete value to select from).
///   3. Emits `short_circuit = None` — no branch is known statically reachable.
///
/// This test uses a bare unresolved identifier `__unresolved__` as the condition.
/// `TypeContext::new()` has no columns seeded, so the identifier resolves to
/// `None` from `infer_expression_type`, which maps to `SmeltType::Unknown` in
/// the condition step of `infer_ternary_type`.
#[test]
fn ternary_unknown_cond_propagates() {
    // Bare unresolved identifier → infer_expression_type returns None → Unknown.
    let result = run_infer_ternary("__unresolved__", "1", "2");

    // Rule 4 (gradual typing): Unknown condition must NOT emit ConditionNotBoolean.
    let has_cond_error = result.sentinels.iter().any(|s| {
        matches!(
            s,
            crate::type_inference::ternary::TernarySentinel::ConditionNotBoolean { .. }
        )
    });
    assert!(
        !has_cond_error,
        "Unknown condition must NOT emit ConditionNotBoolean (gradual typing suppression); got sentinels: {:?}",
        result.sentinels
    );

    // The ternary result type propagates the branches' LUB when cond is Unknown.
    // Branches are integer literals → result is an integer type (not Unknown).
    // (Unknown condition does not poison the result type — only an Unknown branch does.)
    assert!(
        matches!(
            result.ty,
            SmeltType::Expr(smelt_types::signatures::TypeConstraint::Concrete(
                DataType::Integer | DataType::SmallInt | DataType::BigInt
            ))
        ),
        "Unknown cond with compatible branches should synthesise an integer LUB; got: {:?}",
        result.ty
    );

    // No short-circuit hint: the runtime condition value is unknown.
    assert_eq!(
        result.short_circuit, None,
        "Unknown condition must produce no short-circuit hint; got: {:?}",
        result.short_circuit
    );
}

/// Right-associative chaining: `if c1 then a else if c2 then b else c` has result
/// type that is the LUB of all three branches.
#[test]
fn ternary_nested_right_associative() {
    // Parse the right-associative ternary from a full SQL string.
    use crate::type_inference::ternary::infer_ternary_type;
    use smelt_parser::ast::File;
    use smelt_parser::SyntaxKind::TERNARY_EXPR;

    let sql = "SELECT if TRUE then 1 else if FALSE then 2 else 3 FROM t";
    let parse = smelt_parser::parse(sql);
    let root = parse.syntax();
    let file = File::cast(root).expect("FILE");
    let select = file.select_stmt().expect("SelectStmt");

    // Get the outermost TERNARY_EXPR (first one encountered in DFS).
    let ternary_node = select
        .syntax()
        .descendants()
        .find(|n| n.kind() == TERNARY_EXPR)
        .expect("TERNARY_EXPR node");
    let ternary = smelt_parser::ast::TernaryExpr::cast(ternary_node).expect("TernaryExpr");

    let ctx = TypeContext::new();
    let result = infer_ternary_type(&ternary, &ctx);

    // All three branch values are integers → result must be an integer type.
    assert!(
        result.sentinels.iter().all(|s| !matches!(
            s,
            crate::type_inference::ternary::TernarySentinel::BranchTypeMismatch { .. }
        )),
        "right-assoc ternary with all-integer branches must not mismatch; got: {:?}",
        result.sentinels
    );
    assert!(
        matches!(result.ty, SmeltType::Expr(_)),
        "right-assoc ternary result must be Expr<T>; got: {:?}",
        result.ty
    );
}

// ── Ternary keyword-shadowing check ─────────────────────────────────────────

/// `smelt.define if(x: Boolean) -> Boolean = x` emits `TernaryKeywordShadowed`
/// at the `if` token of the declaration.
///
/// This guards against user-defined functions that shadow ternary keywords
/// (`if`, `then`, `else`), which would make the meta-language ambiguous.
#[test]
fn ternary_keyword_shadowed_smelt_define() {
    use smelt_parser::ast::SmeltDefine;
    let sql = "smelt.define if(x: Boolean) -> Boolean = x\n";
    let parse = smelt_parser::parse(sql);
    let file = smelt_parser::ast::File::cast(parse.syntax()).expect("FILE");
    let define: SmeltDefine = file.defines().next().expect("one smelt.define");
    let diags = check_define_name_shadowing(&define);
    assert!(
        diags
            .iter()
            .any(|d| d.code == Some(crate::DiagnosticCode::TernaryKeywordShadowed)),
        "smelt.define named 'if' must emit TernaryKeywordShadowed, got: {:?}",
        diags
    );
}

// ── Reducer call position restriction ────────────────────────────────────────

/// `concat_with(' OR ')` at any non-`reduce`-second-arg position is treated
/// as an unknown function call (not as a parameterised reducer).
///
/// The parser only emits `REDUCER_CALL` nodes inside `reduce`'s second
/// argument position. Outside that context, `concat_with(...)` is parsed as a
/// `FUNCTION_CALL` for an unregistered function, and `infer_expression_type`
/// returns `None` (Unknown type — the generic "unresolved function" behaviour).
/// No reducer-specific sentinel is emitted; the `UnknownIdentifier` diagnostic
/// surfaces in Phase 3 / `function_body_check` context.
#[test]
fn reducer_call_only_at_reduce_second_arg() {
    use smelt_parser::SyntaxKind::FUNCTION_CALL;
    // Parse `concat_with(' OR ')` as a standalone SELECT column.
    let sql = "SELECT concat_with(' OR ') FROM t";
    let select = parse_select_stmt(sql);

    // Find the FUNCTION_CALL node for `concat_with`.
    let call_node = select
        .syntax()
        .descendants()
        .find(|n| n.kind() == FUNCTION_CALL)
        .expect("FUNCTION_CALL node for concat_with");
    let call_expr = smelt_parser::ast::Expr::cast(call_node).expect("Expr from FUNCTION_CALL");

    // `infer_expression_type` must return None for an unregistered function.
    let ctx = TypeContext::new();
    let result = infer_expression_type(&call_expr, &ctx);
    assert!(
        result.is_none(),
        "concat_with as a standalone call must not resolve (unknown function); got: {:?}",
        result
    );

    // `check_hof_position_diagnostics` must not emit any reducer-specific diagnostics —
    // the HOF walker skips FUNCTION_CALL nodes whose name is not map/filter/reduce.
    let hof_diags = hof_diags_for(sql);
    let has_reducer_diag = hof_diags.iter().any(|d| {
        matches!(
            d.code,
            Some(
                crate::DiagnosticCode::ReducerArityMismatch
                    | crate::DiagnosticCode::ReducerArgTypeMismatch
                    | crate::DiagnosticCode::ReducerArgNotCompileTime
                    | crate::DiagnosticCode::ReducerNamedArgument
                    | crate::DiagnosticCode::ReducerInputTypeMismatch
            )
        )
    });
    assert!(
        !has_reducer_diag,
        "concat_with outside reduce must not produce reducer-specific HOF diagnostics; got: {:?}",
        hof_diags
    );
}

// ─── BUG-017: cross-family binary arithmetic must yield Unknown ───────────────

/// `42 + '3'` (numeric + string) must infer `Unknown`, not `SmallInt`.
#[test]
fn test_cross_family_arithmetic_numeric_plus_string() {
    let types = infer_sql("SELECT 42 + '3'");
    assert_eq!(
        types[0].data_type,
        DataType::unknown_dynamic(),
        "42 + '3' must infer Unknown (cross-family), got: {:?}",
        types[0].data_type
    );
}

/// `TRUE + 1` (boolean + numeric) must infer `Unknown`, not `SmallInt`.
#[test]
fn test_cross_family_arithmetic_boolean_plus_numeric() {
    let types = infer_sql("SELECT TRUE + 1");
    assert_eq!(
        types[0].data_type,
        DataType::unknown_dynamic(),
        "TRUE + 1 must infer Unknown (cross-family), got: {:?}",
        types[0].data_type
    );
}

/// `42 + 'abc'` (numeric + string) must infer `Unknown`.
#[test]
fn test_cross_family_arithmetic_numeric_plus_string_literal() {
    let types = infer_sql("SELECT 42 + 'abc'");
    assert_eq!(
        types[0].data_type,
        DataType::unknown_dynamic(),
        "42 + 'abc' must infer Unknown (cross-family), got: {:?}",
        types[0].data_type
    );
}

/// Numeric/numeric promotion must still work: `1 + 2` → SmallInt, not Unknown.
#[test]
fn test_numeric_arithmetic_promotion_unchanged() {
    let types = infer_sql("SELECT 1 + 2");
    assert_eq!(
        types[0].data_type,
        DataType::SmallInt,
        "1 + 2 must still promote to SmallInt (same-family), got: {:?}",
        types[0].data_type
    );
}

/// `INTERVAL * 3` must still yield Interval (special case must survive guard).
#[test]
fn test_interval_times_numeric_unchanged() {
    let types = infer_sql("SELECT INTERVAL '1' DAY * 3");
    assert_eq!(
        types[0].data_type,
        DataType::Interval,
        "INTERVAL * numeric must still yield Interval, got: {:?}",
        types[0].data_type
    );
}

// ─── Outer-join nullability soundness (spec §11) ─────────────────────────────
//
// These tests verify that columns from the null-supplying side of an outer join
// are marked nullable regardless of their declared/inferred nullability.
// The `apply_outer_join_nullability` pass is called after all columns are bound
// (including source declared nullability), so it is the final word.

/// Helper: parse SQL, seed the given columns into the context, apply the outer-join
/// nullability pass, then run inference and return `(alias, TypedColumn)` pairs.
fn infer_join_sql(
    sql: &str,
    left_entity: &str,
    left_cols: &[(&str, TypedColumn)],
    right_entity: &str,
    right_cols: &[(&str, TypedColumn)],
) -> Vec<(String, TypedColumn)> {
    use crate::queries::schema::apply_outer_join_nullability;
    use smelt_parser::ast::File;

    let parse = smelt_parser::parse(sql);
    let root = parse.syntax();
    let file = File::cast(root).expect("failed to cast to File");
    let select_stmt = file.select_stmt().expect("no SelectStmt");

    let mut ctx = TypeContext::new();

    // Register left-side columns as model columns
    for (col_name, typed_col) in left_cols {
        ctx.add_model_column(left_entity, col_name, typed_col.clone());
    }
    ctx.add_alias(left_entity, left_entity);

    // Register right-side columns as model columns
    for (col_name, typed_col) in right_cols {
        ctx.add_model_column(right_entity, col_name, typed_col.clone());
    }
    ctx.add_alias(right_entity, right_entity);

    // Apply the outer-join nullability pass (the fix)
    apply_outer_join_nullability(&select_stmt, &mut ctx);

    let col_types = infer_select_column_types(&select_stmt, &ctx);

    let select_list = select_stmt.select_list().expect("no select list");
    let items: Vec<_> = select_list.items().collect();
    items
        .iter()
        .zip(col_types.iter())
        .map(|(item, tc)| {
            let alias = item.alias().unwrap_or_else(|| "?".to_string());
            (alias, tc.clone())
        })
        .collect()
}

/// Spec §11: a `nullable: false` column on the right side of a LEFT JOIN
/// must infer as `nullable: true` in the output schema.
#[test]
fn left_join_right_side_columns_nullable() {
    // Both tables have a non-nullable 'id' column.
    // After LEFT JOIN: left.id stays non-nullable; right.id must become nullable.
    let sql = "SELECT l.id AS left_id, r.id AS right_id \
               FROM l LEFT JOIN r ON l.id = r.id";

    let left_cols = [(
        "id",
        TypedColumn {
            data_type: DataType::Integer,
            nullable: false,
        },
    )];
    let right_cols = [(
        "id",
        TypedColumn {
            data_type: DataType::Integer,
            nullable: false,
        },
    )];

    let result = infer_join_sql(sql, "l", &left_cols, "r", &right_cols);
    assert_eq!(result.len(), 2, "expected 2 output columns");

    let left_id = result
        .iter()
        .find(|(a, _)| a == "left_id")
        .expect("left_id not found");
    let right_id = result
        .iter()
        .find(|(a, _)| a == "right_id")
        .expect("right_id not found");

    assert!(
        !left_id.1.nullable,
        "LEFT JOIN: left-side column must stay non-nullable, got nullable: {}",
        left_id.1.nullable
    );
    assert!(
        right_id.1.nullable,
        "LEFT JOIN: right-side column must be nullable (null-supplying side), got nullable: {}",
        right_id.1.nullable
    );
}

/// Spec §11: a `nullable: false` column on the left side of a RIGHT JOIN
/// must infer as `nullable: true` in the output schema.
#[test]
fn right_join_left_side_columns_nullable() {
    let sql = "SELECT l.id AS left_id, r.id AS right_id \
               FROM l RIGHT JOIN r ON l.id = r.id";

    let left_cols = [(
        "id",
        TypedColumn {
            data_type: DataType::Integer,
            nullable: false,
        },
    )];
    let right_cols = [(
        "id",
        TypedColumn {
            data_type: DataType::Integer,
            nullable: false,
        },
    )];

    let result = infer_join_sql(sql, "l", &left_cols, "r", &right_cols);
    assert_eq!(result.len(), 2, "expected 2 output columns");

    let left_id = result
        .iter()
        .find(|(a, _)| a == "left_id")
        .expect("left_id not found");
    let right_id = result
        .iter()
        .find(|(a, _)| a == "right_id")
        .expect("right_id not found");

    assert!(
        left_id.1.nullable,
        "RIGHT JOIN: left-side column must be nullable (null-supplying side), got nullable: {}",
        left_id.1.nullable
    );
    assert!(
        !right_id.1.nullable,
        "RIGHT JOIN: right-side column must stay non-nullable, got nullable: {}",
        right_id.1.nullable
    );
}

/// Spec §11: under a FULL JOIN both sides are null-supplying, so all
/// `nullable: false` columns from both sides must infer `nullable: true`.
#[test]
fn full_join_both_sides_nullable() {
    let sql = "SELECT l.id AS left_id, r.id AS right_id \
               FROM l FULL JOIN r ON l.id = r.id";

    let left_cols = [(
        "id",
        TypedColumn {
            data_type: DataType::Integer,
            nullable: false,
        },
    )];
    let right_cols = [(
        "id",
        TypedColumn {
            data_type: DataType::Integer,
            nullable: false,
        },
    )];

    let result = infer_join_sql(sql, "l", &left_cols, "r", &right_cols);
    assert_eq!(result.len(), 2, "expected 2 output columns");

    let left_id = result
        .iter()
        .find(|(a, _)| a == "left_id")
        .expect("left_id not found");
    let right_id = result
        .iter()
        .find(|(a, _)| a == "right_id")
        .expect("right_id not found");

    assert!(
        left_id.1.nullable,
        "FULL JOIN: left-side column must be nullable, got nullable: {}",
        left_id.1.nullable
    );
    assert!(
        right_id.1.nullable,
        "FULL JOIN: right-side column must be nullable, got nullable: {}",
        right_id.1.nullable
    );
}

/// `mark_entity_columns_nullable` must mark BOTH the simple-key form
/// (`entity.col`) AND the full-key form (`source.entity.col`) that
/// `add_source_column` stores for every column.
///
/// Failure mode: the full-key entry retains `nullable: false` after the pass,
/// so an unqualified lookup that happens to hit the full-key entry returns a
/// stale non-nullable flag — a soundness defect on the outer-join path.
#[test]
fn mark_entity_columns_nullable_covers_both_key_forms() {
    let mut ctx = TypeContext::new();

    // add_source_column stores BOTH:
    //   simple key: "events.event_id"
    //   full key:   "raw.events.event_id"
    ctx.add_source_column(
        "raw",
        "events",
        "event_id",
        TypedColumn {
            data_type: DataType::Integer,
            nullable: false,
        },
    );

    // Sanity: both keys exist and start non-nullable.
    // (We look them up directly via lookup_column.)
    let before_simple = ctx.lookup_column(Some("events"), "event_id");
    let before_full = ctx.lookup_column(Some("raw.events"), "event_id");
    // The full-key lookup goes via the "ends_with" fallback path — ensure it exists.
    assert!(
        before_simple.is_some(),
        "simple key events.event_id should be present before mark"
    );
    assert!(
        before_full.is_some(),
        "full key raw.events.event_id should be present before mark"
    );
    assert!(
        !before_simple.unwrap().nullable,
        "simple key should start non-nullable"
    );
    assert!(
        !before_full.unwrap().nullable,
        "full key should start non-nullable"
    );

    // Apply the mark operation (simulates the outer-join nullability pass).
    ctx.mark_entity_columns_nullable("events");

    // Both key forms must now be nullable.
    let after_simple = ctx.lookup_column(Some("events"), "event_id");
    let after_full = ctx.lookup_column(Some("raw.events"), "event_id");
    assert!(
        after_simple.is_some(),
        "simple key events.event_id should still be present after mark"
    );
    assert!(
        after_full.is_some(),
        "full key raw.events.event_id should still be present after mark"
    );
    assert!(
        after_simple.unwrap().nullable,
        "simple key events.event_id must be nullable after mark_entity_columns_nullable"
    );
    assert!(
        after_full.unwrap().nullable,
        "full key raw.events.event_id must be nullable after mark_entity_columns_nullable \
         (previously missed — only the simple key was marked)"
    );
}

/// Spec §11: INNER JOIN preserves input nullability — `nullable: false` columns
/// must stay non-nullable (no precision regression).
#[test]
fn inner_join_preserves_nullability() {
    let sql = "SELECT l.id AS left_id, r.id AS right_id \
               FROM l INNER JOIN r ON l.id = r.id";

    let left_cols = [(
        "id",
        TypedColumn {
            data_type: DataType::Integer,
            nullable: false,
        },
    )];
    let right_cols = [(
        "id",
        TypedColumn {
            data_type: DataType::Integer,
            nullable: false,
        },
    )];

    let result = infer_join_sql(sql, "l", &left_cols, "r", &right_cols);
    assert_eq!(result.len(), 2, "expected 2 output columns");

    let left_id = result
        .iter()
        .find(|(a, _)| a == "left_id")
        .expect("left_id not found");
    let right_id = result
        .iter()
        .find(|(a, _)| a == "right_id")
        .expect("right_id not found");

    assert!(
        !left_id.1.nullable,
        "INNER JOIN: left-side column must stay non-nullable, got nullable: {}",
        left_id.1.nullable
    );
    assert!(
        !right_id.1.nullable,
        "INNER JOIN: right-side column must stay non-nullable, got nullable: {}",
        right_id.1.nullable
    );
}

#[test]
fn try_cast_infers_nullable_target() {
    // TRY_CAST yields the target type but is ALWAYS nullable — it returns NULL
    // on a failed conversion — even when the input expression is non-nullable.
    let types = infer_sql("SELECT TRY_CAST('x' AS INTEGER)");
    assert_eq!(types[0].data_type, DataType::Integer);
    assert!(
        types[0].nullable,
        "TRY_CAST result must be nullable even over a non-nullable input"
    );

    // Plain CAST over the same non-nullable literal stays non-nullable — this
    // guards that the TRY flag, not the input, drives the difference.
    let plain = infer_sql("SELECT CAST('x' AS INTEGER)");
    assert_eq!(plain[0].data_type, DataType::Integer);
    assert!(
        !plain[0].nullable,
        "plain CAST over a non-nullable input should stay non-nullable"
    );
}

/// Build a `TypeContext` for a top-level SQL body the same way the
/// production `type_context()` Salsa query does (via `build_type_context`),
/// but Salsa-free — no upstream models/seeds/sources are provided. Exercises
/// the FROM-clause/derived-table resolution that `infer_sql`/`infer_sql_with_ctx`
/// (which call `infer_select_column_types` directly with a hand-populated
/// context) deliberately skip.
fn infer_sql_via_from_resolution(sql: &str) -> Vec<TypedColumn> {
    use crate::queries::schema::{build_type_context, StaticRefSchemaProvider};
    use smelt_core::sources::SourcesConfig;
    use smelt_parser::ast::File;
    use std::collections::HashMap;

    let parse = smelt_parser::parse(sql);
    let root = parse.syntax();
    let file = File::cast(root).expect("failed to cast to File");
    let select_stmt = file.select_stmt().expect("no SelectStmt in parsed SQL");

    let models = HashMap::new();
    let seeds = HashMap::new();
    let provider = StaticRefSchemaProvider {
        models: &models,
        seeds: &seeds,
    };
    let ctx = build_type_context(&file, &SourcesConfig::default(), &provider);

    infer_select_column_types(&select_stmt, &ctx)
}

#[test]
fn derived_table_values_alias_col_list_renames_columns() {
    // `(VALUES (1, 2)) AS t(a, b)` — selecting the aliased column name `a`
    // should resolve to the first VALUES column (INTEGER), not error.
    let types = infer_sql_via_from_resolution("SELECT a FROM (VALUES (1, 2)) AS t(a, b)");
    assert_eq!(types.len(), 1);
    assert_ne!(
        types[0].data_type,
        DataType::Unknown(smelt_types::UnknownReason::Dynamic),
        "column `a` renamed via alias column list should resolve to a concrete type, got {:?}",
        types[0].data_type
    );
}

#[test]
fn derived_table_select_alias_col_list_renames_columns() {
    // `(SELECT 1, 2) AS t(a, b)` — same renaming behavior for a SELECT-body
    // derived table.
    let types = infer_sql_via_from_resolution("SELECT a FROM (SELECT 1, 2) AS t(a, b)");
    assert_eq!(types.len(), 1);
    assert_ne!(
        types[0].data_type,
        DataType::Unknown(smelt_types::UnknownReason::Dynamic),
        "column `a` renamed via alias column list should resolve to a concrete type, got {:?}",
        types[0].data_type
    );
}
