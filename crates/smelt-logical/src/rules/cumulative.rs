//! Classifier for the `cumulative_aggregate` materialization.
//!
//! See `docs/specs/cumulative_aggregate.md` for the normative spec.
//!
//! The classifier is a pure function that reads an inlined SELECT
//! (post function expansion) plus a small source-timeseries lookup
//! and derives:
//!
//! - `unique_key` — the GROUP BY column list.
//! - `aggregator_columns` — per non-key projection, the
//!   `(per_partition_agg, cross_partition_combiner)` pair from a
//!   fixed allowlist.
//! - `driving_source` — the single timeseries-tagged source in the
//!   FROM clause.
//!
//! Returns a `CumulativeClassification` on success or a list of
//! `CumulativeDiagnostic`s on rejection.

use serde::Serialize;
use smelt_core::config::TimeseriesConfig;
use std::collections::HashMap;

use crate::analysis::monotonicity::NONDETERMINISTIC_FUNCTIONS;
use crate::analysis::source_bounds::{resolve_single_anchor, AnchorAmbiguity};
use crate::analysis::{analyze_select, SelectItemKind};
use smelt_types::SqlFunction;

/// A per-partition aggregator paired with its cross-partition combiner.
///
/// The combiner is a fixed lookup off the per-partition aggregator —
/// `COUNT → SUM`, everything else folds onto itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AggregatorColumn {
    /// The output column name (the projection's `AS` alias).
    pub output_name: String,
    /// The SQL aggregate function called on the source rows for one partition.
    pub per_partition_agg: String,
    /// The SQL aggregate function that combines the target's value with
    /// the new partition's value. Stored as a name; the rule renders it
    /// as `f(target.col, delta.col)` or the equivalent (e.g. `LEAST`/`GREATEST`).
    pub cross_partition_combiner: CrossPartitionCombiner,
}

/// How target and delta values combine for a column on `MERGE`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum CrossPartitionCombiner {
    /// `target.c + delta.c` — additive.
    Sum,
    /// `LEAST(target.c, delta.c)` — minimum.
    Min,
    /// `GREATEST(target.c, delta.c)` — maximum.
    Max,
    /// `target.c AND delta.c`.
    BoolAnd,
    /// `target.c OR delta.c`.
    BoolOr,
    /// `target.c & delta.c`.
    BitAnd,
    /// `target.c | delta.c`.
    BitOr,
    /// `xor(target.c, delta.c)` — function form is DuckDB-compatible and works
    /// in Postgres as well; the `#` infix operator is Postgres-only.
    BitXor,
}

impl CrossPartitionCombiner {
    /// Render the SQL expression that produces the merged value for one column.
    pub fn render(&self, target_col: &str, delta_col: &str) -> String {
        match self {
            CrossPartitionCombiner::Sum => format!("{} + {}", target_col, delta_col),
            CrossPartitionCombiner::Min => format!("LEAST({}, {})", target_col, delta_col),
            CrossPartitionCombiner::Max => format!("GREATEST({}, {})", target_col, delta_col),
            CrossPartitionCombiner::BoolAnd => format!("{} AND {}", target_col, delta_col),
            CrossPartitionCombiner::BoolOr => format!("{} OR {}", target_col, delta_col),
            CrossPartitionCombiner::BitAnd => format!("{} & {}", target_col, delta_col),
            CrossPartitionCombiner::BitOr => format!("{} | {}", target_col, delta_col),
            CrossPartitionCombiner::BitXor => format!("xor({}, {})", target_col, delta_col),
        }
    }
}

/// Lookup a per-partition aggregator name and return its cross-partition
/// combiner. The gate is the shared algebraic-discriminants classifier
/// (`analysis::discriminants::combiner_discriminants`) — only a monoid
/// combiner is admitted; `None` means the aggregator is not a monoid (either
/// holistic, e.g. `AVG`/`STRING_AGG`, or unrecognised).
pub fn combiner_for(agg_name: &str) -> Option<CrossPartitionCombiner> {
    let function = SqlFunction::from_name(agg_name)?;
    let discriminants = crate::analysis::discriminants::combiner_discriminants(function, false);
    if !discriminants.is_monoid {
        return None;
    }
    match function {
        SqlFunction::Count | SqlFunction::Sum => Some(CrossPartitionCombiner::Sum),
        SqlFunction::Min => Some(CrossPartitionCombiner::Min),
        SqlFunction::Max => Some(CrossPartitionCombiner::Max),
        SqlFunction::BoolAnd => Some(CrossPartitionCombiner::BoolAnd),
        SqlFunction::BoolOr => Some(CrossPartitionCombiner::BoolOr),
        SqlFunction::BitAnd => Some(CrossPartitionCombiner::BitAnd),
        SqlFunction::BitOr => Some(CrossPartitionCombiner::BitOr),
        SqlFunction::BitXor => Some(CrossPartitionCombiner::BitXor),
        _ => None,
    }
}

/// A diagnostic code emitted by the cumulative classifier.
///
/// Mirrors `cumulative_aggregate.md` §"Diagnostic codes".
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum CumulativeDiagnostic {
    CumulativeRequiresGroupBy,
    CumulativeUnknownAggregator {
        projection: String,
        offending: String,
    },
    CumulativeGroupByContainsPartitionColumn {
        partition_column: String,
    },
    CumulativeForbidsWindowFunctions,
    CumulativeForbidsNondeterministic {
        offending: String,
    },
    CumulativeNoDrivingSource,
    CumulativeMultipleDrivingSources {
        candidates: Vec<String>,
    },
    SqlNotParseable,
}

impl std::fmt::Display for CumulativeDiagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CumulativeDiagnostic::CumulativeRequiresGroupBy => write!(
                f,
                "CumulativeRequiresGroupBy: cumulative_aggregate SELECT must have a GROUP BY \
                 clause — the GROUP BY columns are the unique key"
            ),
            CumulativeDiagnostic::CumulativeUnknownAggregator {
                projection,
                offending,
            } => write!(
                f,
                "CumulativeUnknownAggregator: projection `{}` uses `{}`, which is not in the \
                 cumulative aggregator allowlist (COUNT, SUM, MIN, MAX, BOOL_AND, BOOL_OR, \
                 BIT_AND, BIT_OR, BIT_XOR). Composite expressions over aggregates are not \
                 allowed — split into separate projections.",
                projection, offending
            ),
            CumulativeDiagnostic::CumulativeGroupByContainsPartitionColumn { partition_column } => {
                write!(
                    f,
                    "CumulativeGroupByContainsPartitionColumn: the GROUP BY contains the driving \
                     source's partition_column `{}`, which produces a per-partition output shape, \
                     not the cumulative one — switch to `materialization: incremental` + \
                     `timeseries:` instead",
                    partition_column
                )
            }
            CumulativeDiagnostic::CumulativeForbidsWindowFunctions => write!(
                f,
                "CumulativeForbidsWindowFunctions: window functions (OVER (...)) are not allowed \
                 in cumulative_aggregate SELECTs — the cumulative state is the window"
            ),
            CumulativeDiagnostic::CumulativeForbidsNondeterministic { offending } => write!(
                f,
                "CumulativeForbidsNondeterministic: non-deterministic function `{}` is not \
                 allowed in cumulative_aggregate SELECTs — cross-partition combine requires \
                 deterministic per-partition output",
                offending
            ),
            CumulativeDiagnostic::CumulativeNoDrivingSource => write!(
                f,
                "CumulativeNoDrivingSource: no source in the FROM clause declares a `timeseries:` \
                 block — declare `timeseries:` on the source, or choose a different materialization"
            ),
            CumulativeDiagnostic::CumulativeMultipleDrivingSources { candidates } => write!(
                f,
                "CumulativeMultipleDrivingSources: multiple timeseries-tagged sources in the \
                 FROM clause ({}). v1 supports exactly one driving source.",
                candidates.join(", ")
            ),
            CumulativeDiagnostic::SqlNotParseable => write!(
                f,
                "SQL body could not be parsed for cumulative classification"
            ),
        }
    }
}

/// The result of classifying a `cumulative_aggregate` model.
#[derive(Debug, Clone, Serialize)]
pub struct CumulativeClassification {
    /// Columns from the GROUP BY list. The order matches the SELECT's
    /// GROUP BY ordering.
    pub unique_key: Vec<String>,
    /// Non-key projections with their derived combiners.
    pub aggregator_columns: Vec<AggregatorColumn>,
    /// The single timeseries-tagged source the rule iterates over.
    /// `name` is the model/source name as it appears in `smelt.<path>`
    /// references; `timeseries` is the source's declared partition shape.
    pub driving_source: DrivingSource,
}

#[derive(Debug, Clone, Serialize)]
pub struct DrivingSource {
    pub name: String,
    pub timeseries: TimeseriesConfig,
}

/// Lookup table for a source's `timeseries:` declaration. The classifier
/// uses this to identify the driving source and to enforce
/// `CumulativeGroupByContainsPartitionColumn`.
pub type SourceTimeseriesMap = HashMap<String, TimeseriesConfig>;

/// Classify a `cumulative_aggregate` model.
///
/// `sql` is the inlined model SQL (post function expansion). `refs` is the
/// list of `smelt.<path>` references discovered in the FROM clause. Source
/// timeseries declarations are looked up via `source_timeseries`.
///
/// Returns the classification on success, or a vector of diagnostics
/// describing every classifier rejection (the function does not short-circuit
/// on the first error — it surfaces every problem it can detect).
pub fn classify_cumulative(
    sql: &str,
    refs: &[String],
    source_timeseries: &SourceTimeseriesMap,
) -> Result<CumulativeClassification, Vec<CumulativeDiagnostic>> {
    let mut diagnostics = Vec::new();

    let analysis = match analyze_select(sql) {
        Some(a) => a,
        None => {
            return Err(vec![CumulativeDiagnostic::SqlNotParseable]);
        }
    };

    // Rule: GROUP BY required.
    if analysis.group_by_exprs.is_empty() {
        diagnostics.push(CumulativeDiagnostic::CumulativeRequiresGroupBy);
    }

    // Build the unique_key as the SELECT aliases corresponding to GROUP BY
    // expressions. Each GROUP BY expression is matched to a projection by
    // textual identity (the analyser already resolves ordinals).
    let mut unique_key: Vec<String> = Vec::new();
    for group_expr in &analysis.group_by_exprs {
        // Find the matching projection by expression text.
        let matched = analysis.items.iter().find_map(|item| match item {
            SelectItemKind::GroupByKey { text, alias, .. } if text == group_expr => {
                Some(alias.clone())
            }
            _ => None,
        });
        if let Some(alias) = matched {
            unique_key.push(alias);
        } else {
            // GROUP BY expression has no matching non-aggregate projection —
            // fall back to the raw expression text. This is unusual but
            // not necessarily fatal (e.g. an expression-based GROUP BY).
            unique_key.push(group_expr.clone());
        }
    }

    // Walk the projection list. Non-key projections must be allowlisted
    // aggregator calls. GroupByKey items are the key columns.
    let mut aggregator_columns: Vec<AggregatorColumn> = Vec::new();
    for item in &analysis.items {
        match item {
            SelectItemKind::GroupByKey { text, alias, .. } => {
                // A "GroupByKey" item is the analyser's classification for
                // any non-aggregate expression. If the projection's text
                // appears in the GROUP BY, it is genuinely a key column.
                // Otherwise it is a composite expression — possibly one
                // that wraps an aggregate (`SUM(x) + 1`) — and is not
                // permitted in a cumulative SELECT.
                let in_group_by = analysis.group_by_exprs.iter().any(|g| g == text);
                if !in_group_by {
                    diagnostics.push(CumulativeDiagnostic::CumulativeUnknownAggregator {
                        projection: alias.clone(),
                        offending: format!("composite expression `{}`", text.trim()),
                    });
                }
            }
            SelectItemKind::CountDistinct { alias, .. } => {
                // COUNT(DISTINCT x) is not commutative under merge (the union
                // of distinct values across partitions cannot be reconstructed
                // from per-partition counts). Refuse.
                diagnostics.push(CumulativeDiagnostic::CumulativeUnknownAggregator {
                    projection: alias.clone(),
                    offending: "COUNT(DISTINCT)".to_string(),
                });
            }
            SelectItemKind::OtherAggregate {
                text, alias, expr, ..
            } => {
                // Extract the outer function name from the already-parsed
                // expression (no string re-parse) and confirm it against the
                // typed aggregate classifier — the same
                // `SqlFunction::is_aggregate` predicate `analysis::mod` used
                // to classify this item as `OtherAggregate` in the first
                // place.
                let agg_name = expr
                    .as_function_call()
                    .and_then(|f| f.name())
                    .map(|n| n.to_ascii_uppercase())
                    .filter(|n| SqlFunction::from_name(n).is_some_and(|f| f.is_aggregate()));
                let combiner = agg_name.as_deref().and_then(combiner_for);
                match (agg_name, combiner) {
                    (Some(agg_upper), Some(combiner)) => {
                        // Verify the projection is a *direct* call — no
                        // composition like `SUM(x) + 1`. We detect this by
                        // checking that the projection text starts with the
                        // function name and ends with `)`, ignoring whitespace.
                        if is_direct_function_call(text, &agg_upper) {
                            aggregator_columns.push(AggregatorColumn {
                                output_name: alias.clone(),
                                per_partition_agg: agg_upper,
                                cross_partition_combiner: combiner,
                            });
                        } else {
                            diagnostics.push(CumulativeDiagnostic::CumulativeUnknownAggregator {
                                projection: alias.clone(),
                                offending: format!("composite expression `{}`", text.trim()),
                            });
                        }
                    }
                    (Some(name), None) => {
                        diagnostics.push(CumulativeDiagnostic::CumulativeUnknownAggregator {
                            projection: alias.clone(),
                            offending: name,
                        });
                    }
                    (None, _) => {
                        diagnostics.push(CumulativeDiagnostic::CumulativeUnknownAggregator {
                            projection: alias.clone(),
                            offending: text.clone(),
                        });
                    }
                }
            }
        }
    }

    // Rule: no window functions in the outer body.
    let upper_sql = sql.to_uppercase();
    if upper_sql.contains("OVER(") || upper_sql.contains("OVER (") {
        diagnostics.push(CumulativeDiagnostic::CumulativeForbidsWindowFunctions);
    }

    // Rule: no non-deterministic functions in the outer body.
    for nd in NONDETERMINISTIC_FUNCTIONS {
        let pattern = format!("{}(", nd);
        if upper_sql.contains(&pattern) {
            diagnostics.push(CumulativeDiagnostic::CumulativeForbidsNondeterministic {
                offending: nd.to_string(),
            });
            break;
        }
    }

    // Find the driving source: the single alias-scoped FROM/JOIN input that
    // is both a collected ref and registered with a `timeseries:` block —
    // the shared anchor resolver (`resolve_single_anchor`) also used by
    // `resolve_join_driving_fact`'s alias-scoped monotonicity trace.
    let alias_sources: Vec<(String, String)> =
        smelt_parser::File::cast(smelt_parser::parse(sql).syntax())
            .and_then(|file| file.select_stmt())
            .and_then(|select| select.from_clause())
            .map(|from_clause| {
                crate::analysis::source_bounds::from_clause_alias_sources(&from_clause)
            })
            .unwrap_or_default();

    let driving_source = match resolve_single_anchor(&alias_sources, |source_name| {
        let key = format!("smelt.{source_name}");
        if !refs.iter().any(|r| r == &key) {
            return None;
        }
        source_timeseries.get(&key).map(|ts| DrivingSource {
            name: key.clone(),
            timeseries: ts.clone(),
        })
    }) {
        Ok(ds) => Some(ds),
        Err(AnchorAmbiguity::NoCandidate) => {
            diagnostics.push(CumulativeDiagnostic::CumulativeNoDrivingSource);
            None
        }
        Err(AnchorAmbiguity::Multiple(candidates)) => {
            diagnostics.push(CumulativeDiagnostic::CumulativeMultipleDrivingSources {
                candidates: candidates
                    .into_iter()
                    .map(|n| format!("smelt.{n}"))
                    .collect(),
            });
            None
        }
    };

    // Rule: GROUP BY must not contain the driving source's partition column.
    if let Some(ds) = &driving_source {
        let partition_col = &ds.timeseries.partition_column;
        let partition_col_lower = partition_col.to_ascii_lowercase();
        let contains_partition = unique_key
            .iter()
            .any(|k| k.to_ascii_lowercase() == partition_col_lower)
            || analysis
                .group_by_exprs
                .iter()
                .any(|e| e.to_ascii_lowercase() == partition_col_lower);
        if contains_partition {
            diagnostics.push(
                CumulativeDiagnostic::CumulativeGroupByContainsPartitionColumn {
                    partition_column: partition_col.clone(),
                },
            );
        }
    }

    if !diagnostics.is_empty() {
        return Err(diagnostics);
    }

    Ok(CumulativeClassification {
        unique_key,
        aggregator_columns,
        driving_source: driving_source.expect("driving_source must be set when diagnostics empty"),
    })
}

/// Verify the projection is a direct call to `expected_fn` and nothing else —
/// i.e. it starts with `<fn>(` (after trimming) and the closing `)` is the
/// last non-whitespace character.
fn is_direct_function_call(text: &str, expected_fn: &str) -> bool {
    let trimmed = text.trim();
    let upper = trimmed.to_ascii_uppercase();
    let prefix = format!("{}(", expected_fn);
    // Allow qualified prefixes like `PG_CATALOG.SUM(`.
    let starts_ok = upper.starts_with(&prefix)
        || upper
            .rsplit('.')
            .next()
            .map(|s| s.starts_with(&prefix))
            .unwrap_or(false);
    if !starts_ok {
        return false;
    }
    // The expression must terminate at the matching close-paren.
    if !trimmed.ends_with(')') {
        return false;
    }
    // Confirm the close-paren matches the open-paren of the outer call —
    // i.e. the parenthesis depth returns to zero exactly at the end.
    let mut depth: i32 = 0;
    let mut closed_at_end = false;
    for (i, ch) in trimmed.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    closed_at_end = i + ch.len_utf8() == trimmed.len();
                    break;
                }
            }
            _ => {}
        }
    }
    closed_at_end
}

#[cfg(test)]
mod tests {
    use super::*;
    use smelt_core::config::Granularity;

    fn ts(partition_col: &str) -> TimeseriesConfig {
        TimeseriesConfig {
            event_time_column: partition_col.to_string(),
            partition_column: partition_col.to_string(),
            granularity: Granularity::Day,
            week_start: None,
            assert_monotonic: false,
        }
    }

    fn events_source_map() -> SourceTimeseriesMap {
        let mut m = HashMap::new();
        m.insert("smelt.silver.events_parsed".to_string(), ts("event_date"));
        m
    }

    /// The motivating device_user_edges SELECT classifies cleanly.
    #[test]
    fn test_classify_simple() {
        let sql = r#"SELECT
    device_id,
    user_id,
    COUNT(*) AS event_count,
    MIN(event_ts) AS first_seen,
    MAX(event_ts) AS last_seen
FROM smelt.silver.events_parsed
WHERE user_id IS NOT NULL
GROUP BY device_id, user_id"#;

        let refs = vec!["smelt.silver.events_parsed".to_string()];
        let classification =
            classify_cumulative(sql, &refs, &events_source_map()).expect("must classify");

        assert_eq!(classification.unique_key, vec!["device_id", "user_id"]);
        assert_eq!(classification.aggregator_columns.len(), 3);
        let event_count = &classification.aggregator_columns[0];
        assert_eq!(event_count.output_name, "event_count");
        assert_eq!(event_count.per_partition_agg, "COUNT");
        assert_eq!(
            event_count.cross_partition_combiner,
            CrossPartitionCombiner::Sum
        );
        let first_seen = &classification.aggregator_columns[1];
        assert_eq!(first_seen.output_name, "first_seen");
        assert_eq!(first_seen.per_partition_agg, "MIN");
        assert_eq!(
            first_seen.cross_partition_combiner,
            CrossPartitionCombiner::Min
        );
        let last_seen = &classification.aggregator_columns[2];
        assert_eq!(last_seen.output_name, "last_seen");
        assert_eq!(last_seen.per_partition_agg, "MAX");
        assert_eq!(
            last_seen.cross_partition_combiner,
            CrossPartitionCombiner::Max
        );
        assert_eq!(
            classification.driving_source.name,
            "smelt.silver.events_parsed"
        );
    }

    /// A SELECT with no GROUP BY produces CumulativeRequiresGroupBy.
    #[test]
    fn test_no_group_by_refused() {
        let sql = "SELECT COUNT(*) AS n FROM smelt.silver.events_parsed";
        let refs = vec!["smelt.silver.events_parsed".to_string()];
        let err = classify_cumulative(sql, &refs, &events_source_map()).unwrap_err();
        assert!(
            err.iter()
                .any(|d| matches!(d, CumulativeDiagnostic::CumulativeRequiresGroupBy)),
            "diagnostics: {:?}",
            err
        );
    }

    /// STRING_AGG on a non-key projection produces CumulativeUnknownAggregator.
    #[test]
    fn test_unknown_aggregator_refused() {
        let sql = r#"SELECT
    device_id,
    STRING_AGG(name, ',') AS names
FROM smelt.silver.events_parsed
GROUP BY device_id"#;
        let refs = vec!["smelt.silver.events_parsed".to_string()];
        let err = classify_cumulative(sql, &refs, &events_source_map()).unwrap_err();
        assert!(
            err.iter().any(|d| matches!(
                d,
                CumulativeDiagnostic::CumulativeUnknownAggregator { offending, .. }
                    if offending.to_uppercase() == "STRING_AGG"
            )),
            "diagnostics: {:?}",
            err
        );
    }

    /// A composite aggregate expression `SUM(x) + 1` is refused.
    #[test]
    fn test_composite_aggregate_expression_refused() {
        let sql = r#"SELECT
    device_id,
    SUM(amount) + 1 AS shifted_sum
FROM smelt.silver.events_parsed
GROUP BY device_id"#;
        let refs = vec!["smelt.silver.events_parsed".to_string()];
        let err = classify_cumulative(sql, &refs, &events_source_map()).unwrap_err();
        assert!(
            err.iter().any(|d| matches!(
                d,
                CumulativeDiagnostic::CumulativeUnknownAggregator { offending, .. }
                    if offending.contains("composite") || offending.contains("expression")
            )),
            "diagnostics: {:?}",
            err
        );
    }

    /// COUNT(DISTINCT x) is refused — not commutative under merge.
    #[test]
    fn test_count_distinct_refused() {
        let sql = r#"SELECT
    device_id,
    COUNT(DISTINCT user_id) AS unique_users
FROM smelt.silver.events_parsed
GROUP BY device_id"#;
        let refs = vec!["smelt.silver.events_parsed".to_string()];
        let err = classify_cumulative(sql, &refs, &events_source_map()).unwrap_err();
        assert!(
            err.iter()
                .any(|d| matches!(d, CumulativeDiagnostic::CumulativeUnknownAggregator { .. })),
            "diagnostics: {:?}",
            err
        );
    }

    /// GROUP BY containing the driving source's partition_column is refused.
    #[test]
    fn test_group_by_contains_partition_column_refused() {
        let sql = r#"SELECT
    device_id,
    user_id,
    event_date,
    COUNT(*) AS n
FROM smelt.silver.events_parsed
GROUP BY device_id, user_id, event_date"#;
        let refs = vec!["smelt.silver.events_parsed".to_string()];
        let err = classify_cumulative(sql, &refs, &events_source_map()).unwrap_err();
        assert!(
            err.iter().any(|d| matches!(
                d,
                CumulativeDiagnostic::CumulativeGroupByContainsPartitionColumn { partition_column }
                    if partition_column == "event_date"
            )),
            "diagnostics: {:?}",
            err
        );
    }

    /// An OVER (...) projection produces CumulativeForbidsWindowFunctions.
    #[test]
    fn test_window_function_refused() {
        let sql = r#"SELECT
    device_id,
    COUNT(*) OVER (PARTITION BY device_id) AS n
FROM smelt.silver.events_parsed
GROUP BY device_id"#;
        let refs = vec!["smelt.silver.events_parsed".to_string()];
        let err = classify_cumulative(sql, &refs, &events_source_map()).unwrap_err();
        assert!(
            err.iter()
                .any(|d| matches!(d, CumulativeDiagnostic::CumulativeForbidsWindowFunctions)),
            "diagnostics: {:?}",
            err
        );
    }

    /// NOW() in the outer body is refused.
    #[test]
    fn test_nondeterministic_refused() {
        let sql = r#"SELECT
    device_id,
    MAX(event_ts) - NOW() AS stale_for
FROM smelt.silver.events_parsed
GROUP BY device_id"#;
        let refs = vec!["smelt.silver.events_parsed".to_string()];
        let err = classify_cumulative(sql, &refs, &events_source_map()).unwrap_err();
        assert!(
            err.iter().any(|d| matches!(
                d,
                CumulativeDiagnostic::CumulativeForbidsNondeterministic { .. }
            )),
            "diagnostics: {:?}",
            err
        );
    }

    /// A SELECT from a source with no timeseries: produces CumulativeNoDrivingSource.
    #[test]
    fn test_zero_driving_sources_refused() {
        let sql = r#"SELECT
    device_id,
    COUNT(*) AS n
FROM smelt.silver.lookup_table
GROUP BY device_id"#;
        let refs = vec!["smelt.silver.lookup_table".to_string()];
        let err = classify_cumulative(sql, &refs, &HashMap::new()).unwrap_err();
        assert!(
            err.iter()
                .any(|d| matches!(d, CumulativeDiagnostic::CumulativeNoDrivingSource)),
            "diagnostics: {:?}",
            err
        );
    }

    /// Two timeseries-tagged sources produces CumulativeMultipleDrivingSources.
    #[test]
    fn test_multiple_driving_sources_refused() {
        let sql = r#"SELECT
    device_id,
    COUNT(*) AS n
FROM smelt.silver.events_a
JOIN smelt.silver.events_b USING (device_id)
GROUP BY device_id"#;
        let refs = vec![
            "smelt.silver.events_a".to_string(),
            "smelt.silver.events_b".to_string(),
        ];
        let mut map = HashMap::new();
        map.insert("smelt.silver.events_a".to_string(), ts("event_date"));
        map.insert("smelt.silver.events_b".to_string(), ts("event_date"));
        let err = classify_cumulative(sql, &refs, &map).unwrap_err();
        assert!(
            err.iter().any(|d| matches!(
                d,
                CumulativeDiagnostic::CumulativeMultipleDrivingSources { .. }
            )),
            "diagnostics: {:?}",
            err
        );
    }

    /// A timeseries-tagged ref that is only reached through a subquery (not
    /// one of the top-level FROM/JOIN inputs) is not a driving-fact
    /// candidate — the alias-scoped resolver only considers the joined
    /// inputs of the outer scope, unlike the former ref-count selection
    /// (which would have flat-counted both refs and refused as ambiguous).
    #[test]
    fn test_driving_source_resolved_via_alias_scope_ignores_non_joined_ref() {
        let sql = r#"SELECT
    device_id,
    COUNT(*) AS n
FROM smelt.silver.events_a
WHERE device_id IN (SELECT device_id FROM smelt.silver.events_b)
GROUP BY device_id"#;
        let refs = vec![
            "smelt.silver.events_a".to_string(),
            "smelt.silver.events_b".to_string(),
        ];
        let mut map = HashMap::new();
        map.insert("smelt.silver.events_a".to_string(), ts("event_date"));
        map.insert("smelt.silver.events_b".to_string(), ts("event_date"));
        let classification =
            classify_cumulative(sql, &refs, &map).expect("must classify: only events_a is joined");
        assert_eq!(classification.driving_source.name, "smelt.silver.events_a");
    }

    /// A SELECT from one timeseries source and one lookup classifies cleanly.
    #[test]
    fn test_lookup_source_admitted() {
        let sql = r#"SELECT
    e.device_id,
    e.user_id,
    COUNT(*) AS event_count
FROM smelt.silver.events_parsed e
JOIN smelt.silver.user_lookup l USING (user_id)
WHERE l.is_active
GROUP BY e.device_id, e.user_id"#;
        let refs = vec![
            "smelt.silver.events_parsed".to_string(),
            "smelt.silver.user_lookup".to_string(),
        ];
        let classification =
            classify_cumulative(sql, &refs, &events_source_map()).expect("must classify");
        assert_eq!(
            classification.driving_source.name,
            "smelt.silver.events_parsed"
        );
    }

    /// Cross-partition combiners render correctly.
    #[test]
    fn test_combiner_rendering() {
        assert_eq!(
            CrossPartitionCombiner::Sum.render("target.x", "delta.x"),
            "target.x + delta.x"
        );
        assert_eq!(
            CrossPartitionCombiner::Min.render("target.x", "delta.x"),
            "LEAST(target.x, delta.x)"
        );
        assert_eq!(
            CrossPartitionCombiner::Max.render("target.x", "delta.x"),
            "GREATEST(target.x, delta.x)"
        );
        assert_eq!(
            CrossPartitionCombiner::BoolOr.render("target.x", "delta.x"),
            "target.x OR delta.x"
        );
    }

    /// combiner_for handles case-insensitive lookups.
    #[test]
    fn test_combiner_lookup_case_insensitive() {
        assert_eq!(combiner_for("count"), Some(CrossPartitionCombiner::Sum));
        assert_eq!(combiner_for("COUNT"), Some(CrossPartitionCombiner::Sum));
        assert_eq!(combiner_for("Sum"), Some(CrossPartitionCombiner::Sum));
        assert_eq!(combiner_for("avg"), None);
        assert_eq!(combiner_for("string_agg"), None);
    }
}
