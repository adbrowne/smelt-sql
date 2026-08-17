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
//! measured live against BigQuery). The portable rewrite every dialect
//! accepts is a chained `SELECT … UNION ALL SELECT …`, which is what
//! [`BackendType::BigQuery`] renders here; every other dialect keeps the
//! `VALUES` table-value constructor unchanged.
//!
//! Both functions require a non-empty row set — deciding what an *empty*
//! row set means (an always-false guard row, a `WHERE FALSE` predicate with
//! no row at all, …) is a per-caller business decision, not a row-set
//! construction detail, so callers handle the empty case themselves before
//! reaching this module.

use crate::config::BackendType;

/// The dialect-appropriate body of an inline row-set constructor: a
/// `VALUES (…), (…)` table-value constructor for dialects that support one,
/// or the portable `SELECT … UNION ALL SELECT …` rewrite GoogleSQL requires.
///
/// `columns` names the row set's columns. For the `VALUES` form the names
/// are not embedded in the body (the caller supplies them separately, e.g.
/// as a CTE's or derived table's external column list); for the
/// `UNION ALL` form they are used to alias the first branch's projections,
/// since that branch is the only one carrying column names on a `SELECT …
/// UNION ALL …` chain.
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
        BackendType::DuckDB | BackendType::Spark => {
            let rows_sql: Vec<String> =
                rows.iter().map(|r| format!("({})", r.join(", "))).collect();
            format!("VALUES {}", rows_sql.join(", "))
        }
        BackendType::BigQuery => rows
            .iter()
            .enumerate()
            .map(|(i, r)| {
                if i == 0 {
                    let projected: Vec<String> = r
                        .iter()
                        .zip(columns.iter())
                        .map(|(lit, name)| format!("{lit} AS {name}"))
                        .collect();
                    format!("SELECT {}", projected.join(", "))
                } else {
                    format!("SELECT {}", r.join(", "))
                }
            })
            .collect::<Vec<_>>()
            .join(" UNION ALL "),
    }
}

/// The full derived-table expression for an inline row set, ready to splice
/// directly into a `FROM`/`JOIN` clause: `(VALUES …) AS alias(cols)` for
/// dialects with a table-value constructor, or `(SELECT … UNION ALL
/// SELECT …) AS alias` for GoogleSQL — column names come from the first
/// branch's aliased projections in that case, so the outer alias carries no
/// column list (GoogleSQL's `AS alias(cols)` external column-list syntax on
/// a derived table is not exercised here).
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
        BackendType::DuckDB | BackendType::Spark => {
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
            body.contains("UNION ALL"),
            "expected a chained UNION ALL rewrite, got: {body}"
        );
        assert_eq!(
            body,
            "SELECT 1 AS id, 'North' AS name UNION ALL SELECT 2, 'South'"
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
            "(SELECT 1 AS id, 'North' AS name UNION ALL SELECT 2, 'South') AS t"
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
}
