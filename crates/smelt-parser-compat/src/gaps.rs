//! Known parser gaps between smelt and PostgreSQL
//!
//! This module tracks known differences between smelt-parser and PostgreSQL's parser.
//! Each gap has a category and one or more patterns that match SQL known to trigger it.
//!
//! Gap categories:
//! - `smelt_fails`: smelt-parser fails but pg_query succeeds
//! - `pg_fails`: smelt-parser succeeds but pg_query fails (extensions)
//! - `fingerprint_mismatch`: Both parse but semantic fingerprints differ

use regex::Regex;
use std::sync::LazyLock;

/// A known parser gap with patterns to match
#[derive(Debug)]
pub struct KnownGap {
    /// Unique identifier for this gap
    pub id: &'static str,
    /// Human-readable description
    pub description: &'static str,
    /// Category: smelt_fails, pg_fails, fingerprint_mismatch
    pub category: &'static str,
    /// Regex patterns that match SQL triggering this gap
    pub patterns: &'static [&'static str],
    /// Severity: low, medium, high
    pub severity: &'static str,
    /// Whether this gap is expected to be fixed
    pub planned_fix: bool,
}

/// All known gaps
///
/// NOTE: This list documents actual parser differences discovered through testing.
/// Many PostgreSQL features are actually supported by smelt, and pg_query accepts
/// smelt-like syntax as valid PostgreSQL function calls.
pub static KNOWN_GAPS: &[KnownGap] = &[
    // ===== PostgreSQL syntax not yet supported by smelt =====
    KnownGap {
        id: "array_subscript",
        description: "Array subscript notation: arr[1], arr[1:2]",
        category: "smelt_fails",
        patterns: &[r"\w+\["],
        severity: "medium",
        planned_fix: true,
    },
    KnownGap {
        id: "json_operators",
        description: "JSON operators: ->, ->>, #>, #>>, @>, <@, ?, ?|, ?&",
        category: "smelt_fails",
        patterns: &[r"->", r"->>", r"#>", r"#>>", r"@>", r"<@"],
        severity: "medium",
        planned_fix: true,
    },
    KnownGap {
        id: "array_literal",
        description: "Array literal syntax: ARRAY[1, 2, 3]",
        category: "smelt_fails",
        patterns: &[r"(?i)\bARRAY\s*\["],
        severity: "medium",
        planned_fix: true,
    },
    KnownGap {
        id: "row_constructor",
        description: "Row constructor: ROW(1, 2, 3)",
        category: "smelt_fails",
        patterns: &[r"(?i)\bROW\s*\("],
        severity: "low",
        planned_fix: true,
    },
    KnownGap {
        id: "values_clause",
        description: "VALUES clause as standalone or in FROM",
        category: "smelt_fails",
        patterns: &[r"(?i)^\s*VALUES\s*\("],
        severity: "medium",
        planned_fix: true,
    },
    KnownGap {
        id: "pattern_match_operators",
        description: "Pattern matching operators: ~, ~*, !~, !~*",
        category: "smelt_fails",
        patterns: &[r"\s~\s", r"\s~\*\s", r"\s!~\s", r"\s!~\*\s"],
        severity: "low",
        planned_fix: true,
    },
    // string_concat_operator gap removed - || is now supported (January 2026)
    KnownGap {
        id: "any_all_some",
        description: "ANY/ALL/SOME array comparisons",
        category: "smelt_fails",
        patterns: &[
            r"(?i)=\s*ANY\s*\(",
            r"(?i)=\s*ALL\s*\(",
            r"(?i)=\s*SOME\s*\(",
        ],
        severity: "medium",
        planned_fix: true,
    },
    KnownGap {
        id: "coalesce_nullif",
        description: "COALESCE and NULLIF functions (may parse but different behavior)",
        category: "fingerprint_mismatch",
        patterns: &[r"(?i)\bCOALESCE\s*\(", r"(?i)\bNULLIF\s*\("],
        severity: "low",
        planned_fix: false,
    },
    // ===== Parser limitations =====
    // expr_in_function gap removed - expressions in function args now supported (January 2026)
    // not_equal_operator gap removed - <> is now supported (January 2026)
    // ===== Printer limitations =====
    // These are issues with smelt-parser's printer, not parsing capability
    // union_all_printing gap removed - UNION ALL printing fixed (January 2026)
    // nulls_ordering_printing gap removed - NULLS FIRST/LAST printing fixed (January 2026)
    KnownGap {
        id: "star_in_expression",
        description: "smelt accepts * in expressions/comparisons which PostgreSQL rejects",
        category: "smelt_accepts_invalid",
        patterns: &[
            r"(?i)\bCAST\s*\(\s*\*",
            r"\*\s+[+\-/]", // * followed by space then operator
            r"\*\s*[=<>!]", // * followed by comparison operator
            r"[=<>!+\-]\s*\*",
            r"\(\s*\*\s*[+\-*/]",
            r"\*\s*\*",                     // a * * - star followed by star
            r"[+\-/]\s*\*",                 // a + * or a / * - operator followed by star
            r"(?i)COALESCE\s*\(\s*\*\s*\)", // COALESCE(*) - star in non-count function
            r"(?i)NULLIF\s*\(\s*\*",        // NULLIF(*) - star in function
            r"\*\s+AS\s+",                  // * AS alias - can't alias star directly
            r"(?i)\bELSE\s+\*",             // ELSE * in CASE
            r"(?i)\bTHEN\s+\*",             // THEN * in CASE
            r"(?i)\bWHEN\s+\*",             // WHEN * in CASE
        ],
        severity: "low",
        planned_fix: true,
    },
    KnownGap {
        id: "reserved_keyword_as_identifier",
        description: "smelt accepts PostgreSQL reserved keywords as identifiers",
        category: "smelt_accepts_invalid",
        patterns: &[
            // PostgreSQL reserved words: do, to, in, end, etc.
            // Match these when used as identifiers (not part of valid SQL syntax)
            // Note: These patterns are broad - they catch places where these keywords
            // appear in identifier positions
            r"(?i)SELECT\s+do\b",
            r"(?i)SELECT\s+to\b",
            r"(?i),\s*do\b",
            r"(?i),\s*to\b",
            r"(?i)\bFROM\s+do\b",
            r"(?i)\bFROM\s+to\b",
            r"(?i)\bJOIN\s+do\b",
            r"(?i)\bJOIN\s+to\b",
            r"(?i)\bBY\s+do\b",
            r"(?i)\bBY\s+to\b",
            r"(?i)=\s*do\b",
            r"(?i)=\s*to\b",
            r"(?i)\bAS\s+do\b",
            r"(?i)\bAS\s+to\b",
            r"(?i)\bAND\s+do\b",
            r"(?i)\bAND\s+to\b",
            r"(?i)\bOR\s+do\b",
            r"(?i)\bOR\s+to\b",
        ],
        severity: "medium",
        planned_fix: true,
    },
    KnownGap {
        id: "expression_printing",
        description: "Printer may not correctly round-trip arithmetic expressions",
        category: "fingerprint_mismatch",
        patterns: &[
            r"[+\-*/]", // Any arithmetic operator may trigger printing issues
        ],
        severity: "medium",
        planned_fix: true,
    },
    // ===== smelt extensions =====
    // Note: PostgreSQL's pg_query accepts smelt.ref() and smelt.source() as valid
    // function call syntax, so they don't actually cause pg_query to fail.
    // The => operator is also valid PostgreSQL syntax for named function arguments.
    KnownGap {
        id: "trailing_comma",
        description: "Trailing commas in SELECT list and GROUP BY (DuckDB extension)",
        category: "pg_fails",
        patterns: &[
            r",\s*FROM\b",
            r",\s*WHERE\b",
            r",\s*GROUP\b",
            r",\s*HAVING\b",
        ],
        severity: "low",
        planned_fix: false, // This is intentional (DuckDB-friendly)
    },
];

/// Compiled regex patterns for efficient matching
static COMPILED_PATTERNS: LazyLock<Vec<(String, Vec<Regex>)>> = LazyLock::new(|| {
    KNOWN_GAPS
        .iter()
        .map(|gap| {
            let patterns = gap
                .patterns
                .iter()
                .filter_map(|p| Regex::new(p).ok())
                .collect();
            (gap.id.to_string(), patterns)
        })
        .collect()
});

/// Check if the given SQL matches a known gap pattern
pub fn is_known_gap(sql: &str, category: &str) -> bool {
    for gap in KNOWN_GAPS.iter() {
        if gap.category != category {
            continue;
        }

        for pattern in gap.patterns {
            if let Ok(re) = Regex::new(pattern) {
                if re.is_match(sql) {
                    return true;
                }
            }
        }
    }
    false
}

/// Get all gap IDs that match the given SQL
pub fn get_matching_gaps(sql: &str) -> Vec<&'static str> {
    let mut matches = Vec::new();

    for (id, patterns) in COMPILED_PATTERNS.iter() {
        for re in patterns {
            if re.is_match(sql) {
                // Find the gap by id to get the static str
                if let Some(gap) = KNOWN_GAPS.iter().find(|g| g.id == id) {
                    matches.push(gap.id);
                }
                break;
            }
        }
    }

    matches
}

/// Get detailed information about all gaps in a category
pub fn get_gaps_by_category(category: &str) -> Vec<&'static KnownGap> {
    KNOWN_GAPS
        .iter()
        .filter(|g| g.category == category)
        .collect()
}

/// Get gaps by severity
pub fn get_gaps_by_severity(severity: &str) -> Vec<&'static KnownGap> {
    KNOWN_GAPS
        .iter()
        .filter(|g| g.severity == severity)
        .collect()
}

/// Summary statistics about known gaps
#[derive(Debug)]
pub struct GapSummary {
    pub total: usize,
    pub smelt_fails: usize,
    pub pg_fails: usize,
    pub fingerprint_mismatch: usize,
    pub high_severity: usize,
    pub medium_severity: usize,
    pub low_severity: usize,
    pub planned_fix: usize,
}

impl GapSummary {
    pub fn compute() -> Self {
        GapSummary {
            total: KNOWN_GAPS.len(),
            smelt_fails: KNOWN_GAPS
                .iter()
                .filter(|g| g.category == "smelt_fails")
                .count(),
            pg_fails: KNOWN_GAPS
                .iter()
                .filter(|g| g.category == "pg_fails")
                .count(),
            fingerprint_mismatch: KNOWN_GAPS
                .iter()
                .filter(|g| g.category == "fingerprint_mismatch")
                .count(),
            high_severity: KNOWN_GAPS.iter().filter(|g| g.severity == "high").count(),
            medium_severity: KNOWN_GAPS.iter().filter(|g| g.severity == "medium").count(),
            low_severity: KNOWN_GAPS.iter().filter(|g| g.severity == "low").count(),
            planned_fix: KNOWN_GAPS.iter().filter(|g| g.planned_fix).count(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_known_gap_array_subscript() {
        assert!(is_known_gap("SELECT arr[1] FROM t", "smelt_fails"));
        assert!(!is_known_gap("SELECT arr FROM t", "smelt_fails"));
    }

    #[test]
    fn test_is_known_gap_json_operator() {
        assert!(is_known_gap("SELECT data->>'name' FROM t", "smelt_fails"));
        assert!(is_known_gap("SELECT data->'nested' FROM t", "smelt_fails"));
    }

    #[test]
    fn test_is_known_gap_trailing_comma() {
        // Trailing comma is a pg_fails gap (smelt accepts, pg rejects)
        assert!(is_known_gap("SELECT a, b, FROM t", "pg_fails"));
    }

    #[test]
    fn test_star_in_expression_patterns() {
        // Test that star_in_expression patterns work
        assert!(is_known_gap("SELECT * + a FROM t", "smelt_accepts_invalid"));
        assert!(is_known_gap("SELECT * / a FROM t", "smelt_accepts_invalid"));
        assert!(is_known_gap("SELECT * - a FROM t", "smelt_accepts_invalid"));
        assert!(is_known_gap("SELECT a * * FROM t", "smelt_accepts_invalid"));
        assert!(is_known_gap(
            "SELECT COUNT(a + *) AS a FROM a",
            "smelt_accepts_invalid"
        ));
        assert!(is_known_gap(
            "SELECT a FROM a WHERE * = a",
            "smelt_accepts_invalid"
        ));
        assert!(is_known_gap(
            "SELECT a FROM a WHERE a = *",
            "smelt_accepts_invalid"
        ));
    }

    #[test]
    fn test_get_matching_gaps() {
        let gaps = get_matching_gaps("SELECT arr[1], data->>'name' FROM t");
        assert!(gaps.contains(&"array_subscript"));
        assert!(gaps.contains(&"json_operators"));
    }

    #[test]
    fn test_gap_summary() {
        let summary = GapSummary::compute();
        assert!(summary.total > 0);
        assert!(summary.smelt_fails > 0);
        assert!(summary.pg_fails > 0);
    }

    #[test]
    fn test_gaps_by_category() {
        let smelt_gaps = get_gaps_by_category("smelt_fails");
        assert!(!smelt_gaps.is_empty());
        for gap in smelt_gaps {
            assert_eq!(gap.category, "smelt_fails");
        }
    }

    #[test]
    fn test_gaps_by_severity() {
        // Check that get_gaps_by_severity filters correctly
        // Note: All high-severity gaps have been fixed (January 2026)
        let high = get_gaps_by_severity("high");
        for gap in high {
            assert_eq!(gap.severity, "high");
        }

        // Medium severity gaps should still exist
        let medium = get_gaps_by_severity("medium");
        assert!(!medium.is_empty());
        for gap in medium {
            assert_eq!(gap.severity, "medium");
        }
    }
}
