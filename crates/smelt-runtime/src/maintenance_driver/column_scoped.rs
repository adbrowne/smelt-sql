use anyhow::Result;
use smelt_backend::{maintenance_dialect, Backend, BackendError, ExecutionResult, PartitionRange};
use smelt_dialect::SqlDialect;
use smelt_logical::analysis::join_shape::{ContributionVerdict, JoinContext};
use smelt_logical::analysis::source_bounds::BoundResult;
use smelt_logical::analysis::walk::model_property_vector;
use smelt_logical::maintenance::choice::WriteSuppression;
use smelt_logical::maintenance::emit::{
    emit_column_scoped_merge, emit_column_scoped_merge_suppressed, MaintenanceDialect,
};
use smelt_logical::maintenance::{PartitionLocal, PlanCell, ScanClamp};
use smelt_state::ddl_duckdb;
use std::time::Instant;

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
    let predicate = changed_row_predicate("target", "source", compared_columns);
    changed_keys_select_over_predicate(
        table,
        unique_key,
        source_select,
        "source",
        &predicate,
        partition_column,
    )
}

/// The shared "new-or-changed keys, projected to their touched partition"
/// query shape both [`changed_keys_select`] (column-scoped MERGE, raw
/// column comparison) and [`keyed_fold_changed_keys_select`] (keyed fold,
/// fold-expression comparison) build — the two differ only in the
/// candidate-relation alias and the predicate text, never in the join/
/// key-projection shape itself.
fn changed_keys_select_over_predicate(
    table: &str,
    unique_key: &[String],
    candidate_select: &str,
    candidate_alias: &str,
    predicate: &str,
    partition_column: Option<&str>,
) -> String {
    let on = unique_key
        .iter()
        .map(|k| format!("target.{k} = {candidate_alias}.{k}"))
        .collect::<Vec<_>>()
        .join(" AND ");
    let key_expr = if unique_key.len() == 1 {
        format!("CAST({candidate_alias}.{} AS VARCHAR)", unique_key[0])
    } else {
        let parts = unique_key
            .iter()
            .map(|k| format!("CAST({candidate_alias}.{k} AS VARCHAR)"))
            .collect::<Vec<_>>()
            .join(", '\u{1}', ");
        format!("CONCAT({parts})")
    };
    let partition_expr = match partition_column {
        Some(col) => format!("CAST({candidate_alias}.{col} AS VARCHAR)"),
        None => "NULL".to_string(),
    };
    let first_key = &unique_key[0];
    format!(
        "SELECT {key_expr} AS delta_key, {partition_expr} AS delta_partition FROM \
         ({candidate_select}) AS {candidate_alias} LEFT JOIN {table} AS target ON {on} \
         WHERE target.{first_key} IS NULL OR ({predicate})"
    )
}

/// Build the `target.c IS DISTINCT FROM (<fold_expr>)` OR-predicate over
/// `compared_columns` — the SAME shape [`smelt_logical::maintenance::emit::
/// emit_keyed_fold_suppressed`] guards its matched arm with (one
/// comparison, two consumers, `docs/specs/incremental_models.md` §"The
/// graph layer" — "Observed deltas on model edges"; kept from drifting off
/// the write's own guard by
/// `crates/smelt-runtime/tests/statement_parity.rs`). Unlike
/// [`changed_row_predicate`]'s raw-column comparison (the column-scoped
/// MERGE's matched arm compares source vs. target directly), a keyed fold's
/// matched arm compares the target's stored value against the FOLDED
/// (combiner-applied) delta value — `folds` supplies each compared column's
/// already-rendered fold expression (`target.c op delta.c`, as
/// `smelt_logical::maintenance::emit::expand_aggregator_column_folds`
/// renders it).
pub fn keyed_fold_changed_row_predicate(
    compared_columns: &[String],
    folds: &[(String, String)],
) -> String {
    compared_columns
        .iter()
        .map(|c| {
            let expr = folds
                .iter()
                .find(|(col, _)| col == c)
                .map(|(_, expr)| expr.as_str())
                .unwrap_or_else(|| {
                    panic!(
                        "keyed_fold_changed_row_predicate: compared column '{c}' is not among \
                         the fold's own columns"
                    )
                });
            format!("target.{c} IS DISTINCT FROM ({expr})")
        })
        .collect::<Vec<_>>()
        .join(" OR ")
}

/// The changed-key `SELECT` for a suppressed keyed fold's observed delta:
/// every delta row that is new (no matching target key) or whose applied
/// fold changes at least one comparable column — the same rowset
/// [`smelt_logical::maintenance::emit::emit_keyed_fold_suppressed`]'s
/// matched arm actually updates, restricted to comparable columns
/// ([`keyed_fold_changed_row_predicate`]). `delta_select` is aliased
/// `delta` (not `source`), matching the fold expressions' own
/// `target.c`/`delta.c` qualification.
pub fn keyed_fold_changed_keys_select(
    table: &str,
    unique_key: &[String],
    delta_select: &str,
    compared_columns: &[String],
    folds: &[(String, String)],
    partition_column: Option<&str>,
) -> String {
    let predicate = keyed_fold_changed_row_predicate(compared_columns, folds);
    changed_keys_select_over_predicate(
        table,
        unique_key,
        delta_select,
        "delta",
        &predicate,
        partition_column,
    )
}

/// The changed-key `SELECT` for a staged-candidate conditional recompute's
/// observed delta (T5): every key whose applied effect was NOT the
/// identity — new (in the candidate, not the target), changed (in both,
/// but at least one comparable column differs — the same `IS DISTINCT
/// FROM` guard [`smelt_logical::maintenance::emit::
/// emit_staged_candidate_conditional_recompute`]'s `delete_changed`
/// statement uses), or departed (in the target, not the candidate — the
/// same anti-join its `delete_departed` statement uses). A key present in
/// both with no comparable-column difference (untouched) never appears —
/// its applied effect IS the identity. `partition_column`, when `Some`,
/// projects the candidate side's own partition column for a new/changed
/// key; a departed key (no candidate row to read a partition value from)
/// always reports `NULL` (folded to an empty `partitions` array by the
/// upsert), matching the write's own inability to name a partition for a
/// row it no longer has any relation over.
pub(super) fn staged_candidate_changed_keys_select(
    table: &str,
    key: &[String],
    candidate_select: &str,
    compared_columns: &[String],
    partition_column: Option<&str>,
) -> String {
    let predicate = changed_row_predicate("target", "candidate", compared_columns);
    let new_or_changed = changed_keys_select_over_predicate(
        table,
        key,
        candidate_select,
        "candidate",
        &predicate,
        partition_column,
    );
    let departed_on = key
        .iter()
        .map(|k| format!("target.{k} = candidate.{k}"))
        .collect::<Vec<_>>()
        .join(" AND ");
    let key_expr = if key.len() == 1 {
        format!("CAST(target.{} AS VARCHAR)", key[0])
    } else {
        let parts = key
            .iter()
            .map(|k| format!("CAST(target.{k} AS VARCHAR)"))
            .collect::<Vec<_>>()
            .join(", '\u{1}', ");
        format!("CONCAT({parts})")
    };
    let departed = format!(
        "SELECT {key_expr} AS delta_key, NULL AS delta_partition FROM {table} AS target WHERE \
         NOT EXISTS (SELECT 1 FROM ({candidate_select}) AS candidate WHERE {departed_on})"
    );
    format!("{new_or_changed} UNION ALL {departed}")
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
/// ([`smelt_logical::analysis::join_shape::join_alias_for_source`] — a
/// leaf-level parse of exactly the join clause this
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
    let Some(alias) =
        smelt_logical::analysis::join_shape::join_alias_for_source(sql, dimension_source)
    else {
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
