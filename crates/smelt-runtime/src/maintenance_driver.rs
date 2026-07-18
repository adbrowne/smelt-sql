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

use crate::transformer::{add_seconds_to_date, subtract_seconds_from_date, TimeRange};
use anyhow::{bail, Context, Result};
use smelt_backend::{
    maintenance_dialect, Backend, BackendError, ExecutionResult, IncrementalStrategy,
};
use smelt_core::config::{CellTechnique, Granularity};
use smelt_dialect::SqlDialect;
use smelt_logical::analysis::join_shape::{ContributionVerdict, JoinContext};
use smelt_logical::analysis::source_bounds::BoundResult;
use smelt_logical::analysis::walk::model_property_vector;
use smelt_logical::maintenance::choice::{resolve_write_suppression, WriteSuppression};
use smelt_logical::maintenance::emit::{
    emit_column_scoped_merge, emit_column_scoped_merge_suppressed, emit_create_table_as,
    MaintenanceStatement, StatementGroup, TargetSlicePredicate,
};
use smelt_logical::maintenance::locality::LocalitySlice;
use smelt_logical::maintenance::{
    MaintenancePlan, PartitionLocal, PlanCell, ScanClamp, SourceFacts, Technique, Trigger,
    WritePattern, WriteSelection,
};
use smelt_state::ddl_duckdb;
use smelt_state::reconciliation::Grade;
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

/// The reconciliation ledger's whole-row group key for the keyed
/// windowed-maintenance driver (`docs/specs/incremental_models.md` §"The
/// reconciliation ledger"). Per-column-group ledger grading for this driver
/// is future narrowing, not correctness-required for MP12: grading the
/// whole cell `Additive` whenever *any* aggregator column is additive is
/// conservative (a merge is refused on repeat even for an idempotent column
/// mixed into the same cell) but never unsafe.
const LEDGER_WHOLE_ROW_GROUP: &str = "{*}";

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
    /// state with one step's compiled delta SQL. `slice` is this step's
    /// target-scan slice predicate, already resolved to concrete bounds by
    /// the driver (`run_windowed_keyed_maintenance`) from the caller's
    /// established `LocalitySlice` — `None` for a keyed model with no
    /// admitted (or no declared) key temporal locality.
    ///
    /// `suppression` is the cell's already-resolved [`WriteSuppression`]
    /// verdict (T1, `docs/plans/20260715-composed-axes-conditional-
    /// maintenance.md` Phase C6): the caller resolves it once, outside the
    /// step loop, from the model's own P2 row identity and P3 change-
    /// comparability over the fold's own output columns — this trait method
    /// does not re-derive admission, only chooses which matched-arm shape to
    /// emit. `slice` and `suppression` compose independently: a composed
    /// (key + time) model's suppressed merge carries both predicates; a bare
    /// keyed model's carries only the suppression arm (never an invented
    /// slice) — the composition itself lives in the single-owner emitter
    /// (`smelt_logical::maintenance::emit::emit_keyed_fold_suppressed`),
    /// this method only threads the two already-resolved values to it.
    fn merge_sql(
        &self,
        schema: &str,
        table: &str,
        delta_sql: &str,
        slice: Option<&TargetSlicePredicate>,
        suppression: &WriteSuppression,
    ) -> String;

    /// The reconciliation ledger's storage grading for this rule's cell
    /// (`docs/specs/incremental_models.md` §"The reconciliation ledger" —
    /// "Storage is graded by algebra"). `Grade::Additive` requires
    /// warehouse-resident delta-identity tracking and never-fold-twice
    /// refusal (MP12); `Grade::Idempotent` needs neither — re-folding a
    /// window is harmless, so no warehouse ledger table is ever created for
    /// an idempotent-only cell. Defaults to `Grade::Idempotent` (no ledger
    /// enforcement) for a rule that doesn't opt in.
    fn ledger_grade(&self) -> Grade {
        Grade::Idempotent
    }

    /// The ledger's `input` key for this rule's deltas — the driving
    /// source's name. Only consulted when [`Self::ledger_grade`] is
    /// `Grade::Additive`.
    fn ledger_input(&self) -> &str {
        ""
    }

    /// Build the out-of-slice match probe SQL for a **checked** route-3
    /// (recurrence-bounded, declared `r`) slice
    /// (`docs/specs/incremental_models.md` §"Key temporal locality", route
    /// 3) — the single-owner emitter is
    /// `smelt_logical::maintenance::emit::emit_recurrence_bound_probe`;
    /// this method's only job is supplying it the rule's own `unique_key`
    /// (which the driver does not otherwise know). `None` refuses the
    /// checked route fail-closed (the driver never silently skips the
    /// check for a rule that cannot build a probe) — the default here
    /// exists only so rules with no keyed shape at all need not implement
    /// it; `keyed`'s own impl (`crate::cumulative::CumulativeClassification`)
    /// always returns `Some`.
    fn recurrence_probe_sql(
        &self,
        schema: &str,
        table: &str,
        delta_sql: &str,
        partition_column: &str,
        slice_lower: &str,
    ) -> Option<String> {
        let _ = (schema, table, delta_sql, partition_column, slice_lower);
        None
    }
}

/// Run the windowed-keyed-maintenance loop: `classify` already happened (its
/// result is `rule`); this steps over `steps` in temporal order, compiles
/// each partition's delta SQL via `compile_step`, and creates the target (on
/// the first step, if it doesn't exist) or merges into it (`rule.merge_sql`)
/// otherwise.
///
/// Fails closed before any backend call if `rule.refuse()` fires.
///
/// **Never-fold-twice (MP12).** When `rule.ledger_grade()` is
/// `Grade::Additive`, every step's create-or-merge action is guarded by the
/// warehouse-resident reconciliation ledger
/// (`docs/specs/incremental_models.md` §"The reconciliation ledger" and
/// §Constraints "Never fold a delta already reflected in the state"): the
/// step's own partition value is its delta identity, folded transactionally
/// with the action via [`Backend::fold_ledger_delta`]. A step whose delta is
/// already reflected — a reprocessed window — refuses the run with a
/// `KeyedReprocessedWindow`-shaped error
/// (`docs/specs/incremental_models.md` §"Reprocessing") instead of silently
/// double-counting. `Grade::Idempotent` cells skip the ledger entirely — no
/// warehouse table is ever created for them.
#[allow(clippy::too_many_arguments)]
pub async fn run_windowed_keyed_maintenance(
    backend: &dyn Backend,
    model_name: &str,
    schema: &str,
    table: &str,
    steps: &[MaintenanceStep],
    rule: &dyn WindowedKeyedRule,
    locality: Option<&LocalitySlice>,
    suppression: &WriteSuppression,
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
    let grade = rule.ledger_grade();

    for (idx, step) in steps.iter().enumerate() {
        let delta_sql = compile_step(step)
            .with_context(|| format!("Failed to compile model: {}", model_name))?;

        let table_exists = backend.table_exists(schema, table).await.unwrap_or(false);

        // The first-run `CREATE TABLE … AS` and the merge both come from
        // the single-owner emitters in `smelt-logical::maintenance::emit`
        // (`docs/specs/incremental_models.md` §"Statement emission (single
        // owner)") — this driver builds no maintenance-statement text of
        // its own.
        let create_group = if !table_exists {
            let qualified_table = format!("{}.{}", schema, table);
            Some(emit_create_table_as(
                &qualified_table,
                &delta_sql,
                smelt_backend::maintenance_dialect(backend.dialect()),
            ))
        } else {
            None
        };
        // Resolve this step's concrete target-scan slice predicate from the
        // caller's established `LocalitySlice` (`docs/specs/
        // incremental_models.md` §"Key temporal locality"). The two routes
        // resolve to structurally different predicates:
        //   - route 1 (`Window`): the step's own partition value, widened
        //     by the derived margins. Date arithmetic is shared with the
        //     source-filter pushdown transform
        //     (`transformer::{subtract,add}_seconds_from_date`) rather than
        //     reimplemented here.
        //   - route 2 (`DeltaValues`): the step's own already-compiled
        //     delta relation's own partition-column values — no date
        //     arithmetic, no widening, since a key-determined column is a
        //     per-key constant regardless of which step reveals it.
        //   - route 3, declared (`RecurrenceBounded`): the same step-
        //     relative window shape as route 1's `Window` (the margin
        //     already folds in the declared `r`), plus — below, before the
        //     merge actually runs — the out-of-slice match probe a checked
        //     bound requires.
        let slice_predicate = locality.map(|slice| match slice {
            LocalitySlice::Window {
                partition_column,
                margin_before,
                margin_after,
            }
            | LocalitySlice::RecurrenceBounded {
                partition_column,
                margin_before,
                margin_after,
                ..
            } => TargetSlicePredicate::Range {
                partition_column: partition_column.clone(),
                lower: subtract_seconds_from_date(&step.partition_value, margin_before.0),
                upper: add_seconds_to_date(&step.partition_value, margin_after.0),
            },
            LocalitySlice::DeltaValues { partition_column } => TargetSlicePredicate::DeltaValues {
                partition_column: partition_column.clone(),
                delta_select: delta_sql.clone(),
            },
        });
        let action_sql = match &create_group {
            Some(group) => group.statements[0].sql.clone(),
            None => rule.merge_sql(
                schema,
                table,
                &delta_sql,
                slice_predicate.as_ref(),
                suppression,
            ),
        };

        // Route 3's declared sub-route (`LocalitySlice::RecurrenceBounded`)
        // is admitted only **checked** (`incremental_models.md` §"Key
        // temporal locality": "A declared `r` is admitted only checked"):
        // before this step's merge action ever runs, probe the target for
        // any delta key that also matches a stored row outside the slice —
        // a violation means the declared bound was wrong, and the run must
        // refuse rather than silently produce an incomplete merge. The
        // probe is read-only and runs first, so a violation never reaches
        // the write path: the target is provably unchanged, satisfying
        // "the run's transaction rolls back" without needing a second,
        // hand-rolled transaction wrapper around the merge itself. A
        // **derived** `r` (`LocalitySlice::Window`) never reaches this
        // branch at all — the check only exists for the declared route.
        if create_group.is_none() {
            if let Some(LocalitySlice::RecurrenceBounded {
                partition_column,
                margin_before,
                ..
            }) = locality
            {
                let slice_lower =
                    subtract_seconds_from_date(&step.partition_value, margin_before.0);
                match rule.recurrence_probe_sql(
                    schema,
                    table,
                    &delta_sql,
                    partition_column,
                    &slice_lower,
                ) {
                    Some(probe_sql) => {
                        let batches = backend.execute_sql(&probe_sql).await.map_err(|e| {
                            anyhow::anyhow!(
                                "Failed to execute recurrence-bound probe for model '{}':\n  \
                                 SQL: {}\n  Error: {}",
                                model_name,
                                probe_sql,
                                e
                            )
                        })?;
                        let rows = crate::check_runner::batches_to_rows(&batches);
                        let violation_count: u64 = rows
                            .first()
                            .and_then(|r| r.get("violation_count"))
                            .ok_or_else(|| {
                                anyhow::anyhow!(
                                    "recurrence-bound probe for model '{}' returned no \
                                     `violation_count` row — refusing to trust an unchecked \
                                     result for a declared key-recurrence bound",
                                    model_name
                                )
                            })?
                            .parse::<u64>()
                            .map_err(|e| {
                                anyhow::anyhow!(
                                    "recurrence-bound probe for model '{}' returned an \
                                     unparseable `violation_count`: {}",
                                    model_name,
                                    e
                                )
                            })?;
                        if violation_count > 0 {
                            let sample_keys = rows
                                .first()
                                .and_then(|r| r.get("sample_keys"))
                                .cloned()
                                .unwrap_or_default();
                            bail!(
                                "KeyedRecurrenceBoundViolated: model '{}' declared a \
                                 key-recurrence bound that {} delta row(s) violate at \
                                 partition {} — matched (or would duplicate) a stored key \
                                 outside the recurrence-bound slice. Sample keys: {}. The run \
                                 is refused before any write (`docs/specs/\
                                 incremental_models.md` §\"Key temporal locality\", route 3).",
                                model_name,
                                violation_count,
                                step.partition_value,
                                sample_keys
                            );
                        }
                    }
                    None => {
                        bail!(
                            "windowed-keyed-maintenance driver refused model '{}': a checked \
                             recurrence-bounded locality slice requires the rule to provide a \
                             recurrence-bound probe, and none was provided — refusing \
                             fail-closed rather than silently skipping the check",
                            model_name
                        );
                    }
                }
            }
        }

        match grade {
            Grade::Additive => {
                // `smelt_state::ddl_duckdb` is the only ledger DDL/DML
                // dialect implemented today (MP12); fail loudly rather than
                // handing another backend DuckDB-flavored SQL it cannot run
                // (`CLAUDE.md` §"Fail-loud discipline").
                if backend.dialect() != SqlDialect::DuckDB {
                    bail!(
                        "{}",
                        BackendError::unsupported(
                            backend.dialect().name(),
                            "additive-fold windowed-keyed maintenance ledger (never-fold-twice)",
                        )
                    );
                }

                let ensure_sql = ddl_duckdb::generate_ledger_table_ddl(schema);
                let insert_sql = ddl_duckdb::generate_ledger_insert_sql(
                    schema,
                    model_name,
                    LEDGER_WHOLE_ROW_GROUP,
                    rule.ledger_input(),
                    &step.partition_value,
                    &step.range.start,
                    &step.range.end,
                );
                let exists_sql = ddl_duckdb::generate_ledger_exists_sql(
                    schema,
                    model_name,
                    LEDGER_WHOLE_ROW_GROUP,
                    rule.ledger_input(),
                    &step.partition_value,
                );

                match backend
                    .fold_ledger_delta(&ensure_sql, &insert_sql, &exists_sql, &action_sql)
                    .await
                {
                    Ok(()) => {}
                    Err(BackendError::AlreadyReflected { message }) => {
                        bail!(
                            "windowed-keyed-maintenance driver refused model '{}': partition \
                             {} from input '{}' is already reflected in the reconciliation \
                             ledger (never-fold-twice — docs/specs/incremental_models.md \
                             §Reprocessing). {}. Mitigations: drop the target table and re-run \
                             for a full rebuild, or perform a manual cascade rebuild.",
                            model_name,
                            step.partition_value,
                            rule.ledger_input(),
                            message
                        );
                    }
                    Err(e) => {
                        bail!(
                            "Failed to execute model '{}':\n  SQL: {}\n  Error: {}",
                            model_name,
                            action_sql,
                            e
                        );
                    }
                }
            }
            Grade::Idempotent => {
                // Both the first-run CREATE and the merge route through
                // `Backend::execute_statement_group` — the single point
                // every emitted maintenance statement flows through
                // (`docs/specs/incremental_models.md` §"Statement emission
                // (single owner)"). The `Additive` branch above is the
                // documented exception: its action statement is interleaved
                // with the reconciliation ledger's own DDL/DML via
                // `fold_ledger_delta`, unchanged by this phase.
                let group = match &create_group {
                    Some(group) => group.clone(),
                    None => StatementGroup {
                        statements: vec![MaintenanceStatement {
                            sql: action_sql.clone(),
                        }],
                        transactional: false,
                    },
                };
                backend.execute_statement_group(&group).await.map_err(|e| {
                    anyhow::anyhow!(
                        "Failed to execute model '{}':\n  SQL: {}\n  Error: {}",
                        model_name,
                        action_sql,
                        e
                    )
                })?;
            }
        }

        debug!(
            "  partition {} ({}/{}) {}",
            step.partition_value,
            idx + 1,
            steps.len(),
            if !table_exists {
                "created target table"
            } else {
                "merged"
            }
        );

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
/// a hardcoded constant (MP11, `docs/specs/incremental_models.md` §"Per-cell
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
        // See the analogous call in `resolve_live_column_scoped_cell` above.
        None,
        // Not (yet) plumbed with declared `key_recurrence` bounds at this
        // call site — this resolver only reads the creation cell's
        // `Technique`, which route 3's declared sub-route does not affect
        // (a locality refusal already yields an empty-cells plan either
        // way, falling back to `backend_default` below).
        &[],
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
/// (`BackendCapabilities::supports_column_scoped_merge`, read via
/// `Backend::capabilities`).
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
/// `incremental_models.md` §"Per-cell admission": a `technique:` pin bypasses
/// the cost model, **never** admission — pinning `rederive_columns` for a
/// cell the plan did not admit (or that a capability-gapped backend cannot
/// run) is a hard, fail-loud error, not a silent fallback to
/// `RegionRecompute`. Absent a pin, an admitted+runnable `ColumnScopedMerge`
/// cell is preferred (the point of this phase — "first live cell where
/// execution differs by column group"); otherwise the safe region-recompute
/// default applies with no error (an unpinned model simply has no
/// column-scoped cell to run yet).
///
/// `write_pin` is an already-validated `cells[].write` registry entry
/// (`smelt_logical::maintenance::resolve_write_pin`'s `Ok` result —
/// registry/capability/equivalence already checked by the caller; this
/// function only asks whether the validated pattern's own
/// [`WriteSelection`] is realizable by THIS narrow (`ColumnScopedMerge` vs
/// `RegionRecompute`) resolver). When present it is consulted **before**
/// `pin` (the `cells[].technique` ladder) and decides the cell alone — same
/// precedence rule as `smelt_logical::maintenance::choice::
/// resolve_cell_choice`'s own write-pin consultation, so the two resolvers
/// agree on which pin wins when a cell carries both. A `write_pin` selecting
/// a technique this resolver has no arm for (`KeyedFold`/`InPlaceUpdate` —
/// this function's own scope is only ever the dimension-merge two-way
/// choice) refuses fail-loud rather than silently falling back to region
/// recompute for a pin that named something else.
pub fn resolve_cell_technique(
    plan: &MaintenancePlan,
    trigger: &Trigger,
    pin: Option<CellTechnique>,
    backend_supports_column_scoped_merge: bool,
) -> Result<ResolvedTechnique> {
    resolve_cell_technique_with_write_pin(
        plan,
        trigger,
        pin,
        None,
        backend_supports_column_scoped_merge,
    )
}

/// [`resolve_cell_technique`] plus an optional already-validated
/// `cells[].write` pin — see that function's doc comment for the full
/// contract and precedence rule. Split out as its own function so the
/// existing `pin`-only call sites (and this module's own unit tests) keep
/// compiling unchanged; production write-pin consultation happens through
/// this entry point once a caller has a resolved [`WritePattern`] in hand.
pub fn resolve_cell_technique_with_write_pin(
    plan: &MaintenancePlan,
    trigger: &Trigger,
    pin: Option<CellTechnique>,
    write_pin: Option<&'static WritePattern>,
    backend_supports_column_scoped_merge: bool,
) -> Result<ResolvedTechnique> {
    let admitted = plan
        .cell_for(trigger)
        .is_some_and(|c| c.technique == Technique::ColumnScopedMerge);
    let live = admitted && backend_supports_column_scoped_merge;

    if let Some(pattern) = write_pin {
        return match pattern.selects() {
            WriteSelection::RegionRecompute => Ok(ResolvedTechnique::RegionRecompute),
            WriteSelection::Technique(Technique::ColumnScopedMerge) if live => {
                Ok(ResolvedTechnique::ColumnScopedMerge)
            }
            WriteSelection::Technique(Technique::ColumnScopedMerge) if admitted => bail!(
                "MaintenanceUnboundedFootprint: write pin '{}' for {trigger:?} resolves to a \
                 column-scoped MERGE admitted by the derived plan, but the target backend does \
                 not support column-scoped MERGE — a capability gap drops the technique from \
                 admission at plan time; refusing rather than silently falling back to a \
                 targeted write at runtime",
                pattern.name
            ),
            WriteSelection::Technique(Technique::ColumnScopedMerge) => bail!(
                "MaintenanceUnboundedFootprint: write pin '{}' for {trigger:?} names a cell the \
                 derived plan did not admit as a column-scoped MERGE — a write pin bypasses the \
                 cost model, never admission (`incremental_models.md` §\"Per-cell write \
                 addressing\"); refusing rather than lowering an unbounded-footprint targeted \
                 write at runtime",
                pattern.name
            ),
            WriteSelection::Technique(other) => bail!(
                "MaintenanceUnboundedFootprint: write pin '{}' for {trigger:?} selects {other:?}, \
                 which this dimension-merge resolver has no lowering for (only ColumnScopedMerge \
                 and the always-available region recompute are reachable here) — refusing rather \
                 than silently substituting a different technique than the one pinned",
                pattern.name
            ),
        };
    }

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
             model, never admission (`incremental_models.md` §\"Per-cell admission\"); refusing \
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
/// Returns the matched source name, its admitted [`PlanCell`], and the
/// resolved [`WriteSuppression`] verdict (T1, `docs/plans/
/// 20260715-composed-axes-conditional-maintenance.md` Phase C4) for the
/// cell's own mutation-sensitive column group, so the caller can pick the
/// right physical primitive from `cell.partition_local` (a genuine
/// `ScanClamp` licenses the horizon-clamped [`execute_column_scoped_merge`];
/// an accepted full scan has no horizon and takes
/// [`execute_column_scoped_merge_full`] instead). `None` when the model
/// carries no maintenance plan, declares no explicitly-mutable source, or no
/// source resolves live — the caller's safe default is the existing
/// region-recompute batch loop, unchanged.
///
/// `WriteSuppression` is resolved here (not re-derived by the caller) from
/// the same `sql`'s P3 change-comparability walk
/// (`smelt_logical::analysis::walk::model_property_vector`, never a fresh ad
/// hoc scan — `architecture.md` §"Property composition walk rule") and the
/// cell's own P2 `row_identity` (already carried on `PlanCell`, C3), folded
/// via `choice::resolve_write_suppression`. The cell's raw column list comes
/// from `result.column_groups` (the same derivation's own `ColumnGroup`s),
/// matched by `PlanCell::group`'s display name — the plan-purity invariant's
/// "derived once, never re-derived" extends to this lookup, not a second
/// column-grouping pass.
pub fn resolve_live_column_scoped_cell(
    sql: &str,
    table: &str,
    metadata: &smelt_core::ModelMetadata,
    sources: &[SourceFacts],
    explicitly_mutable: &HashSet<String>,
    backend_supports_column_scoped_merge: bool,
) -> Option<(String, PlanCell, WriteSuppression)> {
    let result = smelt_db::queries::maintenance::derive_model_maintenance_plan(
        sql,
        table,
        metadata,
        sources,
        explicitly_mutable,
        // Not (yet) plumbed with the driving source's declared granularity
        // at this call site — a keyed model with its own `timeseries:`
        // block fails the locality gate's granularity-equality precondition
        // closed here, same as before this phase (`smelt-db`'s own
        // diagnostic path, `maintenance_plan_diagnostics`, has the real
        // value; the runtime execution path,
        // `smelt-runtime::cumulative::execute_cumulative_aggregate`, is
        // this phase's actual slice-pruning consumer).
        None,
        // Not (yet) plumbed with declared `key_recurrence` bounds at this
        // call site, for the same reason as the granularity `None` above —
        // this resolver only inspects mutation-trigger cells, which key
        // temporal locality's routes do not gate.
        &[],
    )?;
    let cells_cfg: &[smelt_core::config::MaintenanceCellConfig] = metadata
        .maintenance
        .as_ref()
        .map(|m| m.cells.as_slice())
        .unwrap_or(&[]);
    explicitly_mutable.iter().find_map(|source| {
        let trigger = Trigger::UpstreamMutation {
            source: source.clone(),
        };
        // An already-validated `cells[].write` pin for this trigger's cell
        // (`smelt-db`'s pre-execution diagnostic gate already ran
        // `resolve_write_pin`'s registry/capability/equivalence checks — an
        // invalid pin never reaches here, the run would already have been
        // refused with `MaintenanceWritePatternUnavailable`/
        // `MaintenanceWriteAddressingRefused`); this only re-resolves the
        // *name* to its registry entry so `resolve_cell_technique_with_write_pin`
        // can consult which [`smelt_logical::maintenance::WriteSelection`]
        // it maps to, never re-deriving admission itself.
        let write_pin = result.plan.cell_for(&trigger).and_then(|plan_cell| {
            let pin_name = smelt_db::queries::maintenance::matching_write_pin(
                plan_cell,
                &result.column_groups,
                cells_cfg,
            )?;
            smelt_logical::maintenance::lookup_write_pattern(&pin_name)
        });
        let resolved = resolve_cell_technique_with_write_pin(
            &result.plan,
            &trigger,
            None,
            write_pin,
            backend_supports_column_scoped_merge,
        )
        .ok()?;
        if resolved != ResolvedTechnique::ColumnScopedMerge {
            return None;
        }
        let cell = result.plan.cell_for(&trigger)?.clone();
        let group_columns = result
            .column_groups
            .iter()
            .find(|g| g.name() == cell.group)
            .map(|g| g.columns.clone())
            .unwrap_or_default();
        let comparability = model_property_vector(sql, &JoinContext::new())
            .map(|v| v.comparability)
            .unwrap_or_default();
        let suppression =
            resolve_write_suppression(&group_columns, &comparability, &cell.row_identity);
        Some((source.clone(), cell, suppression))
    })
}

/// Execute a live `ColumnScopedMerge` cell whose scan locality is an
/// accepted full scan (`PartitionLocal::No { .. }` with `allow_full_scan`,
/// `incremental_models.md` §"Per-cell admission") — the only shape
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
/// `suppression` is the cell's already-resolved [`WriteSuppression`]
/// verdict ([`resolve_live_column_scoped_cell`]'s own output — this function
/// does not re-derive admission). `WriteSuppression::Suppressed` builds the
/// change-suppressed matched arm ([`emit_column_scoped_merge_suppressed`]);
/// `Unconditional` builds the plain matched arm
/// ([`emit_column_scoped_merge`]), byte-identical to this function's
/// pre-Phase-C4 behaviour. Either way the [`StatementGroup`] is built by the
/// single-owner emitter and handed to [`Backend::execute_statement_group`]
/// directly — never `Backend::merge_into` — so the emitted text is exactly
/// what a backend executes, matching every other technique in this module
/// (`docs/specs/incremental_models.md` §"Statement emission (single
/// owner)").
pub async fn execute_column_scoped_merge_full(
    backend: &dyn Backend,
    schema: &str,
    table: &str,
    unique_key: &[String],
    dimension_batch_sql: &str,
    suppression: &WriteSuppression,
) -> Result<ExecutionResult> {
    let start = Instant::now();
    let full_table = format!("{schema}.{table}");
    let dialect = maintenance_dialect(backend.dialect());
    let group = match suppression {
        WriteSuppression::Suppressed { compared_columns } => emit_column_scoped_merge_suppressed(
            &full_table,
            unique_key,
            dimension_batch_sql,
            compared_columns,
            dialect,
        ),
        WriteSuppression::Unconditional { .. } => {
            emit_column_scoped_merge(&full_table, unique_key, dimension_batch_sql, dialect)
        }
    };
    backend
        .execute_statement_group(&group)
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
/// group through unchanged from the existing target state. `Backend::
/// merge_into`'s default implementation issues the `MERGE`
/// `smelt_logical::maintenance::emit::emit_column_scoped_merge` emits
/// (`docs/specs/incremental_models.md` §"Statement emission (single owner)"),
/// `UPDATE SET *`, which requires the source and target column sets to
/// agree exactly (a column-count mismatch is a hard backend error, not a
/// silent by-name subset) — see that emitter's doc comment for the full
/// contract; passing every other column through unchanged is what keeps
/// the *values* column-scoped even though the physical `SET *` touches
/// every column's assignment.
/// `suppression` is the cell's already-resolved [`WriteSuppression`]
/// verdict, exactly like [`execute_column_scoped_merge_full`]'s own
/// parameter — see that function's doc comment for the emitter/dispatch
/// contract this one shares.
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
    suppression: &WriteSuppression,
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

    let dialect = maintenance_dialect(backend.dialect());
    let group = match suppression {
        WriteSuppression::Suppressed { compared_columns } => emit_column_scoped_merge_suppressed(
            &full_table,
            unique_key,
            &source_sql,
            compared_columns,
            dialect,
        ),
        WriteSuppression::Unconditional { .. } => {
            emit_column_scoped_merge(&full_table, unique_key, &source_sql, dialect)
        }
    };
    backend
        .execute_statement_group(&group)
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
/// `docs/specs/incremental_models.md` §"Per-cell admission") a live
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

    /// The plain unconditional matched arm — the pre-Phase-C6 default for
    /// tests below that don't exercise suppression itself.
    fn unconditional_suppression() -> WriteSuppression {
        WriteSuppression::Unconditional {
            why: "test rule does not exercise write suppression".to_string(),
        }
    }

    /// A rule whose combiner set is never monoid-safe — the driver must
    /// refuse the whole run rather than merge approximately.
    struct AlwaysRefuses;

    impl WindowedKeyedRule for AlwaysRefuses {
        fn refuse(&self) -> Option<String> {
            Some("non-monoid combiner (e.g. MEDIAN) cannot be merged".to_string())
        }
        fn merge_sql(
            &self,
            _schema: &str,
            _table: &str,
            _delta_sql: &str,
            _slice: Option<&TargetSlicePredicate>,
            _suppression: &WriteSuppression,
        ) -> String {
            unreachable!("merge_sql must not be called once refuse() fires")
        }
    }

    /// An in-memory fake backend that records every call it receives so the
    /// driver's classify → step → pushdown → create-or-merge sequencing can
    /// be exercised without a real database.
    struct RecordingBackend {
        table_exists: Mutex<bool>,
        calls: Mutex<Vec<String>>,
        dialect: SqlDialect,
    }

    impl Default for RecordingBackend {
        fn default() -> Self {
            RecordingBackend {
                table_exists: Mutex::new(false),
                calls: Mutex::new(Vec::new()),
                dialect: SqlDialect::DuckDB,
            }
        }
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
            // The `CREATE TABLE … AS` text now arrives here (via the
            // default `execute_statement_group` fallback, since this
            // driver no longer calls `Backend::create_table_as` for this
            // family) rather than through the dedicated `create_table_as`
            // method — flip the same flag a real backend's live
            // `table_exists` query would reflect after running it.
            if sql.starts_with("CREATE TABLE") {
                *self.table_exists.lock().unwrap() = true;
            }
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
            self.dialect
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
        fn merge_sql(
            &self,
            schema: &str,
            table: &str,
            delta_sql: &str,
            _slice: Option<&TargetSlicePredicate>,
            _suppression: &WriteSuppression,
        ) -> String {
            format!("MERGE INTO {}.{} USING ({})", schema, table, delta_sql)
        }
    }

    /// Same as [`SumRule`] but opts into `Grade::Additive` ledger grading
    /// (MP12) — exercises the driver's never-fold-twice wiring without a
    /// real backend.
    struct SumRuleAdditive;

    impl WindowedKeyedRule for SumRuleAdditive {
        fn refuse(&self) -> Option<String> {
            None
        }
        fn merge_sql(
            &self,
            schema: &str,
            table: &str,
            delta_sql: &str,
            _slice: Option<&TargetSlicePredicate>,
            _suppression: &WriteSuppression,
        ) -> String {
            format!("MERGE INTO {}.{} USING ({})", schema, table, delta_sql)
        }
        fn ledger_grade(&self) -> Grade {
            Grade::Additive
        }
        fn ledger_input(&self) -> &str {
            "smelt.events"
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
            None,
            &unconditional_suppression(),
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
            None,
            &unconditional_suppression(),
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
        // The first-run CREATE now comes from `emit_create_table_as`,
        // executed via `execute_statement_group` (its default sequential
        // fallback routes through `execute_sql`, since `RecordingBackend`
        // does not override `execute_statement_group`) — no more
        // `Backend::create_table_as` call for this family
        // (`docs/specs/incremental_models.md` §"Statement emission (single
        // owner)").
        assert!(calls[0].starts_with("execute_sql: CREATE TABLE main.t AS"));
        assert!(calls[0].contains("2024-01-01"));
        assert!(calls[1].starts_with("execute_sql: MERGE INTO main.t"));
        assert!(calls[1].contains("2024-01-02"));
        assert!(calls[2].starts_with("execute_sql: MERGE INTO main.t"));
        assert!(calls[2].contains("2024-01-03"));
    }

    /// MP12: an `Additive`-graded rule routes every step's create-or-merge
    /// action through `Backend::fold_ledger_delta` instead of the plain
    /// `create_table_as`/`execute_sql` path — the never-fold-twice wiring
    /// is reached even without a real database (`RecordingBackend` falls
    /// back to `fold_ledger_delta`'s generic default, which itself calls
    /// `execute_sql` for the ledger DDL/DML and the fold action).
    #[tokio::test]
    async fn additive_grade_routes_through_ledger_fold() {
        let backend = RecordingBackend::default();
        let steps = driving_steps("2024-01-01", "2024-01-02", &Granularity::Day).unwrap();
        run_windowed_keyed_maintenance(
            &backend,
            "model.under.test",
            "main",
            "t",
            &steps,
            &SumRuleAdditive,
            None,
            &unconditional_suppression(),
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
        // The default `fold_ledger_delta` fallback issues ensure + exists +
        // insert + action, all via `execute_sql` — never `create_table_as`,
        // since the ledger-guarded action string carries its own `CREATE
        // TABLE ... AS` text for the create branch.
        assert!(
            calls.iter().any(|c| c.contains("_smelt_ledger")),
            "the ledger table DDL/DML must be issued: {:?}",
            calls
        );
        assert!(
            calls.iter().any(|c| c.contains("CREATE TABLE main.t AS")),
            "the create branch's action must run through the ledger fold: {:?}",
            calls
        );
    }

    /// MP12: the ledger DDL/DML is DuckDB-flavored SQL
    /// (`smelt_state::ddl_duckdb`). An `Additive`-graded rule on a non-DuckDB
    /// backend must fail loudly instead of handing that backend SQL it
    /// cannot run (`CLAUDE.md` §"Fail-loud discipline").
    #[tokio::test]
    async fn additive_grade_on_non_duckdb_backend_fails_loud() {
        let backend = RecordingBackend {
            dialect: SqlDialect::SparkSQL,
            ..Default::default()
        };
        let steps = driving_steps("2024-01-01", "2024-01-02", &Granularity::Day).unwrap();
        let err = run_windowed_keyed_maintenance(
            &backend,
            "model.under.test",
            "main",
            "t",
            &steps,
            &SumRuleAdditive,
            None,
            &unconditional_suppression(),
            |step| {
                Ok(format!(
                    "SELECT * FROM src WHERE d = '{}'",
                    step.partition_value
                ))
            },
        )
        .await
        .unwrap_err();

        assert!(
            backend.calls.lock().unwrap().is_empty(),
            "no SQL must be issued once the dialect guard refuses"
        );
        let message = format!("{err:#}");
        assert!(
            message.contains("Spark SQL"),
            "error must name the unsupported dialect: {message}"
        );
    }

    /// A rule that records the slice predicate it receives from the driver —
    /// used to prove route 2 (key-determined) locality threads a
    /// `LocalitySlice::DeltaValues` through to `merge_sql` as a
    /// `TargetSlicePredicate::DeltaValues` over the step's *own* delta
    /// relation, never a margin-based range
    /// (`docs/specs/incremental_models.md` §"Key temporal locality", route
    /// 2: "the slice is the delta's own partition values — exact
    /// regardless of key age").
    struct CapturingRule {
        captured: Mutex<Vec<Option<TargetSlicePredicate>>>,
    }

    impl WindowedKeyedRule for CapturingRule {
        fn refuse(&self) -> Option<String> {
            None
        }
        fn merge_sql(
            &self,
            schema: &str,
            table: &str,
            delta_sql: &str,
            slice: Option<&TargetSlicePredicate>,
            _suppression: &WriteSuppression,
        ) -> String {
            self.captured.lock().unwrap().push(slice.cloned());
            format!("MERGE INTO {}.{} USING ({})", schema, table, delta_sql)
        }
    }

    #[tokio::test]
    async fn route2_locality_threads_delta_values_slice_over_the_steps_own_delta() {
        let backend = RecordingBackend::default();
        // Three day-steps: the first creates the table (no `merge_sql` call
        // at all — the create branch owns that step); the remaining two
        // exercise `merge_sql` and are the ones this test inspects.
        let steps = driving_steps("2024-01-01", "2024-01-04", &Granularity::Day).unwrap();
        let rule = CapturingRule {
            captured: Mutex::new(Vec::new()),
        };
        let locality = LocalitySlice::DeltaValues {
            partition_column: "first_seen_at".to_string(),
        };
        run_windowed_keyed_maintenance(
            &backend,
            "model.under.test",
            "main",
            "t",
            &steps,
            &rule,
            Some(&locality),
            &unconditional_suppression(),
            |step| {
                Ok(format!(
                    "SELECT id, first_seen_at FROM src WHERE d = '{}'",
                    step.partition_value
                ))
            },
        )
        .await
        .unwrap();

        let captured = rule.captured.lock().unwrap();
        assert_eq!(
            captured.len(),
            2,
            "the two merge steps must each capture a slice: {:?}",
            captured
        );
        for (idx, slice) in captured.iter().enumerate() {
            match slice.as_ref().expect("route 2 must thread a slice") {
                TargetSlicePredicate::DeltaValues {
                    partition_column,
                    delta_select,
                } => {
                    assert_eq!(partition_column, "first_seen_at");
                    // The delta relation threaded through is exactly this
                    // step's own compiled delta — never widened, never a
                    // caller-precomputed range.
                    let expected_date = if idx == 0 { "2024-01-02" } else { "2024-01-03" };
                    assert!(
                        delta_select.contains(expected_date),
                        "step {idx} delta_select must be its own step's delta, got: \
                         {delta_select}"
                    );
                }
                other => panic!(
                    "step {idx}: route 2 must derive a DeltaValues predicate, not a \
                                  Window (margin-based) one: {other:?}"
                ),
            }
        }
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
            row_identity: smelt_logical::maintenance::RowIdentityVerdict {
                identity: smelt_logical::maintenance::RowIdentity::WholeRow,
                proven_mismatch: None,
            },
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
            row_identity: smelt_logical::maintenance::RowIdentityVerdict {
                identity: smelt_logical::maintenance::RowIdentity::WholeRow,
                proven_mismatch: None,
            },
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
