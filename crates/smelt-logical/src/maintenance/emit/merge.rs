//! `MERGE`-shaped statement families: column-scoped re-derivation, the
//! keyed fold, their change-suppressed variants, in-place field backfill,
//! and the departed-key anti-join `DELETE`.

use super::types::*;

/// Column-scoped re-derivation (bottom-left): the keyed `MERGE` production
/// actually executes for `Technique::ColumnScopedMerge`
/// (`crate::maintenance_driver::execute_column_scoped_merge`/
/// `execute_column_scoped_merge_full` in `smelt-runtime`) —
/// `WHEN MATCHED THEN UPDATE SET *`, `WHEN NOT MATCHED THEN INSERT *` on
/// DuckDB and Spark, which both key-match on `unique_key` and update every
/// column from `source_select`'s projection.
///
/// **BigQuery spells both arms out.** GoogleSQL accepts neither star form —
/// `UPDATE SET *` fails with `Expected "(" but got "*"` and `INSERT *` with
/// `Expected keyword ROW or keyword VALUES but got "*"`, both established
/// against the live warehouse by `scripts/bigquery-probe-merge.sh` rather than
/// read from documentation (`multi_backend.md` §Surface — a capability value
/// comes from the warehouse). So the BigQuery branch renders the matched arm as
/// `SET c = source.c` over `columns` and the not-matched arm as `INSERT ROW`.
/// `columns` must therefore be the target's full output projection — the same
/// full-row set `UPDATE SET *` already writes, so the two forms agree on which
/// columns change. It is **inert on DuckDB and Spark**, whose text stays
/// byte-identical whatever is passed, and an empty `columns` under BigQuery
/// would silently emit a MERGE that updates nothing: callers must refuse that
/// case before reaching here (fail-loud discipline), which is why no
/// column-list-free BigQuery path exists in this emitter.
///
/// **Full-row source-projection contract** (moved from
/// `smelt-backend-duckdb`'s doc comment): `UPDATE SET *` requires
/// `source_select`'s projection to carry every target column — DuckDB errors
/// on a column-count/name mismatch, it does not silently subset by name — so
/// the caller must project the FULL target row, not just the re-derived
/// column group, carrying columns outside that group through unchanged from
/// the existing target state (typically via a join back to the target, or —
/// for the model's own recompute SQL — because the model's SELECT already
/// projects every output column by construction). `SET *` then only changes
/// the group's columns' actual values, satisfying `Technique::
/// ColumnScopedMerge`'s contract without a second, column-list-aware MERGE
/// primitive. This emitter does not (and cannot) verify the projection is
/// complete — a caller that violates the contract fails at the backend, not
/// silently.
///
/// Partition-scoping, when the technique is not the declared full-scan case,
/// is the caller's job — fold it into `source_select` the same way
/// `emit_delete_insert`'s caller folds the output clamp into `body`; the
/// emitter adds no predicate of its own on either the scan or the write
/// target (unlike the old, never-production-matching column-list form this
/// replaces).
pub fn emit_column_scoped_merge(
    table: &str,
    unique_key: &[String],
    source_select: &str,
    columns: &[String],
    dialect: MaintenanceDialect,
) -> StatementGroup {
    let on = unique_key
        .iter()
        .map(|k| format!("target.{k} = source.{k}"))
        .collect::<Vec<_>>()
        .join(" AND ");
    let set = whole_row_update_set(columns, "source", dialect);
    let insert = whole_row_insert_arm(dialect);
    StatementGroup {
        statements: vec![MaintenanceStatement::new(format!(
            "MERGE INTO {table} AS target USING ({source_select}) AS source ON {on} \
             WHEN MATCHED THEN UPDATE SET {set} \
             WHEN NOT MATCHED THEN {insert}"
        ))],
        transactional: false,
    }
}

/// The assignment list of a whole-row `UPDATE SET`, in the grammar `dialect`
/// accepts.
///
/// DuckDB and Spark take the star form and ignore `columns` entirely; BigQuery
/// has no star form and is given `c = <alias>.c` for each column.
/// `source_alias` names the `USING` relation, which differs by emitter
/// (`source` for the column-scoped merge, `delta` for the keyed fold).
fn whole_row_update_set(
    columns: &[String],
    source_alias: &str,
    dialect: MaintenanceDialect,
) -> String {
    match dialect {
        MaintenanceDialect::DuckDb | MaintenanceDialect::Spark => "*".to_string(),
        MaintenanceDialect::BigQuery => columns
            .iter()
            .map(|c| format!("{c} = {source_alias}.{c}"))
            .collect::<Vec<_>>()
            .join(", "),
    }
}

/// The `WHEN NOT MATCHED` arm of a whole-row upsert. Needs no column list in
/// either grammar, so emitters that render their own explicit `UPDATE SET`
/// (the keyed folds) use this alone.
fn whole_row_insert_arm(dialect: MaintenanceDialect) -> &'static str {
    match dialect {
        MaintenanceDialect::DuckDb | MaintenanceDialect::Spark => "INSERT *",
        MaintenanceDialect::BigQuery => "INSERT ROW",
    }
}

/// Change-suppressed column-scoped `MERGE` (T1,
/// `docs/specs/model_transforms.md` §"Change-suppressed MERGE"): identical to
/// [`emit_column_scoped_merge`] except the matched arm is guarded by an
/// `IS DISTINCT FROM` predicate over `compared_columns` — `WHEN MATCHED AND
/// (target.c1 IS DISTINCT FROM source.c1 OR …) THEN UPDATE SET *` — so an
/// unchanged-input re-run's `MERGE` matches every row but writes none of
/// them. This suppresses the WRITE only: the `USING (source_select)` scan is
/// exactly the caller's already-compiled delta, untouched — restricting what
/// is evaluated (as opposed to what is written) is a different licence this
/// emitter does not grant (`docs/plans/20260715-composed-axes-conditional-
/// maintenance.md` Phase C4's "no scope creep" note).
///
/// `compared_columns` must be the caller's already fail-closed-admitted set
/// (`crate::maintenance::choice::resolve_write_suppression`'s
/// `WriteSuppression::Suppressed` — every member proven `Comparable` by the
/// P3 change-comparability walk, over a P2-proven row identity): this
/// emitter does no admission of its own, only string assembly, matching
/// every other function in this module.
///
/// # Panics
/// Panics if `compared_columns` is empty — an unconditionally-refusing
/// caller should build [`emit_column_scoped_merge`] instead of handing this
/// emitter a vacuous compare set (a vacuous `OR` predicate would suppress
/// every write, silently turning a matched row into a permanent no-op).
pub fn emit_column_scoped_merge_suppressed(
    table: &str,
    unique_key: &[String],
    source_select: &str,
    compared_columns: &[String],
    columns: &[String],
    dialect: MaintenanceDialect,
) -> StatementGroup {
    assert!(
        !compared_columns.is_empty(),
        "emit_column_scoped_merge_suppressed requires a non-empty compared-column set for {table}"
    );
    let on = unique_key
        .iter()
        .map(|k| format!("target.{k} = source.{k}"))
        .collect::<Vec<_>>()
        .join(" AND ");
    let suppression = compared_columns
        .iter()
        .map(|c| format!("target.{c} IS DISTINCT FROM source.{c}"))
        .collect::<Vec<_>>()
        .join(" OR ");
    let set = whole_row_update_set(columns, "source", dialect);
    let insert = whole_row_insert_arm(dialect);
    StatementGroup {
        statements: vec![MaintenanceStatement::new(format!(
            "MERGE INTO {table} AS target USING ({source_select}) AS source ON {on} \
             WHEN MATCHED AND ({suppression}) THEN UPDATE SET {set} \
             WHEN NOT MATCHED THEN {insert}"
        ))],
        transactional: false,
    }
}

/// In-place field backfill (top-left with an empty input delta): `UPDATE`
/// the stored region from its own columns; no upstream read at all.
///
/// `region` is `Some((partition_col, region))` for a maintenance run's
/// region-scoped backfill, or `None` for an unregioned, unconditional
/// `UPDATE` — the shape backbuild's B1/D1 self-read backfill needs (every
/// row touched, no maintenance `Region` in scope yet). Backbuild's
/// `backbuild::emit::emit_in_place_update` is a thin `None`-region wrapper
/// over this function, not a second, forked emitter
/// (`docs/plans/20260808-substrate-unification.md`, "emitter unification"
/// — the two shapes are one statement family with an optional predicate,
/// not two).
pub fn emit_in_place_update(
    table: &str,
    assignments: &[(String, String)],
    region: Option<(&str, &Region)>,
) -> Vec<String> {
    vec![render_in_place_update(table, assignments, region)]
}

/// Shared string assembly behind [`emit_in_place_update`], exposed
/// `pub(crate)` so `backbuild::emit::emit_in_place_update` can render the
/// unregioned form directly without allocating (and immediately
/// discarding) a one-element `Vec`.
pub(crate) fn render_in_place_update(
    table: &str,
    assignments: &[(String, String)],
    region: Option<(&str, &Region)>,
) -> String {
    let sets = assignments
        .iter()
        .map(|(c, expr)| format!("{c} = {expr}"))
        .collect::<Vec<_>>()
        .join(", ");
    match region {
        Some((partition_col, region)) => format!(
            "UPDATE {table} SET {sets} WHERE {}",
            region.predicate(None, partition_col)
        ),
        None => format!("UPDATE {table} SET {sets}"),
    }
}

/// A target-scan slice predicate for a locality-admitted keyed fold
/// (`docs/specs/incremental_shapes.md` §"Key temporal locality"): restricts
/// the `MERGE`'s `ON` condition to target rows the write provably cannot
/// touch outside of. The two shapes mirror the two routes
/// [`crate::maintenance::locality::LocalitySlice`] can derive:
///
/// - [`TargetSlicePredicate::Range`] (route 1) — target rows whose
///   partition column lies in `[lower, upper]`. Concrete bounds are
///   computed by the caller (one step's own partition value, widened by
///   the route's derived margins); `lower`/`upper` are plain
///   date/timestamp literal text, unescaped (this emitter escapes them,
///   matching every other emitter in this module).
/// - [`TargetSlicePredicate::DeltaValues`] (route 2) — target rows whose
///   partition column appears among the step's own delta relation's
///   partition-column values, read directly off the same already-compiled
///   delta `SELECT` the `MERGE` scans (`delta_select`) rather than a
///   caller-precomputed range: under route 2 the value is a per-key
///   constant, so the delta's own values are the exact (never widened)
///   slice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetSlicePredicate {
    Range {
        partition_column: String,
        lower: String,
        upper: String,
    },
    DeltaValues {
        partition_column: String,
        delta_select: String,
    },
}

/// Fold-a-delta into keyed end-state (top-left): the combiner-aware `MERGE`
/// production actually executes for `refresh: keyed` (`incremental_models.md`,
/// `crate::cumulative::execute_cumulative_aggregate`'s `WindowedKeyedRule`
/// impl) — combine every aggregator column on matched keys via its own
/// cross-partition combiner, insert unseen keys wholesale.
///
/// `folds` is plain data: `(output_column, rendered_combine_expression)`
/// pairs, e.g. `("event_count", "target.event_count + delta.event_count")`
/// or `("first_seen", "LEAST(target.first_seen, delta.first_seen)")`. The
/// caller renders `CrossPartitionCombiner` (`smelt-planner`) to these
/// expression strings *before* calling this emitter — `smelt-logical` sits
/// below `smelt-planner` in the crate layering
/// (`docs/specs/architecture.md` §"Layered single-ownership") and must
/// never depend on it, so the emitter only assembles plain strings it is
/// handed, never chooses or renders a combiner itself.
///
/// `dialect` selects the not-matched arm only. The matched arm is already an
/// explicit `SET` list (the fold expressions), so unlike
/// [`emit_column_scoped_merge`] this family needs no column list to reach
/// GoogleSQL, which rejects `INSERT *` and takes `INSERT ROW`.
///
/// `slice` is the target-scan slice predicate a locality-admitted model
/// (`incremental_shapes.md` §"Key temporal locality") licenses: an extra
/// `AND target.<partition_column> BETWEEN '<lower>' AND '<upper>'` clause on
/// the `ON` condition, restricting which target rows the `MERGE` needs to
/// scan/match. It is provably safe — a target row outside the slice cannot
/// match any delta key (routes 1-2) or is transactionally checked not to
/// (route 3) — and it never changes *which* delta rows merge (`incremental_
/// models.md`: "Pruning is not a write clamp" — every scanned delta row
/// still merges). `None` renders byte-identical output to the pre-locality
/// shape (a non-time-partitioned keyed model, or a locality-admitted model
/// this phase doesn't yet slice-prune).
pub fn emit_keyed_fold(
    schema_table: &str,
    key: &[String],
    folds: &[(String, String)],
    delta_select: &str,
    slice: Option<&TargetSlicePredicate>,
    dialect: MaintenanceDialect,
) -> StatementGroup {
    let mut on = key
        .iter()
        .map(|k| format!("target.{k} = delta.{k}"))
        .collect::<Vec<_>>()
        .join(" AND ");
    if let Some(slice) = slice {
        match slice {
            TargetSlicePredicate::Range {
                partition_column,
                lower,
                upper,
            } => {
                let safe_lower = lower.replace('\'', "''");
                let safe_upper = upper.replace('\'', "''");
                on.push_str(&format!(
                    " AND target.{partition_column} BETWEEN '{safe_lower}' AND '{safe_upper}'"
                ));
            }
            TargetSlicePredicate::DeltaValues {
                partition_column,
                delta_select,
            } => {
                on.push_str(&format!(
                    " AND target.{partition_column} IN (SELECT DISTINCT {partition_column} FROM \
                     ({delta_select}) AS __locality_delta_values)"
                ));
            }
        }
    }
    let sets = folds
        .iter()
        .map(|(col, expr)| format!("{col} = {expr}"))
        .collect::<Vec<_>>()
        .join(", ");
    StatementGroup {
        statements: vec![MaintenanceStatement::new(format!(
            "MERGE INTO {schema_table} AS target USING ({delta_select}) AS delta ON {on} \
             WHEN MATCHED THEN UPDATE SET {sets} \
             WHEN NOT MATCHED THEN {insert}",
            insert = whole_row_insert_arm(dialect)
        ))],
        transactional: false,
    }
}

/// Change-suppressed keyed fold `MERGE` (T1, `docs/specs/model_transforms.md`
/// §"Change-suppressed MERGE and the staged-candidate conditional
/// DELETE+INSERT"): identical to [`emit_keyed_fold`] except the matched arm
/// is guarded by an `IS DISTINCT FROM` predicate comparing each compared
/// fold column's **current stored value** against **what the fold's own
/// combine expression would write** — `WHEN MATCHED AND (target.c1 IS
/// DISTINCT FROM (<c1's combine expr>) OR …) THEN UPDATE SET …` — so a delta
/// whose combine result reproduces the stored value exactly (an idempotent
/// re-merge of already-reflected rows) writes nothing for that row. This
/// mirrors [`emit_column_scoped_merge_suppressed`]'s suppression shape, but
/// the RHS of each comparison is the column's own fold expression (already
/// written in terms of `target.*`/`delta.*`) rather than a plain
/// `source.<col>` reference, since a keyed fold's matched arm never copies
/// the delta's column verbatim — it combines it with the stored value via a
/// `CrossPartitionCombiner`.
///
/// `compared_columns` must name a subset of `folds`'s own output columns
/// (the caller's already fail-closed-admitted set, `crate::maintenance::
/// choice::resolve_write_suppression`'s `WriteSuppression::Suppressed` —
/// every member proven `Comparable` by the P3 change-comparability walk,
/// over a P2-proven row identity): this emitter does no admission of its
/// own, only string assembly, matching every other function in this module.
///
/// # Panics
/// Panics if `compared_columns` is empty (a vacuous `OR` predicate would
/// suppress every write silently), or if any member of `compared_columns`
/// does not name a column present in `folds` (the caller handed this
/// emitter a compare set that does not match the fold it is suppressing).
pub fn emit_keyed_fold_suppressed(
    schema_table: &str,
    key: &[String],
    folds: &[(String, String)],
    delta_select: &str,
    slice: Option<&TargetSlicePredicate>,
    compared_columns: &[String],
    dialect: MaintenanceDialect,
) -> StatementGroup {
    assert!(
        !compared_columns.is_empty(),
        "emit_keyed_fold_suppressed requires a non-empty compared-column set for {schema_table}"
    );
    let mut on = key
        .iter()
        .map(|k| format!("target.{k} = delta.{k}"))
        .collect::<Vec<_>>()
        .join(" AND ");
    if let Some(slice) = slice {
        match slice {
            TargetSlicePredicate::Range {
                partition_column,
                lower,
                upper,
            } => {
                let safe_lower = lower.replace('\'', "''");
                let safe_upper = upper.replace('\'', "''");
                on.push_str(&format!(
                    " AND target.{partition_column} BETWEEN '{safe_lower}' AND '{safe_upper}'"
                ));
            }
            TargetSlicePredicate::DeltaValues {
                partition_column,
                delta_select,
            } => {
                on.push_str(&format!(
                    " AND target.{partition_column} IN (SELECT DISTINCT {partition_column} FROM \
                     ({delta_select}) AS __locality_delta_values)"
                ));
            }
        }
    }
    let suppression = compared_columns
        .iter()
        .map(|c| {
            let expr = folds
                .iter()
                .find(|(col, _)| col == c)
                .map(|(_, expr)| expr.as_str())
                .unwrap_or_else(|| {
                    panic!(
                        "emit_keyed_fold_suppressed: compared column '{c}' for {schema_table} is \
                         not among the fold's own columns"
                    )
                });
            format!("target.{c} IS DISTINCT FROM ({expr})")
        })
        .collect::<Vec<_>>()
        .join(" OR ");
    let sets = folds
        .iter()
        .map(|(col, expr)| format!("{col} = {expr}"))
        .collect::<Vec<_>>()
        .join(", ");
    StatementGroup {
        statements: vec![MaintenanceStatement::new(format!(
            "MERGE INTO {schema_table} AS target USING ({delta_select}) AS delta ON {on} \
             WHEN MATCHED AND ({suppression}) THEN UPDATE SET {sets} \
             WHEN NOT MATCHED THEN {insert}",
            insert = whole_row_insert_arm(dialect)
        ))],
        transactional: false,
    }
}

/// Dialect-keyed null-safe equality spelling, following the printer's
/// convention (`smelt-dialect/src/printer.rs`'s `null_safe_eq`, not reused
/// directly since this emitter carries only a [`MaintenanceDialect`], not a
/// `BackendCapabilities`): `IS NOT DISTINCT FROM` for DuckDB/BigQuery, `<=>`
/// for Spark.
fn null_safe_eq(lhs: &str, rhs: &str, dialect: MaintenanceDialect) -> String {
    match dialect {
        MaintenanceDialect::DuckDb | MaintenanceDialect::BigQuery => {
            format!("{lhs} IS NOT DISTINCT FROM {rhs}")
        }
        MaintenanceDialect::Spark => format!("{lhs} <=> {rhs}"),
    }
}

/// The default `retain_departed` point's anti-join delete leg
/// (`docs/specs/incremental_shapes.md` §"Departed keys and deletion"): a
/// stored key absent from the incoming scan (`delta_select`, the whole-
/// source snapshot-reconcile scan) is deleted. Null-safe key equality
/// (`null_safe_eq`) so a NULL key component cannot silently exempt a row
/// from deletion the way plain `=` would.
///
/// Caller composes this into the same transactional [`StatementGroup`] as
/// the reconcile `MERGE` (`emit_keyed_fold`/`emit_keyed_fold_suppressed`) —
/// this function returns the `DELETE` alone so the caller controls ordering
/// and transactionality; suppressing this leg entirely (the `retain_
/// departed` point) is the caller's decision
/// (`smelt_logical::contract::retain_departed::reconcile_disposition`), not
/// this emitter's.
pub fn emit_departed_key_delete(
    schema_table: &str,
    key: &[String],
    delta_select: &str,
    dialect: MaintenanceDialect,
) -> MaintenanceStatement {
    let join_predicate = key
        .iter()
        .map(|k| {
            null_safe_eq(
                &format!("{schema_table}.{k}"),
                &format!("delta.{k}"),
                dialect,
            )
        })
        .collect::<Vec<_>>()
        .join(" AND ");
    MaintenanceStatement::new(format!(
        "DELETE FROM {schema_table} WHERE NOT EXISTS (SELECT 1 FROM ({delta_select}) AS delta \
         WHERE {join_predicate})"
    ))
}

#[cfg(test)]
mod column_scoped_merge_tests {
    use super::*;

    fn keys() -> Vec<String> {
        vec!["id".to_string()]
    }

    /// The unconditional variant's emitted text must be byte-unchanged from
    /// before this phase — the regression guard the plan phase's TDD list
    /// names explicitly.
    #[test]
    fn unconditional_variant_text_is_unchanged() {
        let group = emit_column_scoped_merge(
            "warehouse.dim_users",
            &keys(),
            "SELECT * FROM delta",
            &[],
            MaintenanceDialect::DuckDb,
        );
        assert_eq!(group.statements.len(), 1);
        assert_eq!(
            group.statements[0].sql,
            "MERGE INTO warehouse.dim_users AS target USING (SELECT * FROM delta) AS source ON \
             target.id = source.id WHEN MATCHED THEN UPDATE SET * WHEN NOT MATCHED THEN INSERT *"
        );
        assert!(!group.transactional);
    }

    /// The suppressed variant's matched arm carries `IS DISTINCT FROM` over
    /// exactly the compared column set, in order, ORed together.
    #[test]
    fn suppressed_variant_carries_is_distinct_from_over_compared_columns() {
        let compared = vec!["tier".to_string(), "email".to_string()];
        let group = emit_column_scoped_merge_suppressed(
            "warehouse.dim_users",
            &keys(),
            "SELECT * FROM delta",
            &compared,
            &[],
            MaintenanceDialect::DuckDb,
        );
        assert_eq!(group.statements.len(), 1);
        assert_eq!(
            group.statements[0].sql,
            "MERGE INTO warehouse.dim_users AS target USING (SELECT * FROM delta) AS source ON \
             target.id = source.id WHEN MATCHED AND (target.tier IS DISTINCT FROM source.tier OR \
             target.email IS DISTINCT FROM source.email) THEN UPDATE SET * WHEN NOT MATCHED THEN \
             INSERT *"
        );
        assert!(!group.transactional);
    }

    /// A vacuous compare set would suppress every write silently — refuse
    /// via panic rather than emit a matched arm that never fires.
    #[test]
    #[should_panic(expected = "requires a non-empty compared-column set")]
    fn suppressed_variant_panics_on_empty_compare_set() {
        emit_column_scoped_merge_suppressed(
            "warehouse.dim_users",
            &keys(),
            "SELECT * FROM delta",
            &[],
            &[],
            MaintenanceDialect::DuckDb,
        );
    }
}

#[cfg(test)]
mod departed_key_delete_tests {
    use super::*;

    /// The anti-join `DELETE` renders `NOT EXISTS` over the delta select
    /// with null-safe key equality, per dialect, multi-column key included.
    #[test]
    fn emit_departed_key_delete_shape() {
        let key = vec!["tenant_id".to_string(), "device_id".to_string()];
        let stmt = emit_departed_key_delete(
            "main.device_daily",
            &key,
            "SELECT * FROM raw.devices",
            MaintenanceDialect::DuckDb,
        );
        assert!(
            stmt.sql
                .starts_with("DELETE FROM main.device_daily WHERE NOT EXISTS"),
            "{}",
            stmt.sql
        );
        assert!(
            stmt.sql.contains(
                "main.device_daily.tenant_id IS NOT DISTINCT FROM delta.tenant_id AND \
                 main.device_daily.device_id IS NOT DISTINCT FROM delta.device_id"
            ),
            "{}",
            stmt.sql
        );
        assert!(
            stmt.sql
                .contains("FROM (SELECT * FROM raw.devices) AS delta"),
            "{}",
            stmt.sql
        );

        let spark_stmt = emit_departed_key_delete(
            "main.device_daily",
            &["device_id".to_string()],
            "SELECT * FROM raw.devices",
            MaintenanceDialect::Spark,
        );
        assert!(
            spark_stmt
                .sql
                .contains("main.device_daily.device_id <=> delta.device_id"),
            "{}",
            spark_stmt.sql
        );
    }
}

#[cfg(test)]
mod keyed_fold_suppressed_tests {
    use super::*;

    fn key() -> Vec<String> {
        vec!["device_id".to_string()]
    }

    fn folds() -> Vec<(String, String)> {
        vec![
            (
                "event_count".to_string(),
                "target.event_count + delta.event_count".to_string(),
            ),
            (
                "last_seen".to_string(),
                "GREATEST(target.last_seen, delta.last_seen)".to_string(),
            ),
        ]
    }

    /// The suppressed variant's matched arm carries `IS DISTINCT FROM` over
    /// exactly the compared fold columns, comparing the stored value against
    /// the fold's own combine expression (not a plain `delta.<col>`).
    #[test]
    fn suppressed_variant_carries_is_distinct_from_over_compared_fold_columns() {
        let group = emit_keyed_fold_suppressed(
            "main.device_daily",
            &key(),
            &folds(),
            "SELECT * FROM delta",
            None,
            &["event_count".to_string()],
            MaintenanceDialect::DuckDb,
        );
        assert_eq!(group.statements.len(), 1);
        assert_eq!(
            group.statements[0].sql,
            "MERGE INTO main.device_daily AS target USING (SELECT * FROM delta) AS delta ON \
             target.device_id = delta.device_id WHEN MATCHED AND (target.event_count IS DISTINCT \
             FROM (target.event_count + delta.event_count)) THEN UPDATE SET event_count = \
             target.event_count + delta.event_count, last_seen = GREATEST(target.last_seen, \
             delta.last_seen) WHEN NOT MATCHED THEN INSERT *"
        );
        assert!(!group.transactional);
    }

    /// Composes with a locality slice predicate on the `ON` condition,
    /// exactly like the unconditional `emit_keyed_fold`.
    #[test]
    fn suppressed_variant_composes_with_slice_predicate() {
        let slice = TargetSlicePredicate::Range {
            partition_column: "event_date".to_string(),
            lower: "2026-01-01".to_string(),
            upper: "2026-01-02".to_string(),
        };
        let group = emit_keyed_fold_suppressed(
            "main.device_daily",
            &key(),
            &folds(),
            "SELECT * FROM delta",
            Some(&slice),
            &["event_count".to_string(), "last_seen".to_string()],
            MaintenanceDialect::DuckDb,
        );
        let sql = &group.statements[0].sql;
        assert!(sql.contains(
            "target.device_id = delta.device_id AND target.event_date BETWEEN '2026-01-01' AND \
             '2026-01-02'"
        ));
        assert!(sql.contains(
            "target.event_count IS DISTINCT FROM (target.event_count + delta.event_count)"
        ));
        assert!(sql.contains(
            "target.last_seen IS DISTINCT FROM (GREATEST(target.last_seen, delta.last_seen))"
        ));
    }

    #[test]
    #[should_panic(expected = "requires a non-empty compared-column set")]
    fn suppressed_variant_panics_on_empty_compare_set() {
        emit_keyed_fold_suppressed(
            "main.device_daily",
            &key(),
            &folds(),
            "SELECT * FROM delta",
            None,
            &[],
            MaintenanceDialect::DuckDb,
        );
    }

    #[test]
    #[should_panic(expected = "is not among the fold's own columns")]
    fn suppressed_variant_panics_on_unknown_compared_column() {
        emit_keyed_fold_suppressed(
            "main.device_daily",
            &key(),
            &folds(),
            "SELECT * FROM delta",
            None,
            &["not_a_fold_column".to_string()],
            MaintenanceDialect::DuckDb,
        );
    }
}

#[cfg(test)]
mod composed_slice_bounded_suppression_tests {
    //! `docs/plans/20260715-composed-axes-conditional-maintenance.md` Phase
    //! C6: composing suppression (C4/C5) with locality (A2) — a composed
    //! (key + time) output's suppressed `MERGE` carries **both** predicates
    //! (the slice on the target read, `IS DISTINCT FROM` on the matched
    //! arm); a bare keyed model with no established locality slice carries
    //! only the suppression arm, never an invented slice. This module makes
    //! that composition explicit at the emitter level (mirroring the
    //! `events_deduped`-shaped fixture: `event_id` key, `first_seen_date`
    //! slice) — `keyed_fold_suppressed_tests::
    //! suppressed_variant_composes_with_slice_predicate`/`suppressed_
    //! variant_carries_is_distinct_from_over_compared_fold_columns` (C5)
    //! already exercise the same emitter branches; these tests are this
    //! phase's own explicit, named proof of the same claim.

    use super::*;

    fn key() -> Vec<String> {
        vec!["event_id".to_string()]
    }

    fn folds() -> Vec<(String, String)> {
        vec![(
            "device_id".to_string(),
            "MIN(target.device_id, delta.device_id)".to_string(),
        )]
    }

    #[test]
    fn composed_model_suppressed_merge_carries_both_predicates() {
        let slice = TargetSlicePredicate::Range {
            partition_column: "first_seen_date".to_string(),
            lower: "2026-04-01".to_string(),
            upper: "2026-04-01".to_string(),
        };
        let group = emit_keyed_fold_suppressed(
            "main.events_deduped",
            &key(),
            &folds(),
            "SELECT * FROM delta",
            Some(&slice),
            &["device_id".to_string()],
            MaintenanceDialect::DuckDb,
        );
        let sql = &group.statements[0].sql;
        assert!(
            sql.contains("target.first_seen_date BETWEEN '2026-04-01' AND '2026-04-01'"),
            "composed model's suppressed merge must carry the slice predicate: {sql}"
        );
        assert!(
            sql.contains("IS DISTINCT FROM"),
            "composed model's suppressed merge must ALSO carry the suppression arm: {sql}"
        );
    }

    #[test]
    fn bare_keyed_model_suppressed_merge_carries_only_the_suppression_arm() {
        let group = emit_keyed_fold_suppressed(
            "main.events_deduped",
            &key(),
            &folds(),
            "SELECT * FROM delta",
            None,
            &["device_id".to_string()],
            MaintenanceDialect::DuckDb,
        );
        let sql = &group.statements[0].sql;
        assert!(
            sql.contains("IS DISTINCT FROM"),
            "bare keyed model's suppressed merge must carry the suppression arm: {sql}"
        );
        assert!(
            !sql.contains("BETWEEN") && !sql.contains(" IN ("),
            "bare keyed model's suppressed merge must never invent a slice: {sql}"
        );
    }
}
