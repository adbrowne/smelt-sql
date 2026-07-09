//! Windowed-keyed-maintenance driver — the mode-agnostic mechanism behind
//! `refresh: keyed`'s window-forward run shape.
//!
//! See `docs/specs/model_transforms.md` §Surface "Windowed-keyed-maintenance
//! driver" and §Semantics "Keyed `merge_into`". The driver is the reusable
//! **classify → step over driving partitions in temporal order → per-partition
//! pushdown → create-or-merge** loop; `keyed` is its first named
//! consumer (`WindowedKeyedRule` impl in `crate::cumulative`).
//!
//! Fail-closed (`model_transforms.md` §Constraints "Equivalence or refusal"):
//! the driver never merges an unsafe combiner approximately. A
//! [`WindowedKeyedRule`] that cannot vouch for every step's combiner refuses
//! the whole run before any backend call is made.

use crate::transformer::TimeRange;
use anyhow::{bail, Context, Result};
use smelt_backend::{Backend, ExecutionResult, IncrementalStrategy};
use smelt_core::config::{CellTechnique, Granularity};
use smelt_logical::analysis::join_shape::{ContributionVerdict, JoinContext};
use smelt_logical::analysis::source_bounds::BoundResult;
use smelt_logical::analysis::walk::model_property_vector;
use smelt_logical::maintenance::{
    MaintenancePlan, PartitionLocal, PlanCell, ScanClamp, SourceFacts, Technique, Trigger,
};
use std::collections::HashSet;
use std::time::Instant;
use tracing::debug;

/// One step of the windowed-keyed-maintenance loop: a single driving-source
/// partition value and the `[start, end)` range it covers.
#[derive(Debug, Clone)]
pub struct MaintenanceStep {
    pub partition_value: String,
    pub range: TimeRange,
}

/// Step over `[start, end)` at `granularity`, producing partitions in
/// temporal order. v1 supports `Day` and `Week` granularity (the shipped
/// motivators); other granularities are refused rather than silently
/// truncated to a single step.
pub fn driving_steps(
    start: &str,
    end: &str,
    granularity: &Granularity,
) -> Result<Vec<MaintenanceStep>> {
    use chrono::{Duration as ChronoDuration, NaiveDate};

    let start_date = NaiveDate::parse_from_str(start, "%Y-%m-%d")
        .with_context(|| format!("Invalid start date: {}", start))?;
    let end_date = NaiveDate::parse_from_str(end, "%Y-%m-%d")
        .with_context(|| format!("Invalid end date: {}", end))?;
    if start_date >= end_date {
        bail!("Start date ({}) must be before end date ({})", start, end);
    }

    let step_days = match granularity {
        Granularity::Day => 1,
        Granularity::Week => 7,
        other => bail!(
            "windowed-keyed-maintenance driver supports day and week granularity; got {:?}",
            other
        ),
    };

    let mut steps = Vec::new();
    let mut current = start_date;
    while current < end_date {
        let next = current + ChronoDuration::days(step_days);
        steps.push(MaintenanceStep {
            partition_value: current.format("%Y-%m-%d").to_string(),
            range: TimeRange {
                start: current.format("%Y-%m-%d").to_string(),
                end: next.format("%Y-%m-%d").to_string(),
            },
        });
        current = next;
    }
    Ok(steps)
}

/// A rule pluggable into the windowed-keyed-maintenance driver. `keyed`'s
/// direct-monoid families are the first named implementor (`crate::cumulative`);
/// the other keyed column families compose the same driver later.
pub trait WindowedKeyedRule: Send + Sync {
    /// `None` when every step is safe to keyed-merge; `Some(reason)` refuses
    /// the **whole run**, before any backend call — a rule that cannot prove
    /// its combiner set is monoid-safe must never merge approximately
    /// (`model_transforms.md` §Constraints "Equivalence or refusal").
    fn refuse(&self) -> Option<String>;

    /// Build the `MERGE INTO` statement combining `schema.table`'s existing
    /// state with one step's compiled delta SQL.
    fn merge_sql(&self, schema: &str, table: &str, delta_sql: &str) -> String;
}

/// Run the windowed-keyed-maintenance loop: `classify` already happened (its
/// result is `rule`); this steps over `steps` in temporal order, compiles
/// each partition's delta SQL via `compile_step`, and creates the target (on
/// the first step, if it doesn't exist) or merges into it (`rule.merge_sql`)
/// otherwise.
///
/// Fails closed before any backend call if `rule.refuse()` fires.
pub async fn run_windowed_keyed_maintenance(
    backend: &dyn Backend,
    model_name: &str,
    schema: &str,
    table: &str,
    steps: &[MaintenanceStep],
    rule: &dyn WindowedKeyedRule,
    mut compile_step: impl FnMut(&MaintenanceStep) -> Result<String>,
) -> Result<ExecutionResult> {
    if let Some(reason) = rule.refuse() {
        bail!(
            "windowed-keyed-maintenance driver refused model '{}': {}",
            model_name,
            reason
        );
    }

    let start = Instant::now();
    let mut total_rows = 0;

    for (idx, step) in steps.iter().enumerate() {
        let delta_sql = compile_step(step)
            .with_context(|| format!("Failed to compile model: {}", model_name))?;

        let table_exists = backend.table_exists(schema, table).await.unwrap_or(false);

        if !table_exists {
            backend
                .create_table_as(schema, table, &delta_sql)
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "Failed to execute model '{}':\n  SQL: {}\n  Error: {}",
                        model_name,
                        delta_sql,
                        e
                    )
                })?;
            debug!(
                "  partition {} ({}/{}) created target table",
                step.partition_value,
                idx + 1,
                steps.len()
            );
        } else {
            let merge_sql = rule.merge_sql(schema, table, &delta_sql);
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
                step.partition_value,
                idx + 1,
                steps.len()
            );
        }

        total_rows = backend.get_row_count(schema, table).await.unwrap_or(0);
    }

    Ok(ExecutionResult {
        model_name: model_name.to_string(),
        duration: start.elapsed(),
        row_count: total_rows,
        preview: None,
    })
}

/// Resolve the `IncrementalStrategy` a model's creation trigger (region
/// recompute over a partition-grain model) should actually execute, by
/// reading the technique the derived `MaintenancePlan` admitted instead of
/// a hardcoded constant (MP11, `docs/specs/maintenance_plan.md` §"Per-cell
/// admission"). Per the "Maintenance-plan purity" invariant (root
/// `CLAUDE.md`), the plan itself is derived exactly once by
/// `smelt-db`'s pure `derive_model_maintenance_plan` — this function calls
/// it and maps the result onto `smelt-backend`'s `IncrementalStrategy`; it
/// never re-implements admission.
///
/// `derive_new_data`'s `Grain::Partition` arm (`smelt-logical`) admits
/// `Technique::DeleteInsert` unconditionally for the creation trigger — no
/// refusal path exists there today — so this call site is a mechanism
/// swap, not an observable behaviour change: it exists so a future
/// admission rule for the creation cell takes effect here automatically,
/// without a second hand-maintained mapping to keep in sync. Falls back to
/// `backend_default` when the model carries no maintenance plan to derive
/// (e.g. `metadata.grain` unset — should not happen once `refresh:
/// incremental` requires a declared grain) or the admitted technique has no
/// `IncrementalStrategy` counterpart (a targeted-write technique never
/// serves the creation trigger's region-recompute corner).
#[allow(clippy::too_many_arguments)]
pub fn resolve_incremental_strategy(
    sql: &str,
    table: &str,
    metadata: &smelt_core::ModelMetadata,
    sources: &[SourceFacts],
    explicitly_mutable: &HashSet<String>,
    backend_default: IncrementalStrategy,
) -> IncrementalStrategy {
    let Some(result) = smelt_db::queries::maintenance::derive_model_maintenance_plan(
        sql,
        table,
        metadata,
        sources,
        explicitly_mutable,
    ) else {
        return backend_default;
    };
    let creation_cell = result
        .plan
        .cells
        .iter()
        .find(|c| matches!(c.trigger, Trigger::NewData { .. }));
    match creation_cell.map(|c| &c.technique) {
        Some(Technique::DeleteInsert) => IncrementalStrategy::DeleteInsert,
        _ => backend_default,
    }
}

/// Which physical technique actually executes for one plan cell, resolved
/// from the derived [`MaintenancePlan`], the operator's optional hard pin
/// (`maintenance.cells[].technique`), and whether the target backend can
/// run a column-scoped `MERGE` at all
/// (`Backend::supports_column_scoped_merge`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolvedTechnique {
    /// No live targeted-write cell for this trigger (unadmitted, or the
    /// backend lacks the capability, and no pin demands one): the caller
    /// falls back to the region-recompute `DELETE`+`INSERT` it already
    /// performs. This is the *safe default*, never a silent substitute for
    /// a technique the operator explicitly pinned.
    RegionRecompute,
    /// An admitted `Technique::ColumnScopedMerge` cell, live on a backend
    /// that can execute it.
    ColumnScopedMerge,
}

/// Resolve which technique executes for `trigger`, mirroring
/// `maintenance_plan.md` §"Per-cell admission": a `technique:` pin bypasses
/// the cost model, **never** admission — pinning `rederive_columns` for a
/// cell the plan did not admit (or that a capability-gapped backend cannot
/// run) is a hard, fail-loud error, not a silent fallback to
/// `RegionRecompute`. Absent a pin, an admitted+runnable `ColumnScopedMerge`
/// cell is preferred (the point of this phase — "first live cell where
/// execution differs by column group"); otherwise the safe region-recompute
/// default applies with no error (an unpinned model simply has no
/// column-scoped cell to run yet).
pub fn resolve_cell_technique(
    plan: &MaintenancePlan,
    trigger: &Trigger,
    pin: Option<CellTechnique>,
    backend_supports_column_scoped_merge: bool,
) -> Result<ResolvedTechnique> {
    let admitted = plan
        .cell_for(trigger)
        .is_some_and(|c| c.technique == Technique::ColumnScopedMerge);
    let live = admitted && backend_supports_column_scoped_merge;

    match pin {
        Some(CellTechnique::RederiveColumns) if live => Ok(ResolvedTechnique::ColumnScopedMerge),
        Some(CellTechnique::RederiveColumns) if admitted => bail!(
            "MaintenanceUnboundedFootprint: pinned technique 'rederive_columns' for {trigger:?} \
             is admitted by the derived plan, but the target backend does not support \
             column-scoped MERGE — a capability gap drops the technique from admission at plan \
             time; refusing rather than silently falling back to a targeted write at runtime"
        ),
        Some(CellTechnique::RederiveColumns) => bail!(
            "MaintenanceUnboundedFootprint: pinned technique 'rederive_columns' for {trigger:?} \
             names a cell the derived plan did not admit — a technique pin bypasses the cost \
             model, never admission (`maintenance_plan.md` §\"Per-cell admission\"); refusing \
             rather than lowering an unbounded-footprint targeted write at runtime"
        ),
        _ if live => Ok(ResolvedTechnique::ColumnScopedMerge),
        _ => Ok(ResolvedTechnique::RegionRecompute),
    }
}

/// Find the first `explicitly_mutable` source whose `Trigger::
/// UpstreamMutation` cell resolves live to `Technique::ColumnScopedMerge`
/// (via [`resolve_cell_technique`]) in the model's derived
/// [`MaintenancePlan`] — the regular incremental execution loop's per-run
/// technique choice (MP11), as distinct from [`resolve_incremental_strategy`]
/// above, which only maps the creation trigger. Per the "Maintenance-plan
/// purity" invariant (root `CLAUDE.md`), this calls
/// `derive_model_maintenance_plan` exactly once and only reads the result —
/// it never re-implements admission itself.
///
/// Returns the matched source name and its admitted [`PlanCell`] so the
/// caller can pick the right physical primitive from `cell.partition_local`
/// (a genuine `ScanClamp` licenses the horizon-clamped
/// [`execute_column_scoped_merge`]; an accepted full scan has no horizon and
/// takes [`execute_column_scoped_merge_full`] instead). `None` when the
/// model carries no maintenance plan, declares no explicitly-mutable
/// source, or no source resolves live — the caller's safe default is the
/// existing region-recompute batch loop, unchanged.
pub fn resolve_live_column_scoped_cell(
    sql: &str,
    table: &str,
    metadata: &smelt_core::ModelMetadata,
    sources: &[SourceFacts],
    explicitly_mutable: &HashSet<String>,
    backend_supports_column_scoped_merge: bool,
) -> Option<(String, PlanCell)> {
    let result = smelt_db::queries::maintenance::derive_model_maintenance_plan(
        sql,
        table,
        metadata,
        sources,
        explicitly_mutable,
    )?;
    explicitly_mutable.iter().find_map(|source| {
        let trigger = Trigger::UpstreamMutation {
            source: source.clone(),
        };
        let resolved = resolve_cell_technique(
            &result.plan,
            &trigger,
            None,
            backend_supports_column_scoped_merge,
        )
        .ok()?;
        if resolved != ResolvedTechnique::ColumnScopedMerge {
            return None;
        }
        result
            .plan
            .cell_for(&trigger)
            .cloned()
            .map(|cell| (source.clone(), cell))
    })
}

/// Execute a live `ColumnScopedMerge` cell whose scan locality is an
/// accepted full scan (`PartitionLocal::No { .. }` with `allow_full_scan`,
/// `maintenance_plan.md` §"Per-cell admission") — the only shape
/// `derive_model_maintenance_plan` currently derives for an
/// `UpstreamMutation` trigger (a clocked mutable source's own scan-bound
/// derivation is deferred; see that function's doc comment). Unlike
/// [`execute_column_scoped_merge`] there is no derived horizon `H` to clamp
/// to — the operator explicitly accepted reading the mutable source in
/// full on the READ side. `dimension_batch_sql` is the model's own
/// re-derivation (every output row, every column) of whatever scope the
/// caller compiled it for — the regular incremental batch loop
/// (`execute.rs`) passes the SAME `[start, end)`-filtered SQL a
/// `DELETE`+`INSERT` batch would otherwise have used, so the WRITE stays
/// targeted to that window (via `unique_key` keyed `MERGE`, not a blind
/// `DELETE`+`INSERT` region rewrite) — matching the cell's own admitted
/// corner (full-input read, targeted write) without regressing a
/// forward-only run's already-processed, un-requested partitions.
pub async fn execute_column_scoped_merge_full(
    backend: &dyn Backend,
    schema: &str,
    table: &str,
    unique_key: &[String],
    dimension_batch_sql: &str,
) -> Result<ExecutionResult> {
    let start = Instant::now();
    backend
        .merge_into(schema, table, dimension_batch_sql, unique_key)
        .await
        .map_err(|e| anyhow::anyhow!("column-scoped MERGE failed for '{schema}.{table}': {e}"))?;
    let row_count = backend.get_row_count(schema, table).await.unwrap_or(0);
    Ok(ExecutionResult {
        model_name: table.to_string(),
        duration: start.elapsed(),
        row_count,
        preview: None,
    })
}

/// Execute one live `ColumnScopedMerge` cell: build the horizon-clamped
/// source `SELECT` (`crate::dimension_horizon_merge::dimension_horizon_merge`
/// — the pure SQL builder F15 already shipped) and `MERGE` it into
/// `schema.table` on `unique_key`. This is the missing physical primitive
/// that turns that builder's SQL text into an executed backend write — the
/// caller must already have obtained `ResolvedTechnique::ColumnScopedMerge`
/// from [`resolve_cell_technique`]; this function does not re-check
/// admission.
///
/// `dimension_batch_sql` must project the **full target row** — every
/// column, not just the re-derived group's — carrying columns outside the
/// group through unchanged from the existing target state. DuckDB's
/// `merge_into` issues `UPDATE SET *`, which requires the source and
/// target column sets to agree exactly (a column-count mismatch is a hard
/// backend error, not a silent by-name subset); passing every other
/// column through unchanged is what keeps the *values* column-scoped even
/// though the physical `SET *` touches every column's assignment.
#[allow(clippy::too_many_arguments)]
pub async fn execute_column_scoped_merge(
    backend: &dyn Backend,
    schema: &str,
    table: &str,
    unique_key: &[String],
    contribution: &ContributionVerdict,
    bound: &BoundResult,
    conv_ts_column: &str,
    conv_ts: &str,
    dimension_batch_sql: &str,
) -> Result<ExecutionResult> {
    let start = Instant::now();
    let full_table = format!("{schema}.{table}");
    let source_sql = crate::dimension_horizon_merge::dimension_horizon_merge(
        contribution,
        bound,
        &full_table,
        conv_ts_column,
        conv_ts,
        dimension_batch_sql,
    )
    .map_err(|reason| anyhow::anyhow!("{reason}"))?;

    backend
        .merge_into(schema, table, &source_sql, unique_key)
        .await
        .map_err(|e| anyhow::anyhow!("column-scoped MERGE failed for '{full_table}': {e}"))?;

    let row_count = backend.get_row_count(schema, table).await.unwrap_or(0);
    Ok(ExecutionResult {
        model_name: table.to_string(),
        duration: start.elapsed(),
        row_count,
        preview: None,
    })
}

/// Derive the [`ContributionVerdict`] [`execute_column_scoped_merge`]
/// requires for the `PartitionLocal::Yes` corner: is `dimension_source`'s
/// join into the model provably one-to-one, so the mutated dimension's
/// contribution folds into the target without needing an inverse
/// (`model_transforms.md` §Semantics "Dimension-driven horizon MERGE").
///
/// `derive_model_maintenance_plan`'s admitted `PlanCell` carries no fan-out
/// proof of its own (`derive_mutation` only derives partition-locality) —
/// this is a second, independent gate the horizon-clamped physical
/// primitive itself demands, computed here from the same composition walk
/// that derives every other composition-relevant model property
/// (`smelt_logical::analysis::walk::model_property_vector`,
/// `architecture.md` §"Property composition walk rule"), never a fresh ad
/// hoc scan.
///
/// Fail-closed: a dimension with no declared `unique_key` cannot license a
/// one-to-one proof at all and refuses outright, as does a model whose
/// outermost `FROM`/`JOIN` carries no join against `dimension_source` at all
/// ([`find_join_alias`] — a leaf-level parse of exactly the join clause this
/// proof cares about, never a re-derivation of admission).
pub fn dimension_join_contribution(
    sql: &str,
    dimension_source: &str,
    dimension_unique_key: &[String],
) -> ContributionVerdict {
    if dimension_unique_key.is_empty() {
        return ContributionVerdict::Refused(format!(
            "source '{dimension_source}' declares no unique_key — the join's cardinality \
             against it cannot be proven one-to-one, so the mutated dimension's contribution \
             cannot be proven to fold into the target without needing an inverse"
        ));
    }
    let Some(alias) = find_join_alias(sql, dimension_source) else {
        return ContributionVerdict::Refused(format!(
            "no top-level join against '{dimension_source}' found in the model's own outermost \
             SELECT — the join's cardinality cannot be proven one-to-one, so the mutated \
             dimension's contribution cannot be proven monotone"
        ));
    };
    let key_cols: Vec<&str> = dimension_unique_key.iter().map(String::as_str).collect();
    let ctx = JoinContext::new().with_composite_unique_key(&alias, &key_cols);
    match model_property_vector(sql, &ctx) {
        Some(pv) if pv.has_fan_out_join => ContributionVerdict::Refused(format!(
            "model has a join that cannot be proven one-to-one against '{dimension_source}'s \
             declared unique_key — a fanned-out join would duplicate rows per merge key, so \
             the horizon-clamped column-scoped MERGE refuses rather than risk a duplicate-key \
             write"
        )),
        Some(_) => ContributionVerdict::Monotone,
        None => ContributionVerdict::Refused(
            "model SQL did not parse to a query the composition walk models — refusing rather \
             than assuming a monotone contribution"
                .to_string(),
        ),
    }
}

/// Find the alias (or bare identifier, when unaliased) `join_shape::fan_out`
/// keys its `JoinContext` lookup on for the top-level join whose
/// `smelt.<path>` table ref resolves to `dimension_source` (the bare,
/// `sources.`-stripped name `SourceFacts::name`/`resolve_live_column_scoped_cell`
/// use).
fn find_join_alias(sql: &str, dimension_source: &str) -> Option<String> {
    let stripped = smelt_parser::strip_frontmatter(sql);
    let parse = smelt_parser::parse(&stripped);
    let file = smelt_parser::File::cast(parse.syntax())?;
    let select = file.select_stmt()?;
    let from = select.from_clause()?;
    for join in from.joins() {
        let table_ref = join.table_ref()?;
        let Some(resolved) =
            smelt_logical::analysis::source_bounds::resolve_table_ref_source_name(&table_ref)
        else {
            continue;
        };
        let matches = resolved == dimension_source
            || resolved
                .strip_prefix("sources.")
                .is_some_and(|bare| bare == dimension_source);
        if matches {
            return table_ref.alias().or_else(|| table_ref.identifier());
        }
    }
    None
}

/// Which physical column-scoped-MERGE corner (MP11,
/// `docs/specs/maintenance_plan.md` §"Per-cell admission") a live
/// `UpstreamMutation` cell dispatches through, mirroring the two shapes
/// `derive_model_maintenance_plan` derives for `Corner::ColumnMerge`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ColumnMergeDispatch {
    /// `PartitionLocal::No` (accepted full scan) —
    /// [`execute_column_scoped_merge_full`].
    Full,
    /// `PartitionLocal::Yes` (a genuine derived `ScanClamp`) —
    /// [`execute_column_scoped_merge`], horizon-clamped to the carried scan.
    Clamped(ScanClamp),
}

/// Decide which physical corner (if any) a live `ColumnScopedMerge` cell
/// dispatches through this run, given the facts the caller has already
/// resolved: whether the target table exists, whether the model declares a
/// `unique_key` to `MERGE` on, and — only consulted for the `Yes` corner —
/// whether the mutated dimension's join contribution is provably monotone
/// ([`dimension_join_contribution`]).
///
/// `None` means the caller falls back to the safe default
/// (region-recompute), exactly like an unadmitted cell — never a runtime
/// error: a missing target table, an undeclared `unique_key`, or an
/// unproven join contribution are all preconditions this run's batches
/// cannot satisfy yet, not a reason to fail the run.
pub fn decide_column_merge_dispatch(
    cell: &PlanCell,
    source: &str,
    table_exists: bool,
    model_declares_unique_key: bool,
    contribution: &ContributionVerdict,
) -> Option<ColumnMergeDispatch> {
    if !table_exists || !model_declares_unique_key {
        return None;
    }
    match &cell.partition_local {
        PartitionLocal::No { .. } => Some(ColumnMergeDispatch::Full),
        PartitionLocal::Yes => {
            let scan = cell.scans.iter().find(|s| s.source == source)?;
            if contribution.is_monotone() {
                Some(ColumnMergeDispatch::Clamped(scan.clone()))
            } else {
                None
            }
        }
    }
}

/// Widen a derived [`ScanClamp`]'s forward reach to at least `batch_width`
/// before handing it to [`execute_column_scoped_merge`] as the horizon `H`.
///
/// `dimension_batch_sql` is already scoped to the current batch's
/// `[start, end)` window (`inject_time_filter`/`inject_source_filters`,
/// `execute.rs`) before `execute_column_scoped_merge` applies its OWN
/// horizon clamp on top. Passing the raw derived `scan.after` straight
/// through would risk NARROWING that already-correct window whenever a
/// batch spans more than the source's own derived margin (e.g. a
/// multi-day backfill batch over a day-granularity clamp), silently
/// dropping the batch's earlier rows from the merge — the horizon clamp
/// may only ever WIDEN the batch window, never narrow it.
pub fn widen_horizon_for_batch(
    scan: &ScanClamp,
    batch_width: smelt_logical::analysis::source_bounds::Seconds,
) -> BoundResult {
    let after = if scan.after.0 > batch_width.0 {
        scan.after
    } else {
        batch_width
    };
    BoundResult::Bounded {
        source_partition_col: scan.column.clone(),
        before: scan.before,
        after,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use smelt_backend::BackendError;
    use smelt_dialect::{BackendCapabilities, SqlDialect};
    use smelt_logical::analysis::source_bounds::Seconds;
    use std::sync::Mutex;

    #[test]
    fn driving_steps_day_granularity_in_temporal_order() {
        let steps = driving_steps("2024-01-01", "2024-01-04", &Granularity::Day).unwrap();
        let values: Vec<&str> = steps.iter().map(|s| s.partition_value.as_str()).collect();
        assert_eq!(values, vec!["2024-01-01", "2024-01-02", "2024-01-03"]);
        assert_eq!(steps[0].range.start, "2024-01-01");
        assert_eq!(steps[0].range.end, "2024-01-02");
    }

    #[test]
    fn driving_steps_week_granularity() {
        let steps = driving_steps("2024-01-01", "2024-01-15", &Granularity::Week).unwrap();
        let values: Vec<&str> = steps.iter().map(|s| s.partition_value.as_str()).collect();
        assert_eq!(values, vec!["2024-01-01", "2024-01-08"]);
        assert_eq!(steps[0].range.end, "2024-01-08");
    }

    #[test]
    fn driving_steps_rejects_unsupported_granularity() {
        let err = driving_steps("2024-01-01", "2024-02-01", &Granularity::Month).unwrap_err();
        assert!(err.to_string().contains("day and week"));
    }

    #[test]
    fn driving_steps_rejects_empty_window() {
        assert!(driving_steps("2024-01-05", "2024-01-01", &Granularity::Day).is_err());
    }

    /// A rule whose combiner set is never monoid-safe — the driver must
    /// refuse the whole run rather than merge approximately.
    struct AlwaysRefuses;

    impl WindowedKeyedRule for AlwaysRefuses {
        fn refuse(&self) -> Option<String> {
            Some("non-monoid combiner (e.g. MEDIAN) cannot be merged".to_string())
        }
        fn merge_sql(&self, _schema: &str, _table: &str, _delta_sql: &str) -> String {
            unreachable!("merge_sql must not be called once refuse() fires")
        }
    }

    /// An in-memory fake backend that records every call it receives so the
    /// driver's classify → step → pushdown → create-or-merge sequencing can
    /// be exercised without a real database.
    #[derive(Default)]
    struct RecordingBackend {
        table_exists: Mutex<bool>,
        calls: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl Backend for RecordingBackend {
        async fn execute_sql(
            &self,
            sql: &str,
        ) -> Result<Vec<arrow::array::RecordBatch>, BackendError> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("execute_sql: {}", sql));
            Ok(vec![])
        }
        async fn create_table_as(
            &self,
            _schema: &str,
            _name: &str,
            sql: &str,
        ) -> Result<(), BackendError> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("create_table_as: {}", sql));
            *self.table_exists.lock().unwrap() = true;
            Ok(())
        }
        async fn create_view_as(
            &self,
            _schema: &str,
            _name: &str,
            _sql: &str,
        ) -> Result<(), BackendError> {
            unreachable!("driver does not create views")
        }
        async fn drop_table_if_exists(
            &self,
            _schema: &str,
            _name: &str,
        ) -> Result<(), BackendError> {
            Ok(())
        }
        async fn drop_view_if_exists(
            &self,
            _schema: &str,
            _name: &str,
        ) -> Result<(), BackendError> {
            Ok(())
        }
        async fn get_row_count(&self, _schema: &str, _name: &str) -> Result<usize, BackendError> {
            Ok(self.calls.lock().unwrap().len())
        }
        async fn get_preview(
            &self,
            _schema: &str,
            _name: &str,
            _limit: usize,
        ) -> Result<Vec<arrow::array::RecordBatch>, BackendError> {
            Ok(vec![])
        }
        async fn table_exists(&self, _schema: &str, _name: &str) -> Result<bool, BackendError> {
            Ok(*self.table_exists.lock().unwrap())
        }
        async fn ensure_schema(&self, _schema: &str) -> Result<(), BackendError> {
            Ok(())
        }
        fn dialect(&self) -> SqlDialect {
            SqlDialect::DuckDB
        }
        fn capabilities(&self) -> BackendCapabilities {
            BackendCapabilities::duckdb()
        }
        async fn load_table(
            &self,
            _schema: &str,
            _name: &str,
            _arrow_schema: arrow::datatypes::SchemaRef,
            _batches: Vec<arrow::array::RecordBatch>,
        ) -> Result<(), BackendError> {
            unreachable!("driver does not load tables")
        }
        async fn delete_partitions(
            &self,
            _schema: &str,
            _name: &str,
            _partition: &smelt_backend::PartitionRange,
        ) -> Result<(), BackendError> {
            unreachable!("driver does not delete partitions")
        }
        async fn insert_into_from_query(
            &self,
            _schema: &str,
            _name: &str,
            _sql: &str,
        ) -> Result<(), BackendError> {
            unreachable!("driver does not insert-into")
        }
        async fn merge_into(
            &self,
            _schema: &str,
            _table: &str,
            _source_sql: &str,
            _unique_key: &[String],
        ) -> Result<(), BackendError> {
            unreachable!("driver merges via execute_sql, not native merge_into")
        }
        async fn insert_overwrite(
            &self,
            _schema: &str,
            _table: &str,
            _sql: &str,
            _partition: &smelt_backend::PartitionRange,
        ) -> Result<(), BackendError> {
            unreachable!("driver does not insert-overwrite")
        }
    }

    /// A monoid `SUM`-style rule: always safe, merges via a fixed template.
    struct SumRule;

    impl WindowedKeyedRule for SumRule {
        fn refuse(&self) -> Option<String> {
            None
        }
        fn merge_sql(&self, schema: &str, table: &str, delta_sql: &str) -> String {
            format!("MERGE INTO {}.{} USING ({})", schema, table, delta_sql)
        }
    }

    #[tokio::test]
    async fn refuses_before_any_backend_call() {
        let backend = RecordingBackend::default();
        let steps = driving_steps("2024-01-01", "2024-01-03", &Granularity::Day).unwrap();
        let result = run_windowed_keyed_maintenance(
            &backend,
            "model.under.test",
            "main",
            "t",
            &steps,
            &AlwaysRefuses,
            |step| {
                Ok(format!(
                    "SELECT * FROM src WHERE d = '{}'",
                    step.partition_value
                ))
            },
        )
        .await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("non-monoid combiner"));
        assert!(backend.calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn sequences_create_then_merge_across_partitions_in_temporal_order() {
        let backend = RecordingBackend::default();
        let steps = driving_steps("2024-01-01", "2024-01-04", &Granularity::Day).unwrap();
        run_windowed_keyed_maintenance(
            &backend,
            "model.under.test",
            "main",
            "t",
            &steps,
            &SumRule,
            |step| {
                Ok(format!(
                    "SELECT * FROM src WHERE d = '{}'",
                    step.partition_value
                ))
            },
        )
        .await
        .unwrap();

        let calls = backend.calls.lock().unwrap();
        assert_eq!(calls.len(), 3);
        assert!(calls[0].starts_with("create_table_as:"));
        assert!(calls[0].contains("2024-01-01"));
        assert!(calls[1].starts_with("execute_sql: MERGE INTO main.t"));
        assert!(calls[1].contains("2024-01-02"));
        assert!(calls[2].starts_with("execute_sql: MERGE INTO main.t"));
        assert!(calls[2].contains("2024-01-03"));
    }

    fn yes_cell(scan: ScanClamp) -> PlanCell {
        PlanCell {
            group: "{status}".to_string(),
            trigger: Trigger::UpstreamMutation {
                source: "dim".to_string(),
            },
            corner: smelt_logical::maintenance::Corner::ColumnMerge,
            technique: Technique::ColumnScopedMerge,
            partition_local: PartitionLocal::Yes,
            scans: vec![scan],
            ledger_catch_up: false,
        }
    }

    fn no_cell() -> PlanCell {
        PlanCell {
            group: "{status}".to_string(),
            trigger: Trigger::UpstreamMutation {
                source: "dim".to_string(),
            },
            corner: smelt_logical::maintenance::Corner::ColumnMerge,
            technique: Technique::ColumnScopedMerge,
            partition_local: PartitionLocal::No {
                source: "dim".to_string(),
                why: "unclocked".to_string(),
            },
            scans: vec![],
            ledger_catch_up: false,
        }
    }

    fn dim_scan() -> ScanClamp {
        ScanClamp {
            source: "dim".to_string(),
            column: "changed_at".to_string(),
            before: Seconds::ZERO,
            after: Seconds::hours(24),
        }
    }

    #[test]
    fn decide_dispatch_full_for_partition_local_no() {
        let dispatch = decide_column_merge_dispatch(
            &no_cell(),
            "dim",
            true,
            true,
            &ContributionVerdict::Monotone,
        )
        .expect("PartitionLocal::No + table exists + unique_key must dispatch Full");
        assert_eq!(dispatch, ColumnMergeDispatch::Full);
    }

    #[test]
    fn decide_dispatch_clamped_for_partition_local_yes_with_monotone_contribution() {
        let cell = yes_cell(dim_scan());
        let dispatch =
            decide_column_merge_dispatch(&cell, "dim", true, true, &ContributionVerdict::Monotone)
                .expect(
                "PartitionLocal::Yes + matching scan + monotone contribution must dispatch Clamped",
            );
        assert_eq!(dispatch, ColumnMergeDispatch::Clamped(dim_scan()));
    }

    #[test]
    fn decide_dispatch_none_when_table_missing() {
        assert_eq!(
            decide_column_merge_dispatch(
                &no_cell(),
                "dim",
                false,
                true,
                &ContributionVerdict::Monotone
            ),
            None,
            "a missing target table must fall back to the safe default, never error"
        );
        assert_eq!(
            decide_column_merge_dispatch(
                &yes_cell(dim_scan()),
                "dim",
                false,
                true,
                &ContributionVerdict::Monotone
            ),
            None
        );
    }

    #[test]
    fn decide_dispatch_none_when_unique_key_undeclared() {
        assert_eq!(
            decide_column_merge_dispatch(
                &no_cell(),
                "dim",
                true,
                false,
                &ContributionVerdict::Monotone
            ),
            None
        );
    }

    #[test]
    fn decide_dispatch_none_when_contribution_not_monotone() {
        let cell = yes_cell(dim_scan());
        let refused = ContributionVerdict::Refused("join fans out".to_string());
        assert_eq!(
            decide_column_merge_dispatch(&cell, "dim", true, true, &refused),
            None,
            "a non-monotone contribution must never dispatch Clamped — the whole point of the \
             proof is to refuse a fanned-out join, not merge it approximately"
        );
    }

    #[test]
    fn decide_dispatch_none_when_no_scan_matches_source() {
        // The plan's only scan is for a DIFFERENT source than the one the
        // caller resolved live — must never dispatch on a mismatched scan.
        let mut cell = yes_cell(dim_scan());
        cell.scans[0].source = "other_source".to_string();
        assert_eq!(
            decide_column_merge_dispatch(&cell, "dim", true, true, &ContributionVerdict::Monotone),
            None
        );
    }

    #[test]
    fn widen_horizon_never_narrows_the_batch_window() {
        let scan = dim_scan(); // after = 24h
        let narrower_batch = Seconds::hours(6);
        let bound = widen_horizon_for_batch(&scan, narrower_batch);
        assert_eq!(
            bound,
            BoundResult::Bounded {
                source_partition_col: "changed_at".to_string(),
                before: Seconds::ZERO,
                after: Seconds::hours(24),
            },
            "a batch narrower than the derived scan margin must keep the derived margin"
        );

        let wider_batch = Seconds::days(3);
        let bound = widen_horizon_for_batch(&scan, wider_batch);
        assert_eq!(
            bound,
            BoundResult::Bounded {
                source_partition_col: "changed_at".to_string(),
                before: Seconds::ZERO,
                after: Seconds::days(3),
            },
            "a batch wider than the derived scan margin must widen to the batch width, never \
             silently drop the batch's earlier rows from the merge"
        );
    }

    #[test]
    fn dimension_join_contribution_refuses_with_no_declared_unique_key() {
        let sql = "SELECT e.id, d.status FROM smelt.sources.events e \
                    JOIN smelt.sources.dim d ON e.dim_id = d.id";
        let verdict = dimension_join_contribution(sql, "dim", &[]);
        assert!(
            !verdict.is_monotone(),
            "no declared unique_key must refuse, never optimistically assume monotone"
        );
    }

    #[test]
    fn dimension_join_contribution_proves_monotone_via_declared_unique_key() {
        let sql = "SELECT e.id, d.status FROM smelt.sources.events e \
                    JOIN smelt.sources.dim d ON e.dim_id = d.id";
        let verdict = dimension_join_contribution(sql, "dim", &["id".to_string()]);
        assert!(
            verdict.is_monotone(),
            "an equi-join on the dimension's declared unique_key must prove one-to-one: \
             {verdict:?}"
        );
    }

    #[test]
    fn dimension_join_contribution_refuses_a_fan_out_join() {
        let sql = "SELECT e.id, d.status FROM smelt.sources.events e \
                    JOIN smelt.sources.dim d ON e.dim_id = d.category";
        let verdict = dimension_join_contribution(sql, "dim", &["id".to_string()]);
        assert!(
            !verdict.is_monotone(),
            "the join equates on `category`, not the declared unique_key `id` — this must \
             refuse, never assume one-to-one: {verdict:?}"
        );
    }
}
