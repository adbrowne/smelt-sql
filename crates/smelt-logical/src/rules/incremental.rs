use serde::Serialize;
use std::collections::HashMap;
use tracing::info;

use crate::analysis::source_bounds::{
    derive_model_bounds, BoundContext, BoundResult, InjectionPoint,
};
use crate::analysis::temporal::{analyze_temporal_dependencies, TemporalOffset};
use crate::analysis::{analyze_select, SelectItemKind};
use crate::graph::{ModelGraph, ModelInfo};
use crate::types::{Opportunity, OpportunityData, Transformation};

/// Non-deterministic function names that produce different results on each run.
const NONDETERMINISTIC_FUNCTIONS: &[&str] = &[
    "RANDOM",
    "RAND",
    "NOW",
    "CURRENT_TIMESTAMP",
    "CURRENT_DATE",
    "UUID",
    "GEN_RANDOM_UUID",
    "SETSEED",
];

/// How safely a model can be backfilled in large batches.
///
/// Derived from temporal dependency analysis (Phase 3). Models with no
/// cross-partition dependencies can process any range in a single query;
/// models with bounded lookback need chunked execution; models with
/// unbounded dependencies must go per-partition or full refresh.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum BatchSafety {
    /// Single query for any range — aggregations are partition-local.
    FullyBatchSafe,
    /// Safe for bounded chunks — window functions or joins need context rows
    /// but the dependency is bounded.
    BoundedSafe {
        /// Maximum recommended chunk size in days. Each chunk query fetches
        /// `[start - context_days, end)` but only writes `[start, end)`.
        max_chunk_days: u32,
        /// Extra context days needed before each chunk's start.
        context_days: u32,
        /// Human-readable explanation of why chunking is needed.
        reason: String,
    },
    /// Must process per-partition — cross-partition dependencies are unbounded.
    PerPartitionOnly {
        /// Human-readable explanation of why per-partition is required.
        reason: String,
    },
}

/// Analyze how safely a model can be backfilled in batches.
///
/// Uses the SQL's temporal dependency (lookback/lookahead) to determine
/// whether a single query can cover any range, or whether the range must
/// be split into chunks or individual partitions.
pub fn analyze_batch_safety(model: &ModelInfo) -> BatchSafety {
    let sql = crate::types::Frontmatter::strip(&model.sql);
    let temporal = analyze_temporal_dependencies(sql);

    // Determine granularity period for converting Periods→Days
    let period_days = model
        .timeseries_config
        .as_ref()
        .map(|c| crate::analysis::temporal::granularity_period_days(&c.granularity))
        .unwrap_or(1);

    // Check lookback
    let lookback_days = match &temporal.lookback {
        TemporalOffset::Zero => 0,
        TemporalOffset::Periods(n) => n * period_days,
        TemporalOffset::Days(n) => *n,
        TemporalOffset::Unbounded { reason } => {
            return BatchSafety::PerPartitionOnly {
                reason: format!("unbounded lookback: {}", reason),
            };
        }
    };

    // Check lookahead
    let lookahead_days = match &temporal.lookahead {
        TemporalOffset::Zero => 0,
        TemporalOffset::Periods(n) => n * period_days,
        TemporalOffset::Days(n) => *n,
        TemporalOffset::Unbounded { reason } => {
            return BatchSafety::PerPartitionOnly {
                reason: format!("unbounded lookahead: {}", reason),
            };
        }
    };

    let context_days = lookback_days.max(lookahead_days);

    if context_days == 0 {
        BatchSafety::FullyBatchSafe
    } else {
        // Recommend chunks at least 3x the context to keep overhead reasonable.
        // Minimum chunk = 7 days, maximum recommended = 90 days.
        let min_chunk = context_days * 3;
        let max_chunk_days = min_chunk.clamp(7, 90);

        if min_chunk > 90 {
            info!(
                "ideal chunk size ({} days, 3x context of {} days) exceeds 90-day cap; \
                 using 90-day chunks — override with --batch-size if needed",
                min_chunk, context_days
            );
        }

        let reasons: Vec<String> = temporal
            .sources
            .iter()
            .map(|s| format!("{:?}", s))
            .collect();

        BatchSafety::BoundedSafe {
            max_chunk_days,
            context_days,
            reason: if reasons.is_empty() {
                format!("temporal dependency of {} day(s)", context_days)
            } else {
                format!(
                    "temporal dependency of {} day(s) from: {}",
                    context_days,
                    reasons.join(", ")
                )
            },
        }
    }
}

/// Detect incremental materialization opportunity from frontmatter config.
pub fn detect(model: &ModelInfo) -> Result<Option<Opportunity>, String> {
    let inc_config = match &model.incremental_config {
        Some(c) => c,
        None => return Ok(None),
    };

    // timeseries: is required when incremental: is present
    let ts_config = model.timeseries_config.as_ref().ok_or_else(|| {
        format!(
            "Model '{}': has incremental config but no timeseries config",
            model.name
        )
    })?;

    let analysis = analyze_select(&model.sql).ok_or_else(|| {
        format!(
            "Model '{}': has incremental config but SQL could not be parsed",
            model.name
        )
    })?;

    let partition_col = &ts_config.partition_column;
    let event_time_column = &ts_config.event_time_column;
    let overrides = &inc_config.safety_overrides;

    // Validate partition_column alias exists in SELECT list
    let partition_item = analysis.items.iter().find(|item| match item {
        SelectItemKind::GroupByKey { alias, .. } => alias == partition_col,
        SelectItemKind::CountDistinct { alias, .. } => alias == partition_col,
        SelectItemKind::OtherAggregate { alias, .. } => alias == partition_col,
    });

    let partition_item = partition_item.ok_or_else(|| {
        format!(
            "Model '{}': incremental partition_column '{}' not found as alias in SELECT list",
            model.name, partition_col
        )
    })?;

    // Get the expression for the partition column
    let partition_expr = match partition_item {
        SelectItemKind::GroupByKey { text, .. } => text.clone(),
        SelectItemKind::CountDistinct { argument, .. } => argument.clone(),
        SelectItemKind::OtherAggregate { text, .. } => text.clone(),
    };

    // Validate it appears in GROUP BY (only required for aggregate models that have a GROUP BY).
    // Per-row models (no GROUP BY) are valid for incremental: the partition_column
    // alias in the SELECT list is sufficient — rows are filtered by partition value.
    if !analysis.group_by_exprs.is_empty() {
        let in_group_by = analysis
            .group_by_exprs
            .iter()
            .any(|expr| expr == &partition_expr);
        if !in_group_by {
            return Err(format!(
                "Model '{}': partition_column '{}' (expression: {}) not found in GROUP BY clause",
                model.name, partition_col, partition_expr
            ));
        }
    }

    // Validate event_time_column is referenced in the SQL
    let stripped_sql = crate::types::Frontmatter::strip(&model.sql);
    if !stripped_sql.contains(event_time_column.as_str()) {
        return Err(format!(
            "Model '{}': event_time_column '{}' not found in SQL",
            model.name, event_time_column
        ));
    }

    // Validate unique_key columns exist in SELECT list (needed for MERGE strategy)
    let select_aliases: Vec<&str> = analysis
        .items
        .iter()
        .map(|item| match item {
            SelectItemKind::GroupByKey { alias, .. } => alias.as_str(),
            SelectItemKind::CountDistinct { alias, .. } => alias.as_str(),
            SelectItemKind::OtherAggregate { alias, .. } => alias.as_str(),
        })
        .collect();

    for key_col in &inc_config.unique_key {
        if !select_aliases.contains(&key_col.as_str()) {
            return Err(format!(
                "Model '{}': unique_key column '{}' not found as alias in SELECT list",
                model.name, key_col
            ));
        }
    }

    // --- Safety checks ---

    // 2a: Window functions (OVER clause)
    // Partition-aligned OVER is admitted: OVER (PARTITION BY <keys>) where
    // <keys> is a superset of the model's partition_column is safe because the
    // window is partition-local and the DELETE+INSERT contract holds.
    // Any OVER without PARTITION BY, or whose PARTITION BY keys do not include
    // the partition_column, is rejected unless allow_window_functions is set.
    if !overrides.allow_window_functions {
        let upper_sql = stripped_sql.to_uppercase();
        if upper_sql.contains("OVER(") || upper_sql.contains("OVER (") {
            // Extract every OVER clause and check its PARTITION BY keys.
            if let Some(bad_over) = find_inadmissible_over(stripped_sql, partition_col) {
                return Err(format!(
                    "Model '{}': window function with OVER clause is not compatible with \
                     incremental materialization — window partition keys ({}) do not include \
                     the model's partition_column '{}'. Use OVER (PARTITION BY {} ...) to make \
                     it partition-aligned, or set safety_overrides.allow_window_functions: true",
                    model.name, bad_over, partition_col, partition_col
                ));
            }
        }
    }

    // 2b: HAVING clause
    if !overrides.allow_having {
        let upper_sql = stripped_sql.to_uppercase();
        // Check for HAVING keyword at word boundary (not inside a string or identifier)
        if has_keyword_at_boundary(&upper_sql, "HAVING") {
            return Err(format!(
                "Model '{}': HAVING clause is not compatible with incremental materialization \
                 — groups may change eligibility between incremental and full runs",
                model.name
            ));
        }
    }

    // 2c: LIMIT clause
    if !overrides.allow_limit {
        let upper_sql = stripped_sql.to_uppercase();
        if has_keyword_at_boundary(&upper_sql, "LIMIT") {
            return Err(format!(
                "Model '{}': LIMIT clause is not compatible with incremental materialization \
                 — different time ranges would produce different row subsets",
                model.name
            ));
        }
    }

    // 2d: Subqueries in FROM
    if !overrides.allow_subqueries {
        // Check for '(' in FROM clause text (indicates subquery)
        if analysis.from_text.contains('(')
            && !analysis.from_text.contains("smelt.ref(")
            && !analysis.from_text.contains("smelt.source(")
        {
            return Err(format!(
                "Model '{}': subqueries in FROM clause are not yet supported with incremental \
                 materialization",
                model.name
            ));
        }
    }

    // 2e: Non-deterministic functions
    if !overrides.allow_nondeterministic {
        let upper_sql = stripped_sql.to_uppercase();
        for func_name in NONDETERMINISTIC_FUNCTIONS {
            if has_keyword_at_boundary(&upper_sql, func_name) {
                return Err(format!(
                    "Model '{}': non-deterministic function '{}' is not compatible with \
                     incremental materialization — results will differ between runs",
                    model.name, func_name
                ));
            }
        }
    }

    // 2f: SELECT DISTINCT
    if !overrides.allow_distinct {
        let upper_sql = stripped_sql.trim().to_uppercase();
        // Check for SELECT DISTINCT (not COUNT(DISTINCT ...))
        if upper_sql.starts_with("SELECT DISTINCT")
            || upper_sql.starts_with("SELECT  DISTINCT")
            || upper_sql.contains("\nSELECT DISTINCT")
        {
            return Err(format!(
                "Model '{}': SELECT DISTINCT is not compatible with incremental materialization \
                 — deduplication results may differ on partial data",
                model.name
            ));
        }
    }

    Ok(Some(Opportunity {
        rule_name: "incremental".to_string(),
        model: model.name.clone(),
        description: format!(
            "Incremental materialization on partition column '{}' (source: '{}', granularity: {:?})",
            partition_col, event_time_column, ts_config.granularity,
        ),
        data: OpportunityData::Incremental {
            event_time_column: event_time_column.clone(),
            partition_column: partition_col.clone(),
            granularity: ts_config.granularity.clone(),
        },
    }))
}

/// Scan `sql` for OVER clauses that are not partition-aligned with `partition_col`.
///
/// Returns `Some(description)` for the first OVER clause whose PARTITION BY
/// keys do not form a superset of `{partition_col}`.
///
/// An OVER clause is admissible when its `PARTITION BY` list contains
/// `partition_col` (case-insensitive, trimmed). An OVER clause with no
/// `PARTITION BY` is inadmissible.
///
/// Returns `None` when every OVER clause in the SQL is admissible (or there
/// are none).
fn find_inadmissible_over(sql: &str, partition_col: &str) -> Option<String> {
    let upper_sql = sql.to_uppercase();
    let partition_col_upper = partition_col.to_uppercase();

    let mut search_from = 0;
    while let Some(over_pos) = find_over_keyword(&upper_sql, search_from) {
        // Advance past "OVER" so we don't re-match the same position.
        search_from = over_pos + 4;

        // Skip whitespace after OVER to find the opening '('.
        let rest = &upper_sql[search_from..];
        let paren_offset = match rest.find('(') {
            Some(p) => p,
            None => continue,
        };
        // Ensure only whitespace between OVER and '('.
        let between = &rest[..paren_offset];
        if !between.trim().is_empty() {
            continue;
        }

        let paren_start = search_from + paren_offset; // position of '(' in upper_sql
                                                      // Extract the balanced content inside the OVER (...).
        let over_content = match extract_balanced_parens(&upper_sql, paren_start) {
            Some(c) => c,
            None => continue,
        };
        search_from = paren_start + over_content.len() + 2; // skip '(' content ')'

        // A window carrying a bounded RANGE INTERVAL frame (Form A) is admitted
        // regardless of PARTITION BY alignment: the bounded lookback widens the
        // source read to cover the frame, so the window is partition-local up to
        // that bound and the DELETE+INSERT contract still holds. An UNBOUNDED
        // frame is cumulative-across-history and is NOT made safe this way.
        if has_bounded_range_interval_frame(&over_content) {
            continue;
        }

        // Check for PARTITION BY inside the window spec.
        if let Some(pb_pos) = find_partition_by_in_over(&over_content) {
            let after_pb = &over_content[pb_pos..];
            // Keys end at ORDER BY, ROWS/RANGE/GROUPS, or end of content.
            let keys_text = trim_to_window_clause_end(after_pb);
            // Parse the comma-separated key identifiers.
            let keys: Vec<String> = keys_text
                .split(',')
                .map(|k| k.trim().to_string())
                .filter(|k| !k.is_empty())
                .collect();

            // Check that partition_col is among the keys.
            let contains_partition_col = keys.iter().any(|k| {
                // Strip any trailing qualifiers (e.g. "t.event_date" → "event_date")
                let bare = k.rsplit('.').next().unwrap_or(k);
                bare.trim() == partition_col_upper
            });

            if !contains_partition_col {
                let key_display: Vec<&str> = keys.iter().map(|s| s.as_str()).collect();
                return Some(key_display.join(", "));
            }
        } else {
            // No PARTITION BY at all — inadmissible.
            return Some("<no PARTITION BY>".to_string());
        }
    }
    None
}

/// True if a window spec's frame is a bounded `RANGE BETWEEN INTERVAL '…'
/// PRECEDING [AND …]` (Form A) with no `UNBOUNDED` bound. Such a frame has a
/// finite lookback/lookforward that the source-bound deriver picks up and the
/// planner widens the source read to cover, so the window is partition-local up
/// to that bound — safe for incremental even when PARTITION BY is not aligned to
/// the partition_column. An `UNBOUNDED` bound reads across all history and is
/// deliberately excluded.
fn has_bounded_range_interval_frame(over_content: &str) -> bool {
    let upper = over_content.to_uppercase();
    upper.contains("RANGE BETWEEN") && upper.contains("INTERVAL") && !upper.contains("UNBOUNDED")
}

/// Find the next position of the OVER keyword (word boundary) at or after `from`.
fn find_over_keyword(upper_sql: &str, from: usize) -> Option<usize> {
    let bytes = upper_sql.as_bytes();
    let kw = b"OVER";
    let mut i = from;
    while i + 4 <= bytes.len() {
        if &bytes[i..i + 4] == kw {
            let before_ok = i == 0 || !bytes[i - 1].is_ascii_alphanumeric() && bytes[i - 1] != b'_';
            let after_ok = i + 4 >= bytes.len()
                || (!bytes[i + 4].is_ascii_alphanumeric() && bytes[i + 4] != b'_');
            if before_ok && after_ok {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

/// Extract the content inside a balanced `(...)` starting at `paren_pos`
/// (which must point at a `(`). Returns the inner text (excluding outer parens).
fn extract_balanced_parens(sql: &str, paren_pos: usize) -> Option<String> {
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

/// Find the position (within `over_content`) just after "PARTITION BY ".
/// Returns the offset of the first key character.
fn find_partition_by_in_over(over_content: &str) -> Option<usize> {
    let kw = "PARTITION BY ";
    let upper = over_content.to_uppercase();
    // Find at word boundary
    let mut i = 0;
    while let Some(pos) = upper[i..].find(kw) {
        let abs = i + pos;
        let before_ok = abs == 0 || !upper.as_bytes()[abs - 1].is_ascii_alphanumeric();
        if before_ok {
            return Some(abs + kw.len());
        }
        i = abs + 1;
    }
    // Also try without trailing space (for "PARTITION BY\n")
    let kw2 = "PARTITION BY";
    let mut i = 0;
    while let Some(pos) = upper[i..].find(kw2) {
        let abs = i + pos;
        let before_ok = abs == 0 || !upper.as_bytes()[abs - 1].is_ascii_alphanumeric();
        let after_pos = abs + kw2.len();
        let after_ok =
            after_pos >= upper.len() || !upper.as_bytes()[after_pos].is_ascii_alphanumeric();
        if before_ok && after_ok {
            // Skip trailing whitespace
            let skip = upper[after_pos..].len() - upper[after_pos..].trim_start().len();
            return Some(after_pos + skip);
        }
        i = abs + 1;
    }
    None
}

/// Trim a PARTITION BY key list to just the key portion, stopping before
/// ORDER BY, ROWS, RANGE, GROUPS, or end of text.
fn trim_to_window_clause_end(text: &str) -> &str {
    let upper = text.to_uppercase();
    let terminators = ["ORDER BY", "ROWS ", "RANGE ", "GROUPS "];
    let mut end = text.len();
    for term in &terminators {
        if let Some(pos) = upper.find(term) {
            if pos < end {
                end = pos;
            }
        }
    }
    &text[..end]
}

/// Check if a keyword appears at a word boundary in uppercase text.
fn has_keyword_at_boundary(upper_sql: &str, keyword: &str) -> bool {
    let bytes = upper_sql.as_bytes();
    let kw_bytes = keyword.as_bytes();

    for i in 0..bytes.len() {
        if i + kw_bytes.len() > bytes.len() {
            break;
        }
        if &bytes[i..i + kw_bytes.len()] == kw_bytes {
            let before_ok = i == 0 || !bytes[i - 1].is_ascii_alphanumeric();
            let after_ok = i + kw_bytes.len() >= bytes.len()
                || !bytes[i + kw_bytes.len()].is_ascii_alphanumeric();
            if before_ok && after_ok {
                return true;
            }
        }
    }
    false
}

/// Derive per-source bounds *and* their pushdown-depth injection point for an
/// incremental model, in a single downward walk over the model's SQL.
///
/// Builds a `BoundContext` from the model's upstream refs (only those with
/// `timeseries:` declared are included; lookup sources are skipped), then
/// derives each source's [`BoundResult`] from `derive_model_bounds` exactly
/// once. The same call that produces the bound also classifies its
/// [`InjectionPoint`] (`BoundResult::injection_point`), so the value used to
/// widen the per-source scan and the value that decides whether the outer
/// output clamp is still needed can never disagree — there is no second,
/// independent derivation of either window (research
/// `20260703-model-updates.md` §3.2/§3.5).
///
/// Also returns `Err(diagnostic)` when any bound is `NotDerivable` — that
/// triggers the same refusal path as a failed safety check (fail-closed,
/// unchanged from before this walk was unified).
pub fn derive_model_source_bounds(
    model: &ModelInfo,
    graph: &ModelGraph,
) -> Result<HashMap<String, (InjectionPoint, BoundResult)>, String> {
    // Build the context: map ref name → partition_column for timeseries refs.
    let mut ctx = BoundContext::new();
    for ref_name in &model.refs {
        if let Some(upstream) = graph.get(ref_name) {
            if let Some(ts) = &upstream.timeseries_config {
                ctx.add_source(ref_name, &ts.partition_column);
            }
            // Lookup sources (no timeseries) are silently skipped.
        }
    }

    let stripped = crate::types::Frontmatter::strip(&model.sql);
    let bounds = derive_model_bounds(stripped, &ctx);

    // Check for NotDerivable — refuse the model.
    for (source_name, bound) in &bounds {
        if *bound == BoundResult::NotDerivable {
            return Err(format!(
                "Model '{}': cannot derive a temporal bound for source '{}'. \
                 The SQL contains a window function (e.g. LAG/LEAD) without an explicit \
                 RANGE BETWEEN INTERVAL clause. Rewrite using Form A \
                 (RANGE BETWEEN INTERVAL '...' PRECEDING) or Form B \
                 (WHERE col BETWEEN expr - INTERVAL '...' AND expr) to make the \
                 lookback derivable, or remove the temporal dependency.",
                model.name, source_name
            ));
        }
    }

    Ok(bounds
        .into_iter()
        .map(|(source_name, bound)| {
            let injection_point = bound.injection_point();
            (source_name, (injection_point, bound))
        })
        .collect())
}

/// Produce a SetIncremental transformation for a model.
pub fn optimize(model: &ModelInfo) -> Result<Option<Transformation>, String> {
    let opportunity = detect(model)?;
    match opportunity {
        None => Ok(None),
        Some(opp) => match &opp.data {
            OpportunityData::Incremental {
                event_time_column,
                partition_column,
                granularity,
            } => Ok(Some(Transformation::SetIncremental {
                model: model.name.clone(),
                event_time_column: event_time_column.clone(),
                partition_column: partition_column.clone(),
                granularity: granularity.clone(),
            })),
            _ => Ok(None),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::TimeseriesConfig;
    use crate::types::{BatchedConfig, BatchedSafetyOverrides, Granularity};

    fn model(name: &str, sql: &str, partition_column: &str) -> ModelInfo {
        model_with_event_time(name, sql, partition_column, "event_timestamp")
    }

    fn model_with_event_time(
        name: &str,
        sql: &str,
        partition_column: &str,
        event_time_column: &str,
    ) -> ModelInfo {
        ModelInfo {
            name: name.to_string(),
            sql: sql.to_string(),
            refs: vec![],
            timeseries_config: Some(TimeseriesConfig {
                event_time_column: event_time_column.to_string(),
                partition_column: partition_column.to_string(),
                granularity: Granularity::Day,
                week_start: None,
            }),
            incremental_config: Some(BatchedConfig {
                unique_key: vec![],
                safety_overrides: BatchedSafetyOverrides::default(),
            }),
        }
    }

    fn model_with_overrides(
        name: &str,
        sql: &str,
        partition_column: &str,
        overrides: BatchedSafetyOverrides,
    ) -> ModelInfo {
        ModelInfo {
            name: name.to_string(),
            sql: sql.to_string(),
            refs: vec![],
            timeseries_config: Some(TimeseriesConfig {
                event_time_column: "event_timestamp".to_string(),
                partition_column: partition_column.to_string(),
                granularity: Granularity::Day,
                week_start: None,
            }),
            incremental_config: Some(BatchedConfig {
                unique_key: vec![],
                safety_overrides: overrides,
            }),
        }
    }

    #[test]
    fn test_detect_incremental() {
        let m = model(
            "daily",
            "SELECT date_trunc('day', event_timestamp) as event_date, user_id, COUNT(*) as cnt FROM events GROUP BY 1, 2",
            "event_date",
        );
        let opp = detect(&m).unwrap().unwrap();
        assert_eq!(opp.rule_name, "incremental");
        match opp.data {
            OpportunityData::Incremental {
                ref event_time_column,
                ref partition_column,
                ref granularity,
            } => {
                assert_eq!(event_time_column, "event_timestamp");
                assert_eq!(partition_column, "event_date");
                assert_eq!(granularity, &Granularity::Day);
            }
            _ => panic!("Expected Incremental data"),
        }
    }

    #[test]
    fn test_detect_with_explicit_event_time_column() {
        let m = model_with_event_time(
            "daily",
            "SELECT date_trunc('day', my_ts) as event_date, user_id, COUNT(*) as cnt FROM events GROUP BY 1, 2",
            "event_date",
            "my_ts",
        );
        let opp = detect(&m).unwrap().unwrap();
        match opp.data {
            OpportunityData::Incremental {
                ref event_time_column,
                ..
            } => {
                assert_eq!(event_time_column, "my_ts");
            }
            _ => panic!("Expected Incremental data"),
        }
    }

    #[test]
    fn test_detect_event_time_column_not_in_sql() {
        let m = model_with_event_time(
            "daily",
            "SELECT date_trunc('day', event_timestamp) as event_date, user_id, COUNT(*) as cnt FROM events GROUP BY 1, 2",
            "event_date",
            "nonexistent_column",
        );
        let result = detect(&m);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found in SQL"));
    }

    #[test]
    fn test_detect_with_granularity() {
        use crate::graph::TimeseriesConfig;
        let m = ModelInfo {
            name: "hourly".to_string(),
            sql: "SELECT date_trunc('hour', event_timestamp) as event_hour, COUNT(*) as cnt FROM events GROUP BY 1".to_string(),
            refs: vec![],
            timeseries_config: Some(TimeseriesConfig {
                event_time_column: "event_timestamp".to_string(),
                partition_column: "event_hour".to_string(),
                granularity: Granularity::Hour,
                week_start: None,
            }),
            incremental_config: Some(BatchedConfig {
                unique_key: vec![],
                safety_overrides: BatchedSafetyOverrides::default(),
            }),
        };
        let opp = detect(&m).unwrap().unwrap();
        match opp.data {
            OpportunityData::Incremental {
                ref granularity, ..
            } => {
                assert_eq!(granularity, &Granularity::Hour);
            }
            _ => panic!("Expected Incremental data"),
        }
    }

    #[test]
    fn test_detect_no_config() {
        let m = ModelInfo {
            name: "test".to_string(),
            sql: "SELECT a FROM t GROUP BY 1".to_string(),
            refs: vec![],
            timeseries_config: None,
            incremental_config: None,
        };
        assert!(detect(&m).unwrap().is_none());
    }

    #[test]
    fn test_detect_invalid_partition_column() {
        let m = model("test", "SELECT a FROM t GROUP BY 1", "nonexistent_column");
        let result = detect(&m);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found as alias"));
    }

    #[test]
    fn test_detect_rejects_window_functions() {
        let m = model(
            "windowed",
            "SELECT date_trunc('day', event_timestamp) as event_date, ROW_NUMBER() OVER (PARTITION BY user_id ORDER BY event_timestamp) as rn FROM events GROUP BY 1",
            "event_date",
        );
        let result = detect(&m);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("window function"));
    }

    #[test]
    fn test_detect_allows_window_functions_with_override() {
        let m = model_with_overrides(
            "windowed",
            "SELECT date_trunc('day', event_timestamp) as event_date, ROW_NUMBER() OVER (PARTITION BY user_id ORDER BY event_timestamp) as rn FROM events GROUP BY 1",
            "event_date",
            BatchedSafetyOverrides {
                allow_window_functions: true,
                ..Default::default()
            },
        );
        let result = detect(&m);
        assert!(result.is_ok());
        assert!(result.unwrap().is_some());
    }

    #[test]
    fn test_over_with_bounded_range_interval_frame_is_admissible() {
        // A window whose PARTITION BY does NOT include the partition_column is
        // still partition-local when it carries a bounded RANGE INTERVAL frame:
        // the Form A lookback widens the source read to cover the frame, so the
        // DELETE+INSERT contract holds. It must be admitted without an override.
        let sql = "SELECT MAX(boundary_ts) OVER (PARTITION BY device_id ORDER BY event_ts \
                   RANGE BETWEEN INTERVAL '1 day' PRECEDING AND CURRENT ROW) AS s FROM events";
        assert_eq!(find_inadmissible_over(sql, "session_start_date"), None);
    }

    #[test]
    fn test_over_without_frame_still_inadmissible() {
        // No frame, non-aligned PARTITION BY → still inadmissible (bare window
        // function reads unbounded prior rows within the partition).
        let sql =
            "SELECT ROW_NUMBER() OVER (PARTITION BY device_id ORDER BY event_ts) AS rn FROM events";
        assert!(find_inadmissible_over(sql, "session_start_date").is_some());
    }

    #[test]
    fn test_over_with_unbounded_range_still_inadmissible() {
        // An UNBOUNDED PRECEDING frame is genuinely cumulative-across-history and
        // is not made safe by the lookback widening — still inadmissible.
        let sql = "SELECT SUM(x) OVER (PARTITION BY device_id ORDER BY event_ts \
                   RANGE BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW) AS s FROM events";
        assert!(find_inadmissible_over(sql, "session_start_date").is_some());
    }

    #[test]
    fn test_detect_rejects_having() {
        let m = model(
            "having_model",
            "SELECT date_trunc('day', event_timestamp) as event_date, user_id, COUNT(*) as cnt FROM events GROUP BY 1, 2 HAVING COUNT(*) > 10",
            "event_date",
        );
        let result = detect(&m);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("HAVING"));
    }

    #[test]
    fn test_detect_allows_having_with_override() {
        let m = model_with_overrides(
            "having_model",
            "SELECT date_trunc('day', event_timestamp) as event_date, user_id, COUNT(*) as cnt FROM events GROUP BY 1, 2 HAVING COUNT(*) > 10",
            "event_date",
            BatchedSafetyOverrides {
                allow_having: true,
                ..Default::default()
            },
        );
        let result = detect(&m);
        assert!(result.is_ok());
        assert!(result.unwrap().is_some());
    }

    #[test]
    fn test_detect_rejects_limit() {
        let m = model(
            "limited",
            "SELECT date_trunc('day', event_timestamp) as event_date, user_id, COUNT(*) as cnt FROM events GROUP BY 1, 2 LIMIT 100",
            "event_date",
        );
        let result = detect(&m);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("LIMIT"));
    }

    #[test]
    fn test_detect_rejects_subquery() {
        let m = model(
            "subquery_model",
            "SELECT date_trunc('day', event_timestamp) as event_date, user_id, COUNT(*) as cnt FROM (SELECT * FROM events WHERE active = true) sub GROUP BY 1, 2",
            "event_date",
        );
        let result = detect(&m);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("subqueries"));
    }

    #[test]
    fn test_detect_rejects_nondeterministic() {
        let m = model(
            "random_model",
            "SELECT date_trunc('day', event_timestamp) as event_date, RANDOM() as r, COUNT(*) as cnt FROM events GROUP BY 1, 2",
            "event_date",
        );
        let result = detect(&m);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("non-deterministic"));
        assert!(err.contains("RANDOM"));
    }

    #[test]
    fn test_detect_rejects_select_distinct() {
        let m = model(
            "distinct_model",
            "SELECT DISTINCT date_trunc('day', event_timestamp) as event_date, user_id FROM events GROUP BY 1, 2",
            "event_date",
        );
        let result = detect(&m);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("SELECT DISTINCT"));
    }

    #[test]
    fn test_detect_allows_count_distinct() {
        // COUNT(DISTINCT ...) inside an aggregate should NOT trigger the DISTINCT check
        let m = model(
            "count_distinct_model",
            "SELECT date_trunc('day', event_timestamp) as event_date, COUNT(DISTINCT user_id) as unique_users FROM events GROUP BY 1",
            "event_date",
        );
        let result = detect(&m);
        assert!(result.is_ok());
        assert!(result.unwrap().is_some());
    }

    #[test]
    fn test_optimize_produces_transformation() {
        let m = model(
            "daily",
            "SELECT date_trunc('day', event_timestamp) as event_date, user_id, COUNT(*) as cnt FROM events GROUP BY 1, 2",
            "event_date",
        );
        let t = optimize(&m).unwrap().unwrap();
        match t {
            Transformation::SetIncremental {
                model,
                event_time_column,
                partition_column,
                granularity,
            } => {
                assert_eq!(model, "daily");
                assert_eq!(event_time_column, "event_timestamp");
                assert_eq!(partition_column, "event_date");
                assert_eq!(granularity, Granularity::Day);
            }
            _ => panic!("Expected SetIncremental"),
        }
    }

    #[test]
    fn test_detect_with_weekly_granularity() {
        use smelt_core::config::Weekday;
        let m = ModelInfo {
            name: "weekly".to_string(),
            sql: "SELECT date_trunc('week', event_timestamp) as event_week, user_id, COUNT(*) as cnt FROM events GROUP BY 1, 2".to_string(),
            refs: vec![],
            timeseries_config: Some(TimeseriesConfig {
                event_time_column: "event_timestamp".to_string(),
                partition_column: "event_week".to_string(),
                granularity: Granularity::Week,
                week_start: Some(Weekday::Monday),
            }),
            incremental_config: Some(BatchedConfig {
                unique_key: vec![],
                safety_overrides: BatchedSafetyOverrides::default(),
            }),
        };
        let opp = detect(&m).unwrap().unwrap();
        assert_eq!(opp.rule_name, "incremental");
        match opp.data {
            OpportunityData::Incremental {
                ref granularity, ..
            } => {
                assert_eq!(granularity, &Granularity::Week);
            }
            _ => panic!("Expected Incremental data"),
        }
    }

    // --- Per-row (non-aggregate) model tests ---

    /// A per-row SELECT (no GROUP BY) with `partition_column` in the SELECT
    /// list must be accepted by `detect()`.  The Phase 2 fix skips the
    /// GROUP-BY membership check when `group_by_exprs` is empty.
    #[test]
    fn test_detect_per_row_model_no_group_by() {
        let m = model_with_event_time(
            "eventstream",
            "SELECT event_date, user_id, amount FROM smelt.upstream WHERE event_date >= start_date",
            "event_date",
            "event_date",
        );
        let result = detect(&m);
        assert!(
            result.is_ok(),
            "per-row model (no GROUP BY) must not error; got: {:?}",
            result
        );
        assert!(
            result.unwrap().is_some(),
            "per-row model (no GROUP BY) must be detected as incremental"
        );
    }

    /// Negative companion: a per-row SELECT whose SELECT list does **not**
    /// contain the `partition_column` alias must still return an error.
    #[test]
    fn test_detect_per_row_model_missing_partition_column() {
        let m = model_with_event_time(
            "eventstream_bad",
            "SELECT user_id, amount FROM smelt.upstream WHERE event_date >= start_date",
            "event_date",
            "event_date",
        );
        let result = detect(&m);
        assert!(
            result.is_err(),
            "per-row model missing partition_column in SELECT must error; got: {:?}",
            result
        );
        assert!(
            result.unwrap_err().contains("not found as alias"),
            "error must name the missing partition_column alias"
        );
    }

    #[test]
    fn test_detect_with_valid_unique_key() {
        let m = ModelInfo {
            name: "daily".to_string(),
            sql: "SELECT date_trunc('day', event_timestamp) as event_date, user_id, COUNT(*) as cnt FROM events GROUP BY 1, 2".to_string(),
            refs: vec![],
            timeseries_config: Some(TimeseriesConfig {
                event_time_column: "event_timestamp".to_string(),
                partition_column: "event_date".to_string(),
                granularity: Granularity::Day,
                week_start: None,
            }),
            incremental_config: Some(BatchedConfig {
                unique_key: vec!["event_date".to_string(), "user_id".to_string()],
                safety_overrides: BatchedSafetyOverrides::default(),
            }),
        };
        let result = detect(&m);
        assert!(result.is_ok());
        assert!(result.unwrap().is_some());
    }

    #[test]
    fn test_detect_rejects_invalid_unique_key() {
        let m = ModelInfo {
            name: "daily".to_string(),
            sql: "SELECT date_trunc('day', event_timestamp) as event_date, user_id, COUNT(*) as cnt FROM events GROUP BY 1, 2".to_string(),
            refs: vec![],
            timeseries_config: Some(TimeseriesConfig {
                event_time_column: "event_timestamp".to_string(),
                partition_column: "event_date".to_string(),
                granularity: Granularity::Day,
                week_start: None,
            }),
            incremental_config: Some(BatchedConfig {
                unique_key: vec!["nonexistent_col".to_string()],
                safety_overrides: BatchedSafetyOverrides::default(),
            }),
        };
        let result = detect(&m);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .contains("unique_key column 'nonexistent_col' not found"));
    }

    // --- BatchSafety tests ---

    #[test]
    fn test_batch_safety_simple_aggregate() {
        // Pure GROUP BY with no window functions → fully batch safe
        let m = model(
            "daily",
            "SELECT date_trunc('day', event_timestamp) as event_date, user_id, COUNT(*) as cnt FROM events GROUP BY 1, 2",
            "event_date",
        );
        let safety = analyze_batch_safety(&m);
        assert_eq!(safety, BatchSafety::FullyBatchSafe);
    }

    #[test]
    fn test_batch_safety_lag_function() {
        // LAG(col, 3) → bounded lookback of 3 periods
        let m = model_with_overrides(
            "lagged",
            "SELECT user_id, event_timestamp, LAG(amount, 3) OVER (ORDER BY event_timestamp) as prev FROM events",
            "event_date",
            BatchedSafetyOverrides {
                allow_window_functions: true,
                ..Default::default()
            },
        );
        let safety = analyze_batch_safety(&m);
        match safety {
            BatchSafety::BoundedSafe { context_days, .. } => {
                assert!(
                    context_days >= 3,
                    "expected context >= 3, got {}",
                    context_days
                );
            }
            other => panic!("Expected BoundedSafe, got {:?}", other),
        }
    }

    #[test]
    fn test_batch_safety_unbounded_window() {
        // UNBOUNDED PRECEDING with RANGE → per partition only
        let m = model_with_overrides(
            "running",
            "SELECT user_id, event_timestamp, SUM(amount) OVER (ORDER BY event_timestamp RANGE BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW) as running FROM events",
            "event_date",
            BatchedSafetyOverrides {
                allow_window_functions: true,
                ..Default::default()
            },
        );
        let safety = analyze_batch_safety(&m);
        match safety {
            BatchSafety::PerPartitionOnly { reason } => {
                assert!(reason.contains("unbounded"), "reason: {}", reason);
            }
            other => panic!("Expected PerPartitionOnly, got {:?}", other),
        }
    }

    #[test]
    fn test_batch_safety_no_incremental_config() {
        // No incremental config → still fully batch safe (uses default period_days=1)
        let m = ModelInfo {
            name: "plain".to_string(),
            sql: "SELECT a, SUM(b) FROM t GROUP BY 1".to_string(),
            refs: vec![],
            timeseries_config: None,
            incremental_config: None,
        };
        let safety = analyze_batch_safety(&m);
        assert_eq!(safety, BatchSafety::FullyBatchSafe);
    }

    // --- Partition-aligned OVER admissibility tests (Phase 3) ---

    /// A window function OVER (PARTITION BY device_id, session_seq) on a model
    /// whose partition_column is `device_id` should be admissible (superset rule).
    /// `{device_id, session_seq} ⊇ {device_id}` → admitted.
    #[test]
    fn test_admissible_over_partition_by_superset() {
        // partition_column = device_id; OVER uses (device_id, session_seq) — superset
        // Per-row model (no GROUP BY) so we just need partition_col in SELECT.
        let m = model_with_event_time(
            "sessions",
            "SELECT device_id, session_seq, event_date, \
             FIRST_VALUE(event_date) OVER (PARTITION BY device_id, session_seq ORDER BY event_ts) AS session_start_date \
             FROM smelt.silver.events_parsed WHERE event_date >= start_date",
            "device_id",
            "event_date",
        );
        let result = detect(&m);
        assert!(
            result.is_ok(),
            "partition-aligned OVER (superset) must not be rejected; got: {:?}",
            result
        );
        assert!(
            result.unwrap().is_some(),
            "partition-aligned OVER (superset) model must classify as incremental"
        );
    }

    /// A window function OVER (PARTITION BY user_id) on a model whose
    /// partition_column is `device_id` should be rejected — the PARTITION BY
    /// keys `{user_id}` do not contain `device_id`.
    #[test]
    fn test_inadmissible_over_partition_by_disjoint() {
        // partition_column = device_id; OVER uses (user_id) — disjoint
        let m = model_with_event_time(
            "wrong_window",
            "SELECT device_id, event_date, \
             ROW_NUMBER() OVER (PARTITION BY user_id ORDER BY event_ts) AS rn \
             FROM smelt.silver.events_parsed WHERE event_date >= start_date",
            "device_id",
            "event_date",
        );
        let result = detect(&m);
        assert!(
            result.is_err(),
            "OVER with disjoint PARTITION BY must be rejected; got: {:?}",
            result
        );
        let err = result.unwrap_err();
        assert!(
            err.contains("window function"),
            "error must mention window function; got: {}",
            err
        );
    }

    // --- B0: unified pushdown-depth walk (derive_model_source_bounds) ---

    /// The confirmed §3.2 under-read harness: a model with a genuine bounded
    /// lookback (`RANGE BETWEEN INTERVAL '2 days' PRECEDING`) derives its
    /// write-window margin and its per-source scan margin from the *same*
    /// walk — there is only one `derive_model_bounds` call feeding both, so
    /// the two numbers cannot independently disagree. The derived `(before,
    /// after)` must match the frame's `INTERVAL '2 days'`, and the injection
    /// point must be `OuterClamp` (a real lookback keeps both layers).
    #[test]
    fn test_unified_walk_matches_frame_interval() {
        let mut graph = ModelGraph::new();
        graph.add_model(ModelInfo {
            name: "upstream".to_string(),
            sql: String::new(),
            refs: vec![],
            timeseries_config: Some(TimeseriesConfig {
                event_time_column: "event_ts".to_string(),
                partition_column: "event_date".to_string(),
                granularity: Granularity::Day,
                week_start: None,
            }),
            incremental_config: None,
        });

        let m = ModelInfo {
            name: "windowed".to_string(),
            sql: "SELECT device_id, event_date, \
                  SUM(amount) OVER (PARTITION BY device_id ORDER BY event_ts \
                  RANGE BETWEEN INTERVAL '2 days' PRECEDING AND CURRENT ROW) AS running \
                  FROM upstream"
                .to_string(),
            refs: vec!["upstream".to_string()],
            timeseries_config: Some(TimeseriesConfig {
                event_time_column: "event_ts".to_string(),
                partition_column: "event_date".to_string(),
                granularity: Granularity::Day,
                week_start: None,
            }),
            incremental_config: Some(BatchedConfig {
                unique_key: vec![],
                safety_overrides: BatchedSafetyOverrides::default(),
            }),
        };

        let bounds = derive_model_source_bounds(&m, &graph).expect("bound must be derivable");
        let (injection_point, bound) = bounds.get("upstream").expect("upstream must have a bound");
        match bound {
            BoundResult::Bounded { before, after, .. } => {
                assert_eq!(
                    *before,
                    crate::analysis::source_bounds::Seconds::days(2),
                    "derived before-margin must match the frame's INTERVAL '2 days'"
                );
                assert_eq!(*after, crate::analysis::source_bounds::Seconds::ZERO);
            }
            other => panic!("Expected Bounded, got {:?}", other),
        }
        assert_eq!(
            *injection_point,
            InjectionPoint::OuterClamp,
            "a genuine lookback margin must keep the outer clamp layer"
        );
    }

    /// A transparent single-source subquery (no lookback: no INTERVAL,
    /// no window function) derives a zero-margin bound and classifies as
    /// `InjectionPoint::Source` — the source-level filter alone is both the
    /// scan-pruning filter and the exact output clamp; no outer wrap.
    #[test]
    fn test_transparent_single_source_pushes_to_source_no_outer_wrap() {
        let mut graph = ModelGraph::new();
        graph.add_model(ModelInfo {
            name: "upstream".to_string(),
            sql: String::new(),
            refs: vec![],
            timeseries_config: Some(TimeseriesConfig {
                event_time_column: "event_date".to_string(),
                partition_column: "event_date".to_string(),
                granularity: Granularity::Day,
                week_start: None,
            }),
            incremental_config: None,
        });

        let m = ModelInfo {
            name: "passthrough".to_string(),
            sql: "SELECT event_date, user_id, amount FROM upstream WHERE amount > 0".to_string(),
            refs: vec!["upstream".to_string()],
            timeseries_config: Some(TimeseriesConfig {
                event_time_column: "event_date".to_string(),
                partition_column: "event_date".to_string(),
                granularity: Granularity::Day,
                week_start: None,
            }),
            incremental_config: Some(BatchedConfig {
                unique_key: vec![],
                safety_overrides: BatchedSafetyOverrides::default(),
            }),
        };

        let bounds = derive_model_source_bounds(&m, &graph).expect("bound must be derivable");
        assert_eq!(bounds.len(), 1, "exactly one source-level filter expected");
        let (injection_point, bound) = bounds.get("upstream").expect("upstream must have a bound");
        assert_eq!(
            *bound,
            BoundResult::Bounded {
                source_partition_col: "event_date".to_string(),
                before: crate::analysis::source_bounds::Seconds::ZERO,
                after: crate::analysis::source_bounds::Seconds::ZERO,
            }
        );
        assert_eq!(
            *injection_point,
            InjectionPoint::Source,
            "transparent slice must push to the source with no outer wrap"
        );
    }

    /// Fail-closed (unchanged by B0): a `NotDerivable` source (bare LAG/LEAD
    /// with no RANGE frame) still refuses at planning time, naming the
    /// construct.
    #[test]
    fn test_not_derivable_still_refuses() {
        let mut graph = ModelGraph::new();
        graph.add_model(ModelInfo {
            name: "upstream".to_string(),
            sql: String::new(),
            refs: vec![],
            timeseries_config: Some(TimeseriesConfig {
                event_time_column: "event_ts".to_string(),
                partition_column: "event_date".to_string(),
                granularity: Granularity::Day,
                week_start: None,
            }),
            incremental_config: None,
        });

        let m = ModelInfo {
            name: "bare_lag".to_string(),
            sql: "SELECT device_id, event_date, LAG(amount) OVER (PARTITION BY device_id ORDER BY event_ts) AS prev \
                  FROM upstream"
                .to_string(),
            refs: vec!["upstream".to_string()],
            timeseries_config: Some(TimeseriesConfig {
                event_time_column: "event_ts".to_string(),
                partition_column: "event_date".to_string(),
                granularity: Granularity::Day,
                week_start: None,
            }),
            incremental_config: Some(BatchedConfig {
                unique_key: vec![],
                safety_overrides: BatchedSafetyOverrides {
                    allow_window_functions: true,
                    ..Default::default()
                },
            }),
        };

        let result = derive_model_source_bounds(&m, &graph);
        assert!(result.is_err(), "bare LAG without RANGE must refuse");
        let err = result.unwrap_err();
        assert!(
            err.contains("cannot derive a temporal bound"),
            "error must name the derivation failure: {}",
            err
        );
    }

    /// A window function OVER (PARTITION BY event_date) on a model whose
    /// partition_column is `event_date` should be admissible — equality is a
    /// superset too: `{event_date} ⊇ {event_date}`.
    #[test]
    fn test_admissible_over_partition_by_equals() {
        // partition_column = event_date; OVER uses (event_date) — exact equality
        let m = model_with_event_time(
            "daily_windowed",
            "SELECT event_date, user_id, \
             FIRST_VALUE(user_id) OVER (PARTITION BY event_date ORDER BY event_ts) AS first_user \
             FROM smelt.silver.events_parsed WHERE event_date >= start_date",
            "event_date",
            "event_date",
        );
        let result = detect(&m);
        assert!(
            result.is_ok(),
            "partition-aligned OVER (equality) must not be rejected; got: {:?}",
            result
        );
        assert!(
            result.unwrap().is_some(),
            "partition-aligned OVER (equality) model must classify as incremental"
        );
    }
}
