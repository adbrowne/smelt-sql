//! Per-partition execution loop for `refresh: keyed` table models.
//!
//! See `docs/specs/incremental_models.md` §"The key grain (`grain: key`)" for the normative spec. This module is
//! the mode's built seed: it only drives the direct-monoid (additive +
//! extremal/lattice) column families.
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
use crate::maintenance_driver::{driving_steps, run_windowed_keyed_maintenance, WindowedKeyedRule};
use crate::transformer::{inject_source_filters, SourceBound, TimeRange};
use anyhow::{Context, Result};
use smelt_backend::{Backend, ExecutionResult};
use smelt_core::ModelFile;
use smelt_logical::maintenance::emit::{emit_keyed_fold, MaintenanceDialect};
use smelt_planner::{
    classify_cumulative, combiner_for, AggregatorColumn, CrossPartitionCombiner,
    CumulativeClassification, KeyedDiagnostic, SourceTimeseriesMap,
};
use smelt_state::reconciliation::Grade;
use std::collections::HashMap;
use tracing::info;

/// `keyed`'s [`WindowedKeyedRule`] impl: its classification already
/// gated every aggregator column through `combiner_for` (the monoid-only
/// allowlist) at classify time, but the driver re-checks independently —
/// defense in depth against a future classifier bug ever handing the driver
/// an unsafe combiner (`model_transforms.md` §Constraints "Equivalence or
/// refusal").
impl WindowedKeyedRule for CumulativeClassification {
    fn refuse(&self) -> Option<String> {
        for col in &self.aggregator_columns {
            if combiner_for(&col.per_partition_agg).is_none() {
                return Some(format!(
                    "aggregator `{}` on column `{}` is not a monoid combiner",
                    col.per_partition_agg, col.output_name
                ));
            }
        }
        None
    }

    fn merge_sql(&self, schema: &str, table: &str, delta_sql: &str) -> String {
        build_cumulative_merge_sql(schema, table, delta_sql, self)
    }

    /// `Grade::Additive` iff any aggregator column's cross-partition
    /// combiner is `Sum` — an additive fold double-counts on a repeat merge
    /// (`docs/specs/incremental_models.md` §"The reconciliation ledger" —
    /// "Storage is graded by algebra"). The remaining catalogued combiners
    /// (`Min`/`Max`/`BoolAnd`/`BoolOr`/`BitAnd`/`BitOr`/`BitXor`) are the
    /// extremal/lattice family and grade `Idempotent`. Mixing an additive
    /// column with idempotent ones in the same cell still grades the whole
    /// cell `Additive` — conservative (never unsafe), per
    /// `WindowedKeyedRule::ledger_grade`'s doc comment.
    fn ledger_grade(&self) -> Grade {
        let any_additive = self
            .aggregator_columns
            .iter()
            .any(|col| matches!(col.cross_partition_combiner, CrossPartitionCombiner::Sum));
        if any_additive {
            Grade::Additive
        } else {
            Grade::Idempotent
        }
    }

    fn ledger_input(&self) -> &str {
        &self.driving_source.name
    }
}

/// Execute a single keyed model over the given run window.
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

    // 1. Classify the model SQL.
    let clean_sql = smelt_parser::strip_frontmatter(&model.content).to_string();
    let refs = collect_refs_from_sql(&clean_sql);

    let classification = classify_cumulative(&clean_sql, &refs, source_timeseries)
        .map_err(|diagnostics| format_classifier_error(model_name, &diagnostics))?;

    let driving_source_name = classification.driving_source.name.clone();
    let driving_ts = classification.driving_source.timeseries.clone();

    info!(
        "Running model: {} (keyed, driving source = {})",
        model_name, driving_source_name
    );

    // 2. Refuse reprocessing (MP12): the windowed-keyed-maintenance driver
    //    (step 3 below) grades this classification's cell via
    //    `WindowedKeyedRule::ledger_grade` above. For an `Additive`-graded
    //    cell — at least one `SUM`-family aggregator column — every step's
    //    create-or-merge action is folded through the warehouse-resident
    //    reconciliation ledger (`docs/specs/incremental_models.md` §"The
    //    reconciliation ledger"), transactionally with the write
    //    (`Backend::fold_ledger_delta`); a step whose delta identity (its
    //    own partition value) is already reflected refuses the run instead
    //    of double-counting (`docs/specs/incremental_models.md` §"Reprocessing" —
    //    `KeyedReprocessedWindow`). An `Idempotent`-graded cell (no
    //    additive column) needs no ledger — re-merging a window is
    //    harmless — and no warehouse ledger table is ever created for it.
    //    The operator's escape hatch for a genuine reprocess remains
    //    dropping the target table before re-running (full rebuild) or a
    //    manual cascade rebuild.

    // 3. Step over the driving source's partitions in temporal order via the
    //    mode-agnostic windowed-keyed-maintenance driver.
    let steps = driving_steps(&time_range.start, &time_range.end, &driving_ts.granularity)
        .with_context(|| {
            format!(
                "Failed to generate partition values for {} over [{}, {})",
                model_name, time_range.start, time_range.end
            )
        })?;

    if steps.is_empty() {
        anyhow::bail!(
            "Run window [{}, {}) covers no partitions of granularity {:?}",
            time_range.start,
            time_range.end,
            driving_ts.granularity
        );
    }

    run_windowed_keyed_maintenance(
        backend,
        model_name,
        schema,
        db_table_name,
        &steps,
        &classification,
        |step| {
            // 4. Per-partition pushdown: inject the driving source's
            //    `[step.start, step.end)` filter, then compile (resolves
            //    smelt.<path> refs to schema.table_name, inlines ephemerals).
            let mut bound_map = HashMap::new();
            bound_map.insert(
                driving_source_name.clone(),
                SourceBound {
                    partition_col: driving_ts.partition_column.clone(),
                    before_secs: 0,
                    after_secs: 0,
                },
            );
            let pushed = inject_source_filters(&clean_sql, &bound_map, &step.range);

            let compiled = compiler
                .get(target)
                .compile_with_sql_and_ephemerals(model, schema, &pushed, resolver)
                .with_context(|| format!("Failed to compile model: {}", model_name))?;

            if verbose {
                tracing::debug!(
                    "-- {} (partition {})\n{}",
                    model_name,
                    step.partition_value,
                    compiled.sql
                );
            }

            Ok(compiled.sql)
        },
    )
    .await
}

/// Build a `MERGE INTO` statement that combines target and delta values
/// per the classifier's cross-partition combiners.
///
/// Thin wrapper over the single-owner emitter
/// (`smelt_logical::maintenance::emit::emit_keyed_fold`,
/// `docs/specs/incremental_models.md` §"Statement emission (single owner)"):
/// this function's only remaining job is rendering each aggregator column's
/// `CrossPartitionCombiner` to a plain SQL expression string — the emitter
/// itself never depends on `smelt-planner`
/// (`docs/specs/architecture.md` §"Layered single-ownership") — then handing
/// the rendered `(column, expression)` pairs to the emitter, which owns the
/// `MERGE` shape.
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
    let folds: Vec<(String, String)> = classification
        .aggregator_columns
        .iter()
        .map(|col: &AggregatorColumn| {
            let target_col = format!("target.{}", col.output_name);
            let delta_col = format!("delta.{}", col.output_name);
            let expr = col.cross_partition_combiner.render(&target_col, &delta_col);
            (col.output_name.clone(), expr)
        })
        .collect();

    let schema_table = format!("{schema}.{table}");
    let group = emit_keyed_fold(
        &schema_table,
        &classification.unique_key,
        &folds,
        delta_sql,
        MaintenanceDialect::DuckDb,
    );
    group.statements[0].sql.clone()
}

/// Collect `smelt.<path>` references from raw SQL by scanning for the prefix.
///
/// Delegates to [`smelt_planner::collect_path_refs`] — the single shared
/// implementation so the runtime's keyed dispatch and the analysis-layer
/// diagnostic gate reach the identical driving-source lookup (Diagnostic parity
/// rule).
fn collect_refs_from_sql(sql: &str) -> Vec<String> {
    smelt_planner::collect_path_refs(sql)
}

/// Classify a keyed model's SQL, collecting its `smelt.<path>` refs and
/// looking the driving source up in `source_timeseries`. Returns the
/// classification on success or a formatted error on rejection.
///
/// This is the single entry point both run-pipeline paths use to enforce the
/// classifier — including the **no-window full-refresh** path. A classifier
/// rejection must refuse the model rather than silently materialise forbidden
/// SQL (`incremental_models.md` §"Key-grain constraints" #4 — "The catalogue is closed and the
/// classifier is fail-closed").
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
fn format_classifier_error(model_name: &str, diagnostics: &[KeyedDiagnostic]) -> anyhow::Error {
    let lines: Vec<String> = diagnostics.iter().map(|d| format!("  - {}", d)).collect();
    anyhow::anyhow!(
        "Model '{}' failed keyed classification:\n{}",
        model_name,
        lines.join("\n")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use smelt_core::config::TimeseriesConfig;
    use smelt_planner::{AggregatorColumn, CrossPartitionCombiner, DrivingSource};

    fn dummy_ts() -> TimeseriesConfig {
        TimeseriesConfig {
            event_time_column: "event_date".to_string(),
            partition_column: "event_date".to_string(),
            granularity: smelt_core::config::Granularity::Day,
            week_start: None,
            assert_monotonic: false,
        }
    }

    /// `build_cumulative_merge_sql` is a thin wrapper over the single-owner
    /// `emit_keyed_fold` emitter (`docs/specs/incremental_models.md`
    /// §"Statement emission (single owner)"): this test asserts its output
    /// is byte-identical to a direct emitter call over the same rendered
    /// combiner expressions, not merely emitter-*shaped* (contains checks
    /// alone would pass even if a stray character crept into the wrapper's
    /// own formatting).
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
        let delta_sql = "SELECT device_id, user_id, COUNT(*) AS event_count, MIN(event_ts) AS first_seen, MAX(event_ts) AS last_seen FROM events GROUP BY 1, 2";
        let sql =
            build_cumulative_merge_sql("main", "device_user_edges", delta_sql, &classification);
        assert!(sql.contains("MERGE INTO main.device_user_edges"));
        assert!(sql.contains("target.device_id = delta.device_id"));
        assert!(sql.contains("target.user_id = delta.user_id"));
        assert!(sql.contains("event_count = target.event_count + delta.event_count"));
        assert!(sql.contains("first_seen = LEAST(target.first_seen, delta.first_seen)"));
        assert!(sql.contains("last_seen = GREATEST(target.last_seen, delta.last_seen)"));
        assert!(sql.contains("WHEN NOT MATCHED THEN INSERT *"));

        let expected = emit_keyed_fold(
            "main.device_user_edges",
            &classification.unique_key,
            &[
                (
                    "event_count".to_string(),
                    "target.event_count + delta.event_count".to_string(),
                ),
                (
                    "first_seen".to_string(),
                    "LEAST(target.first_seen, delta.first_seen)".to_string(),
                ),
                (
                    "last_seen".to_string(),
                    "GREATEST(target.last_seen, delta.last_seen)".to_string(),
                ),
            ],
            delta_sql,
            MaintenanceDialect::DuckDb,
        );
        assert_eq!(
            sql, expected.statements[0].sql,
            "build_cumulative_merge_sql must be byte-identical to a direct emitter call"
        );
    }

    /// The `WindowedKeyedRule` impl must refuse a non-monoid combiner
    /// independently of the classifier that produced it — defense in depth
    /// against ever merging one approximately (`model_transforms.md`
    /// §Constraints "Equivalence or refusal").
    #[test]
    fn refuses_non_monoid_combiner_independently_of_classifier() {
        let classification = CumulativeClassification {
            unique_key: vec!["device_id".to_string()],
            aggregator_columns: vec![AggregatorColumn {
                output_name: "median_latency".to_string(),
                per_partition_agg: "MEDIAN".to_string(),
                cross_partition_combiner: CrossPartitionCombiner::Sum,
            }],
            driving_source: DrivingSource {
                name: "smelt.silver.events_parsed".to_string(),
                timeseries: dummy_ts(),
            },
        };
        let reason = classification.refuse();
        assert!(reason.is_some(), "MEDIAN is not a monoid combiner");
        assert!(reason.unwrap().contains("MEDIAN"));
    }

    #[test]
    fn admits_monoid_combiner() {
        let classification = CumulativeClassification {
            unique_key: vec!["device_id".to_string()],
            aggregator_columns: vec![AggregatorColumn {
                output_name: "event_count".to_string(),
                per_partition_agg: "COUNT".to_string(),
                cross_partition_combiner: CrossPartitionCombiner::Sum,
            }],
            driving_source: DrivingSource {
                name: "smelt.silver.events_parsed".to_string(),
                timeseries: dummy_ts(),
            },
        };
        assert!(classification.refuse().is_none());
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
