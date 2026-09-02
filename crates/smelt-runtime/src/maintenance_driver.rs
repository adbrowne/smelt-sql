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
use arrow::array::Array;
use smelt_backend::{
    maintenance_dialect, Backend, BackendError, ExecutionResult, IncrementalStrategy,
    PartitionRange,
};
use smelt_core::config::{CellTechnique, Granularity};
use smelt_dialect::SqlDialect;
use smelt_logical::analysis::fingerprint;
use smelt_logical::analysis::fingerprint::Projection as FingerprintProjection;
use smelt_logical::analysis::join_shape::{ContributionVerdict, JoinContext};
use smelt_logical::analysis::source_bounds::BoundResult;
use smelt_logical::analysis::walk::model_property_vector;
use smelt_logical::maintenance::choice::{
    effective_override, resolve_cell_choice, resolve_recompute_restriction,
    resolve_write_suppression, resolve_write_variant, ChosenTechnique, RecomputeRestriction,
    WriteSuppression,
};
use smelt_logical::maintenance::derive::SourceReferentialIntegrity;
use smelt_logical::maintenance::emit::{
    emit_column_scoped_merge, emit_column_scoped_merge_suppressed,
    emit_count_preservation_probe_from_body, emit_create_table_as, emit_delete_insert,
    emit_delete_insert_delta_restricted, emit_fingerprint_digest_select,
    emit_fingerprint_sidecar_diff, emit_per_group_recompute, emit_repair_group_digest_select,
    emit_repair_group_sidecar_diff, emit_staged_candidate_conditional_recompute,
    widened_scan_predicate, MaintenanceDialect, MaintenanceStatement, Region, StatementGroup,
    TargetSlicePredicate,
};
use smelt_logical::maintenance::locality::LocalitySlice;
use smelt_logical::maintenance::repair::{discovery_posture, RepairDiscoveryPosture};
use smelt_logical::maintenance::{
    MaintenancePlan, PartitionLocal, PlanCell, RowIdentity, RowPreservation, ScanClamp,
    SkeletonSourceClosure, SourceFacts, Technique, Trigger, WritePattern, WriteSelection,
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
    ///
    /// `dialect` is the target's maintenance-statement dialect (`docs/specs/
    /// multi_backend.md` §"Whole-row MERGE"), resolved once by the driver
    /// from `backend.dialect()` — the same resolution every other
    /// maintenance statement in this driver uses (`smelt_backend::
    /// maintenance_dialect`). Implementors thread it straight to their own
    /// single-owner emitter call; this trait method never hardcodes a
    /// dialect of its own.
    fn merge_sql(
        &self,
        schema: &str,
        table: &str,
        delta_sql: &str,
        slice: Option<&TargetSlicePredicate>,
        suppression: &WriteSuppression,
        dialect: MaintenanceDialect,
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
    /// (`docs/specs/incremental_shapes.md` §"Key temporal locality", route
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
        dialect: MaintenanceDialect,
    ) -> Option<String> {
        let _ = (
            schema,
            table,
            delta_sql,
            partition_column,
            slice_lower,
            dialect,
        );
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
/// (`docs/specs/incremental_shapes.md` §"Reprocessing") instead of silently
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
    retry: &crate::execute::RetryPolicy<'_>,
    probe_policy: &crate::probes::ProbePolicy,
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
        // incremental_shapes.md` §"Key temporal locality"). The two routes
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
                ..
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
                smelt_backend::maintenance_dialect(backend.dialect()),
            ),
        };

        // Route 3's declared sub-route (`LocalitySlice::RecurrenceBounded`)
        // is admitted only **checked** (`incremental_shapes.md` §"Key
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
                    smelt_backend::maintenance_dialect(backend.dialect()),
                ) {
                    Some(probe_sql) => {
                        let ctx = crate::probes::ProbeContext {
                            probe_code: "KeyedRecurrenceBoundViolated".to_string(),
                            fact: "key_recurrence".to_string(),
                            model: model_name.to_string(),
                            cell: format!("{schema}.{table} keyed merge"),
                            remedy: "correct or widen the declared key-recurrence bound `r`, or \
                                     backfill the affected key"
                                .to_string(),
                        };
                        match crate::probes::dispatch_probe(backend, probe_policy, &ctx, &probe_sql)
                            .await
                            .map_err(|e| anyhow::anyhow!("{e}"))?
                        {
                            crate::probes::ProbeVerdict::Skipped(_)
                            | crate::probes::ProbeVerdict::Held => {}
                            crate::probes::ProbeVerdict::Violated { count, sample_keys } => {
                                bail!(
                                    "KeyedRecurrenceBoundViolated: model '{}' declared a \
                                     key-recurrence bound that {} delta row(s) violate at \
                                     partition {} — matched (or would duplicate) a stored key \
                                     outside the recurrence-bound slice. Sample keys: {}. The run \
                                     is refused before any write (`docs/specs/\
                                     incremental_models.md` §\"Key temporal locality\", route 3).{}",
                                    model_name,
                                    count,
                                    step.partition_value,
                                    sample_keys,
                                    crate::probes::probe_violation_suffix(&ctx)
                                );
                            }
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
                            "KeyedReprocessedWindow: model '{}' refused — partition {} \
                             (window {}..{}) from input '{}' is already reflected in the \
                             reconciliation ledger (never-fold-twice — \
                             docs/specs/incremental_models.md §Reprocessing). {}. Re-run with \
                             `--full-refresh` to rebuild the target from scratch.",
                            model_name,
                            step.partition_value,
                            step.range.start,
                            step.range.end,
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
                crate::execute::retry_backend_call(retry, || {
                    backend.execute_statement_group(&group)
                })
                .await
                .map_err(|e| {
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
        // This resolver only reads the creation (`NewData`) cell — a
        // `ColumnAdded` trigger never affects it, so no deployed-schema
        // snapshot is needed here.
        &[],
        &SourceReferentialIntegrity::new(),
        None,
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

/// Resolve the plain `Trigger::NewData` incremental fold's own per-cell
/// `deferral` scheduling verdict (`docs/outcomes/20260815-definition-delta-
/// migrate/phases/14-plan.md`) — the only trigger family where
/// `contract.cells[].deferral` is validly declarable (`resolve_deferral`
/// requires an interval-representable clock; every other live per-cell
/// dispatch resolver serves an inadmissible trigger, see phase 12's
/// decision log). Thin: derives the model's maintenance plan the same way
/// [`resolve_incremental_strategy`] does, reads its own column groups (the
/// fold's own groups — creation is whole-row, so a group here is exactly
/// one payload column-group the model derives, never re-implemented here),
/// and hands both that and the caller-resolved `cell_decisions` (already
/// licensed via `contract_probes::deferral_cell_decisions`'s own lag
/// comparison — this function makes no independent lag judgement) to
/// `smelt_logical::contract::deferral::fold_deferral_verdict`, the
/// single-owner coverage rule.
///
/// Returns `(Proceed, [])` whenever the model declares no cell-level
/// `deferral` at all, or has no `Trigger::NewData` cell to serve — never a
/// silent skip for an undeclared model.
pub fn resolve_fold_deferral(
    sql: &str,
    table: &str,
    metadata: &smelt_core::ModelMetadata,
    sources: &[SourceFacts],
    explicitly_mutable: &HashSet<String>,
    cell_decisions: &[crate::contract_probes::CellDeferralDecision],
) -> (
    smelt_logical::contract::deferral::FoldDeferralVerdict,
    Vec<String>,
) {
    use smelt_logical::contract::deferral::{
        cell_address, fold_deferral_verdict, DeclaredFoldCell, FoldDeferralVerdict, RunLicense,
    };

    let no_deferral = (FoldDeferralVerdict::Proceed, Vec::new());

    let Some(cells_cfg) = metadata.contract.as_ref().map(|c| c.cells.as_slice()) else {
        return no_deferral;
    };
    let declared: Vec<DeclaredFoldCell> = cells_cfg
        .iter()
        .filter(|cell_cfg| cell_cfg.deferral.is_some())
        .filter_map(|cell_cfg| {
            let address = cell_address(&cell_cfg.columns, &cell_cfg.on);
            let decision = cell_decisions.iter().find(|d| d.address == address)?;
            Some(DeclaredFoldCell {
                address,
                columns: cell_cfg.columns.clone(),
                on: cell_cfg.on.clone(),
                skip_licensed: matches!(decision.license, RunLicense::Skip { .. }),
            })
        })
        .collect();
    if declared.is_empty() {
        return no_deferral;
    }

    let Some(result) = smelt_db::queries::maintenance::derive_model_maintenance_plan(
        sql,
        table,
        metadata,
        sources,
        explicitly_mutable,
        None,
        &[],
        &[],
        &SourceReferentialIntegrity::new(),
        None,
    ) else {
        return no_deferral;
    };
    let Some(creation_source) = result.plan.cells.iter().find_map(|c| match &c.trigger {
        Trigger::NewData { source } => Some(source.clone()),
        _ => None,
    }) else {
        return no_deferral;
    };
    let fold_groups: Vec<(Vec<String>, String)> = result
        .column_groups
        .iter()
        .map(|g| (g.columns.clone(), creation_source.clone()))
        .collect();

    let verdict = fold_deferral_verdict(&declared, &fold_groups);
    let addresses = match &verdict {
        FoldDeferralVerdict::SkipFold { addresses } => addresses.clone(),
        FoldDeferralVerdict::Proceed => Vec::new(),
    };
    (verdict, addresses)
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

/// Legacy two-way (`RegionRecompute`/`ColumnScopedMerge`) resolver, retained
/// **only** for `crates/smelt-runtime/tests/technique_lowering.rs`'s narrow
/// unit coverage of that two-way choice in isolation. It has **zero
/// production call sites**: the live execute path resolves entirely through
/// `smelt_logical::maintenance::choice::resolve_cell_choice`, dispatched from
/// [`resolve_live_column_scoped_cell`] below (Phase 2, `docs/plans/
/// 20260719-prod-w7-bakeoff.md`). `pub` (not `pub(crate)`) only because
/// `technique_lowering.rs` is a `tests/` integration test compiled as a
/// separate crate and needs external visibility; do not add new production
/// callers — extend `resolve_cell_choice` and thread the result through
/// `resolve_live_column_scoped_cell` instead.
///
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
/// Like [`resolve_cell_technique`], this has no production call site — it
/// exists solely for `technique_lowering.rs`'s two-way unit coverage; the
/// live path is `resolve_cell_choice` via [`resolve_live_column_scoped_cell`].
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
            // `diff_patch` is not reachable through this dimension-merge
            // resolver (only `ColumnScopedMerge` and the always-available
            // region recompute are) — no live routing exists for it yet in
            // any case (a later phase's scope), so this refuses the same
            // way the `Technique(other)` arm above does rather than
            // silently substituting a different technique.
            WriteSelection::DiffPatch => bail!(
                "MaintenanceUnboundedFootprint: write pin '{}' for {trigger:?} selects diff_patch, \
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
/// (via `smelt_logical::maintenance::choice::resolve_cell_choice`, see below)
/// in the model's derived [`MaintenancePlan`] — the regular incremental
/// execution loop's per-run
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
///
/// This is the ladder's single production dispatch site for the
/// Fold/Recompute/RederiveColumns family dimension
/// (`smelt_logical::maintenance::choice::resolve_cell_choice`) — a
/// frontmatter `cells[].technique` hard pin or `cells[].prefer` soft
/// preference on this trigger's cell is threaded in via
/// [`effective_override`] and actually consulted, rather than the
/// pin-less two-way resolver this call site used before (Phase 2,
/// `docs/plans/20260719-prod-w7-bakeoff.md`). An inadmissible hard pin
/// surfaces as [`smelt_logical::maintenance::choice::ChoiceRefusal`],
/// mapped here to a real `Err` — the fail-loud discipline (root
/// `CLAUDE.md`) forbids silently falling back to region recompute for a
/// pin the derived plan does not admit.
pub fn resolve_live_column_scoped_cell(
    sql: &str,
    table: &str,
    metadata: &smelt_core::ModelMetadata,
    sources: &[SourceFacts],
    explicitly_mutable: &HashSet<String>,
    backend_supports_column_scoped_merge: bool,
    technique_overrides: &[crate::types::CellTechniqueOverride],
) -> Result<Option<(String, PlanCell, WriteSuppression)>> {
    let Some(result) = smelt_db::queries::maintenance::derive_model_maintenance_plan(
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
        // This resolver only inspects `UpstreamMutation` cells — a
        // `ColumnAdded` trigger never affects them, so no deployed-schema
        // snapshot is needed here.
        &[],
        &SourceReferentialIntegrity::new(),
        None,
    ) else {
        return Ok(None);
    };
    let cells_cfg: &[smelt_core::config::MaintenanceCellConfig] = metadata
        .maintenance
        .as_ref()
        .map(|m| m.cells.as_slice())
        .unwrap_or(&[]);
    // Request overrides enter the SAME `effective_override` ladder as
    // frontmatter `cells[]` entries, converted to the matching shape
    // (`prefer`/`write` left `None` — request scope only carries a hard
    // technique pin). `matching_cell` (in `smelt-logical`, not touched by
    // this phase) is first-match-wins, so request overrides are placed
    // BEFORE the frontmatter cells in the combined slice: that is how
    // "request scope is narrower than file scope" (`docs/plans/
    // 20260719-prod-w7-bakeoff.md` Phase 3, decision B1) is realized —
    // a request override for a cell also pinned in frontmatter is found
    // first and wins.
    let request_cells: Vec<smelt_core::config::MaintenanceCellConfig> = technique_overrides
        .iter()
        .map(|o| smelt_core::config::MaintenanceCellConfig {
            columns: o.columns.clone(),
            on: o.on.clone(),
            prefer: None,
            technique: Some(o.technique),
            write: None,
        })
        .collect();
    let combined_cells: Vec<smelt_core::config::MaintenanceCellConfig> = request_cells
        .iter()
        .cloned()
        .chain(cells_cfg.iter().cloned())
        .collect();
    for source in explicitly_mutable {
        let trigger = Trigger::UpstreamMutation {
            source: source.clone(),
        };
        // A trigger commonly derives MULTIPLE sibling cells, one per
        // membership-sensitive column group a shared join admits
        // (`docs/plans/20260808-membership-sensitivity.md` Phase 1) — every
        // one of them must be offered a chance to match a `cells[]`
        // override scoped to ITS OWN columns, never only the first
        // (`MaintenancePlan::cell_for`'s own doc comment on this exact bug,
        // `docs/plans/20260808-membership-sensitivity.md` Phase 3's fix).
        let sibling_cells: Vec<PlanCell> = result.plan.cells_for(&trigger).cloned().collect();
        if sibling_cells.is_empty() {
            continue;
        }
        let sibling_group_columns: Vec<Vec<String>> = sibling_cells
            .iter()
            .map(|c| {
                result
                    .column_groups
                    .iter()
                    .find(|g| g.name() == c.group)
                    .map(|g| g.columns.clone())
                    .unwrap_or_default()
            })
            .collect();
        // Fail-loud: a HARD `cells[on: source].technique` pin whose
        // `columns` address NONE of this trigger's own sibling groups is a
        // dangling/misconfigured pin — under the pre-Phase-3 first-match
        // lookup it would silently never be consulted by anything; refuse
        // instead of vanishing (root `CLAUDE.md` §"Fail-loud discipline").
        // A soft `prefer` in the same situation is not flagged here — it
        // never refuses even when it names a resolvable technique the cell
        // doesn't have (`resolve_cell_choice`'s own contract).
        if let Some(dangling) = smelt_logical::maintenance::choice::unaddressed_technique_pin(
            &combined_cells,
            source,
            &sibling_group_columns,
        ) {
            bail!(
                "MaintenanceUnboundedFootprint: cells[on: {source}].technique pin (columns: \
                 {:?}) does not address any of this trigger's own derived column groups ({:?}) \
                 — a hard technique pin must name columns belonging to exactly one of the \
                 trigger's admitted cells, never columns absent from every one of them",
                dangling.columns,
                sibling_group_columns,
            );
        }
        for (cell, group_columns) in sibling_cells.iter().zip(sibling_group_columns.iter()) {
            // An already-validated `cells[].write` pin for this cell
            // (`smelt-db`'s pre-execution diagnostic gate already ran
            // `resolve_write_pin`'s registry/capability/equivalence checks —
            // an invalid pin never reaches here, the run would already have
            // been refused with `MaintenanceWritePatternUnavailable`/
            // `MaintenanceWriteAddressingRefused`); this only re-resolves
            // the *name* to its registry entry so `resolve_cell_choice` can
            // consult which [`smelt_logical::maintenance::WriteSelection`]
            // it maps to, never re-deriving admission itself.
            let write_pin = smelt_db::queries::maintenance::matching_write_pin(
                cell,
                &result.column_groups,
                cells_cfg,
            )
            .and_then(|pin_name| smelt_logical::maintenance::lookup_write_pattern(&pin_name));
            // The override ladder (`defaults.prefer` → `cells[].prefer` →
            // `cells[].technique`, narrower scope winning) narrowed to THIS
            // sibling cell's own trigger + column group — the SAME
            // `overrides` value feeds both the family choice below and the
            // write-suppression variant resolution further down, so a
            // `cells[].technique` entry naming e.g. `suppress`/
            // `unconditional` for this cell is visible to both dimensions
            // from one ladder evaluation.
            let overrides = effective_override(
                metadata
                    .maintenance
                    .as_ref()
                    .and_then(|m| m.defaults.as_ref()),
                &combined_cells,
                source,
                group_columns,
            );
            let chosen = resolve_cell_choice(
                Some(cell),
                &trigger,
                &overrides,
                write_pin,
                backend_supports_column_scoped_merge,
            )
            .map_err(|refusal| anyhow::anyhow!(refusal.to_string()))?;
            if chosen != ChosenTechnique::Admitted(Technique::ColumnScopedMerge) {
                continue;
            }
            let comparability = model_property_vector(sql, &JoinContext::new())
                .map(|v| v.comparability)
                .unwrap_or_default();
            let raw_suppression =
                resolve_write_suppression(group_columns, &comparability, &cell.row_identity);
            // Fold the first-build/definition-change-backfill posture (or an
            // explicit `prefer`/`technique` override on this dimension) into
            // the proof: a cell admitted but not preferred (`cell.ledger_catch_up`
            // or `Trigger::Backfill` — no prior stored state on this group to
            // diff against) resolves the unconditional matched arm by default,
            // exactly as if the P2/P3 proof itself had refused — unless an
            // explicit pin/preference overrides that default. This is the
            // resolver's own rule, never a runtime special case here.
            //
            // A `technique: suppress` pin forcing suppression on over a genuine
            // P2/P3 proof failure is a hard `ChoiceRefusal`, propagated as a
            // real run error below — mirroring how the family dimension's
            // own `resolve_cell_choice` refusal above already fails the run,
            // never a silent fallback to region recompute
            // (`incremental_models.md` §"Per-cell write addressing" →
            // "User pins").
            let (suppression, _variant_reason) = resolve_write_variant(
                &raw_suppression,
                &cell.trigger,
                cell.ledger_catch_up,
                &overrides,
            )
            .map_err(|refusal| anyhow::anyhow!(refusal.to_string()))?;
            return Ok(Some((source.clone(), cell.clone(), suppression)));
        }
    }
    Ok(None)
}

/// Resolve a live `Trigger::ColumnAdded` cell that resolves to
/// `Technique::InPlaceUpdate` (`docs/plans/20260809-sensitivity-precision.md`
/// Phase 6, `docs/specs/definition_deltas.md` §"The verdict per column group") — the production entry point for the definition-change
/// trigger, distinct from [`resolve_live_column_scoped_cell`]/
/// [`resolve_live_membership_recompute_cell`] above (which only ever
/// inspect `NewData`/`UpstreamMutation` cells).
///
/// `deployed_column_names` is the caller's own I/O: `smelt-runtime` is the
/// one caller with real access to the deployed-schema snapshot the runtime
/// `schema_evolution` module already reads/writes
/// (`crate::schema_evolution::infer_deployed_columns`/
/// `save_deployed_schema`) — `derive_model_maintenance_plan` itself does no
/// I/O (Salsa-purity rule). An empty slice (no known deployed schema) derives
/// no trigger at all, same as `smelt-db`'s own diagnostic path.
///
/// Returns the admitted cell plus its ready-to-execute `(column,
/// expression)` assignment pairs — the added columns' own defining
/// expressions read straight from the model's current SQL via
/// [`smelt_logical::maintenance::derive::column_def_from_sql`], the SAME
/// source [`crate::diagnostics::build_technique_statements`]'s
/// `Technique::InPlaceUpdate` preview arm reads, and the same source the
/// `PureBackfill` classification (`smelt_logical::analysis::
/// definition_change::classify_definition_change`) was proven against —
/// never a fresh re-derivation of either the trigger or the assignments.
/// `None` when the model carries no maintenance plan, no deployed snapshot
/// is known, or no cell resolves to `InPlaceUpdate` (no `ColumnAdded`
/// trigger fired, the added column(s) classified `UpstreamRederive`, or a
/// skeleton add refused).
pub fn resolve_live_in_place_update_cell(
    sql: &str,
    table: &str,
    metadata: &smelt_core::ModelMetadata,
    sources: &[SourceFacts],
    deployed_column_names: &[String],
) -> Option<(PlanCell, Vec<(String, String)>)> {
    if deployed_column_names.is_empty() {
        return None;
    }
    let result = smelt_db::queries::maintenance::derive_model_maintenance_plan(
        sql,
        table,
        metadata,
        sources,
        &HashSet::new(),
        None,
        &[],
        deployed_column_names,
        &SourceReferentialIntegrity::new(),
        None,
    )?;
    let cell = result
        .plan
        .cells
        .iter()
        .find(|c| {
            matches!(c.trigger, Trigger::ColumnAdded { .. })
                && c.technique == Technique::InPlaceUpdate
        })?
        .clone();
    let Trigger::ColumnAdded { columns } = &cell.trigger else {
        unreachable!("filtered above")
    };
    let mut assignments = Vec::with_capacity(columns.len());
    for col in columns {
        let def = smelt_logical::maintenance::derive::column_def_from_sql(sql, col)?;
        assignments.push((col.clone(), def.expr.syntax().text().to_string()));
    }
    Some((cell, assignments))
}

/// Find the first `explicitly_mutable` source whose `Trigger::
/// UpstreamMutation` cell resolves live to `Technique::DeleteInsert` over a
/// proven `RowIdentity::Key` — the membership-sensitive counterpart of
/// [`resolve_live_column_scoped_cell`] above, added for the **keyed run
/// loop only** (`docs/plans/20260808-membership-sensitivity.md` Phase 2).
///
/// Per `incremental_models.md` §"The plan matrix": "A membership-sensitive
/// group … must be repaired by a technique that can create and delete rows:
/// the recompute family (delete+insert, change-suppressed where the staged
/// candidate is comparable), never a column-scoped merge, which cannot fix
/// which rows exist." `derive_model_maintenance_plan` (Phase 1 of that plan)
/// now assigns exactly such a cell `Technique::DeleteInsert` +
/// `Corner::RecomputeRegion` for a membership-sensitive column group.
///
/// A `Technique::DeleteInsert` cell is deliberately **not** surfaced here
/// unless the cell's own [`RowIdentity`] proved a real `Key(_)` — a `grain:
/// partition` output's `WholeRow` identity has no key
/// `smelt_logical::maintenance::emit::emit_staged_candidate_conditional` can
/// join stored rows to candidate rows on (that emitter panics on an empty
/// key), and the whole-row `EXCEPT ALL`-both-ways realisation for a keyless
/// region remains unbuilt (`docs/specs/model_transforms.md` §Known
/// Divergences). A `grain: partition` model's `DeleteInsert` membership cell
/// is left to the existing unconditional region `DELETE`+`INSERT` batch loop
/// (`execute.rs`'s plain incremental path, unchanged by this phase) — the
/// always-correct, always-available fallback the plan matrix names for
/// exactly this shape.
///
/// This function only ever surfaces a cell when [`resolve_write_variant`]
/// resolves `WriteSuppression::Suppressed` — `emit_staged_candidate_
/// conditional` has no unconditional counterpart (unlike the column-scoped
/// `MERGE` family), so an `Unconditional`/refused verdict falls through to
/// `None`, same fail-soft posture `resolve_live_column_scoped_cell` already
/// has for its own write-variant dimension (see that function's own doc
/// comment on the "known gap" this mirrors).
///
/// **Departed keys.** Dispatches to [`smelt_logical::maintenance::emit::
/// emit_staged_candidate_conditional_recompute`] (`docs/plans/
/// 20260808-membership-sensitivity.md` Phase 3), not the region-scoped
/// [`smelt_logical::maintenance::emit::emit_staged_candidate_conditional`] —
/// this resolver's `candidate_select` is always the model's own FULL
/// (unwindowed) recompute, so a stored row whose key is entirely absent from
/// it has genuinely *departed* (e.g. the dimension row a fact joined on was
/// itself deleted) rather than merely being out of a run's touched region,
/// and the recompute variant's extra anti-join `DELETE` removes it.
/// The write route a live membership-sensitive `Technique::DeleteInsert`
/// cell ([`resolve_live_membership_recompute_cell`]) resolves to — the
/// staged-candidate conditional recompute
/// ([`execute_staged_membership_recompute`]), or a `write: diff_patch` pin
/// over that same cell, routed through [`execute_diff_patch`] instead
/// (`docs/outcomes/20260815-definition-delta-migrate/phases/12-plan.md`).
/// The `diff_patch` leg's delete-leg completeness is sound unconditionally:
/// this resolver's candidate is always the model's own FULL (unwindowed)
/// recompute — a region recompute's own coverage IS its slice, by
/// construction, mirroring `resolve_cell_choice`'s `DeleteLeg::Complete`
/// grant for the region `DeleteInsert` default.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MembershipRecomputeWrite {
    /// `emit_staged_candidate_conditional_recompute`'s change-suppressed
    /// conditional `DELETE`+`INSERT`.
    StagedRecompute { compared_columns: Vec<String> },
    /// [`emit_diff_patch`]'s diff-then-patch pattern, admitted via
    /// [`smelt_logical::maintenance::diff_patch::admit_diff_patch`].
    DiffPatch { compared_columns: Vec<String> },
}

/// A live membership-recompute cell as
/// [`resolve_live_membership_recompute_cell`] returns it: the trigger's
/// source name, the cell itself, its column group's own derived columns,
/// and the resolved write route.
pub type LiveMembershipRecomputeCell = (String, PlanCell, Vec<String>, MembershipRecomputeWrite);

pub fn resolve_live_membership_recompute_cell(
    sql: &str,
    table: &str,
    metadata: &smelt_core::ModelMetadata,
    sources: &[SourceFacts],
    explicitly_mutable: &HashSet<String>,
    technique_overrides: &[crate::types::CellTechniqueOverride],
) -> Result<Option<LiveMembershipRecomputeCell>> {
    let Some(result) = smelt_db::queries::maintenance::derive_model_maintenance_plan(
        sql,
        table,
        metadata,
        sources,
        explicitly_mutable,
        None,
        &[],
        // This resolver only inspects `UpstreamMutation` cells — a
        // `ColumnAdded` trigger never affects them, so no deployed-schema
        // snapshot is needed here.
        &[],
        &SourceReferentialIntegrity::new(),
        None,
    ) else {
        return Ok(None);
    };
    let cells_cfg: &[smelt_core::config::MaintenanceCellConfig] = metadata
        .maintenance
        .as_ref()
        .map(|m| m.cells.as_slice())
        .unwrap_or(&[]);
    let request_cells: Vec<smelt_core::config::MaintenanceCellConfig> = technique_overrides
        .iter()
        .map(|o| smelt_core::config::MaintenanceCellConfig {
            columns: o.columns.clone(),
            on: o.on.clone(),
            prefer: None,
            technique: Some(o.technique),
            write: None,
        })
        .collect();
    let combined_cells: Vec<smelt_core::config::MaintenanceCellConfig> = request_cells
        .iter()
        .cloned()
        .chain(cells_cfg.iter().cloned())
        .collect();
    for source in explicitly_mutable {
        let trigger = Trigger::UpstreamMutation {
            source: source.clone(),
        };
        // Same sibling-cell fix as `resolve_live_column_scoped_cell` above
        // (`docs/plans/20260808-membership-sensitivity.md` Phase 3) — a
        // trigger can derive multiple membership-sensitive sibling cells,
        // and a `cells[]` override must be matched against each one's own
        // columns, never only the first.
        let sibling_cells: Vec<PlanCell> = result.plan.cells_for(&trigger).cloned().collect();
        if sibling_cells.is_empty() {
            continue;
        }
        let sibling_group_columns: Vec<Vec<String>> = sibling_cells
            .iter()
            .map(|c| {
                result
                    .column_groups
                    .iter()
                    .find(|g| g.name() == c.group)
                    .map(|g| g.columns.clone())
                    .unwrap_or_default()
            })
            .collect();
        if let Some(dangling) = smelt_logical::maintenance::choice::unaddressed_technique_pin(
            &combined_cells,
            source,
            &sibling_group_columns,
        ) {
            bail!(
                "MaintenanceUnboundedFootprint: cells[on: {source}].technique pin (columns: \
                 {:?}) does not address any of this trigger's own derived column groups ({:?}) \
                 — a hard technique pin must name columns belonging to exactly one of the \
                 trigger's admitted cells, never columns absent from every one of them",
                dangling.columns,
                sibling_group_columns,
            );
        }
        for (cell, group_columns) in sibling_cells.iter().zip(sibling_group_columns.iter()) {
            if cell.technique != Technique::DeleteInsert {
                continue;
            }
            let RowIdentity::Key(key) = &cell.row_identity.identity else {
                continue;
            };
            if key.is_empty() {
                continue;
            }
            let write_pin = smelt_db::queries::maintenance::matching_write_pin(
                cell,
                &result.column_groups,
                cells_cfg,
            )
            .and_then(|pin_name| smelt_logical::maintenance::lookup_write_pattern(&pin_name));
            let overrides = effective_override(
                metadata
                    .maintenance
                    .as_ref()
                    .and_then(|m| m.defaults.as_ref()),
                &combined_cells,
                source,
                group_columns,
            );
            // `resolve_cell_choice`'s resolvable set for this cell is `{recompute,
            // DeleteInsert}` (the cell's own admitted technique IS the always-
            // available region recompute for this family, per `resolve_cell_
            // choice`'s own doc comment: "the second live alternative … is the
            // always-admissible whole-region recompute"). Absent an override that
            // asks for something this narrow resolver has no lowering for, both
            // resolvable members land here as `Admitted(Technique::DeleteInsert)`
            // — a `RegionRecompute` choice from a `technique: recompute` pin/
            // `prefer` is handled the same way `resolve_live_column_scoped_cell`
            // handles it: it simply isn't THIS live cell, so this source is
            // skipped and the caller's own default (the plain incremental batch
            // loop, unaware of this dimension) applies.
            let chosen = resolve_cell_choice(
                Some(cell),
                &trigger,
                &overrides,
                write_pin,
                // Column-scoped MERGE backend capability is irrelevant to this
                // resolver's own resolvable set (`{recompute, DeleteInsert}`
                // never contains `ColumnScopedMerge`) — passed `false` so a
                // `write_pin`/pin naming `ColumnScopedMerge` correctly refuses
                // here rather than appearing spuriously "live".
                false,
            )
            .map_err(|refusal| anyhow::anyhow!(refusal.to_string()))?;
            let comparability = model_property_vector(sql, &JoinContext::new())
                .map(|v| v.comparability)
                .unwrap_or_default();
            match chosen {
                ChosenTechnique::Admitted(Technique::DeleteInsert) => {
                    let raw_suppression = resolve_write_suppression(
                        group_columns,
                        &comparability,
                        &cell.row_identity,
                    );
                    // A `technique: suppress` pin whose P2/P3 proof refused
                    // propagates as a real run error (`incremental_models.md`
                    // §"Per-cell write addressing" → "User pins") — never a
                    // silent fallback to region recompute.
                    let (suppression, _variant_reason) = resolve_write_variant(
                        &raw_suppression,
                        &cell.trigger,
                        cell.ledger_catch_up,
                        &overrides,
                    )
                    .map_err(|refusal| anyhow::anyhow!(refusal.to_string()))?;
                    // `emit_staged_candidate_conditional` has no
                    // unconditional counterpart (unlike
                    // `emit_column_scoped_merge`/`emit_column_scoped_merge_
                    // suppressed`) — an `Unconditional` verdict here has no
                    // sound lowering this resolver can hand the caller, so
                    // it is treated exactly like a refused write-variant:
                    // skip this source, fall through to the caller's safe
                    // default.
                    let WriteSuppression::Suppressed { compared_columns } = suppression else {
                        continue;
                    };
                    return Ok(Some((
                        source.clone(),
                        cell.clone(),
                        group_columns.clone(),
                        MembershipRecomputeWrite::StagedRecompute { compared_columns },
                    )));
                }
                ChosenTechnique::DiffPatch {
                    recompute: Technique::DeleteInsert,
                    delete_leg,
                } => {
                    // The candidate this resolver's caller writes is always
                    // the model's own FULL unwindowed recompute — a region
                    // recompute's own coverage IS its slice, so the delete
                    // leg is sound regardless of what `resolve_cell_choice`
                    // proved (it already grants `Complete` here too, but
                    // this arm does not depend on that — it re-derives its
                    // own completeness argument rather than trusting an
                    // upstream `Omitted` it would otherwise have to refuse
                    // on for no real reason).
                    let _ = delete_leg;
                    let admitted = smelt_logical::maintenance::diff_patch::admit_diff_patch(
                        group_columns,
                        &comparability,
                        &cell.row_identity,
                        Ok(()),
                    )
                    .map_err(|refusal| {
                        anyhow::anyhow!(
                            "MaintenanceDiffPatchRefused: a `write: diff_patch` pin over a \
                             membership-sensitive Technique::DeleteInsert cell for group '{}' \
                             could not be admitted: {refusal:?}",
                            cell.group,
                        )
                    })?;
                    return Ok(Some((
                        source.clone(),
                        cell.clone(),
                        group_columns.clone(),
                        MembershipRecomputeWrite::DiffPatch {
                            compared_columns: admitted.compared_columns,
                        },
                    )));
                }
                _ => continue,
            }
        }
    }
    Ok(None)
}

/// Execute a live, membership-sensitive `Technique::DeleteInsert` cell
/// (`resolve_live_membership_recompute_cell` above) via the staged-candidate
/// conditional `DELETE`+`INSERT`, full-recompute variant
/// (`smelt_logical::maintenance::emit::
/// emit_staged_candidate_conditional_recompute`) — the "full-model recompute
/// staged, change-suppressed where comparable" realisation
/// `incremental_models.md` §"The plan matrix" names for a
/// membership-sensitive group. `key` is the cell's own proven
/// `RowIdentity::Key` (never `WholeRow` — the caller only reaches here when
/// the resolver above already proved a real key); `candidate_select` is the
/// model's own FULL (unwindowed) recompiled SQL — the entire current
/// admitted+enriched state, not a time-windowed slice — so a departed OR
/// newly-admitted key is represented correctly, and the recompute variant's
/// own anti-join `DELETE` removes a departed key rather than leaving it
/// stale. `compared_columns` is the already fail-closed-admitted
/// `WriteSuppression::Suppressed` set.
pub async fn execute_staged_membership_recompute(
    backend: &dyn Backend,
    schema: &str,
    table: &str,
    key: &[String],
    candidate_select: &str,
    compared_columns: &[String],
    retry: &crate::execute::RetryPolicy<'_>,
) -> Result<ExecutionResult> {
    let start = Instant::now();
    let full_table = format!("{schema}.{table}");
    let dialect = maintenance_dialect(backend.dialect());
    let staged_relation = format!("__smelt_staged_{table}");
    let group = emit_staged_candidate_conditional_recompute(
        &full_table,
        &staged_relation,
        key,
        candidate_select,
        compared_columns,
        dialect,
    );
    crate::execute::retry_backend_call(retry, || backend.execute_statement_group(&group))
        .await
        .map_err(|e| {
            anyhow::anyhow!("staged-candidate membership recompute failed for '{full_table}': {e}")
        })?;
    let row_count = backend.get_row_count(schema, table).await.unwrap_or(0);
    Ok(ExecutionResult {
        model_name: table.to_string(),
        duration: start.elapsed(),
        row_count,
        preview: None,
    })
}

/// Which write leg a live `Technique::PerGroupRecompute` cell resolves to —
/// the repair family's own targeted `DELETE`+`INSERT`
/// ([`execute_per_group_recompute`]), or a `write: diff_patch` pin over that
/// same cell ([`execute_diff_patch`]). Both read the identical affected-key
/// set, candidate select and key — only the write leg differs
/// (`docs/outcomes/20260809-repair-family/phases/07-plan.md`), so this is a
/// mode carried alongside the resolved cell rather than a second resolver.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepairWrite {
    /// `ChosenTechnique::Admitted(Technique::PerGroupRecompute)` — the
    /// repair family's own targeted delete+insert.
    TargetedDeleteInsert,
    /// `ChosenTechnique::DiffPatch { recompute: Technique::PerGroupRecompute,
    /// .. }`, admitted via [`smelt_logical::maintenance::diff_patch::
    /// admit_diff_patch`].
    DiffPatch {
        compared_columns: Vec<String>,
        delete_leg: smelt_logical::maintenance::diff_patch::DeleteLeg,
    },
}

/// How a live `Technique::PerGroupRecompute` cell discovers its affected-key
/// relation (P9, `docs/specs/incremental_models.md` §"The repair family" —
/// "Obligation 7 over a `mutable_snapshot` source"): the append-only clamped
/// rescan, or — for a [`smelt_logical::maintenance::MutationProfile::MutableSnapshot`] source with no
/// native change feed — the group-grain fingerprint sidecar diff, the only
/// route that can witness a group whose entire window contribution departed
/// the source (a vanished group leaves no row for a clamped scan to select,
/// but the sidecar keeps a stored comparandum for it).
#[derive(Debug, Clone)]
pub enum RepairDiscovery {
    /// The append-only widened-scan `SELECT DISTINCT`
    /// ([`repair_affected_keys_select`]).
    ClampedScan,
    /// The group-grain sidecar diff ([`diff_repair_group_sidecar_changed_keys`]).
    /// `digest_columns` is the P4 fingerprint projection's column set for
    /// this source, read straight off the cell's own derived
    /// `fingerprint_projections` — never re-derived.
    SidecarDiff { digest_columns: Vec<String> },
}

/// A live repair cell as [`resolve_live_per_group_recompute_cell`] returns
/// it: the trigger's source name, the cell itself, its proven
/// `RowIdentity::Key`, the cell's own derived bounded read slice, the
/// resolved write leg ([`RepairWrite`]), and how its affected-key relation
/// is discovered ([`RepairDiscovery`]).
pub type LiveRepairCell = (
    String,
    PlanCell,
    Vec<String>,
    ScanClamp,
    RepairWrite,
    RepairDiscovery,
);

/// Resolve a `Technique::PerGroupRecompute` cell's already-chosen
/// [`ChosenTechnique`] (`resolve_cell_choice`'s output — the override ladder
/// has already run) into the [`RepairWrite`] this lowering executes, or
/// `Ok(None)` when `chosen` is not this cell's live technique at all
/// (`RegionRecompute`, or `Admitted` of a technique other than
/// `PerGroupRecompute` — never reachable in practice since
/// [`resolve_live_per_group_recompute_cell`] only ever calls this with a
/// cell whose own `technique` is `PerGroupRecompute`, but a defensive arm
/// nonetheless).
///
/// Pure and independently unit-testable — split out of
/// `resolve_live_per_group_recompute_cell`'s loop body so the fail-loud
/// `DiffPatch { recompute: <not PerGroupRecompute> }` arm ("a `diff_patch`
/// pin over the region `DeleteInsert` default fails loud by name rather
/// than falling through to the default write",
/// `docs/outcomes/20260809-repair-family/phases/07-plan.md`) is exercisable
/// without needing a full plan derivation to reach it.
pub fn resolve_repair_write(
    chosen: &ChosenTechnique,
    group_columns: &[String],
    comparability: &[smelt_logical::analysis::walk::ColumnComparability],
    row_identity: &smelt_logical::maintenance::RowIdentityVerdict,
    group_label: &str,
) -> Result<Option<RepairWrite>> {
    match chosen {
        ChosenTechnique::Admitted(Technique::PerGroupRecompute) => {
            Ok(Some(RepairWrite::TargetedDeleteInsert))
        }
        ChosenTechnique::DiffPatch {
            recompute: Technique::PerGroupRecompute,
            delete_leg,
        } => {
            let slice_complete = match delete_leg {
                smelt_logical::maintenance::diff_patch::DeleteLeg::Complete => Ok(()),
                smelt_logical::maintenance::diff_patch::DeleteLeg::Omitted { why } => {
                    Err(why.clone())
                }
            };
            let admitted = smelt_logical::maintenance::diff_patch::admit_diff_patch(
                group_columns,
                comparability,
                row_identity,
                slice_complete,
            )
            .map_err(|refusal| {
                anyhow::anyhow!(
                    "MaintenanceDiffPatchRefused: a `write: diff_patch` pin over a \
                     Technique::PerGroupRecompute cell for group '{group_label}' could not be \
                     admitted: {refusal:?}"
                )
            })?;
            Ok(Some(RepairWrite::DiffPatch {
                compared_columns: admitted.compared_columns,
                delete_leg: admitted.delete_leg,
            }))
        }
        ChosenTechnique::DiffPatch { recompute, .. } => {
            bail!(
                "MaintenanceDiffPatchUnroutable: a `write: diff_patch` pin over group \
                 '{group_label}' resolved over technique {recompute:?} — only \
                 Technique::PerGroupRecompute has a diff_patch lowering today; the region \
                 DeleteInsert default fails loud rather than silently falling through to the \
                 default write",
            );
        }
        ChosenTechnique::RegionRecompute | ChosenTechnique::Admitted(_) => Ok(None),
    }
}

/// The proven group key of a `Technique::PerGroupRecompute` cell — the
/// repair family's fail-loud identity check
/// (`docs/specs/incremental_models.md` §"The repair family").
///
/// `smelt_logical::maintenance::repair::derive_repair_cell` only ever builds
/// a `RowIdentity::Key`, so a `WholeRow` (or empty-key) identity on a
/// per-group-recompute cell is an internal inconsistency, never a shape this
/// lowering may quietly widen: a repair with no group key to restrict its
/// `DELETE`/`INSERT` to would rewrite every stored row. Refuse by name
/// instead (root `CLAUDE.md` §"Fail-loud discipline"), never a skip.
pub fn repair_cell_key(cell: &PlanCell) -> Result<Vec<String>> {
    let RowIdentity::Key(key) = &cell.row_identity.identity else {
        bail!(
            "MaintenanceRepairIdentityUnproven: a Technique::PerGroupRecompute cell for group \
             '{}' carries RowIdentity::WholeRow — the repair family recomputes whole key \
             groups and has no meaning without a proven group key; widening it to every stored \
             row is never the fallback",
            cell.group
        );
    };
    if key.is_empty() {
        bail!(
            "MaintenanceRepairIdentityUnproven: a Technique::PerGroupRecompute cell for group \
             '{}' carries an empty RowIdentity::Key — the repair family recomputes whole key \
             groups and has no meaning without a proven group key",
            cell.group
        );
    }
    Ok(key.clone())
}

/// Find the first source whose `Trigger::NewData` cell resolves live to
/// `Technique::PerGroupRecompute` — the repair family's counterpart of
/// [`resolve_live_column_scoped_cell`]/[`resolve_live_membership_recompute_cell`]
/// (`docs/specs/incremental_models.md` §"The repair family").
///
/// Unlike those two, this resolver scans the model's **own driving/fold**
/// trigger, not an enrichment dimension's mutation trigger:
/// `derive_new_data`'s key-grain branch narrows a faithful-fold
/// source-posture refusal into a repair cell attached to that same
/// `Trigger::NewData { source }`
/// (`smelt_logical::maintenance::derive::derive_new_data`). The repair cell
/// is therefore an **alternative to** the `KeyedFold` cell for that trigger,
/// not a technique dispatched alongside one — which is why the keyed run
/// loop routes it *instead of* the cumulative fold rather than after it.
/// `Trigger::UpstreamMutation` cells are scanned too, so a future derivation
/// that admits repair for a mutation trigger needs no second resolver.
///
/// Returns the source name, the cell, its proven `RowIdentity::Key`
/// ([`repair_cell_key`] — a `WholeRow` identity is a fail-loud `bail!`,
/// never a skip) and the cell's own derived [`ScanClamp`] (the bounded
/// per-group read slice `repair::admit_per_group_recompute` proved as
/// obligation 4), and how its affected-key relation is discovered
/// ([`RepairDiscovery`]). The plan is derived exactly once here
/// (maintenance-plan purity, root `CLAUDE.md`); nothing downstream
/// re-derives admission.
///
/// `dialect` gates [`RepairDiscovery::SidecarDiff`]: a
/// [`smelt_logical::maintenance::MutationProfile::MutableSnapshot`] source routes to the group-grain
/// sidecar diff, which is DuckDB-only (matching the per-row sidecar's own
/// posture, `diff_fingerprint_sidecar_changed_keys`) — a non-DuckDB target
/// fails loud here, before any backend call, rather than silently falling
/// back to the unsound current-source scan.
pub fn resolve_live_per_group_recompute_cell(
    sql: &str,
    table: &str,
    metadata: &smelt_core::ModelMetadata,
    sources: &[SourceFacts],
    explicitly_mutable: &HashSet<String>,
    technique_overrides: &[crate::types::CellTechniqueOverride],
    dialect: SqlDialect,
) -> Result<Option<LiveRepairCell>> {
    let Some(result) = smelt_db::queries::maintenance::derive_model_maintenance_plan(
        sql,
        table,
        metadata,
        sources,
        explicitly_mutable,
        // Same `None`/`&[]` posture as the sibling resolvers above: the
        // repair cell's own admission (affected-key discovery + bounded
        // slice) reads neither the driving source's declared granularity,
        // nor declared key recurrences, nor a deployed-schema snapshot.
        None,
        &[],
        &[],
        &SourceReferentialIntegrity::new(),
        None,
    ) else {
        return Ok(None);
    };
    let cells_cfg: &[smelt_core::config::MaintenanceCellConfig] = metadata
        .maintenance
        .as_ref()
        .map(|m| m.cells.as_slice())
        .unwrap_or(&[]);
    let request_cells: Vec<smelt_core::config::MaintenanceCellConfig> = technique_overrides
        .iter()
        .map(|o| smelt_core::config::MaintenanceCellConfig {
            columns: o.columns.clone(),
            on: o.on.clone(),
            prefer: None,
            technique: Some(o.technique),
            write: None,
        })
        .collect();
    let combined_cells: Vec<smelt_core::config::MaintenanceCellConfig> = request_cells
        .iter()
        .cloned()
        .chain(cells_cfg.iter().cloned())
        .collect();

    for facts in sources {
        for trigger in [
            Trigger::NewData {
                source: facts.name.clone(),
            },
            Trigger::UpstreamMutation {
                source: facts.name.clone(),
            },
        ] {
            let sibling_cells: Vec<PlanCell> = result.plan.cells_for(&trigger).cloned().collect();
            if sibling_cells.is_empty() {
                continue;
            }
            let sibling_group_columns: Vec<Vec<String>> = sibling_cells
                .iter()
                .map(|c| {
                    result
                        .column_groups
                        .iter()
                        .find(|g| g.name() == c.group)
                        .map(|g| g.columns.clone())
                        .unwrap_or_default()
                })
                .collect();
            // Same dangling-pin fail-loud gate the sibling resolvers apply
            // (`docs/plans/20260808-membership-sensitivity.md` Phase 3).
            if let Some(dangling) = smelt_logical::maintenance::choice::unaddressed_technique_pin(
                &combined_cells,
                &facts.name,
                &sibling_group_columns,
            ) {
                bail!(
                    "MaintenanceUnboundedFootprint: cells[on: {}].technique pin (columns: \
                     {:?}) does not address any of this trigger's own derived column groups \
                     ({:?}) — a hard technique pin must name columns belonging to exactly one \
                     of the trigger's admitted cells, never columns absent from every one of \
                     them",
                    facts.name,
                    dangling.columns,
                    sibling_group_columns,
                );
            }
            for (cell, group_columns) in sibling_cells.iter().zip(sibling_group_columns.iter()) {
                if cell.technique != Technique::PerGroupRecompute {
                    continue;
                }
                // Fail-loud BEFORE the choice ladder: an unprovable group
                // key is an internal inconsistency, not an override
                // outcome.
                let key = repair_cell_key(cell)?;
                let write_pin = smelt_db::queries::maintenance::matching_write_pin(
                    cell,
                    &result.column_groups,
                    cells_cfg,
                )
                .and_then(|pin_name| smelt_logical::maintenance::lookup_write_pattern(&pin_name));
                let overrides = effective_override(
                    metadata
                        .maintenance
                        .as_ref()
                        .and_then(|m| m.defaults.as_ref()),
                    &combined_cells,
                    &facts.name,
                    group_columns,
                );
                let chosen = resolve_cell_choice(
                    Some(cell),
                    &trigger,
                    &overrides,
                    write_pin,
                    // `{recompute, PerGroupRecompute}` never contains
                    // `ColumnScopedMerge`, so the backend's MERGE capability
                    // is irrelevant to this resolver's resolvable set — the
                    // same `false` the membership resolver passes.
                    false,
                )
                .map_err(|refusal| anyhow::anyhow!(refusal.to_string()))?;
                // A `technique: recompute` pin (→ `RegionRecompute`) is
                // simply not THIS live cell: skip the source and let the
                // caller's own default apply, exactly as the sibling
                // resolvers do for a choice they have no lowering for. A
                // `write: diff_patch` pin routes here too, but only when its
                // underlying recompute is `PerGroupRecompute` — the sole
                // recompute `resolve_cell_choice` ever grants
                // `DeleteLeg::Complete` (the region `DeleteInsert` default
                // has no lowering and fails loud by name rather than
                // silently falling through to the default write);
                // [`resolve_repair_write`] carries the full decision table.
                let comparability = model_property_vector(sql, &JoinContext::new())
                    .map(|v| v.comparability)
                    .unwrap_or_default();
                let Some(write) = resolve_repair_write(
                    &chosen,
                    group_columns,
                    &comparability,
                    &cell.row_identity,
                    &cell.group,
                )?
                else {
                    continue;
                };
                // The cell's own derived slice — obligation 4's bounded
                // per-group read footprint, matched to the trigger's own
                // source (never another source's clamp).
                let Some(slice) = cell
                    .scans
                    .iter()
                    .find(|c| c.source == facts.name)
                    .or_else(|| cell.scans.first())
                else {
                    bail!(
                        "MaintenanceRepairSliceMissing: a Technique::PerGroupRecompute cell for \
                         group '{}' on source '{}' carries no derived ScanClamp — the bounded \
                         per-group read slice is admission obligation 4 and is never assumed",
                        cell.group,
                        facts.name,
                    );
                };
                // P9 (`docs/specs/incremental_models.md` §"The repair
                // family" — "Obligation 7 over a `mutable_snapshot`
                // source"): a source with no native change feed and no
                // tombstone/change history needs the group-grain sidecar
                // diff to witness a wholly-deleted group; every other
                // posture keeps the ordinary clamped current-source scan.
                let discovery =
                    if discovery_posture(facts.mutation) == RepairDiscoveryPosture::SidecarDiff {
                        if dialect != SqlDialect::DuckDB {
                            return Err(BackendError::unsupported(
                                dialect.name(),
                                "group-grain fingerprint-sidecar affected-key discovery for a \
                             mutable_snapshot repair source (P9)",
                            )
                            .into());
                        }
                        let digest_columns: Vec<String> =
                            match cell.fingerprint_projections.get(&facts.name) {
                                Some(FingerprintProjection::Columns(cols)) => {
                                    cols.iter().cloned().collect()
                                }
                                _ => Vec::new(),
                            };
                        if digest_columns.is_empty() {
                            bail!(
                            "MaintenanceRepairDigestColumnsMissing: a Technique::PerGroupRecompute \
                             cell for group '{}' resolved a MutableSnapshot delta posture on \
                             source '{}' with no P4 fingerprint projection columns — the \
                             group-grain sidecar digest has nothing to hash",
                            cell.group,
                            facts.name,
                        );
                        }
                        RepairDiscovery::SidecarDiff { digest_columns }
                    } else {
                        RepairDiscovery::ClampedScan
                    };
                return Ok(Some((
                    facts.name.clone(),
                    cell.clone(),
                    key,
                    slice.clone(),
                    write,
                    discovery,
                )));
            }
        }
    }
    Ok(None)
}

/// The affected-key relation a repair reads: the distinct group keys present
/// in the mutated source over the cell's own bounded slice, projected as a
/// single canonical `delta_key` column — the same expression
/// ([`smelt_logical::maintenance::emit::key_expr_for_columns`])
/// [`emit_fingerprint_sidecar_diff`]/[`emit_repair_group_sidecar_diff`]
/// project, so the append-only clamped-scan path and the `mutable_snapshot`
/// group-grain sidecar-diff path ([`diff_repair_group_sidecar_changed_keys`] +
/// [`repair_keys_literal_select`])
/// yield the SAME one-column relation shape — [`emit_per_group_recompute`]
/// joins against it by key EXPRESSION, never by raw key columns, because a
/// deleted group's typed column values are unrecoverable by construction.
///
/// Pure string builder, per this module's "callers resolve strings, emitters
/// assemble" contract — [`emit_per_group_recompute`] consumes this as opaque
/// `SELECT` text. The clamp is pushed into **this** read (and only this
/// one): the widened source band is what bounds how many groups a repair
/// touches, while the recompute of each touched group must stay unbounded
/// so the group is recomputed whole (see
/// [`repair_candidate_select`]).
///
/// `clamp: None` yields the unpredicated read. That branch is only reachable
/// where admission already proved the slice by another route —
/// `repair::admit_per_group_recompute` refuses `RepairSliceUnbounded` rather
/// than admitting a clamp-less cell, so this is not a silent widening path.
pub fn repair_affected_keys_select(
    source_table: &str,
    key: &[String],
    clamp: Option<&ScanClamp>,
    region: &Region,
) -> String {
    let key_expr = smelt_logical::maintenance::emit::key_expr_for_columns(key);
    match clamp {
        Some(clamp) => format!(
            "SELECT DISTINCT {key_expr} AS delta_key FROM {source_table} WHERE {}",
            widened_scan_predicate(clamp, region)
        ),
        None => format!("SELECT DISTINCT {key_expr} AS delta_key FROM {source_table}"),
    }
}

/// The candidate relation a repair stages: the model's **full** (unwindowed)
/// recompiled SQL, semi-joined to `affected_keys_select`'s single-column
/// `delta_key` relation.
///
/// Full, not windowed, because the repair family's promise is that an
/// affected group's stored value equals a full refresh of that group — a
/// non-invertible combiner (`MAX`) over a retracted contribution cannot be
/// fixed from a window's rows alone. The semi-join is what keeps the
/// recompute *bounded*: only the groups the affected-keys read named are
/// recomputed. `EXISTS` rather than a row-value `IN`, so a composite key
/// lowers identically across dialects. The join compares
/// [`repair_affected_keys_select`]'s own canonical key expression over the
/// candidate's key columns against `delta_key`, mirroring
/// [`repair_slice_predicate`]'s and `emit_per_group_recompute`'s identical
/// shape.
/// Widen `clean_sql` (the model's own raw, pre-compile SELECT) with one
/// `, <per_partition_expr> AS <name>` per `state_columns` — a named wrapper
/// over [`smelt_logical::maintenance::emit::state_augmented_projection`]
/// with the repair path's own error text, so the widening is independently
/// unit-testable rather than inlined at each call site
/// (`docs/outcomes/20260809-repair-family/phases/10-plan.md`). Mirrors
/// `smelt-runtime::cumulative::execute_windowed_keyed`/
/// `execute_snapshot_reconcile`'s own use of the same emitter: the fold's
/// create/merge path already carries a decomposed combiner's hidden state
/// columns in the physical table, so a repair's own candidate/insert must
/// supply them too, or the `INSERT`'s implicit column list mismatches the
/// table. `state_columns.is_empty()` returns `clean_sql` unchanged.
pub fn repair_augmented_model_sql(
    clean_sql: &str,
    state_columns: &[smelt_logical::analysis::decomposed_state::StateColumn],
) -> Result<String> {
    smelt_logical::maintenance::emit::state_augmented_projection(clean_sql, state_columns).map_err(
        |_| {
            anyhow::anyhow!(
                "Failed to append decomposed-state columns to a repair candidate: the model's \
                 SELECT could not be parsed"
            )
        },
    )
}

pub fn repair_candidate_select(
    full_model_sql: &str,
    key: &[String],
    affected_keys_select: &str,
) -> String {
    let candidate_key_columns: Vec<String> = key
        .iter()
        .map(|k| format!("__smelt_repair_candidate.{k}"))
        .collect();
    let candidate_key_expr =
        smelt_logical::maintenance::emit::key_expr_for_columns(&candidate_key_columns);
    format!(
        "SELECT __smelt_repair_candidate.* FROM ({full_model_sql}) AS __smelt_repair_candidate \
         WHERE EXISTS (SELECT 1 FROM ({affected_keys_select}) AS __smelt_repair_keys WHERE \
         {candidate_key_expr} = __smelt_repair_keys.delta_key)"
    )
}

/// Inputs to refresh the group-grain fingerprint sidecar transactionally
/// with a repair write (P9, `docs/outcomes/20260809-repair-family/phases/
/// 09-plan.md` task 6) — passed to [`execute_per_group_recompute`]/
/// [`execute_diff_patch`] only when the live cell's discovery is
/// [`RepairDiscovery::SidecarDiff`]; `None` for the ordinary clamped-scan
/// path, which has no sidecar partition to refresh.
pub struct RepairSidecarRefresh<'a> {
    pub schema: &'a str,
    pub source_address: &'a str,
    pub source_table: &'a str,
    pub group_key: &'a [String],
    pub digest_columns: &'a [String],
    pub model_sql: &'a str,
}

/// Execute a live `Technique::PerGroupRecompute` cell
/// ([`resolve_live_per_group_recompute_cell`]) via the repair family's
/// targeted `DELETE`+`INSERT` over the affected-key relation
/// ([`emit_per_group_recompute`]) — the same emitter → `retry_backend_call`
/// → [`Backend::execute_statement_group`] shape
/// [`execute_staged_membership_recompute`] uses, so the executed text is
/// exactly the single owner's output (`docs/specs/incremental_models.md`
/// §"Statement emission (single owner)").
///
/// `sidecar_refresh: Some(..)` (a [`RepairDiscovery::SidecarDiff`] cell)
/// routes the SAME emitted [`StatementGroup`] through
/// [`refresh_repair_group_sidecar`] instead of a bare
/// [`Backend::execute_statement_group`] call — the group-grain sidecar
/// partition refreshes in the SAME backend transaction as this write
/// (mirroring [`refresh_fingerprint_sidecar`]'s own transactional shape),
/// so a failed write leaves the sidecar untouched rather than
/// half-committed.
#[allow(clippy::too_many_arguments)]
pub async fn execute_per_group_recompute(
    backend: &dyn Backend,
    schema: &str,
    table: &str,
    key: &[String],
    affected_keys_select: &str,
    candidate_select: &str,
    retry: &crate::execute::RetryPolicy<'_>,
    sidecar_refresh: Option<&RepairSidecarRefresh<'_>>,
) -> Result<ExecutionResult> {
    let start = Instant::now();
    let full_table = format!("{schema}.{table}");
    let dialect = maintenance_dialect(backend.dialect());
    let staged_relation = repair_staged_relation(table);
    let group = emit_per_group_recompute(
        &full_table,
        &staged_relation,
        key,
        affected_keys_select,
        candidate_select,
        dialect,
    );
    match sidecar_refresh {
        None => {
            crate::execute::retry_backend_call(retry, || backend.execute_statement_group(&group))
                .await
                .map_err(|e| {
                    anyhow::anyhow!("per-group recompute failed for '{full_table}': {e}")
                })?;
        }
        Some(refresh) => {
            crate::execute::retry_backend_call(retry, || {
                refresh_repair_group_sidecar(
                    backend,
                    refresh.schema,
                    refresh.source_address,
                    refresh.source_table,
                    refresh.group_key,
                    refresh.digest_columns,
                    refresh.model_sql,
                    &group,
                )
            })
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "per-group recompute (with group-grain sidecar refresh) failed for \
                     '{full_table}': {e}"
                )
            })?;
        }
    }
    let row_count = backend.get_row_count(schema, table).await.unwrap_or(0);
    Ok(ExecutionResult {
        model_name: table.to_string(),
        duration: start.elapsed(),
        row_count,
        preview: None,
    })
}

/// The staged temp relation name a repair uses for `table` — one derivation,
/// so a parity test (and the technique preview) can name the same relation
/// the live run does without guessing.
pub fn repair_staged_relation(table: &str) -> String {
    format!("__smelt_repair_{table}")
}

/// The staged temp relation name a `diff_patch` write over a repair cell
/// uses for `table` — a distinct prefix from [`repair_staged_relation`] so a
/// parity test can name each group's own relation without ambiguity.
pub fn diff_patch_staged_relation(table: &str) -> String {
    format!("__smelt_diff_patch_{table}")
}

/// The `diff_patch` slice restriction for a repair cell: the candidate's own
/// slice is the affected-key set, not a partition region
/// ([`emit_diff_patch`]'s doc comment) — an `EXISTS` over the affected-keys
/// read's single-column `delta_key` relation, `table`-qualified on every key
/// column (via the same canonical key expression
/// [`repair_affected_keys_select`]/[`repair_candidate_select`] use) so it
/// composes unambiguously into both the update-leg and delete-leg `DELETE`s
/// `emit_diff_patch` builds.
pub fn repair_slice_predicate(table: &str, key: &[String], affected_keys_select: &str) -> String {
    let table_key_columns: Vec<String> = key.iter().map(|k| format!("{table}.{k}")).collect();
    let table_key_expr = smelt_logical::maintenance::emit::key_expr_for_columns(&table_key_columns);
    format!(
        "EXISTS (SELECT 1 FROM ({affected_keys_select}) AS __smelt_repair_keys WHERE \
         {table_key_expr} = __smelt_repair_keys.delta_key)"
    )
}

/// Execute a `write: diff_patch` pin over a live `Technique::PerGroupRecompute`
/// cell ([`resolve_live_per_group_recompute_cell`]) via [`emit_diff_patch`] —
/// same emitter → `retry_backend_call` → [`Backend::execute_statement_group`]
/// shape [`execute_per_group_recompute`] uses, so the executed text is
/// exactly the single owner's output (`docs/specs/incremental_models.md`
/// §"Statement emission (single owner)"). `sidecar_refresh` carries the same
/// meaning as [`execute_per_group_recompute`]'s own parameter.
#[allow(clippy::too_many_arguments)]
pub async fn execute_diff_patch(
    backend: &dyn Backend,
    schema: &str,
    table: &str,
    key: &[String],
    candidate_select: &str,
    compared_columns: &[String],
    slice_predicate: &str,
    delete_leg: &smelt_logical::maintenance::diff_patch::DeleteLeg,
    retry: &crate::execute::RetryPolicy<'_>,
    sidecar_refresh: Option<&RepairSidecarRefresh<'_>>,
) -> Result<ExecutionResult> {
    let start = Instant::now();
    let full_table = format!("{schema}.{table}");
    let dialect = maintenance_dialect(backend.dialect());
    let staged_relation = diff_patch_staged_relation(table);
    let group = smelt_logical::maintenance::emit::emit_diff_patch(
        &full_table,
        &staged_relation,
        key,
        candidate_select,
        compared_columns,
        slice_predicate,
        delete_leg,
        dialect,
    );
    match sidecar_refresh {
        None => {
            crate::execute::retry_backend_call(retry, || backend.execute_statement_group(&group))
                .await
                .map_err(|e| anyhow::anyhow!("diff_patch write failed for '{full_table}': {e}"))?;
        }
        Some(refresh) => {
            crate::execute::retry_backend_call(retry, || {
                refresh_repair_group_sidecar(
                    backend,
                    refresh.schema,
                    refresh.source_address,
                    refresh.source_table,
                    refresh.group_key,
                    refresh.digest_columns,
                    refresh.model_sql,
                    &group,
                )
            })
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "diff_patch write (with group-grain sidecar refresh) failed for \
                     '{full_table}': {e}"
                )
            })?;
        }
    }
    let row_count = backend.get_row_count(schema, table).await.unwrap_or(0);
    Ok(ExecutionResult {
        model_name: table.to_string(),
        duration: start.elapsed(),
        row_count,
        preview: None,
    })
}

/// A live key-addressed model-edge cell as
/// [`resolve_live_key_addressed_model_edge_cell`] returns it: the upstream
/// edge's own name (`== key_scope.from`), the cell itself, its `KeyScope`
/// (the downstream's own key columns to restrict the recompute to), the
/// upstream's own `KeyedUpsert` key columns (the group-grain sidecar's own
/// `group_key`), the digest column set the sidecar hashes (derived from the
/// downstream's own CLEAN sql — never recomputed against compiled SQL, which
/// carries physical table names rather than `smelt.*` refs the walk-backed
/// fingerprint classifier matches against), and the resolved write leg.
pub type LiveKeyAddressedModelEdgeCell = (
    String,
    PlanCell,
    smelt_logical::maintenance::KeyScope,
    Vec<String>,
    Vec<String>,
    RepairWrite,
);

/// Resolve a `Technique::PerGroupRecompute` cell derived over a
/// **key-addressed model edge** (`docs/specs/incremental_models.md`
/// §"Upstream model edges") — the sibling of
/// [`resolve_live_per_group_recompute_cell`] for a cell whose bounded read is
/// a `KeyScope` (an upstream's own affected key set) rather than a
/// `ScanClamp` over a declared source. The plan is derived exactly once here
/// via [`smelt_db::queries::maintenance::derive_model_maintenance_plan_with_edges`]
/// (maintenance-plan purity, root `CLAUDE.md`) — `model_edges` must be the
/// SAME edge list (with each upstream's own derived `output_shape`) that
/// produced the cell this run will execute.
///
/// Two fail-loud legs run BEFORE any backend call
/// (`docs/outcomes/20260809-output-delta-typing/phases/07-plan.md`):
/// - a non-DuckDB target dialect — the group-grain sidecar diff this cell's
///   execution needs is DuckDB-only, matching every other sidecar consumer
///   in this module;
/// - a `key_scope.keys` column the upstream relation does not actually
///   carry — checked against the upstream edge's own declared
///   `ModelEdge::unique_key` (the upstream's real output-table column
///   names), never against the downstream's own guess. A mismatch (the
///   downstream renamed the key column it read) is refused by name rather
///   than silently querying a column the upstream table does not have.
pub fn resolve_live_key_addressed_model_edge_cell(
    sql: &str,
    table: &str,
    metadata: &smelt_core::ModelMetadata,
    sources: &[SourceFacts],
    explicitly_mutable: &HashSet<String>,
    model_edges: &[smelt_logical::maintenance::derive::ModelEdge],
    dialect: SqlDialect,
) -> Result<Option<LiveKeyAddressedModelEdgeCell>> {
    let Some(result) = smelt_db::queries::maintenance::derive_model_maintenance_plan_with_edges(
        sql,
        table,
        metadata,
        sources,
        explicitly_mutable,
        model_edges,
        None,
        &[],
        &[],
        &SourceReferentialIntegrity::new(),
        None,
    ) else {
        return Ok(None);
    };

    for cell in &result.plan.cells {
        if cell.technique != Technique::PerGroupRecompute {
            continue;
        }
        let Some(key_scope) = cell.key_scope.clone() else {
            continue;
        };
        let Some(edge) = model_edges.iter().find(|e| e.name == key_scope.from) else {
            bail!(
                "MaintenanceKeyAddressedEdgeMissing: a key-addressed model-edge cell names \
                 upstream '{}' but no matching ModelEdge was supplied — internal \
                 inconsistency between plan derivation and the caller's own edge list",
                key_scope.from
            );
        };
        // The upstream's own proven `KeyedUpsert` key columns — the SAME
        // `edge.output_shape` fact `append_model_edge_cells` reads to admit
        // this cell in the first place (`derive.rs`'s `let Some(OutputDelta::
        // KeyedUpsert { keys }) = &edge.output_shape`), never `edge.
        // unique_key` (a separate, often-undeclared metadata field the walk-
        // derived shape does not depend on).
        let Some(smelt_logical::analysis::output_delta::OutputDelta::KeyedUpsert {
            keys: upstream_keys,
        }) = &edge.output_shape
        else {
            bail!(
                "MaintenanceKeyAddressedEdgeMissing: a key-addressed model-edge cell names \
                 upstream '{}', but its own ModelEdge no longer carries a KeyedUpsert \
                 output_shape — internal inconsistency between plan derivation (which required \
                 this fact to admit the cell) and the caller's own edge list",
                edge.name
            );
        };
        if dialect != SqlDialect::DuckDB {
            return Err(BackendError::unsupported(
                dialect.name(),
                "key-addressed model-edge affected-key discovery over a KeyedUpsert upstream \
                 (group-grain fingerprint-sidecar diff)",
            )
            .into());
        }
        if !key_scope
            .keys
            .iter()
            .all(|k| upstream_keys.iter().any(|u| u.eq_ignore_ascii_case(k)))
        {
            bail!(
                "MaintenanceKeyScopeColumnMissing: a key-addressed model-edge cell for upstream \
                 '{}' names key column(s) {:?}, but the upstream's own proven KeyedUpsert key \
                 columns are {:?} — a key_scope column absent from the upstream relation is \
                 refused, never widened to every key",
                edge.name,
                key_scope.keys,
                upstream_keys,
            );
        }
        let comparability = model_property_vector(sql, &JoinContext::new())
            .map(|v| v.comparability)
            .unwrap_or_default();
        let chosen = resolve_cell_choice(
            Some(cell),
            &cell.trigger,
            &smelt_logical::maintenance::choice::EffectiveOverride {
                prefer: None,
                technique: None,
            },
            None,
            false,
        )
        .map_err(|refusal| anyhow::anyhow!(refusal.to_string()))?;
        let Some(write) = resolve_repair_write(
            &chosen,
            &key_scope.keys,
            &comparability,
            &cell.row_identity,
            &cell.group,
        )?
        else {
            continue;
        };
        // The digest column set the group-grain sidecar hashes: whatever
        // columns of the upstream this downstream's own (clean) SQL
        // actually reads (`fingerprint::fingerprint_projection`, the same
        // walk-backed leaf classifier `delta_shape_for_source` reuses for
        // the ordinary repair-family sidecar), falling back to the
        // upstream's own key columns alone when the projection cannot be
        // classified — a narrower digest than the ideal (misses a
        // payload-only mutation the downstream's SQL does not itself read),
        // never a widening. Derived here, from `sql` (clean), not at
        // execution time from compiled SQL — a compiled model's `smelt.*`
        // refs are already rewritten to physical table names, which this
        // walk cannot match against.
        let digest_columns: Vec<String> = match fingerprint::fingerprint_projection(sql, &edge.name)
        {
            FingerprintProjection::Columns(cols) => cols.into_iter().collect(),
            FingerprintProjection::FullRow { .. } => upstream_keys.clone(),
        };
        return Ok(Some((
            edge.name.clone(),
            cell.clone(),
            key_scope,
            upstream_keys.clone(),
            digest_columns,
            write,
        )));
    }
    Ok(None)
}

/// The affected-key relation a key-addressed model-edge cell reads
/// (`docs/specs/incremental_models.md` §"Upstream model edges"): the
/// group-grain fingerprint sidecar diff over the upstream's own output
/// table, at the upstream's `KeyedUpsert` key grain, projected onto the
/// downstream's own key columns
/// ([`smelt_logical::maintenance::emit::emit_key_addressed_affected_keys_select`]).
/// A DuckDB-only discovery route — `resolve_live_key_addressed_model_edge_cell`
/// already refused a non-DuckDB dialect before any backend call is reached.
///
/// Returns an empty resolved key list when the sidecar diff discovers no
/// changed upstream keys — the caller reports a no-op rather than executing
/// an empty-but-real write.
#[allow(clippy::too_many_arguments)]
pub async fn resolve_key_addressed_affected_keys(
    backend: &dyn Backend,
    schema: &str,
    upstream_source_address: &str,
    upstream_table: &str,
    upstream_output_table: &str,
    upstream_keys: &[String],
    digest_columns: &[String],
    downstream_keys: &[String],
    model_sql: &str,
) -> std::result::Result<(Vec<String>, String), BackendError> {
    let changed_keys = diff_repair_group_sidecar_changed_keys(
        backend,
        schema,
        upstream_source_address,
        upstream_table,
        upstream_output_table,
        upstream_keys,
        digest_columns,
        model_sql,
    )
    .await?;
    let dialect = maintenance_dialect(backend.dialect());
    let affected_keys_select =
        smelt_logical::maintenance::emit::emit_key_addressed_affected_keys_select(
            upstream_table,
            upstream_keys,
            downstream_keys,
            &changed_keys,
            dialect,
        );
    Ok((changed_keys, affected_keys_select))
}

/// Execute a live key-addressed model-edge cell
/// ([`resolve_live_key_addressed_model_edge_cell`]): discover the upstream's
/// changed keys via the group-grain fingerprint sidecar diff over its own
/// output table, project them onto the downstream's own key columns
/// ([`resolve_key_addressed_affected_keys`]), stage the downstream's full
/// SQL semi-joined to that key relation
/// ([`repair_candidate_select`]), and write via the resolved
/// [`RepairWrite`] leg — [`execute_per_group_recompute`] for
/// `TargetedDeleteInsert`, [`execute_diff_patch`] for `DiffPatch`. The
/// upstream sidecar refreshes in the SAME backend transaction as this write
/// ([`RepairSidecarRefresh`]), so a failed write never leaves the sidecar
/// advanced past a change it did not actually consume.
///
/// An empty changed-key set is a legitimate no-op: this returns
/// `Ok(None)` rather than executing an empty-but-real write, matching
/// [`emit_key_addressed_affected_keys_select`]'s own well-typed-empty
/// convention (`docs/outcomes/20260809-output-delta-typing/phases/07-plan.md`
/// task 5).
#[allow(clippy::too_many_arguments)]
pub async fn execute_key_addressed_model_edge_cell(
    backend: &dyn Backend,
    schema: &str,
    table: &str,
    upstream_source_address: &str,
    upstream_table: &str,
    upstream_keys: &[String],
    digest_columns: &[String],
    downstream_keys: &[String],
    clean_model_sql: &str,
    compiled_model_sql: &str,
    write: &RepairWrite,
    retry: &crate::execute::RetryPolicy<'_>,
) -> Result<Option<ExecutionResult>> {
    let full_table = format!("{schema}.{table}");
    let (changed_keys, affected_keys_select) = resolve_key_addressed_affected_keys(
        backend,
        schema,
        upstream_source_address,
        upstream_table,
        &full_table,
        upstream_keys,
        digest_columns,
        downstream_keys,
        clean_model_sql,
    )
    .await
    .map_err(|e| {
        anyhow::anyhow!(
            "key-addressed affected-key discovery failed over upstream '{upstream_table}' for \
             '{full_table}': {e}"
        )
    })?;
    if changed_keys.is_empty() {
        return Ok(None);
    }
    let candidate_select =
        repair_candidate_select(compiled_model_sql, downstream_keys, &affected_keys_select);
    let sidecar_refresh = RepairSidecarRefresh {
        schema,
        source_address: upstream_source_address,
        source_table: upstream_table,
        group_key: upstream_keys,
        digest_columns,
        model_sql: clean_model_sql,
    };
    let result = match write {
        RepairWrite::TargetedDeleteInsert => {
            execute_per_group_recompute(
                backend,
                schema,
                table,
                downstream_keys,
                &affected_keys_select,
                &candidate_select,
                retry,
                Some(&sidecar_refresh),
            )
            .await?
        }
        RepairWrite::DiffPatch {
            compared_columns,
            delete_leg,
        } => {
            let slice_predicate =
                repair_slice_predicate(table, downstream_keys, &affected_keys_select);
            execute_diff_patch(
                backend,
                schema,
                table,
                downstream_keys,
                &candidate_select,
                compared_columns,
                &slice_predicate,
                delete_leg,
                retry,
                Some(&sidecar_refresh),
            )
            .await?
        }
    };
    Ok(Some(result))
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
#[allow(clippy::too_many_arguments)]
pub async fn execute_column_scoped_merge_full(
    backend: &dyn Backend,
    schema: &str,
    table: &str,
    unique_key: &[String],
    dimension_batch_sql: &str,
    columns: &[String],
    suppression: &WriteSuppression,
    window: &PartitionRange,
    retry: &crate::execute::RetryPolicy<'_>,
) -> Result<ExecutionResult> {
    let start = Instant::now();
    let full_table = format!("{schema}.{table}");
    let dialect = maintenance_dialect(backend.dialect());
    execute_column_scoped_write_with_observed_delta(
        backend,
        schema,
        table,
        unique_key,
        dimension_batch_sql,
        columns,
        suppression,
        dialect,
        window,
        retry,
    )
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

/// Build the `IS DISTINCT FROM` OR-predicate over `compared_columns` — the
/// SAME shape [`emit_column_scoped_merge_suppressed`]
/// (`smelt_logical::maintenance::emit`) guards its matched arm with. Not a
/// shared emitter export: D1 ruled observed-delta recording is smelt-state
/// bookkeeping (warehouse-resident, alongside the reconciliation ledger),
/// not emitter-authored maintenance-statement text
/// (`docs/specs/incremental_models.md` §"The graph layer" — "Observed
/// deltas on model edges"), so it sits outside
/// `smelt_logical::maintenance::emit`'s single-owner rule the same way
/// `Backend::fold_ledger_delta`'s ledger DML does. Kept from drifting off
/// the write's own guard by a dedicated cross-check test
/// (`crates/smelt-runtime/tests/statement_parity.rs`).
pub fn changed_row_predicate(left: &str, right: &str, compared_columns: &[String]) -> String {
    compared_columns
        .iter()
        .map(|c| format!("{left}.{c} IS DISTINCT FROM {right}.{c}"))
        .collect::<Vec<_>>()
        .join(" OR ")
}

/// The changed-key `SELECT` a conditional column-scoped MERGE's observed
/// delta is recorded from: every row the guarded matched arm actually
/// updates (its compared columns differ) plus every unmatched row the
/// always-unconditional insert arm inserts — exactly the rowset
/// [`emit_column_scoped_merge_suppressed`] writes, restricted to
/// **comparable columns only** (P3's change-comparability verdict is the
/// only membership authority — an Incomparable column's own flutter, e.g.
/// a `plausible` audit stamp, never appears in `compared_columns`, so it
/// can never dirty this query). `partition_column`, when `Some`, names a
/// column present in `source_select`'s own full-row projection (the
/// model's declared partition column) to report as the touched partition;
/// `None` records every row's partition as `NULL` (folded to an empty
/// `partitions` array by the upsert) — a bare keyed model with no
/// partition axis.
///
/// **Known limitation, deliberately not fixed here.** For a multi-column
/// `unique_key`, `key_expr` joins each `CAST(... AS VARCHAR)` column with an
/// unescaped `\u{1}` separator — the same collision shape
/// `smelt_logical::maintenance::emit::concat_varchar_expr` had before its
/// own fix (a column value containing a literal `\u{1}` byte can make two
/// distinct composite keys reassemble into the same joined string). Unlike
/// that sidecar helper, this function's output is NOT an opaque
/// equality-only token: the recorded `delta_key` is later spliced back in
/// as a literal predicate value against a REAL column
/// (`emit_delete_insert_delta_restricted`'s `restrict_column IN
/// (delta_keys)`), so switching this to a hashed/tagged construction (the
/// sidecar's fix) would break that literal-match contract wherever
/// `restrict_column` is a single physical column being compared against a
/// composite hash — a materially different, coordinated change to the
/// restriction/consumption path, not a same-shape substitution. Tracked as
/// an open item rather than silently left alone; revisit alongside whatever
/// work gives composite-key restriction its own literal-decomposable
/// representation.
pub fn changed_keys_select(
    table: &str,
    unique_key: &[String],
    source_select: &str,
    compared_columns: &[String],
    partition_column: Option<&str>,
) -> String {
    let on = unique_key
        .iter()
        .map(|k| format!("target.{k} = source.{k}"))
        .collect::<Vec<_>>()
        .join(" AND ");
    let key_expr = if unique_key.len() == 1 {
        format!("CAST(source.{} AS VARCHAR)", unique_key[0])
    } else {
        let parts = unique_key
            .iter()
            .map(|k| format!("CAST(source.{k} AS VARCHAR)"))
            .collect::<Vec<_>>()
            .join(", '\u{1}', ");
        format!("CONCAT({parts})")
    };
    let partition_expr = match partition_column {
        Some(col) => format!("CAST(source.{col} AS VARCHAR)"),
        None => "NULL".to_string(),
    };
    let first_key = &unique_key[0];
    let suppression = changed_row_predicate("target", "source", compared_columns);
    format!(
        "SELECT {key_expr} AS delta_key, {partition_expr} AS delta_partition FROM \
         ({source_select}) AS source LEFT JOIN {table} AS target ON {on} \
         WHERE target.{first_key} IS NULL OR ({suppression})"
    )
}

/// Execute a live `ColumnScopedMerge` cell's write, and — when the cell's
/// [`WriteSuppression`] verdict is `Suppressed` — record its observed
/// output delta in the SAME backend transaction (T5,
/// `docs/specs/incremental_models.md` §"The graph layer" — "Observed
/// deltas on model edges"). `Unconditional` writes are not recorded — the
/// record is a byproduct of the conditional write's already-computed
/// changed-row set, never derived after the fact for an unconditional one.
/// `window` identifies the run window this write covers (the observed-
/// delta table's own idempotent-replace key, `PRIMARY KEY (model_name,
/// window_start, window_end)`); `window.column`, when non-empty, is also
/// the partition-column projection `changed_keys_select` reports as the
/// touched partition.
///
/// Only DuckDB has an observed-delta storage implementation today — the
/// same DuckDB-only posture `Backend::fold_ledger_delta`'s doc comment
/// documents for the reconciliation ledger (`smelt_state::ddl_duckdb` is
/// the only dialect implemented); a non-DuckDB backend fails loudly rather
/// than being handed DuckDB-flavored SQL it cannot run.
#[allow(clippy::too_many_arguments)]
async fn execute_column_scoped_write_with_observed_delta(
    backend: &dyn Backend,
    schema: &str,
    table: &str,
    unique_key: &[String],
    source_select: &str,
    columns: &[String],
    suppression: &WriteSuppression,
    dialect: MaintenanceDialect,
    window: &PartitionRange,
    retry: &crate::execute::RetryPolicy<'_>,
) -> std::result::Result<(), BackendError> {
    // Both arms below emit a whole-row MERGE, so a dialect without a star form
    // needs `columns` populated. Refuse here — before either emitter runs —
    // rather than let a matched arm that assigns nothing reach the warehouse.
    smelt_backend::require_merge_columns(backend.dialect(), schema, table, columns)?;
    let full_table = format!("{schema}.{table}");
    match suppression {
        WriteSuppression::Suppressed { compared_columns } => {
            let group = emit_column_scoped_merge_suppressed(
                &full_table,
                unique_key,
                source_select,
                compared_columns,
                columns,
                dialect,
            );
            if backend.dialect() != SqlDialect::DuckDB {
                return Err(BackendError::unsupported(
                    backend.dialect().name(),
                    "observed-delta recording for a change-suppressed column-scoped MERGE (T5)",
                ));
            }
            let ensure_sql = ddl_duckdb::generate_observed_delta_table_ddl(schema);
            let partition_column = if window.column.is_empty() {
                None
            } else {
                Some(window.column.as_str())
            };
            let changed_keys_query = changed_keys_select(
                &full_table,
                unique_key,
                source_select,
                compared_columns,
                partition_column,
            );
            let record_sql = ddl_duckdb::generate_observed_delta_upsert_sql(
                schema,
                table,
                &window.start,
                &window.end,
                &changed_keys_query,
            );
            crate::execute::retry_backend_call(retry, || {
                backend.execute_conditional_write_and_record_observed_delta(
                    &ensure_sql,
                    &group,
                    &record_sql,
                )
            })
            .await
        }
        WriteSuppression::Unconditional { .. } => {
            let group =
                emit_column_scoped_merge(&full_table, unique_key, source_select, columns, dialect);
            crate::execute::retry_backend_call(retry, || backend.execute_statement_group(&group))
                .await
        }
    }
}

// ── T3: delta-restricted region recompute over a model edge ────────────
// (`docs/plans/20260715-composed-axes-conditional-maintenance.md` Phase E3)

/// Read the exact observed-delta changed-key set an upstream driving model
/// edge recorded for `[window_start, window_end)` (T5, Group D). `None` = no
/// row was ever recorded for this window — the "pre-D2 upstream" / never-
/// recorded case, the trigger for the widen-never-narrow fallback — distinct
/// from `Some(&[])`'s "recorded and present-and-empty" (a fully-suppressed
/// upstream run; `incremental_models.md` §"The graph layer" — "Empty and
/// absent are distinct").
///
/// DuckDB-only, matching every other `_smelt_observed_delta` consumer in
/// this module (`execute_column_scoped_write_with_observed_delta` above).
/// Unlike that function's *write*-side capability gap (a hard error — the
/// caller asked for a technique the backend cannot provide), a missing
/// delta on the *read* side is always a legal fallback trigger, so a non-
/// DuckDB backend reads back `None` rather than erroring.
pub async fn read_observed_delta_changed_keys(
    backend: &dyn Backend,
    schema: &str,
    model: &str,
    window_start: &str,
    window_end: &str,
) -> std::result::Result<Option<Vec<String>>, BackendError> {
    Ok(
        read_observed_delta(backend, schema, model, window_start, window_end)
            .await?
            .map(|od| od.changed_keys),
    )
}

/// Decode a single `VARCHAR[]` column of an observed-delta result batch
/// into owned strings, skipping a null list entry or a non-string-array
/// column shape (defensive — the DDL guarantees `VARCHAR[] NOT NULL`, but
/// this never panics on an unexpected shape).
fn decode_string_list_column(batch: &arrow::array::RecordBatch, column: &str) -> Vec<String> {
    let mut out = Vec::new();
    let Some(col) = batch.column_by_name(column) else {
        return out;
    };
    let Some(list) = col.as_any().downcast_ref::<arrow::array::ListArray>() else {
        return out;
    };
    for i in 0..list.len() {
        if list.is_null(i) {
            continue;
        }
        let values = list.value(i);
        let Some(strings) = values.as_any().downcast_ref::<arrow::array::StringArray>() else {
            continue;
        };
        for j in 0..strings.len() {
            if !strings.is_null(j) {
                out.push(strings.value(j).to_string());
            }
        }
    }
    out
}

/// Read the exact observed delta (both `changed_keys` and `partitions`) an
/// upstream driving model edge recorded for `[window_start, window_end)` —
/// the single decode site [`read_observed_delta_changed_keys`] and
/// [`crate::propagation::load_observed_delta_lookup`] both re-express
/// themselves over. `None` = no row was ever recorded for this window (the
/// widen-never-narrow fallback trigger); `Some` — even with both vectors
/// empty — means a row exists (§"Empty and absent are distinct").
///
/// DuckDB-only, matching every other `_smelt_observed_delta` consumer in
/// this module: a missing delta on the read side is always a legal
/// fallback trigger, so a non-DuckDB backend reads back `None` rather than
/// erroring.
pub async fn read_observed_delta(
    backend: &dyn Backend,
    schema: &str,
    model: &str,
    window_start: &str,
    window_end: &str,
) -> std::result::Result<Option<ddl_duckdb::ObservedDelta>, BackendError> {
    if backend.dialect() != SqlDialect::DuckDB {
        return Ok(None);
    }
    let ensure_sql = ddl_duckdb::generate_observed_delta_table_ddl(schema);
    backend.execute_sql(&ensure_sql).await?;

    let select_sql =
        ddl_duckdb::generate_observed_delta_select_sql(schema, model, window_start, window_end);
    let batches = backend.execute_sql(&select_sql).await?;
    let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
    if total_rows == 0 {
        return Ok(None);
    }

    let mut changed_keys = Vec::new();
    let mut partitions = Vec::new();
    for batch in &batches {
        changed_keys.extend(decode_string_list_column(batch, "changed_keys"));
        partitions.extend(decode_string_list_column(batch, "partitions"));
    }
    Ok(Some(ddl_duckdb::ObservedDelta {
        changed_keys,
        partitions,
    }))
}

// ── F3: fingerprint sidecar — synthesized external change feed ─────────
// (`docs/plans/20260715-composed-axes-conditional-maintenance.md` Phase F3;
// `docs/specs/sources.md` §"The fingerprint sidecar")
//
// Builds and consumes the row-content fingerprint sidecar for a
// `mutable_snapshot` external source with no native change feed: the diff
// (`diff_fingerprint_sidecar_changed_keys`) synthesizes an exact changed-key
// set from a full re-scan of the source compared against the sidecar's
// stored digests; the refresh (`refresh_fingerprint_sidecar`) then brings
// the sidecar's stored digests up to date with the source's current
// content, riding in the same backend transaction as the write that
// consumed the diff. Wiring this changed-key set into the maintenance
// plan's own trigger/technique selection (deciding WHEN a live run uses the
// sidecar-derived delta instead of the whole-table one) is a licence change
// scoped to a later phase (T3 over external sources) — these functions are
// a standalone, independently-tested capability today, matching P4's own
// "no consumer reads it yet" framing (`model_properties.md` §"Fingerprint
// projection").

/// Resolve which columns a fingerprint sidecar digests for one `(model,
/// external source)` pair: the P4 verdict's own column set, or — fail-
/// closed — `all_source_columns` when the verdict is `FullRow`
/// (`model_properties.md` §"Fingerprint projection": "an unprojectable
/// consumption ... yields `FullRow`, never a guessed subset"). Pure data —
/// no sidecar/digest machinery, matching
/// `smelt_logical::maintenance::derive`'s own "pure data, no
/// sidecar/digest machinery here" framing for the P4 derivation itself.
pub fn resolve_fingerprint_digest_columns(
    projection: &FingerprintProjection,
    all_source_columns: &[String],
) -> Vec<String> {
    match projection {
        FingerprintProjection::Columns(cols) => cols.iter().cloned().collect(),
        FingerprintProjection::FullRow { .. } => all_source_columns.to_vec(),
    }
}

// ── F4: fingerprint sidecar invalidation ────────────────────────────────
// (`docs/plans/20260715-composed-axes-conditional-maintenance.md` Phase F4
// — "Sidecar invalidation"; `docs/specs/sources.md` §"The fingerprint
// sidecar")
//
// A sidecar partition's stored digests are only trustworthy comparanda for
// a diff if nothing that could change what "the same row" or "the same
// digest" means has changed underneath it since the last refresh. Three
// independent things can invalidate that trust, any one of which must widen
// the next diff to "everything in the source is changed" — never a
// narrower, partially-trusted comparison, and never a silent skip:
//
// - the digest-construction version (`FINGERPRINT_SIDECAR_DIGEST_VERSION`)
//   — bumped only when `emit_fingerprint_digest_select`'s own hashing
//   scheme changes shape;
// - the P4 fingerprint projection's identity (already the sidecar's own
//   partition key — a projection change lands in a fresh, unpopulated
//   partition by construction, no extra mechanism needed);
// - the consuming model's own SQL definition, hashed the same way
//   `IntervalStore::get_or_create` invalidates covered intervals on a
//   model edit (`smelt_state::intervals::compute_model_hash`) — this is
//   the one trigger that can go stale WITHOUT a fresh partition, since two
//   different model SQL texts can resolve the identical P4 projection.

/// Digest-construction version for the fingerprint sidecar's stored digests
/// (`emit_fingerprint_digest_select`'s `sha256(...)` shape). Part of every
/// partition's identity stamp — bump this only when that construction
/// changes in a way that makes a previously stored digest no longer
/// comparable to a freshly computed one, so every stamp stored under the
/// old scheme is detected as mismatched (never silently trusted) on the
/// next diff.
const FINGERPRINT_SIDECAR_DIGEST_VERSION: &str = "v1";

/// Compute the fingerprint sidecar's identity stamp for one `(model,
/// external source)` pair — the combined value invalidation compares the
/// freshly computed one against on every run, mirroring
/// `IntervalStore::get_or_create`'s `model_hash != model_hash`
/// invalidation-on-mismatch precedent (`smelt_state::intervals`). Combines:
/// - [`FINGERPRINT_SIDECAR_DIGEST_VERSION`] (the digest-algorithm version);
/// - `projection_identity` (the caller's already-resolved P4 projection
///   identity — `smelt_logical::analysis::fingerprint::projection_identity`);
/// - a hash of `model_sql` (the consuming model's own SQL text), via
///   [`smelt_state::intervals::compute_model_hash`] — the same hash
///   `IntervalStore` uses to invalidate covered intervals on a model edit.
///
/// Any one of the three inputs changing produces a different stamp — this
/// is deliberately coarse (a model edit unrelated to this source still
/// invalidates the sidecar for it) rather than attempting to prove the edit
/// was irrelevant: the fail-loud/widen-never-narrow posture this codebase
/// takes everywhere else in the maintenance layer.
pub fn compute_fingerprint_sidecar_stamp(projection_identity: &str, model_sql: &str) -> String {
    let model_hash = smelt_state::intervals::compute_model_hash(model_sql);
    format!("{FINGERPRINT_SIDECAR_DIGEST_VERSION}:{projection_identity}:{model_hash}")
}

/// Read-side: the synthesized changed-key set the fingerprint sidecar
/// derives for `(source_address, source_table)` under `projection` (the
/// caller's already-resolved P4 fingerprint projection;
/// `all_source_columns` resolves the fail-closed `FullRow` case). Ensures
/// the sidecar table exists, then runs the emitter-authored diff query
/// (`smelt_logical::maintenance::emit::emit_fingerprint_sidecar_diff`).
///
/// An absent sidecar makes every current source row "changed" by
/// construction (`docs/specs/sources.md` §"The fingerprint sidecar" —
/// "First run and `--full-refresh`") — no special-casing needed here, the
/// diff query's own `FULL OUTER JOIN` produces that result against an
/// empty (or not-yet-created) sidecar partition.
///
/// DuckDB-only, matching every other `_smelt_fingerprint_sidecar`/
/// `_smelt_observed_delta` consumer in this module. Unlike
/// `read_observed_delta_changed_keys`'s read-side fallback (a missing
/// delta is always a legal widen-never-narrow trigger, so it reads back
/// `None` on a non-DuckDB backend), a caller asking for a sidecar diff at
/// all has already chosen the sidecar-backed path — a non-DuckDB backend
/// here fails loudly (`docs/specs/sources.md` §"The fingerprint sidecar" —
/// "DuckDB-scoped today ... a non-DuckDB target fails loud rather than
/// silently skipping the sidecar").
///
/// `model_sql` is the consuming model's own SQL text — folded into the
/// partition's identity stamp ([`compute_fingerprint_sidecar_stamp`]) so a
/// model-definition edit invalidates this partition even when it leaves
/// the P4 projection's column set (and therefore `identity`) unchanged.
/// Before running the diff, this checks whether any stored row's stamp no
/// longer matches the freshly computed one and, if so, logs a `tracing::
/// warn!` — the diff itself always structurally excludes a mismatched row
/// from the comparison (`emit_fingerprint_sidecar_diff`'s own `stamp =
/// '...'` filter), so this check changes no behaviour, it only makes an
/// invalidation loud rather than silent.
#[allow(clippy::too_many_arguments)]
pub async fn diff_fingerprint_sidecar_changed_keys(
    backend: &dyn Backend,
    schema: &str,
    source_address: &str,
    source_table: &str,
    source_key: &[String],
    projection: &FingerprintProjection,
    all_source_columns: &[String],
    model_sql: &str,
) -> std::result::Result<Vec<String>, BackendError> {
    if backend.dialect() != SqlDialect::DuckDB {
        return Err(BackendError::unsupported(
            backend.dialect().name(),
            "fingerprint-sidecar diff for a mutable_snapshot external source (F3)",
        ));
    }
    let ensure_sql = ddl_duckdb::generate_fingerprint_sidecar_table_ddl(schema);
    backend.execute_sql(&ensure_sql).await?;

    let digest_columns = resolve_fingerprint_digest_columns(projection, all_source_columns);
    let identity = fingerprint::projection_identity(projection);
    let stamp = compute_fingerprint_sidecar_stamp(&identity, model_sql);

    let stale_check_sql = ddl_duckdb::generate_fingerprint_sidecar_stale_check_sql(
        schema,
        source_address,
        &identity,
        &stamp,
    );
    let stale_rows = backend.execute_sql(&stale_check_sql).await?;
    if stale_rows.iter().any(|batch| batch.num_rows() > 0) {
        tracing::warn!(
            source_address,
            projection_identity = %identity,
            "fingerprint sidecar stamp mismatch detected (model definition, P4 projection, or \
             digest version changed — or the stored stamp was corrupted); treating the stale \
             partition as absent and rebuilding via the whole-table delta"
        );
    }

    let sidecar_table = format!("{schema}.{}", ddl_duckdb::FINGERPRINT_SIDECAR_TABLE_NAME);
    let dialect = maintenance_dialect(backend.dialect());
    let diff_sql = emit_fingerprint_sidecar_diff(
        source_table,
        source_key,
        &digest_columns,
        &sidecar_table,
        source_address,
        &identity,
        &stamp,
        dialect,
    );
    let batches = backend.execute_sql(&diff_sql).await?;
    Ok(extract_delta_keys(&batches))
}

/// Extract every non-NULL `delta_key` value from a query's result batches —
/// shared by [`diff_fingerprint_sidecar_changed_keys`],
/// [`diff_repair_group_sidecar_changed_keys`], and the stored-output-key
/// read that function's degenerate-comparandum leg falls back to, so the
/// arrow-array downcast lives in exactly one place.
fn extract_delta_keys(batches: &[arrow::record_batch::RecordBatch]) -> Vec<String> {
    let mut keys = Vec::new();
    for batch in batches {
        let Some(col) = batch.column_by_name("delta_key") else {
            continue;
        };
        let Some(arr) = col.as_any().downcast_ref::<arrow::array::StringArray>() else {
            continue;
        };
        for i in 0..arr.len() {
            if !arr.is_null(i) {
                keys.push(arr.value(i).to_string());
            }
        }
    }
    keys
}

/// Write-side: refresh the fingerprint sidecar to match `source_table`'s
/// CURRENT content for `(source_address, projection)`, riding in the SAME
/// backend transaction as `write_group` — the consuming write this refresh
/// is paired with (`docs/specs/sources.md` §"The fingerprint sidecar" —
/// "Transactionality"). Call this AFTER
/// [`diff_fingerprint_sidecar_changed_keys`] has already read the
/// changed-key set the write is about to consume — refreshing first would
/// make a subsequent diff compare the source against itself and observe no
/// changes.
///
/// DuckDB-only, matching [`diff_fingerprint_sidecar_changed_keys`]'s own
/// posture; a non-DuckDB backend fails loudly rather than being handed
/// DuckDB-flavored SQL it cannot run.
///
/// `model_sql` must be the SAME consuming-model SQL text passed to the
/// paired [`diff_fingerprint_sidecar_changed_keys`] call this refresh
/// follows — it is folded into every refreshed row's stamp
/// ([`compute_fingerprint_sidecar_stamp`]), which is what "self-heals" a
/// stale partition: this upsert runs over every currently-observed key
/// (not just a changed subset), so it unconditionally re-stamps every
/// still-existing row with the current stamp, matching
/// `generate_fingerprint_sidecar_refresh_sql`'s own doc comment.
#[allow(clippy::too_many_arguments)]
pub async fn refresh_fingerprint_sidecar(
    backend: &dyn Backend,
    schema: &str,
    source_address: &str,
    source_table: &str,
    source_key: &[String],
    projection: &FingerprintProjection,
    all_source_columns: &[String],
    model_sql: &str,
    write_group: &StatementGroup,
) -> std::result::Result<(), BackendError> {
    if backend.dialect() != SqlDialect::DuckDB {
        return Err(BackendError::unsupported(
            backend.dialect().name(),
            "fingerprint-sidecar refresh for a mutable_snapshot external source (F3)",
        ));
    }
    let ensure_sql = ddl_duckdb::generate_fingerprint_sidecar_table_ddl(schema);
    let digest_columns = resolve_fingerprint_digest_columns(projection, all_source_columns);
    let identity = fingerprint::projection_identity(projection);
    let stamp = compute_fingerprint_sidecar_stamp(&identity, model_sql);
    let dialect = maintenance_dialect(backend.dialect());
    let digest_select =
        emit_fingerprint_digest_select(source_table, source_key, &digest_columns, dialect);
    let refresh_sql = ddl_duckdb::generate_fingerprint_sidecar_refresh_sql(
        schema,
        source_address,
        &identity,
        &stamp,
        &digest_select,
    );
    let gc_sql = ddl_duckdb::generate_fingerprint_sidecar_gc_sql(
        schema,
        source_address,
        &identity,
        &digest_select,
    );
    backend
        .execute_write_and_refresh_fingerprint_sidecar(
            &ensure_sql,
            write_group,
            &refresh_sql,
            &gc_sql,
        )
        .await
}

// ── P9: group-grain fingerprint sidecar (the repair family) ────────────
// (`docs/outcomes/20260809-repair-family/phases/09-plan.md`;
// `docs/specs/sources.md` §"The fingerprint sidecar" — "Partition grain";
// `docs/specs/incremental_models.md` §"The repair family" — "Obligation 7
// over a `mutable_snapshot` source")
//
// Same sidecar table, same stamp/invalidation machinery as the per-row
// grain above — only the partition identity and what one sidecar row
// represents differ. A group-grain partition never collides with a P4
// per-row partition: `repair_group_partition_identity`'s `repair:group=...`
// text can never equal `fingerprint::projection_identity`'s own `cols:...`/
// `full_row` shapes.

/// The repair-scoped sidecar partition identity for a group-grain digest:
/// `model` (via the caller's own `source_address`, unchanged — the
/// partition key stays `(source_address, projection_identity, source_key)`)
/// plus the group-key and digest-column sets, so two different repair cells
/// over the SAME source (a different group key, or a different digest
/// column set) land in different, non-colliding partitions, and neither
/// collides with the per-row [`fingerprint::projection_identity`] partition
/// a P4-driven consumer of the same source might also hold (that always
/// starts `cols:` or is exactly `full_row`; this always starts `repair:`).
fn repair_group_partition_identity(group_key: &[String], digest_columns: &[String]) -> String {
    format!(
        "repair:group={}:digest={}",
        group_key.join(","),
        digest_columns.join(",")
    )
}

/// Build a single-column `delta_key` relation (the same shape
/// [`repair_affected_keys_select`] and the sidecar-diff emitters project)
/// from an already-resolved list of literal key values — the bridge between
/// [`diff_repair_group_sidecar_changed_keys`]'s resolved `Vec<String>` (an
/// executed read, not opaque SQL text) and every downstream repair builder
/// ([`repair_candidate_select`], [`repair_slice_predicate`],
/// [`emit_per_group_recompute`]), which all consume `affected_keys_select`
/// as SQL text regardless of which discovery route produced it.
///
/// An empty `keys` list yields a well-typed EMPTY relation (`WHERE FALSE`),
/// never an invalid `VALUES ()` — a repair with no affected keys this run is
/// a legitimate (if unusual) outcome, not an error.
///
/// The row-set constructor itself — `VALUES (…)` where `dialect` supports
/// one, the portable `SELECT … UNION ALL SELECT …` rewrite GoogleSQL
/// requires otherwise — comes from `smelt_core::build_row_set_table`, the
/// single dialect-aware owner.
pub fn repair_keys_literal_select(keys: &[String], dialect: MaintenanceDialect) -> String {
    if keys.is_empty() {
        return "SELECT CAST(NULL AS VARCHAR) AS delta_key WHERE FALSE".to_string();
    }
    let rows: Vec<Vec<String>> = keys
        .iter()
        .map(|k| vec![format!("'{}'", k.replace('\'', "''"))])
        .collect();
    let row_set = smelt_core::build_row_set_table(
        maintenance_dialect_to_backend_type(dialect),
        "__smelt_repair_group_keys",
        &["delta_key"],
        &rows,
    );
    format!("SELECT * FROM {row_set}")
}

/// Map [`MaintenanceDialect`] (the maintenance-statement dialect, three
/// variants) to [`smelt_core::BackendType`] (the row-set owner's dialect
/// parameter) — a 1:1 relabeling, not a lossy collapse: both enumerate
/// exactly DuckDB, Spark, and BigQuery.
fn maintenance_dialect_to_backend_type(dialect: MaintenanceDialect) -> smelt_core::BackendType {
    match dialect {
        MaintenanceDialect::DuckDb => smelt_core::BackendType::DuckDB,
        MaintenanceDialect::Spark => smelt_core::BackendType::Spark,
        MaintenanceDialect::BigQuery => smelt_core::BackendType::BigQuery,
    }
}

/// Read-side: the repair family's group-grain affected-key set for a
/// `MutationProfile::MutableSnapshot` source with no native change feed
/// (P9). Mirrors [`diff_fingerprint_sidecar_changed_keys`]'s shape (ensure
/// the sidecar table, detect staleness, run the emitter-authored diff), but
/// over [`emit_repair_group_sidecar_diff`]'s group-grain digest and a
/// repair-scoped partition identity ([`repair_group_partition_identity`]),
/// and with one further obligation the per-row read has no analogue for:
/// **an absent or stale-stamped comparandum cannot distinguish "a group
/// that vanished" from "a group that never existed"**
/// (`docs/specs/incremental_models.md` §"The repair family" — "Obligation 7
/// over a `mutable_snapshot` source"), so for such a run the returned set
/// additionally unions every key currently present in the stored
/// `output_table` — a sound over-approximation that degenerates to a
/// whole-table repair for that one run and self-heals once
/// [`refresh_repair_group_sidecar`] populates a trustworthy comparandum.
///
/// DuckDB-only, matching every other sidecar consumer in this module — a
/// non-DuckDB backend fails loud (`BackendError::unsupported`) rather than
/// silently falling back to the unsound current-source scan.
#[allow(clippy::too_many_arguments)]
pub async fn diff_repair_group_sidecar_changed_keys(
    backend: &dyn Backend,
    schema: &str,
    source_address: &str,
    source_table: &str,
    output_table: &str,
    group_key: &[String],
    digest_columns: &[String],
    model_sql: &str,
) -> std::result::Result<Vec<String>, BackendError> {
    if backend.dialect() != SqlDialect::DuckDB {
        return Err(BackendError::unsupported(
            backend.dialect().name(),
            "group-grain fingerprint-sidecar diff for a mutable_snapshot repair source (P9)",
        ));
    }
    let ensure_sql = ddl_duckdb::generate_fingerprint_sidecar_table_ddl(schema);
    backend.execute_sql(&ensure_sql).await?;

    let identity = repair_group_partition_identity(group_key, digest_columns);
    let stamp = compute_fingerprint_sidecar_stamp(&identity, model_sql);

    let exists_sql = ddl_duckdb::generate_fingerprint_sidecar_partition_exists_sql(
        schema,
        source_address,
        &identity,
    );
    let exists_rows = backend.execute_sql(&exists_sql).await?;
    let partition_absent = !exists_rows.iter().any(|batch| batch.num_rows() > 0);

    let stale_check_sql = ddl_duckdb::generate_fingerprint_sidecar_stale_check_sql(
        schema,
        source_address,
        &identity,
        &stamp,
    );
    let stale_rows = backend.execute_sql(&stale_check_sql).await?;
    let has_stale = stale_rows.iter().any(|batch| batch.num_rows() > 0);
    if has_stale {
        tracing::warn!(
            source_address,
            projection_identity = %identity,
            "group-grain fingerprint sidecar stamp mismatch detected (model definition or \
             digest inputs changed, or the stored stamp was corrupted); treating the stale \
             partition as absent and widening the affected set to every currently-observed \
             group plus every stored output group"
        );
    }

    let sidecar_table = format!("{schema}.{}", ddl_duckdb::FINGERPRINT_SIDECAR_TABLE_NAME);
    let dialect = maintenance_dialect(backend.dialect());
    let diff_sql = emit_repair_group_sidecar_diff(
        source_table,
        group_key,
        digest_columns,
        &sidecar_table,
        source_address,
        &identity,
        &stamp,
        dialect,
    );
    let batches = backend.execute_sql(&diff_sql).await?;
    let mut keys = extract_delta_keys(&batches);

    if partition_absent || has_stale {
        let output_key_columns: Vec<String> = group_key
            .iter()
            .map(|k| format!("{output_table}.{k}"))
            .collect();
        let output_key_expr =
            smelt_logical::maintenance::emit::key_expr_for_columns(&output_key_columns);
        let stored_keys_sql = format!("SELECT {output_key_expr} AS delta_key FROM {output_table}");
        let stored_batches = backend.execute_sql(&stored_keys_sql).await?;
        keys.extend(extract_delta_keys(&stored_batches));
        keys.sort();
        keys.dedup();
    }

    Ok(keys)
}

/// Write-side: refresh the group-grain sidecar partition to match
/// `source_table`'s CURRENT group-grain digests, riding in the SAME backend
/// transaction as `write_group` — the repair's own write. Mirrors
/// [`refresh_fingerprint_sidecar`]'s shape and transactional contract
/// exactly (call AFTER [`diff_repair_group_sidecar_changed_keys`] has
/// already read the affected set this refresh's write is about to
/// consume), reusing the SAME DDL/DML the per-row sidecar uses
/// (`generate_fingerprint_sidecar_table_ddl`,
/// `generate_fingerprint_sidecar_refresh_sql`,
/// `generate_fingerprint_sidecar_gc_sql`) — only the digest select and
/// partition identity differ.
///
/// Populating this on the create/full-refresh path (in addition to every
/// live repair run) is what keeps a model's FIRST incremental repair from
/// taking the absent-comparandum degradation every single time: without an
/// initial populate, every run would find no partition, union in the whole
/// stored output, and never build a trustworthy baseline.
#[allow(clippy::too_many_arguments)]
pub async fn refresh_repair_group_sidecar(
    backend: &dyn Backend,
    schema: &str,
    source_address: &str,
    source_table: &str,
    group_key: &[String],
    digest_columns: &[String],
    model_sql: &str,
    write_group: &StatementGroup,
) -> std::result::Result<(), BackendError> {
    if backend.dialect() != SqlDialect::DuckDB {
        return Err(BackendError::unsupported(
            backend.dialect().name(),
            "group-grain fingerprint-sidecar refresh for a mutable_snapshot repair source (P9)",
        ));
    }
    let ensure_sql = ddl_duckdb::generate_fingerprint_sidecar_table_ddl(schema);
    let identity = repair_group_partition_identity(group_key, digest_columns);
    let stamp = compute_fingerprint_sidecar_stamp(&identity, model_sql);
    let dialect = maintenance_dialect(backend.dialect());
    let digest_select =
        emit_repair_group_digest_select(source_table, group_key, digest_columns, dialect);
    let refresh_sql = ddl_duckdb::generate_fingerprint_sidecar_refresh_sql(
        schema,
        source_address,
        &identity,
        &stamp,
        &digest_select,
    );
    let gc_sql = ddl_duckdb::generate_fingerprint_sidecar_gc_sql(
        schema,
        source_address,
        &identity,
        &digest_select,
    );
    backend
        .execute_write_and_refresh_fingerprint_sidecar(
            &ensure_sql,
            write_group,
            &refresh_sql,
            &gc_sql,
        )
        .await
}

/// Pure: decide the [`RecomputeRestriction`] verdict and build the
/// resulting [`StatementGroup`] — the single decision-and-emit call site
/// both [`execute_delete_insert_with_delta_restriction`]'s live executor
/// AND the `--dry-run`/`smelt explain` reporting path in
/// `crate::execute::execute_project` route through, so a dry-run's reported
/// statement can never structurally diverge from what a live run with the
/// same inputs would emit (`docs/specs/cli.md` §"`--dry-run` prints the
/// maintenance statements"). A dry-run has no backend to consult, so it
/// always calls this with `observed_delta: None` — [`resolve_recompute_
/// restriction`] then always resolves `Unrestricted`, so a dry-run's
/// reported text is always the ordinary widened scan (the honest choice:
/// a dry-run cannot know whether a live run's delta read would restrict).
#[allow(clippy::too_many_arguments)]
pub fn build_delete_insert_group_dispatched(
    table: &str,
    partition_col: &str,
    region: &Region,
    body: &str,
    restrict_column: Option<&str>,
    skeleton_source_closure: Option<&SkeletonSourceClosure>,
    observed_delta: Option<&[String]>,
    dialect: MaintenanceDialect,
) -> StatementGroup {
    let restriction = resolve_recompute_restriction(skeleton_source_closure, observed_delta);
    match (restrict_column, restriction) {
        (Some(col), RecomputeRestriction::Restricted { delta_keys }) => {
            emit_delete_insert_delta_restricted(
                table,
                partition_col,
                region,
                body,
                col,
                &delta_keys,
                dialect,
            )
        }
        _ => emit_delete_insert(table, partition_col, region, body, dialect),
    }
}

/// Execute a model-edge creation-trigger region recompute, restricting it to
/// an exact upstream delta's changed-key set when licensed
/// ([`resolve_recompute_restriction`]'s two-factor admission: P1 skeleton-
/// source closure `Closed` ∧ a non-empty recorded delta). Falls back to the
/// ordinary widened-scan [`emit_delete_insert`] — byte-identical to today's
/// unrestricted region recompute — for an `Open`/absent `skeleton_source_
/// closure`, no `restrict_column` (the cell has no proven row identity to
/// restrict on), an absent delta, or a present-but-empty one.
///
/// Returns the [`StatementGroup`] actually executed, mirroring
/// `execute_column_scoped_write_with_observed_delta`'s shape so a caller
/// (and a test) can assert on exactly what ran.
///
/// `body` and `probe_body` are deliberately two different strings: `body`
/// is what the emitted DELETE/INSERT actually executes (a caller compiling
/// through `SqlCompiler` passes its type-cast-wrapped `CompiledModel::sql`),
/// while `probe_body` is the pre-wrap body the count-preservation probe
/// below reads its enrichment join from (`CompiledModel::body_sql`). Never
/// derive one from the other here — see
/// `docs/plans/20260819-source-derived-projection.md` Phase 5.
#[allow(clippy::too_many_arguments)]
pub async fn execute_delete_insert_with_delta_restriction(
    backend: &dyn Backend,
    schema: &str,
    table: &str,
    partition_col: &str,
    region: &Region,
    body: &str,
    probe_body: &str,
    restrict_column: Option<&str>,
    skeleton_source_closure: Option<&SkeletonSourceClosure>,
    upstream_model: &str,
    window_start: &str,
    window_end: &str,
    dialect: MaintenanceDialect,
    retry: &crate::execute::RetryPolicy<'_>,
    probe_policy: &crate::probes::ProbePolicy,
) -> std::result::Result<StatementGroup, BackendError> {
    let full_table = format!("{schema}.{table}");
    let closed = skeleton_source_closure.is_some_and(|c| c.is_closed());
    let mut delta = if restrict_column.is_some() && closed {
        read_observed_delta_changed_keys(backend, schema, upstream_model, window_start, window_end)
            .await?
    } else {
        None
    };
    // The declared-`referential_integrity` route's row-preservation leg is
    // an *unverified* world-fact until a run actually probes it
    // (`model_properties.md` §"Skeleton-source closure", §"Probe
    // obligation"): a `JoinShape` (`LEFT JOIN`) route needs no runtime
    // check, but a `DeclaredReferentialIntegrity` route must dispatch the
    // count-preservation probe over the touched region *before* trusting
    // the restriction it licenses — never after the write, and never
    // silently skipped.
    let restriction_taken =
        restrict_column.is_some() && delta.as_deref().is_some_and(|d| !d.is_empty());
    if restriction_taken {
        if let Some(SkeletonSourceClosure::Closed {
            row_preservation: RowPreservation::DeclaredReferentialIntegrity { source },
        }) = skeleton_source_closure
        {
            let ctx = crate::probes::ProbeContext {
                probe_code: "SourceCountPreservationViolated".to_string(),
                fact: "referential_integrity".to_string(),
                model: table.to_string(),
                cell: format!("{full_table} declared-route delta restriction"),
                remedy: "correct or backfill the dimension's missing key, or drop the \
                         declaration"
                    .to_string(),
            };
            // The count-preservation probe's `driving_count`/`enriched_count`
            // row shape does not match the shared `violation_count`/
            // `sample_keys` contract `dispatch_probe` parses, so this site
            // consults `should_dispatch` directly (the same cadence policy,
            // the same single decision function) rather than reusing
            // `dispatch_probe`'s generic executor.
            match smelt_logical::maintenance::should_dispatch(
                probe_policy.cadence,
                probe_policy.run_ordinal,
            ) {
                smelt_logical::maintenance::ProbeDispatch::Skip(_) => {}
                smelt_logical::maintenance::ProbeDispatch::Dispatch => {
                    match emit_count_preservation_probe_from_body(probe_body, source) {
                        Some(probe) => {
                            let batches = backend.execute_sql(&probe.sql).await?;
                            let rows = crate::check_runner::batches_to_rows(&batches);
                            let (driving_count, enriched_count) = rows
                                .first()
                                .and_then(|r| {
                                    Some((
                                        r.get("driving_count")?.clone(),
                                        r.get("enriched_count")?.clone(),
                                    ))
                                })
                                .ok_or_else(|| BackendError::ExecutionFailed {
                                    model: table.to_string(),
                                    message: format!(
                                        "count-preservation probe for declared \
                                         referential_integrity on '{source}' returned no \
                                         driving_count/enriched_count row — refusing to trust \
                                         an unchecked declared-route narrowing"
                                    ),
                                })?;
                            let driving_count: i64 = driving_count.parse().unwrap_or(i64::MAX);
                            let enriched_count: i64 = enriched_count.parse().unwrap_or(-1);
                            if enriched_count < driving_count {
                                return Err(BackendError::ExecutionFailed {
                                    model: table.to_string(),
                                    message: format!(
                                        "SourceCountPreservationViolated: '{source}' declares \
                                         referential_integrity, but the enrichment join over the \
                                         touched region ({window_start}..{window_end}) returned \
                                         {enriched_count} row(s) against {driving_count} driving \
                                         row(s) — some driving row's join key has no match in the \
                                         dimension; correct or backfill the dimension's missing \
                                         key, or drop the declaration.{}",
                                        crate::probes::probe_violation_suffix(&ctx)
                                    ),
                                });
                            }
                        }
                        None => {
                            tracing::warn!(
                                source = %source,
                                table = %table,
                                "declared referential_integrity closure could not build a \
                                 count-preservation probe from this model's own body — dropping \
                                 the delta restriction and falling back to the widened scan"
                            );
                            delta = None;
                        }
                    }
                }
            }
        }
    }
    let group = build_delete_insert_group_dispatched(
        &full_table,
        partition_col,
        region,
        body,
        restrict_column,
        skeleton_source_closure,
        delta.as_deref(),
        dialect,
    );
    crate::execute::retry_backend_call(retry, || backend.execute_statement_group(&group)).await?;
    Ok(group)
}

/// The facts [`build_delete_insert_group_dispatched`]/
/// [`execute_delete_insert_with_delta_restriction`] need to attempt T3 delta
/// restriction for a model-edge-sourced creation cell, resolved by
/// [`resolve_live_delta_restriction_facts`].
#[derive(Debug, Clone)]
pub struct DeltaRestrictionFacts {
    /// The driving model edge's bare address (`Trigger::NewData`'s `source`
    /// name) — the upstream whose observed-delta table is read.
    pub upstream_model: String,
    /// The model's own region row identity, when it resolves to exactly one
    /// column (`RowIdentity::Key(_)` with one element) — this phase's
    /// semi-join restriction is single-column only. `None` for a composite
    /// key or `RowIdentity::WholeRow`, in which case the caller must fall
    /// back to the ordinary widened scan (matching an absent P1 closure).
    pub restrict_column: Option<String>,
    /// The cell's P1 skeleton-source-closure verdict, carried through
    /// unchanged for [`resolve_recompute_restriction`] to consult.
    pub skeleton_source_closure: Option<SkeletonSourceClosure>,
}

/// Resolve [`DeltaRestrictionFacts`] for a model driven (at least in part)
/// by an upstream **maintained-model** edge (`model_edges`, built by the
/// caller mirroring `crate::propagation::derive_clamp_and_locality`'s own
/// edge extraction — never re-derived here). Routes through the SAME
/// edge-aware derivation `smelt explain`/the propagation graph already
/// consume (`derive_model_maintenance_plan_with_edges` →
/// `append_model_edge_cells`) rather than re-implementing admission
/// (`CLAUDE.md` §"Maintenance-plan purity").
///
/// `model_edges.first()` is this call's driving edge: `append_model_edge_
/// cells` derives ONE shared P1 closure verdict for every edge of a model
/// (see that function's own doc comment — the verdict is a property of the
/// model's own query shape, not of which edge triggered the recompute), so
/// picking any one edge's cell yields the same closure either way; the
/// first edge is simply the one whose observed-delta table the caller then
/// reads. A model with more than one maintained-model upstream restricts
/// only against this first edge's delta in this phase — a later phase may
/// widen this to try every edge in turn.
///
/// Returns `None` when `model_edges` is empty, the plan derives no creation
/// cell for the driving edge (e.g. `Refusal::ReachNotDerivable`), or
/// `metadata`'s resolved grain has no partition axis for a model edge to
/// clamp to — the caller's safe default in every `None` case is the
/// ordinary widened scan.
pub fn resolve_live_delta_restriction_facts(
    sql: &str,
    table: &str,
    metadata: &smelt_core::ModelMetadata,
    sources: &[SourceFacts],
    explicitly_mutable: &HashSet<String>,
    model_edges: &[smelt_logical::maintenance::derive::ModelEdge],
) -> Option<DeltaRestrictionFacts> {
    let driving_edge = model_edges.first()?;
    let result = smelt_db::queries::maintenance::derive_model_maintenance_plan_with_edges(
        sql,
        table,
        metadata,
        sources,
        explicitly_mutable,
        model_edges,
        // See `resolve_incremental_strategy`'s analogous call: not (yet)
        // plumbed with a driving-source granularity or declared
        // `key_recurrence` bounds at this call site — this resolver only
        // reads the model-edge creation cell's closure/row-identity facts,
        // which key temporal locality's routes do not gate.
        None,
        &[],
        // This resolver only reads the model-edge `NewData` creation cell —
        // a `ColumnAdded` trigger never affects it, so no deployed-schema
        // snapshot is needed here.
        &[],
        &SourceReferentialIntegrity::new(),
        None,
    )?;
    let cell = result.plan.cell_for(&Trigger::NewData {
        source: driving_edge.name.clone(),
    })?;
    let restrict_column = match &cell.row_identity.identity {
        RowIdentity::Key(cols) if cols.len() == 1 => Some(cols[0].clone()),
        _ => None,
    };
    Some(DeltaRestrictionFacts {
        upstream_model: driving_edge.name.clone(),
        restrict_column,
        skeleton_source_closure: cell.skeleton_source_closure.clone(),
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
    columns: &[String],
    suppression: &WriteSuppression,
    window: &PartitionRange,
    retry: &crate::execute::RetryPolicy<'_>,
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
    execute_column_scoped_write_with_observed_delta(
        backend,
        schema,
        table,
        unique_key,
        &source_sql,
        columns,
        suppression,
        dialect,
        window,
        retry,
    )
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

    /// A retry policy that never retries — these unit tests exercise the
    /// driver against `RecordingBackend`, a synchronous test double, so
    /// there is no `ExecuteRequest`/run reporter to derive a policy from
    /// (`docs/plans/20260719-prod-w2-operability.md` Phase 6). Retry
    /// behaviour itself is covered end-to-end by `tests/retry.rs`.
    const NO_OP_REPORTER: crate::reporter::NoOpReporter = crate::reporter::NoOpReporter;
    fn no_retry_policy() -> crate::execute::RetryPolicy<'static> {
        crate::execute::RetryPolicy {
            retry_max: 0,
            base_backoff_ms: 0,
            run_id: "maintenance-driver-unit-test",
            model_name: "maintenance-driver-unit-test",
            reporter: &NO_OP_REPORTER,
        }
    }

    /// The rejection test at the `maintenance_driver` call site: for
    /// `MaintenanceDialect::BigQuery` the emitted affected-keys relation is
    /// never a `FROM (VALUES …)` table-value constructor, and for
    /// DuckDB/Spark it is byte-identical (both route through the shared
    /// `smelt_core::build_row_set_table` owner).
    #[test]
    fn repair_keys_literal_select_bigquery_is_not_a_values_constructor() {
        let keys = vec!["a".to_string(), "b".to_string()];
        let select = repair_keys_literal_select(&keys, MaintenanceDialect::BigQuery);
        assert!(
            !select.contains("VALUES"),
            "BigQuery has no table-value constructor, got: {select}"
        );
        assert!(select.contains("UNION ALL"));
    }

    #[test]
    fn repair_keys_literal_select_duckdb_and_spark_are_byte_identical() {
        let keys = vec!["a".to_string(), "b".to_string()];
        let duckdb = repair_keys_literal_select(&keys, MaintenanceDialect::DuckDb);
        let spark = repair_keys_literal_select(&keys, MaintenanceDialect::Spark);
        assert_eq!(duckdb, spark);
        assert_eq!(
            duckdb,
            "SELECT * FROM (VALUES ('a'), ('b')) AS __smelt_repair_group_keys(delta_key)"
        );
    }

    #[test]
    fn repair_keys_literal_select_empty_keys_is_dialect_independent() {
        let empty: Vec<String> = vec![];
        for dialect in [
            MaintenanceDialect::DuckDb,
            MaintenanceDialect::Spark,
            MaintenanceDialect::BigQuery,
        ] {
            let select = repair_keys_literal_select(&empty, dialect);
            assert_eq!(
                select,
                "SELECT CAST(NULL AS VARCHAR) AS delta_key WHERE FALSE"
            );
        }
    }

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
            _dialect: MaintenanceDialect,
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
            _columns: &[String],
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
            _dialect: MaintenanceDialect,
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
            _dialect: MaintenanceDialect,
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
            &no_retry_policy(),
            &crate::probes::ProbePolicy::per_run(),
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
            &no_retry_policy(),
            &crate::probes::ProbePolicy::per_run(),
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
            &no_retry_policy(),
            &crate::probes::ProbePolicy::per_run(),
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
            &no_retry_policy(),
            &crate::probes::ProbePolicy::per_run(),
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
    /// (`docs/specs/incremental_shapes.md` §"Key temporal locality", route
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
            _dialect: MaintenanceDialect,
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
            &no_retry_policy(),
            &crate::probes::ProbePolicy::per_run(),
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
            skeleton_source_closure: None,
            fingerprint_projections: std::collections::BTreeMap::new(),
            key_scope: None,
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
            skeleton_source_closure: None,
            fingerprint_projections: std::collections::BTreeMap::new(),
            key_scope: None,
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
