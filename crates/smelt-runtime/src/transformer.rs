//! Query transformation for incremental materialization
//!
//! This module provides AST-based query transformation to inject time filters
//! for incremental materialization. It uses the smelt-parser to find the correct
//! insertion points and modifies the SQL string accordingly.

use chrono::{DateTime, Utc};
use smelt_logical::analysis::monotonicity::{classify_function_determinism, FunctionDeterminism};
use smelt_parser::{parse, File, FunctionCall};
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
        "output-clamp column '{0}' is qualified: the clamp ranges over the model's \
         output schema, where an inner-alias qualifier is out of scope — pass the \
         unqualified output column name"
    )]
    QualifiedClampColumn(String),

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

/// True when `source_bounds` describes the "transparent slice" (B0, research
/// `20260703-model-updates.md` §3.3/§3.5): exactly one source, with a
/// zero-margin bound (`before_secs == 0 && after_secs == 0`).
///
/// For this case a single per-source scan filter is both the scan-pruning
/// filter and the exact output clamp, because the source-level bound and the
/// model's own write window are, by construction, the same window (no
/// lookback/lookahead). The caller should skip the outer `inject_time_filter`
/// wrap and rely solely on `inject_source_filters` — the two filters would
/// otherwise be textually redundant (same bounds, different injection
/// points).
///
/// A model with more than one bounded source, or any nonzero margin, keeps
/// both layers: the outer clamp remains load-bearing whenever a genuine
/// lookback makes the scan window wider than the output window.
///
/// **This check alone is not sufficient for a model with a derived,
/// skewing `partition_column`** (`docs/specs/model_transforms.md` §Semantics
/// "The output window is derived, never assumed"): a skewed model's scan
/// margin and its output window are two genuinely different ranges even
/// when there is exactly one zero-margin source, because the *output*
/// window itself is wider than the *run* window the source-level filter is
/// built from. Callers must additionally require the model's own derived
/// skew (`smelt_logical::analysis::walk::model_partition_skew`) to be
/// `Skew::ZERO` before skipping the outer clamp — see
/// `crate::execute::derive_batch_filtered_sql`, the single call site that
/// composes this check with the skew gate.
pub fn is_transparent_single_source(
    source_bounds: &std::collections::HashMap<String, SourceBound>,
) -> bool {
    let mut it = source_bounds.values();
    match (it.next(), it.next()) {
        (Some(only), None) => only.before_secs == 0 && only.after_secs == 0,
        _ => false,
    }
}

/// Apply the outer output clamp: restrict the model's **output** to the
/// write window `[start, end)`.
///
/// The clamp is applied to a wrapping projection over the model's output
/// schema — `SELECT * FROM (<sql>) AS _smelt_output_clamp WHERE <col> …` —
/// never spliced into the model's own outermost `WHERE`
/// (`docs/specs/model_transforms.md` §"Source-filter pushdown + the two
/// clamps"; design fork F1/G-11). The wrap is what makes the clamp bind
/// unambiguously when several FROM items expose the clamp column's name (a
/// self-referential model, two same-named timeseries sources), and it
/// filters output *rows* — evaluated after any window function the
/// outermost `SELECT` computes, so it can never undercut a widened-scan
/// margin.
///
/// # Arguments
/// * `sql` - The original SQL query
/// * `event_time_column` - An **unqualified** column of the model's output
///   schema. A qualified (dotted) name is rejected: the wrapping
///   projection's scope has no inner aliases to qualify by.
/// * `range` - The time range (start inclusive, end exclusive)
///
/// # Example
/// ```ignore
/// let sql = "SELECT * FROM users WHERE active = true";
/// let range = TimeRange { start: "2024-01-15".into(), end: "2024-01-18".into() };
/// let result = inject_time_filter(sql, "created_at", &range)?;
/// // Result: "SELECT * FROM (\nSELECT * FROM users WHERE active = true\n) AS _smelt_output_clamp \
/// //          WHERE created_at >= '2024-01-15' AND created_at < '2024-01-18'"
/// ```
pub fn inject_time_filter(
    sql: &str,
    event_time_column: &str,
    range: &TimeRange,
) -> Result<String, TransformError> {
    // Contract: the clamp column names a column of the model's *output*
    // schema; a dotted name is an inner-alias reference, definitionally out
    // of scope in the wrapping projection.
    if event_time_column.contains('.') {
        return Err(TransformError::QualifiedClampColumn(
            event_time_column.to_string(),
        ));
    }

    // Validate the input is a SELECT with a FROM — clamping anything else
    // is meaningless and refused (unchanged from the pre-wrap contract).
    let parse_result = parse(sql);
    let file = File::cast(parse_result.syntax()).ok_or(TransformError::ParseFailed)?;
    let stmt = file.select_stmt().ok_or(TransformError::NoSelectStmt)?;
    if stmt.from_clause().is_none() {
        return Err(TransformError::NoFromClause);
    }

    // Escape single quotes (defensive)
    let safe_column = event_time_column.replace('\'', "''");
    let safe_start = range.start.replace('\'', "''");
    let safe_end = range.end.replace('\'', "''");

    Ok(format!(
        "SELECT * FROM (\n{sql}\n) AS _smelt_output_clamp \
         WHERE {safe_column} >= '{safe_start}' AND {safe_column} < '{safe_end}'"
    ))
}

/// Freeze every run-deterministic clock call (`NOW()`, `CURRENT_TIMESTAMP()`,
/// `CURRENT_DATE()`) in `sql` to a single literal derived from
/// `run_timestamp` — the compile-time pinning transform described in
/// `docs/specs/model_transforms.md` §"Compile-time pinning of run-deterministic
/// clocks".
///
/// This is what makes the non-determinism admission gate
/// (`smelt_logical::rules::incremental::check_nondeterminism`) sound: that
/// gate admits a direct SELECT-list projection of a run-deterministic
/// function even into an unlisted column, on the assumption that the value
/// is frozen once per run. Without this transform each per-chunk backfill
/// query would evaluate `NOW()` independently at execution time, producing a
/// different literal per chunk and breaking that assumption.
///
/// Only calls classified [`FunctionDeterminism::RunDeterministic`] by
/// [`classify_function_determinism`] are touched — row-nondeterministic
/// calls (`RANDOM()`, `UUID()`, ...) and ordinary function calls are left
/// exactly as written. Uses the parsed AST (not text scanning) to find each
/// call's byte range, so a substring match inside a string literal or an
/// identifier (e.g. a column named `now_flag`, or a string literal
/// containing the text `NOW()`) is never touched — only nodes the parser
/// actually recognises as a function-call expression are replaced.
///
/// `sql` that fails to parse is returned unchanged (fail-soft here; the
/// compiler's own parse step is the authoritative gate for malformed SQL —
/// this transform runs on SQL that has already round-tripped through the
/// planner).
///
/// Idempotent: since the literal is derived solely from `run_timestamp`
/// (never from the current wall clock), calling this twice with the same
/// `run_timestamp` on semantically equivalent SQL yields the same literal
/// both times.
pub fn pin_run_deterministic_clocks(sql: &str, run_timestamp: DateTime<Utc>) -> String {
    let parse_result = parse(sql);
    let Some(file) = File::cast(parse_result.syntax()) else {
        return sql.to_string();
    };

    // Use `CAST('...' AS <type>)` rather than the bare `TIMESTAMP '...'` /
    // `DATE '...'` typed-literal shorthand. The parser *does* have a
    // dedicated typed-literal production for that shorthand
    // (`smelt-parser::parser::expr::is_typed_literal`), and
    // `smelt-db/src/type_inference/literal.rs` does have an explicit
    // `TIMESTAMP '...'`/`DATE '...'`/etc. case — but that case is
    // unreachable for `TIMESTAMP '...'`/`TIMESTAMPTZ '...'` literals whose
    // string portion contains a decimal point (as our fractional-seconds
    // format `%.f` always produces): `infer_literal_type`
    // (`smelt-db/src/type_inference/literal.rs`) runs its numeric-literal
    // fast path (`infer_numeric_literal_type`) *before* the typed-literal
    // keyword checks, and that fast path treats any literal text containing
    // `.` as decimal/double *unless* it also contains `e`/`E`, in which case
    // it short-circuits straight to `DataType::Double` — and the word
    // "TIMESTAMP" (and "TIMESTAMPTZ") itself contains an `E`. So a bare
    // `TIMESTAMP '2026-07-05 12:00:00.000000'` literal is misinferred as
    // `Double` before the dedicated typed-literal case ever runs, corrupting
    // the type-conforming CAST that `SqlCompiler::apply_type_casts`
    // (`smelt-runtime/src/compile.rs`) wraps around every SELECT column
    // (verified empirically: swapping this function to emit bare typed
    // literals reproduces `Conversion Error: Unimplemented type for cast
    // (TIMESTAMP -> DOUBLE)` in the `nondeterministic_columns` e2e tests).
    // `CAST(expr AS type)` is a first-class AST node the inferencer already
    // understands (it is the mechanism `apply_type_casts` itself emits), so
    // this sidesteps the bug and stays self-consistent. The underlying
    // ordering bug in `infer_numeric_literal_type`/`infer_literal_type`
    // belongs to `smelt-db` and is tracked separately, not fixed here.
    let timestamp_literal = format!(
        "CAST('{}' AS TIMESTAMP)",
        run_timestamp.format("%Y-%m-%d %H:%M:%S%.f")
    );
    let date_literal = format!("CAST('{}' AS DATE)", run_timestamp.format("%Y-%m-%d"));

    // Collect (start, end, literal) for every run-deterministic call, then
    // splice back-to-front so earlier byte offsets stay valid after later
    // (higher-offset) replacements have already changed the string length.
    let mut ranges: Vec<(usize, usize, String)> = file
        .syntax()
        .descendants()
        .filter_map(FunctionCall::cast)
        .filter_map(|call| {
            let name = call.name()?;
            if classify_function_determinism(&name) != FunctionDeterminism::RunDeterministic {
                return None;
            }
            let literal = if name.eq_ignore_ascii_case("CURRENT_DATE") {
                date_literal.clone()
            } else {
                timestamp_literal.clone()
            };
            let range = call.syntax().text_range();
            Some((
                usize::from(range.start()),
                usize::from(range.end()),
                literal,
            ))
        })
        .collect();

    ranges.sort_by_key(|r| std::cmp::Reverse(r.0));

    let mut result = sql.to_string();
    for (start, end, literal) in ranges {
        result.replace_range(start..end, &literal);
    }
    result
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

    // ─── is_transparent_single_source (B0) ───────────────────────────────────

    /// A single source with a zero-margin bound is the transparent slice —
    /// the outer clamp is redundant.
    #[test]
    fn test_transparent_single_source_true_for_zero_margin() {
        let mut bounds = std::collections::HashMap::new();
        bounds.insert(
            "smelt.silver.events_parsed".to_string(),
            SourceBound {
                partition_col: "event_date".to_string(),
                before_secs: 0,
                after_secs: 0,
            },
        );
        assert!(is_transparent_single_source(&bounds));
    }

    /// A single source with a nonzero lookback margin keeps the outer clamp.
    #[test]
    fn test_transparent_single_source_false_for_nonzero_margin() {
        let mut bounds = std::collections::HashMap::new();
        bounds.insert(
            "smelt.silver.events_parsed".to_string(),
            SourceBound {
                partition_col: "event_date".to_string(),
                before_secs: 86400,
                after_secs: 0,
            },
        );
        assert!(!is_transparent_single_source(&bounds));
    }

    /// More than one bounded source (e.g. a join) keeps the outer clamp even
    /// when every source has a zero margin — the routing is conservative and
    /// only special-cases the single-source case.
    #[test]
    fn test_transparent_single_source_false_for_multiple_sources() {
        let mut bounds = std::collections::HashMap::new();
        bounds.insert(
            "smelt.silver.events_parsed".to_string(),
            SourceBound {
                partition_col: "event_date".to_string(),
                before_secs: 0,
                after_secs: 0,
            },
        );
        bounds.insert(
            "smelt.silver.other".to_string(),
            SourceBound {
                partition_col: "event_date".to_string(),
                before_secs: 0,
                after_secs: 0,
            },
        );
        assert!(!is_transparent_single_source(&bounds));
    }

    /// An empty bound map (no timeseries-declared sources) is not the
    /// transparent-single-source case — callers keep the outer clamp so the
    /// model's own output is still constrained to the run window.
    #[test]
    fn test_transparent_single_source_false_for_empty() {
        let bounds = std::collections::HashMap::new();
        assert!(!is_transparent_single_source(&bounds));
    }

    // ─── Two-layer widened-scan + exact output clamp invariant ───────────────

    /// Locks the "two windows differ; write window = output window" invariant
    /// at the transformer-function level (`docs/specs/model_transforms.md`
    /// §Semantics — "Source-filter pushdown + the two clamps").
    ///
    /// For a model with a genuine lookback margin, `inject_source_filters`
    /// (the scan) must read a *wider* window than the narrow run window,
    /// while `inject_time_filter` (the output clamp), when called with that
    /// SAME narrow run window, must clamp to exactly that window — never the
    /// widened scan window. This is the function-level contract the
    /// `execute.rs` batch loop must uphold: the scan margin is read but never
    /// re-written.
    #[test]
    fn test_scan_widens_but_output_clamp_stays_exact_to_run_window() {
        let run_range = TimeRange {
            start: "2024-01-15".into(),
            end: "2024-01-16".into(),
        };

        // A source with a real 1-day lookback margin (e.g. a bounded RANGE
        // INTERVAL window frame's derived bound).
        let mut bounds = std::collections::HashMap::new();
        bounds.insert(
            "smelt.silver.events_parsed".to_string(),
            SourceBound {
                partition_col: "event_date".to_string(),
                before_secs: 86400, // 1 day
                after_secs: 0,
            },
        );

        let sql = "SELECT * FROM smelt.silver.events_parsed";
        let scan_sql = inject_source_filters(sql, &bounds, &run_range);

        // The scan filter must be WIDER than the run window: start is pulled
        // back a day, end is unchanged (no lookahead).
        assert!(
            scan_sql.contains("event_date >= '2024-01-14'"),
            "scan filter must widen the start by the lookback margin: {scan_sql}"
        );
        assert!(
            scan_sql.contains("event_date < '2024-01-16'"),
            "scan filter end must be unwidened (no lookahead): {scan_sql}"
        );
        // Sanity: the scan window differs from the run window at all.
        assert!(
            !scan_sql.contains("event_date >= '2024-01-15'"),
            "scan filter must NOT equal the narrow run window's start: {scan_sql}"
        );

        // The output clamp, called with the SAME narrow run_range, must equal
        // the run window exactly — never the widened scan window.
        let clamp_sql = inject_time_filter("SELECT * FROM staged", "event_date", &run_range)
            .expect("inject_time_filter must succeed");

        assert!(
            clamp_sql.contains("event_date >= '2024-01-15' AND event_date < '2024-01-16'"),
            "output clamp must equal the exact output window [2024-01-15, 2024-01-16): {clamp_sql}"
        );
        // The clamp must NOT contain the widened scan's start date — the
        // margin is read by the scan but never re-written by the clamp.
        assert!(
            !clamp_sql.contains("2024-01-14"),
            "output clamp must not leak the widened scan's margin: {clamp_sql}"
        );
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

        // The clamp lives on a wrapping projection over the model's output,
        // never spliced into the model body (F1/G-11 subquery wrap).
        assert!(result.starts_with("SELECT * FROM ("));
        assert!(result.contains("AS _smelt_output_clamp"));
        assert!(result.contains("WHERE event_time >= '2024-01-15' AND event_time < '2024-01-18'"));
        assert!(result.contains("SELECT * FROM smelt.models.transactions"));
    }

    #[test]
    fn test_inject_filter_with_existing_where() {
        let sql = "SELECT * FROM smelt.models.transactions WHERE status = 'active'";
        let range = TimeRange {
            start: "2024-01-15".into(),
            end: "2024-01-18".into(),
        };

        let result = inject_time_filter(sql, "event_time", &range).unwrap();

        // The model's own WHERE is untouched inside the wrap; the clamp is
        // the wrapping projection's WHERE.
        assert!(result.contains("WHERE status = 'active'"));
        assert!(result.contains(
            "_smelt_output_clamp WHERE event_time >= '2024-01-15' AND event_time < '2024-01-18'"
        ));
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

        // The original WHERE stays inside the wrapped body; the clamp lands
        // on the wrapping projection, evaluated over the model's output.
        assert!(
            result.contains("WHERE transaction_timestamp IS NOT NULL"),
            "Missing original WHERE. Got: {}",
            result
        );
        assert!(result.contains(
            "_smelt_output_clamp WHERE transaction_timestamp >= '2024-01-15' \
             AND transaction_timestamp < '2024-01-18'"
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

        // Contract change with the F1 subquery wrap (deliberate, see
        // 03-design-forks.md F1): the clamp column is an UNQUALIFIED column
        // of the model's output schema — a qualified inner-alias name is
        // definitionally out of scope in the wrapping projection and is
        // rejected rather than emitted broken.
        let err = inject_time_filter(sql, "orders.created_at", &range)
            .expect_err("qualified clamp column must be rejected");
        assert!(matches!(err, TransformError::QualifiedClampColumn(_)));

        // The unqualified output column clamps the join fine.
        let result = inject_time_filter(sql, "created_at", &range).unwrap();
        assert!(result.contains(
            "_smelt_output_clamp WHERE created_at >= '2024-01-15' AND created_at < '2024-01-18'"
        ));
        // JOINs should still be there, inside the wrapped body.
        assert!(result.contains("INNER JOIN"));
    }

    // ─── pin_run_deterministic_clocks tests ────────────────────────────────

    #[test]
    fn test_pin_run_deterministic_clocks_stable_across_calls() {
        let sql = "SELECT NOW() AS inserted_at, CURRENT_DATE() AS d, RANDOM() AS r FROM events";
        let run_ts = DateTime::parse_from_rfc3339("2026-07-05T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        let first = pin_run_deterministic_clocks(sql, run_ts);
        let second = pin_run_deterministic_clocks(sql, run_ts);

        assert_eq!(
            first, second,
            "pinning must be idempotent for the same run_timestamp"
        );
        assert!(
            first.contains("CAST('2026-07-05 12:00:00"),
            "NOW() must be pinned to a literal timestamp: {first}"
        );
        assert!(
            first.contains("CAST('2026-07-05' AS DATE)"),
            "CURRENT_DATE() must be pinned to a literal date: {first}"
        );
        // RANDOM() is row-nondeterministic and must never be pinned.
        assert!(
            first.contains("RANDOM()"),
            "RANDOM() must be left untouched: {first}"
        );
        assert!(!first.contains("NOW()"), "NOW() call must be replaced");
    }

    #[test]
    fn test_pin_run_deterministic_clocks_multiple_occurrences() {
        let sql = "SELECT NOW() AS a, NOW() AS b, NOW() AS c FROM events";
        let run_ts = DateTime::parse_from_rfc3339("2026-07-05T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        let result = pin_run_deterministic_clocks(sql, run_ts);

        assert!(
            !result.contains("NOW()"),
            "every NOW() occurrence must be replaced: {result}"
        );
        let occurrences = result.matches("CAST('2026-07-05 12:00:00").count();
        assert_eq!(
            occurrences, 3,
            "all three NOW() calls must resolve to the same literal, byte offsets must not corrupt: {result}"
        );
    }

    #[test]
    fn test_pin_run_deterministic_clocks_ignores_lookalike_text() {
        // `now_flag` is a bare identifier (no call parens) and must not be
        // touched; the string literal containing "NOW()" text must not be
        // touched either — only actual FUNCTION_CALL AST nodes are pinned.
        let sql = "SELECT now_flag, 'call NOW() later' AS note, NOW() AS inserted_at FROM events";
        let run_ts = DateTime::parse_from_rfc3339("2026-07-05T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        let result = pin_run_deterministic_clocks(sql, run_ts);

        assert!(
            result.contains("now_flag"),
            "bare identifier must be untouched: {result}"
        );
        assert!(
            result.contains("'call NOW() later'"),
            "string literal must be untouched: {result}"
        );
        assert!(
            result.contains("CAST('2026-07-05 12:00:00"),
            "the real NOW() call must still be pinned: {result}"
        );
    }
}
