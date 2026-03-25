use serde::Serialize;

use crate::analysis::temporal::{analyze_temporal_dependencies, TemporalOffset};
use crate::analysis::{analyze_select, SelectItemKind};
use crate::graph::ModelInfo;
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
        .incremental_config
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
            eprintln!(
                "Note: ideal chunk size ({} days, 3x context of {} days) exceeds 90-day cap. \
                 Using 90-day chunks. Override with --batch-size if needed.",
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

    let analysis = analyze_select(&model.sql).ok_or_else(|| {
        format!(
            "Model '{}': has incremental config but SQL could not be parsed",
            model.name
        )
    })?;

    let partition_col = &inc_config.partition_column;
    let event_time_column = &inc_config.event_time_column;
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

    // Validate it appears in GROUP BY
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
    if !overrides.allow_window_functions {
        let upper_sql = stripped_sql.to_uppercase();
        if upper_sql.contains("OVER(") || upper_sql.contains("OVER (") {
            return Err(format!(
                "Model '{}': window functions (OVER clause) are not compatible with incremental \
                 materialization — they may produce different results on partial data",
                model.name
            ));
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
            partition_col, event_time_column, inc_config.granularity,
        ),
        data: OpportunityData::Incremental {
            event_time_column: event_time_column.clone(),
            partition_column: partition_col.clone(),
            granularity: inc_config.granularity.clone(),
        },
    }))
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
    use crate::types::{Granularity, IncrementalConfig, IncrementalSafetyOverrides, Weekday};

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
            incremental_config: Some(IncrementalConfig {
                enabled: true,
                partition_column: partition_column.to_string(),
                event_time_column: event_time_column.to_string(),
                granularity: Granularity::Day,
                unique_key: vec![],
                safety_overrides: IncrementalSafetyOverrides::default(),
            }),
        }
    }

    fn model_with_overrides(
        name: &str,
        sql: &str,
        partition_column: &str,
        overrides: IncrementalSafetyOverrides,
    ) -> ModelInfo {
        ModelInfo {
            name: name.to_string(),
            sql: sql.to_string(),
            refs: vec![],
            incremental_config: Some(IncrementalConfig {
                enabled: true,
                partition_column: partition_column.to_string(),
                event_time_column: "event_timestamp".to_string(),
                granularity: Granularity::Day,
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
        let m = ModelInfo {
            name: "hourly".to_string(),
            sql: "SELECT date_trunc('hour', event_timestamp) as event_hour, COUNT(*) as cnt FROM events GROUP BY 1".to_string(),
            refs: vec![],
            incremental_config: Some(IncrementalConfig {
                enabled: true,
                partition_column: "event_hour".to_string(),
                event_time_column: "event_timestamp".to_string(),
                granularity: Granularity::Hour,
                unique_key: vec![],
                safety_overrides: IncrementalSafetyOverrides::default(),
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
        assert!(result.unwrap_err().contains("window functions"));
    }

    #[test]
    fn test_detect_allows_window_functions_with_override() {
        let m = model_with_overrides(
            "windowed",
            "SELECT date_trunc('day', event_timestamp) as event_date, ROW_NUMBER() OVER (PARTITION BY user_id ORDER BY event_timestamp) as rn FROM events GROUP BY 1",
            "event_date",
            IncrementalSafetyOverrides {
                allow_window_functions: true,
                ..Default::default()
            },
        );
        let result = detect(&m);
        assert!(result.is_ok());
        assert!(result.unwrap().is_some());
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
            IncrementalSafetyOverrides {
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
        let m = ModelInfo {
            name: "weekly".to_string(),
            sql: "SELECT date_trunc('week', event_timestamp) as event_week, user_id, COUNT(*) as cnt FROM events GROUP BY 1, 2".to_string(),
            refs: vec![],
            incremental_config: Some(IncrementalConfig {
                enabled: true,
                partition_column: "event_week".to_string(),
                event_time_column: "event_timestamp".to_string(),
                granularity: Granularity::Week {
                    week_start: Weekday::Monday,
                },
                unique_key: vec![],
                safety_overrides: IncrementalSafetyOverrides::default(),
            }),
        };
        let opp = detect(&m).unwrap().unwrap();
        assert_eq!(opp.rule_name, "incremental");
        match opp.data {
            OpportunityData::Incremental {
                ref granularity, ..
            } => {
                assert_eq!(
                    granularity,
                    &Granularity::Week {
                        week_start: Weekday::Monday
                    }
                );
            }
            _ => panic!("Expected Incremental data"),
        }
    }

    #[test]
    fn test_detect_with_valid_unique_key() {
        let m = ModelInfo {
            name: "daily".to_string(),
            sql: "SELECT date_trunc('day', event_timestamp) as event_date, user_id, COUNT(*) as cnt FROM events GROUP BY 1, 2".to_string(),
            refs: vec![],
            incremental_config: Some(IncrementalConfig {
                enabled: true,
                partition_column: "event_date".to_string(),
                event_time_column: "event_timestamp".to_string(),
                granularity: Granularity::Day,
                unique_key: vec!["event_date".to_string(), "user_id".to_string()],
                safety_overrides: IncrementalSafetyOverrides::default(),
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
            incremental_config: Some(IncrementalConfig {
                enabled: true,
                partition_column: "event_date".to_string(),
                event_time_column: "event_timestamp".to_string(),
                granularity: Granularity::Day,
                unique_key: vec!["nonexistent_col".to_string()],
                safety_overrides: IncrementalSafetyOverrides::default(),
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
            IncrementalSafetyOverrides {
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
            IncrementalSafetyOverrides {
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
            incremental_config: None,
        };
        let safety = analyze_batch_safety(&m);
        assert_eq!(safety, BatchSafety::FullyBatchSafe);
    }
}
