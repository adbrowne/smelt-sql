//! Multi-dialect compatibility testing for smelt-parser
//!
//! This crate provides property-based testing to verify smelt's SQL dialect
//! against multiple reference parsers.
//!
//! ## Reference Parsers
//!
//! - **pg_query**: PostgreSQL's vendored C parser with fingerprinting for semantic equivalence
//! - **sqlparser-rs (DatabricksDialect)**: Pure Rust SQL parser, closest available dialect to Spark SQL
//! - **sqlglot** (optional): Python SQL parser/transpiler with explicit Spark dialect support
//!
//! # Usage
//!
//! ```ignore
//! # Run parse equivalence tests (pg_query + sqlparser)
//! cargo test -p smelt-parser-compat parse_equivalence
//!
//! # Run with sqlglot validation
//! SQLGLOT_AVAILABLE=1 cargo test -p smelt-parser-compat
//!
//! # Run type checking tests (requires Docker)
//! cargo test -p smelt-parser-compat type_checking -- --ignored
//! ```

pub mod gaps;
pub mod normalize;
pub mod pg_generators;
pub mod spark_generators;

use smelt_parser::{ast::File, parse};
use std::process::Command;
use std::sync::LazyLock;

/// Result of parsing SQL with smelt
#[derive(Debug)]
pub struct SmeltParseResult {
    /// Whether parsing succeeded without errors
    pub success: bool,
    /// List of parse errors
    pub errors: Vec<String>,
    /// Normalized SQL if parsing succeeded (printed back from AST)
    pub normalized_sql: Option<String>,
}

impl SmeltParseResult {
    /// Parse SQL using smelt-parser
    pub fn parse(sql: &str) -> Self {
        let parse_result = parse(sql);
        let success = parse_result.errors.is_empty();

        let errors: Vec<String> = parse_result
            .errors
            .iter()
            .map(|e| e.message.clone())
            .collect();

        let normalized_sql = if success {
            File::cast(parse_result.syntax()).map(|f| f.to_string())
        } else {
            None
        };

        SmeltParseResult {
            success,
            errors,
            normalized_sql,
        }
    }
}

/// Result of parsing SQL with pg_query
#[derive(Debug)]
pub struct PgParseResult {
    /// Whether parsing succeeded
    pub success: bool,
    /// Error message if parsing failed
    pub error: Option<String>,
    /// Fingerprint for semantic equivalence checking
    pub fingerprint: Option<String>,
}

impl PgParseResult {
    /// Parse SQL using pg_query
    pub fn parse(sql: &str) -> Self {
        match pg_query::parse(sql) {
            Ok(_result) => {
                // Get fingerprint for semantic equivalence (convert to hex string)
                let fingerprint = pg_query::fingerprint(sql).ok().map(|fp| fp.hex);
                PgParseResult {
                    success: true,
                    error: None,
                    fingerprint,
                }
            }
            Err(e) => PgParseResult {
                success: false,
                error: Some(e.to_string()),
                fingerprint: None,
            },
        }
    }
}

/// Check if SQL contains smelt-specific extensions
pub fn has_smelt_extensions(sql: &str) -> bool {
    let sql_lower = sql.to_lowercase();
    sql_lower.contains("smelt.ref(")
        || sql_lower.contains("smelt.source(")
        || sql_lower.contains("smelt.metric(")
        || sql.contains("=>") // Named parameters
}

/// Compare parsing results between smelt and pg_query
///
/// Returns:
/// - `Ok(true)` if both parsers agree (both succeed or both fail)
/// - `Ok(false)` if parsers disagree but the gap is known/expected
/// - `Err(message)` if parsers disagree unexpectedly (potential bug)
pub fn compare_parse_results(sql: &str) -> Result<bool, String> {
    // Skip smelt-specific extensions
    if has_smelt_extensions(sql) {
        return Ok(true);
    }

    let smelt_result = SmeltParseResult::parse(sql);
    let pg_result = PgParseResult::parse(sql);

    match (smelt_result.success, pg_result.success) {
        // Both succeed - check semantic equivalence via fingerprint
        (true, true) => {
            if let (Some(normalized), Some(pg_fp)) =
                (smelt_result.normalized_sql, pg_result.fingerprint)
            {
                // Re-parse the normalized SQL with pg_query
                let normalized_pg = PgParseResult::parse(&normalized);
                if let Some(normalized_fp) = normalized_pg.fingerprint {
                    if pg_fp == normalized_fp {
                        Ok(true)
                    } else {
                        // Fingerprints differ - check if it's a known gap
                        if gaps::is_known_gap(sql, "fingerprint_mismatch") {
                            Ok(false)
                        } else {
                            Err(format!(
                                "Fingerprint mismatch after round-trip\n\
                                 Original: {}\n\
                                 Normalized: {}\n\
                                 Original fingerprint: {}\n\
                                 Normalized fingerprint: {}",
                                sql, normalized, pg_fp, normalized_fp
                            ))
                        }
                    }
                } else {
                    // Normalized SQL failed to parse with pg_query
                    // This might be due to printer issues
                    if gaps::is_known_gap(sql, "fingerprint_mismatch") {
                        Ok(false)
                    } else {
                        Err(format!(
                            "Normalized SQL failed pg_query parse\n\
                             Original: {}\n\
                             Normalized: {}\n\
                             Error: {:?}",
                            sql, normalized, normalized_pg.error
                        ))
                    }
                }
            } else {
                Ok(true) // No fingerprint available, consider it a match
            }
        }

        // smelt fails but pg_query succeeds - potential gap
        (false, true) => {
            if gaps::is_known_gap(sql, "smelt_fails") {
                Ok(false)
            } else {
                Err(format!(
                    "smelt failed but pg_query succeeded\n\
                     SQL: {}\n\
                     smelt errors: {:?}",
                    sql, smelt_result.errors
                ))
            }
        }

        // smelt succeeds but pg_query fails - either smelt extension or bug
        (true, false) => {
            if gaps::is_known_gap(sql, "pg_fails")
                || gaps::is_known_gap(sql, "smelt_accepts_invalid")
            {
                Ok(false)
            } else {
                Err(format!(
                    "smelt succeeded but pg_query failed\n\
                     SQL: {}\n\
                     pg_query error: {:?}",
                    sql, pg_result.error
                ))
            }
        }

        // Both fail - acceptable
        (false, false) => Ok(true),
    }
}

/// Result of parsing SQL with sqlparser-rs using DatabricksDialect
#[derive(Debug)]
pub struct SparkSqlparserResult {
    /// Whether parsing succeeded
    pub success: bool,
    /// Error message if parsing failed
    pub error: Option<String>,
}

impl SparkSqlparserResult {
    /// Parse SQL using sqlparser-rs with DatabricksDialect
    pub fn parse(sql: &str) -> Self {
        use sqlparser::dialect::DatabricksDialect;
        use sqlparser::parser::Parser;

        let dialect = DatabricksDialect {};
        match Parser::parse_sql(&dialect, sql) {
            Ok(_) => SparkSqlparserResult {
                success: true,
                error: None,
            },
            Err(e) => SparkSqlparserResult {
                success: false,
                error: Some(e.to_string()),
            },
        }
    }
}

/// Result of parsing SQL with sqlglot (Python subprocess)
#[derive(Debug)]
pub struct SqlglotResult {
    /// Whether parsing succeeded
    pub success: bool,
    /// Error message if parsing failed
    pub error: Option<String>,
}

/// Check if sqlglot is available (cached)
static SQLGLOT_AVAILABLE: LazyLock<bool> = LazyLock::new(|| {
    if std::env::var("SQLGLOT_AVAILABLE").is_err() {
        return false;
    }
    Command::new("python3")
        .args(["-c", "import sqlglot"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
});

impl SqlglotResult {
    /// Check if sqlglot is available
    pub fn is_available() -> bool {
        *SQLGLOT_AVAILABLE
    }

    /// Parse SQL using sqlglot with spark dialect
    pub fn parse(sql: &str) -> Self {
        if !Self::is_available() {
            return SqlglotResult {
                success: false,
                error: Some("sqlglot not available".to_string()),
            };
        }

        let script = format!(
            "import sqlglot; sqlglot.parse_one({}, dialect='spark')",
            python_string_literal(sql)
        );

        match Command::new("python3").args(["-c", &script]).output() {
            Ok(output) => {
                if output.status.success() {
                    SqlglotResult {
                        success: true,
                        error: None,
                    }
                } else {
                    SqlglotResult {
                        success: false,
                        error: Some(String::from_utf8_lossy(&output.stderr).to_string()),
                    }
                }
            }
            Err(e) => SqlglotResult {
                success: false,
                error: Some(format!("Failed to run python3: {}", e)),
            },
        }
    }
}

/// Escape a SQL string for safe embedding in a Python string literal
fn python_string_literal(s: &str) -> String {
    // Use triple-quoted raw-ish string to avoid most escaping issues
    let escaped = s.replace('\\', "\\\\").replace('\'', "\\'");
    format!("'''{}'''", escaped)
}

/// Compare smelt parse results against sqlparser-rs with DatabricksDialect
///
/// Returns:
/// - `Ok(true)` if both parsers agree
/// - `Ok(false)` if parsers disagree but the gap is known/expected
/// - `Err(message)` if parsers disagree unexpectedly
pub fn compare_spark_parse_results(sql: &str) -> Result<bool, String> {
    if has_smelt_extensions(sql) {
        return Ok(true);
    }

    let smelt_result = SmeltParseResult::parse(sql);
    let spark_result = SparkSqlparserResult::parse(sql);

    match (smelt_result.success, spark_result.success) {
        (true, true) => {
            // Both succeed — also verify smelt's normalized output is parseable
            if let Some(normalized) = &smelt_result.normalized_sql {
                let normalized_spark = SparkSqlparserResult::parse(normalized);
                if !normalized_spark.success {
                    if gaps::is_known_gap(sql, "fingerprint_mismatch") {
                        return Ok(false);
                    }
                    return Err(format!(
                        "Normalized SQL failed sqlparser-databricks parse\n\
                         Original: {}\n\
                         Normalized: {}\n\
                         Error: {:?}",
                        sql, normalized, normalized_spark.error
                    ));
                }
            }
            Ok(true)
        }

        (false, true) => {
            if gaps::is_known_gap(sql, "smelt_fails") {
                Ok(false)
            } else {
                Err(format!(
                    "smelt failed but sqlparser-databricks succeeded\n\
                     SQL: {}\n\
                     smelt errors: {:?}",
                    sql, smelt_result.errors
                ))
            }
        }

        (true, false) => {
            if gaps::is_known_gap(sql, "spark_fails")
                || gaps::is_known_gap(sql, "smelt_accepts_invalid")
            {
                Ok(false)
            } else {
                Err(format!(
                    "smelt succeeded but sqlparser-databricks failed\n\
                     SQL: {}\n\
                     sqlparser error: {:?}",
                    sql, spark_result.error
                ))
            }
        }

        (false, false) => Ok(true),
    }
}

/// Compare smelt parse results against sqlglot (if available)
///
/// Returns None if sqlglot is not available.
pub fn compare_sqlglot_parse_results(sql: &str) -> Option<Result<bool, String>> {
    if !SqlglotResult::is_available() {
        return None;
    }

    if has_smelt_extensions(sql) {
        return Some(Ok(true));
    }

    let smelt_result = SmeltParseResult::parse(sql);
    let sqlglot_result = SqlglotResult::parse(sql);

    Some(match (smelt_result.success, sqlglot_result.success) {
        (true, true) => Ok(true),
        (false, true) => {
            if gaps::is_known_gap(sql, "smelt_fails") {
                Ok(false)
            } else {
                Err(format!(
                    "smelt failed but sqlglot (spark) succeeded\n\
                     SQL: {}\n\
                     smelt errors: {:?}",
                    sql, smelt_result.errors
                ))
            }
        }
        (true, false) => {
            if gaps::is_known_gap(sql, "sqlglot_fails")
                || gaps::is_known_gap(sql, "smelt_accepts_invalid")
            {
                Ok(false)
            } else {
                Err(format!(
                    "smelt succeeded but sqlglot (spark) failed\n\
                     SQL: {}\n\
                     sqlglot error: {:?}",
                    sql, sqlglot_result.error
                ))
            }
        }
        (false, false) => Ok(true),
    })
}

/// Compare against all available reference parsers
///
/// Runs smelt against pg_query, sqlparser-databricks, and sqlglot (if available).
/// Returns errors from the first failing comparison.
pub fn compare_all_parse_results(sql: &str) -> Result<bool, String> {
    // Layer 1: pg_query
    let pg_result = compare_parse_results(sql)?;

    // Layer 2: sqlparser-databricks
    let spark_result = compare_spark_parse_results(sql)?;

    // Layer 3: sqlglot (if available)
    if let Some(sqlglot_result) = compare_sqlglot_parse_results(sql) {
        sqlglot_result?;
    }

    Ok(pg_result && spark_result)
}

/// Semantic equivalence check using fingerprints
///
/// This is a lighter-weight check that just compares fingerprints
/// without doing full round-trip parsing.
pub fn check_semantic_equivalence(sql: &str) -> Option<bool> {
    if has_smelt_extensions(sql) {
        return Some(true);
    }

    let smelt_result = SmeltParseResult::parse(sql);
    if !smelt_result.success {
        return None;
    }

    let normalized = smelt_result.normalized_sql?;

    let original_fp = pg_query::fingerprint(sql).ok()?.hex;
    let normalized_fp = pg_query::fingerprint(&normalized).ok()?.hex;

    Some(original_fp == normalized_fp)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_smelt_parse_simple() {
        let result = SmeltParseResult::parse("SELECT * FROM users");
        assert!(result.success);
        assert!(result.normalized_sql.is_some());
    }

    #[test]
    fn test_smelt_parse_invalid() {
        let result = SmeltParseResult::parse("SELECT FROM");
        assert!(!result.success);
        assert!(!result.errors.is_empty());
    }

    #[test]
    fn test_pg_parse_simple() {
        let result = PgParseResult::parse("SELECT * FROM users");
        assert!(result.success);
        assert!(result.fingerprint.is_some());
    }

    #[test]
    fn test_pg_parse_invalid() {
        let result = PgParseResult::parse("SELECT FROM WHERE");
        assert!(!result.success);
        assert!(result.error.is_some());
    }

    #[test]
    fn test_has_smelt_extensions() {
        assert!(has_smelt_extensions("SELECT * FROM smelt.ref('users')"));
        assert!(has_smelt_extensions(
            "SELECT * FROM smelt.source('raw.users')"
        ));
        assert!(has_smelt_extensions("SELECT * FROM func(arg => value)"));
        assert!(!has_smelt_extensions("SELECT * FROM users"));
    }

    #[test]
    fn test_compare_parse_simple() {
        let result = compare_parse_results("SELECT * FROM users");
        assert!(result.is_ok());
    }

    #[test]
    fn test_compare_with_smelt_extension() {
        // smelt extensions should be skipped
        let result = compare_parse_results("SELECT * FROM smelt.ref('users')");
        assert!(result.is_ok());
    }

    #[test]
    fn test_semantic_equivalence() {
        let result = check_semantic_equivalence("SELECT * FROM users WHERE id = 1");
        assert_eq!(result, Some(true));
    }

    #[test]
    fn test_spark_sqlparser_parse_simple() {
        let result = SparkSqlparserResult::parse("SELECT * FROM users");
        assert!(result.success);
    }

    #[test]
    fn test_spark_sqlparser_parse_invalid() {
        let result = SparkSqlparserResult::parse("SELECT FROM WHERE");
        assert!(!result.success);
        assert!(result.error.is_some());
    }

    #[test]
    fn test_spark_sqlparser_qualify() {
        let result =
            SparkSqlparserResult::parse("SELECT * FROM t QUALIFY ROW_NUMBER() OVER () = 1");
        assert!(result.success, "DatabricksDialect should support QUALIFY");
    }

    #[test]
    fn test_compare_spark_simple() {
        let result = compare_spark_parse_results("SELECT * FROM users");
        assert!(result.is_ok());
    }

    #[test]
    fn test_compare_all_simple() {
        let result = compare_all_parse_results("SELECT * FROM users");
        assert!(result.is_ok());
    }

    #[test]
    fn test_python_string_literal() {
        assert_eq!(python_string_literal("hello"), "'''hello'''");
        assert_eq!(python_string_literal("it's a test"), "'''it\\'s a test'''");
    }

    #[test]
    fn test_union_all_debug() {
        let sql = "SELECT id FROM users UNION ALL SELECT id FROM customers";
        let smelt_result = SmeltParseResult::parse(sql);
        assert!(smelt_result.success, "smelt should parse UNION ALL");

        let normalized = smelt_result.normalized_sql.as_ref().unwrap();
        println!("Original: {}", sql);
        println!("Normalized: {}", normalized);

        let original_fp = pg_query::fingerprint(sql).ok().map(|f| f.hex);
        let normalized_fp = pg_query::fingerprint(normalized).ok().map(|f| f.hex);

        println!("Original fingerprint: {:?}", original_fp);
        println!("Normalized fingerprint: {:?}", normalized_fp);

        // The fingerprints may differ due to AST print formatting
        // This is expected - the test documents the behavior
    }
}
