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
use smelt_logical::analysis::join_shape::JoinContext;
use smelt_logical::analysis::walk::model_property_vector;
use smelt_logical::maintenance::choice::{resolve_write_suppression, WriteSuppression};
use smelt_logical::maintenance::derive::row_identity;
use smelt_logical::maintenance::emit::{
    emit_keyed_fold, emit_keyed_fold_suppressed, emit_recurrence_bound_probe, MaintenanceDialect,
    TargetSlicePredicate,
};
use smelt_logical::maintenance::locality::{
    establish_locality, partition_column_provably_not_null, LocalityInputs, LocalitySlice,
};
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
            match &col.cross_partition_combiner {
                // The order-monotone overwrite family (`MAX_BY`/`MIN_BY`) is
                // not a monoid — `combiner_for`'s allowlist deliberately
                // doesn't cover it (`analysis::discriminants::Monotone::Order`
                // is a semilattice fold, not a commutative monoid). It is
                // already verified at classify time
                // (`rules::cumulative::classify_order_monotone_column`), so
                // this defense-in-depth pass has nothing further to check.
                CrossPartitionCombiner::OrderMonotone { .. } => {}
                _ => {
                    if combiner_for(&col.per_partition_agg).is_none() {
                        return Some(format!(
                            "aggregator `{}` on column `{}` is not a monoid combiner",
                            col.per_partition_agg, col.output_name
                        ));
                    }
                }
            }
        }
        None
    }

    fn merge_sql(
        &self,
        schema: &str,
        table: &str,
        delta_sql: &str,
        slice: Option<&TargetSlicePredicate>,
        suppression: &WriteSuppression,
    ) -> String {
        build_cumulative_merge_sql(schema, table, delta_sql, self, slice, suppression)
    }

    /// `Grade::Additive` iff any aggregator column's cross-partition
    /// combiner is `Sum` — an additive fold double-counts on a repeat merge
    /// (`docs/specs/incremental_models.md` §"The reconciliation ledger" —
    /// "Storage is graded by algebra"). The remaining catalogued combiners
    /// (`Min`/`Max`/`BoolAnd`/`BoolOr`/`BitAnd`/`BitOr`/`BitXor`, and the
    /// order-monotone overwrite family `OrderMonotone`) grade `Idempotent`:
    /// re-merging the SAME already-reflected delta twice leaves the
    /// incumbent-wins comparison unchanged (`delta.ord > target.ord` is
    /// false the second time, since after the first merge
    /// `target.ord == delta.ord`) — a re-run converges, it does not
    /// double-count. Mixing an additive column with idempotent ones in the
    /// same cell still grades the whole cell `Additive` — conservative
    /// (never unsafe), per `WindowedKeyedRule::ledger_grade`'s doc comment.
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

    /// `keyed`'s own `unique_key` is exactly what
    /// `emit_recurrence_bound_probe` (`smelt_logical::maintenance::emit`,
    /// the single-owner emitter for this statement) needs to build the
    /// route-3 checked-merge probe — this impl supplies it and delegates
    /// the SQL text construction entirely to that emitter.
    fn recurrence_probe_sql(
        &self,
        schema: &str,
        table: &str,
        delta_sql: &str,
        partition_column: &str,
        slice_lower: &str,
        dialect: MaintenanceDialect,
    ) -> Option<String> {
        let schema_table = format!("{schema}.{table}");
        Some(
            emit_recurrence_bound_probe(
                &schema_table,
                &self.unique_key,
                partition_column,
                delta_sql,
                slice_lower,
                dialect,
            )
            .sql,
        )
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
    source_key_recurrence: &HashMap<String, smelt_core::sources::KeyRecurrence>,
    verbose: bool,
    retry: &crate::execute::RetryPolicy<'_>,
) -> Result<ExecutionResult> {
    let model_name = &model.address_segments.join(".");
    let _ = (target, compiler); // reserved for future per-target compiler dispatch

    // 1. Classify the model SQL.
    let clean_sql = smelt_parser::strip_frontmatter(&model.content).to_string();
    let refs = collect_refs_from_sql(&clean_sql);
    let model_has_timeseries = model
        .metadata
        .as_ref()
        .is_some_and(|m| m.timeseries.is_some());

    let classification =
        classify_cumulative(&clean_sql, &refs, source_timeseries, model_has_timeseries)
            .map_err(|diagnostics| format_classifier_error(model_name, &diagnostics))?;

    let driving_source_name = classification.driving_source.name.clone();
    let driving_ts = classification.driving_source.timeseries.clone();

    info!(
        "Running model: {} (keyed, driving source = {})",
        model_name, driving_source_name
    );

    // 1b. When the model declares its own `timeseries:` block, key temporal
    //     locality (`docs/specs/incremental_models.md` §"Key temporal
    //     locality") must be established before any merge is emitted — the
    //     single seam (`smelt_logical::maintenance::locality::establish_
    //     locality`) is a pure function, so calling it here (in addition to
    //     `smelt-db`'s plan-derivation call site) is not a second place
    //     deciding admissibility: both calls are deterministic over the same
    //     facts and must agree, including `partition_column_not_null`
    //     (`partition_column_provably_not_null`, the single shared
    //     derivation both call sites use).
    let locality_slice: Option<LocalitySlice> =
        match model.metadata.as_ref().and_then(|m| m.timeseries.as_ref()) {
            Some(own_ts) => {
                let declared_functional_dependencies = model
                    .metadata
                    .as_ref()
                    .map(|m| m.functional_dependencies.as_slice())
                    .unwrap_or(&[]);
                let inputs = LocalityInputs {
                    model_name: model_name.clone(),
                    unique_key: classification.unique_key.clone(),
                    partition_column: own_ts.partition_column.clone(),
                    granularity: own_ts.granularity,
                    // Shared with `smelt-db`'s static plan-derivation call
                    // site (`smelt_logical::maintenance::locality::
                    // partition_column_provably_not_null`'s own doc
                    // comment): a model `smelt-db` admits through the
                    // locality gate must also be admitted here, or the run
                    // would fail on a model `smelt explain` reported as
                    // valid.
                    partition_column_not_null: partition_column_provably_not_null(
                        &clean_sql,
                        &classification.unique_key,
                        &own_ts.partition_column,
                        Some(&driving_ts.partition_column),
                    ),
                    driving_source_name: driving_source_name.clone(),
                    driving_source_has_clock: true,
                    driving_source_granularity: Some(driving_ts.granularity),
                    driving_source_partition_column: Some(driving_ts.partition_column.clone()),
                    declared_functional_dependencies,
                    driving_source_key_recurrence: source_key_recurrence.get(&driving_source_name),
                    sql: &clean_sql,
                };
                match establish_locality(&inputs) {
                    Ok(slice) => Some(slice),
                    Err(refusal) => {
                        anyhow::bail!("{}", refusal.message(model_name));
                    }
                }
            }
            None => None,
        };

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

    // Resolved once, up front (like `locality_slice` above), from the
    // model's own P2 row identity and P3 change-comparability over the
    // fold's own output columns (`docs/plans/20260715-composed-axes-
    // conditional-maintenance.md` Phase C6) — never re-derived per step.
    let suppression = resolve_cumulative_write_suppression(&classification, &clean_sql);

    run_windowed_keyed_maintenance(
        backend,
        model_name,
        schema,
        db_table_name,
        &steps,
        &classification,
        locality_slice.as_ref(),
        &suppression,
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
        retry,
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
/// Shape (unconditional):
/// ```sql
/// MERGE INTO schema.table AS target
/// USING (<delta_sql>) AS delta
/// ON target.k1 = delta.k1 AND target.k2 = delta.k2
/// WHEN MATCHED THEN UPDATE SET
///     col_a = <combiner>(target.col_a, delta.col_a),
///     ...
/// WHEN NOT MATCHED THEN INSERT *
/// ```
///
/// `suppression` is the cell's already-resolved [`WriteSuppression`] verdict
/// (T1, `docs/plans/20260715-composed-axes-conditional-maintenance.md`
/// Phase C6 — extending Phase C5's keyed-fold suppression emitter into the
/// live `refresh: keyed` maintenance loop): `WriteSuppression::Suppressed`
/// dispatches to [`emit_keyed_fold_suppressed`] (the matched arm gains an
/// `IS DISTINCT FROM` guard over the compared fold columns, composing with
/// `slice` unchanged — both predicates land on the same `ON` clause when
/// both are present, and a bare keyed model with no locality slice carries
/// only the suppression arm); `WriteSuppression::Unconditional` keeps this
/// function's pre-Phase-C6 [`emit_keyed_fold`] dispatch, byte-identical.
/// This function does no admission of its own — the caller (`execute_
/// cumulative_aggregate`) resolves `suppression` once, from the model's own
/// P2 row identity and P3 change-comparability over the fold's own output
/// columns.
pub fn build_cumulative_merge_sql(
    schema: &str,
    table: &str,
    delta_sql: &str,
    classification: &CumulativeClassification,
    slice: Option<&TargetSlicePredicate>,
    suppression: &WriteSuppression,
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
    let group = match suppression {
        WriteSuppression::Suppressed { compared_columns } => emit_keyed_fold_suppressed(
            &schema_table,
            &classification.unique_key,
            &folds,
            delta_sql,
            slice,
            compared_columns,
            MaintenanceDialect::DuckDb,
        ),
        WriteSuppression::Unconditional { .. } => emit_keyed_fold(
            &schema_table,
            &classification.unique_key,
            &folds,
            delta_sql,
            slice,
            MaintenanceDialect::DuckDb,
        ),
    };
    group.statements[0].sql.clone()
}

/// Resolve this classification's [`WriteSuppression`] verdict
/// (`smelt_logical::maintenance::choice::resolve_write_suppression`): P2 row
/// identity comes from the classifier's own already-proven `unique_key`
/// (the classifier only reaches `Grain::Key` admission over a proven
/// `GROUP BY` key, so treating it as the declared key for [`row_identity`]
/// is not a second, independent proof — it is the same key `derive.rs`'s
/// own `Technique::KeyedFold` cell carries as `PlanCell::row_identity`, read
/// off the classifier directly rather than re-deriving a `MaintenancePlan`);
/// P3 change-comparability comes from the shared composition walk
/// (`model_property_vector`) over the model's own SQL. `compared_columns`
/// is exactly the fold's own output columns — there is nothing else a
/// keyed-fold cell's matched arm could write.
fn resolve_cumulative_write_suppression(
    classification: &CumulativeClassification,
    sql: &str,
) -> WriteSuppression {
    let group_columns: Vec<String> = classification
        .aggregator_columns
        .iter()
        .map(|col| col.output_name.clone())
        .collect();
    let identity = row_identity(&classification.unique_key, sql);
    let comparability = model_property_vector(sql, &JoinContext::new())
        .map(|v| v.comparability)
        .unwrap_or_default();
    resolve_write_suppression(&group_columns, &comparability, &identity)
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
///
/// `model_has_timeseries` is whether the model's own frontmatter declares a
/// `timeseries:` block — threaded through to `classify_cumulative` so
/// `KeyedGroupByContainsPartitionColumn` is narrowed to the no-`timeseries:`
/// case (a model with its own `timeseries:` block is decided by the key
/// temporal locality gate instead, `maintenance::locality::establish_locality`).
pub fn classify_cumulative_sql(
    model_name: &str,
    clean_sql: &str,
    source_timeseries: &SourceTimeseriesMap,
    model_has_timeseries: bool,
) -> Result<CumulativeClassification> {
    let refs = collect_refs_from_sql(clean_sql);
    classify_cumulative(clean_sql, &refs, source_timeseries, model_has_timeseries)
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

    /// The plain unconditional matched arm — the pre-Phase-C6 default the
    /// existing byte-identity tests below still exercise, so the
    /// `emit_keyed_fold` dispatch path stays unchanged.
    fn unconditional() -> WriteSuppression {
        WriteSuppression::Unconditional {
            why: "test exercises the unconditional dispatch path directly".to_string(),
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
        let sql = build_cumulative_merge_sql(
            "main",
            "device_user_edges",
            delta_sql,
            &classification,
            None,
            &unconditional(),
        );
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
            None,
            MaintenanceDialect::DuckDb,
        );
        assert_eq!(
            sql, expected.statements[0].sql,
            "build_cumulative_merge_sql must be byte-identical to a direct emitter call"
        );
    }

    /// A locality-admitted model's `MERGE` carries a target-side partition
    /// predicate over the slice (`docs/specs/incremental_models.md` §"Key
    /// temporal locality") — a non-time-partitioned keyed model's SQL (the
    /// `None` case above) stays byte-unchanged; passing `Some` only adds the
    /// extra `AND` clause, nothing else in the statement shifts.
    #[test]
    fn build_cumulative_merge_sql_with_slice_carries_target_partition_predicate() {
        let classification = CumulativeClassification {
            unique_key: vec!["device_id".to_string(), "event_date".to_string()],
            aggregator_columns: vec![AggregatorColumn {
                output_name: "event_count".to_string(),
                per_partition_agg: "COUNT".to_string(),
                cross_partition_combiner: CrossPartitionCombiner::Sum,
            }],
            driving_source: DrivingSource {
                name: "smelt.sources.raw.events".to_string(),
                timeseries: dummy_ts(),
            },
        };
        let delta_sql = "SELECT device_id, event_date, COUNT(*) AS event_count FROM events \
                          WHERE event_date = '2026-01-02' GROUP BY 1, 2";
        let slice = TargetSlicePredicate::Range {
            partition_column: "event_date".to_string(),
            lower: "2026-01-02".to_string(),
            upper: "2026-01-02".to_string(),
        };
        let without_slice = build_cumulative_merge_sql(
            "main",
            "device_daily",
            delta_sql,
            &classification,
            None,
            &unconditional(),
        );
        let with_slice = build_cumulative_merge_sql(
            "main",
            "device_daily",
            delta_sql,
            &classification,
            Some(&slice),
            &unconditional(),
        );
        assert!(
            with_slice.contains("AND target.event_date BETWEEN '2026-01-02' AND '2026-01-02'"),
            "expected slice predicate in: {with_slice}"
        );
        assert_eq!(
            with_slice,
            format!(
                "{} AND target.event_date BETWEEN '2026-01-02' AND '2026-01-02'{}",
                &without_slice[..without_slice.find(" WHEN MATCHED").unwrap()],
                &without_slice[without_slice.find(" WHEN MATCHED").unwrap()..]
            ),
            "the slice predicate must be the ONLY difference from the unsliced merge"
        );
    }

    /// Route 2 (key-determined) locality carries a `DeltaValues` slice
    /// (`docs/specs/incremental_models.md` §"Key temporal locality", route
    /// 2) — the target scan is pruned to exactly the partition-column
    /// values the step's own delta relation carries, read off that same
    /// relation rather than a caller-precomputed range.
    #[test]
    fn build_cumulative_merge_sql_with_delta_values_slice_carries_in_subquery_predicate() {
        let classification = CumulativeClassification {
            unique_key: vec!["transaction_id".to_string()],
            aggregator_columns: vec![AggregatorColumn {
                output_name: "max_amount".to_string(),
                per_partition_agg: "MAX".to_string(),
                cross_partition_combiner: CrossPartitionCombiner::Max,
            }],
            driving_source: DrivingSource {
                name: "smelt.sources.raw.transactions".to_string(),
                timeseries: dummy_ts(),
            },
        };
        let delta_sql = "SELECT transaction_id, MIN(transaction_timestamp) AS first_seen_at, \
                          MAX(amount) AS max_amount FROM transactions GROUP BY 1";
        let slice = TargetSlicePredicate::DeltaValues {
            partition_column: "first_seen_at".to_string(),
            delta_select: delta_sql.to_string(),
        };
        let with_slice = build_cumulative_merge_sql(
            "main",
            "transaction_first_seen",
            delta_sql,
            &classification,
            Some(&slice),
            &unconditional(),
        );
        assert!(
            with_slice.contains(
                "AND target.first_seen_at IN (SELECT DISTINCT first_seen_at FROM (SELECT \
                 transaction_id, MIN(transaction_timestamp) AS first_seen_at, MAX(amount) AS \
                 max_amount FROM transactions GROUP BY 1) AS __locality_delta_values)"
            ),
            "expected DeltaValues predicate in: {with_slice}"
        );
        assert!(
            !with_slice.contains("BETWEEN"),
            "route 2's slice must never render as a margin-based range: {with_slice}"
        );
    }

    /// `docs/plans/20260715-composed-axes-conditional-maintenance.md`
    /// Phase C6: a `WriteSuppression::Suppressed` verdict dispatches to
    /// `emit_keyed_fold_suppressed` instead of the unconditional
    /// `emit_keyed_fold` — the matched arm gains an `IS DISTINCT FROM`
    /// guard over exactly the compared fold columns, byte-identical to a
    /// direct emitter call.
    #[test]
    fn build_cumulative_merge_sql_dispatches_suppressed_variant() {
        let classification = CumulativeClassification {
            unique_key: vec!["device_id".to_string()],
            aggregator_columns: vec![AggregatorColumn {
                output_name: "event_count".to_string(),
                per_partition_agg: "COUNT".to_string(),
                cross_partition_combiner: CrossPartitionCombiner::Sum,
            }],
            driving_source: DrivingSource {
                name: "smelt.sources.raw.events".to_string(),
                timeseries: dummy_ts(),
            },
        };
        let delta_sql = "SELECT device_id, COUNT(*) AS event_count FROM events GROUP BY device_id";
        let suppression = WriteSuppression::Suppressed {
            compared_columns: vec!["event_count".to_string()],
        };
        let sql = build_cumulative_merge_sql(
            "main",
            "device_daily",
            delta_sql,
            &classification,
            None,
            &suppression,
        );

        let expected = emit_keyed_fold_suppressed(
            "main.device_daily",
            &classification.unique_key,
            &[(
                "event_count".to_string(),
                "target.event_count + delta.event_count".to_string(),
            )],
            delta_sql,
            None,
            &["event_count".to_string()],
            MaintenanceDialect::DuckDb,
        );
        assert_eq!(
            sql, expected.statements[0].sql,
            "build_cumulative_merge_sql must dispatch Suppressed to emit_keyed_fold_suppressed, \
             byte-identical to a direct emitter call"
        );
        assert!(sql.contains("IS DISTINCT FROM"));
    }

    /// Phase C6's own claim: a composed (key + time) model's suppressed
    /// `MERGE` carries **both** predicates (the locality slice on the
    /// target read, `IS DISTINCT FROM` on the matched arm); a bare keyed
    /// model with no established locality slice carries only the
    /// suppression arm — never an invented slice. Both shapes dispatch
    /// through the SAME `build_cumulative_merge_sql` call, `slice` being
    /// the only thing that differs.
    #[test]
    fn build_cumulative_merge_sql_composed_suppression_carries_both_predicates_bare_carries_only_one(
    ) {
        let classification = CumulativeClassification {
            unique_key: vec!["device_id".to_string(), "event_date".to_string()],
            aggregator_columns: vec![AggregatorColumn {
                output_name: "max_amount".to_string(),
                per_partition_agg: "MAX".to_string(),
                cross_partition_combiner: CrossPartitionCombiner::Max,
            }],
            driving_source: DrivingSource {
                name: "smelt.sources.raw.events".to_string(),
                timeseries: dummy_ts(),
            },
        };
        let delta_sql = "SELECT device_id, event_date, MAX(amount) AS max_amount FROM events \
                          WHERE event_date = '2026-01-02' GROUP BY 1, 2";
        let suppression = WriteSuppression::Suppressed {
            compared_columns: vec!["max_amount".to_string()],
        };

        // Bare keyed (no established locality slice): suppression arm only,
        // no invented slice.
        let bare = build_cumulative_merge_sql(
            "main",
            "device_daily",
            delta_sql,
            &classification,
            None,
            &suppression,
        );
        assert!(
            bare.contains("IS DISTINCT FROM"),
            "bare keyed suppressed merge must carry the suppression arm: {bare}"
        );
        assert!(
            !bare.contains("BETWEEN") && !bare.contains(" IN ("),
            "bare keyed suppressed merge must never invent a slice: {bare}"
        );

        // Composed (key + time): both predicates, on the same ON clause.
        let slice = TargetSlicePredicate::Range {
            partition_column: "event_date".to_string(),
            lower: "2026-01-02".to_string(),
            upper: "2026-01-02".to_string(),
        };
        let composed = build_cumulative_merge_sql(
            "main",
            "device_daily",
            delta_sql,
            &classification,
            Some(&slice),
            &suppression,
        );
        assert!(
            composed.contains("AND target.event_date BETWEEN '2026-01-02' AND '2026-01-02'"),
            "composed suppressed merge must carry the slice predicate: {composed}"
        );
        assert!(
            composed.contains("IS DISTINCT FROM"),
            "composed suppressed merge must ALSO carry the suppression arm: {composed}"
        );

        let expected = emit_keyed_fold_suppressed(
            "main.device_daily",
            &classification.unique_key,
            &[(
                "max_amount".to_string(),
                "GREATEST(target.max_amount, delta.max_amount)".to_string(),
            )],
            delta_sql,
            Some(&slice),
            &["max_amount".to_string()],
            MaintenanceDialect::DuckDb,
        );
        assert_eq!(
            composed, expected.statements[0].sql,
            "composed suppression+slice dispatch must be byte-identical to a direct emitter call"
        );
    }

    /// Phase C6: `execute_cumulative_aggregate`'s own suppression resolver —
    /// a fully comparable fold over the classifier's own proven `unique_key`
    /// resolves `Suppressed`, naming exactly the fold's own output columns
    /// (mirrors `events_deduped.sql`'s `MIN`-folded shape).
    #[test]
    fn resolve_cumulative_write_suppression_admits_comparable_min_fold() {
        let classification = CumulativeClassification {
            unique_key: vec!["event_id".to_string()],
            aggregator_columns: vec![
                AggregatorColumn {
                    output_name: "device_id".to_string(),
                    per_partition_agg: "MIN".to_string(),
                    cross_partition_combiner: CrossPartitionCombiner::Min,
                },
                AggregatorColumn {
                    output_name: "first_seen_date".to_string(),
                    per_partition_agg: "MIN".to_string(),
                    cross_partition_combiner: CrossPartitionCombiner::Min,
                },
            ],
            driving_source: DrivingSource {
                name: "smelt.sources.raw.events".to_string(),
                timeseries: dummy_ts(),
            },
        };
        let sql = "SELECT event_id, MIN(device_id) AS device_id, \
                    MIN(CAST(event_date AS DATE)) AS first_seen_date \
                    FROM smelt.sources.raw.events GROUP BY event_id";
        let suppression = resolve_cumulative_write_suppression(&classification, sql);
        assert_eq!(
            suppression,
            WriteSuppression::Suppressed {
                compared_columns: vec!["device_id".to_string(), "first_seen_date".to_string()]
            },
            "a MIN-folded group over a proven key must admit suppression, not refuse: \
             {suppression:?}"
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
