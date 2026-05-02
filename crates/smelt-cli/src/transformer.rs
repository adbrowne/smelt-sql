//! Query transformation for incremental materialization
//!
//! This module provides AST-based query transformation to inject time filters
//! for incremental materialization. It uses the smelt-parser to find the correct
//! insertion points and modifies the SQL string accordingly.

use smelt_parser::{parse, File};
use thiserror::Error;

/// Time range for filtering (inclusive start, exclusive end)
#[derive(Debug, Clone, serde::Serialize)]
pub struct TimeRange {
    pub start: String, // ISO 8601 date: YYYY-MM-DD
    pub end: String,   // ISO 8601 date: YYYY-MM-DD (exclusive)
}

/// Errors that can occur during query transformation
#[derive(Debug, Error)]
pub enum TransformError {
    #[error("Failed to parse SQL: query is malformed")]
    ParseFailed,

    #[error("No SELECT statement found in query")]
    NoSelectStmt,

    #[error("No FROM clause found - cannot inject time filter")]
    NoFromClause,

    #[error(
        "Query contains subqueries which are not yet supported for incremental transformation"
    )]
    SubqueryNotSupported,
}

/// Transform a SQL query to filter by event time range.
///
/// This function injects a WHERE clause filter to restrict the query
/// to only process data within the specified time range.
///
/// # Arguments
/// * `sql` - The original SQL query
/// * `event_time_column` - The column name to filter on
/// * `range` - The time range (start inclusive, end exclusive)
///
/// # Returns
/// The transformed SQL with the time filter injected, or an error if
/// the transformation cannot be safely applied.
///
/// # Example
/// ```ignore
/// let sql = "SELECT * FROM users WHERE active = true";
/// let range = TimeRange { start: "2024-01-15".into(), end: "2024-01-18".into() };
/// let result = inject_time_filter(sql, "created_at", &range)?;
/// // Result: "SELECT * FROM users WHERE active = true AND (created_at >= '2024-01-15' AND created_at < '2024-01-18')"
/// ```
pub fn inject_time_filter(
    sql: &str,
    event_time_column: &str,
    range: &TimeRange,
) -> Result<String, TransformError> {
    // Parse the SQL to get AST
    let parse_result = parse(sql);
    let file = File::cast(parse_result.syntax()).ok_or(TransformError::ParseFailed)?;
    let stmt = file.select_stmt().ok_or(TransformError::NoSelectStmt)?;

    // Build the filter expression
    // Escape single quotes in the column name (defensive)
    let safe_column = event_time_column.replace('\'', "''");
    let safe_start = range.start.replace('\'', "''");
    let safe_end = range.end.replace('\'', "''");

    let filter = format!(
        "{} >= '{}' AND {} < '{}'",
        safe_column, safe_start, safe_column, safe_end
    );

    // Determine where to inject the filter
    if let Some(where_clause) = stmt.where_clause() {
        // Append to existing WHERE clause
        let where_end = usize::from(where_clause.text_range().end());

        // Split the SQL at the end of the WHERE clause
        let (before, after) = sql.split_at(where_end);

        Ok(format!("{} AND ({}){}", before, filter, after))
    } else if let Some(from_clause) = stmt.from_clause() {
        // Insert new WHERE clause after FROM
        let from_end = usize::from(from_clause.text_range().end());

        // Split the SQL at the end of the FROM clause
        let (before, after) = sql.split_at(from_end);

        Ok(format!("{} WHERE {}{}", before, filter, after))
    } else {
        Err(TransformError::NoFromClause)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_inject_filter_no_where_clause() {
        let sql = "SELECT * FROM smelt.models.transactions";
        let range = TimeRange {
            start: "2024-01-15".into(),
            end: "2024-01-18".into(),
        };

        let result = inject_time_filter(sql, "event_time", &range).unwrap();

        assert!(result.contains("WHERE event_time >= '2024-01-15' AND event_time < '2024-01-18'"));
        assert!(result.starts_with("SELECT * FROM smelt.models.transactions"));
    }

    #[test]
    fn test_inject_filter_with_existing_where() {
        let sql = "SELECT * FROM smelt.models.transactions WHERE status = 'active'";
        let range = TimeRange {
            start: "2024-01-15".into(),
            end: "2024-01-18".into(),
        };

        let result = inject_time_filter(sql, "event_time", &range).unwrap();

        assert!(result.contains("WHERE status = 'active'"));
        assert!(result.contains("AND (event_time >= '2024-01-15' AND event_time < '2024-01-18')"));
    }

    #[test]
    fn test_inject_filter_with_group_by() {
        let sql = r#"
SELECT
    DATE(transaction_timestamp) as revenue_date,
    user_id,
    SUM(amount) as total_revenue
FROM smelt.models.transactions
WHERE transaction_timestamp IS NOT NULL
GROUP BY 1, 2
"#;
        let range = TimeRange {
            start: "2024-01-15".into(),
            end: "2024-01-18".into(),
        };

        let result = inject_time_filter(sql, "transaction_timestamp", &range).unwrap();

        // Should have both the original WHERE and the new filter
        assert!(
            result.contains("WHERE transaction_timestamp IS NOT NULL"),
            "Missing original WHERE. Got: {}",
            result
        );
        assert!(result.contains(
            "AND (transaction_timestamp >= '2024-01-15' AND transaction_timestamp < '2024-01-18')"
        ));
        // GROUP BY should still be there
        assert!(result.contains("GROUP BY 1, 2"));
    }

    #[test]
    fn test_no_from_clause_error() {
        let sql = "SELECT 1 + 1";
        let range = TimeRange {
            start: "2024-01-15".into(),
            end: "2024-01-18".into(),
        };

        let result = inject_time_filter(sql, "event_time", &range);
        assert!(matches!(result, Err(TransformError::NoFromClause)));
    }

    #[test]
    fn test_with_join() {
        let sql = "SELECT * FROM smelt.models.orders INNER JOIN smelt.models.users ON orders.user_id = users.id";
        let range = TimeRange {
            start: "2024-01-15".into(),
            end: "2024-01-18".into(),
        };

        let result = inject_time_filter(sql, "orders.created_at", &range).unwrap();

        // Should have WHERE clause injected
        assert!(result.contains(
            "WHERE orders.created_at >= '2024-01-15' AND orders.created_at < '2024-01-18'"
        ));
        // JOINs should still be there
        assert!(result.contains("INNER JOIN"));
    }
}
