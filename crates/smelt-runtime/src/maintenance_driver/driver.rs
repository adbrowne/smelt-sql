use crate::transformer::{add_seconds_to_date, subtract_seconds_from_date, TimeRange};
use anyhow::{bail, Context, Result};
use smelt_backend::{Backend, BackendError, ExecutionResult};
use smelt_core::config::Granularity;
use smelt_dialect::SqlDialect;
use smelt_logical::maintenance::choice::WriteSuppression;
use smelt_logical::maintenance::emit::{
    emit_create_table_as, MaintenanceDialect, MaintenanceStatement, StatementGroup,
    TargetSlicePredicate,
};
use smelt_logical::maintenance::locality::LocalitySlice;
use smelt_logical::maintenance::WritePattern;
use smelt_state::ddl_duckdb;
use smelt_state::reconciliation::Grade;
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
                axis: smelt_logical::PartitionAxis::Calendar,
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

    /// Build the [`StatementGroup`] that actually realises `mechanism`
    /// (`smelt_logical::maintenance::choice::resolve_keyed_write_mechanism`,
    /// 27d/27g) — the write-pin-aware counterpart of [`Self::merge_sql`].
    /// Defaults to a one-statement group wrapping [`Self::merge_sql`] for
    /// [`KeyedWriteMechanism::Merge`], so the unpinned path (every rule that
    /// never sees a `staged_candidate` pin) stays byte-identical to the
    /// pre-27g `merge_sql`-only dispatch. There is no default shape for
    /// [`KeyedWriteMechanism::StagedCandidate`] — a rule reaching that arm
    /// must override this method (`crate::cumulative::CumulativeClassification`
    /// does); the default panics rather than silently falling back to
    /// `merge_sql`, since `resolve_keyed_write_mechanism` only ever produces
    /// `StagedCandidate` for a rule this driver's real (`keyed`) family
    /// serves.
    fn write_group(
        &self,
        schema: &str,
        table: &str,
        delta_sql: &str,
        slice: Option<&TargetSlicePredicate>,
        mechanism: &smelt_logical::maintenance::choice::KeyedWriteMechanism,
        dialect: MaintenanceDialect,
    ) -> StatementGroup {
        use smelt_logical::maintenance::choice::KeyedWriteMechanism;
        match mechanism {
            KeyedWriteMechanism::Merge(suppression) => StatementGroup {
                statements: vec![MaintenanceStatement {
                    sql: self.merge_sql(schema, table, delta_sql, slice, suppression, dialect),
                }],
                transactional: false,
            },
            KeyedWriteMechanism::StagedCandidate { .. } => {
                panic!(
                    "windowed-keyed-maintenance driver: rule produced \
                     KeyedWriteMechanism::StagedCandidate but does not override \
                     WindowedKeyedRule::write_group to realise it"
                )
            }
        }
    }

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

    /// Build the changed-keys `SELECT` a suppressed keyed fold's observed
    /// output delta is recorded from (T5,
    /// `docs/specs/incremental_models.md` §"The graph layer" — "Observed
    /// deltas on model edges") — the rule supplies its own `unique_key` and
    /// fold expressions, which the driver does not otherwise know.
    /// `partition_column`, when `Some`, is the locality-admitted model's own
    /// declared partition column (the record's touched-partition
    /// projection); `None` for a bare keyed model with no partition axis.
    /// `None` refuses recording fail-closed for a rule with no keyed-fold
    /// shape at all — the default here exists only so a rule that never
    /// reaches the suppressed-idempotent branch below need not implement
    /// it; `keyed`'s own impl
    /// (`crate::cumulative::CumulativeClassification`) always returns
    /// `Some`.
    fn observed_delta_changed_keys_sql(
        &self,
        schema: &str,
        table: &str,
        delta_sql: &str,
        compared_columns: &[String],
        partition_column: Option<&str>,
    ) -> Option<String> {
        let _ = (schema, table, delta_sql, compared_columns, partition_column);
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
/// double-counting. `Grade::Idempotent` cells also write to the same ledger
/// table, keyed identically, but via an `ON CONFLICT DO NOTHING` upsert
/// rather than a refusal — a re-run-tolerant model's re-merge of an
/// already-recorded window is a no-op, not an error
/// (`docs/specs/incremental_shapes.md` §"The transactional frontier write
/// (merge ledger)").
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
    write_pin: Option<&'static WritePattern>,
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

    // Resolve the write mechanism once, before any step runs (`docs/outcomes/
    // 20260815-definition-delta-migrate/phases/27g-plan.md`): a `write:` pin
    // selects between the keyed `MERGE` and the merge-less staged-candidate
    // mechanism within the `KeyedFold` technique family
    // (`smelt_logical::maintenance::choice::resolve_keyed_write_mechanism`).
    // `Err` (a pin the backend/suppression combination cannot honour) and
    // `Ok(None)` (neither mechanism is admissible) both refuse the whole run
    // before any backend call — the same fail-closed posture `rule.refuse()`
    // above already establishes for combiner safety.
    let mechanism = match smelt_logical::maintenance::choice::resolve_keyed_write_mechanism(
        suppression,
        backend.capabilities().supports_merge,
        write_pin,
    ) {
        Ok(Some(mechanism)) => mechanism,
        Ok(None) => bail!(
            "windowed-keyed-maintenance driver refused model '{}': the backend cannot run \
             MERGE and the write is unconditional (no comparable compared-column set) — no \
             merge-less unconditional keyed-fold mechanism exists",
            model_name
        ),
        Err(refusal) => bail!(
            "windowed-keyed-maintenance driver refused model '{}': {}",
            model_name,
            refusal
        ),
    };

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
        let action_group = match &create_group {
            Some(group) => group.clone(),
            None => rule.write_group(
                schema,
                table,
                &delta_sql,
                slice_predicate.as_ref(),
                &mechanism,
                smelt_backend::maintenance_dialect(backend.dialect()),
            ),
        };
        let action_sql = action_group
            .statements
            .iter()
            .map(|s| s.sql.as_str())
            .collect::<Vec<_>>()
            .join(";\n");

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

                // `fold_ledger_delta` wraps exactly one action statement
                // (`Backend::fold_ledger_delta`'s own signature) — the
                // staged-candidate mechanism's multi-statement transactional
                // group has no ledger-folded realisation today. An
                // additive-graded cell always resolves `KeyedWriteMechanism::
                // Merge` absent a `staged_candidate` pin, so this only fires
                // for a pin explicitly requesting the merge-less mechanism
                // over an additive fold — refuse fail-closed rather than
                // silently mis-wrapping it.
                if action_group.statements.len() != 1 {
                    bail!(
                        "windowed-keyed-maintenance driver refused model '{}': the resolved \
                         write mechanism emits {} statements, but the never-fold-twice \
                         reconciliation ledger (MP12) only wraps a single action statement — \
                         an additive-graded cell has no ledger-folded staged-candidate \
                         realisation",
                        model_name,
                        action_group.statements.len()
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
                // `fold_ledger_delta`.
                //
                // A change-suppressed keyed fold's MERGE (T5, `docs/specs/
                // incremental_models.md` §"The graph layer" — "Observed
                // deltas on model edges") additionally records its observed
                // output delta in the SAME backend transaction as the
                // write — but only past the first (table-creating) step:
                // `create_group` is a plain `CREATE TABLE ... AS`, not a
                // conditional write, so there is no changed-row set to
                // record from it. `Grade::Additive` (ledger-folded) ISN'T
                // reached here — this arm only ever runs for
                // `Grade::Idempotent` cells.
                //
                // Every step ALSO writes a re-run-tolerance bookkeeping
                // record into the SAME merge ledger the `Additive` arm
                // above uses (`docs/specs/incremental_shapes.md` §"The
                // transactional frontier write (merge ledger)" — "every
                // window-forward keyed model maintains a per-model
                // frontier", unqualified by grading), keyed identically —
                // `LEDGER_WHOLE_ROW_GROUP`/`rule.ledger_input()` — via
                // `ON CONFLICT DO NOTHING` rather than `Additive`'s
                // never-fold-twice `PRIMARY KEY` refusal, since a repeat
                // merge of the same window is never a correctness violation
                // for an idempotent cell. The first (table-creating) step
                // is recorded too — that window is merged state — because
                // it always falls into the `_` arm below (its `create_group`
                // is never the `(None, Suppressed)` pattern the first arm
                // matches). The ledger substrate is DuckDB-only today (same
                // posture as the `Additive` arm and the observed-delta
                // record below); on any other dialect the record is
                // skipped — never silently, but the channel is no longer a
                // reporter event (the old `RunReporter` stand-in method was
                // retired, `docs/outcomes/20260904-state-residency/
                // outcome.md` phase 6): the affected cell's own recorded
                // `state_downgrade` (`smelt-logical`'s `resolve_availability`)
                // is now the user-visible channel, surfaced by `smelt
                // explain` — this is bookkeeping, not a correctness gate, so
                // the run itself proceeds.
                let ledger_bookkeeping = if backend.dialect() == SqlDialect::DuckDB {
                    let ledger_ensure = ddl_duckdb::generate_ledger_table_ddl(schema);
                    let ledger_upsert = ddl_duckdb::generate_ledger_upsert_sql(
                        schema,
                        model_name,
                        LEDGER_WHOLE_ROW_GROUP,
                        rule.ledger_input(),
                        &step.partition_value,
                        &step.range.start,
                        &step.range.end,
                    );
                    Some((ledger_ensure, ledger_upsert))
                } else {
                    tracing::debug!(
                        model = model_name,
                        run_id = retry.run_id,
                        dialect = backend.dialect().name(),
                        "re-run-tolerant keyed model merge-ledger bookkeeping record skipped: \
                         the ledger substrate is DuckDB-only today — the affected cell's own \
                         recorded state_downgrade is the user-visible channel"
                    );
                    None
                };

                match (&create_group, suppression) {
                    (None, WriteSuppression::Suppressed { compared_columns }) => {
                        if backend.dialect() != SqlDialect::DuckDB {
                            bail!(
                                "{}",
                                BackendError::unsupported(
                                    backend.dialect().name(),
                                    "observed-delta recording for a change-suppressed keyed \
                                     fold (T5)",
                                )
                            );
                        }
                        let partition_column = locality.map(LocalitySlice::partition_column);
                        let changed_keys_query = match rule.observed_delta_changed_keys_sql(
                            schema,
                            table,
                            &delta_sql,
                            compared_columns,
                            partition_column,
                        ) {
                            Some(sql) => sql,
                            None => bail!(
                                "windowed-keyed-maintenance driver refused model '{}': a \
                                 change-suppressed keyed fold requires the rule to provide an \
                                 observed-delta changed-keys query, and none was provided — \
                                 refusing fail-closed rather than silently skipping the record",
                                model_name
                            ),
                        };
                        let ensure_sql = ddl_duckdb::generate_observed_delta_table_ddl(schema);
                        let record_sql = ddl_duckdb::generate_observed_delta_upsert_sql(
                            schema,
                            table,
                            &step.range.start,
                            &step.range.end,
                            &changed_keys_query,
                        );
                        let mut ensure_sqls = vec![ensure_sql];
                        let mut pre_write_sqls = vec![record_sql];
                        if let Some((ledger_ensure, ledger_upsert)) = &ledger_bookkeeping {
                            ensure_sqls.push(ledger_ensure.clone());
                            pre_write_sqls.push(ledger_upsert.clone());
                        }
                        crate::execute::retry_backend_call(retry, || {
                            backend.execute_write_with_bookkeeping(
                                &ensure_sqls,
                                &pre_write_sqls,
                                &action_group,
                            )
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
                    _ => match &ledger_bookkeeping {
                        Some((ledger_ensure, ledger_upsert)) => {
                            let ensure_sqls = vec![ledger_ensure.clone()];
                            let pre_write_sqls = vec![ledger_upsert.clone()];
                            crate::execute::retry_backend_call(retry, || {
                                backend.execute_write_with_bookkeeping(
                                    &ensure_sqls,
                                    &pre_write_sqls,
                                    &action_group,
                                )
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
                        None => {
                            crate::execute::retry_backend_call(retry, || {
                                backend.execute_statement_group(&action_group)
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
                    },
                }
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
