//! Execution loops for `refresh: keyed` table models, one per derived run
//! shape (`docs/specs/incremental_shapes.md` §"The two run shapes").
//!
//! **Window-forward** (`execute_cumulative_aggregate`), for a run window
//! `[run_start, run_end)`:
//!
//! 1. Classify the model's SQL (`smelt_planner::classify_cumulative`).
//! 2. Step over the driving source's partitions in temporal order.
//! 3. For each partition `D`: source-filter pushdown injects
//!    `<driving_source>.<partition_col> ∈ [D, D + granularity)` and the
//!    rule either creates the target table from the delta SELECT (first
//!    run) or emits a combiner-aware `MERGE INTO`.
//!
//! **Snapshot-reconcile** (`execute_snapshot_reconcile`, Phase 3,
//! `docs/plans/20260809-keyed-frontier.md`): no window — the whole source
//! is re-scanned every run; see that function's own doc comment.

use crate::compile::{CompilerRegistry, EphemeralResolver};
use crate::maintenance_driver::{
    driving_steps, keyed_fold_changed_keys_select, run_windowed_keyed_maintenance,
    WindowedKeyedRule,
};
use crate::transformer::{inject_source_filters, SourceBound, TimeRange};
use anyhow::{Context, Result};
use smelt_backend::{Backend, ExecutionResult};
use smelt_core::ModelFile;
use smelt_logical::analysis::join_shape::JoinContext;
use smelt_logical::analysis::walk::model_property_vector;
use smelt_logical::contract::retain_departed::{reconcile_disposition, DepartedKeyDisposition};
use smelt_logical::maintenance::choice::{
    resolve_write_suppression, resolve_write_variant, EffectiveOverride, WriteSuppression,
};
use smelt_logical::maintenance::derive::row_identity;
use smelt_logical::maintenance::emit::{
    emit_departed_key_delete, emit_keyed_fold, emit_keyed_fold_suppressed,
    emit_recurrence_bound_probe, MaintenanceDialect, MaintenanceStatement, StatementGroup,
    TargetSlicePredicate,
};
use smelt_logical::maintenance::locality::{
    establish_locality, partition_column_provably_not_null, LocalityInputs, LocalitySlice,
};
use smelt_logical::maintenance::Trigger;
use smelt_planner::{
    classify_cumulative, combiner_for, CrossPartitionCombiner, CumulativeClassification,
    KeyedDiagnostic, SourceTimeseriesMap,
};
use smelt_state::reconciliation::Grade;
use std::collections::HashMap;
use tracing::info;

/// The state-column combiner allowlist `WindowedKeyedRule::refuse`'s
/// defense-in-depth pass verifies a decomposed-state column's own state
/// columns against — every combiner shape a state column can carry today
/// (`analysis::decomposed_state::decompose_to_state`/`decompose_once_write`,
/// `docs/outcomes/20260809-rung2-state-shapes`).
fn is_recognised_state_combiner(combiner: &CrossPartitionCombiner) -> bool {
    matches!(
        combiner,
        CrossPartitionCombiner::Sum
            | CrossPartitionCombiner::Min
            | CrossPartitionCombiner::Max
            | CrossPartitionCombiner::BoolAnd
            | CrossPartitionCombiner::BoolOr
            | CrossPartitionCombiner::BitAnd
            | CrossPartitionCombiner::BitOr
            | CrossPartitionCombiner::BitXor
            | CrossPartitionCombiner::OrderMonotone { .. }
            | CrossPartitionCombiner::OnceWrite
    )
}

/// `keyed`'s [`WindowedKeyedRule`] impl: its classification already
/// gated every aggregator column through `combiner_for` (the monoid-only
/// allowlist) at classify time, but the driver re-checks independently —
/// defense in depth against a future classifier bug ever handing the driver
/// an unsafe combiner (`model_transforms.md` §Constraints "Equivalence or
/// refusal").
impl WindowedKeyedRule for CumulativeClassification {
    fn refuse(&self) -> Option<String> {
        for col in &self.aggregator_columns {
            // A state-bearing column's presented value has no monoid fold of
            // its own — `MAX_BY`/`MIN_BY`'s `OrderMonotone`, once-write's
            // fallback/multi-candidate state, and the decomposed-fold
            // family's `Recomputed` all fold through their hidden state
            // columns instead of `per_partition_agg`
            // (`docs/outcomes/20260809-rung2-state-shapes` row 7). Check
            // `col.state` first and re-verify each state column's own
            // combiner against the same allowlist below, rather than
            // consulting `combiner_for(per_partition_agg)` (`AVG`/
            // `STDDEV_*`/`VAR_*` never appear in that allowlist — it is a
            // fold over the *presented* value, which a state-bearing column
            // never has).
            if let Some(state) = &col.state {
                for state_col in &state.state_columns {
                    if !is_recognised_state_combiner(&state_col.combiner) {
                        return Some(format!(
                            "internal error: state column `{}` backing `{}` carries an \
                             unrecognised combiner — the classifier only derives already- \
                             recognised combiners into decomposed state",
                            state_col.name, col.output_name
                        ));
                    }
                }
                continue;
            }

            match &col.cross_partition_combiner {
                // The order-monotone overwrite family (`MAX_BY`/`MIN_BY`) is
                // not a monoid — `combiner_for`'s allowlist deliberately
                // doesn't cover it (`analysis::discriminants::Monotone::Order`
                // is a semilattice fold, not a commutative monoid). It is
                // already verified at classify time
                // (`rules::cumulative::classify_order_monotone_column`), so
                // this defense-in-depth pass has nothing further to check.
                CrossPartitionCombiner::OrderMonotone { .. } => {}
                // The plain-overwrite family (`ANY_VALUE`) is the
                // snapshot-reconcile run shape's own family — it never
                // reaches this window-forward driver (the classifier
                // refuses it window-forward), but is matched explicitly
                // rather than falling into the `combiner_for` allowlist
                // check below, which would spuriously refuse it.
                CrossPartitionCombiner::PlainOverwrite => {}
                // The once-write family (`COALESCE`) is likewise not a
                // monoid `combiner_for` allowlists — its admission proof
                // (key-derived, or a declared functional dependency over
                // the coalesced value's source column, not structurally
                // disproven by a fan-out join or a set-operation barrier)
                // is already verified at
                // classify time (`rules::cumulative::classify_once_write`),
                // so this defense-in-depth pass has nothing further to
                // check (`docs/plans/20260809-keyed-frontier.md` Phase 4).
                CrossPartitionCombiner::OnceWrite => {}
                // Unreachable in a well-formed classification: every
                // `Recomputed` column the classifier produces
                // (`rules::cumulative::classify_decomposed_fold_column`)
                // always carries `state: Some(..)`, caught by the branch
                // above. Reaching this arm means a `Recomputed` column with
                // no state slipped through classification — an internal
                // invariant violation, not a model error.
                CrossPartitionCombiner::Recomputed => {
                    return Some(format!(
                        "internal error: aggregator `{}` on column `{}` is `Recomputed` but \
                         carries no decomposed state — a `Recomputed` presented column must \
                         always be state-bearing",
                        col.per_partition_agg, col.output_name
                    ));
                }
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
        dialect: MaintenanceDialect,
    ) -> String {
        build_cumulative_merge_sql(schema, table, delta_sql, self, slice, suppression, dialect)
    }

    /// Realises a `write: staged_candidate` pin's mechanism
    /// (`resolve_keyed_write_mechanism`, 27d/27g): the merge-less
    /// staged-candidate group over the fold's own post-fold candidate rows
    /// ([`keyed_fold_candidate_select`]), mirroring what
    /// [`build_cumulative_merge_sql`]'s matched arm computes for the
    /// `MERGE`-capable path. `slice` (key temporal locality) has no
    /// staged-candidate realisation yet — this mechanism is reachable only
    /// for a bare keyed model until locality composes with it. `Merge`
    /// dispatches to the trait default (wraps [`Self::merge_sql`]).
    #[allow(clippy::too_many_arguments)]
    fn write_group(
        &self,
        schema: &str,
        table: &str,
        delta_sql: &str,
        slice: Option<&TargetSlicePredicate>,
        mechanism: &smelt_logical::maintenance::choice::KeyedWriteMechanism,
        dialect: MaintenanceDialect,
        capabilities: &smelt_backend::BackendCapabilities,
    ) -> smelt_logical::maintenance::emit::StatementGroup {
        use smelt_logical::maintenance::choice::KeyedWriteMechanism;
        match mechanism {
            KeyedWriteMechanism::Merge(suppression) => {
                smelt_logical::maintenance::emit::StatementGroup {
                    statements: vec![smelt_logical::maintenance::emit::MaintenanceStatement {
                        sql: self.merge_sql(schema, table, delta_sql, slice, suppression, dialect),
                    }],
                    transactional: false,
                }
            }
            KeyedWriteMechanism::StagedCandidate { compared_columns } => {
                let folds: Vec<(String, String)> = self
                    .aggregator_columns
                    .iter()
                    .flat_map(smelt_logical::maintenance::emit::expand_aggregator_column_folds)
                    .collect();
                let schema_table = format!("{schema}.{table}");
                let candidate_select =
                    smelt_logical::maintenance::emit::keyed_fold_candidate_select(
                        &schema_table,
                        &self.unique_key,
                        &folds,
                        delta_sql,
                        dialect,
                    );
                let staged_relation =
                    smelt_logical::maintenance::emit::StagedRelation::derive_for_capabilities(
                        "__smelt_staged_",
                        table,
                        capabilities,
                    );
                smelt_logical::maintenance::emit::emit_staged_candidate_conditional(
                    &schema_table,
                    &staged_relation,
                    &self.unique_key,
                    &candidate_select,
                    compared_columns,
                    dialect,
                )
            }
        }
    }

    /// `Grade::Additive` iff any aggregator column's cross-partition
    /// combiner belongs to the **additive fold** family — `Sum` or `BitXor`
    /// (`docs/specs/incremental_shapes.md` §"The column-family catalogue")
    /// — since re-merging an already-reflected delta does not converge for
    /// either (`docs/specs/incremental_models.md` §"The reconciliation
    /// ledger" — "Storage is graded by algebra"). `Sum` double-counts
    /// (`x + d + d`); `BitXor` is self-inverse and *cancels* the window's
    /// contribution (`x XOR d XOR d == x`) — a different corruption, the
    /// same non-idempotence, so both must keep a ledger and refuse a
    /// reprocessed window rather than silently write wrong state.
    ///
    /// The remaining catalogued combiners (`Min`/`Max`/`BoolAnd`/`BoolOr`/
    /// `BitAnd`/`BitOr`, the order-monotone overwrite family
    /// `OrderMonotone`, and the once-write family `OnceWrite`) grade
    /// `Idempotent`: re-merging the SAME already-reflected delta twice
    /// leaves the state unchanged — the lattice combiners are idempotent
    /// (`GREATEST(x, d) == GREATEST(GREATEST(x, d), d)`), the incumbent-wins
    /// comparison is false the second time (after the first merge
    /// `target.ord == delta.ord`), and `COALESCE(target.c, delta.c)` is a
    /// no-op once `target.c` is set — a re-run converges. Mixing an additive
    /// column with idempotent ones in the same cell still grades the whole
    /// cell `Additive` — conservative (never unsafe), per
    /// `WindowedKeyedRule::ledger_grade`'s doc comment.
    fn ledger_grade(&self) -> Grade {
        // Delegates to the single owner of the re-run-tolerance verdict
        // (`smelt_logical::rules::cumulative::execution_postures`,
        // `docs/outcomes/20260815-keyed-grain-residue` phase 4) rather than
        // re-deriving it here — this rule's own doc comment above states
        // the rationale (additive columns double-count/cancel on a re-run),
        // but the derivation itself lives in `smelt-logical` so `smelt
        // explain` prints the same verdict this grading consumes.
        if smelt_logical::execution_postures(&self.aggregator_columns)
            .rerun_tolerant
            .holds
        {
            Grade::Idempotent
        } else {
            Grade::Additive
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
    #[allow(clippy::too_many_arguments)]
    fn recurrence_probe_sql(
        &self,
        schema: &str,
        table: &str,
        delta_sql: &str,
        partition_column: &str,
        slice_lower: &str,
        column_type: smelt_logical::maintenance::emit::PartitionColumnType,
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
                column_type,
                dialect,
            )
            .sql,
        )
    }

    /// `keyed`'s own `unique_key` plus its aggregator columns' rendered
    /// fold expressions (the SAME `expand_aggregator_column_folds` call
    /// [`build_cumulative_merge_sql`] uses to build the live MERGE) are
    /// exactly what [`keyed_fold_changed_keys_select`] needs — this impl
    /// supplies them and delegates the query shape entirely to that
    /// function.
    fn observed_delta_changed_keys_sql(
        &self,
        schema: &str,
        table: &str,
        delta_sql: &str,
        compared_columns: &[String],
        partition_column: Option<&str>,
        dialect: MaintenanceDialect,
    ) -> Option<String> {
        use smelt_logical::maintenance::emit::expand_aggregator_column_folds as expand;
        let folds: Vec<(String, String)> =
            self.aggregator_columns.iter().flat_map(expand).collect();
        Some(keyed_fold_changed_keys_select(
            &format!("{schema}.{table}"),
            &self.unique_key,
            delta_sql,
            compared_columns,
            &folds,
            partition_column,
            dialect,
        ))
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
    source_infos: &[smelt_core::SourceInfo],
    verbose: bool,
    retry: &crate::execute::RetryPolicy<'_>,
    probe_policy: &crate::probes::ProbePolicy,
) -> Result<ExecutionResult> {
    let model_name = &model.address_segments.join(".");

    // 1. Classify the model SQL.
    let clean_sql = smelt_parser::strip_frontmatter(&model.content).to_string();
    let refs = collect_refs_from_sql(&clean_sql);
    let model_has_timeseries = model
        .metadata
        .as_ref()
        .is_some_and(|m| m.timeseries.is_some());
    let declared_fds: &[smelt_core::config::FunctionalDependency] = model
        .metadata
        .as_ref()
        .map(|m| m.functional_dependencies.as_slice())
        .unwrap_or(&[]);

    let classification = classify_cumulative(
        &clean_sql,
        &refs,
        source_timeseries,
        model_has_timeseries,
        declared_fds,
    )
    .map_err(|diagnostics| format_classifier_error(model_name, &diagnostics))?;

    let driving_source_name = classification.driving_source.name.clone();
    // This function is only reachable via the window-forward run shape —
    // the caller (`execute.rs`'s keyed dispatch) refuses a windowed run for
    // a snapshot-reconcile model before ever reaching here
    // (`docs/specs/incremental_shapes.md` §"The two run shapes"). A `None`
    // here would be an internal invariant violation, not a model error —
    // fail loud rather than silently treating it as anything else.
    let driving_ts = classification
        .driving_source
        .timeseries
        .clone()
        .ok_or_else(|| {
            anyhow::anyhow!(
                "internal error: model '{}' derived the snapshot-reconcile run shape but reached \
             the window-forward executor",
                model_name
            )
        })?;

    info!(
        "Running model: {} (keyed, driving source = {})",
        model_name, driving_source_name
    );

    // 1b. When the model declares its own `timeseries:` block, key temporal
    //     locality (`docs/specs/incremental_shapes.md` §"Key temporal
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
    //    cell — at least one additive-fold aggregator column (`SUM`/`COUNT`
    //    or the self-inverse `BIT_XOR`) — every step's
    //    create-or-merge action is folded through the warehouse-resident
    //    reconciliation ledger (`docs/specs/incremental_models.md` §"The
    //    reconciliation ledger"), transactionally with the write
    //    (`Backend::fold_ledger_delta`); a step whose delta identity (its
    //    own partition value) is already reflected refuses the run instead
    //    of double-counting (`docs/specs/incremental_shapes.md` §"Reprocessing" —
    //    `KeyedReprocessedWindow`). An `Idempotent`-graded cell (no
    //    additive column) needs no ledger — re-merging a window is
    //    harmless — and no warehouse ledger table is ever created for it.
    //    The operator's escape hatch for a genuine reprocess remains
    //    dropping the target table before re-running (full rebuild) or a
    //    manual cascade rebuild.

    // The driving source's declared partition-column type (`docs/specs/
    // incremental_shapes.md` §"The partition grain" rule 8a) — resolved once
    // via the single owner (`crate::execute::sources::
    // source_partition_column_type`, the same derivation `build_model_source_
    // bounds` uses for a plain incremental model's own pushdown), and reused
    // for both the driving-source pushdown filter's `SourceBound` and the
    // per-step `driving_steps` window.
    let driving_column_type = crate::execute::sources::source_partition_column_type(
        source_infos,
        &driving_source_name,
        &driving_ts.partition_column,
    );

    // The model's own maintained partition column's declared/inferred type,
    // resolved through the SAME projection `apply_type_casts` uses
    // (`SqlCompiler::resolve_partition_column_type`) rather than a separate
    // inference — only meaningful (and only resolved) when this model
    // declares its own `timeseries:` block, i.e. when `locality_slice` above
    // is `Some` and a `TargetSlicePredicate::Range` can actually be built.
    let model_column_type = match model.metadata.as_ref().and_then(|m| m.timeseries.as_ref()) {
        Some(own_ts) if locality_slice.is_some() => compiler
            .get(target)
            .resolve_partition_column_type(&clean_sql, &own_ts.partition_column),
        _ => smelt_logical::maintenance::emit::PartitionColumnType::Undeclared,
    };

    // 3. Step over the driving source's partitions in temporal order via the
    //    mode-agnostic windowed-keyed-maintenance driver.
    let steps = driving_steps(
        &time_range.start,
        &time_range.end,
        &driving_ts.granularity,
        driving_column_type,
    )
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
    // Folds the override ladder's write-suppression dimension in
    // (`docs/outcomes/20260815-definition-delta-migrate/phases/33-plan.md`).
    let write_suppression_overrides = model
        .metadata
        .as_deref()
        .map(|m| {
            smelt_db::queries::maintenance::keyed_fold_effective_override(m, &driving_source_name)
        })
        .unwrap_or_default();
    let suppression = resolve_cumulative_write_suppression(
        &classification,
        &clean_sql,
        &write_suppression_overrides,
    )
    .map_err(|refusal| anyhow::anyhow!("{}", refusal))?;

    // The hidden decomposed-state columns every state-bearing aggregator
    // column carries (`docs/specs/incremental_shapes.md` §"Decomposed state
    // (rung 2) in keyed models") — derived once, like `suppression` above.
    // Empty for every column family admitted before this mechanism existed,
    // in which case `state_augmented_projection` below is a no-op.
    let state_columns = classification.state_columns();

    // The `maintenance.cells[].write` pin (if any) addressing this keyed
    // fold's write, resolved once, up front (`docs/outcomes/
    // 20260815-definition-delta-migrate/phases/27g-plan.md`) — a keyed fold's
    // cell is whole-row, so it matches by `on:` address alone
    // (`smelt_db::queries::maintenance::keyed_fold_write_pin`).
    let write_pin = model
        .metadata
        .as_deref()
        .and_then(|m| smelt_db::queries::maintenance::keyed_fold_write_pin(m, &driving_source_name))
        .and_then(|pin_name| smelt_logical::maintenance::lookup_write_pattern(&pin_name));

    run_windowed_keyed_maintenance(
        backend,
        model_name,
        schema,
        db_table_name,
        &steps,
        &classification,
        locality_slice.as_ref(),
        model_column_type,
        &suppression,
        write_pin,
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
                    column_type: driving_column_type,
                },
            );
            let pushed = inject_source_filters(&clean_sql, &bound_map, &step.range);

            // State augmentation happens on this RAW, pre-compile SQL, not
            // the compiled/cast-wrapped output: the state columns' own
            // `per_partition_expr`s (e.g. `ARG_MAX(val, d)`) reference the
            // model's own source columns, which are only in scope at this
            // select level — the compiler's `_smelt_typed` cast wrapper
            // exposes only the model's already-declared presented columns,
            // not the raw columns a state expression needs
            // (`docs/outcomes/20260809-rung2-state-shapes` row 5).
            let pushed = smelt_logical::maintenance::emit::state_augmented_projection(
                &pushed,
                &state_columns,
            )
            .map_err(|_| {
                anyhow::anyhow!(
                    "Failed to append decomposed-state columns to model '{}': its SELECT could \
                     not be parsed",
                    model_name
                )
            })?;

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
        probe_policy,
    )
    .await
}

/// Execute a single keyed model under the snapshot-reconcile run shape
/// (`docs/specs/incremental_shapes.md` §"The two run shapes"): no
/// `[run_start, run_end)` window — the whole source is re-scanned every
/// run. First run (target does not yet exist) creates the table from the
/// compiled SELECT directly; every subsequent run `MERGE`s the whole-source
/// scan into the existing target via [`build_cumulative_merge_sql`], then —
/// unless `contract.retain_departed` is declared
/// (`smelt_logical::contract::retain_departed::reconcile_disposition`) —
/// deletes any key present in the target but absent from the incoming scan
/// ([`emit_departed_key_delete`]), the merge and delete running as one
/// transactional [`StatementGroup`] (`incremental_shapes.md` §"Departed
/// keys and deletion"). No reconciliation ledger: `classification`'s
/// plain-overwrite columns are
/// idempotent by construction (re-running an unchanged snapshot converges),
/// so `Grade::Idempotent` semantics apply without any ledger bookkeeping —
/// this executor never touches one.
///
/// `classification` must have already derived the snapshot-reconcile run
/// shape (`classification.is_snapshot_reconcile()`); the caller
/// (`execute.rs`'s keyed dispatch) is the single admission gate that
/// resolves this before ever reaching here.
#[allow(clippy::too_many_arguments)]
pub async fn execute_snapshot_reconcile(
    backend: &dyn Backend,
    model: &ModelFile,
    compiler: &CompilerRegistry,
    resolver: &EphemeralResolver,
    target: &str,
    schema: &str,
    db_table_name: &str,
    classification: &CumulativeClassification,
    probe_sink: &mut Vec<smelt_state::ProbeRecord>,
) -> Result<ExecutionResult> {
    let model_name = &model.address_segments.join(".");
    let start = std::time::Instant::now();

    let clean_sql = smelt_parser::strip_frontmatter(&model.content).to_string();

    // State augmentation happens on this RAW, pre-compile SQL — see the
    // matching comment in `execute_windowed_keyed` for why (the state
    // expressions need the model's own source columns, only in scope
    // before the compiler's `_smelt_typed` cast wrapper).
    let state_columns = classification.state_columns();
    let augmented_sql =
        smelt_logical::maintenance::emit::state_augmented_projection(&clean_sql, &state_columns)
            .map_err(|_| {
                anyhow::anyhow!(
                    "Failed to append decomposed-state columns to model '{}': its SELECT could \
                     not be parsed",
                    model_name
                )
            })?;

    let compiled = compiler
        .get(target)
        .compile_with_sql_and_ephemerals(model, schema, &augmented_sql, resolver)
        .with_context(|| format!("Failed to compile model: {}", model_name))?;

    let table_exists = backend
        .table_exists(schema, db_table_name)
        .await
        .with_context(|| {
            format!(
                "Failed to check whether table exists: {}.{}",
                schema, db_table_name
            )
        })?;
    if !table_exists {
        backend
            .create_table_as(schema, db_table_name, &compiled.sql)
            .await
            .with_context(|| format!("Failed to create keyed model {}", model_name))?;
    } else {
        let write_suppression_overrides = model
            .metadata
            .as_deref()
            .map(|m| {
                smelt_db::queries::maintenance::keyed_fold_effective_override(
                    m,
                    &classification.driving_source.name,
                )
            })
            .unwrap_or_default();
        let suppression = resolve_cumulative_write_suppression(
            classification,
            &clean_sql,
            &write_suppression_overrides,
        )
        .map_err(|refusal| anyhow::anyhow!("{}", refusal))?;
        let dialect = smelt_backend::maintenance_dialect(backend.dialect())?;
        let merge_sql = build_cumulative_merge_sql(
            schema,
            db_table_name,
            &compiled.sql,
            classification,
            None,
            &suppression,
            dialect,
        );
        let schema_table = format!("{schema}.{db_table_name}");
        let mut statements = vec![MaintenanceStatement { sql: merge_sql }];
        let declared_retain_departed = model
            .metadata
            .as_deref()
            .and_then(|m| m.contract.as_ref())
            .and_then(|c| c.retain_departed.as_ref());
        let transactional = match reconcile_disposition(declared_retain_departed) {
            DepartedKeyDisposition::Delete => {
                statements.push(emit_departed_key_delete(
                    &schema_table,
                    &classification.unique_key,
                    &compiled.sql,
                    dialect,
                ));
                true
            }
            DepartedKeyDisposition::Retain { tombstone } => {
                // The declared point's probe: the reconcile scan's own
                // anti-join, dispatched at the pre-write site instead of
                // running the default point's delete
                // (`smelt_logical::contract::retain_departed::
                // emit_departed_key_probe`). `current_table` is a
                // parenthesised subquery over this run's compiled scan — the
                // probe emitter appends its own `c` alias.
                let key_refs: Vec<&str> = classification
                    .unique_key
                    .iter()
                    .map(String::as_str)
                    .collect();
                let probe = smelt_logical::contract::retain_departed::emit_departed_key_probe(
                    &schema_table,
                    &format!("({})", compiled.sql),
                    &key_refs,
                    tombstone.as_deref(),
                );
                let batches = backend.execute_sql(&probe.sql).await.with_context(|| {
                    format!(
                        "Failed to run the retain_departed probe for keyed model {}",
                        model_name
                    )
                })?;
                let rows = crate::check_runner::batches_to_rows(&batches);
                let retained: u64 = rows
                    .first()
                    .and_then(|r| r.get("retained_departed_count"))
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0);
                tracing::info!(
                    model = %model_name,
                    retained_departed_count = retained,
                    "contract.retain_departed: reconcile anti-join probe"
                );
                // Dispatched on every reconcile that suppresses the
                // default point's delete, independent of the project's
                // `probes:` cadence — this probe stands in for the delete
                // the default point would otherwise have run, so a
                // cadence skip would suppress the delete while verifying
                // nothing (`docs/outcomes/20260815-definition-delta-migrate/
                // phases/34-plan.md`).
                probe_sink.push(smelt_state::ProbeRecord {
                    fact: "contract.retain_departed".to_string(),
                    probe: "ContractDepartedKeyUnmarked".to_string(),
                    outcome: smelt_state::ProbeRecordOutcome::Dispatched,
                    observed: Some(retained),
                });
                if tombstone.is_some() {
                    let unmarked: u64 = rows
                        .first()
                        .and_then(|r| r.get("unmarked_departed_count"))
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(0);
                    anyhow::ensure!(
                        unmarked == 0,
                        "ContractDepartedKeyUnmarked: model '{}' declares \
                         contract.retain_departed with a tombstone column, but {unmarked} \
                         departed key(s) are not marked departed — every row a reconcile no \
                         longer scans from the source must have its tombstone set before it is \
                         exempted from comparison \
                         (`docs/specs/incremental_models.md` §\"Retention (retain_departed)\")",
                        model_name
                    );
                }
                false
            }
        };
        let group = StatementGroup {
            statements,
            transactional,
        };
        backend
            .execute_statement_group(&group)
            .await
            .with_context(|| format!("Failed to reconcile keyed model {}", model_name))?;
    }

    let row_count = backend
        .get_row_count(schema, db_table_name)
        .await
        .unwrap_or(0);
    Ok(ExecutionResult {
        model_name: model_name.clone(),
        duration: start.elapsed(),
        row_count,
        preview: None,
    })
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
    dialect: MaintenanceDialect,
) -> String {
    let folds: Vec<(String, String)> = classification
        .aggregator_columns
        .iter()
        .flat_map(smelt_logical::maintenance::emit::expand_aggregator_column_folds)
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
            dialect,
        ),
        WriteSuppression::Unconditional { .. } => emit_keyed_fold(
            &schema_table,
            &classification.unique_key,
            &folds,
            delta_sql,
            slice,
            dialect,
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
///
/// Folds the override ladder's write-suppression dimension in via
/// [`resolve_write_variant`] (`docs/outcomes/20260815-definition-delta-
/// migrate/phases/33-plan.md`) — `maintenance.cells[].technique: suppress|
/// unconditional`/`prefer:` addressing this keyed fold's driving source was
/// previously silently ignored on this route. Both keyed call sites reach a
/// merge only once the target table already exists (`run_windowed_keyed_
/// maintenance` emits `emit_create_table_as` for a non-existent table; the
/// snapshot-reconcile executor resolves suppression inside its `else` arm
/// of `!table_exists`), so `Trigger::Backfill`/`ledger_catch_up` can never
/// be observed here — `keyed`'s classifier runs outside the
/// `MaintenancePlan` machinery and carries no `PlanCell`. Passing
/// `Trigger::NewData` with `ledger_catch_up: false` is therefore a
/// derivation from the route's own structure, not an assumption: every
/// reachable call is, by construction, a steady-state write over an
/// already-populated table.
fn resolve_cumulative_write_suppression(
    classification: &CumulativeClassification,
    sql: &str,
    overrides: &EffectiveOverride,
) -> Result<WriteSuppression, smelt_logical::maintenance::choice::ChoiceRefusal> {
    let group_columns: Vec<String> = classification
        .aggregator_columns
        .iter()
        .map(|col| col.output_name.clone())
        .collect();
    let identity = row_identity(&classification.unique_key, sql);
    let comparability = model_property_vector(sql, &JoinContext::new())
        .map(|v| v.comparability)
        .unwrap_or_default();
    let suppression = resolve_write_suppression(&group_columns, &comparability, &identity);
    let trigger = Trigger::NewData {
        source: classification.driving_source.name.clone(),
    };
    resolve_write_variant(&suppression, &trigger, false, overrides).map(|(variant, _)| variant)
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
/// SQL (`incremental_shapes.md` §"Key-grain constraints" #4 — "The catalogue is closed and the
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
    declared_functional_dependencies: &[smelt_core::config::FunctionalDependency],
) -> Result<CumulativeClassification> {
    let refs = collect_refs_from_sql(clean_sql);
    classify_cumulative(
        clean_sql,
        &refs,
        source_timeseries,
        model_has_timeseries,
        declared_functional_dependencies,
    )
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
mod tests;
