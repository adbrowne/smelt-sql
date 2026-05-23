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

/// Bound for a single source: the partition column and the (before, after) seconds offset.
#[derive(Debug, Clone)]
pub struct SourceBound {
    /// The source's `timeseries.partition_column`.
    pub partition_col: String,
    /// Seconds to look back before `run_start`.
    pub before_secs: u64,
    /// Seconds to look forward after `run_end`.
    pub after_secs: u64,
}

/// Inject per-source pushdown filters into the SQL.
///
/// For each entry in `source_bounds` whose `before_secs` or `after_secs` is nonzero
/// (or even for zero, to be explicit), this function wraps each `smelt.<path>` reference
/// that matches the source with a subquery filter:
///
/// ```sql
/// (SELECT * FROM smelt.<path> WHERE partition_col >= 'run_start' AND partition_col < 'run_end')
/// ```
///
/// Sources without a bound entry (lookups without `timeseries:`) are left untouched.
///
/// # Arguments
/// * `sql` — The SQL with `smelt.<path>` references intact (pre-compilation, post-frontmatter-strip).
/// * `source_bounds` — Map from `smelt.<path>` key (as it appears in the SQL, e.g. `smelt.silver.events_parsed`) to its bound.
/// * `range` — The run window [start, end).
///
/// # Returns
/// SQL with each bounded source reference wrapped in a pushdown subquery.
pub fn inject_source_filters(
    sql: &str,
    source_bounds: &std::collections::HashMap<String, SourceBound>,
    range: &TimeRange,
) -> String {
    if source_bounds.is_empty() {
        return sql.to_string();
    }

    let mut result = sql.to_string();

    for (source_path, bound) in source_bounds {
        // The source_path is the full smelt ref like "smelt.silver.events_parsed"
        // or just the leaf name "events_parsed" depending on context.
        // We search for the smelt path as it appears in the SQL.
        let smelt_ref = source_path.clone();

        // Compute the pushdown window: run_start - before .. run_end + after
        let filter_start = subtract_seconds_from_date(&range.start, bound.before_secs);
        let filter_end = add_seconds_to_date(&range.end, bound.after_secs);

        let safe_col = bound.partition_col.replace('\'', "''");
        let safe_start = filter_start.replace('\'', "''");
        let safe_end = filter_end.replace('\'', "''");

        // Wrap each occurrence of `smelt_ref` in the SQL with a subquery filter.
        // We need to be careful not to match partial identifiers.
        // We replace `smelt_ref` when it appears as a standalone reference (not as
        // part of a smelt function call like `smelt.functions.*`).
        result =
            wrap_source_ref_with_filter(&result, &smelt_ref, &safe_col, &safe_start, &safe_end);
    }

    result
}

/// Subtract `secs` seconds from an ISO date string `date` (YYYY-MM-DD).
///
/// Only handles whole-day subtraction for day-granularity partitions.
/// Sub-day offsets are rounded up to the next day boundary (conservative).
fn subtract_seconds_from_date(date: &str, secs: u64) -> String {
    if secs == 0 {
        return date.to_string();
    }
    subtract_add_days(date, secs, false)
}

/// Add `secs` seconds to an ISO date string `date` (YYYY-MM-DD).
///
/// Only handles whole-day addition. Sub-day offsets are rounded up to the
/// next day boundary (conservative).
fn add_seconds_to_date(date: &str, secs: u64) -> String {
    if secs == 0 {
        return date.to_string();
    }
    subtract_add_days(date, secs, true)
}

/// Shared arithmetic: add/subtract whole days from YYYY-MM-DD.
/// `secs` is converted to days (ceiling for fractions).
fn subtract_add_days(date: &str, secs: u64, add: bool) -> String {
    let days = secs.div_ceil(86400);

    // Parse YYYY-MM-DD
    let parts: Vec<&str> = date.splitn(3, '-').collect();
    if parts.len() != 3 {
        // Not a simple date string — return as-is (timestamp with time component, etc.)
        return date.to_string();
    }
    let Ok(year) = parts[0].parse::<i64>() else {
        return date.to_string();
    };
    let Ok(month) = parts[1].parse::<i64>() else {
        return date.to_string();
    };
    let Ok(day) = parts[2].parse::<i64>() else {
        return date.to_string();
    };

    // Convert to Julian day number for arithmetic
    let jdn = ymd_to_jdn(year, month, day);
    let new_jdn = if add {
        jdn + days as i64
    } else {
        jdn - days as i64
    };
    let (ny, nm, nd) = jdn_to_ymd(new_jdn);
    format!("{:04}-{:02}-{:02}", ny, nm, nd)
}

/// Convert Gregorian date to Julian Day Number.
fn ymd_to_jdn(y: i64, m: i64, d: i64) -> i64 {
    // Algorithm from https://en.wikipedia.org/wiki/Julian_day#Julian_day_number_calculation
    let a = (14 - m) / 12;
    let yr = y + 4800 - a;
    let mn = m + 12 * a - 3;
    d + (153 * mn + 2) / 5 + 365 * yr + yr / 4 - yr / 100 + yr / 400 - 32045
}

/// Convert Julian Day Number to Gregorian date.
fn jdn_to_ymd(jdn: i64) -> (i64, i64, i64) {
    // Algorithm from https://en.wikipedia.org/wiki/Julian_day#Julian_day_number_calculation
    let l = jdn + 68569;
    let n = 4 * l / 146097;
    let l = l - (146097 * n + 3) / 4;
    let i = 4000 * (l + 1) / 1461001;
    let l = l - 1461 * i / 4 + 31;
    let j = 80 * l / 2447;
    let d = l - 2447 * j / 80;
    let l = j / 11;
    let m = j + 2 - 12 * l;
    let y = 100 * (n - 49) + i + l;
    (y, m, d)
}

/// Wrap all occurrences of `smelt_ref` in `sql` with a filter subquery.
///
/// Replaces:  `smelt_ref`
/// With:      `(SELECT * FROM smelt_ref WHERE col >= 'start' AND col < 'end')`
///
/// Only replaces when the reference appears in a FROM context (not inside
/// a smelt function call `smelt.functions.*`). This is a conservative
/// text-substitution that avoids wrapping function-namespace refs.
fn wrap_source_ref_with_filter(
    sql: &str,
    smelt_ref: &str,
    partition_col: &str,
    filter_start: &str,
    filter_end: &str,
) -> String {
    // Skip function-namespace refs — we only pushdown actual data source refs.
    // A source ref like "smelt.silver.events_parsed" is a data ref.
    // A function call like "smelt.functions.sessionize" is not.
    // The smelt_ref passed here is the source *name* (e.g. "events_parsed"),
    // or the full path (e.g. "smelt.silver.events_parsed").
    // We'll look for the full "smelt.<path>" pattern in the SQL.

    let replacement = format!(
        "(SELECT * FROM {} WHERE {} >= '{}' AND {} < '{}')",
        smelt_ref, partition_col, filter_start, partition_col, filter_end
    );

    // Replace all occurrences of `smelt_ref` that appear as standalone identifiers
    // (bounded by whitespace, '(', ')', ',', or start/end of string).
    let mut result = String::with_capacity(sql.len() + replacement.len());
    // smelt_ref is always an ASCII path like "smelt.silver.events_parsed", so
    // byte-level comparison is correct.  We must not index into `sql` as a
    // char-indexed string because `sql` may contain multi-byte UTF-8 sequences
    // (e.g. em-dashes in SQL comments) and Rust's `&str[byte..byte]` panics
    // when the indices are not on char boundaries.
    let ref_bytes = smelt_ref.as_bytes();
    let ref_len = ref_bytes.len();
    let bytes = sql.as_bytes();
    let n = bytes.len();
    let mut i = 0;

    while i < n {
        // Check if `smelt_ref` starts at position i (byte comparison is safe
        // because smelt_ref is ASCII).
        if i + ref_len <= n && &bytes[i..i + ref_len] == ref_bytes {
            // Check word boundaries using the surrounding bytes.
            let before_ok = i == 0 || !is_smelt_path_char(bytes[i - 1]);
            let after_ok = i + ref_len >= n || !is_smelt_path_char(bytes[i + ref_len]);

            if before_ok && after_ok {
                result.push_str(&replacement);
                i += ref_len;
                continue;
            }
        }
        // Advance by one complete UTF-8 character (which may be 1–4 bytes).
        // This avoids splitting multi-byte sequences when the current position
        // does not match `smelt_ref`.
        let ch = sql[i..].chars().next().expect("non-empty suffix");
        result.push(ch);
        i += ch.len_utf8();
    }

    result
}

/// Characters that can appear in a smelt path identifier segment.
fn is_smelt_path_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'.'
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

    // ─── Phase 5 TDD tests ────────────────────────────────────────────────────

    /// Given a model with a derived bound `{events_parsed: Bounded(event_date, 1d, 0)}`,
    /// the compiled SQL contains `WHERE event_date >= <start>-1d AND event_date < <end>`
    /// on the `events_parsed` FROM, *in addition* to the outer model WHERE.
    #[test]
    fn test_pushdown_emits_per_reference() {
        let sql = "SELECT device_id, session_start_date FROM smelt.silver.events_parsed";
        let range = TimeRange {
            start: "2024-01-15".into(),
            end: "2024-01-16".into(),
        };
        let mut bounds = std::collections::HashMap::new();
        bounds.insert(
            "smelt.silver.events_parsed".to_string(),
            SourceBound {
                partition_col: "event_date".to_string(),
                before_secs: 86400, // 1 day
                after_secs: 0,
            },
        );

        let result = inject_source_filters(sql, &bounds, &range);

        // Source filter: start - 1d = 2024-01-14, end unchanged = 2024-01-16
        assert!(
            result.contains("WHERE event_date >= '2024-01-14' AND event_date < '2024-01-16'"),
            "Source pushdown filter missing or wrong: {result}"
        );
        // Original smelt ref should be replaced
        assert!(
            result.contains("smelt.silver.events_parsed"),
            "smelt ref must remain inside the subquery: {result}"
        );
        // The subquery wrapper should be present
        assert!(
            result.contains("(SELECT * FROM smelt.silver.events_parsed"),
            "subquery wrapper missing: {result}"
        );
    }

    /// A lookup source (no `timeseries:`) gets no pushdown WHERE.
    #[test]
    fn test_pushdown_skips_lookups() {
        let sql = "SELECT * FROM smelt.lookup.regions";
        let range = TimeRange {
            start: "2024-01-15".into(),
            end: "2024-01-16".into(),
        };
        // Empty bounds — regions is a lookup, not in the map
        let bounds = std::collections::HashMap::new();

        let result = inject_source_filters(sql, &bounds, &range);

        // No subquery wrapping
        assert_eq!(result, sql, "Lookup must not be wrapped: {result}");
    }

    /// A model calling `smelt.functions.sessionize(...)` with `source => smelt.silver.events_parsed`;
    /// pushdown lands inside the expanded body's FROM.
    ///
    /// Per the Phase 4 Known Divergence, bound derivation runs on the outer SQL body only —
    /// patterns inside `smelt.define` function bodies are not automatically visible.
    /// Pushdown follows the same scope: the filter is injected on the `smelt.silver.events_parsed`
    /// reference wherever it appears in the SQL text, including inside named-parameter calls.
    #[test]
    fn test_pushdown_inside_function_body() {
        // The source ref appears inside a function call named parameter.
        let sql = "SELECT * FROM smelt.functions.sessionize(source => smelt.silver.events_parsed, partition_col => device_id)";
        let range = TimeRange {
            start: "2024-01-15".into(),
            end: "2024-01-16".into(),
        };
        let mut bounds = std::collections::HashMap::new();
        bounds.insert(
            "smelt.silver.events_parsed".to_string(),
            SourceBound {
                partition_col: "event_date".to_string(),
                before_secs: 86400, // 1 day
                after_secs: 0,
            },
        );

        let result = inject_source_filters(sql, &bounds, &range);

        // The events_parsed reference inside the function call is wrapped.
        // smelt.functions.sessionize must remain untouched.
        assert!(
            result.contains("smelt.functions.sessionize"),
            "Function call must not be wrapped: {result}"
        );
        assert!(
            result.contains("(SELECT * FROM smelt.silver.events_parsed"),
            "Source ref inside function call must be wrapped: {result}"
        );
        assert!(
            result.contains("WHERE event_date >= '2024-01-14'"),
            "Pushdown filter must use run_start - 1d: {result}"
        );
    }

    /// A self-join takes the union bound and emits the same widened filter on both references.
    #[test]
    fn test_pushdown_same_source_twice() {
        let sql = "SELECT a.device_id FROM smelt.silver.events_parsed a JOIN smelt.silver.events_parsed b ON a.device_id = b.device_id";
        let range = TimeRange {
            start: "2024-01-15".into(),
            end: "2024-01-16".into(),
        };
        let mut bounds = std::collections::HashMap::new();
        // Union bound: before = 1d, after = 0
        bounds.insert(
            "smelt.silver.events_parsed".to_string(),
            SourceBound {
                partition_col: "event_date".to_string(),
                before_secs: 86400, // 1 day
                after_secs: 0,
            },
        );

        let result = inject_source_filters(sql, &bounds, &range);

        // Both occurrences should be wrapped
        let count = result
            .matches("(SELECT * FROM smelt.silver.events_parsed")
            .count();
        assert_eq!(
            count, 2,
            "Both self-join references must be wrapped; got {count} wrappers in: {result}"
        );
    }

    /// Zero-offset bounds (partition-local sources): pushdown is still emitted
    /// since the spec requires it for correctness visibility, but the range equals the run window.
    #[test]
    fn test_pushdown_zero_offset_equals_run_window() {
        let sql = "SELECT event_date FROM smelt.silver.events_parsed";
        let range = TimeRange {
            start: "2024-01-15".into(),
            end: "2024-01-16".into(),
        };
        let mut bounds = std::collections::HashMap::new();
        bounds.insert(
            "smelt.silver.events_parsed".to_string(),
            SourceBound {
                partition_col: "event_date".to_string(),
                before_secs: 0,
                after_secs: 0,
            },
        );

        let result = inject_source_filters(sql, &bounds, &range);

        // With zero offsets, the filter range equals the run window
        assert!(
            result.contains("WHERE event_date >= '2024-01-15' AND event_date < '2024-01-16'"),
            "Zero-offset pushdown must match run window: {result}"
        );
    }

    /// Date arithmetic: subtract 1 day from 2024-03-01 → 2024-02-29 (leap year).
    #[test]
    fn test_date_arithmetic_leap_year() {
        let result = subtract_seconds_from_date("2024-03-01", 86400);
        assert_eq!(result, "2024-02-29");
    }

    /// Date arithmetic: add 1 day from 2023-02-28 → 2023-03-01 (non-leap year).
    #[test]
    fn test_date_arithmetic_end_of_month() {
        let result = add_seconds_to_date("2023-02-28", 86400);
        assert_eq!(result, "2023-03-01");
    }

    // ─── Existing inject_time_filter tests ───────────────────────────────────

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
