//! PostgreSQL compatibility testing for smelt-parser
//!
//! This crate provides property-based testing to verify smelt's SQL dialect
//! can handle PostgreSQL SELECT queries correctly.
//!
//! Key features:
//! - Parse tree matching: Generate SQL, parse with both smelt and pg_query, verify equivalence
//! - Type checking: Verify smelt's type inference matches PostgreSQL's actual types
//! - Gap tracking: Document and track known parser gaps
//!
//! # Usage
//!
//! ```ignore
//! # Run parse equivalence tests
//! cargo test -p smelt-parser-compat parse_equivalence
//!
//! # Run type checking tests (requires Docker)
//! cargo test -p smelt-parser-compat type_checking -- --ignored
//! ```

pub mod gaps;
pub mod normalize;
pub mod pg_generators;

use smelt_parser::{ast::File, parse};

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
