//! Parse tree normalization utilities
//!
//! This module provides utilities for normalizing SQL for comparison purposes.
//! It helps account for semantically equivalent but syntactically different SQL.

use regex::Regex;
use std::sync::LazyLock;

/// Normalize whitespace in SQL
pub fn normalize_whitespace(sql: &str) -> String {
    static WS_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());
    WS_RE.replace_all(sql.trim(), " ").to_string()
}

/// Normalize keywords to uppercase
pub fn normalize_keywords(sql: &str) -> String {
    static KEYWORDS: &[&str] = &[
        "SELECT",
        "FROM",
        "WHERE",
        "AND",
        "OR",
        "NOT",
        "AS",
        "ON",
        "JOIN",
        "INNER",
        "LEFT",
        "RIGHT",
        "FULL",
        "CROSS",
        "OUTER",
        "GROUP",
        "BY",
        "HAVING",
        "ORDER",
        "LIMIT",
        "OFFSET",
        "UNION",
        "ALL",
        "DISTINCT",
        "CASE",
        "WHEN",
        "THEN",
        "ELSE",
        "END",
        "CAST",
        "IN",
        "BETWEEN",
        "IS",
        "NULL",
        "TRUE",
        "FALSE",
        "ASC",
        "DESC",
        "NULLS",
        "FIRST",
        "LAST",
        "WITH",
        "RECURSIVE",
        "OVER",
        "PARTITION",
        "ROWS",
        "RANGE",
        "GROUPS",
        "UNBOUNDED",
        "PRECEDING",
        "FOLLOWING",
        "CURRENT",
        "ROW",
        "FILTER",
        "EXISTS",
        "USING",
        "LATERAL",
        "TABLESAMPLE",
        "BERNOULLI",
        "SYSTEM",
        "REPEATABLE",
        // Spark-specific keywords
        "QUALIFY",
        "PIVOT",
        "UNPIVOT",
        "TRANSFORM",
        "AGGREGATE",
    ];

    let mut result = sql.to_string();
    for kw in KEYWORDS {
        let re = Regex::new(&format!(r"(?i)\b{}\b", kw)).unwrap();
        result = re.replace_all(&result, *kw).to_string();
    }
    result
}

/// Normalize SQL for comparison
///
/// Applies:
/// - Whitespace normalization
/// - Keyword uppercasing
pub fn normalize_sql(sql: &str) -> String {
    let sql = normalize_whitespace(sql);
    normalize_keywords(&sql)
}

/// Check if two SQL statements are syntactically equivalent
/// after normalization
pub fn are_syntactically_equivalent(sql1: &str, sql2: &str) -> bool {
    normalize_sql(sql1) == normalize_sql(sql2)
}

/// Extract simple structure from SQL for debugging
#[derive(Debug, Clone)]
pub struct SqlStructure {
    pub has_select: bool,
    pub has_from: bool,
    pub has_where: bool,
    pub has_group_by: bool,
    pub has_having: bool,
    pub has_order_by: bool,
    pub has_limit: bool,
    pub has_with: bool,
    pub has_union: bool,
    pub join_count: usize,
}

impl SqlStructure {
    /// Extract structure from SQL string
    pub fn extract(sql: &str) -> Self {
        let sql_upper = sql.to_uppercase();

        // Count JOINs (all types)
        let join_re = Regex::new(r"(?i)\bJOIN\b").unwrap();
        let join_count = join_re.find_iter(sql).count();

        SqlStructure {
            has_select: sql_upper.contains("SELECT"),
            has_from: sql_upper.contains("FROM"),
            has_where: sql_upper.contains("WHERE"),
            has_group_by: sql_upper.contains("GROUP BY"),
            has_having: sql_upper.contains("HAVING"),
            has_order_by: sql_upper.contains("ORDER BY"),
            has_limit: sql_upper.contains("LIMIT"),
            has_with: sql_upper.contains("WITH"),
            has_union: sql_upper.contains("UNION"),
            join_count,
        }
    }

    /// Check if structures are compatible
    pub fn is_compatible(&self, other: &SqlStructure) -> bool {
        self.has_select == other.has_select
            && self.has_from == other.has_from
            && self.has_where == other.has_where
            && self.has_group_by == other.has_group_by
            && self.has_having == other.has_having
            && self.has_order_by == other.has_order_by
            && self.has_limit == other.has_limit
            && self.has_with == other.has_with
            && self.has_union == other.has_union
            && self.join_count == other.join_count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_whitespace() {
        assert_eq!(
            normalize_whitespace("SELECT  *  FROM   users"),
            "SELECT * FROM users"
        );
        assert_eq!(
            normalize_whitespace("  SELECT\n*\t FROM\r\nusers  "),
            "SELECT * FROM users"
        );
    }

    #[test]
    fn test_normalize_keywords() {
        assert_eq!(
            normalize_keywords("select * from users where id = 1"),
            "SELECT * FROM users WHERE id = 1"
        );
        assert_eq!(
            normalize_keywords("SeLeCt * FrOm users"),
            "SELECT * FROM users"
        );
    }

    #[test]
    fn test_are_syntactically_equivalent() {
        assert!(are_syntactically_equivalent(
            "SELECT * FROM users",
            "select  *  from  users"
        ));
        assert!(are_syntactically_equivalent(
            "SELECT name AS n FROM t",
            "select name as n from t"
        ));
        assert!(!are_syntactically_equivalent(
            "SELECT * FROM users",
            "SELECT * FROM orders"
        ));
    }

    #[test]
    fn test_sql_structure_extract() {
        let structure =
            SqlStructure::extract("SELECT * FROM users WHERE id = 1 ORDER BY name LIMIT 10");
        assert!(structure.has_select);
        assert!(structure.has_from);
        assert!(structure.has_where);
        assert!(!structure.has_group_by);
        assert!(structure.has_order_by);
        assert!(structure.has_limit);
    }

    #[test]
    fn test_sql_structure_joins() {
        let structure = SqlStructure::extract(
            "SELECT * FROM a INNER JOIN b ON a.id = b.id LEFT JOIN c ON b.id = c.id",
        );
        assert_eq!(structure.join_count, 2);
    }

    #[test]
    fn test_sql_structure_compatible() {
        let s1 = SqlStructure::extract("SELECT * FROM users WHERE id = 1");
        let s2 = SqlStructure::extract("select name from customers where active = true");
        assert!(s1.is_compatible(&s2));

        let s3 = SqlStructure::extract("SELECT * FROM users ORDER BY name");
        assert!(!s1.is_compatible(&s3)); // s3 has ORDER BY, s1 doesn't
    }
}
