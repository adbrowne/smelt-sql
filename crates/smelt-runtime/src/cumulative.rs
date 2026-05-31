//! Per-partition execution loop for `materialization: cumulative_aggregate`.
//!
//! See `docs/specs/cumulative_aggregate.md` for the normative spec.
//!
//! For a run window `[run_start, run_end)`:
//!
//! 1. Classify the model's SQL (`smelt_planner::classify_cumulative`).
//! 2. Step over the driving source's partitions in temporal order.
//! 3. For each partition `D`: source-filter pushdown injects
//!    `<driving_source>.<partition_col> ∈ [D, D + granularity)` and the
//!    rule either creates the target table from the delta SELECT (first
//!    run) or emits a combiner-aware `MERGE INTO`.

use crate::compile::{CompilerRegistry, EphemeralResolver};
use crate::transformer::{inject_source_filters, SourceBound, TimeRange};
use anyhow::{Context, Result};
use smelt_backend::{Backend, ExecutionResult};
use smelt_core::config::TimeseriesConfig;
use smelt_core::ModelFile;
use smelt_planner::{
    classify_cumulative, AggregatorColumn, CumulativeClassification, CumulativeDiagnostic,
    SourceTimeseriesMap,
};
use std::collections::HashMap;
use std::time::Instant;
use tracing::{debug, info};

/// Execute a single cumulative_aggregate model over the given run window.
///
/// Returns the total ExecutionResult (rows summed across partitions, duration
/// summed). The driving source's `timeseries:` block is read from the
/// `source_timeseries` map by `smelt.<path>` key.
#[allow(clippy::too_many_arguments)]
pub async fn execute_cumulative_aggregate(
    backend: &dyn Backend,
    model: &ModelFile,
    compiler: &CompilerRegistry,
    resolver: &EphemeralResolver,
    target: &str,
    schema: &str,
    db_table_name: &str,
    time_range: &TimeRange,
    source_timeseries: &SourceTimeseriesMap,
    verbose: bool,
) -> Result<ExecutionResult> {
    let model_name = &model.address_segments.join(".");
    let _ = (target, compiler); // reserved for future per-target compiler dispatch
    let start = Instant::now();

    // 1. Classify the model SQL.
    let clean_sql = smelt_parser::strip_frontmatter(&model.content).to_string();
    let refs = collect_refs_from_sql(&clean_sql);

    let classification = classify_cumulative(&clean_sql, &refs, source_timeseries)
        .map_err(|diagnostics| format_classifier_error(model_name, &diagnostics))?;

    let driving_source_name = classification.driving_source.name.clone();
    let driving_ts = classification.driving_source.timeseries.clone();

    info!(
        "Running model: {} (cumulative_aggregate, driving source = {})",
        model_name, driving_source_name
    );

    // 2. Refuse reprocessing: if the target table already exists, the run
    //    window must be append-only (no overlap with already-merged
    //    partitions). v1 policy: if the table exists, refuse the run unless
    //    explicitly opted in via `--full-refresh` (which truncates first).
    //
    //    The fine-grained "exactly which partitions are stale" check needs
    //    persistent state (Known Divergences in the spec). v1 is conservative
    //    and matches the spec's §"Reprocessing semantics" — refuse on any
    //    overlap with existing data; the operator falls back to a full
    //    refresh.
    //
    //    This conservative behaviour is also implemented as "table doesn't
    //    exist => normal merge loop; table exists => refuse" rather than
    //    "table exists => silently rebuild". The full-refresh opt-in is
    //    handled by the caller (run.rs) by dropping the table before the
    //    cumulative path is dispatched, so by the time we reach here the
    //    table either does not exist or is being appended to by an
    //    operator who has accepted the implicit double-count risk.
    //
    //    For now we do *not* check existence here — the run.rs caller is
    //    responsible for managing full-refresh semantics. This block is a
    //    placeholder for future stricter checks once we have a watermark
    //    store.

    // 3. Generate partition values from the run window in temporal order.
    let partitions =
        generate_partitions(&time_range.start, &time_range.end, &driving_ts.granularity)
            .with_context(|| {
                format!(
                    "Failed to generate partition values for {} over [{}, {})",
                    model_name, time_range.start, time_range.end
                )
            })?;

    if partitions.is_empty() {
        anyhow::bail!(
            "Run window [{}, {}) covers no partitions of granularity {:?}",
            time_range.start,
            time_range.end,
            driving_ts.granularity
        );
    }

    debug!(
        "Stepping over {} partition(s) of {} (granularity = {:?})",
        partitions.len(),
        driving_source_name,
        driving_ts.granularity
    );

    let mut total_rows = 0;

    for (idx, partition_value) in partitions.iter().enumerate() {
        let partition_range = single_partition_range(partition_value, &driving_ts);

        let mut bound_map = HashMap::new();
        bound_map.insert(
            driving_source_name.clone(),
            SourceBound {
                partition_col: driving_ts.partition_column.clone(),
                before_secs: 0,
                after_secs: 0,
            },
        );

        let pushed = inject_source_filters(&clean_sql, &bound_map, &partition_range);

        // Compile the per-partition SQL (resolves smelt.<path> refs to
        // schema.table_name, inlines ephemerals).
        let compiled = compiler
            .get(target)
            .compile_with_sql_and_ephemerals(model, schema, &pushed, resolver)
            .with_context(|| format!("Failed to compile model: {}", model_name))?;

        if verbose {
            println!("-- {} (partition {})", model_name, partition_value);
            println!("{}", compiled.sql);
        }

        // First partition (table doesn't yet exist): CREATE TABLE AS the
        // delta SELECT. Subsequent partitions: MERGE INTO with combiners.
        let table_exists = backend
            .table_exists(schema, db_table_name)
            .await
            .unwrap_or(false);

        if !table_exists {
            backend
                .create_table_as(schema, db_table_name, &compiled.sql)
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "Failed to execute model '{}':\n  SQL: {}\n  Error: {}",
                        model_name,
                        compiled.sql,
                        e
                    )
                })?;
            debug!(
                "  partition {} ({}/{}) created target table",
                partition_value,
                idx + 1,
                partitions.len()
            );
        } else {
            let merge_sql =
                build_cumulative_merge_sql(schema, db_table_name, &compiled.sql, &classification);
            backend.execute_sql(&merge_sql).await.map_err(|e| {
                anyhow::anyhow!(
                    "Failed to execute model '{}':\n  SQL: {}\n  Error: {}",
                    model_name,
                    merge_sql,
                    e
                )
            })?;
            debug!(
                "  partition {} ({}/{}) merged",
                partition_value,
                idx + 1,
                partitions.len()
            );
        }

        let row_count = backend
            .get_row_count(schema, db_table_name)
            .await
            .unwrap_or(0);
        total_rows = row_count;
    }

    Ok(ExecutionResult {
        model_name: model_name.to_string(),
        duration: start.elapsed(),
        row_count: total_rows,
        preview: None,
    })
}

/// Build a `MERGE INTO` statement that combines target and delta values
/// per the classifier's cross-partition combiners.
///
/// Shape:
/// ```sql
/// MERGE INTO schema.table AS target
/// USING (<delta_sql>) AS delta
/// ON target.k1 = delta.k1 AND target.k2 = delta.k2
/// WHEN MATCHED THEN UPDATE SET
///     col_a = <combiner>(target.col_a, delta.col_a),
///     ...
/// WHEN NOT MATCHED THEN INSERT *
/// ```
pub fn build_cumulative_merge_sql(
    schema: &str,
    table: &str,
    delta_sql: &str,
    classification: &CumulativeClassification,
) -> String {
    let on_clause = classification
        .unique_key
        .iter()
        .map(|k| format!("target.{} = delta.{}", k, k))
        .collect::<Vec<_>>()
        .join(" AND ");

    let set_clause = classification
        .aggregator_columns
        .iter()
        .map(|col: &AggregatorColumn| {
            let target_col = format!("target.{}", col.output_name);
            let delta_col = format!("delta.{}", col.output_name);
            let expr = col.cross_partition_combiner.render(&target_col, &delta_col);
            format!("{} = {}", col.output_name, expr)
        })
        .collect::<Vec<_>>()
        .join(", ");

    format!(
        "MERGE INTO {}.{} AS target USING ({}) AS delta ON {} \
         WHEN MATCHED THEN UPDATE SET {} \
         WHEN NOT MATCHED THEN INSERT *",
        schema, table, delta_sql, on_clause, set_clause
    )
}

/// Generate partition values from `[start, end)` at the given granularity.
///
/// v1 supports `Day` granularity (the motivator). Coarser granularities are
/// passed through but produce only the start value; refining is reserved for
/// later work.
fn generate_partitions(
    start: &str,
    end: &str,
    granularity: &smelt_core::config::Granularity,
) -> Result<Vec<String>> {
    use chrono::{Duration as ChronoDuration, NaiveDate};
    use smelt_core::config::Granularity;

    let start_date = NaiveDate::parse_from_str(start, "%Y-%m-%d")
        .with_context(|| format!("Invalid start date: {}", start))?;
    let end_date = NaiveDate::parse_from_str(end, "%Y-%m-%d")
        .with_context(|| format!("Invalid end date: {}", end))?;
    if start_date >= end_date {
        anyhow::bail!("Start date ({}) must be before end date ({})", start, end);
    }

    let mut values = Vec::new();
    match granularity {
        Granularity::Day => {
            let mut current = start_date;
            while current < end_date {
                values.push(current.format("%Y-%m-%d").to_string());
                current += ChronoDuration::days(1);
            }
        }
        Granularity::Week => {
            let mut current = start_date;
            while current < end_date {
                values.push(current.format("%Y-%m-%d").to_string());
                current += ChronoDuration::days(7);
            }
        }
        other => {
            anyhow::bail!(
                "cumulative_aggregate v1 supports day and week granularity; got {:?}",
                other
            );
        }
    }
    Ok(values)
}

/// Compute the single-partition [start, end) range from a partition value
/// and the driving source's granularity.
fn single_partition_range(partition_value: &str, ts: &TimeseriesConfig) -> TimeRange {
    use chrono::{Duration as ChronoDuration, NaiveDate};
    use smelt_core::config::Granularity;

    // For day granularity, partition_value is YYYY-MM-DD and the end is the
    // next day. Other granularities follow similar arithmetic.
    let start = partition_value.to_string();
    let end = match ts.granularity {
        Granularity::Day => {
            let d = NaiveDate::parse_from_str(partition_value, "%Y-%m-%d")
                .expect("partition value is YYYY-MM-DD");
            (d + ChronoDuration::days(1)).format("%Y-%m-%d").to_string()
        }
        Granularity::Hour => {
            // Partition value is "YYYY-MM-DD HH:00:00"; end is next hour.
            // For simplicity, parse the date and add 1 hour by reformatting.
            let dt = chrono::NaiveDateTime::parse_from_str(partition_value, "%Y-%m-%d %H:%M:%S")
                .expect("partition value is YYYY-MM-DD HH:MM:SS");
            (dt + ChronoDuration::hours(1))
                .format("%Y-%m-%d %H:%M:%S")
                .to_string()
        }
        Granularity::Week => {
            let d = NaiveDate::parse_from_str(partition_value, "%Y-%m-%d")
                .expect("partition value is YYYY-MM-DD");
            (d + ChronoDuration::days(7)).format("%Y-%m-%d").to_string()
        }
        Granularity::Month | Granularity::Quarter | Granularity::Year => {
            // For these coarser granularities, the run-window arithmetic
            // is delegated to the partition iterator — single-partition
            // pushdown isn't materially different from the run-window
            // pushdown. Keep this branch a placeholder that returns the
            // same value; cumulative for month+ is not a v1 motivator.
            partition_value.to_string()
        }
    };

    TimeRange { start, end }
}

/// Collect `smelt.<path>` references from raw SQL by scanning for the prefix.
///
/// Delegates to [`smelt_planner::collect_path_refs`] — the single shared
/// implementation so the runtime's cumulative dispatch and the analysis-layer
/// diagnostic gate reach the identical driving-source lookup (Diagnostic parity
/// rule).
fn collect_refs_from_sql(sql: &str) -> Vec<String> {
    smelt_planner::collect_path_refs(sql)
}

/// Classify a cumulative model's SQL, collecting its `smelt.<path>` refs and
/// looking the driving source up in `source_timeseries`. Returns the
/// classification on success or a formatted error on rejection.
///
/// This is the single entry point both run-pipeline paths use to enforce the
/// classifier — including the **no-window full-refresh** path. A classifier
/// rejection must refuse the model rather than silently materialise forbidden
/// SQL (`cumulative_aggregate.md` Constraint #10 — "No silent downgrade").
pub fn classify_cumulative_sql(
    model_name: &str,
    clean_sql: &str,
    source_timeseries: &SourceTimeseriesMap,
) -> Result<CumulativeClassification> {
    let refs = collect_refs_from_sql(clean_sql);
    classify_cumulative(clean_sql, &refs, source_timeseries)
        .map_err(|diags| format_classifier_error(model_name, &diags))
}

/// Format classifier diagnostics into a single error message for the CLI.
fn format_classifier_error(
    model_name: &str,
    diagnostics: &[CumulativeDiagnostic],
) -> anyhow::Error {
    let lines: Vec<String> = diagnostics.iter().map(|d| format!("  - {}", d)).collect();
    anyhow::anyhow!(
        "Model '{}' failed cumulative_aggregate classification:\n{}",
        model_name,
        lines.join("\n")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use smelt_planner::{AggregatorColumn, CrossPartitionCombiner, DrivingSource};

    fn dummy_ts() -> TimeseriesConfig {
        TimeseriesConfig {
            event_time_column: "event_date".to_string(),
            partition_column: "event_date".to_string(),
            granularity: smelt_core::config::Granularity::Day,
            week_start: None,
        }
    }

    #[test]
    fn test_build_cumulative_merge_sql() {
        let classification = CumulativeClassification {
            unique_key: vec!["device_id".to_string(), "user_id".to_string()],
            aggregator_columns: vec![
                AggregatorColumn {
                    output_name: "event_count".to_string(),
                    per_partition_agg: "COUNT".to_string(),
                    cross_partition_combiner: CrossPartitionCombiner::Sum,
                },
                AggregatorColumn {
                    output_name: "first_seen".to_string(),
                    per_partition_agg: "MIN".to_string(),
                    cross_partition_combiner: CrossPartitionCombiner::Min,
                },
                AggregatorColumn {
                    output_name: "last_seen".to_string(),
                    per_partition_agg: "MAX".to_string(),
                    cross_partition_combiner: CrossPartitionCombiner::Max,
                },
            ],
            driving_source: DrivingSource {
                name: "smelt.silver.events_parsed".to_string(),
                timeseries: dummy_ts(),
            },
        };
        let sql = build_cumulative_merge_sql(
            "main",
            "device_user_edges",
            "SELECT device_id, user_id, COUNT(*) AS event_count, MIN(event_ts) AS first_seen, MAX(event_ts) AS last_seen FROM events GROUP BY 1, 2",
            &classification,
        );
        assert!(sql.contains("MERGE INTO main.device_user_edges"));
        assert!(sql.contains("target.device_id = delta.device_id"));
        assert!(sql.contains("target.user_id = delta.user_id"));
        assert!(sql.contains("event_count = target.event_count + delta.event_count"));
        assert!(sql.contains("first_seen = LEAST(target.first_seen, delta.first_seen)"));
        assert!(sql.contains("last_seen = GREATEST(target.last_seen, delta.last_seen)"));
        assert!(sql.contains("WHEN NOT MATCHED THEN INSERT *"));
    }

    #[test]
    fn test_single_partition_range_day() {
        let ts = dummy_ts();
        let r = single_partition_range("2024-01-15", &ts);
        assert_eq!(r.start, "2024-01-15");
        assert_eq!(r.end, "2024-01-16");
    }

    #[test]
    fn test_collect_refs_simple() {
        let sql = "SELECT * FROM smelt.silver.events_parsed WHERE id > 0";
        let refs = collect_refs_from_sql(sql);
        assert_eq!(refs, vec!["smelt.silver.events_parsed".to_string()]);
    }

    #[test]
    fn test_collect_refs_skips_functions() {
        let sql = "SELECT smelt.functions.foo(x) FROM smelt.silver.events";
        let refs = collect_refs_from_sql(sql);
        assert_eq!(refs, vec!["smelt.silver.events".to_string()]);
    }
}
