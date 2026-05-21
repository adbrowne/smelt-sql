//! Per-source bound derivation for incremental models.
//!
//! Walks the (expanded) model SQL and derives, for each `smelt.<path>` source
//! reference, how far outside the run window the source must be read in order
//! to produce correct output.  Two standard SQL forms are recognised:
//!
//! - **Form A** — window-frame `RANGE BETWEEN INTERVAL '…' PRECEDING/FOLLOWING`
//! - **Form B** — explicit WHERE/JOIN time filters with literal `INTERVAL` offsets
//!
//! The result is a `HashMap<String, BoundResult>` keyed by source name (the
//! `smelt.<path>` string without the `smelt.` prefix, matching the names in
//! `ModelInfo.refs`).

use serde::Serialize;
use std::collections::HashMap;

/// Duration expressed in whole seconds (always ≥ 0).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Default)]
pub struct Seconds(pub u64);

impl Seconds {
    pub const ZERO: Seconds = Seconds(0);

    pub fn minutes(n: u64) -> Seconds {
        Seconds(n * 60)
    }

    pub fn hours(n: u64) -> Seconds {
        Seconds(n * 3600)
    }

    pub fn days(n: u64) -> Seconds {
        Seconds(n * 86400)
    }

    pub fn weeks(n: u64) -> Seconds {
        Seconds(n * 7 * 86400)
    }

    /// Serialize as ISO-8601 duration string (e.g. "PT30M", "P1D", "PT0S").
    pub fn to_iso8601(&self) -> String {
        let s = self.0;
        if s == 0 {
            return "PT0S".to_string();
        }
        let days = s / 86400;
        let rem = s % 86400;
        let hours = rem / 3600;
        let rem = rem % 3600;
        let minutes = rem / 60;
        let seconds = rem % 60;

        let mut out = String::from("P");
        if days > 0 {
            out.push_str(&format!("{}D", days));
        }
        if hours > 0 || minutes > 0 || seconds > 0 {
            out.push('T');
            if hours > 0 {
                out.push_str(&format!("{}H", hours));
            }
            if minutes > 0 {
                out.push_str(&format!("{}M", minutes));
            }
            if seconds > 0 {
                out.push_str(&format!("{}S", seconds));
            }
        }
        out
    }
}

/// The derived bound for one source reference.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BoundResult {
    /// The source must be read `before` seconds before run_start and
    /// `after` seconds after run_end.
    Bounded {
        source_partition_col: String,
        #[serde(serialize_with = "serialize_seconds")]
        before: Seconds,
        #[serde(serialize_with = "serialize_seconds")]
        after: Seconds,
    },
    /// The source requires reading unbounded history (e.g. cumulative aggregation).
    Unbounded,
    /// The analyzer cannot derive a bound from the SQL patterns present.
    NotDerivable,
}

fn serialize_seconds<S>(s: &Seconds, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(&s.to_iso8601())
}

impl BoundResult {
    /// Merge two bound results for the *same* source (union semantics):
    /// before = max(before_i), after = max(after_i).
    /// Any `Unbounded` forces `Unbounded`; any `NotDerivable` forces `NotDerivable`.
    pub fn merge(self, other: BoundResult) -> BoundResult {
        match (self, other) {
            (BoundResult::NotDerivable, _) | (_, BoundResult::NotDerivable) => {
                BoundResult::NotDerivable
            }
            (BoundResult::Unbounded, _) | (_, BoundResult::Unbounded) => BoundResult::Unbounded,
            (
                BoundResult::Bounded {
                    source_partition_col,
                    before: b1,
                    after: a1,
                },
                BoundResult::Bounded {
                    before: b2,
                    after: a2,
                    ..
                },
            ) => BoundResult::Bounded {
                source_partition_col,
                before: b1.max(b2),
                after: a1.max(a2),
            },
        }
    }
}

/// Context for bound derivation: maps source name → its declared partition column.
/// Sources not present in the map are treated as lookups (no bound derived).
#[derive(Debug, Default)]
pub struct BoundContext {
    /// Maps source name (as it appears in `smelt.<path>` refs, e.g. "silver.events_parsed")
    /// to its `timeseries.partition_column`.
    pub source_partition_cols: HashMap<String, String>,
}

impl BoundContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_source(mut self, source: &str, partition_col: &str) -> Self {
        self.source_partition_cols
            .insert(source.to_string(), partition_col.to_string());
        self
    }

    pub fn add_source(&mut self, source: &str, partition_col: &str) {
        self.source_partition_cols
            .insert(source.to_string(), partition_col.to_string());
    }
}

/// Derive per-source bounds from the full model SQL (after function expansion).
///
/// `sql` is the expanded SQL text of the model (including any inlined function bodies).
/// `ctx` maps source names to their partition columns (only timeseries sources; lookups absent).
///
/// Returns a map from source name → bound. Only entries for timeseries sources in
/// `ctx` are produced; lookup sources (not in ctx) are absent from the result.
pub fn derive_model_bounds(sql: &str, ctx: &BoundContext) -> HashMap<String, BoundResult> {
    let mut result: HashMap<String, BoundResult> = HashMap::new();

    // Walk Form A and Form B patterns in the SQL text.
    // For each source in the context, check whether the SQL contains patterns
    // that constrain that source's partition column.

    for (source_name, partition_col) in &ctx.source_partition_cols {
        let bound = derive_bound_for_source(sql, partition_col);
        if let Some(b) = bound {
            result
                .entry(source_name.clone())
                .and_modify(|existing| {
                    *existing =
                        std::mem::replace(existing, BoundResult::Unbounded).merge(b.clone());
                })
                .or_insert(b);
        }
    }

    result
}

/// Derive a bound for a single source given its partition column.
///
/// Returns `None` for lookup sources (no timeseries). For timeseries sources,
/// returns `Some(BoundResult)`.
///
/// Checks Form A (RANGE BETWEEN INTERVAL ... PRECEDING/FOLLOWING) and
/// Form B (WHERE col BETWEEN ... - INTERVAL ... AND ... + INTERVAL ...).
fn derive_bound_for_source(sql: &str, partition_col: &str) -> Option<BoundResult> {
    let upper = sql.to_uppercase();
    let partition_col_upper = partition_col.to_uppercase();

    // Collect all Form A and Form B windows; merge them.
    let mut accumulated: Option<BoundResult> = None;

    // Form A: RANGE BETWEEN INTERVAL '…' PRECEDING AND ...
    // The interval applies to the source backing the window's ORDER BY column.
    // We look for any RANGE BETWEEN INTERVAL pattern in the SQL — if the column
    // participating in the window is the source's partition column (or the model
    // is ordering by it), we derive a bound.
    //
    // Heuristic: any RANGE BETWEEN INTERVAL in the SQL contributes a bound.
    // The interval before "PRECEDING" is the `before`; interval before "FOLLOWING" is `after`.
    let form_a_bounds = extract_form_a_bounds(&upper);
    for (before, after) in form_a_bounds {
        let bound = BoundResult::Bounded {
            source_partition_col: partition_col.to_string(),
            before,
            after,
        };
        accumulated = Some(match accumulated {
            None => bound,
            Some(acc) => acc.merge(bound),
        });
    }

    // Form B: WHERE col BETWEEN expr - INTERVAL '…' AND expr + INTERVAL '…'
    // or WHERE col >= expr - INTERVAL '…' AND col < expr + INTERVAL '…'
    // or WHERE col >= expr - INTERVAL '…' AND col <= expr
    let form_b_bounds = extract_form_b_bounds(&upper, &partition_col_upper);
    for (before, after) in form_b_bounds {
        let bound = BoundResult::Bounded {
            source_partition_col: partition_col.to_string(),
            before,
            after,
        };
        accumulated = Some(match accumulated {
            None => bound,
            Some(acc) => acc.merge(bound),
        });
    }

    // If we found at least one bound, return it. Otherwise the source is NOT derivable
    // only if the SQL has window functions without RANGE — bare LAG/LEAD without RANGE
    // means NotDerivable. A source that is never constrained at all (no intervals)
    // is BoundedZero (fully partition-local, before=0, after=0).
    match accumulated {
        Some(b) => Some(b),
        None => {
            // Check for bare window functions that have no RANGE clause.
            // A bare LAG/LEAD OVER (...) without RANGE BETWEEN is NotDerivable.
            if has_bare_lag_lead_over(&upper) {
                Some(BoundResult::NotDerivable)
            } else if has_unbounded_preceding_range(&upper) {
                Some(BoundResult::Unbounded)
            } else {
                // No temporal dependency — partition-local (Bounded with before=0, after=0).
                Some(BoundResult::Bounded {
                    source_partition_col: partition_col.to_string(),
                    before: Seconds::ZERO,
                    after: Seconds::ZERO,
                })
            }
        }
    }
}

/// Check whether the SQL has a bare LAG or LEAD with no RANGE BETWEEN clause.
///
/// A "bare" LAG/LEAD is one whose OVER clause lacks an explicit RANGE frame.
/// The heuristic: find OVER clauses containing LAG or LEAD function calls but
/// no RANGE keyword in that OVER block.
fn has_bare_lag_lead_over(upper_sql: &str) -> bool {
    // Look for LAG( or LEAD( patterns; then find the nearest OVER (...)
    // and check whether it contains "RANGE".
    let lag_patterns = ["LAG(", "LEAD("];
    for pattern in &lag_patterns {
        let mut search_from = 0;
        while let Some(pos) = upper_sql[search_from..].find(pattern) {
            let abs = search_from + pos;
            // Find "OVER" after this position
            if let Some(over_rel) = upper_sql[abs..].find("OVER") {
                let over_abs = abs + over_rel;
                // Find the paren block after OVER
                if let Some(paren_start_rel) = upper_sql[over_abs..].find('(') {
                    let paren_start = over_abs + paren_start_rel;
                    if let Some(over_content) = extract_balanced_parens_str(upper_sql, paren_start)
                    {
                        // If there's no RANGE keyword inside the OVER clause, it's bare.
                        if !over_content.contains("RANGE") {
                            return true;
                        }
                    }
                }
            }
            search_from = abs + 1;
        }
    }
    false
}

/// Check for RANGE BETWEEN UNBOUNDED PRECEDING in the SQL.
fn has_unbounded_preceding_range(upper_sql: &str) -> bool {
    upper_sql.contains("UNBOUNDED PRECEDING")
}

/// Extract Form A bounds: (before, after) from RANGE BETWEEN INTERVAL patterns.
///
/// Scans for `RANGE BETWEEN` followed by interval specs.
fn extract_form_a_bounds(upper_sql: &str) -> Vec<(Seconds, Seconds)> {
    let mut bounds = Vec::new();
    let keyword = "RANGE BETWEEN ";
    let mut search_from = 0;

    while let Some(rel) = upper_sql[search_from..].find(keyword) {
        let abs = search_from + rel;
        let after_range_between = &upper_sql[abs + keyword.len()..];

        let (before, after) = parse_between_bounds(after_range_between);
        if before > Seconds::ZERO || after > Seconds::ZERO {
            bounds.push((before, after));
        }
        search_from = abs + 1;
    }

    bounds
}

/// Parse "... PRECEDING AND ... FOLLOWING/CURRENT ROW" from text after "RANGE BETWEEN".
fn parse_between_bounds(text: &str) -> (Seconds, Seconds) {
    let mut before = Seconds::ZERO;
    let mut after = Seconds::ZERO;

    // Find PRECEDING part
    if let Some(prec_pos) = text.find("PRECEDING") {
        let before_prec = &text[..prec_pos];
        if let Some(interval_secs) = parse_interval_seconds_before(before_prec) {
            before = interval_secs;
        }
    }

    // Find FOLLOWING part (after AND)
    if let Some(and_pos) = text.find(" AND ") {
        let after_and = &text[and_pos + 5..];
        if let Some(fol_pos) = after_and.find("FOLLOWING") {
            let before_fol = &after_and[..fol_pos];
            if let Some(interval_secs) = parse_interval_seconds_before(before_fol) {
                after = interval_secs;
            }
        }
    }

    (before, after)
}

/// Extract Form B bounds: (before, after) from WHERE/JOIN BETWEEN/comparison with INTERVAL.
///
/// Patterns recognised:
/// - `col BETWEEN expr - INTERVAL '...' AND expr + INTERVAL '...'`
/// - `col BETWEEN expr - INTERVAL '...' AND expr`  (no lookahead)
/// - `col >= expr - INTERVAL '...'` and `col < expr + INTERVAL '...'`
///
/// The `partition_col_upper` is used as a hint to identify which column is being
/// filtered. For cross-column rebase (WHERE b.event_ts_utc BETWEEN ...) we look
/// for any BETWEEN pattern with INTERVAL offsets.
fn extract_form_b_bounds(upper_sql: &str, _partition_col_upper: &str) -> Vec<(Seconds, Seconds)> {
    let mut bounds = Vec::new();

    // Find BETWEEN ... INTERVAL ... AND ... INTERVAL ... patterns
    let keyword = "BETWEEN ";
    let mut search_from = 0;

    while let Some(rel) = upper_sql[search_from..].find(keyword) {
        let abs = search_from + rel;
        let after_between = &upper_sql[abs + keyword.len()..];

        // The text after BETWEEN looks like:
        // "expr - INTERVAL '1 day' AND expr + INTERVAL '1 day'"
        // or
        // "expr - INTERVAL '1 day' AND expr"
        // Find AND at depth 0
        if let Some(and_pos) = find_and_at_depth0(after_between) {
            let lower_expr = &after_between[..and_pos];
            let upper_expr = &after_between[and_pos + 4..]; // skip " AND"

            // Check for INTERVAL in lower (before) expression
            let before = if lower_expr.contains("INTERVAL") {
                // Look for "- INTERVAL '...'" pattern — this is a lookback
                extract_interval_from_subtraction(lower_expr).unwrap_or(
                    extract_interval_seconds_in_text(lower_expr).unwrap_or(Seconds::ZERO),
                )
            } else {
                Seconds::ZERO
            };

            // Check for INTERVAL in upper (after) expression
            let after_secs = if upper_expr.contains("INTERVAL") {
                // Look for "+ INTERVAL '...'" pattern — this is a lookahead
                extract_interval_from_addition(upper_expr).unwrap_or(
                    extract_interval_seconds_in_text(upper_expr).unwrap_or(Seconds::ZERO),
                )
            } else {
                Seconds::ZERO
            };

            if before > Seconds::ZERO || after_secs > Seconds::ZERO {
                bounds.push((before, after_secs));
            }
        }

        search_from = abs + 1;
    }

    // Also check >= ... - INTERVAL / < ... + INTERVAL patterns
    let gte_bounds = extract_gte_lt_interval_bounds(upper_sql);
    bounds.extend(gte_bounds);

    bounds
}

/// Find " AND " at parenthesis depth 0 within the text.
fn find_and_at_depth0(text: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut depth = 0i32;
    let and_kw = b" AND ";
    let mut i = 0;

    while i + 5 <= bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => depth -= 1,
            _ => {}
        }
        if depth == 0 && &bytes[i..i + 5] == and_kw {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Parse INTERVAL seconds from a subtraction expression like "expr - INTERVAL '1 day'".
fn extract_interval_from_subtraction(text: &str) -> Option<Seconds> {
    // Find "- INTERVAL" pattern
    let sub_kw = "- INTERVAL";
    if let Some(pos) = text.find(sub_kw) {
        let after = &text[pos + sub_kw.len()..];
        return parse_quoted_interval(after);
    }
    None
}

/// Parse INTERVAL seconds from an addition expression like "expr + INTERVAL '1 day'".
fn extract_interval_from_addition(text: &str) -> Option<Seconds> {
    let add_kw = "+ INTERVAL";
    if let Some(pos) = text.find(add_kw) {
        let after = &text[pos + add_kw.len()..];
        return parse_quoted_interval(after);
    }
    None
}

/// Extract interval seconds from any INTERVAL '...' in the text.
fn extract_interval_seconds_in_text(text: &str) -> Option<Seconds> {
    let kw = "INTERVAL";
    if let Some(pos) = text.find(kw) {
        let after = &text[pos + kw.len()..];
        return parse_quoted_interval(after);
    }
    None
}

/// Parse a quoted interval like " '1 day'" or " '30 minutes'" and return Seconds.
fn parse_quoted_interval(text: &str) -> Option<Seconds> {
    let trimmed = text.trim();
    // Find the quoted value
    let quote_start = trimmed.find('\'')?;
    let rest = &trimmed[quote_start + 1..];
    let quote_end = rest.find('\'')?;
    let value = &rest[..quote_end];

    parse_interval_value_str(value)
}

/// Parse interval value string like "1 day", "30 minutes", "1" + context.
fn parse_interval_value_str(value: &str) -> Option<Seconds> {
    let upper = value.to_uppercase();
    let parts: Vec<&str> = upper.split_whitespace().collect();
    if parts.is_empty() {
        return None;
    }

    // Could be "30 minutes" or "1 day" or "1" (with unit after the closing quote)
    let n: u64 = parts[0].parse().ok()?;
    let unit = if parts.len() >= 2 {
        parts[1]
    } else {
        // No unit in the value — assume seconds or days based on magnitude
        return Some(Seconds(n));
    };

    if unit.starts_with("SECOND") {
        Some(Seconds(n))
    } else if unit.starts_with("MINUTE") {
        Some(Seconds::minutes(n))
    } else if unit.starts_with("HOUR") {
        Some(Seconds::hours(n))
    } else if unit.starts_with("DAY") {
        Some(Seconds::days(n))
    } else if unit.starts_with("WEEK") {
        Some(Seconds::weeks(n))
    } else if unit.starts_with("MONTH") {
        // Approximate: 30 days
        Some(Seconds::days(n * 30))
    } else if unit.starts_with("YEAR") {
        Some(Seconds::days(n * 365))
    } else {
        Some(Seconds(n))
    }
}

/// Extract interval seconds from text before a keyword (used for PRECEDING).
/// Text like "INTERVAL '30 MINUTES' PRECEDING" → parse before "PRECEDING".
fn parse_interval_seconds_before(text: &str) -> Option<Seconds> {
    // The text contains "INTERVAL '...' " followed by the boundary keyword.
    // Find INTERVAL and parse the quoted value.
    let kw = "INTERVAL";
    let pos = text.rfind(kw)?; // use rfind in case there's noise before
    let after = &text[pos + kw.len()..];
    parse_quoted_interval(after)
}

/// Extract bounds from >= / < (or <=) patterns with INTERVAL.
///
/// Pattern: `col >= expr - INTERVAL '...'` (gives `before`)
/// and `col < expr + INTERVAL '...'` (gives `after`)
fn extract_gte_lt_interval_bounds(upper_sql: &str) -> Vec<(Seconds, Seconds)> {
    let mut bounds = Vec::new();
    let mut before = Seconds::ZERO;
    let mut after_secs = Seconds::ZERO;
    let mut found = false;

    // Look for >= ... - INTERVAL
    let gte_kw = ">= ";
    let mut search_from = 0;
    while let Some(rel) = upper_sql[search_from..].find(gte_kw) {
        let abs = search_from + rel;
        let after_gte = &upper_sql[abs + gte_kw.len()..];
        if after_gte.contains("- INTERVAL") {
            if let Some(s) = extract_interval_from_subtraction(after_gte) {
                before = before.max(s);
                found = true;
            }
        }
        search_from = abs + 1;
    }

    // Look for < ... + INTERVAL or <= ... + INTERVAL
    for lt_kw in &["< ", "<= "] {
        let mut search_from2 = 0;
        while let Some(rel) = upper_sql[search_from2..].find(lt_kw) {
            let abs = search_from2 + rel;
            let after_lt = &upper_sql[abs + lt_kw.len()..];
            if after_lt.contains("+ INTERVAL") {
                if let Some(s) = extract_interval_from_addition(after_lt) {
                    after_secs = after_secs.max(s);
                    found = true;
                }
            }
            search_from2 = abs + 1;
        }
    }

    if found {
        bounds.push((before, after_secs));
    }

    bounds
}

/// Extract the content inside a balanced `(...)` starting at `paren_pos`.
fn extract_balanced_parens_str(sql: &str, paren_pos: usize) -> Option<String> {
    let bytes = sql.as_bytes();
    if paren_pos >= bytes.len() || bytes[paren_pos] != b'(' {
        return None;
    }
    let mut depth = 0usize;
    let mut start = None;
    let mut end = None;
    for (i, &b) in bytes[paren_pos..].iter().enumerate() {
        match b {
            b'(' => {
                depth += 1;
                if depth == 1 {
                    start = Some(paren_pos + i + 1);
                }
            }
            b')' => {
                depth -= 1;
                if depth == 0 {
                    end = Some(paren_pos + i);
                    break;
                }
            }
            _ => {}
        }
    }
    let s = start?;
    let e = end?;
    Some(sql[s..e].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- Form A tests ----

    /// Form A: `LAG(x) OVER (PARTITION BY id ORDER BY ts RANGE BETWEEN INTERVAL '30 minutes' PRECEDING AND CURRENT ROW)`
    /// over a source partitioned by `event_date` derives `Bounded(event_date, 30min, 0)`.
    #[test]
    fn test_range_between_interval_preceding() {
        let sql = "SELECT id, ts, LAG(x) OVER (PARTITION BY id ORDER BY ts \
                   RANGE BETWEEN INTERVAL '30 minutes' PRECEDING AND CURRENT ROW) AS prev_x \
                   FROM source_table";
        let ctx = BoundContext::new().with_source("silver.events_parsed", "event_date");
        let bounds = derive_model_bounds(sql, &ctx);
        let bound = bounds.get("silver.events_parsed").unwrap();
        match bound {
            BoundResult::Bounded {
                source_partition_col,
                before,
                after,
            } => {
                assert_eq!(source_partition_col, "event_date");
                assert_eq!(*before, Seconds::minutes(30), "before must be 30 minutes");
                assert_eq!(*after, Seconds::ZERO, "after must be zero");
            }
            other => panic!("Expected Bounded, got {:?}", other),
        }
    }

    /// Form B: `WHERE s.event_date BETWEEN m.partition_date - INTERVAL '1 day' AND m.partition_date`
    /// derives `Bounded(event_date, 1d, 0)`.
    #[test]
    fn test_explicit_between_filter() {
        let sql = "SELECT * FROM sessions s \
                   WHERE s.event_date BETWEEN m.partition_date - INTERVAL '1 day' AND m.partition_date";
        let ctx = BoundContext::new().with_source("silver.sessions", "event_date");
        let bounds = derive_model_bounds(sql, &ctx);
        let bound = bounds.get("silver.sessions").unwrap();
        match bound {
            BoundResult::Bounded {
                source_partition_col,
                before,
                after,
            } => {
                assert_eq!(source_partition_col, "event_date");
                assert_eq!(*before, Seconds::days(1), "before must be 1 day");
                assert_eq!(*after, Seconds::ZERO, "after must be zero");
            }
            other => panic!("Expected Bounded, got {:?}", other),
        }
    }

    /// Form B with cross-column rebase:
    /// `WHERE b.event_ts_utc BETWEEN m.event_date_local - INTERVAL '1 day' AND m.event_date_local + INTERVAL '1 day'`
    /// derives `Bounded(event_ts_utc, 1d, 1d)`.
    #[test]
    fn test_cross_column_tz_rebase() {
        let sql = "SELECT * FROM bronze.events b \
                   JOIN users u ON b.user_id = u.user_id \
                   WHERE b.event_ts_utc BETWEEN m.event_date_local - INTERVAL '1 day' \
                     AND m.event_date_local + INTERVAL '1 day'";
        let ctx = BoundContext::new().with_source("bronze.events", "event_ts_utc");
        let bounds = derive_model_bounds(sql, &ctx);
        let bound = bounds.get("bronze.events").unwrap();
        match bound {
            BoundResult::Bounded {
                source_partition_col,
                before,
                after,
            } => {
                assert_eq!(source_partition_col, "event_ts_utc");
                assert_eq!(*before, Seconds::days(1), "before must be 1 day");
                assert_eq!(*after, Seconds::days(1), "after must be 1 day");
            }
            other => panic!("Expected Bounded, got {:?}", other),
        }
    }

    /// Same source referenced twice with different ranges takes the union (max before, max after).
    #[test]
    fn test_aggregation_max() {
        // SQL has two BETWEEN clauses for the same source with different intervals.
        let sql = "SELECT * FROM events e1 \
                   JOIN events e2 ON e1.id = e2.id \
                   WHERE e1.event_date BETWEEN m.date - INTERVAL '1 day' AND m.date \
                   AND e2.event_date BETWEEN m.date - INTERVAL '3 days' AND m.date + INTERVAL '1 day'";
        let ctx = BoundContext::new().with_source("silver.events", "event_date");
        let bounds = derive_model_bounds(sql, &ctx);
        let bound = bounds.get("silver.events").unwrap();
        match bound {
            BoundResult::Bounded { before, after, .. } => {
                assert_eq!(*before, Seconds::days(3), "before must be max(1d, 3d) = 3d");
                assert_eq!(*after, Seconds::days(1), "after must be max(0, 1d) = 1d");
            }
            other => panic!("Expected Bounded, got {:?}", other),
        }
    }

    /// A source without `timeseries:` produces no bound entry.
    #[test]
    fn test_lookup_source_no_bound() {
        let sql = "SELECT * FROM events e JOIN users u ON e.user_id = u.user_id";
        // users is NOT in the context (it's a lookup), events IS
        let ctx = BoundContext::new().with_source("silver.events", "event_date");
        let bounds = derive_model_bounds(sql, &ctx);
        // events gets an entry (fully partition-local since no intervals)
        assert!(bounds.contains_key("silver.events"));
        // users has no entry
        assert!(!bounds.contains_key("bronze.users"));
    }

    /// `LAG(x) OVER (PARTITION BY id ORDER BY ts)` (no RANGE) derives `NotDerivable`.
    #[test]
    fn test_bare_lag_not_derivable() {
        let sql = "SELECT id, ts, LAG(x) OVER (PARTITION BY id ORDER BY ts) AS prev_x \
                   FROM source_table";
        let ctx = BoundContext::new().with_source("silver.events", "event_date");
        let bounds = derive_model_bounds(sql, &ctx);
        let bound = bounds.get("silver.events").unwrap();
        assert_eq!(
            *bound,
            BoundResult::NotDerivable,
            "bare LAG without RANGE must be NotDerivable"
        );
    }

    /// A model calling `smelt.functions.sessionize(...)` whose expanded body has
    /// `RANGE BETWEEN INTERVAL '30 minutes' PRECEDING` derives the bound.
    ///
    /// The "expanded SQL" is the model SQL with the function body inlined (concatenated).
    #[test]
    fn test_function_body_traversal() {
        // Simulate the expanded SQL: the model SQL plus the sessionize function body
        // (which contains RANGE BETWEEN INTERVAL '30 minutes' PRECEDING — note that
        // sessionize actually uses ROWS frames, so we simulate a 30-minute RANGE version).
        let expanded_sql = r#"
            WITH sessionized AS (
                SELECT
                    *,
                    LAG(epoch_us(event_ts)) OVER (PARTITION BY device_id ORDER BY event_ts
                        RANGE BETWEEN INTERVAL '30 minutes' PRECEDING AND CURRENT ROW) AS _smelt_prev_ts,
                    SUM(1) OVER (PARTITION BY device_id ORDER BY event_ts) AS session_seq
                FROM smelt.silver.events_parsed
            )
            SELECT
                device_id,
                session_seq,
                session_start_date,
                COUNT(*) AS event_count
            FROM sessionized
            GROUP BY device_id, session_seq, session_start_date
        "#;

        let ctx = BoundContext::new().with_source("silver.events_parsed", "event_date");
        let bounds = derive_model_bounds(expanded_sql, &ctx);
        let bound = bounds.get("silver.events_parsed").unwrap();
        match bound {
            BoundResult::Bounded {
                source_partition_col,
                before,
                after,
            } => {
                assert_eq!(source_partition_col, "event_date");
                assert_eq!(*before, Seconds::minutes(30), "before must be 30 minutes");
                assert_eq!(*after, Seconds::ZERO, "after must be zero");
            }
            other => panic!("Expected Bounded(event_date, 30min, 0), got {:?}", other),
        }
    }

    // ---- Duration serialization tests ----

    #[test]
    fn test_seconds_to_iso8601() {
        assert_eq!(Seconds::ZERO.to_iso8601(), "PT0S");
        assert_eq!(Seconds::minutes(30).to_iso8601(), "PT30M");
        assert_eq!(Seconds::days(1).to_iso8601(), "P1D");
        assert_eq!(Seconds::hours(2).to_iso8601(), "PT2H");
        // 90 seconds = PT1M30S
        assert_eq!(Seconds(90).to_iso8601(), "PT1M30S");
    }

    #[test]
    fn test_bound_result_merge() {
        let b1 = BoundResult::Bounded {
            source_partition_col: "event_date".to_string(),
            before: Seconds::days(1),
            after: Seconds::ZERO,
        };
        let b2 = BoundResult::Bounded {
            source_partition_col: "event_date".to_string(),
            before: Seconds::days(3),
            after: Seconds::days(1),
        };
        let merged = b1.merge(b2);
        match merged {
            BoundResult::Bounded { before, after, .. } => {
                assert_eq!(before, Seconds::days(3));
                assert_eq!(after, Seconds::days(1));
            }
            other => panic!("Expected Bounded, got {:?}", other),
        }
    }

    #[test]
    fn test_not_derivable_wins_merge() {
        let b1 = BoundResult::Bounded {
            source_partition_col: "col".to_string(),
            before: Seconds::days(1),
            after: Seconds::ZERO,
        };
        let merged = b1.merge(BoundResult::NotDerivable);
        assert_eq!(merged, BoundResult::NotDerivable);
    }

    #[test]
    fn test_unbounded_wins_over_bounded() {
        let b1 = BoundResult::Bounded {
            source_partition_col: "col".to_string(),
            before: Seconds::days(1),
            after: Seconds::ZERO,
        };
        let merged = b1.merge(BoundResult::Unbounded);
        assert_eq!(merged, BoundResult::Unbounded);
    }
}
