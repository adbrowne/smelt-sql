//! Data generation for nullability property tests.
//!
//! The key insight: we want to test that when smelt claims `nullable: false`, the actual
//! DuckDB results contain zero NULLs, even when nullable source columns are populated
//! with a high density of NULL values.
//!
//! Strategy:
//!   - Build a real DuckDB table (not a CTE with literals) with actual NULL rows.
//!   - Nullable columns get ~50% NULL density.
//!   - Non-nullable columns get zero NULLs.
//!   - The SELECT query mirrors the CTE-based query but references the real table.
//!
//! The harness must observe actual NULLs propagating through expressions.

use super::duckdb_oracle::DuckDbOracle;
use super::generators::{is_aggregate_expr, is_window_expr, QueryShape, TypedExpr, TypedSource};
use smelt_types::DataType;

/// A table name used for all real-data nullability tests.
/// Each oracle connection is a fresh in-memory DB, so no conflicts.
const TABLE_NAME: &str = "tbl";

/// Canonical NULL-bearing literal for a given DataType.
/// Returns the SQL literal `NULL` cast to the appropriate type.
fn null_literal(dt: &DataType) -> String {
    let type_str = sql_type_name(dt);
    format!("CAST(NULL AS {type_str})")
}

/// Non-null literal value for a given DataType.
fn non_null_literal(dt: &DataType, row_idx: usize) -> String {
    match dt {
        DataType::Boolean => {
            if row_idx.is_multiple_of(2) {
                "TRUE".to_string()
            } else {
                "FALSE".to_string()
            }
        }
        DataType::SmallInt => format!("CAST({} AS SMALLINT)", (row_idx % 100) as i16),
        DataType::Integer => format!("CAST({} AS INTEGER)", (row_idx % 1000) as i32),
        DataType::BigInt => format!("CAST({} AS BIGINT)", row_idx as i64),
        DataType::Float => format!("CAST({:.2} AS FLOAT)", row_idx as f64 + 0.5),
        DataType::Double => format!("CAST({:.2} AS DOUBLE)", row_idx as f64 + 0.14),
        DataType::Decimal { precision, scale } => {
            let val = row_idx as f64 * 1.23 + 1.0;
            format!("CAST({val:.2} AS DECIMAL({precision},{scale}))")
        }
        DataType::Varchar { .. } | DataType::Text | DataType::Char { .. } => {
            format!("CAST('val{row_idx}' AS VARCHAR)")
        }
        DataType::Date => {
            // Vary the year to produce distinct dates; keep month/day in valid range.
            let year = 2000 + (row_idx % 24) as i32;
            let month = (row_idx % 12) as i32 + 1;
            let day = (row_idx % 28) as i32 + 1;
            format!("CAST('{year:04}-{month:02}-{day:02}' AS DATE)")
        }
        DataType::Timestamp { .. } => {
            let year = 2000 + (row_idx % 24) as i32;
            let month = (row_idx % 12) as i32 + 1;
            let day = (row_idx % 28) as i32 + 1;
            let hour = row_idx % 24;
            format!("CAST('{year:04}-{month:02}-{day:02} {hour:02}:00:00' AS TIMESTAMP)")
        }
        DataType::Time => {
            let hour = row_idx % 24;
            let min = row_idx % 60;
            format!("CAST('{hour:02}:{min:02}:00' AS TIME)")
        }
        DataType::Interval => {
            format!("CAST('{} days' AS INTERVAL)", (row_idx % 30) + 1)
        }
        // For complex types and unknowns, fall back to NULL (will be caught by DDL failure)
        _ => null_literal(dt),
    }
}

/// SQL type name for CREATE TABLE DDL.
fn sql_type_name(dt: &DataType) -> &'static str {
    match dt {
        DataType::Boolean => "BOOLEAN",
        DataType::SmallInt => "SMALLINT",
        DataType::Integer => "INTEGER",
        DataType::BigInt => "BIGINT",
        DataType::Float => "FLOAT",
        DataType::Double => "DOUBLE",
        DataType::Decimal { .. } => "DECIMAL(10,2)",
        DataType::Varchar { .. } | DataType::Text | DataType::Char { .. } => "VARCHAR",
        DataType::Date => "DATE",
        DataType::Timestamp { .. } => "TIMESTAMP",
        DataType::Time => "TIME",
        DataType::Interval => "INTERVAL",
        DataType::Blob => "BLOB",
        DataType::Array(_) => "JSON",
        DataType::Struct(_) => "JSON",
        DataType::Map(_, _) => "JSON",
        DataType::Null | DataType::Unknown(_) => "VARCHAR",
    }
}

/// Generate a `CREATE TABLE tbl (col1 TYPE1, ...)` statement.
fn create_table_ddl(columns: &[TypedSource]) -> String {
    let col_defs: Vec<String> = columns
        .iter()
        .map(|c| {
            let type_str = sql_type_name(&c.data_type);
            format!("{} {}", c.name, type_str)
        })
        .collect();
    format!("CREATE TABLE {TABLE_NAME} ({})", col_defs.join(", "))
}

/// Generate `INSERT INTO tbl VALUES (...)` for a set of rows.
///
/// NULL density: ~50% for nullable columns (every other row gets NULL).
/// For this harness, all source columns are treated as nullable (the generator
/// only creates nullable sources), so we produce actual NULLs in ~50% of rows.
fn insert_rows_ddl(columns: &[TypedSource], n_rows: usize) -> String {
    let mut rows = Vec::new();
    for row_idx in 0..n_rows {
        let vals: Vec<String> = columns
            .iter()
            .map(|c| {
                // ~50% NULL density: odd rows get NULL for this column
                // We rotate which row goes NULL per column to ensure overlap
                let col_offset = c.name.len(); // deterministic per-column offset
                if (row_idx + col_offset) % 2 == 1 {
                    null_literal(&c.data_type)
                } else {
                    non_null_literal(&c.data_type, row_idx)
                }
            })
            .collect();
        rows.push(format!("({})", vals.join(", ")));
    }
    format!("INSERT INTO {TABLE_NAME} VALUES {}", rows.join(", "))
}

/// Build the SELECT query over the real table (not a CTE).
/// Mirrors `assemble_cte_query` but uses `FROM tbl` instead of `FROM data`.
fn build_select_query(exprs: &[TypedExpr], shape: &QueryShape) -> Option<String> {
    // For the real-table SELECT, we need to re-express group columns.
    // The `assemble_cte_query` already produced the right structure;
    // we mirror it here but substitute `data` → `tbl`.
    let select_sql = match shape {
        QueryShape::GroupBy { group_columns }
        | QueryShape::GroupByHaving { group_columns, .. }
        | QueryShape::GroupByWindow { group_columns } => {
            let mut select_items: Vec<String> = Vec::new();
            for (i, gc) in group_columns.iter().enumerate() {
                select_items.push(format!("{gc} AS grp_{i}"));
            }
            let agg_exprs: Vec<&TypedExpr> =
                exprs.iter().filter(|e| is_aggregate_expr(&e.sql)).collect();
            if agg_exprs.is_empty() {
                select_items.push("COUNT(*) AS expr_0".to_string());
            } else {
                for e in &agg_exprs {
                    select_items.push(format!("{} AS {}", e.sql, e.alias));
                }
            }
            if matches!(shape, QueryShape::GroupByWindow { .. }) {
                let order_expr = if !agg_exprs.is_empty() {
                    agg_exprs[0].sql.clone()
                } else {
                    "COUNT(*)".to_string()
                };
                select_items.push(format!("RANK() OVER (ORDER BY {order_expr}) AS win_rank"));
            }
            let having_clause = if let QueryShape::GroupByHaving {
                having_predicate, ..
            } = shape
            {
                format!(" HAVING {having_predicate}")
            } else {
                String::new()
            };
            let group_by_clause = format!("GROUP BY {}", group_columns.join(", "));
            format!(
                "SELECT {} FROM {TABLE_NAME} {group_by_clause}{having_clause}",
                select_items.join(", ")
            )
        }
        QueryShape::Distinct => {
            let scalar_exprs: Vec<&TypedExpr> = exprs
                .iter()
                .filter(|e| !is_aggregate_expr(&e.sql) && !is_window_expr(&e.sql))
                .collect();
            if scalar_exprs.is_empty() {
                return None; // can't build without expressions
            }
            let selected: Vec<String> = scalar_exprs
                .iter()
                .map(|e| format!("{} AS {}", e.sql, e.alias))
                .collect();
            format!("SELECT DISTINCT {} FROM {TABLE_NAME}", selected.join(", "))
        }
        QueryShape::Scalar | QueryShape::Window => {
            let has_aggregate = exprs.iter().any(|e| is_aggregate_expr(&e.sql));
            let has_window = exprs.iter().any(|e| is_window_expr(&e.sql));
            let selected_exprs: Vec<&TypedExpr> = if has_aggregate && has_window {
                exprs
                    .iter()
                    .filter(|e| !is_aggregate_expr(&e.sql))
                    .collect()
            } else if has_aggregate {
                exprs.iter().filter(|e| is_aggregate_expr(&e.sql)).collect()
            } else {
                exprs.iter().collect()
            };
            let selected_exprs = if selected_exprs.is_empty() {
                exprs.iter().collect()
            } else {
                selected_exprs
            };
            if selected_exprs.is_empty() {
                return None;
            }
            let select_list: Vec<String> = selected_exprs
                .iter()
                .map(|e| format!("{} AS {}", e.sql, e.alias))
                .collect();
            format!("SELECT {} FROM {TABLE_NAME}", select_list.join(", "))
        }
    };

    // Rewrite `FROM data` references inside subqueries to `FROM tbl`
    // (EXISTS subqueries reference `data` by name from the CTE)
    let rewritten = select_sql.replace("FROM data", &format!("FROM {TABLE_NAME}"));
    Some(rewritten)
}

/// Build a pair of `(setup_sql, select_sql)` for the real-data nullability check.
///
/// `setup_sql` is a multi-statement string: `CREATE TABLE ... ; INSERT INTO ...`.
/// `select_sql` is the SELECT to run for the nullability assertion.
///
/// Returns `None` if the shape/expressions don't support real-table generation.
pub fn build_null_bearing_query(
    columns: &[TypedSource],
    exprs: &[TypedExpr],
    shape: &QueryShape,
) -> Option<(String, String)> {
    let create = create_table_ddl(columns);
    let insert = insert_rows_ddl(columns, 10); // 10 rows → ~5 NULLs per nullable column
    let setup = format!("{create};\n{insert}");
    let select = build_select_query(exprs, shape)?;
    Some((setup, select))
}

/// Check nullability soundness: for each column in `expected_nullable` that is `false`,
/// the column at the same position in DuckDB results must have null_count == 0.
///
/// Returns a list of violation descriptions (empty = sound).
pub fn check_nullability_soundness(
    oracle: &DuckDbOracle,
    sql: &str,
    expected_nullable: &[(String, bool)],
) -> Result<Vec<String>, String> {
    let observed = oracle.count_nulls_per_column(sql)?;
    let mut violations = Vec::new();
    for (i, (name, nullable)) in expected_nullable.iter().enumerate() {
        if !nullable {
            let null_count = observed.get(i).map(|(_, c)| *c).unwrap_or(0);
            if null_count > 0 {
                violations.push(format!(
                    "column {} ({}) inferred non-nullable but DuckDB returned {} NULLs",
                    i, name, null_count
                ));
            }
        }
    }
    Ok(violations)
}

// ---- Smoke test setup helpers ----

/// Returns (setup_sql, check_sql, expected_nullability) for the COALESCE smoke.
///
/// `COALESCE(nullable_col, 0)` over a nullable INTEGER column.
/// smelt should infer this as `nullable: false` (a non-null literal fallback).
/// DuckDB must return zero NULLs.
pub fn smoke_coalesce_non_nullable_setup() -> (String, String, Vec<(String, bool)>) {
    let setup = format!(
        "CREATE TABLE {TABLE_NAME} (x INTEGER);\n\
         INSERT INTO {TABLE_NAME} VALUES (NULL), (1), (NULL), (2), (NULL)"
    );
    let check = format!("SELECT COALESCE(x, 0) AS result FROM {TABLE_NAME}");
    // smelt will infer COALESCE(nullable, literal) as non-nullable
    let expected = vec![("result".to_string(), false)];
    (setup, check, expected)
}

/// Returns (setup_sql, check_sql, col_name) for the nullable passthrough smoke.
///
/// A nullable INTEGER column projected as-is.
/// The test MUST observe actual NULLs in results (null_count > 0).
pub fn smoke_nullable_passthrough_setup() -> (String, String, String) {
    let setup = format!(
        "CREATE TABLE {TABLE_NAME} (x INTEGER);\n\
         INSERT INTO {TABLE_NAME} VALUES (NULL), (1), (NULL), (2), (NULL)"
    );
    let check = format!("SELECT x AS x_out FROM {TABLE_NAME}");
    let col_name = "x_out".to_string();
    (setup, check, col_name)
}

// ---- Join test helpers ----

/// The left and right table names used in two-table join tests.
/// Each oracle connection is a fresh in-memory DB, so no conflicts.
pub const LEFT_TABLE: &str = "left_tbl";
pub const RIGHT_TABLE: &str = "right_tbl";

/// A join type used in the join property test.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinKind {
    Inner,
    Left,
    Right,
    Full,
}

impl JoinKind {
    pub fn sql_keyword(self) -> &'static str {
        match self {
            JoinKind::Inner => "INNER",
            JoinKind::Left => "LEFT",
            JoinKind::Right => "RIGHT",
            JoinKind::Full => "FULL",
        }
    }
}

/// Build a `CREATE TABLE name (col TYPE, ..., lkey INTEGER)` statement.
pub fn create_keyed_table_ddl(table_name: &str, columns: &[TypedSource]) -> String {
    let col_defs: Vec<String> = columns
        .iter()
        .map(|c| {
            let type_str = sql_type_name(&c.data_type);
            format!("{} {}", c.name, type_str)
        })
        .collect();
    // Append an integer join key column.
    format!(
        "CREATE TABLE {table_name} ({}, jkey INTEGER)",
        col_defs.join(", ")
    )
}

/// Insert rows into a keyed table.
///
/// `key_value` is the INTEGER value for the `jkey` column — all rows get the same key value.
/// Nullable columns at ~50% NULL density; key column is never NULL.
pub fn insert_keyed_rows_ddl(
    table_name: &str,
    columns: &[TypedSource],
    n_rows: usize,
    key_value: i64,
) -> String {
    let mut rows = Vec::new();
    for row_idx in 0..n_rows {
        let vals: Vec<String> = columns
            .iter()
            .map(|c| {
                let col_offset = c.name.len();
                if (row_idx + col_offset) % 2 == 1 {
                    null_literal(&c.data_type)
                } else {
                    non_null_literal(&c.data_type, row_idx)
                }
            })
            .collect();
        // Append the join key.
        let row_vals = format!("{}, {}", vals.join(", "), key_value);
        rows.push(format!("({row_vals})"));
    }
    format!("INSERT INTO {table_name} VALUES {}", rows.join(", "))
}

/// Build the CTE query for smelt inference of a two-table join.
///
/// Structure:
/// ```sql
/// WITH l AS (SELECT col_casts..., CAST(1 AS INTEGER) AS lkey, 42 AS l_guard),
///      r AS (SELECT r_col_casts..., CAST(999 AS INTEGER) AS rkey, 42 AS r_guard)
/// SELECT l.lcol_0 AS l_lcol_0, ..., r.rcol_0 AS r_rcol_0, ..., l.l_guard AS l_guard, r.r_guard AS r_guard
/// FROM l [JOIN_KIND] JOIN r ON l.lkey = r.rkey
/// ```
///
/// Keys are disjoint (1 vs 999) so for outer joins no rows match.
pub fn build_join_cte_query(
    left_cols: &[TypedSource],
    right_cols: &[TypedSource],
    join_kind: JoinKind,
) -> String {
    // Left CTE — prefix "l_" to avoid name collision
    let left_cte_items: Vec<String> = left_cols
        .iter()
        .map(|c| format!("{} AS l_{}", c.cast_sql, c.name))
        .collect();
    let left_cte = format!(
        "SELECT {}, CAST(1 AS INTEGER) AS lkey, 42 AS l_guard",
        left_cte_items.join(", ")
    );

    // Right CTE — prefix "r_" to avoid name collision
    let right_cte_items: Vec<String> = right_cols
        .iter()
        .map(|c| format!("{} AS r_{}", c.cast_sql, c.name))
        .collect();
    let right_cte = format!(
        "SELECT {}, CAST(999 AS INTEGER) AS rkey, 42 AS r_guard",
        right_cte_items.join(", ")
    );

    // SELECT list: all left cols with l. qualifier, then all right cols with r. qualifier,
    // then the guards.
    let mut select_items: Vec<String> = Vec::new();
    for c in left_cols {
        select_items.push(format!("l.l_{} AS l_{}", c.name, c.name));
    }
    for c in right_cols {
        select_items.push(format!("r.r_{} AS r_{}", c.name, c.name));
    }
    select_items.push("l.l_guard AS l_guard".to_string());
    select_items.push("r.r_guard AS r_guard".to_string());

    let join_keyword = join_kind.sql_keyword();
    format!(
        "WITH l AS ({left_cte}), r AS ({right_cte}) \
         SELECT {select} FROM l {join_keyword} JOIN r ON l.lkey = r.rkey",
        select = select_items.join(", ")
    )
}

/// Build the setup SQL (CREATE + INSERT) and SELECT SQL for a two-table join DuckDB check.
///
/// Uses disjoint key ranges: left table has jkey=1, right table has jkey=999.
/// This guarantees outer joins produce NULL-supplying rows for the null-side columns.
pub fn build_join_real_table_query(
    left_cols: &[TypedSource],
    right_cols: &[TypedSource],
    join_kind: JoinKind,
) -> (String, String) {
    let create_left = create_keyed_table_ddl(LEFT_TABLE, left_cols);
    let insert_left = insert_keyed_rows_ddl(LEFT_TABLE, left_cols, 5, 1); // key=1
    let create_right = create_keyed_table_ddl(RIGHT_TABLE, right_cols);
    let insert_right = insert_keyed_rows_ddl(RIGHT_TABLE, right_cols, 5, 999); // key=999 (disjoint)

    let setup = format!("{create_left};\n{insert_left};\n{create_right};\n{insert_right}");

    // SELECT list matching the CTE query aliases exactly.
    let mut select_items: Vec<String> = Vec::new();
    for c in left_cols {
        select_items.push(format!("l.{} AS l_{}", c.name, c.name));
    }
    for c in right_cols {
        select_items.push(format!("r.{} AS r_{}", c.name, c.name));
    }
    select_items.push("l.l_guard AS l_guard".to_string());
    select_items.push("r.r_guard AS r_guard".to_string());

    let join_keyword = join_kind.sql_keyword();
    let select_sql = format!(
        "SELECT {select} FROM {LEFT_TABLE} l {join_keyword} JOIN {RIGHT_TABLE} r ON l.jkey = r.jkey",
        select = select_items.join(", ")
    );

    (setup, select_sql)
}

// ---- Set-operation test helpers ----

/// Build the CTE query for smelt inference of a UNION ALL with mixed nullability.
///
/// Branch 1: `42 AS guard` (non-nullable), `col_cast AS col` (nullable)
/// Branch 2: `CAST(NULL AS INTEGER) AS guard` (nullable), `col_cast AS col` (nullable)
///
/// The guard column should infer as nullable (42 UNION NULL → nullable per §11 set-op rule).
pub fn build_setop_mixed_nullability_cte() -> String {
    "WITH data AS (SELECT CAST(42 AS INTEGER) AS x) \
     SELECT 42 AS guard, x FROM data \
     UNION ALL \
     SELECT CAST(NULL AS INTEGER) AS guard, x FROM data"
        .to_string()
}

/// Build the CTE query for smelt inference of a UNION ALL with uniform non-nullable guard.
///
/// Both branches have `42 AS guard` (non-nullable).
/// The guard column should infer as non-nullable (42 UNION ALL 42 → non-nullable).
pub fn build_setop_uniform_nonnullable_cte() -> String {
    "WITH data AS (SELECT CAST(42 AS INTEGER) AS x) \
     SELECT 42 AS guard, x FROM data \
     UNION ALL \
     SELECT 42 AS guard, x FROM data"
        .to_string()
}

// is_aggregate_expr and is_window_expr are imported from generators.rs — the single
// source of truth.  Do NOT duplicate them here; use the imported versions above.
