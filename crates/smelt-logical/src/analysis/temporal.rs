//! Temporal dependency analysis for incremental models.
//!
//! Examines the query AST to infer how much historical context (lookback)
//! or future context (lookahead) each query needs beyond the requested
//! time range. This is combined with upstream data latency to compute
//! the effective window for incremental execution.

use serde::Serialize;
use smelt_parser::{parse, File, FrameUnit, SelectStmt};

use crate::analysis::source_bounds;

/// How much temporal context a query needs beyond its requested time range.
#[derive(Debug, Clone, Default, Serialize)]
pub struct TemporalDependency {
    /// How many periods/days of prior data the query needs.
    pub lookback: TemporalOffset,
    /// How many periods/days of future data the query needs (rare — LEAD, lookahead joins).
    pub lookahead: TemporalOffset,
    /// Where the dependencies were detected (for explain output).
    pub sources: Vec<TemporalSource>,
}

/// A temporal offset that can be expressed in periods (rows) or a fixed duration.
#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub enum TemporalOffset {
    /// Zero — no temporal dependency in this direction.
    #[default]
    Zero,
    /// N periods (row-based: ROWS BETWEEN N PRECEDING, LAG(col, N)).
    /// Converted to days by multiplying by the model's granularity.
    Periods(u32),
    /// Fixed duration in days (RANGE BETWEEN INTERVAL 'N days' PRECEDING,
    /// JOIN ON a.day = b.day - INTERVAL 'N days').
    Days(u32),
    /// Unbounded — query scans full history (correlated subquery, UNBOUNDED PRECEDING with RANGE).
    Unbounded { reason: String },
}

impl TemporalOffset {
    /// Convert to days given a granularity period size in days.
    /// Returns None for Unbounded.
    pub fn to_days(&self, period_days: u32) -> Option<u32> {
        match self {
            TemporalOffset::Zero => Some(0),
            TemporalOffset::Periods(n) => Some(n * period_days),
            TemporalOffset::Days(n) => Some(*n),
            TemporalOffset::Unbounded { .. } => None,
        }
    }

    /// Returns true if this offset is unbounded.
    pub fn is_unbounded(&self) -> bool {
        matches!(self, TemporalOffset::Unbounded { .. })
    }

    /// Merge two offsets, taking the larger one.
    pub fn max_with(&self, other: &TemporalOffset) -> TemporalOffset {
        match (self, other) {
            (TemporalOffset::Unbounded { .. }, _) | (_, TemporalOffset::Unbounded { .. }) => {
                // Prefer the one that has a reason
                if let TemporalOffset::Unbounded { reason } = self {
                    TemporalOffset::Unbounded {
                        reason: reason.clone(),
                    }
                } else if let TemporalOffset::Unbounded { reason } = other {
                    TemporalOffset::Unbounded {
                        reason: reason.clone(),
                    }
                } else {
                    unreachable!()
                }
            }
            (TemporalOffset::Zero, x) | (x, TemporalOffset::Zero) => x.clone(),
            (TemporalOffset::Days(a), TemporalOffset::Days(b)) => {
                TemporalOffset::Days((*a).max(*b))
            }
            (TemporalOffset::Periods(a), TemporalOffset::Periods(b)) => {
                TemporalOffset::Periods((*a).max(*b))
            }
            // When mixing periods and days, we can't compare directly since
            // 1 period could be 1 day, 7 days (weekly), or 30 days (monthly).
            // Conservatively keep both by returning the one that could be larger.
            // Since periods expand with granularity (1 period >= 1 day), prefer
            // Periods when in doubt — it will be correctly resolved to days later
            // via `to_days(period_days)`.
            (TemporalOffset::Days(d), TemporalOffset::Periods(p))
            | (TemporalOffset::Periods(p), TemporalOffset::Days(d)) => {
                // We can't know the true comparison without granularity info.
                // Keep Periods since it scales with granularity and is more likely
                // to be the larger value. The caller resolves via to_days(period_days).
                // Only pick Days if it's clearly larger even assuming 1 period = 1 day.
                if *d > *p {
                    TemporalOffset::Days(*d)
                } else {
                    TemporalOffset::Periods(*p)
                }
            }
        }
    }
}

/// Where a temporal dependency was detected.
#[derive(Debug, Clone, Serialize)]
pub enum TemporalSource {
    /// Window frame: e.g., "SUM OVER ROWS BETWEEN 6 PRECEDING AND CURRENT ROW"
    WindowFrame { function: String, frame: String },
    /// LAG/LEAD function with offset
    LagLead { function: String, offset: u32 },
    /// JOIN with temporal offset: e.g., "ON a.day = b.day - INTERVAL '1 day'"
    JoinOffset { interval: String },
    /// WHERE clause with temporal offset
    WhereOffset { interval: String },
    /// Unbounded dependency (correlated subquery, UNBOUNDED PRECEDING with RANGE)
    Unbounded { reason: String },
}

/// Analyze a SQL query for temporal dependencies.
///
/// Examines window functions, LAG/LEAD, JOIN conditions, and WHERE clauses
/// to determine how much historical and future context the query needs.
pub fn analyze_temporal_dependencies(sql: &str) -> TemporalDependency {
    let stripped = crate::types::Frontmatter::strip(sql);
    let parse_result = parse(stripped);
    let root = parse_result.syntax();
    let file = match File::cast(root) {
        Some(f) => f,
        None => return TemporalDependency::default(),
    };
    let select = match file.select_stmt() {
        Some(s) => s,
        None => return TemporalDependency::default(),
    };

    let mut dep = TemporalDependency::default();
    analyze_one_select(&select, &mut dep);
    // Window-frame RANGE INTERVAL lookbacks declared inside a derived table or
    // an inlined `smelt.define` body are invisible to the AST scan above (it
    // only reaches the outer SELECT list, and the expanded body lands as a
    // derived table the re-parse may not surface cleanly). Scan the whole
    // statement text for explicit `RANGE BETWEEN INTERVAL … PRECEDING/FOLLOWING`
    // frames as a robust catch. Only explicit INTERVAL frames are recognized,
    // so this only ever adds a *bounded* lookback; bounds merge via `max_with`,
    // so re-seeing the outer frame already handled by the AST is harmless.
    analyze_subquery_range_frames(stripped, &mut dep);
    dep
}

/// Analyze a single SELECT level — its window functions (SELECT list), WHERE
/// time filters, and JOIN time filters — merging the findings into `dep`. Does
/// not descend into subqueries; the caller walks those separately.
fn analyze_one_select(select: &SelectStmt, dep: &mut TemporalDependency) {
    if let Some(select_list) = select.select_list() {
        for item in select_list.items() {
            if let Some(expr) = item.expression() {
                analyze_expr_temporal(&expr, dep);
            }
        }
    }

    if let Some(where_clause) = select.where_clause() {
        let where_text = where_clause.text().to_uppercase();
        analyze_where_temporal(&where_text, dep);
    }

    if let Some(from_clause) = select.from_clause() {
        let from_text = from_clause.text();
        analyze_join_temporal(&from_text, dep);
    }
}

/// Text scan for window-frame `RANGE BETWEEN INTERVAL '…' PRECEDING/FOLLOWING`
/// lookbacks anywhere in the statement — notably inside a derived table or an
/// inlined `smelt.define` body, which the AST window scan (outer SELECT list
/// only) does not reach and which the re-parse may not surface as a clean
/// subquery. Only explicit `INTERVAL` frames are recognized, so this only ever
/// adds a *bounded* lookback — it never introduces an unbounded classification
/// for a default-frame window.
fn analyze_subquery_range_frames(sql_text: &str, dep: &mut TemporalDependency) {
    let upper = sql_text.to_uppercase();
    if !upper.contains("RANGE BETWEEN") || !upper.contains("INTERVAL") {
        return;
    }
    let Some(days) = find_interval_in_text(&upper) else {
        return;
    };
    if days == 0 {
        return;
    }
    if upper.contains("PRECEDING") {
        dep.sources.push(TemporalSource::WindowFrame {
            function: "(derived table)".to_string(),
            frame: format!("RANGE BETWEEN INTERVAL '{} day' PRECEDING", days),
        });
        dep.lookback = dep.lookback.max_with(&TemporalOffset::Days(days));
    }
    if upper.contains("FOLLOWING") && !upper.contains("UNBOUNDED FOLLOWING") {
        dep.lookahead = dep.lookahead.max_with(&TemporalOffset::Days(days));
    }
}

/// Analyze an expression for temporal dependencies (window functions, LAG/LEAD).
fn analyze_expr_temporal(expr: &smelt_parser::Expr, dep: &mut TemporalDependency) {
    // Check for window spec (OVER clause)
    if let Some(window_spec) = expr.window_spec() {
        if let Some(func) = expr.as_function_call() {
            let func_name = func.name().unwrap_or_default().to_uppercase();

            // Check for LAG/LEAD specifically
            if func_name == "LAG" || func_name == "LEAD" {
                let offset = extract_lag_lead_offset(&func);
                let source = TemporalSource::LagLead {
                    function: func_name.clone(),
                    offset,
                };

                if func_name == "LAG" {
                    dep.lookback = dep.lookback.max_with(&TemporalOffset::Periods(offset));
                } else {
                    dep.lookahead = dep.lookahead.max_with(&TemporalOffset::Periods(offset));
                }
                dep.sources.push(source);
                return;
            }

            // Check window frame
            if let Some(frame) = window_spec.frame() {
                let frame_text = format_frame(&frame);
                let (lookback, lookahead) = analyze_window_frame(&frame);

                dep.sources.push(TemporalSource::WindowFrame {
                    function: func_name,
                    frame: frame_text,
                });
                dep.lookback = dep.lookback.max_with(&lookback);
                dep.lookahead = dep.lookahead.max_with(&lookahead);
            } else {
                // Window function with no explicit frame — default frame depends on
                // whether ORDER BY is present:
                // - With ORDER BY: RANGE BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW
                // - Without ORDER BY: RANGE BETWEEN UNBOUNDED PRECEDING AND UNBOUNDED FOLLOWING
                // Both are unbounded lookback.
                dep.sources.push(TemporalSource::Unbounded {
                    reason: "window function with default frame (no explicit ROWS/RANGE clause)"
                        .to_string(),
                });
                dep.lookback = dep.lookback.max_with(&TemporalOffset::Unbounded {
                    reason: "window function with default frame".to_string(),
                });
            }
        }
    }
}

/// Extract the offset from LAG(col, N) or LEAD(col, N). Default is 1.
fn extract_lag_lead_offset(func: &smelt_parser::FunctionCall) -> u32 {
    let args = func.arguments();
    if args.len() >= 2 {
        // Second argument is the offset
        let offset_text = args[1].text().trim().to_string();
        offset_text.parse::<u32>().unwrap_or(1)
    } else {
        1 // Default offset
    }
}

/// Format a window frame for display.
fn format_frame(frame: &smelt_parser::WindowFrame) -> String {
    let unit = match frame.unit() {
        Some(FrameUnit::Rows) => "ROWS",
        Some(FrameUnit::Range) => "RANGE",
        Some(FrameUnit::Groups) => "GROUPS",
        None => "UNKNOWN",
    };
    let bounds: Vec<String> = frame.bounds().iter().map(|b| b.text()).collect();
    format!("{} BETWEEN {}", unit, bounds.join(" AND "))
}

/// Analyze a window frame to determine lookback and lookahead.
fn analyze_window_frame(frame: &smelt_parser::WindowFrame) -> (TemporalOffset, TemporalOffset) {
    let bounds = frame.bounds();
    let unit = frame.unit();

    let mut lookback = TemporalOffset::Zero;
    let mut lookahead = TemporalOffset::Zero;

    for bound in &bounds {
        let text = bound.text().to_uppercase();
        let trimmed = text.trim();

        if trimmed.contains("UNBOUNDED") && trimmed.contains("PRECEDING") {
            // UNBOUNDED PRECEDING
            if unit == Some(FrameUnit::Range) {
                lookback = TemporalOffset::Unbounded {
                    reason: "RANGE BETWEEN UNBOUNDED PRECEDING".to_string(),
                };
            } else {
                // ROWS BETWEEN UNBOUNDED PRECEDING — still unbounded
                lookback = TemporalOffset::Unbounded {
                    reason: "ROWS BETWEEN UNBOUNDED PRECEDING".to_string(),
                };
            }
        } else if trimmed.contains("UNBOUNDED") && trimmed.contains("FOLLOWING") {
            lookahead = TemporalOffset::Unbounded {
                reason: "UNBOUNDED FOLLOWING".to_string(),
            };
        } else if trimmed.contains("PRECEDING") {
            // N PRECEDING — extract N
            if let Some(n) = extract_bound_number(trimmed) {
                if trimmed.contains("INTERVAL") || unit == Some(FrameUnit::Range) {
                    // RANGE or INTERVAL — days
                    let days = extract_interval_days(trimmed).unwrap_or(n);
                    lookback = lookback.max_with(&TemporalOffset::Days(days));
                } else {
                    // ROWS — periods
                    lookback = lookback.max_with(&TemporalOffset::Periods(n));
                }
            }
        } else if trimmed.contains("FOLLOWING") {
            if let Some(n) = extract_bound_number(trimmed) {
                if trimmed.contains("INTERVAL") || unit == Some(FrameUnit::Range) {
                    let days = extract_interval_days(trimmed).unwrap_or(n);
                    lookahead = lookahead.max_with(&TemporalOffset::Days(days));
                } else {
                    lookahead = lookahead.max_with(&TemporalOffset::Periods(n));
                }
            }
        }
        // CURRENT ROW — no offset needed
    }

    (lookback, lookahead)
}

/// Extract the numeric value from a frame bound like "6 PRECEDING" or "3 FOLLOWING".
fn extract_bound_number(text: &str) -> Option<u32> {
    text.split_whitespace()
        .find_map(|word| word.parse::<u32>().ok())
}

/// Extract days from an interval expression like "INTERVAL '3' DAY" or
/// "INTERVAL '3 days'", by routing the recognised unit through the shared
/// [`source_bounds::parse_interval`] — the single interval-literal parser
/// used everywhere in this crate (see `docs/specs/model_properties.md`
/// "Unified bound / reach derivation"). Month/year fold to
/// `Offset::Symbolic` there, since a month/year has no fixed length in
/// seconds — but this module's `EffectiveWindow` is an advisory
/// day-granular estimate consumed to widen the actual filter/DELETE range
/// for real incremental runs (unlike `source_bounds`'s fail-closed pushdown
/// proof), so silently falling through to the caller's bare-number fallback
/// would under-widen the lookback by ~30-90x. Apply the same calendar
/// approximation this estimate has always used: month → 30 days/period,
/// year → 365 days/period.
fn extract_interval_days(text: &str) -> Option<u32> {
    let n = extract_bound_number(text)?;
    let upper = text.to_uppercase();
    let unit = if upper.contains("DAY") {
        "DAY"
    } else if upper.contains("WEEK") {
        "WEEK"
    } else if upper.contains("MONTH") {
        "MONTH"
    } else if upper.contains("YEAR") {
        "YEAR"
    } else if upper.contains("HOUR") {
        "HOUR"
    } else {
        // Default: assume days.
        "DAY"
    };

    match source_bounds::parse_interval(&format!("{n} {unit}"))? {
        source_bounds::Offset::Seconds(s) => {
            // Round up to whole days — a sub-day duration (e.g. HOUR) still
            // needs at least 1 day of lookback/lookahead margin.
            let days = s.0.div_ceil(86400) as u32;
            Some(if n > 0 { days.max(1) } else { 0 })
        }
        source_bounds::Offset::Symbolic(_) => Some(match unit {
            "MONTH" => n * 30,
            "YEAR" => n * 365,
            _ => n,
        }),
    }
}

/// Analyze WHERE clause text for temporal offsets (e.g., col >= day - INTERVAL '3 days').
fn analyze_where_temporal(where_text: &str, dep: &mut TemporalDependency) {
    // Look for INTERVAL patterns in WHERE clause
    if let Some(days) = find_interval_in_text(where_text) {
        dep.sources.push(TemporalSource::WhereOffset {
            interval: format!("{} days", days),
        });
        dep.lookback = dep.lookback.max_with(&TemporalOffset::Days(days));
    }
}

/// Analyze JOIN clause text for temporal offsets.
fn analyze_join_temporal(from_text: &str, dep: &mut TemporalDependency) {
    let upper = from_text.to_uppercase();

    // Look for INTERVAL patterns in JOIN ON conditions
    // Common patterns: ON a.day = b.day - INTERVAL '1 day'
    //                  ON a.day = b.day + INTERVAL '1 day'
    if !upper.contains("JOIN") {
        return;
    }

    if let Some(days) = find_interval_in_text(&upper) {
        // Determine direction: if there's a minus before INTERVAL, it's lookback
        // If plus, could be lookahead
        let is_minus = upper.contains("- INTERVAL") || upper.contains("-INTERVAL");
        let is_plus = upper.contains("+ INTERVAL") || upper.contains("+INTERVAL");

        if is_minus {
            dep.sources.push(TemporalSource::JoinOffset {
                interval: format!("{} days", days),
            });
            dep.lookback = dep.lookback.max_with(&TemporalOffset::Days(days));
        } else if is_plus {
            dep.sources.push(TemporalSource::JoinOffset {
                interval: format!("{} days", days),
            });
            dep.lookahead = dep.lookahead.max_with(&TemporalOffset::Days(days));
        }
    }
}

/// Find all INTERVAL 'N days/day/weeks/...' patterns in text and return the max number of days.
fn find_interval_in_text(text: &str) -> Option<u32> {
    let upper = text.to_uppercase();
    let mut max_days: Option<u32> = None;
    let mut search_from = 0;

    while let Some(rel_pos) = upper[search_from..].find("INTERVAL") {
        let interval_pos = search_from + rel_pos;
        let after = &upper[interval_pos + 8..];

        // Find the quoted value
        if let Some(quote_start) = after.find('\'') {
            let rest = &after[quote_start + 1..];
            if let Some(quote_end) = rest.find('\'') {
                let value = &rest[..quote_end];

                // Parse the value — could be "3" or "3 days"
                if let Some(n) = value.split_whitespace().find_map(|w| w.parse::<u32>().ok()) {
                    let combined = format!("{} {}", value, &rest[quote_end + 1..]);
                    if let Some(days) = extract_interval_days_from_combined(&combined, n) {
                        max_days = Some(max_days.map_or(days, |prev: u32| prev.max(days)));
                    }
                }

                // Advance past this INTERVAL occurrence
                search_from = interval_pos + 8 + quote_start + 1 + quote_end + 1;
                continue;
            }
        }

        // Couldn't parse this one, skip past the keyword
        search_from = interval_pos + 8;
    }

    max_days
}

/// Given a combined interval text (the quoted value plus any unit text that
/// followed the closing quote, e.g. `"3 " + "DAY"` for `INTERVAL '3' DAY`)
/// and the numeric value, determine days — routed through the shared
/// [`source_bounds::parse_interval`] (see `extract_interval_days`'s doc for
/// why month/year apply a calendar approximation here rather than folding
/// through to `Offset::Symbolic`/`None`).
fn extract_interval_days_from_combined(text: &str, n: u32) -> Option<u32> {
    let upper = text.to_uppercase();
    let unit = if upper.contains("DAY") {
        "DAY"
    } else if upper.contains("WEEK") {
        "WEEK"
    } else if upper.contains("MONTH") {
        "MONTH"
    } else if upper.contains("YEAR") {
        "YEAR"
    } else if upper.contains("HOUR") {
        "HOUR"
    } else {
        // Default: assume days.
        "DAY"
    };

    match source_bounds::parse_interval(&format!("{n} {unit}"))? {
        source_bounds::Offset::Seconds(s) => {
            let days = s.0.div_ceil(86400) as u32;
            Some(if n > 0 { days.max(1) } else { 0 })
        }
        source_bounds::Offset::Symbolic(_) => Some(match unit {
            "MONTH" => n * 30,
            "YEAR" => n * 365,
            _ => n,
        }),
    }
}

/// The effective window to apply during incremental execution.
///
/// Combines AST-inferred temporal dependencies with upstream data latency
/// to determine the actual filter range and partition range for execution.
#[derive(Debug, Clone, Serialize)]
pub struct EffectiveWindow {
    /// Total lookback in days (max of temporal dependency and data latency).
    pub lookback_days: u32,
    /// Total lookahead in days (from temporal dependency only; latency doesn't affect lookahead).
    pub lookahead_days: u32,
    /// Whether the lookback is unbounded (requires full refresh or explicit override).
    pub is_unbounded: bool,
    /// Human-readable explanation of how the window was computed.
    pub explanation: String,
}

/// Compute the effective window from temporal dependencies and data latency.
///
/// The formula is:
///   effective_lookback  = max(ast_inferred_lookback, data_latency)
///   effective_lookahead = ast_inferred_lookahead  (data_latency doesn't affect lookahead)
///
/// `period_days` is the number of days per partition period (1 for Day, 7 for Week, etc.)
/// and is used to convert period-based offsets to days.
pub fn compute_effective_window(
    temporal: &TemporalDependency,
    data_latency_days: u32,
    period_days: u32,
) -> EffectiveWindow {
    let ast_lookback_days = temporal.lookback.to_days(period_days);
    let ast_lookahead_days = temporal.lookahead.to_days(period_days);

    let is_unbounded = temporal.lookback.is_unbounded() || temporal.lookahead.is_unbounded();

    let lookback_days = if let Some(ast_days) = ast_lookback_days {
        ast_days.max(data_latency_days)
    } else {
        0 // Unbounded — lookback_days is meaningless
    };

    let lookahead_days = ast_lookahead_days.unwrap_or(0);

    let explanation = format_explanation(
        temporal,
        data_latency_days,
        lookback_days,
        lookahead_days,
        is_unbounded,
    );

    EffectiveWindow {
        lookback_days,
        lookahead_days,
        is_unbounded,
        explanation,
    }
}

fn format_explanation(
    temporal: &TemporalDependency,
    data_latency_days: u32,
    lookback_days: u32,
    lookahead_days: u32,
    is_unbounded: bool,
) -> String {
    let mut parts = Vec::new();

    // Describe temporal dependency sources
    for source in &temporal.sources {
        match source {
            TemporalSource::WindowFrame { function, frame } => {
                parts.push(format!("{} {}", function, frame));
            }
            TemporalSource::LagLead { function, offset } => {
                parts.push(format!("{}(col, {})", function, offset));
            }
            TemporalSource::JoinOffset { interval } => {
                parts.push(format!("JOIN offset: {}", interval));
            }
            TemporalSource::WhereOffset { interval } => {
                parts.push(format!("WHERE offset: {}", interval));
            }
            TemporalSource::Unbounded { reason } => {
                parts.push(format!("unbounded: {}", reason));
            }
        }
    }

    if is_unbounded {
        return format!(
            "unbounded lookback (sources: {})",
            if parts.is_empty() {
                "none".to_string()
            } else {
                parts.join(", ")
            }
        );
    }

    let ast_part = if parts.is_empty() {
        "0 days (partition-local)".to_string()
    } else {
        parts.join(", ")
    };

    if data_latency_days > 0 {
        format!(
            "lookback={} days, lookahead={} days (AST: {}, data latency: {} days)",
            lookback_days, lookahead_days, ast_part, data_latency_days
        )
    } else {
        format!(
            "lookback={} days, lookahead={} days (AST: {})",
            lookback_days, lookahead_days, ast_part
        )
    }
}

/// Returns the number of days per period for a given granularity.
pub fn granularity_period_days(granularity: &smelt_core::Granularity) -> u32 {
    match granularity {
        smelt_core::Granularity::Hour => 1, // sub-day: 1 day per period for conservative estimate
        smelt_core::Granularity::Day => 1,
        smelt_core::Granularity::Week => 7,
        smelt_core::Granularity::Month => 30,
        smelt_core::Granularity::Quarter => 91,
        smelt_core::Granularity::Year => 365,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_group_by_no_dependency() {
        let sql = "SELECT date_trunc('day', event_time) as d, SUM(amount) as total FROM events GROUP BY 1";
        let dep = analyze_temporal_dependencies(sql);
        assert_eq!(dep.lookback, TemporalOffset::Zero);
        assert_eq!(dep.lookahead, TemporalOffset::Zero);
        assert!(dep.sources.is_empty());
    }

    #[test]
    fn test_lag_default_offset() {
        let sql = "SELECT user_id, amount, LAG(amount) OVER (PARTITION BY user_id ORDER BY day) as prev_amount FROM events";
        let dep = analyze_temporal_dependencies(sql);
        assert_eq!(dep.lookback, TemporalOffset::Periods(1));
        assert_eq!(dep.lookahead, TemporalOffset::Zero);
        assert_eq!(dep.sources.len(), 1);
        assert!(
            matches!(&dep.sources[0], TemporalSource::LagLead { function, offset } if function == "LAG" && *offset == 1)
        );
    }

    #[test]
    fn test_lag_explicit_offset() {
        let sql = "SELECT user_id, LAG(amount, 3) OVER (ORDER BY day) as prev_3 FROM events";
        let dep = analyze_temporal_dependencies(sql);
        assert_eq!(dep.lookback, TemporalOffset::Periods(3));
    }

    #[test]
    fn test_lead_offset() {
        let sql = "SELECT user_id, LEAD(amount, 2) OVER (ORDER BY day) as next_2 FROM events";
        let dep = analyze_temporal_dependencies(sql);
        assert_eq!(dep.lookback, TemporalOffset::Zero);
        assert_eq!(dep.lookahead, TemporalOffset::Periods(2));
    }

    #[test]
    fn test_rows_between_n_preceding() {
        let sql = "SELECT user_id, SUM(amount) OVER (PARTITION BY user_id ORDER BY day ROWS BETWEEN 6 PRECEDING AND CURRENT ROW) as rolling_7d FROM events";
        let dep = analyze_temporal_dependencies(sql);
        assert_eq!(dep.lookback, TemporalOffset::Periods(6));
        assert_eq!(dep.lookahead, TemporalOffset::Zero);
        assert!(matches!(
            &dep.sources[0],
            TemporalSource::WindowFrame { .. }
        ));
    }

    #[test]
    fn test_rows_between_preceding_and_following() {
        let sql = "SELECT user_id, AVG(amount) OVER (ORDER BY day ROWS BETWEEN 3 PRECEDING AND 2 FOLLOWING) as centered_avg FROM events";
        let dep = analyze_temporal_dependencies(sql);
        assert_eq!(dep.lookback, TemporalOffset::Periods(3));
        assert_eq!(dep.lookahead, TemporalOffset::Periods(2));
    }

    #[test]
    fn test_unbounded_preceding_rows() {
        let sql = "SELECT user_id, SUM(amount) OVER (PARTITION BY user_id ORDER BY day ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW) as running_total FROM events";
        let dep = analyze_temporal_dependencies(sql);
        assert!(dep.lookback.is_unbounded());
    }

    #[test]
    fn test_multiple_window_functions_max() {
        let sql = r#"
            SELECT
                user_id,
                LAG(amount, 2) OVER (ORDER BY day) as prev_2,
                LAG(amount, 5) OVER (ORDER BY day) as prev_5
            FROM events
        "#;
        let dep = analyze_temporal_dependencies(sql);
        assert_eq!(dep.lookback, TemporalOffset::Periods(5));
        assert_eq!(dep.sources.len(), 2);
    }

    #[test]
    fn test_join_with_interval_lookback() {
        let sql = "SELECT a.day, a.amount, b.amount as prev_amount FROM events a INNER JOIN events b ON a.day = b.day - INTERVAL '1' DAY";
        let dep = analyze_temporal_dependencies(sql);
        assert_eq!(dep.lookback, TemporalOffset::Days(1));
        assert!(matches!(&dep.sources[0], TemporalSource::JoinOffset { .. }));
    }

    #[test]
    fn test_where_with_interval() {
        let sql = "SELECT * FROM events WHERE event_time >= partition_date - INTERVAL '3' DAY";
        let dep = analyze_temporal_dependencies(sql);
        assert_eq!(dep.lookback, TemporalOffset::Days(3));
        assert!(matches!(
            &dep.sources[0],
            TemporalSource::WhereOffset { .. }
        ));
    }

    #[test]
    fn test_frontmatter_stripped() {
        let sql =
            "---\nname: test\n---\nSELECT user_id, LAG(amount) OVER (ORDER BY day) FROM events";
        let dep = analyze_temporal_dependencies(sql);
        assert_eq!(dep.lookback, TemporalOffset::Periods(1));
    }

    #[test]
    fn test_offset_max_with() {
        assert_eq!(
            TemporalOffset::Periods(3).max_with(&TemporalOffset::Periods(5)),
            TemporalOffset::Periods(5)
        );
        assert_eq!(
            TemporalOffset::Days(3).max_with(&TemporalOffset::Days(7)),
            TemporalOffset::Days(7)
        );
        assert_eq!(
            TemporalOffset::Zero.max_with(&TemporalOffset::Periods(3)),
            TemporalOffset::Periods(3)
        );
        assert!(TemporalOffset::Periods(5)
            .max_with(&TemporalOffset::Unbounded {
                reason: "test".into()
            })
            .is_unbounded());
    }

    #[test]
    fn test_to_days() {
        assert_eq!(TemporalOffset::Zero.to_days(1), Some(0));
        assert_eq!(TemporalOffset::Periods(6).to_days(1), Some(6)); // daily granularity
        assert_eq!(TemporalOffset::Periods(6).to_days(7), Some(42)); // weekly granularity
        assert_eq!(TemporalOffset::Days(10).to_days(1), Some(10));
        assert_eq!(
            TemporalOffset::Unbounded {
                reason: "test".into()
            }
            .to_days(1),
            None
        );
    }

    #[test]
    fn test_effective_window_partition_local() {
        // Simple GROUP BY — no temporal dependency, no latency
        let dep = TemporalDependency::default();
        let window = compute_effective_window(&dep, 0, 1);
        assert_eq!(window.lookback_days, 0);
        assert_eq!(window.lookahead_days, 0);
        assert!(!window.is_unbounded);
    }

    #[test]
    fn test_effective_window_latency_only() {
        // No temporal dependency, but 3-day data latency
        let dep = TemporalDependency::default();
        let window = compute_effective_window(&dep, 3, 1);
        assert_eq!(window.lookback_days, 3);
        assert_eq!(window.lookahead_days, 0);
    }

    #[test]
    fn test_effective_window_ast_wins_over_latency() {
        // 6-day temporal dependency from window function, 3-day latency
        let dep = TemporalDependency {
            lookback: TemporalOffset::Periods(6),
            lookahead: TemporalOffset::Zero,
            sources: vec![],
        };
        let window = compute_effective_window(&dep, 3, 1);
        assert_eq!(window.lookback_days, 6); // AST wins
    }

    #[test]
    fn test_effective_window_latency_wins_over_ast() {
        // 2-day temporal dependency, 5-day latency
        let dep = TemporalDependency {
            lookback: TemporalOffset::Days(2),
            lookahead: TemporalOffset::Zero,
            sources: vec![],
        };
        let window = compute_effective_window(&dep, 5, 1);
        assert_eq!(window.lookback_days, 5); // Latency wins
    }

    #[test]
    fn test_effective_window_with_lookahead() {
        // LEAD(col, 2) with daily granularity
        let dep = TemporalDependency {
            lookback: TemporalOffset::Zero,
            lookahead: TemporalOffset::Periods(2),
            sources: vec![],
        };
        let window = compute_effective_window(&dep, 0, 1);
        assert_eq!(window.lookback_days, 0);
        assert_eq!(window.lookahead_days, 2);
    }

    #[test]
    fn test_effective_window_unbounded() {
        let dep = TemporalDependency {
            lookback: TemporalOffset::Unbounded {
                reason: "test".into(),
            },
            lookahead: TemporalOffset::Zero,
            sources: vec![],
        };
        let window = compute_effective_window(&dep, 3, 1);
        assert!(window.is_unbounded);
    }

    #[test]
    fn test_range_interval_months_preceding_approximates_30_days_per_month() {
        let sql = "SELECT user_id, SUM(amount) OVER (PARTITION BY user_id ORDER BY day RANGE BETWEEN INTERVAL '3 months' PRECEDING AND CURRENT ROW) as rolling_3mo FROM events";
        let dep = analyze_temporal_dependencies(sql);
        assert_eq!(dep.lookback, TemporalOffset::Days(90));
    }

    #[test]
    fn test_range_interval_years_preceding_approximates_365_days_per_year() {
        let sql = "SELECT user_id, SUM(amount) OVER (PARTITION BY user_id ORDER BY day RANGE BETWEEN INTERVAL '2 years' PRECEDING AND CURRENT ROW) as rolling_2yr FROM events";
        let dep = analyze_temporal_dependencies(sql);
        assert_eq!(dep.lookback, TemporalOffset::Days(730));
    }

    #[test]
    fn test_effective_window_weekly_granularity() {
        // 2 periods lookback with weekly granularity = 14 days
        let dep = TemporalDependency {
            lookback: TemporalOffset::Periods(2),
            lookahead: TemporalOffset::Zero,
            sources: vec![],
        };
        let window = compute_effective_window(&dep, 0, 7);
        assert_eq!(window.lookback_days, 14);
    }
}
