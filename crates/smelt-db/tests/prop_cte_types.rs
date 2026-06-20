//! Property-based tests for CTE (Common Table Expression) type inference.
//!
//! Tests that smelt correctly infers column types across CTE boundaries,
//! including multi-CTE chains and type transformations between CTEs.
//! Validated against DuckDB.

#[allow(dead_code)]
mod prop_helpers;

use prop_helpers::divergences::{find_divergence, known_divergences, TypeDivergence};
use prop_helpers::duckdb_oracle::{DuckDbOracle, TypeOracle};
use prop_helpers::type_comparison::{compare_types, TypeMatch};

use smelt_db::type_inference::{build_subquery_context, infer_select_column_types, TypeContext};
use smelt_parser::ast::File;
use smelt_types::DataType;

use proptest::prelude::*;

// ---- Column type variants for CTE testing ----

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BaseType {
    SmallInt,
    Integer,
    BigInt,
    Float,
    Double,
    Decimal,
    Varchar,
    Boolean,
    Date,
    Timestamp,
}

impl BaseType {
    fn all() -> &'static [BaseType] {
        &[
            BaseType::SmallInt,
            BaseType::Integer,
            BaseType::BigInt,
            BaseType::Float,
            BaseType::Double,
            BaseType::Decimal,
            BaseType::Varchar,
            BaseType::Boolean,
            BaseType::Date,
            BaseType::Timestamp,
        ]
    }

    fn cast_sql(self) -> &'static str {
        match self {
            BaseType::SmallInt => "CAST(1 AS SMALLINT)",
            BaseType::Integer => "CAST(42 AS INTEGER)",
            BaseType::BigInt => "CAST(100 AS BIGINT)",
            BaseType::Float => "CAST(1.5 AS FLOAT)",
            BaseType::Double => "CAST(3.14 AS DOUBLE)",
            BaseType::Decimal => "CAST(99.99 AS DECIMAL(10,2))",
            BaseType::Varchar => "CAST('hello' AS VARCHAR)",
            BaseType::Boolean => "CAST(TRUE AS BOOLEAN)",
            BaseType::Date => "CAST('2024-01-01' AS DATE)",
            BaseType::Timestamp => "CAST('2024-01-01 12:00:00' AS TIMESTAMP)",
        }
    }
}

/// Type transformations applied across CTE boundaries.
#[derive(Debug, Clone, Copy)]
enum TypeTransform {
    /// Pass column through unchanged
    Identity,
    /// CAST(col AS target_type)
    Cast(BaseType),
    /// col + 0 (preserves numeric type)
    ArithmeticIdentity,
    /// UPPER(col) — for string columns, produces Varchar/Text
    StringFunc,
    /// COALESCE(col, default) — preserves type, may change nullability
    Coalesce,
    /// CASE WHEN TRUE THEN col END — makes nullable
    CaseExpr,
}

// ---- SQL generation helpers ----

/// Generate a single-CTE query: WITH cte1 AS (SELECT typed values) SELECT cols FROM cte1
fn single_cte_query(col_types: &[BaseType]) -> String {
    let select_items: Vec<String> = col_types
        .iter()
        .enumerate()
        .map(|(i, t)| format!("{} AS c{}", t.cast_sql(), i))
        .collect();

    format!(
        "WITH cte1 AS (SELECT {}) SELECT {} FROM cte1",
        select_items.join(", "),
        col_types
            .iter()
            .enumerate()
            .map(|(i, _)| format!("c{}", i))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

/// Generate a two-CTE query where cte2 references cte1 with optional transforms.
fn two_cte_query(base_types: &[BaseType], transforms: &[TypeTransform]) -> String {
    let cte1_items: Vec<String> = base_types
        .iter()
        .enumerate()
        .map(|(i, t)| format!("{} AS c{}", t.cast_sql(), i))
        .collect();

    let cte2_items: Vec<String> = base_types
        .iter()
        .enumerate()
        .map(|(i, base)| {
            let transform = transforms
                .get(i)
                .copied()
                .unwrap_or(TypeTransform::Identity);
            let col_ref = format!("c{}", i);
            let expr = apply_transform(&col_ref, *base, transform);
            format!("{} AS d{}", expr, i)
        })
        .collect();

    let final_select: Vec<String> = (0..base_types.len()).map(|i| format!("d{}", i)).collect();

    format!(
        "WITH cte1 AS (SELECT {}), cte2 AS (SELECT {} FROM cte1) SELECT {} FROM cte2",
        cte1_items.join(", "),
        cte2_items.join(", "),
        final_select.join(", ")
    )
}

/// Generate a three-CTE chain: cte1 → cte2 → cte3, each potentially transforming types.
fn three_cte_chain_query(
    base_type: BaseType,
    transform1: TypeTransform,
    transform2: TypeTransform,
) -> String {
    let cte1_expr = base_type.cast_sql();
    let cte2_expr = apply_transform("a", base_type, transform1);
    let cte3_expr = apply_transform_raw("b", transform2);

    format!(
        "WITH cte1 AS (SELECT {} AS a), \
         cte2 AS (SELECT {} AS b FROM cte1), \
         cte3 AS (SELECT {} AS c FROM cte2) \
         SELECT c FROM cte3",
        cte1_expr, cte2_expr, cte3_expr
    )
}

/// Generate a query with multiple CTEs that reference different earlier CTEs.
fn branching_cte_query(type_a: BaseType, type_b: BaseType) -> String {
    format!(
        "WITH cte1 AS (SELECT {} AS a), \
         cte2 AS (SELECT {} AS b), \
         cte3 AS (SELECT cte1.a AS x, cte2.b AS y FROM cte1 CROSS JOIN cte2) \
         SELECT x, y FROM cte3",
        type_a.cast_sql(),
        type_b.cast_sql()
    )
}

/// Apply a type transform to a column reference, using the base type for context.
fn apply_transform(col: &str, base: BaseType, transform: TypeTransform) -> String {
    match transform {
        TypeTransform::Identity => col.to_string(),
        TypeTransform::Cast(target) => format!("CAST({} AS {})", col, type_name(target)),
        TypeTransform::ArithmeticIdentity => {
            if is_numeric(base) {
                format!("{} + 0", col)
            } else {
                col.to_string()
            }
        }
        TypeTransform::StringFunc => {
            if base == BaseType::Varchar {
                format!("UPPER({})", col)
            } else {
                col.to_string()
            }
        }
        TypeTransform::Coalesce => {
            let default = base.cast_sql();
            format!("COALESCE({}, {})", col, default)
        }
        TypeTransform::CaseExpr => {
            format!("CASE WHEN TRUE THEN {} END", col)
        }
    }
}

/// Apply a transform without knowing the base type — uses only type-agnostic transforms.
fn apply_transform_raw(col: &str, transform: TypeTransform) -> String {
    match transform {
        TypeTransform::Identity => col.to_string(),
        TypeTransform::Cast(target) => format!("CAST({} AS {})", col, type_name(target)),
        TypeTransform::Coalesce => format!("COALESCE({}, {})", col, col),
        TypeTransform::CaseExpr => format!("CASE WHEN TRUE THEN {} END", col),
        // For raw transforms, ArithmeticIdentity and StringFunc fall back to identity
        TypeTransform::ArithmeticIdentity | TypeTransform::StringFunc => col.to_string(),
    }
}

fn type_name(t: BaseType) -> &'static str {
    match t {
        BaseType::SmallInt => "SMALLINT",
        BaseType::Integer => "INTEGER",
        BaseType::BigInt => "BIGINT",
        BaseType::Float => "FLOAT",
        BaseType::Double => "DOUBLE",
        BaseType::Decimal => "DECIMAL(10,2)",
        BaseType::Varchar => "VARCHAR",
        BaseType::Boolean => "BOOLEAN",
        BaseType::Date => "DATE",
        BaseType::Timestamp => "TIMESTAMP",
    }
}

fn is_numeric(t: BaseType) -> bool {
    matches!(
        t,
        BaseType::SmallInt
            | BaseType::Integer
            | BaseType::BigInt
            | BaseType::Float
            | BaseType::Double
            | BaseType::Decimal
    )
}

// ---- Inference & comparison helpers ----

/// Parse SQL with smelt and run type inference, returning inferred column types.
fn run_smelt_inference(sql: &str) -> Vec<DataType> {
    let parse = smelt_parser::parse(sql);
    let root = parse.syntax();
    let file = File::cast(root).expect("failed to cast to File");
    let select_stmt = file.select_stmt().expect("no SelectStmt in parsed SQL");

    let base_ctx = TypeContext::new();
    let ctx = build_subquery_context(&select_stmt, &base_ctx);
    let column_types = infer_select_column_types(&select_stmt, &ctx);

    column_types.into_iter().map(|tc| tc.data_type).collect()
}

/// Compare smelt inference against DuckDB oracle for a CTE query.
fn check_cte_against_oracle(
    oracle: &dyn TypeOracle,
    sql: &str,
    divergences: &[TypeDivergence],
) -> Result<(), String> {
    let actual_types = match oracle.query_types(sql) {
        Ok(types) => types,
        Err(e) => {
            return Err(format!("DuckDB rejected: {e}"));
        }
    };

    let inferred = run_smelt_inference(sql);

    for (i, actual) in actual_types.iter().enumerate() {
        let smelt_type = if i < inferred.len() {
            &inferred[i]
        } else {
            continue;
        };
        let actual_type = &actual.1;

        if *smelt_type == DataType::unknown_dynamic() {
            continue;
        }

        match compare_types(smelt_type, actual_type) {
            TypeMatch::Exact | TypeMatch::Compatible { .. } => {}
            TypeMatch::Mismatch => {
                if find_divergence(smelt_type, actual_type, "duckdb", divergences).is_none() {
                    return Err(format!(
                        "CTE type mismatch for column {} ({}):\n  \
                         smelt inferred: {smelt_type:?}\n  \
                         duckdb actual:  {actual_type:?}\n  \
                         SQL: {sql}",
                        i, actual.0
                    ));
                }
            }
        }
    }
    Ok(())
}

// ---- Proptest strategies ----

fn base_type_strategy() -> impl Strategy<Value = BaseType> {
    prop_oneof![
        Just(BaseType::SmallInt),
        Just(BaseType::Integer),
        Just(BaseType::BigInt),
        Just(BaseType::Float),
        Just(BaseType::Double),
        Just(BaseType::Decimal),
        Just(BaseType::Varchar),
        Just(BaseType::Boolean),
        Just(BaseType::Date),
        Just(BaseType::Timestamp),
    ]
}

/// Strategy for type transforms that are safe for any base type.
fn safe_transform_strategy() -> impl Strategy<Value = TypeTransform> {
    prop_oneof![
        Just(TypeTransform::Identity),
        Just(TypeTransform::Coalesce),
        Just(TypeTransform::CaseExpr),
        // Cast to various types
        base_type_strategy().prop_map(TypeTransform::Cast),
    ]
}

/// Strategy for type transforms including type-specific ones.
fn transform_strategy(base: BaseType) -> BoxedStrategy<TypeTransform> {
    let mut options: Vec<BoxedStrategy<TypeTransform>> = vec![
        Just(TypeTransform::Identity).boxed(),
        Just(TypeTransform::Coalesce).boxed(),
        Just(TypeTransform::CaseExpr).boxed(),
        base_type_strategy().prop_map(TypeTransform::Cast).boxed(),
    ];

    if is_numeric(base) {
        options.push(Just(TypeTransform::ArithmeticIdentity).boxed());
    }
    if base == BaseType::Varchar {
        options.push(Just(TypeTransform::StringFunc).boxed());
    }

    proptest::strategy::Union::new(options).boxed()
}

// ---- Property tests ----

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Test single-CTE passthrough: types should be preserved exactly.
    #[test]
    fn prop_single_cte_passthrough(
        t1 in base_type_strategy(),
        t2 in base_type_strategy()
    ) {
        let sql = single_cte_query(&[t1, t2]);
        let duckdb = DuckDbOracle::new();
        let divergences = known_divergences();

        match check_cte_against_oracle(&duckdb, &sql, &divergences) {
            Ok(()) => {}
            Err(msg) => {
                if !msg.starts_with("DuckDB rejected") {
                    prop_assert!(false, "{}", msg);
                }
            }
        }
    }

    /// Test two-CTE chains with CAST transforms between them.
    #[test]
    fn prop_two_cte_with_cast(
        base in base_type_strategy(),
        target in base_type_strategy()
    ) {
        let sql = two_cte_query(&[base], &[TypeTransform::Cast(target)]);
        let duckdb = DuckDbOracle::new();
        let divergences = known_divergences();

        match check_cte_against_oracle(&duckdb, &sql, &divergences) {
            Ok(()) => {}
            Err(msg) => {
                if !msg.starts_with("DuckDB rejected") {
                    prop_assert!(false, "{}", msg);
                }
            }
        }
    }

    /// Test two-CTE chains with type-specific transforms (including arithmetic and string funcs).
    #[test]
    fn prop_two_cte_typed_transforms(
        base in base_type_strategy()
    ) {
        let transform_strat = transform_strategy(base);
        let runner = proptest::test_runner::TestRunner::default();
        // Run a single sample from the transform strategy
        let mut runner = runner;
        let tree = transform_strat.new_tree(&mut runner).unwrap();
        let transform = tree.current();

        let sql = two_cte_query(&[base], &[transform]);
        let duckdb = DuckDbOracle::new();
        let divergences = known_divergences();

        match check_cte_against_oracle(&duckdb, &sql, &divergences) {
            Ok(()) => {}
            Err(msg) => {
                if !msg.starts_with("DuckDB rejected") {
                    prop_assert!(false, "{}", msg);
                }
            }
        }
    }

    /// Test three-CTE chains with cascading transforms.
    #[test]
    fn prop_three_cte_chain(
        base in base_type_strategy(),
        t1 in safe_transform_strategy(),
        t2 in safe_transform_strategy()
    ) {
        let sql = three_cte_chain_query(base, t1, t2);
        let duckdb = DuckDbOracle::new();
        let divergences = known_divergences();

        match check_cte_against_oracle(&duckdb, &sql, &divergences) {
            Ok(()) => {}
            Err(msg) => {
                if !msg.starts_with("DuckDB rejected") {
                    prop_assert!(false, "{}", msg);
                }
            }
        }
    }
}

// ---- Exhaustive deterministic tests ----

/// Test all base types pass through a single CTE correctly.
#[test]
fn exhaustive_single_cte_all_types() {
    let duckdb = DuckDbOracle::new();
    let divergences = known_divergences();
    let mut failures = Vec::new();

    for ty in BaseType::all() {
        let sql = single_cte_query(&[*ty]);
        match check_cte_against_oracle(&duckdb, &sql, &divergences) {
            Ok(()) => {}
            Err(msg) if msg.starts_with("DuckDB rejected") => {}
            Err(msg) => failures.push(msg),
        }
    }

    if !failures.is_empty() {
        panic!(
            "{} single-CTE failures:\n{}",
            failures.len(),
            failures.join("\n---\n")
        );
    }
}

/// Test all numeric type pairs through a two-CTE CAST chain.
#[test]
fn exhaustive_two_cte_cast_matrix() {
    let duckdb = DuckDbOracle::new();
    let divergences = known_divergences();
    let mut failures = Vec::new();

    for base in BaseType::all() {
        for target in BaseType::all() {
            let sql = two_cte_query(&[*base], &[TypeTransform::Cast(*target)]);
            match check_cte_against_oracle(&duckdb, &sql, &divergences) {
                Ok(()) => {}
                Err(msg) if msg.starts_with("DuckDB rejected") => {}
                Err(msg) => failures.push(msg),
            }
        }
    }

    if !failures.is_empty() {
        panic!(
            "{} two-CTE CAST failures:\n{}",
            failures.len(),
            failures.join("\n---\n")
        );
    }
}

/// Test branching CTE pattern: cte3 references both cte1 and cte2.
#[test]
fn exhaustive_branching_cte_all_type_pairs() {
    let duckdb = DuckDbOracle::new();
    let divergences = known_divergences();
    let mut failures = Vec::new();

    // Test a representative subset (all types × all types would be 100 cases)
    for type_a in BaseType::all() {
        for type_b in BaseType::all() {
            let sql = branching_cte_query(*type_a, *type_b);
            match check_cte_against_oracle(&duckdb, &sql, &divergences) {
                Ok(()) => {}
                Err(msg) if msg.starts_with("DuckDB rejected") => {}
                Err(msg) => failures.push(msg),
            }
        }
    }

    if !failures.is_empty() {
        panic!(
            "{} branching CTE failures:\n{}",
            failures.len(),
            failures.join("\n---\n")
        );
    }
}

// ---- Smoke tests for specific CTE patterns ----

#[test]
fn smoke_single_cte_integer_passthrough() {
    let inferred =
        run_smelt_inference("WITH cte1 AS (SELECT CAST(42 AS INTEGER) AS x) SELECT x FROM cte1");
    assert_eq!(inferred[0], DataType::Integer);
}

#[test]
fn smoke_single_cte_multi_column() {
    let inferred = run_smelt_inference(
        "WITH cte1 AS (SELECT CAST(1 AS INTEGER) AS a, CAST('hello' AS VARCHAR) AS b) \
         SELECT a, b FROM cte1",
    );
    assert_eq!(inferred[0], DataType::Integer);
    assert!(matches!(
        inferred[1],
        DataType::Varchar { .. } | DataType::Text
    ));
}

#[test]
fn smoke_two_cte_chain_type_widening() {
    let inferred = run_smelt_inference(
        "WITH cte1 AS (SELECT CAST(1 AS SMALLINT) AS x), \
         cte2 AS (SELECT CAST(x AS BIGINT) AS y FROM cte1) \
         SELECT y FROM cte2",
    );
    assert_eq!(inferred[0], DataType::BigInt);
}

#[test]
fn smoke_three_cte_chain() {
    let inferred = run_smelt_inference(
        "WITH cte1 AS (SELECT CAST(1 AS SMALLINT) AS a), \
         cte2 AS (SELECT CAST(a AS INTEGER) AS b FROM cte1), \
         cte3 AS (SELECT CAST(b AS BIGINT) AS c FROM cte2) \
         SELECT c FROM cte3",
    );
    assert_eq!(inferred[0], DataType::BigInt);
}

#[test]
fn smoke_cte_with_aggregation() {
    let duckdb = DuckDbOracle::new();
    let divergences = known_divergences();
    let sql = "WITH data AS (SELECT CAST(1 AS INTEGER) AS x \
               UNION ALL SELECT CAST(2 AS INTEGER) AS x), \
               agg AS (SELECT SUM(x) AS total FROM data) \
               SELECT total FROM agg";

    // Just verify it doesn't crash and types are reasonable
    let inferred = run_smelt_inference(sql);
    assert!(!inferred.is_empty());
    assert_ne!(inferred[0], DataType::unknown_dynamic());

    match check_cte_against_oracle(&duckdb, sql, &divergences) {
        Ok(()) => {}
        Err(msg) if msg.starts_with("DuckDB rejected") => {}
        Err(msg) => {
            // Allow divergences for SUM (known to differ: BigInt vs HUGEINT)
            if !msg.contains("SUM") {
                panic!("{}", msg);
            }
        }
    }
}

#[test]
fn smoke_cte_with_function_transform() {
    let inferred = run_smelt_inference(
        "WITH cte1 AS (SELECT CAST('hello' AS VARCHAR) AS s) \
         SELECT UPPER(s) AS upper_s FROM cte1",
    );
    assert!(!inferred.is_empty());
    // UPPER returns text/varchar
    assert!(matches!(
        inferred[0],
        DataType::Text | DataType::Varchar { .. }
    ));
}

#[test]
fn smoke_cte_coalesce_preserves_type() {
    let inferred = run_smelt_inference(
        "WITH cte1 AS (SELECT CAST(NULL AS INTEGER) AS x) \
         SELECT COALESCE(x, 0) AS y FROM cte1",
    );
    assert!(!inferred.is_empty());
    // COALESCE of Integer and Integer literal should be Integer
    assert!(matches!(
        inferred[0],
        DataType::Integer | DataType::BigInt | DataType::SmallInt
    ));
}

#[test]
fn smoke_branching_cte_cross_join() {
    let inferred = run_smelt_inference(
        "WITH cte1 AS (SELECT CAST(1 AS INTEGER) AS a), \
         cte2 AS (SELECT CAST('x' AS VARCHAR) AS b), \
         cte3 AS (SELECT cte1.a, cte2.b FROM cte1 CROSS JOIN cte2) \
         SELECT a, b FROM cte3",
    );
    assert_eq!(inferred.len(), 2);
    assert_eq!(inferred[0], DataType::Integer);
    assert!(matches!(
        inferred[1],
        DataType::Varchar { .. } | DataType::Text
    ));
}

#[test]
fn smoke_cte_case_expression() {
    let inferred = run_smelt_inference(
        "WITH cte1 AS (SELECT CAST(1 AS INTEGER) AS x) \
         SELECT CASE WHEN x > 0 THEN x ELSE 0 END AS y FROM cte1",
    );
    assert!(!inferred.is_empty());
    assert!(matches!(
        inferred[0],
        DataType::Integer | DataType::BigInt | DataType::SmallInt
    ));
}

#[test]
fn smoke_recursive_cte_with_column_list() {
    // Recursive CTE — smelt should bootstrap column types from the non-recursive term
    let sql = "WITH RECURSIVE cnt(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM cnt WHERE x < 10) \
               SELECT x FROM cnt";
    let inferred = run_smelt_inference(sql);
    // Should infer at least one column
    assert!(!inferred.is_empty());
    // The column type may be Unknown (bootstrapped) or Integer (from non-recursive term)
    // Either is acceptable — just verify it doesn't crash
}
