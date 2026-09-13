//! The single dialect-aware owner for inline row-set construction.
//!
//! Every production path that needs to splice a small literal row set into
//! generated SQL — a seed's ephemeral CTE, a repair's affected-key list, an
//! append-only baseline probe's recorded partitions, a `smelt.test` mock
//! dataset — goes through [`row_set_body`] (or its FROM/JOIN-position
//! wrapper, [`build_row_set_table`]) rather than formatting `VALUES (…)`
//! itself.
//!
//! GoogleSQL has no table-value constructor: `FROM (VALUES (1), (2))` is a
//! syntax error (`400 Syntax error: Expected keyword JOIN but got ')'`,
//! measured live against BigQuery). What [`BackendType::BigQuery`] renders
//! instead is `SELECT * FROM UNNEST([STRUCT(… AS col), (…)])` — GoogleSQL's
//! own array-of-structs form, **one** query operand no matter how many rows
//! it carries. Every other dialect keeps the `VALUES` table-value
//! constructor unchanged.
//!
//! The chained `SELECT … UNION ALL SELECT …` this used to render is also
//! valid GoogleSQL, and it does not scale: one query operand *per row*. A
//! live dogfood run refused a 5,797-row baseline probe outright — *"Not
//! enough resources for query planning - too many subqueries or query is
//! too complex"* at 692,597 characters
//! (`docs/outcomes/20260906-bigquery-dogfood-spine/phases/12-summary.md`
//! finding 1b). The array form has no per-row planning cost.
//!
//! The first array element carries the column names as `AS` aliases and
//! therefore types the whole array; later elements are bare tuples. A first
//! row whose cell is an untyped `NULL` literal would type that field from
//! `NULL` — a hazard the `UNION ALL` form shared, since GoogleSQL types a
//! bare `NULL` as `INT64` in both — so callers that may emit NULL in the
//! first row cast it (`seeds::ephemeral` does).
//!
//! Both functions require a non-empty row set — deciding what an *empty*
//! row set means (an always-false guard row, a `WHERE FALSE` predicate with
//! no row at all, …) is a per-caller business decision, not a row-set
//! construction detail, so callers handle the empty case themselves before
//! reaching this module.

use crate::config::BackendType;

/// The dialect-appropriate body of an inline row-set constructor: a
/// `VALUES (…), (…)` table-value constructor for dialects that support one,
/// or the `SELECT * FROM UNNEST([STRUCT(…), …])` array form GoogleSQL
/// requires.
///
/// `columns` names the row set's columns. For the `VALUES` form the names
/// are not embedded in the body (the caller supplies them separately, e.g.
/// as a CTE's or derived table's external column list); for the `UNNEST`
/// form they alias the **first** array element's fields, which is what
/// names the fields of every element and types the array.
///
/// `rows[i][j]` is the already-formatted SQL literal for row `i`, column
/// `j`.
///
/// # Panics
/// Panics if `rows` is empty — an empty row set has no single portable
/// rendering (a bare `VALUES ()` is invalid SQL on every dialect); callers
/// decide what an empty row set means for their own call site and never
/// reach this function in that case.
pub fn row_set_body(dialect: BackendType, columns: &[&str], rows: &[Vec<String>]) -> String {
    assert!(
        !rows.is_empty(),
        "row_set_body requires at least one row; callers own the empty-row-set case"
    );
    match dialect {
        BackendType::DuckDB | BackendType::Spark | BackendType::Databricks => {
            let rows_sql: Vec<String> =
                rows.iter().map(|r| format!("({})", r.join(", "))).collect();
            format!("VALUES {}", rows_sql.join(", "))
        }
        BackendType::BigQuery => {
            let elements: Vec<String> = rows
                .iter()
                .enumerate()
                .map(|(i, r)| {
                    if i == 0 {
                        // The first element names the fields and thereby
                        // types the array.
                        let projected: Vec<String> = r
                            .iter()
                            .zip(columns.iter())
                            .map(|(lit, name)| format!("{lit} AS {name}"))
                            .collect();
                        format!("STRUCT({})", projected.join(", "))
                    } else {
                        format!("({})", r.join(", "))
                    }
                })
                .collect();
            format!("SELECT * FROM UNNEST([{}])", elements.join(", "))
        }
    }
}

/// The full derived-table expression for an inline row set, ready to splice
/// directly into a `FROM`/`JOIN` clause: `(VALUES …) AS alias(cols)` for
/// dialects with a table-value constructor, or
/// `(SELECT * FROM UNNEST([STRUCT(…), …])) AS alias` for GoogleSQL — column
/// names come from the first array element's field aliases in that case, so
/// the outer alias carries no column list (GoogleSQL's `AS alias(cols)`
/// external column-list syntax on a derived table is not exercised here).
///
/// # Panics
/// Panics if `rows` is empty; see [`row_set_body`].
pub fn build_row_set_table(
    dialect: BackendType,
    alias: &str,
    columns: &[&str],
    rows: &[Vec<String>],
) -> String {
    match dialect {
        BackendType::DuckDB | BackendType::Spark | BackendType::Databricks => {
            format!(
                "({}) AS {alias}({})",
                row_set_body(dialect, columns, rows),
                columns.join(", ")
            )
        }
        BackendType::BigQuery => {
            format!("({}) AS {alias}", row_set_body(dialect, columns, rows))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows() -> Vec<Vec<String>> {
        vec![
            vec!["1".to_string(), "'North'".to_string()],
            vec!["2".to_string(), "'South'".to_string()],
        ]
    }

    /// The rejection test: BigQuery's emitted form is never a `FROM
    /// (VALUES …)` table-value constructor.
    #[test]
    fn bigquery_row_set_body_is_not_a_values_table_constructor() {
        let body = row_set_body(BackendType::BigQuery, &["id", "name"], &rows());
        assert!(
            !body.contains("VALUES"),
            "BigQuery has no table-value constructor, got: {body}"
        );
        assert!(
            !body.contains("UNION ALL"),
            "the UNION ALL rewrite costs one query operand per row, got: {body}"
        );
        assert_eq!(
            body,
            "SELECT * FROM UNNEST([STRUCT(1 AS id, 'North' AS name), (2, 'South')])"
        );
    }

    #[test]
    fn bigquery_row_set_table_is_not_a_values_table_constructor() {
        let table = build_row_set_table(BackendType::BigQuery, "t", &["id", "name"], &rows());
        assert!(
            !table.contains("VALUES"),
            "BigQuery has no table-value constructor, got: {table}"
        );
        assert_eq!(
            table,
            "(SELECT * FROM UNNEST([STRUCT(1 AS id, 'North' AS name), (2, 'South')])) AS t"
        );
    }

    /// DuckDB and Spark keep today's `VALUES` table-value constructor,
    /// byte-identical to each other and to the pre-existing hand-written
    /// form (`(VALUES (…), (…)) AS alias(cols)`).
    #[test]
    fn duckdb_and_spark_row_set_table_is_byte_identical() {
        let duckdb = build_row_set_table(BackendType::DuckDB, "t", &["id", "name"], &rows());
        let spark = build_row_set_table(BackendType::Spark, "t", &["id", "name"], &rows());
        assert_eq!(duckdb, spark);
        assert_eq!(duckdb, "(VALUES (1, 'North'), (2, 'South')) AS t(id, name)");
    }

    #[test]
    fn duckdb_row_set_body_is_a_plain_values_list() {
        let body = row_set_body(BackendType::DuckDB, &["id", "name"], &rows());
        assert_eq!(body, "VALUES (1, 'North'), (2, 'South')");
    }

    #[test]
    #[should_panic(expected = "requires at least one row")]
    fn row_set_body_panics_on_empty_rows() {
        row_set_body(BackendType::DuckDB, &["id"], &[]);
    }
    /// The property the live refusal was about: GoogleSQL's rendering costs
    /// a fixed number of query operands, not one per row. A 5,797-row set —
    /// the size the dogfood run actually hit — renders as a single `UNNEST`
    /// over one array literal.
    #[test]
    fn bigquery_row_set_operand_count_is_independent_of_row_count() {
        let many: Vec<Vec<String>> = (0..5_797)
            .map(|i| vec![i.to_string(), format!("'p{i}'")])
            .collect();
        let body = row_set_body(BackendType::BigQuery, &["id", "name"], &many);
        assert_eq!(
            body.matches("SELECT").count(),
            1,
            "one query operand regardless of row count"
        );
        assert_eq!(body.matches("UNNEST").count(), 1);
        assert!(!body.contains("UNION ALL"));
        // Every row is still carried.
        assert_eq!(body.matches("'p").count(), 5_797);
    }
}
