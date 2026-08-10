//! Physical maintenance SQL emission — the single author of every
//! maintenance statement a run executes
//! (`docs/specs/incremental_models.md` §"Statement emission (single owner)").
//!
//! One emitter per [`Technique`](super::Technique), following the
//! physical-maintenance notation of
//! `docs/research/20260705-refresh-as-maintenance-plan/07-example-catalogue.md`:
//! the partition predicate is carried on **both** the scan and the write
//! target wherever the op is region-scoped — a predicate stated only on one
//! side is a logical bound the storage layer cannot use
//! (`01-framework.md` §5).
//!
//! Emission is pure string construction over a caller-supplied SELECT body
//! (the model SQL with source refs resolved to physical table names); clamp
//! *injection into* the body is the runtime transformer's job
//! (`smelt-runtime/src/transformer.rs`) and is deliberately not duplicated
//! here — an emitter never adds a predicate the caller did not already fold
//! into the body it hands in, so the emitted text is exactly what a backend
//! executes, byte for byte.
//!
//! Backends *execute* the [`StatementGroup`]s these functions return; they
//! never author maintenance-statement text of their own
//! (`docs/specs/architecture.md` §"Constraints & Invariants" item 12).

use super::diff_patch::DeleteLeg;
use super::ScanClamp;

/// One SQL statement a maintenance run executes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaintenanceStatement {
    pub sql: String,
}

impl MaintenanceStatement {
    pub(crate) fn new(sql: String) -> Self {
        Self { sql }
    }
}

/// An ordered group of [`MaintenanceStatement`]s produced by one emitter
/// call, plus whether they must run inside a single backend transaction. A
/// paired region `DELETE`+`INSERT` is transactional: a failed `INSERT` must
/// roll back its `DELETE` (`docs/specs/incremental_models.md` §"Statement
/// emission (single owner)").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatementGroup {
    pub statements: Vec<MaintenanceStatement>,
    pub transactional: bool,
}

/// The backend SQL dialect a [`StatementGroup`] is rendered for. Dialect
/// differences (e.g. a `MERGE … UPDATE SET *` requiring a full-row source
/// projection versus an explicit column-list `SET`) live in the emitters as
/// dialect-keyed variants, not in backend string construction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaintenanceDialect {
    DuckDb,
    Spark,
}

/// A half-open region `[start, end)` on the output partition column; values
/// are SQL literals (already quoted where needed).
#[derive(Debug, Clone)]
pub struct Region {
    pub start: String,
    pub end: String,
}

/// The widened scan predicate a derived [`ScanClamp`] implies for
/// maintaining output region `[start, end)`: the source's partition column
/// over `[start − before, end + after)`. This is the *derived* number turned
/// into SQL — the caller injects it into the read (the body's source scan),
/// so a wrongly-derived window fails the equivalence oracle rather than
/// silently over- or under-reading.
pub fn widened_scan_predicate(clamp: &ScanClamp, region: &Region) -> String {
    let lower = if clamp.before.0 == 0 {
        region.start.clone()
    } else {
        format!("{} - INTERVAL '{} seconds'", region.start, clamp.before.0)
    };
    let upper = if clamp.after.0 == 0 {
        region.end.clone()
    } else {
        format!("{} + INTERVAL '{} seconds'", region.end, clamp.after.0)
    };
    format!("{col} >= {lower} AND {col} < {upper}", col = clamp.column)
}

impl Region {
    pub fn predicate(&self, qualifier: Option<&str>, column: &str) -> String {
        let col = match qualifier {
            Some(q) => format!("{q}.{column}"),
            None => column.to_string(),
        };
        format!(
            "{col} >= {start} AND {col} < {end}",
            start = self.start,
            end = self.end
        )
    }
}

/// Recompute-a-region (bottom-right): `DELETE` exactly the write window,
/// `INSERT` its recompute, as one transactional [`StatementGroup`]. The
/// DELETE's range must equal exactly what the INSERT writes
/// (`docs/specs/model_transforms.md` §"Write window = output window"); the
/// INSERT does **not** re-add the predicate — `body` is the caller's
/// already-clamped compiled SELECT (the output clamp is injected upstream,
/// `smelt-runtime/src/transformer.rs`), so re-wrapping it here would be a
/// second, redundant filter the emitter has no business adding.
///
/// `dialect` is accepted for signature symmetry with the other emitters in
/// this module; the DELETE/INSERT shape is currently dialect-invariant
/// (DuckDB and Spark share the same `DELETE FROM … WHERE …` / `INSERT INTO
/// … <select>` grammar for this family).
pub fn emit_delete_insert(
    table: &str,
    partition_col: &str,
    region: &Region,
    body: &str,
    _dialect: MaintenanceDialect,
) -> StatementGroup {
    let pred = region.predicate(None, partition_col);
    StatementGroup {
        statements: vec![
            MaintenanceStatement::new(format!("DELETE FROM {table} WHERE {pred}")),
            MaintenanceStatement::new(format!("INSERT INTO {table} {body}")),
        ],
        transactional: true,
    }
}

/// Delta-restricted region recompute (T3, `docs/specs/model_transforms.md`
/// §"Delta-restricted enrichment join"): identical to [`emit_delete_insert`]
/// except both the `DELETE` and the `INSERT` gain an extra `restrict_column
/// IN (...)` semi-join predicate over `delta_keys` — the exact upstream
/// observed delta on this cell's driving model edge
/// (`crate::maintenance::choice::resolve_recompute_restriction`, licensed
/// only when P1 skeleton-source closure is `Closed` for the cell's
/// enrichment join(s) *and* a non-empty delta was recorded).
///
/// This restricts recompute **breadth** only: `body` is exactly the same
/// already-clamped compiled SELECT [`emit_delete_insert`] would have
/// received — the caller's read of upstream state (`S`) is untouched; the
/// semi-join filters candidate *output* rows after `body` evaluates them,
/// never the source scan itself (`docs/plans/20260715-composed-axes-
/// conditional-maintenance.md` Phase E3's "no scope creep" note, mirroring
/// C4's for write suppression). Wrapping `body` in an outer `SELECT … WHERE`
/// (rather than injecting the predicate into `body`'s own text) keeps this
/// emitter agnostic to `body`'s internal shape, the same way
/// [`emit_column_scoped_merge`]'s `USING (source_select)` wrapping does.
///
/// A caller with `RecomputeRestriction::Unrestricted` (`Open` closure, an
/// absent delta, or a present-but-empty one) must call
/// [`emit_delete_insert`] directly instead of this function with a vacuous
/// key set — the two emitted statement groups are then byte-identical to
/// today's unrestricted form, never a partially-restricted one.
///
/// # Panics
/// Panics if `delta_keys` is empty — an empty delta means nothing changed
/// upstream and should never reach this emitter (the caller's fail-closed
/// admission, `resolve_recompute_restriction`, treats an empty delta as
/// `Unrestricted`, matching the change-suppressed emitters' non-empty-set
/// contract).
pub fn emit_delete_insert_delta_restricted(
    table: &str,
    partition_col: &str,
    region: &Region,
    body: &str,
    restrict_column: &str,
    delta_keys: &[String],
    _dialect: MaintenanceDialect,
) -> StatementGroup {
    assert!(
        !delta_keys.is_empty(),
        "emit_delete_insert_delta_restricted requires a non-empty delta key set for {table} — \
         an empty delta must fall back to emit_delete_insert, never a vacuous restriction"
    );
    let pred = region.predicate(None, partition_col);
    let key_list = delta_keys
        .iter()
        .map(|k| format!("'{}'", k.replace('\'', "''")))
        .collect::<Vec<_>>()
        .join(", ");
    StatementGroup {
        statements: vec![
            MaintenanceStatement::new(format!(
                "DELETE FROM {table} WHERE {pred} AND {restrict_column} IN ({key_list})"
            )),
            MaintenanceStatement::new(format!(
                "INSERT INTO {table} SELECT * FROM ({body}) AS _smelt_delta_scope WHERE \
                 _smelt_delta_scope.{restrict_column} IN ({key_list})"
            )),
        ],
        transactional: true,
    }
}

/// Column-scoped re-derivation (bottom-left): the keyed `MERGE` production
/// actually executes for `Technique::ColumnScopedMerge`
/// (`crate::maintenance_driver::execute_column_scoped_merge`/
/// `execute_column_scoped_merge_full` in `smelt-runtime`) —
/// `WHEN MATCHED THEN UPDATE SET *`, `WHEN NOT MATCHED THEN INSERT *`. There
/// is no column-list `SET` variant: DuckDB and Spark both key-match on
/// `unique_key` and update every column from `source_select`'s projection
/// (dialect-invariant text for this family — no branch reads `dialect` yet,
/// kept for signature symmetry with the other emitters in this module).
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
    _dialect: MaintenanceDialect,
) -> StatementGroup {
    let on = unique_key
        .iter()
        .map(|k| format!("target.{k} = source.{k}"))
        .collect::<Vec<_>>()
        .join(" AND ");
    StatementGroup {
        statements: vec![MaintenanceStatement::new(format!(
            "MERGE INTO {table} AS target USING ({source_select}) AS source ON {on} \
             WHEN MATCHED THEN UPDATE SET * \
             WHEN NOT MATCHED THEN INSERT *"
        ))],
        transactional: false,
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
    _dialect: MaintenanceDialect,
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
    StatementGroup {
        statements: vec![MaintenanceStatement::new(format!(
            "MERGE INTO {table} AS target USING ({source_select}) AS source ON {on} \
             WHEN MATCHED AND ({suppression}) THEN UPDATE SET * \
             WHEN NOT MATCHED THEN INSERT *"
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
/// (`docs/specs/incremental_models.md` §"Key temporal locality"): restricts
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
/// `dialect` is accepted for signature symmetry with the other emitters in
/// this module; the keyed-fold `MERGE` shape is currently dialect-invariant
/// (no branch reads it yet).
///
/// `slice` is the target-scan slice predicate a locality-admitted model
/// (`incremental_models.md` §"Key temporal locality") licenses: an extra
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
    _dialect: MaintenanceDialect,
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
             WHEN NOT MATCHED THEN INSERT *"
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
    _dialect: MaintenanceDialect,
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
             WHEN NOT MATCHED THEN INSERT *"
        ))],
        transactional: false,
    }
}

/// The staged-candidate conditional `DELETE`+`INSERT` (T2, `docs/specs/
/// model_transforms.md` §"Change-suppressed MERGE and the staged-candidate
/// conditional DELETE+INSERT"): the merge-less realisation of the same
/// no-op-write-elimination licence, for a backend that cannot run `MERGE` at
/// all (a documented gap: Spark-over-Parquet). One transaction:
///
/// 1. `CREATE TEMP TABLE <staged_relation> AS <candidate_select> LIMIT 0` —
///    stage an empty relation shaped like the candidate rows.
/// 2. `INSERT INTO <staged_relation> <candidate_select>` — populate it with
///    this run's computed candidate rows (the caller's already-compiled
///    delta/re-derivation SELECT, full-row projection, same contract as
///    [`emit_column_scoped_merge`]'s `source_select`).
/// 3. `DELETE FROM <table> USING <staged_relation> WHERE <key join> AND
///    (<IS DISTINCT FROM over compared_columns>)` — remove exactly the
///    stored rows whose staged candidate differs from what is stored (never
///    a row whose applied effect is the identity).
/// 4. `INSERT INTO <table> SELECT s.* FROM <staged_relation> AS s WHERE NOT
///    EXISTS (target row still present for this key)` — reinsert the rows
///    just deleted, plus any brand-new key the target never had. A row
///    whose staged candidate matched the stored state was never deleted in
///    step 3, so it is correctly skipped here too — it still "exists" in
///    the target under its own key.
/// 5. `DROP TABLE <staged_relation>` — cleanup.
///
/// Every statement is transactional as one unit
/// (`StatementGroup::transactional`): the group is only ever handed to
/// [`crate::maintenance`]'s consumers via `Backend::execute_statement_group`,
/// whose real (DuckDB) implementation runs it inside one native transaction,
/// so a failure at any step (including a candidate-select error) rolls back
/// every earlier statement — the temp relation's own `CREATE` included —
/// leaving both the target table and the temp-relation namespace exactly as
/// they were before the group started.
///
/// This phase's realisation is the **keyed-shaped** path only — `key` is a
/// declared/proven row identity (`RowIdentity::Key`, never `WholeRow`); the
/// whole-row `EXCEPT ALL`-both-ways realisation for a keyless region remains
/// unbuilt (`docs/specs/model_transforms.md` §Known Divergences).
///
/// # Panics
/// Panics if `key` or `compared_columns` is empty — an identity-free or
/// vacuous-compare call has no sound conditional shape to emit (the caller
/// must fail closed to the unconditional region `DELETE`+`INSERT`
/// ([`emit_delete_insert`]) instead of reaching this emitter).
pub fn emit_staged_candidate_conditional(
    table: &str,
    staged_relation: &str,
    key: &[String],
    candidate_select: &str,
    compared_columns: &[String],
    _dialect: MaintenanceDialect,
) -> StatementGroup {
    assert!(
        !key.is_empty(),
        "emit_staged_candidate_conditional requires a non-empty row identity (key) for {table}"
    );
    assert!(
        !compared_columns.is_empty(),
        "emit_staged_candidate_conditional requires a non-empty compared-column set for {table}"
    );
    let key_join_table_staged = key
        .iter()
        .map(|k| format!("{table}.{k} = {staged_relation}.{k}"))
        .collect::<Vec<_>>()
        .join(" AND ");
    let key_join_t_s = key
        .iter()
        .map(|k| format!("t.{k} = s.{k}"))
        .collect::<Vec<_>>()
        .join(" AND ");
    let suppression = compared_columns
        .iter()
        .map(|c| format!("{table}.{c} IS DISTINCT FROM {staged_relation}.{c}"))
        .collect::<Vec<_>>()
        .join(" OR ");
    let create = format!(
        "CREATE TEMP TABLE {staged_relation} AS SELECT * FROM ({candidate_select}) AS \
         __smelt_staged_shape LIMIT 0"
    );
    let insert_candidates = format!("INSERT INTO {staged_relation} {candidate_select}");
    let delete = format!(
        "DELETE FROM {table} USING {staged_relation} WHERE {key_join_table_staged} AND \
         ({suppression})"
    );
    let insert = format!(
        "INSERT INTO {table} SELECT s.* FROM {staged_relation} AS s WHERE NOT EXISTS (SELECT 1 \
         FROM {table} AS t WHERE {key_join_t_s})"
    );
    let drop = format!("DROP TABLE {staged_relation}");
    StatementGroup {
        statements: vec![
            MaintenanceStatement::new(create),
            MaintenanceStatement::new(insert_candidates),
            MaintenanceStatement::new(delete),
            MaintenanceStatement::new(insert),
            MaintenanceStatement::new(drop),
        ],
        transactional: true,
    }
}

/// The staged-candidate conditional `DELETE`+`INSERT`, **full-recompute**
/// variant (`docs/plans/20260808-membership-sensitivity.md` Phase 3): the
/// membership-sensitive counterpart of [`emit_staged_candidate_conditional`]
/// above, for the single production caller whose `candidate_select` is
/// always the model's own FULL (unwindowed) recompute — never a
/// region/window-scoped delta
/// (`smelt-runtime`'s `execute_staged_membership_recompute`). Because the
/// candidate is genuinely the model's entire current state, a stored row
/// whose key is absent from it is not "out of this run's touched region" —
/// it has genuinely **departed** (e.g. the dimension row a fact joined on
/// was itself deleted, so the fact no longer appears in the recompute at
/// all), and must be deleted.
///
/// This is a distinct emitter, not a modified [`emit_staged_candidate_conditional`],
/// because that function's own "absence = out of this run's touched region,
/// leave untouched" contract is *correct* and load-bearing for its own
/// region/window-scoped callers (`crates/smelt-runtime/tests/
/// statement_parity.rs::staged_candidate_conditional_statements_come_from_the_emitter`'s
/// "user 3 … must be left untouched entirely"; `crates/smelt-cli/tests/
/// maintenance_conformance/gate.rs::keyed_pool_t1_t2_and_full_refresh_agree_at_fixed_s`'s
/// "device 3 (absent from the delta) must never be touched") — conflating
/// the two absence semantics into one function would silently break one
/// caller to fix the other. Single-owner discipline
/// (`docs/specs/incremental_models.md` §"Statement emission (single owner)")
/// is preserved by keeping both variants declared side by side in this
/// module, sharing every predicate-building helper's *shape*.
///
/// Adds one more transactional step to [`emit_staged_candidate_conditional`]'s
/// five:
///
/// 1. `CREATE TEMP TABLE <staged_relation> AS <candidate_select> LIMIT 0`
/// 2. `INSERT INTO <staged_relation> <candidate_select>`
/// 3. `DELETE FROM <table> USING <staged_relation> WHERE <key join> AND
///    (<IS DISTINCT FROM over compared_columns>)` — matched-and-changed rows.
/// 4. **`DELETE FROM <table> WHERE NOT EXISTS (SELECT 1 FROM
///    <staged_relation> WHERE <key join>)`** — departed rows: every stored
///    row whose key does not appear in the staged candidate at all. A
///    no-op (deletes zero rows) whenever every stored key is still present
///    in the candidate — the change-suppression contract for step 3's
///    matched-unchanged rows is untouched by this step, since it never
///    matches a still-present key.
/// 5. `INSERT INTO <table> SELECT s.* FROM <staged_relation> AS s WHERE NOT
///    EXISTS (target row still present for this key)` — reinserts rows
///    deleted by step 3, plus brand-new keys. A row deleted by step 4
///    (departed) is never reinserted here: its key is absent from
///    `<staged_relation>` by construction, so it is not among `s.*`.
/// 6. `DROP TABLE <staged_relation>`
///
/// The whole group runs in one transaction, same as
/// [`emit_staged_candidate_conditional`].
///
/// # NULL-keyed rows (known caveat, not yet closed)
/// Every join this emitter builds (steps 3-5) compares keys with plain
/// `=`, never a NULL-safe join — mirroring [`emit_staged_candidate_conditional`]'s
/// own predicate-building style. SQL's `NULL = NULL` is never true, so a
/// stored row whose key is (or contains) `NULL` never matches ANY staged
/// candidate row on ANY of these joins, on ANY run — not even one where its
/// own key is still genuinely present, unchanged, in the candidate. The
/// practical effect: step 4's departed-key `DELETE` deletes it, and step 5's
/// reinsert immediately reinserts it (its key is genuinely absent from the
/// live `<table>` at that point, by step 4's own action) — every single
/// run, forever. End-state equivalence with a full-refresh oracle still
/// holds (the reinserted row's values are correct), but the change-
/// suppression contract ("nothing changed → nothing written") silently does
/// not hold for that one row: it is delete+reinserted even when genuinely
/// unchanged. `key_expr_for_columns` (below, ~line 1093) already has a
/// `COALESCE`-based NULL-safe join pattern for a different call site; this
/// emitter does not yet use it — tracked in `docs/TODO.md`.
///
/// # Panics
/// Same contract as [`emit_staged_candidate_conditional`]: panics if `key`
/// or `compared_columns` is empty.
pub fn emit_staged_candidate_conditional_recompute(
    table: &str,
    staged_relation: &str,
    key: &[String],
    candidate_select: &str,
    compared_columns: &[String],
    _dialect: MaintenanceDialect,
) -> StatementGroup {
    assert!(
        !key.is_empty(),
        "emit_staged_candidate_conditional_recompute requires a non-empty row identity (key) \
         for {table}"
    );
    assert!(
        !compared_columns.is_empty(),
        "emit_staged_candidate_conditional_recompute requires a non-empty compared-column set \
         for {table}"
    );
    let key_join_table_staged = key
        .iter()
        .map(|k| format!("{table}.{k} = {staged_relation}.{k}"))
        .collect::<Vec<_>>()
        .join(" AND ");
    let key_join_t_s = key
        .iter()
        .map(|k| format!("t.{k} = s.{k}"))
        .collect::<Vec<_>>()
        .join(" AND ");
    let key_join_table_s_departed = key
        .iter()
        .map(|k| format!("{table}.{k} = s.{k}"))
        .collect::<Vec<_>>()
        .join(" AND ");
    let suppression = compared_columns
        .iter()
        .map(|c| format!("{table}.{c} IS DISTINCT FROM {staged_relation}.{c}"))
        .collect::<Vec<_>>()
        .join(" OR ");
    let create = format!(
        "CREATE TEMP TABLE {staged_relation} AS SELECT * FROM ({candidate_select}) AS \
         __smelt_staged_shape LIMIT 0"
    );
    let insert_candidates = format!("INSERT INTO {staged_relation} {candidate_select}");
    let delete_changed = format!(
        "DELETE FROM {table} USING {staged_relation} WHERE {key_join_table_staged} AND \
         ({suppression})"
    );
    let delete_departed = format!(
        "DELETE FROM {table} WHERE NOT EXISTS (SELECT 1 FROM {staged_relation} AS s WHERE \
         {key_join_table_s_departed})"
    );
    let insert = format!(
        "INSERT INTO {table} SELECT s.* FROM {staged_relation} AS s WHERE NOT EXISTS (SELECT 1 \
         FROM {table} AS t WHERE {key_join_t_s})"
    );
    let drop = format!("DROP TABLE {staged_relation}");
    StatementGroup {
        statements: vec![
            MaintenanceStatement::new(create),
            MaintenanceStatement::new(insert_candidates),
            MaintenanceStatement::new(delete_changed),
            MaintenanceStatement::new(delete_departed),
            MaintenanceStatement::new(insert),
            MaintenanceStatement::new(drop),
        ],
        transactional: true,
    }
}

/// The repair family's per-group recompute (`docs/specs/incremental_models.md`
/// §"The repair family"): a targeted `DELETE`+`INSERT` restricted to the
/// admitted [`super::repair::AdmittedRepair`]'s affected-key relation —
/// `candidate_select` is the caller's already-bounded per-group recompute
/// (`super::repair::AdmittedRepair::slice`'s clamp, already injected into
/// `candidate_select`'s read by the runtime transformer, mirroring every
/// other emitter's "caller-clamped body" contract).
///
/// `affected_keys_select` is a **single-column `delta_key` relation** — the
/// same canonical shape [`key_expr_for_columns`] builds, whether the caller
/// sourced it from the append-only clamped scan
/// (`smelt_runtime::maintenance_driver::repair_affected_keys_select`) or the
/// `mutable_snapshot` group-grain sidecar diff ([`emit_repair_group_sidecar_diff`]).
/// One shape for both paths, joined here by KEY EXPRESSION rather than by
/// raw key columns (`table.k1 = __smelt_affected.k1 AND ...`): a deleted
/// group's typed column values are unrecoverable by construction — the
/// sidecar diff's "vanished" leg has nothing but the group's own
/// `delta_key` text to offer — so a column-shaped join could never serve
/// that path, and the append-only path adopts the same shape rather than
/// carrying two joins.
///
/// 1. `CREATE TEMP TABLE <staged_relation> AS <candidate_select> LIMIT 0`
/// 2. `INSERT INTO <staged_relation> <candidate_select>`
/// 3. `DELETE FROM <table> USING (<affected_keys_select>) WHERE <key-expr
///    join>` — every stored row whose key expression is in the affected-key
///    relation, so a group that vanished entirely from the recompute (its
///    key no longer appears in `candidate_select`) is still removed.
/// 4. `INSERT INTO <table> SELECT s.* FROM <staged_relation> AS s JOIN
///    (<affected_keys_select>) ON <key-expr join>` — restricted to the SAME
///    affected-key relation as step 3, so both write statements are
///    predicated on the named key set; no statement touches `table`
///    unrestricted.
/// 5. `DROP TABLE <staged_relation>`
///
/// The whole group runs in one transaction — a failed `INSERT` must roll
/// back the `DELETE` (`docs/specs/incremental_models.md` §"Statement
/// emission (single owner)").
///
/// # Panics
/// Panics if `key` is empty — per-group recompute has no meaning without a
/// group key to restrict its writes to.
pub fn emit_per_group_recompute(
    table: &str,
    staged_relation: &str,
    key: &[String],
    affected_keys_select: &str,
    candidate_select: &str,
    _dialect: MaintenanceDialect,
) -> StatementGroup {
    assert!(
        !key.is_empty(),
        "emit_per_group_recompute requires a non-empty row identity (key) for {table}"
    );

    let affected_relation = format!(
        "(SELECT DISTINCT delta_key FROM ({affected_keys_select}) AS __smelt_affected_src) AS \
         __smelt_affected"
    );

    let create = format!(
        "CREATE TEMP TABLE {staged_relation} AS SELECT * FROM ({candidate_select}) AS \
         __smelt_staged_shape LIMIT 0"
    );
    let insert_candidates = format!("INSERT INTO {staged_relation} {candidate_select}");

    let table_key_columns: Vec<String> = key.iter().map(|k| format!("{table}.{k}")).collect();
    let table_key_expr = key_expr_for_columns(&table_key_columns);
    let delete = format!(
        "DELETE FROM {table} USING {affected_relation} WHERE {table_key_expr} = \
         __smelt_affected.delta_key"
    );

    let staged_key_columns: Vec<String> = key.iter().map(|k| format!("s.{k}")).collect();
    let staged_key_expr = key_expr_for_columns(&staged_key_columns);
    let insert = format!(
        "INSERT INTO {table} SELECT s.* FROM {staged_relation} AS s JOIN {affected_relation} ON \
         {staged_key_expr} = __smelt_affected.delta_key"
    );

    let drop = format!("DROP TABLE {staged_relation}");

    StatementGroup {
        statements: vec![
            MaintenanceStatement::new(create),
            MaintenanceStatement::new(insert_candidates),
            MaintenanceStatement::new(delete),
            MaintenanceStatement::new(insert),
            MaintenanceStatement::new(drop),
        ],
        transactional: true,
    }
}

/// The `diff_patch` write pattern (`docs/specs/incremental_models.md`
/// §"The write-pattern set is open" → "`diff_patch` — compute, diff, write
/// only the difference"): stage the computed candidate slice, then patch
/// stored state to match it — updating rows whose compared columns differ,
/// inserting rows the stored slice is missing, and (only when the caller's
/// [`DeleteLeg`] proves the candidate is complete over the slice) deleting
/// stored rows the candidate no longer contains.
///
/// This is **one function with a conditional statement**, not two sibling
/// emitters the way [`emit_staged_candidate_conditional`] /
/// [`emit_staged_candidate_conditional_recompute`] split — that pair splits
/// because their difference is a distinct, fixed caller population (a
/// region-scoped caller versus the one full-recompute membership-sensitive
/// caller); `diff_patch`'s delete-leg degradation is instead a per-call
/// *runtime* fact (this call's own [`DeleteLeg`] verdict), so branching on
/// it inside one function is the correct shape, not a second copy of the
/// other four statements.
///
/// 1. `CREATE TEMP TABLE <staged_relation> AS <candidate_select> LIMIT 0` —
///    stage an empty relation shaped like the candidate rows.
/// 2. `INSERT INTO <staged_relation> <candidate_select>` — populate it with
///    this run's computed candidate rows (already slice-restricted by
///    construction — the caller's `candidate_select` is the slice, not the
///    whole table).
/// 3. **Update leg** (always emitted): `DELETE FROM <table> USING
///    <staged_relation> WHERE <key join> AND (<IS DISTINCT FROM over
///    compared_columns>) AND <slice_predicate>` — remove exactly the
///    stored rows, within the slice, whose staged candidate differs from
///    what is stored. The slice restriction is load-bearing: without it a
///    stored row outside the slice could spuriously match the key join
///    against a staged row that does not actually correspond to it.
/// 4. **Delete leg** (only when `delete_leg` is [`DeleteLeg::Complete`] —
///    omitted entirely otherwise): `DELETE FROM <table> WHERE
///    <slice_predicate> AND NOT EXISTS (<staged_relation> row for this
///    key)` — remove stored rows, WITHIN THE SLICE, absent from the
///    candidate. The outer slice restriction is load-bearing here too, and
///    for a different reason than step 3's: `diff_patch`'s candidate is only
///    ever a slice of the table, never the full current state (unlike
///    [`emit_staged_candidate_conditional_recompute`]'s full-recompute
///    candidate) — a stored row's absence from the candidate says nothing
///    about whether it departed when that row lies OUTSIDE the slice in the
///    first place, so this delete must never reach past the slice boundary.
///
/// `slice_predicate` is a single caller-composed predicate (already
/// `<table>`-qualified on every column it names), not a `(partition_col,
/// Region)` pair: a keyed aggregate output routed through the repair family
/// has no partition column at all, only an affected-key-set membership test
/// — exactly the slice the routable recompute produces — so the pattern
/// cannot be tied to a partition axis (per this module's "callers resolve
/// strings, emitters assemble" contract). A region-partitioned caller passes
/// `region.predicate(Some(table), col)` verbatim.
/// 5. `INSERT INTO <table> SELECT s.* FROM <staged_relation> AS s WHERE NOT
///    EXISTS (target row still present for this key)` — insert candidate
///    rows the target does not yet have. No additional slice restriction is
///    needed here: the candidate rows are already slice-restricted by
///    construction (`candidate_select`).
/// 6. `DROP TABLE <staged_relation>` — cleanup.
///
/// One transaction, same contract as every other staged-candidate emitter
/// in this module (`StatementGroup::transactional`).
///
/// # Panics
/// Panics if `key` or `compared_columns` is empty — mirrors
/// [`emit_staged_candidate_conditional`]'s own contract: an identity-free or
/// vacuous-compare call has no sound diff shape to emit.
#[allow(clippy::too_many_arguments)]
pub fn emit_diff_patch(
    table: &str,
    staged_relation: &str,
    key: &[String],
    candidate_select: &str,
    compared_columns: &[String],
    slice_predicate: &str,
    delete_leg: &DeleteLeg,
    _dialect: MaintenanceDialect,
) -> StatementGroup {
    assert!(
        !key.is_empty(),
        "emit_diff_patch requires a non-empty row identity (key) for {table}"
    );
    assert!(
        !compared_columns.is_empty(),
        "emit_diff_patch requires a non-empty compared-column set for {table}"
    );

    let key_join_table_staged = key
        .iter()
        .map(|k| format!("{table}.{k} = {staged_relation}.{k}"))
        .collect::<Vec<_>>()
        .join(" AND ");
    let key_join_t_s = key
        .iter()
        .map(|k| format!("t.{k} = s.{k}"))
        .collect::<Vec<_>>()
        .join(" AND ");
    let key_join_table_s_departed = key
        .iter()
        .map(|k| format!("{table}.{k} = s.{k}"))
        .collect::<Vec<_>>()
        .join(" AND ");
    let suppression = compared_columns
        .iter()
        .map(|c| format!("{table}.{c} IS DISTINCT FROM {staged_relation}.{c}"))
        .collect::<Vec<_>>()
        .join(" OR ");

    let create = format!(
        "CREATE TEMP TABLE {staged_relation} AS SELECT * FROM ({candidate_select}) AS \
         __smelt_staged_shape LIMIT 0"
    );
    let insert_candidates = format!("INSERT INTO {staged_relation} {candidate_select}");
    let delete_changed = format!(
        "DELETE FROM {table} USING {staged_relation} WHERE {key_join_table_staged} AND \
         ({suppression}) AND {slice_predicate}"
    );
    let insert = format!(
        "INSERT INTO {table} SELECT s.* FROM {staged_relation} AS s WHERE NOT EXISTS (SELECT 1 \
         FROM {table} AS t WHERE {key_join_t_s})"
    );
    let drop = format!("DROP TABLE {staged_relation}");

    let mut statements = vec![
        MaintenanceStatement::new(create),
        MaintenanceStatement::new(insert_candidates),
        MaintenanceStatement::new(delete_changed),
    ];

    if let DeleteLeg::Complete = delete_leg {
        let delete_departed = format!(
            "DELETE FROM {table} WHERE {slice_predicate} AND NOT EXISTS (SELECT 1 FROM \
             {staged_relation} AS s WHERE {key_join_table_s_departed})"
        );
        statements.push(MaintenanceStatement::new(delete_departed));
    }

    statements.push(MaintenanceStatement::new(insert));
    statements.push(MaintenanceStatement::new(drop));

    StatementGroup {
        statements,
        transactional: true,
    }
}

/// The out-of-slice match probe for a **checked** route-3 (recurrence-
/// bounded, declared `r`) merge (`docs/specs/incremental_models.md`
/// §"Key temporal locality", route 3): a read-only query the caller
/// (`smelt-runtime`'s `maintenance_driver`) executes and inspects the
/// result of *before* running the merge action, so a violation is caught
/// without ever writing to the target — "the whole transaction rolls
/// back" trivially, since nothing was written yet.
///
/// Returns one row with two columns:
/// - `violation_count` (`BIGINT`) — the number of distinct keys the step's
///   own delta shares with a stored target row whose partition column lies
///   *before* the slice's lower bound (`slice_lower`) — exactly the
///   "matched (or would duplicate) a stored key outside the slice" check
///   the spec names.
/// - `sample_keys` (`VARCHAR`, `NULL` when `violation_count` is `0`) — up
///   to 5 comma-joined key tuples, for the `KeyedRecurrenceBoundViolated`
///   diagnostic's "sample keys" obligation.
///
/// `slice_lower` is the same concrete lower-bound literal the merge's own
/// `TargetSlicePredicate::Range` uses (the step's own partition value,
/// widened backward by the declared `r` plus margins) — this emitter does
/// no date arithmetic of its own, matching every other emitter in this
/// module.
pub fn emit_recurrence_bound_probe(
    schema_table: &str,
    key: &[String],
    partition_column: &str,
    delta_select: &str,
    slice_lower: &str,
    dialect: MaintenanceDialect,
) -> MaintenanceStatement {
    let cast_type = probe_dialect_string_type(dialect);
    let key_list = key.join(", ");
    let join_cond = key
        .iter()
        .map(|k| format!("target.{k} = delta.{k}"))
        .collect::<Vec<_>>()
        .join(" AND ");
    let key_concat = key
        .iter()
        .map(|k| format!("CAST(target.{k} AS {cast_type})"))
        .collect::<Vec<_>>()
        .join(" || '|' || ");
    let safe_lower = slice_lower.replace('\'', "''");
    let violations_select = format!(
        "SELECT DISTINCT {key_concat} AS violation_key \
         FROM {schema_table} AS target \
         JOIN (SELECT DISTINCT {key_list} FROM ({delta_select})) AS delta ON {join_cond} \
         WHERE target.{partition_column} < '{safe_lower}'"
    );
    let sql = wrap_violation_probe("__recurrence_violations", &violations_select, dialect);
    MaintenanceStatement::new(sql)
}

/// The unsized string-cast type name for a probe's key-display expression:
/// DuckDB accepts an unsized `VARCHAR`; Spark SQL requires a length on
/// `VARCHAR` (`DATATYPE_MISSING_SIZE`), so its unsized string type is
/// `STRING`. Confirmed live against Spark
/// (`docs/plans/20260720-prod-w9-spark-conformance-twin.md` Phase 5).
pub(crate) fn probe_dialect_string_type(dialect: MaintenanceDialect) -> &'static str {
    match dialect {
        MaintenanceDialect::DuckDb => "VARCHAR",
        MaintenanceDialect::Spark => "STRING",
    }
}

/// The "join up to 5 sampled `violation_key` values into one string"
/// aggregate: DuckDB has `STRING_AGG`; Spark SQL has no `STRING_AGG`, so the
/// equivalent join-aggregate is `CONCAT_WS(', ', COLLECT_LIST(...))`.
/// Shared by every probe emitter in this module (`docs/outcomes/
/// 20260809-probe-backed-facts/phases/02-plan.md` — "Design constraint: one
/// probe result shape").
fn probe_dialect_sample_agg(dialect: MaintenanceDialect) -> String {
    match dialect {
        MaintenanceDialect::DuckDb => "STRING_AGG(violation_key, ', ')".to_string(),
        MaintenanceDialect::Spark => "CONCAT_WS(', ', COLLECT_LIST(violation_key))".to_string(),
    }
}

/// Wraps a caller-supplied `violations_select` — any `SELECT` projecting a
/// `violation_key` column, one row per offending identifier — into the
/// canonical one-row probe result every emitter in this module returns:
/// `violation_count` (the number of offending rows) and `sample_keys` (up to
/// 5 comma-joined offending identifiers, `NULL` when `violation_count` is
/// `0`). `cte_name` lets each caller pick its own descriptive CTE alias
/// (`docs/specs/model_properties.md` §"Probe obligation" — "a probe's answer
/// is a single `violation_count`/`sample_keys` row").
fn wrap_violation_probe(
    cte_name: &str,
    violations_select: &str,
    dialect: MaintenanceDialect,
) -> String {
    let sample_expr = probe_dialect_sample_agg(dialect);
    format!(
        "WITH {cte_name} AS ({violations_select}) \
         SELECT COUNT(*) AS violation_count, \
                (SELECT {sample_expr} FROM \
                 (SELECT violation_key FROM {cte_name} LIMIT 5) AS __sample) \
                 AS sample_keys \
         FROM {cte_name}"
    )
}

/// A key's display expression for a probe's `violation_key` column: each
/// column CAST to the dialect's unsized string type and pipe-concatenated —
/// the same composite-key shape [`emit_recurrence_bound_probe`] uses,
/// factored out so the four declaration probes below share it.
fn probe_key_display_expr(columns: &[String], cast_type: &str) -> String {
    columns
        .iter()
        .map(|c| format!("CAST({c} AS {cast_type})"))
        .collect::<Vec<_>>()
        .join(" || '|' || ")
}

/// The referential-integrity count-preservation tripwire
/// (`docs/specs/sources.md` §"Referential integrity"; `model_properties.md`
/// §"Skeleton-source closure" P1, row-preservation conjunct 4): a read-only
/// query the caller (`smelt-runtime`) executes and inspects *before*
/// trusting an inner-join enrichment recompute a declared
/// `referential_integrity` world-fact licensed, so a violation is caught
/// without ever writing to the target — "the whole transaction rolls back"
/// trivially, the same shape [`emit_recurrence_bound_probe`] uses for
/// route 3's out-of-slice match probe.
///
/// Returns one row with two columns:
/// - `driving_count` (`BIGINT`) — the row count of `driving_select` (the
///   fact side alone, scoped to the touched region) — the count a
///   row-preserving join must not fall short of.
/// - `enriched_count` (`BIGINT`) — the row count of `enriched_select` (the
///   SAME region's inner-join enrichment recompute). `enriched_count <
///   driving_count` disproves the declared `referential_integrity` — some
///   driving row's join key had no match in the dimension, so the inner
///   join silently dropped it — and the caller fails the run loudly
///   (`SourceCountPreservationViolated`) rather than trusting the
///   declaration's licensed technique against a stale or simply wrong
///   fact.
///
/// `driving_select`/`enriched_select` are the caller's own already-compiled
/// `SELECT`s (scoped to the touched region); this emitter does no scoping
/// of its own, matching every other function in this module.
pub fn emit_count_preservation_probe(
    driving_select: &str,
    enriched_select: &str,
) -> MaintenanceStatement {
    let sql = format!(
        "SELECT (SELECT COUNT(*) FROM ({driving_select}) AS __smelt_driving) AS driving_count, \
         (SELECT COUNT(*) FROM ({enriched_select}) AS __smelt_enriched) AS enriched_count"
    );
    MaintenanceStatement::new(sql)
}

/// Build [`emit_count_preservation_probe`]'s `driving_select`/`enriched_select`
/// pair directly from a model's own already-compiled `body_sql`, for a
/// caller (`smelt-runtime`'s `execute_delete_insert_with_delta_restriction`)
/// that has the body but not a hand-maintained pair of scoped `SELECT`s: the
/// **enriched** side is `body_sql`'s own top-level `FROM`/`JOIN`/`WHERE`
/// text unchanged (the join against `enrichment_source` still present); the
/// **driving** side is the SAME text with that one join clause spliced out
/// by text range — never re-authored, never re-derived from a second parse
/// of a different string, so both sides carry byte-identical scoping
/// (WHERE, other joins) except for the one join this probe exists to
/// falsify.
///
/// `body_sql` is already-compiled SQL (`smelt.<path>` refs already resolved
/// to physical `schema.table` names by the SQL compiler, matching what
/// `execute_delete_insert_with_delta_restriction` actually holds at its
/// call site) — the join is found by `TableRef::bare_path_text`'s plain
/// dotted-identifier text, never `analysis::source_bounds::
/// resolve_table_ref_source_name` (which only recognises unresolved
/// `smelt.<path>` refs and would never match a compiled body). A match is
/// exact-path or last-segment equality, so a caller may name
/// `enrichment_source` either as the full physical path (`main.dim`) or
/// just its bare table name (`dim`).
///
/// Fail-closed to `None` — never a best-effort guess — when `body_sql` has
/// no top-level `SELECT`, no `FROM` clause, or no join against
/// `enrichment_source` found in that `FROM` clause's own joins.
pub fn emit_count_preservation_probe_from_body(
    body_sql: &str,
    enrichment_source: &str,
) -> Option<MaintenanceStatement> {
    let parse = smelt_parser::parse(body_sql);
    let file = smelt_parser::File::cast(parse.syntax())?;
    let select = file.select_stmt()?;
    let from_clause = select.from_clause()?;

    let last_segment = |s: &str| s.rsplit('.').next().unwrap_or(s).to_string();
    let target_last = last_segment(enrichment_source);
    let join = from_clause.joins().find(|join| {
        join.table_ref()
            .and_then(|table_ref| table_ref.bare_path_text())
            .is_some_and(|path| path == enrichment_source || last_segment(&path) == target_last)
    })?;

    let from_range = from_clause.text_range();
    let join_range = join.syntax().text_range();
    let where_suffix = select
        .where_clause()
        .map(|w| format!(" {}", &body_sql[w.text_range()]))
        .unwrap_or_default();

    let enriched_from = &body_sql[from_range];
    let before_join = smelt_parser::TextRange::new(from_range.start(), join_range.start());
    let after_join = smelt_parser::TextRange::new(join_range.end(), from_range.end());
    let driving_from = format!("{}{}", &body_sql[before_join], &body_sql[after_join]);

    let driving_select = format!("SELECT 1 {driving_from}{where_suffix}");
    let enriched_select = format!("SELECT 1 {enriched_from}{where_suffix}");

    Some(emit_count_preservation_probe(
        &driving_select,
        &enriched_select,
    ))
}

/// The functional-dependency probe (`docs/specs/model_properties.md`
/// §"Probe obligation", row `functional_dependencies:`): a read-only query
/// re-aggregating the declared `key` over `scope_select`'s own processed
/// rows and counting the distinct `determines` values found per key. A key
/// with more than one distinct `determines` value disproves the declared
/// per-key constancy the once-write column family was admitted on the
/// strength of (`model_properties.md` §Known Divergences).
///
/// Returns the same `violation_count`/`sample_keys` shape every probe in
/// this module returns (`docs/outcomes/20260809-probe-backed-facts/
/// phases/02-plan.md`): the number of offending keys, and up to 5
/// comma-joined offending key values.
///
/// `scope_select` is the caller's own already-compiled `SELECT` over the
/// run's processed rows; this emitter does no scoping of its own, matching
/// every other function in this module.
///
/// # Panics
/// Panics if `key` is empty — an empty `GROUP BY` would aggregate the whole
/// scope into one row, never a per-key constancy check.
pub fn emit_functional_dependency_probe(
    scope_select: &str,
    key: &[String],
    determines: &str,
    dialect: MaintenanceDialect,
) -> MaintenanceStatement {
    assert!(
        !key.is_empty(),
        "emit_functional_dependency_probe requires a non-empty key"
    );
    let cast_type = probe_dialect_string_type(dialect);
    let key_list = key.join(", ");
    let violation_key = probe_key_display_expr(key, cast_type);
    let violations_select = format!(
        "SELECT {violation_key} AS violation_key \
         FROM ({scope_select}) AS __smelt_scope \
         GROUP BY {key_list} \
         HAVING COUNT(DISTINCT {determines}) > 1"
    );
    let sql = wrap_violation_probe("__fd_violations", &violations_select, dialect);
    MaintenanceStatement::new(sql)
}

/// The bounded-domain probe (`docs/specs/model_properties.md` §"Probe
/// obligation", row `bounded_domain:`): a read-only query counting the
/// distinct values of the declared `column` within `scope_select`'s own
/// processed region and comparing that count against the declared
/// `max_cardinality`.
///
/// Returns one row: `violation_count` is the distinct-value count when it
/// exceeds `max_cardinality`, `0` when the domain is within cap;
/// `sample_keys` is up to 5 comma-joined sample values, `NULL` when within
/// cap — the same shape every probe in this module returns, specialised
/// (unlike the key-recurring probes) to a magnitude check rather than a
/// membership check.
///
/// `scope_select` is the caller's own already-compiled `SELECT` over the
/// run's processed region; this emitter does no scoping of its own,
/// matching every other function in this module.
pub fn emit_bounded_domain_probe(
    scope_select: &str,
    column: &str,
    max_cardinality: u64,
    dialect: MaintenanceDialect,
) -> MaintenanceStatement {
    let cast_type = probe_dialect_string_type(dialect);
    let sample_expr = probe_dialect_sample_agg(dialect);
    let sql = format!(
        "WITH __bounded_domain_values AS (\
            SELECT DISTINCT CAST({column} AS {cast_type}) AS violation_key \
            FROM ({scope_select}) AS __smelt_scope\
         ), __bounded_domain_count AS (\
            SELECT COUNT(*) AS distinct_count FROM __bounded_domain_values\
         ) \
         SELECT CASE WHEN distinct_count > {max_cardinality} THEN distinct_count ELSE 0 END \
                AS violation_count, \
                CASE WHEN distinct_count > {max_cardinality} THEN \
                  (SELECT {sample_expr} FROM \
                   (SELECT violation_key FROM __bounded_domain_values LIMIT 5) AS __sample) \
                ELSE NULL END AS sample_keys \
         FROM __bounded_domain_count"
    );
    MaintenanceStatement::new(sql)
}

/// The monotonicity probe (`docs/specs/model_properties.md` §"Probe
/// obligation", row `timeseries.assert_monotonic`): a read-only query
/// re-deriving the traced event-time ordering over `scope_select`'s own
/// processed rows, per `partition_key` — a `LAG` window over
/// `event_time_column` ordered by itself within each partition, flagging a
/// row whose event time falls below its partition predecessor's.
///
/// Returns the same `violation_count`/`sample_keys` shape every probe in
/// this module returns: the number of out-of-order rows, and up to 5
/// comma-joined offending partition-key values.
///
/// `scope_select` is the caller's own already-compiled `SELECT` over the
/// run's processed rows; this emitter does no scoping of its own, matching
/// every other function in this module.
///
/// # Panics
/// Panics if `partition_key` is empty — an empty partition would order the
/// entire scope as one sequence, never a per-partition monotonicity check.
pub fn emit_monotonicity_probe(
    scope_select: &str,
    partition_key: &[String],
    event_time_column: &str,
    dialect: MaintenanceDialect,
) -> MaintenanceStatement {
    assert!(
        !partition_key.is_empty(),
        "emit_monotonicity_probe requires a non-empty partition key"
    );
    let cast_type = probe_dialect_string_type(dialect);
    let partition_list = partition_key.join(", ");
    let violation_key = probe_key_display_expr(partition_key, cast_type);
    // The `LAG` must be ordered by the run's own PROCESSED-row order (a
    // `ROW_NUMBER() OVER ()` ordinal over `scope_select`'s own row
    // sequence), never by `event_time_column` itself — ordering the window
    // by the very column being checked would sort every partition into
    // non-decreasing order by construction, making a violation
    // undetectable. "The traced event-time ordering" (`docs/specs/
    // model_properties.md` §"Probe obligation") means the order rows were
    // processed in, which the caller's `scope_select` already reflects
    // (e.g. an append-only ingestion order).
    let violations_select = format!(
        "SELECT {violation_key} AS violation_key \
         FROM (\
            SELECT {partition_list}, __smelt_event_time, \
                   LAG(__smelt_event_time) OVER (\
                       PARTITION BY {partition_list} ORDER BY __smelt_seq\
                   ) AS __smelt_prev_event_time \
            FROM (\
               SELECT {partition_list}, {event_time_column} AS __smelt_event_time, \
                      ROW_NUMBER() OVER () AS __smelt_seq \
               FROM ({scope_select}) AS __smelt_scope\
            ) AS __smelt_seqd\
         ) AS __smelt_lagged \
         WHERE __smelt_prev_event_time IS NOT NULL \
         AND __smelt_event_time < __smelt_prev_event_time"
    );
    let sql = wrap_violation_probe("__monotonicity_violations", &violations_select, dialect);
    MaintenanceStatement::new(sql)
}

/// One partition's recorded baseline for [`emit_append_only_posture_probe`]:
/// the row count and skeleton-column fingerprint observed the last time the
/// posture was checked, kept by the caller (the run driver, phases 3-4 of
/// `docs/outcomes/20260809-probe-backed-facts/outcome.md`) — this emitter
/// carries no storage of its own, matching the maintenance-plan purity
/// invariant (`docs/specs/architecture.md` §"Constraints & Invariants" item
/// 12).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppendOnlyBaselinePartition {
    /// The partition column's value, as text.
    pub partition_value: String,
    /// The row count last recorded for this partition.
    pub recorded_count: i64,
    /// The row-content fingerprint (hex `sha256`) last recorded for this
    /// partition, over the same `digest_columns` the caller passes to
    /// [`emit_append_only_posture_probe`].
    pub recorded_fingerprint: String,
    /// Whether this partition's fingerprint leg is checked this run. A
    /// whole-partition fingerprint changes on a *legitimate* append, so the
    /// caller gates this to `false` for the still-open partition (the
    /// recorded maximum `partition_value`) and `true` for every partition
    /// strictly below it (`docs/outcomes/20260809-probe-backed-facts/
    /// outcome.md` phase 6: "the frontier gate"). The count leg is never
    /// gated — a count decrease always violates append-only posture
    /// regardless of this flag.
    pub check_fingerprint: bool,
}

/// The append-only posture probe (`docs/specs/model_properties.md` §"Probe
/// obligation", row `mutation_profile.kind: append_only`; `docs/specs/
/// sources.md` §Semantics 4 — "watermark-monotonicity + frontier-checksum"):
/// a read-only query comparing each partition's CURRENT row count and
/// row-content fingerprint (over `digest_columns`, the source's skeleton
/// columns) against a caller-supplied recorded `baseline` — a partition
/// whose row count decreased (a delete or reload) or whose fingerprint
/// changed (an in-place update) disproves the declared `append_only`
/// posture.
///
/// Returns the same `violation_count`/`sample_keys` shape every probe in
/// this module returns: the number of violating partitions, and up to 5
/// comma-joined offending partition values.
///
/// The per-row fingerprint reuses [`row_fingerprint_expr`] (the same
/// collision-free construction the fingerprint sidecar digests with); the
/// per-partition aggregate fingerprint is `sha256` of those row
/// fingerprints joined in a fixed (sorted) order, so two runs over the same
/// unchanged partition content always agree regardless of physical row
/// order.
///
/// `source_table` is already fully qualified (`schema.table`); `baseline`
/// is rendered as a `VALUES` list, one row per recorded partition — a
/// partition with no baseline row is not compared (nothing recorded yet to
/// disprove). Each baseline row's `check_fingerprint` gates whether that
/// partition's fingerprint leg participates this run — the count leg is
/// never gated (`AppendOnlyBaselinePartition::check_fingerprint`'s doc
/// comment).
///
/// # Panics
/// Panics if `digest_columns` or `baseline` is empty — nothing to
/// fingerprint, or nothing recorded to compare against, is not a
/// degenerate always-passing probe.
pub fn emit_append_only_posture_probe(
    source_table: &str,
    partition_column: &str,
    digest_columns: &[String],
    baseline: &[AppendOnlyBaselinePartition],
    dialect: MaintenanceDialect,
) -> MaintenanceStatement {
    assert!(
        !digest_columns.is_empty(),
        "emit_append_only_posture_probe requires a non-empty digest column set for {source_table}"
    );
    assert!(
        !baseline.is_empty(),
        "emit_append_only_posture_probe requires a non-empty recorded baseline for {source_table}"
    );
    let cast_type = probe_dialect_string_type(dialect);
    let snapshot =
        emit_append_only_baseline_snapshot(source_table, partition_column, digest_columns, dialect);
    let baseline_values = baseline
        .iter()
        .map(|b| {
            let value = b.partition_value.replace('\'', "''");
            let fingerprint = b.recorded_fingerprint.replace('\'', "''");
            let check_fingerprint = if b.check_fingerprint { "TRUE" } else { "FALSE" };
            format!(
                "('{value}', {}, '{fingerprint}', {check_fingerprint})",
                b.recorded_count
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let violations_select = format!(
        "SELECT CAST(__current.partition_value AS {cast_type}) AS violation_key \
         FROM ({}) AS __current \
         JOIN (VALUES {baseline_values}) AS __baseline(partition_value, recorded_count, \
               recorded_fingerprint, check_fingerprint) \
         ON __current.partition_value = __baseline.partition_value \
         WHERE __current.current_count < __baseline.recorded_count \
         OR (__baseline.check_fingerprint AND __current.current_fingerprint IS DISTINCT FROM \
             __baseline.recorded_fingerprint)",
        snapshot.sql
    );
    let sql = wrap_violation_probe("__append_only_violations", &violations_select, dialect);
    MaintenanceStatement::new(sql)
}

/// The per-partition CURRENT-state `SELECT` [`emit_append_only_posture_probe`]
/// compares its recorded baseline against: `partition_value`,
/// `current_count`, and `current_fingerprint` for every partition presently
/// in `source_table`. Extracted as its own emitter so the runtime can
/// execute it standalone to refresh the recorded baseline after a held
/// probe (`docs/outcomes/20260809-probe-backed-facts/outcome.md` phase 6) —
/// the recorded and compared fingerprints are then the literally same SQL
/// construction, never two independent renderings that could drift.
///
/// `source_table` is already fully qualified (`schema.table`). The
/// fingerprint construction is identical to
/// [`emit_append_only_posture_probe`]'s own (see that function's doc
/// comment).
///
/// # Panics
/// Panics if `digest_columns` is empty — nothing to fingerprint is not a
/// degenerate probe.
pub fn emit_append_only_baseline_snapshot(
    source_table: &str,
    partition_column: &str,
    digest_columns: &[String],
    dialect: MaintenanceDialect,
) -> MaintenanceStatement {
    assert!(
        !digest_columns.is_empty(),
        "emit_append_only_baseline_snapshot requires a non-empty digest column set for {source_table}"
    );
    let cast_type = probe_dialect_string_type(dialect);
    let row_hash = row_fingerprint_expr(digest_columns, dialect);
    let agg_fingerprint = match dialect {
        MaintenanceDialect::DuckDb => {
            format!("sha256(STRING_AGG({row_hash}, '' ORDER BY {row_hash}))")
        }
        MaintenanceDialect::Spark => {
            format!("sha256(CONCAT_WS('', SORT_ARRAY(COLLECT_LIST({row_hash}))))")
        }
    };
    let sql = format!(
        "SELECT CAST({partition_column} AS {cast_type}) AS partition_value, \
         COUNT(*) AS current_count, {agg_fingerprint} AS current_fingerprint \
         FROM {source_table} \
         GROUP BY {partition_column}"
    );
    MaintenanceStatement::new(sql)
}

/// First-run `CREATE TABLE … AS` for a windowed-keyed-maintenance cell
/// (`maintenance_driver::run_windowed_keyed_maintenance`'s create arm): the
/// target table does not exist yet, so the first step's delta becomes the
/// table wholesale — no read of prior state, no `MERGE`.
///
/// `table` is already fully qualified (`schema.table`); `select_sql` is the
/// caller's already-compiled delta `SELECT` for that step, unmodified.
///
/// `dialect` is accepted for signature symmetry with the other emitters in
/// this module; `CREATE TABLE … AS SELECT …` is dialect-invariant across
/// DuckDB and Spark for this family.
pub fn emit_create_table_as(
    table: &str,
    select_sql: &str,
    dialect: MaintenanceDialect,
) -> StatementGroup {
    // Every step after this bootstrap CREATE for a merge-based cell (keyed
    // fold, column-scoped merge) is a `MERGE INTO`, which Spark refuses
    // against a plain (default-format, non-Delta) managed table — so the
    // bootstrap itself must specify `USING DELTA` on Spark. DuckDB has no
    // such format clause. `MaintenanceDialect::Spark` covers only the
    // merge-capable Spark family (see `smelt_backend::maintenance_dialect`),
    // so this is never reached for the cross-engine Parquet read path
    // (`docs/plans/20260720-prod-w9-spark-conformance-twin.md` Phase 4).
    let using_clause = match dialect {
        MaintenanceDialect::DuckDb => "",
        MaintenanceDialect::Spark => " USING DELTA",
    };
    StatementGroup {
        statements: vec![MaintenanceStatement::new(format!(
            "CREATE TABLE {table}{using_clause} AS {select_sql}"
        ))],
        transactional: false,
    }
}

/// First-run bootstrap for a **self-referential** partition-grain model
/// (`docs/specs/incremental_models.md` §"First-run and backfill" —
/// "First-run bootstrap for a self-referential model"): the target does not
/// exist yet, and the model's own first-batch SELECT reads that same target
/// via `smelt.<self>`, so `CREATE TABLE … AS SELECT …` cannot resolve it —
/// no engine can create a table and read it in the same statement. Instead
/// this emitter authors a plain, empty `CREATE TABLE` from the caller's own
/// inferred output schema (column name, SQL type) — no `SELECT`, no read of
/// any table. The subsequent batch loop then executes the model's first
/// partition (and every later one) as the ordinary region `DELETE`+`INSERT`
/// (`emit_delete_insert`); the self-read over this empty table correctly
/// sees no prior state.
///
/// `columns` is plain data the caller already resolved — a self-
/// referential model's own output-schema fixpoint
/// (`smelt-runtime`'s `UpstreamSchemas::from_database`, which refines what
/// `resolved_model_schema` alone cannot: that Salsa query's `cycle_initial`
/// BREAKS a genuine self-referential cycle with an empty schema rather than
/// iterating it to a fixpoint). This function does no inference of its own,
/// only string assembly, preserving the maintenance-plan purity invariant
/// (`docs/specs/architecture.md` §"Constraints & Invariants" item 12).
///
/// `table` is already fully qualified (`schema.table`). `dialect` selects
/// the SQL type spelling for `Text`/`Varchar` columns: Spark 4+ rejects a
/// bare `VARCHAR` (no length), so those columns render as `STRING`; DuckDB
/// takes bare `VARCHAR`. Every other `DataType` renders via
/// `DataType::to_backend_sql()`, the same mapping `wrap_with_type_casts`
/// (`smelt-dialect`) uses for its CAST wrapper.
///
/// # Panics
/// Panics if `columns` is empty — a model with no resolvable output columns
/// cannot be bootstrapped into a valid `CREATE TABLE`; the caller must not
/// reach this emitter without a non-empty resolved schema.
pub fn emit_create_empty_table(
    table: &str,
    columns: &[(String, smelt_types::DataType)],
    dialect: MaintenanceDialect,
) -> StatementGroup {
    assert!(
        !columns.is_empty(),
        "emit_create_empty_table requires a non-empty resolved output schema for {table}"
    );
    let col_defs = columns
        .iter()
        .map(|(name, dt)| format!("{name} {}", bootstrap_column_sql_type(dt, dialect)))
        .collect::<Vec<_>>()
        .join(", ");
    StatementGroup {
        statements: vec![MaintenanceStatement::new(format!(
            "CREATE TABLE {table} ({col_defs})"
        ))],
        transactional: false,
    }
}

/// The SQL type spelling `emit_create_empty_table` uses for one column.
/// Mirrors `smelt_dialect::type_conformance::type_cast_sql`'s Spark-STRING
/// carve-out (that function is not reused directly: `smelt-logical` and
/// `smelt-dialect` are sibling crates over `smelt-types`/`smelt-parser`, and
/// this module's dependency footprint is deliberately kept to `smelt-types`
/// alone, matching every other emitter in this file).
fn bootstrap_column_sql_type(dt: &smelt_types::DataType, dialect: MaintenanceDialect) -> String {
    match (dt, dialect) {
        (smelt_types::DataType::Text, MaintenanceDialect::Spark)
        | (smelt_types::DataType::Varchar { max_length: None }, MaintenanceDialect::Spark) => {
            "STRING".to_string()
        }
        _ => dt.to_backend_sql(),
    }
}

// ── Decomposed-state column fold expansion (`docs/specs/
// incremental_models.md` §"Decomposed state (rung 2) in keyed models",
// "Combiner over state") ─────────────────────────────────────────────────

/// Expand one [`AggregatorColumn`](crate::rules::cumulative::AggregatorColumn)
/// into its `(column, combine_expression)` fold pairs for the `MERGE`'s
/// `SET` clause. Single-owner statement rule (`docs/specs/
/// incremental_models.md` §"Statement emission (single owner)"): both the
/// executed keyed-fold `MERGE` (`smelt-runtime::cumulative::
/// build_cumulative_merge_sql`) and the `smelt explain` preview
/// (`smelt-runtime::diagnostics`) call this so their fold shapes can never
/// diverge (`docs/outcomes/20260809-rung2-state-shapes` row 7).
///
/// A stateless column (`state: None`, every family admitted before this
/// mechanism existed) still produces exactly the one pair it always has. A
/// state-bearing column expands into one pair per hidden state column (each
/// folded by its own combiner over `target.<c>`/`delta.<c>`) plus the
/// presented column, set to the presentation expression with every
/// state-column reference substituted by that column's own *merged*
/// expression — so the presented value is always recomputed fresh from the
/// just-merged state, never folded directly.
pub fn expand_aggregator_column_folds(
    col: &crate::rules::cumulative::AggregatorColumn,
) -> Vec<(String, String)> {
    let Some(state) = &col.state else {
        let target_col = format!("target.{}", col.output_name);
        let delta_col = format!("delta.{}", col.output_name);
        let expr = col.cross_partition_combiner.render(&target_col, &delta_col);
        return vec![(col.output_name.clone(), expr)];
    };

    let mut folds: Vec<(String, String)> = Vec::with_capacity(state.state_columns.len() + 1);
    let mut merged_by_name: Vec<(String, String)> = Vec::with_capacity(state.state_columns.len());
    for state_col in &state.state_columns {
        let target_col = format!("target.{}", state_col.name);
        let delta_col = format!("delta.{}", state_col.name);
        let merged = state_col.combiner.render(&target_col, &delta_col);
        merged_by_name.push((state_col.name.clone(), merged.clone()));
        folds.push((state_col.name.clone(), merged));
    }
    // One simultaneous pass over the ORIGINAL presentation expression, not a
    // chain of dependent substitutions — a state column's own merged
    // expression can embed another state column's qualified name (the
    // order-monotone `v` column's fold text names its sibling `o` column,
    // e.g. `target.status__o`), and re-scanning already-substituted text for
    // the next name would corrupt it (`docs/outcomes/
    // 20260809-rung2-state-shapes` row 5).
    let presentation_expr = substitute_identifiers(&state.presentation_expr, &merged_by_name);
    folds.push((col.output_name.clone(), presentation_expr));
    folds
}

/// Replace every whole-identifier occurrence of each `(name, replacement)`
/// pair in `text`, in one simultaneous left-to-right pass over the
/// ORIGINAL `text` — a match must not be preceded or followed by another
/// identifier character (`[A-Za-z0-9_]`), so `avg_amount__sum` is not
/// matched inside `avg_amount__sum_2`. A single pass (rather than N
/// sequential single-name substitutions) matters: a replacement text can
/// itself contain another pair's `name` as a substring (a state column's
/// merged fold expression naming a sibling state column), and re-scanning
/// already-substituted output for the next name would corrupt it. Used to
/// rewrite a `DecomposedState` presentation expression's state-column
/// references onto their merged fold expressions
/// (`expand_aggregator_column_folds`) — plain string substitution over SQL
/// identifiers, not general SQL rewriting.
fn substitute_identifiers(text: &str, replacements: &[(String, String)]) -> String {
    fn is_ident_char(c: char) -> bool {
        c.is_ascii_alphanumeric() || c == '_'
    }

    let bytes = text.as_bytes();
    let mut result = String::with_capacity(text.len());
    let mut skip_until = 0usize;
    for (i, ch) in text.char_indices() {
        if i < skip_until {
            continue;
        }
        let matched = replacements.iter().find(|(name, _)| {
            text[i..].starts_with(name.as_str())
                && (i == 0 || !is_ident_char(bytes[i - 1] as char))
                && {
                    let after = i + name.len();
                    after >= bytes.len() || !is_ident_char(bytes[after] as char)
                }
        });
        if let Some((name, replacement)) = matched {
            result.push('(');
            result.push_str(replacement);
            result.push(')');
            skip_until = i + name.len();
            continue;
        }
        result.push(ch);
    }
    result
}

// ── Decomposed state (rung 2) select augmentation (`docs/specs/
// incremental_models.md` §"Decomposed state (rung 2) in keyed models") ────

/// Why [`state_augmented_projection`] could not append the state columns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StateAugmentRefusal {
    /// `sql` could not be parsed, or its SELECT list could not be located —
    /// fail-closed rather than text-splice blind.
    Unparseable,
}

/// Append one `, <per_partition_expr> AS <name>` select item per
/// `state_columns` to `sql`'s own SELECT list, leaving every other clause
/// (the key/GROUP BY columns, the model's own presented select items, WHERE/
/// FROM/GROUP BY) byte-unchanged. `state_columns` is derived once from the
/// classification (`decomposed_state::DecomposedState::state_columns`
/// across every state-bearing `AggregatorColumn`); the caller applies this
/// to the compiled delta SELECT so the stored table and the delta agree on
/// columns before `CREATE TABLE AS` / `MERGE ... WHEN NOT MATCHED THEN
/// INSERT *` (`docs/specs/incremental_models.md` §"Decomposed state (rung 2)
/// in keyed models"). `state_columns.is_empty()` returns `sql` unchanged —
/// the stateless shape every column family admitted before this mechanism
/// existed still produces.
///
/// The insertion point is located via the CST (the last select item's own
/// `text_range`), never a whole-text scan — this emitter is a leaf
/// operation over one already-parsed SELECT, not a second admission pass
/// (`docs/specs/architecture.md` §"Property composition walk rule").
/// Refuses (never mangles the string) when `sql` doesn't parse or its
/// SELECT list can't be located.
pub fn state_augmented_projection(
    sql: &str,
    state_columns: &[crate::analysis::decomposed_state::StateColumn],
) -> Result<String, StateAugmentRefusal> {
    if state_columns.is_empty() {
        return Ok(sql.to_string());
    }
    let parse = smelt_parser::parse(sql);
    let file = smelt_parser::File::cast(parse.syntax()).ok_or(StateAugmentRefusal::Unparseable)?;
    let select = file.select_stmt().ok_or(StateAugmentRefusal::Unparseable)?;
    let list = select
        .select_list()
        .ok_or(StateAugmentRefusal::Unparseable)?;
    let last_item = list
        .items()
        .last()
        .ok_or(StateAugmentRefusal::Unparseable)?;
    let insert_at: usize = last_item.range().end().into();

    let mut additions = String::new();
    for state_col in state_columns {
        additions.push_str(&format!(
            ", {} AS {}",
            state_col.per_partition_expr, state_col.name
        ));
    }
    let mut out = String::with_capacity(sql.len() + additions.len());
    out.push_str(&sql[..insert_at]);
    out.push_str(&additions);
    out.push_str(&sql[insert_at..]);
    Ok(out)
}

// ── Presentation projection (rung 2, `docs/specs/incremental_models.md`
// §"Decomposed state (rung 2) in keyed models" → "Presentation
// projection") ────────────────────────────────────────────────────────

/// Why [`presentation_projection`] could not hide state columns behind a
/// wildcard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PresentationRefusal {
    /// `sql` could not be parsed, or its SELECT list could not be located —
    /// fail-closed rather than text-splice blind.
    Unparseable,
    /// A wildcard's relation could not be resolved while a state-bearing
    /// model was in scope. Passing it through unrewritten risks leaking
    /// state columns into the consumer's schema, so this refuses instead of
    /// guessing (`docs/specs/incremental_models.md` §"Decomposed state
    /// (rung 2) in keyed models" → "Presentation projection").
    UnresolvableWildcard {
        /// The offending wildcard's own source text (`*` or
        /// `<qualifier>.*`).
        wildcard: String,
    },
}

impl std::fmt::Display for PresentationRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PresentationRefusal::Unparseable => {
                write!(f, "SQL could not be parsed for presentation projection")
            }
            PresentationRefusal::UnresolvableWildcard { wildcard } => write!(
                f,
                "wildcard `{wildcard}` could not be resolved to a FROM/JOIN relation while a \
                 state-bearing model is in scope"
            ),
        }
    }
}

/// One relation in a SELECT's `FROM`/`JOIN` clause, as `presentation_
/// projection` needs it: the name a wildcard can qualify it by, and (when
/// it is a `smelt.<path>` reference) the leaf model name `state_bearing`
/// is keyed by.
struct Relation {
    /// The name this relation can be referenced by from the select list —
    /// its explicit/implicit alias, falling back to the leaf model name or
    /// plain identifier when unaliased. `None` only for a relation this
    /// walk cannot name at all (e.g. an unaliased subquery).
    qualifier: Option<String>,
    /// The name `state_bearing` is keyed by: a `smelt.<path>` reference's
    /// leaf segment, or a plain identifier. `None` for a relation with
    /// neither (a subquery), which can never be state-bearing.
    resolved_name: Option<String>,
}

/// Resolve one `TableRef`'s leaf `smelt.<path>` model name (value or
/// call form), mirroring the leaf-segment extraction every other
/// `smelt-logical` walk over a `FROM`/`JOIN` clause already duplicates
/// (`analysis/walk.rs`'s `normalize_table_ref`, `analysis/source_bounds.rs`,
/// `rules/incremental.rs`) — there is no shared helper to call instead, and
/// `smelt-logical` has no dependency on `smelt-db` to borrow one from.
fn table_ref_model_name(table_ref: &smelt_parser::ast::TableRef) -> Option<String> {
    table_ref
        .smelt_path_ref()
        .and_then(|p| p.segments().last().cloned())
        .or_else(|| {
            table_ref
                .smelt_path_call()
                .and_then(|p| p.segments().last().cloned())
        })
}

/// All relations a SELECT's `FROM`/comma-list/`JOIN`s contribute, in source
/// order — the same `table_refs().chain(joins()...)` traversal
/// `analysis/walk.rs`'s `normalize_from` already uses for from-clause
/// enumeration.
fn from_relations(from: &smelt_parser::ast::FromClause) -> Vec<Relation> {
    from.table_refs()
        .chain(from.joins().filter_map(|j| j.table_ref()))
        .map(|table_ref| {
            let resolved_name = table_ref_model_name(&table_ref).or_else(|| table_ref.identifier());
            let qualifier = table_ref.alias().or_else(|| resolved_name.clone());
            Relation {
                qualifier,
                resolved_name,
            }
        })
        .collect()
}

/// Rewrite `sql`'s wildcard select items so a state-bearing model's
/// `__part` state columns never reach a consumer's schema: a wildcard over
/// a relation `state_bearing` names is expanded to that model's presented
/// columns (`state_bearing`'s values, in schema order); a wildcard over a
/// relation `state_bearing` does not name is left byte-unchanged.
/// `state_bearing` maps model name → presented column names — its values
/// come from the public schema (`UpstreamSchemas::models` at the caller),
/// its keys from the set of models classified as state-bearing; this
/// function invents no new source of truth for "which columns are
/// presented".
///
/// Returns `sql` byte-identical when no relation in scope is
/// state-bearing (`state_bearing.is_empty()` or none of its keys appear in
/// `sql`'s `FROM`/`JOIN`) — the parity path every project not using
/// decomposed state still takes. Refuses
/// ([`PresentationRefusal::UnresolvableWildcard`]) rather than passing a
/// wildcard through unexpanded when a state-bearing relation is in scope
/// and the wildcard's own relation cannot be resolved (an unknown
/// qualifier, or a bare `*` over an unnameable relation) — a silent
/// pass-through there would leak state columns into the consumer's schema.
///
/// The rewrite locates each wildcard select item via its own CST
/// `range()`, never a whole-text scan for `*` — this emitter is a leaf
/// operation over one already-parsed SELECT (`docs/specs/architecture.md`
/// §"Property composition walk rule"), so a `*` inside a string literal
/// (wrapped in an `EXPRESSION` node, `SelectItem::is_wildcard()` returns
/// `false` for it) is never touched.
pub fn presentation_projection(
    sql: &str,
    state_bearing: &std::collections::BTreeMap<String, Vec<String>>,
) -> Result<String, PresentationRefusal> {
    let parse = smelt_parser::parse(sql);
    let file = smelt_parser::File::cast(parse.syntax()).ok_or(PresentationRefusal::Unparseable)?;
    let select = file.select_stmt().ok_or(PresentationRefusal::Unparseable)?;
    let list = select
        .select_list()
        .ok_or(PresentationRefusal::Unparseable)?;
    let relations: Vec<Relation> = select
        .from_clause()
        .map(|from| from_relations(&from))
        .unwrap_or_default();

    let any_state_bearing = relations
        .iter()
        .any(|r| matches_state_bearing(r, state_bearing));
    if !any_state_bearing {
        return Ok(sql.to_string());
    }

    let mut out = String::with_capacity(sql.len());
    let mut last_end: usize = 0;
    for item in list.items() {
        let replacement = if let Some(qualifier) = item.qualified_wildcard_target() {
            match relations
                .iter()
                .find(|r| r.qualifier.as_deref() == Some(qualifier.as_str()))
            {
                Some(rel) => match state_bearing_columns(rel, state_bearing) {
                    Some(cols) => Some(
                        cols.iter()
                            .map(|c| format!("{qualifier}.{c}"))
                            .collect::<Vec<_>>()
                            .join(", "),
                    ),
                    None => None,
                },
                None => {
                    return Err(PresentationRefusal::UnresolvableWildcard {
                        wildcard: sql[item.range()].to_string(),
                    });
                }
            }
        } else if item.is_wildcard() {
            Some(
                expand_bare_wildcard(&relations, state_bearing).ok_or_else(|| {
                    PresentationRefusal::UnresolvableWildcard {
                        wildcard: sql[item.range()].to_string(),
                    }
                })?,
            )
        } else {
            None
        };

        if let Some(replacement) = replacement {
            let start: usize = item.range().start().into();
            let end: usize = item.range().end().into();
            out.push_str(&sql[last_end..start]);
            out.push_str(&replacement);
            last_end = end;
        }
    }
    out.push_str(&sql[last_end..]);
    Ok(out)
}

fn matches_state_bearing(
    rel: &Relation,
    state_bearing: &std::collections::BTreeMap<String, Vec<String>>,
) -> bool {
    rel.resolved_name
        .as_deref()
        .is_some_and(|n| state_bearing.contains_key(n))
}

fn state_bearing_columns<'a>(
    rel: &Relation,
    state_bearing: &'a std::collections::BTreeMap<String, Vec<String>>,
) -> Option<&'a Vec<String>> {
    rel.resolved_name
        .as_deref()
        .and_then(|n| state_bearing.get(n))
}

/// Expand a bare `*` given the relations in scope. A single relation
/// expands to its bare (unqualified) presented column list; multiple
/// relations expand per-relation — a state-bearing relation to its
/// qualified presented columns, a non-state-bearing (or non-state-bearing-
/// unresolvable) relation to `<qualifier>.*`. `None` means the wildcard
/// cannot be resolved (a relation this walk cannot name, still in scope
/// alongside a state-bearing one) and the caller must refuse.
fn expand_bare_wildcard(
    relations: &[Relation],
    state_bearing: &std::collections::BTreeMap<String, Vec<String>>,
) -> Option<String> {
    if relations.len() == 1 {
        let rel = &relations[0];
        return match state_bearing_columns(rel, state_bearing) {
            Some(cols) => Some(cols.join(", ")),
            // Only reachable if this sole relation isn't state-bearing —
            // but the caller only reaches `expand_bare_wildcard` once it
            // has already established some relation in scope IS
            // state-bearing, and with one relation that must be this one.
            // Kept as a refusal (never a silent unchanged copy) rather
            // than an `unreachable!()`, so a future relaxation of the
            // caller's gate fails loud instead of miscompiling.
            None => None,
        };
    }
    let mut parts = Vec::with_capacity(relations.len());
    for rel in relations {
        match state_bearing_columns(rel, state_bearing) {
            Some(cols) => {
                let qualifier = rel.qualifier.as_deref()?;
                for c in cols {
                    parts.push(format!("{qualifier}.{c}"));
                }
            }
            None => {
                let qualifier = rel.qualifier.as_deref()?;
                parts.push(format!("{qualifier}.*"));
            }
        }
    }
    Some(parts.join(", "))
}

// ── Fingerprint sidecar diff (F3, `docs/plans/20260715-composed-axes-
// conditional-maintenance.md` Phase F3; `docs/specs/sources.md` §"The
// fingerprint sidecar") ────────────────────────────────────────────────
//
// The sidecar's own storage DDL/DML (table creation, the upsert-refresh,
// the GC delete) is warehouse-resident bookkeeping, in the same excluded
// class as the reconciliation ledger and the observed-output-delta record
// (`docs/specs/incremental_models.md` §"Statement emission (single
// owner)"'s third exclusion) — it lives in `smelt_state::ddl_duckdb`, not
// here. The DIFF query below is different: unlike the ledger/observed-delta
// bookkeeping, it is not a record of smelt's own run history — it is the
// derived comparison that decides which source keys count as "changed", the
// same kind of maintenance-relevant computation `emit_column_scoped_merge_
// suppressed`'s `IS DISTINCT FROM` guard is. F3 rules it emitter-authored.

/// Tag prefixed onto a NULL column's pre-image before hashing — see
/// [`column_fingerprint_expr`]. A single-character tag with no column
/// content appended, so its pre-image (`'N'`) can never be reproduced by a
/// real column value's own tagged pre-image (which always starts with
/// [`VALUE_TAG`] instead).
const NULL_TAG: &str = "N";

/// Tag prefixed onto a real (non-NULL) column value's pre-image before
/// hashing — see [`column_fingerprint_expr`]. Distinct from [`NULL_TAG`] so
/// no column value, however it stringifies, can ever collide with the NULL
/// pre-image.
const VALUE_TAG: &str = "V";

/// A single column's fingerprint: `sha256` of a tagged pre-image,
/// `'N'` when the column is NULL, `'V' || CAST(col AS VARCHAR)` otherwise.
/// This is the collision-free replacement for the old
/// `COALESCE(CAST(col AS VARCHAR), sentinel)` scheme: that scheme conflated
/// a real value literally equal to the sentinel string with a true NULL
/// (both coalesced to the identical sentinel text). Here NULL and every
/// real value start from structurally disjoint pre-images — `'N'` alone
/// vs. `'V'` followed by (possibly empty, possibly arbitrary) content — so
/// no column content, whatever it contains, can ever produce the NULL
/// pre-image.
///
/// The output is always a fixed-length 64-character hex string (DuckDB's
/// `sha256()` return shape), which is what lets [`concat_varchar_expr`]
/// join multiple columns' fingerprints with no separator at all — see its
/// own doc comment for why fixed-length concatenation removes the
/// separator-collision hazard structurally rather than by convention.
fn column_fingerprint_expr(column: &str, cast_type: &str) -> String {
    format!(
        "sha256(CASE WHEN {column} IS NULL THEN '{NULL_TAG}' ELSE CONCAT('{VALUE_TAG}', CAST({column} AS {cast_type})) END)"
    )
}

/// A row-content fingerprint over one or more DIGEST columns: always the
/// full collision-free construction, single- or multi-column alike. This is
/// a digest-of-digests: each column is hashed independently first via
/// [`column_fingerprint_expr`] into a FIXED-length (64 hex character)
/// output, and only those fixed-length outputs are concatenated — with no
/// separator, because none is needed. The old scheme joined raw
/// (variable-length) `CAST(... AS VARCHAR)` text with a `\u{1}` separator
/// character; since that separator was not escaped within column content, a
/// column value that itself contained a literal `\u{1}` byte could make two
/// genuinely different multi-column tuples reassemble into the identical
/// joined string (e.g. columns `('John\u{1}Smith', 'X')` and `('John',
/// 'Smith\u{1}X')` joined to the same text). Fixed-length concatenation
/// removes this class of bug structurally: every joined component is
/// exactly 64 characters, so there is no byte position at which one
/// column's content could be misread as spanning into an adjacent column's
/// slot, regardless of what that content is.
///
/// This is safe to use unconditionally for a digest because `delta_digest`
/// is never surfaced to a caller as a literal value — it is only ever
/// compared for equality against another digest computed the same way
/// (`IS DISTINCT FROM`). Contrast [`key_expr_for_columns`], which builds
/// the sidecar's KEY expression and — for a single column — must stay a
/// literal, un-hashed value instead; see that function's own doc comment
/// for why.
fn concat_varchar_expr(columns: &[String]) -> String {
    concat_varchar_expr_typed(columns, "VARCHAR")
}

/// [`concat_varchar_expr`], parameterized over the unsized string-cast type
/// name — DuckDB's `VARCHAR` for every existing (DuckDB-only) caller, or the
/// dialect's own type via [`probe_dialect_string_type`] for
/// [`row_fingerprint_expr`]'s dialect-aware probe caller.
fn concat_varchar_expr_typed(columns: &[String], cast_type: &str) -> String {
    let per_column = columns
        .iter()
        .map(|c| column_fingerprint_expr(c, cast_type))
        .collect::<Vec<_>>()
        .join(", ");
    if columns.len() == 1 {
        // Already a single fixed-length sha256 digest — nothing to join.
        per_column
    } else {
        format!("CONCAT({per_column})")
    }
}

/// A whole-row content fingerprint over `columns`: `sha256` of the
/// per-column digest-of-digests concatenation ([`concat_varchar_expr_typed`])
/// — the same construction [`emit_fingerprint_digest_select`] uses for its
/// `delta_digest` column, factored out so
/// [`emit_append_only_posture_probe`] can build the identical row-content
/// hash without re-authoring the hashing SQL. `dialect` selects the
/// unsized string-cast type ([`probe_dialect_string_type`]) — DuckDB's
/// `VARCHAR` or Spark's `STRING` — so the fingerprint is well-formed under
/// either dialect, unlike [`concat_varchar_expr`]'s DuckDB-only default.
fn row_fingerprint_expr(columns: &[String], dialect: MaintenanceDialect) -> String {
    format!(
        "sha256({})",
        concat_varchar_expr_typed(columns, probe_dialect_string_type(dialect))
    )
}

/// The NULL-key sentinel: a KEY column that is truly NULL is coalesced to
/// this fixed marker purely so `delta_key` never violates the sidecar's
/// `source_key VARCHAR NOT NULL` column. Unlike [`NULL_TAG`]/[`VALUE_TAG`],
/// this is NOT collision-free against an adversarial real value — a real
/// source-key column whose content happened to literally equal this marker
/// would be indistinguishable from a true NULL key. That gap is deliberate
/// and narrower in scope than the digest fix: see [`key_expr_for_columns`].
const KEY_NULL_SENTINEL: &str = "\u{2}NULL\u{2}";

/// Builds the sidecar's KEY expression (`delta_key`) over `columns` — the
/// row's identifying key.
///
/// **Single column: stays a literal, un-hashed value.** Unlike the digest
/// expression, `delta_key` is not an opaque comparison-only token: it is
/// surfaced to callers (`smelt_runtime::maintenance_driver::
/// diff_fingerprint_sidecar_changed_keys`'s returned `Vec<String>`) and
/// consumed downstream as a literal predicate value spliced back against
/// the source's own real key column
/// (`emit_delete_insert_delta_restricted`'s `restrict_column IN
/// (delta_keys)`) — the same literal-value contract
/// `smelt_runtime::maintenance_driver::changed_keys_select`'s own
/// `key_expr` upholds (see that function's doc comment for the parallel
/// case). Hashing a single-column key would silently break every consumer
/// that expects `delta_key` to equal the real key's own text. A NULL key
/// column is coalesced to [`KEY_NULL_SENTINEL`] purely to satisfy the
/// sidecar's `NOT NULL` column — narrower than the digest's fix (source
/// identity keys are not expected to be NULL in practice, and the
/// literal-value contract above forecloses hashing NULL away the way the
/// digest does).
///
/// **Multi-column: gets the full collision-free construction.** A
/// composite key has no literal consumer today — no downstream restriction
/// wiring exists for a composite key (`emit_delete_insert_delta_restricted`'s
/// `restrict_column` is always a single physical column) — so there is no
/// contract to preserve, and the composite-key collision the review flagged
/// (two distinct real composite keys silently overwriting the same sidecar
/// row because their old-scheme joined text collided) is worth closing.
///
/// `pub`, not module-private: the repair family's runtime driver
/// (`smelt_runtime::maintenance_driver`) builds the SAME canonical
/// `delta_key` expression over the model's own group-key columns, for the
/// affected-key relation and its `emit_per_group_recompute` joins
/// (`docs/outcomes/20260809-repair-family/phases/09-plan.md`) — one shape,
/// shared by both the sidecar diff and the append-only clamped-scan path,
/// never a second, independently-typed key expression.
pub fn key_expr_for_columns(columns: &[String]) -> String {
    if columns.len() == 1 {
        format!(
            "COALESCE(CAST({} AS VARCHAR), '{KEY_NULL_SENTINEL}')",
            columns[0]
        )
    } else {
        concat_varchar_expr(columns)
    }
}

/// The row-content digest `SELECT` over an external `mutable_snapshot`
/// source (`docs/specs/sources.md` §"The fingerprint sidecar" — "Digest"):
/// `sha256(...)` over `digest_columns` — the caller's already-resolved P4
/// fingerprint projection (`model_properties.md` §"Fingerprint
/// projection"; the source's FULL column list when the projection failed
/// closed to `FullRow`) — keyed by `source_key`. Pure string construction,
/// matching this module's whole-file convention: the caller resolves which
/// columns to digest and which key columns identify a row; this emitter
/// only builds the SQL.
///
/// `dialect` is accepted for signature symmetry with every other emitter in
/// this module; only the DuckDB shape is built today (`sha256()` is a
/// DuckDB built-in scalar function) — a Spark digest-select variant is
/// unbuilt, matching this phase's DuckDB-only sidecar scope. The runtime
/// caller (`smelt_runtime::maintenance_driver`) gates on the backend's
/// dialect before ever reaching this function, so a Spark target fails
/// loud at that call site rather than being handed DuckDB-flavored SQL.
///
/// # Panics
/// Panics if `source_key` or `digest_columns` is empty — a caller with no
/// key to identify rows by, or nothing to digest, has no business building
/// a sidecar digest select at all.
pub fn emit_fingerprint_digest_select(
    source_table: &str,
    source_key: &[String],
    digest_columns: &[String],
    _dialect: MaintenanceDialect,
) -> String {
    assert!(
        !source_key.is_empty(),
        "emit_fingerprint_digest_select requires a non-empty source key for {source_table}"
    );
    assert!(
        !digest_columns.is_empty(),
        "emit_fingerprint_digest_select requires a non-empty digest column set for {source_table}"
    );
    let key_expr = key_expr_for_columns(source_key);
    let digest_expr = row_fingerprint_expr(digest_columns, MaintenanceDialect::DuckDb);
    format!("SELECT {key_expr} AS delta_key, {digest_expr} AS delta_digest FROM {source_table}")
}

/// The synthesized external change-feed diff (`docs/specs/sources.md`
/// §"The fingerprint sidecar"): compares `source_table`'s CURRENT
/// `(key, digest)` pairs (via [`emit_fingerprint_digest_select`]) against
/// the sidecar's own stored partition for `(source_address,
/// projection_identity)`, producing the changed-key set a `mutable_snapshot`
/// source's otherwise whole-table delta collapses to.
///
/// A `FULL OUTER JOIN` so three shapes all surface as a changed
/// `delta_key`: a source key with no sidecar row (new — or, on a first run
/// against an unpopulated sidecar, EVERY row, which is exactly the
/// whole-table delta the widen-never-narrow default already produces, with
/// no special-casing needed here); a sidecar row with no source key (the
/// source row was deleted — GC's own trigger, reported via
/// `COALESCE(..., sidecar.source_key)`); and a matched pair whose digests
/// differ (`IS DISTINCT FROM`, the same exact-compare shape every other
/// change-suppression guard in this module uses). A matched pair with equal
/// digests is excluded — never surfaced as a false "changed" result, the
/// digest-soundness oracle's negative leg
/// (`docs/plans/20260715-composed-axes-conditional-maintenance.md` Phase
/// F1's ruling; `docs/specs/sources.md` §"The fingerprint sidecar" —
/// "Digest" — "the collision-soundness invariant").
///
/// `sidecar_table` is already fully qualified (`schema.table`);
/// `source_address`/`projection_identity`/`stamp` are plain string values,
/// escaped here (this emitter, like every other, does its own literal
/// quoting — see `emit_delete_insert_delta_restricted`'s `delta_keys`
/// handling for the same pattern).
///
/// `stamp` (Phase F4,
/// `docs/plans/20260715-composed-axes-conditional-maintenance.md` —
/// "Sidecar invalidation") is the freshly computed identity stamp
/// (`smelt_runtime::maintenance_driver::compute_fingerprint_sidecar_stamp`
/// — digest-construction version, this same `projection_identity`, and the
/// consuming model's own SQL provenance combined). The sidecar-side
/// subquery filters on `stamp = '...'` in addition to `source_address`/
/// `projection_identity`: a stored row whose stamp does not match is
/// excluded from the comparison entirely, so it never joins against the
/// current source content — structurally identical to that key having no
/// sidecar row at all. This is the mechanism that makes an invalidated
/// partition (a model-definition edit, a P4 projection change reusing the
/// same identity text is impossible by construction, or a digest-version
/// bump) degrade to exactly the same whole-table delta an absent sidecar
/// already produces above — never a narrower, partially-trusted
/// comparison, and never a silent skip.
#[allow(clippy::too_many_arguments)]
pub fn emit_fingerprint_sidecar_diff(
    source_table: &str,
    source_key: &[String],
    digest_columns: &[String],
    sidecar_table: &str,
    source_address: &str,
    projection_identity: &str,
    stamp: &str,
    dialect: MaintenanceDialect,
) -> String {
    let digest_select =
        emit_fingerprint_digest_select(source_table, source_key, digest_columns, dialect);
    sidecar_diff_over_digest_select(
        &digest_select,
        sidecar_table,
        source_address,
        projection_identity,
        stamp,
    )
}

/// Shared `FULL OUTER JOIN` shape both [`emit_fingerprint_sidecar_diff`]
/// (per-row grain) and [`emit_repair_group_sidecar_diff`] (group grain, P9)
/// build over their own `digest_select` — the comparison logic (three-way
/// new/deleted/changed classification, the stamp filter) is identical at
/// either grain; only what `digest_select` projects one `delta_key`/
/// `delta_digest` pair PER (a source row, or a source-derived output group)
/// differs. See [`emit_fingerprint_sidecar_diff`]'s own doc comment for the
/// full rationale — this helper exists purely to keep that rationale in one
/// place rather than duplicated across two near-identical `format!` bodies.
fn sidecar_diff_over_digest_select(
    digest_select: &str,
    sidecar_table: &str,
    source_address: &str,
    projection_identity: &str,
    stamp: &str,
) -> String {
    let source_address_lit = source_address.replace('\'', "''");
    let projection_identity_lit = projection_identity.replace('\'', "''");
    let stamp_lit = stamp.replace('\'', "''");
    format!(
        "SELECT COALESCE(__smelt_src.delta_key, __smelt_sidecar.source_key) AS delta_key \
         FROM ({digest_select}) AS __smelt_src \
         FULL OUTER JOIN (SELECT source_key, digest FROM {sidecar_table} \
         WHERE source_address = '{source_address_lit}' AND projection_identity = '{projection_identity_lit}' \
         AND stamp = '{stamp_lit}') \
         AS __smelt_sidecar ON __smelt_src.delta_key = __smelt_sidecar.source_key \
         WHERE __smelt_sidecar.source_key IS NULL \
         OR __smelt_src.delta_key IS NULL \
         OR __smelt_src.delta_digest IS DISTINCT FROM __smelt_sidecar.digest"
    )
}

/// The repair family's group-grain digest `SELECT`
/// (`docs/specs/sources.md` §"The fingerprint sidecar" — "Partition grain";
/// `docs/specs/incremental_models.md` §"The repair family" — "Obligation 7
/// over a `mutable_snapshot` source"): one row per `group_key` value,
/// projecting the same canonical `delta_key` expression
/// [`emit_fingerprint_digest_select`] builds ([`key_expr_for_columns`] over
/// `group_key`), paired with an **order-insensitive** digest over that
/// group's own contributing source rows.
///
/// Each contributing row is hashed independently first, via the same
/// tagged, NULL-safe, fixed-length per-row digest [`concat_varchar_expr`]
/// builds for the per-row sidecar (`sha256(...)` over `digest_columns`);
/// `hash(...)` (DuckDB's scalar hash, `UBIGINT`) turns that fixed-length
/// hex digest into an integer, and `bit_xor(...)` combines every row's
/// integer digest within the group — XOR is commutative and associative, so
/// the group's digest does not depend on the order its rows are read in
/// (the same content in a different row order digests identically), while
/// removing, adding, or changing any one row's content still flips bits in
/// the combined result (a collision needs two DISTINCT per-row digest sets
/// to XOR to the same value, no likelier than the per-row sidecar's own
/// assumed SHA-256 collision-soundness invariant `sources.md` §"The
/// fingerprint sidecar" — "Digest" already relies on).
///
/// `dialect` is accepted for signature symmetry with
/// [`emit_fingerprint_digest_select`]; only the DuckDB shape (`sha256`,
/// `hash`, `bit_xor` are all DuckDB built-ins) is built today, matching this
/// phase's DuckDB-only scope.
///
/// # Panics
/// Panics if `group_key` or `digest_columns` is empty — mirrors
/// [`emit_fingerprint_digest_select`]'s own contract.
pub fn emit_repair_group_digest_select(
    source_table: &str,
    group_key: &[String],
    digest_columns: &[String],
    _dialect: MaintenanceDialect,
) -> String {
    assert!(
        !group_key.is_empty(),
        "emit_repair_group_digest_select requires a non-empty group key for {source_table}"
    );
    assert!(
        !digest_columns.is_empty(),
        "emit_repair_group_digest_select requires a non-empty digest column set for \
         {source_table}"
    );
    let key_expr = key_expr_for_columns(group_key);
    let group_by_list = group_key.join(", ");
    let row_digest_expr = concat_varchar_expr(digest_columns);
    format!(
        "SELECT {key_expr} AS delta_key, CAST(bit_xor(hash(sha256({row_digest_expr}))) AS \
         VARCHAR) AS delta_digest FROM {source_table} GROUP BY {group_by_list}"
    )
}

/// The repair family's group-grain counterpart of
/// [`emit_fingerprint_sidecar_diff`] (P9,
/// `docs/specs/incremental_models.md` §"The repair family" — "Obligation 7
/// over a `mutable_snapshot` source"): the same `FULL OUTER JOIN` diff
/// shape, over [`emit_repair_group_digest_select`]'s group-grain digest
/// instead of the per-row one — so a group whose entire contribution
/// departed the source still surfaces via the diff's "sidecar row with no
/// matching source key" leg (`__smelt_src.delta_key IS NULL`), even though
/// no source row survives to name it.
#[allow(clippy::too_many_arguments)]
pub fn emit_repair_group_sidecar_diff(
    source_table: &str,
    group_key: &[String],
    digest_columns: &[String],
    sidecar_table: &str,
    source_address: &str,
    projection_identity: &str,
    stamp: &str,
    dialect: MaintenanceDialect,
) -> String {
    let digest_select =
        emit_repair_group_digest_select(source_table, group_key, digest_columns, dialect);
    sidecar_diff_over_digest_select(
        &digest_select,
        sidecar_table,
        source_address,
        projection_identity,
        stamp,
    )
}

/// A key-addressed model edge's affected-keys relation
/// (`docs/specs/incremental_models.md` §"Upstream model edges"): the
/// downstream's own key columns (`KeyScope::keys`), distinct, for every
/// upstream row whose own `KeyedUpsert` key (`upstream_keys`) is one of the
/// already-resolved `changed_keys` — the changed-key set the group-grain
/// fingerprint sidecar diff over the upstream's output table discovered
/// (`diff_repair_group_sidecar_changed_keys`, `smelt-runtime`). This is the
/// key-correspondence projection: an upstream key that changed does not
/// necessarily equal the downstream's own key column set, so the relation
/// re-selects the downstream's key expression over the upstream table rather
/// than reusing the changed keys directly.
///
/// Same `key_expr_for_columns` canonicalisation
/// [`repair_affected_keys_select`]/[`repair_candidate_select`] (`smelt-
/// runtime`) use for the resulting `delta_key` column, so this relation
/// composes into the same repair-family candidate-select/write emitters
/// unchanged — only how the affected-key relation itself is discovered
/// differs from the ordinary clamped-scan repair path.
///
/// `changed_keys` is a literal `VARCHAR` value list (already resolved by the
/// caller's sidecar-diff read, not opaque SQL) — the same shape
/// [`super::super::maintenance_driver`]'s `repair_keys_literal_select`-style
/// callers pass. An empty `changed_keys` yields a well-typed EMPTY relation
/// (`WHERE FALSE`), never an unrestricted `SELECT DISTINCT`: a run
/// discovering no changed upstream keys touches nothing.
///
/// `dialect` is accepted for signature symmetry with this module's other
/// repair-family emitters; only the DuckDB shape is built today, matching
/// this phase's DuckDB-only discovery-route scope.
///
/// # Panics
/// Panics if `upstream_keys` or `downstream_keys` is empty — mirrors
/// [`emit_repair_group_digest_select`]'s own contract.
pub fn emit_key_addressed_affected_keys_select(
    upstream_table: &str,
    upstream_keys: &[String],
    downstream_keys: &[String],
    changed_keys: &[String],
    _dialect: MaintenanceDialect,
) -> String {
    assert!(
        !upstream_keys.is_empty(),
        "emit_key_addressed_affected_keys_select requires a non-empty upstream key for \
         {upstream_table}"
    );
    assert!(
        !downstream_keys.is_empty(),
        "emit_key_addressed_affected_keys_select requires a non-empty downstream key for \
         {upstream_table}"
    );
    let downstream_key_expr = key_expr_for_columns(downstream_keys);
    if changed_keys.is_empty() {
        return format!(
            "SELECT {downstream_key_expr} AS delta_key FROM {upstream_table} WHERE FALSE"
        );
    }
    let upstream_key_expr = key_expr_for_columns(upstream_keys);
    let literals = changed_keys
        .iter()
        .map(|k| format!("'{}'", k.replace('\'', "''")))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "SELECT DISTINCT {downstream_key_expr} AS delta_key FROM {upstream_table} WHERE \
         {upstream_key_expr} IN ({literals})"
    )
}

#[cfg(test)]
mod fingerprint_sidecar_tests {
    use super::*;

    /// Run `emit_fingerprint_digest_select` for a single-row `source_table`
    /// (a derived-table expression, e.g. `(SELECT 1 AS id, 'x' AS val)")
    /// against a real DuckDB and return the resulting `delta_digest` value
    /// — used by the two collision-regression tests below to prove the
    /// FIX's actual SQL output against real DuckDB semantics (NULL
    /// handling, `chr()`, `CONCAT`), not merely against string-literal
    /// expectations of what DuckDB is assumed to do.
    fn digest_for_source(
        conn: &duckdb::Connection,
        source_table: &str,
        source_key: &[String],
        digest_columns: &[String],
    ) -> String {
        let sql = emit_fingerprint_digest_select(
            source_table,
            source_key,
            digest_columns,
            MaintenanceDialect::DuckDb,
        );
        conn.query_row(&sql, [], |row| row.get::<_, String>(1))
            .expect("digest select query")
    }

    /// Same as [`digest_for_source`] but returns the `delta_key` column
    /// instead — used by the composite-source-key collision regression
    /// test below.
    fn key_for_source(
        conn: &duckdb::Connection,
        source_table: &str,
        source_key: &[String],
        digest_columns: &[String],
    ) -> String {
        let sql = emit_fingerprint_digest_select(
            source_table,
            source_key,
            digest_columns,
            MaintenanceDialect::DuckDb,
        );
        conn.query_row(&sql, [], |row| row.get::<_, String>(0))
            .expect("key select query")
    }

    #[test]
    fn digest_select_single_column_key_and_digest() {
        let sql = emit_fingerprint_digest_select(
            "raw.dim_users",
            &["user_id".to_string()],
            &["name".to_string()],
            MaintenanceDialect::DuckDb,
        );
        // The single-column KEY stays literal (`COALESCE(CAST(... AS
        // VARCHAR), sentinel)`) — it is surfaced downstream as a real
        // predicate value, unlike the digest, which is always the full
        // tagged-hash construction (see `key_expr_for_columns`'s doc
        // comment for why the two differ).
        assert_eq!(
            sql,
            "SELECT COALESCE(CAST(user_id AS VARCHAR), '\u{2}NULL\u{2}') AS delta_key, \
             sha256(sha256(CASE WHEN name IS NULL THEN 'N' ELSE CONCAT('V', CAST(name AS \
             VARCHAR)) END)) AS delta_digest FROM raw.dim_users"
        );
    }

    #[test]
    fn digest_select_multi_column_key_and_digest_concatenates() {
        let sql = emit_fingerprint_digest_select(
            "raw.dim_users",
            &["tenant_id".to_string(), "user_id".to_string()],
            &["name".to_string(), "tier".to_string()],
            MaintenanceDialect::DuckDb,
        );
        // Each column is hashed to a fixed-length digest FIRST, and only
        // those fixed-length digests are concatenated — no separator, since
        // fixed-length components have no boundary to confuse.
        assert!(sql.contains(
            "CONCAT(sha256(CASE WHEN tenant_id IS NULL THEN 'N' ELSE CONCAT('V', CAST(tenant_id \
             AS VARCHAR)) END), sha256(CASE WHEN user_id IS NULL THEN 'N' ELSE CONCAT('V', CAST(\
             user_id AS VARCHAR)) END)) AS delta_key"
        ));
        assert!(sql.contains(
            "sha256(CONCAT(sha256(CASE WHEN name IS NULL THEN 'N' ELSE CONCAT('V', CAST(name AS \
             VARCHAR)) END), sha256(CASE WHEN tier IS NULL THEN 'N' ELSE CONCAT('V', CAST(tier \
             AS VARCHAR)) END))) AS delta_digest"
        ));
    }

    /// Regression for the NULL-vs-empty-string digest collision (the first
    /// bug this fingerprint scheme had): DuckDB's `CONCAT` silently drops
    /// NULL arguments, so before any fix at all, `CONCAT(NULL, sep, 'x')`
    /// and `CONCAT('', sep, 'x')` produced the identical string (and
    /// therefore the identical digest) — a false-negative "unchanged"
    /// verdict for a row whose projected value went from empty string to
    /// NULL (or vice versa). The tagged pre-image construction rules this
    /// out structurally: a NULL column's pre-image is the bare tag `'N'`,
    /// disjoint from EVERY real value's pre-image (`'V' || content`,
    /// including the empty string, `'V'`), so the two can never coincide.
    #[test]
    fn digest_select_distinguishes_null_from_empty_string_in_multi_column_projection() {
        let sql = emit_fingerprint_digest_select(
            "raw.dim_users",
            &["user_id".to_string()],
            &["name".to_string(), "tier".to_string()],
            MaintenanceDialect::DuckDb,
        );
        // Each column branches on its own NULL-ness independently, so a
        // NULL `name` renders as the `'N'` tag branch, structurally
        // distinct from the `'V'`-tagged empty-string branch — not simply
        // vanishing from a CONCAT the way a dropped NULL argument would.
        assert!(sql.contains(
            "CASE WHEN name IS NULL THEN 'N' ELSE CONCAT('V', CAST(name AS VARCHAR)) END"
        ));
        assert!(sql.contains(
            "CASE WHEN tier IS NULL THEN 'N' ELSE CONCAT('V', CAST(tier AS VARCHAR)) END"
        ));
    }

    /// Regression for the NULL-digest crash (the second bug this
    /// fingerprint scheme had): before any fix, a single-column projection
    /// built `sha256(CAST(col AS VARCHAR))` directly, so a NULL projected
    /// value produced `sha256(NULL) = NULL` in DuckDB — which then violated
    /// the sidecar's `NOT NULL digest` column constraint on upsert. The
    /// emitted digest expression must never let a NULL value reach
    /// `sha256` un-tagged, for both the single- and multi-column shapes.
    #[test]
    fn digest_select_single_column_never_feeds_sha256_a_bare_null() {
        let single = emit_fingerprint_digest_select(
            "raw.dim_users",
            &["user_id".to_string()],
            &["name".to_string()],
            MaintenanceDialect::DuckDb,
        );
        assert!(single.contains(
            "sha256(CASE WHEN name IS NULL THEN 'N' ELSE CONCAT('V', CAST(name AS VARCHAR)) END)"
        ));
        assert!(!single.contains("sha256(CAST(name AS VARCHAR))"));

        let multi = emit_fingerprint_digest_select(
            "raw.dim_users",
            &["user_id".to_string()],
            &["name".to_string(), "tier".to_string()],
            MaintenanceDialect::DuckDb,
        );
        // Every column reaching the hash must be wrapped in the tagged
        // CASE — none of the bare `CAST(... AS VARCHAR)` forms may appear
        // unwrapped, and no raw `CONCAT(CAST(` (the old un-hashed
        // multi-column join shape) may appear either.
        assert!(!multi.contains("sha256(CAST("));
        assert!(!multi.contains("CONCAT(CAST("));
    }

    /// Regression for the separator-collision bug found in a follow-up
    /// review: the earlier fix joined raw (unescaped) column text with a
    /// `\u{1}` separator, so a column value that itself contained a literal
    /// `\u{1}` byte could make two DISTINCT multi-column tuples reassemble
    /// into the identical joined string before hashing —
    /// `('John\u{1}Smith', 'X')` and `('John', 'Smith\u{1}X')` both joined
    /// to `John\u{1}Smith\u{1}X`. Confirmed empirically against a real
    /// DuckDB: this computes the ACTUAL digest SQL's result for both
    /// tuples and asserts they differ, proving the fixed-length
    /// digest-of-digests construction never lets one column's content
    /// bleed across a boundary into another, regardless of what that
    /// content contains.
    #[test]
    fn digest_distinguishes_tuples_that_collided_under_the_old_separator_scheme() {
        let conn = duckdb::Connection::open_in_memory().expect("open in-memory duckdb");
        let source_key = vec!["id".to_string()];
        let digest_columns = vec!["name".to_string(), "tier".to_string()];

        // Tuple A: `name` contains a literal SOH (`\u{1}`) byte before
        // "Smith".
        let digest_a = digest_for_source(
            &conn,
            "(SELECT 1 AS id, 'John' || chr(1) || 'Smith' AS name, 'X' AS tier)",
            &source_key,
            &digest_columns,
        );
        // Tuple B: a DIFFERENT (name, tier) pair whose old-scheme joined
        // text was byte-identical to tuple A's:
        // `'John' + SEP + 'Smith' + SEP + 'X'`.
        let digest_b = digest_for_source(
            &conn,
            "(SELECT 2 AS id, 'John' AS name, 'Smith' || chr(1) || 'X' AS tier)",
            &source_key,
            &digest_columns,
        );

        assert_ne!(
            digest_a, digest_b,
            "two genuinely different (name, tier) tuples must never hash identically, even when \
             a column's own content contains the old separator byte"
        );
    }

    /// Regression for the sentinel-collision bug found in the same
    /// follow-up review: the earlier fix coalesced a NULL column to the
    /// fixed sentinel string `\u{2}NULL\u{2}`, so a REAL column value that
    /// happened to literally equal that sentinel text was indistinguishable
    /// from a true NULL of the same row shape. Confirmed empirically
    /// against a real DuckDB: computes the actual digest for a true-NULL
    /// row and for a row whose value is literally the old sentinel text,
    /// and asserts they differ.
    #[test]
    fn digest_distinguishes_a_real_value_equal_to_the_old_sentinel_from_a_true_null() {
        let conn = duckdb::Connection::open_in_memory().expect("open in-memory duckdb");
        let source_key = vec!["id".to_string()];
        let digest_columns = vec!["val".to_string()];

        let digest_null = digest_for_source(
            &conn,
            "(SELECT 1 AS id, CAST(NULL AS VARCHAR) AS val)",
            &source_key,
            &digest_columns,
        );
        let digest_sentinel_lookalike = digest_for_source(
            &conn,
            "(SELECT 2 AS id, (chr(2) || 'NULL' || chr(2)) AS val)",
            &source_key,
            &digest_columns,
        );

        assert_ne!(
            digest_null, digest_sentinel_lookalike,
            "a real column value equal to the old NULL sentinel must never hash identically to a \
             true NULL"
        );
    }

    /// Regression for the composite `source_key` half of the
    /// separator-collision bug: [`key_expr_for_columns`] reuses
    /// [`concat_varchar_expr`] for a MULTI-column key (only a single-column
    /// key stays literal — see that function's doc comment), so a composite
    /// key is exposed to the exact same old-scheme collision the digest
    /// was. This is a real correctness hazard beyond a false "unchanged"
    /// verdict: two distinct real composite source keys reassembling to the
    /// SAME `delta_key` string would conflate onto the SAME sidecar row
    /// (`source_key` is part of the sidecar's own primary key), silently
    /// overwriting one key's stored digest with the other's. Confirmed
    /// empirically against a real DuckDB: computes the actual `delta_key`
    /// for two engineered-to-collide composite keys and asserts they
    /// differ.
    #[test]
    fn composite_source_key_distinguishes_tuples_that_collided_under_the_old_separator_scheme() {
        let conn = duckdb::Connection::open_in_memory().expect("open in-memory duckdb");
        let source_key = vec!["tenant".to_string(), "user".to_string()];
        let digest_columns = vec!["val".to_string()];

        let key_a = key_for_source(
            &conn,
            "(SELECT 'John' || chr(1) || 'Smith' AS tenant, 'X' AS user, 'v' AS val)",
            &source_key,
            &digest_columns,
        );
        let key_b = key_for_source(
            &conn,
            "(SELECT 'John' AS tenant, 'Smith' || chr(1) || 'X' AS user, 'v' AS val)",
            &source_key,
            &digest_columns,
        );

        assert_ne!(
            key_a, key_b,
            "two genuinely different composite source keys must never produce the same \
             delta_key, even when a key column's own content contains the old separator byte"
        );
    }

    #[test]
    #[should_panic(expected = "non-empty source key")]
    fn digest_select_panics_on_empty_source_key() {
        emit_fingerprint_digest_select(
            "raw.dim_users",
            &[],
            &["name".to_string()],
            MaintenanceDialect::DuckDb,
        );
    }

    #[test]
    #[should_panic(expected = "non-empty digest column set")]
    fn digest_select_panics_on_empty_digest_columns() {
        emit_fingerprint_digest_select(
            "raw.dim_users",
            &["user_id".to_string()],
            &[],
            MaintenanceDialect::DuckDb,
        );
    }

    #[test]
    fn sidecar_diff_full_outer_joins_source_against_sidecar_partition() {
        let sql = emit_fingerprint_sidecar_diff(
            "raw.dim_users",
            &["user_id".to_string()],
            &["name".to_string(), "tier".to_string()],
            "main._smelt_fingerprint_sidecar",
            "smelt.sources.dim_users",
            "cols:name,tier",
            "v1:cols:name,tier:sha256:deadbeef",
            MaintenanceDialect::DuckDb,
        );
        assert!(sql.contains("FULL OUTER JOIN"));
        assert!(sql.contains("FROM main._smelt_fingerprint_sidecar"));
        assert!(sql.contains("source_address = 'smelt.sources.dim_users'"));
        assert!(sql.contains("projection_identity = 'cols:name,tier'"));
        assert!(sql.contains("stamp = 'v1:cols:name,tier:sha256:deadbeef'"));
        assert!(sql.contains("__smelt_src.delta_digest IS DISTINCT FROM __smelt_sidecar.digest"));
        assert!(sql.contains("__smelt_sidecar.source_key IS NULL"));
        assert!(sql.contains("__smelt_src.delta_key IS NULL"));
        assert!(sql.contains(
            "SELECT COALESCE(CAST(user_id AS VARCHAR), '\u{2}NULL\u{2}') AS delta_key, \
             sha256(CONCAT(sha256(CASE WHEN name IS NULL THEN 'N' ELSE CONCAT('V', CAST(name AS \
             VARCHAR)) END), sha256(CASE WHEN tier IS NULL THEN 'N' ELSE CONCAT('V', CAST(tier \
             AS VARCHAR)) END))) AS delta_digest FROM raw.dim_users"
        ));
    }

    #[test]
    fn sidecar_diff_escapes_single_quotes_in_literals() {
        let sql = emit_fingerprint_sidecar_diff(
            "raw.dim_users",
            &["user_id".to_string()],
            &["name".to_string()],
            "main._smelt_fingerprint_sidecar",
            "smelt.sources.dim's_users",
            "cols:name",
            "stamp's",
            MaintenanceDialect::DuckDb,
        );
        assert!(sql.contains("source_address = 'smelt.sources.dim''s_users'"));
        assert!(sql.contains("stamp = 'stamp''s'"));
    }

    /// Phase F4 (`docs/plans/20260715-composed-axes-conditional-maintenance.md`
    /// — "Sidecar invalidation"): a stale-stamped row must never be joined
    /// against the current source content — the `stamp = '...'` filter
    /// excludes it from the sidecar-side subquery regardless of
    /// `source_address`/`projection_identity` matching, structurally
    /// identical to that key having no sidecar row at all.
    #[test]
    fn sidecar_diff_stamp_filter_excludes_mismatched_rows_from_the_comparison() {
        let sql = emit_fingerprint_sidecar_diff(
            "raw.dim_users",
            &["user_id".to_string()],
            &["name".to_string()],
            "main._smelt_fingerprint_sidecar",
            "smelt.sources.dim_users",
            "cols:name",
            "v2:cols:name:sha256:newhash",
            MaintenanceDialect::DuckDb,
        );
        // The sidecar-side subquery must filter on the CURRENT stamp only —
        // a row stamped under any other value is never a candidate match.
        assert!(sql.contains(
            "WHERE source_address = 'smelt.sources.dim_users' AND projection_identity = \
             'cols:name' AND stamp = 'v2:cols:name:sha256:newhash'"
        ));
    }

    /// Run [`emit_repair_group_digest_select`] over `source_table` (a
    /// derived-table expression) against a real DuckDB and return the
    /// `(delta_key, delta_digest)` pairs it produces, sorted by key — used
    /// by the group-digest order-insensitivity and vanished-group tests
    /// below to prove the FIX's actual SQL output against real DuckDB
    /// semantics, not merely string-literal expectations.
    fn group_digests(
        conn: &duckdb::Connection,
        source_table: &str,
        group_key: &[String],
        digest_columns: &[String],
    ) -> Vec<(String, String)> {
        let sql = emit_repair_group_digest_select(
            source_table,
            group_key,
            digest_columns,
            MaintenanceDialect::DuckDb,
        );
        let mut stmt = conn.prepare(&sql).expect("prepare group digest select");
        let mut rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .expect("query group digest select")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect group digest rows");
        rows.sort();
        rows
    }

    /// P9 test 1 (`docs/outcomes/20260809-repair-family/phases/09-plan.md`):
    /// the group digest is an order-insensitive aggregate — inserting the
    /// same group's rows in a different order must not change its digest —
    /// while removing one of the group's rows must.
    #[test]
    fn repair_group_digest_select_is_order_insensitive() {
        let conn = duckdb::Connection::open_in_memory().expect("open in-memory duckdb");
        let group_key = vec!["customer_id".to_string()];
        let digest_columns = vec!["amount".to_string()];

        let forward = group_digests(
            &conn,
            "(SELECT * FROM (VALUES (1, 10), (1, 20), (1, 30)) AS t(customer_id, amount))",
            &group_key,
            &digest_columns,
        );
        let shuffled = group_digests(
            &conn,
            "(SELECT * FROM (VALUES (1, 30), (1, 10), (1, 20)) AS t(customer_id, amount))",
            &group_key,
            &digest_columns,
        );
        assert_eq!(
            forward, shuffled,
            "the same group's rows in a different order must digest identically"
        );

        let one_row_deleted = group_digests(
            &conn,
            "(SELECT * FROM (VALUES (1, 10), (1, 20)) AS t(customer_id, amount))",
            &group_key,
            &digest_columns,
        );
        assert_ne!(
            forward, one_row_deleted,
            "deleting one of the group's rows must change its digest"
        );
    }

    /// P9 test 2: the group-grain sidecar diff over a group-grain partition
    /// reports a group present in the sidecar and absent from the source —
    /// the `__smelt_src.delta_key IS NULL` leg — with the vanished group's
    /// key value intact, proving a wholly-deleted group is still
    /// discoverable via the stored comparandum.
    #[test]
    fn repair_group_digest_diff_reports_a_vanished_group() {
        let conn = duckdb::Connection::open_in_memory().expect("open in-memory duckdb");
        let source_table = "(SELECT * FROM (VALUES (1, 10)) AS t(customer_id, amount))";
        let group_key = vec!["customer_id".to_string()];
        let digest_columns = vec!["amount".to_string()];

        // Customer 1's digest under the CURRENT source content — seeded
        // into the sidecar as an already-matching comparandum, so it must
        // NOT surface as changed; only customer 2 (present in the sidecar,
        // absent from the source) should.
        let customer_1_digest = group_digests(&conn, source_table, &group_key, &digest_columns)
            .into_iter()
            .find(|(key, _)| key == "1")
            .expect("customer 1's digest")
            .1;
        conn.execute_batch(&format!(
            "CREATE TABLE sidecar (source_address VARCHAR, projection_identity VARCHAR, \
             source_key VARCHAR, digest VARCHAR, stamp VARCHAR); \
             INSERT INTO sidecar VALUES \
             ('src', 'repair:group=customer_id:digest=amount', '1', '{customer_1_digest}', \
             'stamp1'), \
             ('src', 'repair:group=customer_id:digest=amount', '2', \
             'stale-digest-for-vanished-group', 'stamp1');"
        ))
        .expect("seed sidecar: customer 1 matches current content, customer 2 has vanished");

        let sql = emit_repair_group_sidecar_diff(
            source_table,
            &group_key,
            &digest_columns,
            "sidecar",
            "src",
            "repair:group=customer_id:digest=amount",
            "stamp1",
            MaintenanceDialect::DuckDb,
        );
        let mut stmt = conn.prepare(&sql).expect("prepare group sidecar diff");
        let keys: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .expect("query group sidecar diff")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect diff keys");
        assert_eq!(
            keys,
            vec!["2".to_string()],
            "customer 2's group vanished entirely from the source — the diff must still report \
             it, sourced from the sidecar's own stored comparandum, while customer 1's unchanged \
             group must not surface: {keys:?}"
        );
    }
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
            MaintenanceDialect::DuckDb,
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

#[cfg(test)]
mod staged_candidate_conditional_tests {
    use super::*;

    /// The staged-candidate group emits exactly the five statements in
    /// order: temp-relation CREATE, candidate INSERT, conditional
    /// DELETE+INSERT reading the staged relation with an `IS DISTINCT FROM`
    /// restriction, DROP — flagged one-transaction.
    #[test]
    fn staged_group_emits_ordered_statements_as_one_transaction() {
        let group = emit_staged_candidate_conditional(
            "main.dim_users",
            "__smelt_staged_dim_users",
            &["user_id".to_string()],
            "SELECT user_id, tier, email FROM source_delta",
            &["tier".to_string(), "email".to_string()],
            MaintenanceDialect::DuckDb,
        );

        assert!(group.transactional);
        assert_eq!(group.statements.len(), 5);
        assert_eq!(
            group.statements[0].sql,
            "CREATE TEMP TABLE __smelt_staged_dim_users AS SELECT * FROM (SELECT user_id, tier, \
             email FROM source_delta) AS __smelt_staged_shape LIMIT 0"
        );
        assert_eq!(
            group.statements[1].sql,
            "INSERT INTO __smelt_staged_dim_users SELECT user_id, tier, email FROM source_delta"
        );
        assert_eq!(
            group.statements[2].sql,
            "DELETE FROM main.dim_users USING __smelt_staged_dim_users WHERE main.dim_users.user_id \
             = __smelt_staged_dim_users.user_id AND (main.dim_users.tier IS DISTINCT FROM \
             __smelt_staged_dim_users.tier OR main.dim_users.email IS DISTINCT FROM \
             __smelt_staged_dim_users.email)"
        );
        assert_eq!(
            group.statements[3].sql,
            "INSERT INTO main.dim_users SELECT s.* FROM __smelt_staged_dim_users AS s WHERE NOT \
             EXISTS (SELECT 1 FROM main.dim_users AS t WHERE t.user_id = s.user_id)"
        );
        assert_eq!(
            group.statements[4].sql,
            "DROP TABLE __smelt_staged_dim_users"
        );
    }

    #[test]
    #[should_panic(expected = "requires a non-empty row identity")]
    fn staged_group_panics_on_empty_key() {
        emit_staged_candidate_conditional(
            "main.dim_users",
            "__smelt_staged_dim_users",
            &[],
            "SELECT user_id, tier FROM source_delta",
            &["tier".to_string()],
            MaintenanceDialect::DuckDb,
        );
    }

    #[test]
    #[should_panic(expected = "requires a non-empty compared-column set")]
    fn staged_group_panics_on_empty_compare_set() {
        emit_staged_candidate_conditional(
            "main.dim_users",
            "__smelt_staged_dim_users",
            &["user_id".to_string()],
            "SELECT user_id, tier FROM source_delta",
            &[],
            MaintenanceDialect::DuckDb,
        );
    }
}

#[cfg(test)]
mod delta_restricted_delete_insert_tests {
    use super::*;

    fn region() -> Region {
        Region {
            start: "'2026-07-01'".to_string(),
            end: "'2026-07-02'".to_string(),
        }
    }

    #[test]
    fn restricted_group_gains_the_semi_join_on_both_statements() {
        let group = emit_delete_insert_delta_restricted(
            "main.events_enriched",
            "event_date",
            &region(),
            "SELECT event_id, event_date, session_id FROM events_enriched_recompute",
            "event_id",
            &["ev-1".to_string(), "ev-2".to_string()],
            MaintenanceDialect::DuckDb,
        );

        assert!(group.transactional);
        assert_eq!(group.statements.len(), 2);
        assert_eq!(
            group.statements[0].sql,
            "DELETE FROM main.events_enriched WHERE event_date >= '2026-07-01' AND event_date < \
             '2026-07-02' AND event_id IN ('ev-1', 'ev-2')"
        );
        assert_eq!(
            group.statements[1].sql,
            "INSERT INTO main.events_enriched SELECT * FROM (SELECT event_id, event_date, \
             session_id FROM events_enriched_recompute) AS _smelt_delta_scope WHERE \
             _smelt_delta_scope.event_id IN ('ev-1', 'ev-2')"
        );
    }

    #[test]
    fn restricted_group_escapes_single_quotes_in_delta_keys() {
        let group = emit_delete_insert_delta_restricted(
            "main.t",
            "d",
            &region(),
            "SELECT 1",
            "k",
            &["o'brien".to_string()],
            MaintenanceDialect::DuckDb,
        );
        assert!(group.statements[0].sql.contains("k IN ('o''brien')"));
    }

    #[test]
    #[should_panic(expected = "requires a non-empty delta key set")]
    fn restricted_group_panics_on_empty_delta_keys() {
        emit_delete_insert_delta_restricted(
            "main.t",
            "d",
            &region(),
            "SELECT 1",
            "k",
            &[],
            MaintenanceDialect::DuckDb,
        );
    }

    /// The fallback path (`Open` closure or no recorded delta) must call
    /// `emit_delete_insert` directly rather than this function — asserting
    /// the two are byte-identical when the same inputs would otherwise
    /// apply (E3 review checklist: "either absent -> byte-identical
    /// unrestricted statement").
    #[test]
    fn unrestricted_fallback_is_byte_identical_to_emit_delete_insert() {
        let body = "SELECT event_id, event_date FROM events_enriched_recompute";
        let restricted_fallback = emit_delete_insert(
            "main.events_enriched",
            "event_date",
            &region(),
            body,
            MaintenanceDialect::DuckDb,
        );
        let direct = emit_delete_insert(
            "main.events_enriched",
            "event_date",
            &region(),
            body,
            MaintenanceDialect::DuckDb,
        );
        assert_eq!(restricted_fallback, direct);
    }
}

#[cfg(test)]
mod count_preservation_probe_tests {
    use super::*;

    #[test]
    fn probe_compares_driving_and_enriched_row_counts() {
        let probe = emit_count_preservation_probe(
            "SELECT event_id FROM main.raw_events WHERE event_date >= '2026-07-01'",
            "SELECT e.event_id FROM main.raw_events e JOIN main.raw_users u ON e.user_id = \
             u.user_id WHERE e.event_date >= '2026-07-01'",
        );
        assert_eq!(
            probe.sql,
            "SELECT (SELECT COUNT(*) FROM (SELECT event_id FROM main.raw_events WHERE \
             event_date >= '2026-07-01') AS __smelt_driving) AS driving_count, (SELECT \
             COUNT(*) FROM (SELECT e.event_id FROM main.raw_events e JOIN main.raw_users u ON \
             e.user_id = u.user_id WHERE e.event_date >= '2026-07-01') AS __smelt_enriched) AS \
             enriched_count"
        );
    }
}

#[cfg(test)]
mod per_group_recompute_tests {
    use super::*;

    fn key() -> Vec<String> {
        vec!["customer_id".to_string()]
    }

    #[test]
    fn emit_per_group_recompute_deletes_affected_keys_and_inserts_slice_recompute() {
        let group = emit_per_group_recompute(
            "main.customer_totals",
            "__staged",
            &key(),
            "SELECT customer_id FROM delta",
            "SELECT customer_id, SUM(amount) AS total FROM orders_slice GROUP BY customer_id",
            MaintenanceDialect::DuckDb,
        );
        assert!(group.transactional);
        let sqls: Vec<&str> = group.statements.iter().map(|s| s.sql.as_str()).collect();
        assert_eq!(sqls.len(), 5);
        assert!(sqls[0].starts_with("CREATE TEMP TABLE __staged AS SELECT * FROM"));
        assert!(sqls[1].starts_with("INSERT INTO __staged SELECT customer_id, SUM(amount)"));
        assert!(
            sqls[2].starts_with("DELETE FROM main.customer_totals USING"),
            "{}",
            sqls[2]
        );
        assert!(
            sqls[3].starts_with("INSERT INTO main.customer_totals SELECT s.* FROM __staged"),
            "{}",
            sqls[3]
        );
        assert_eq!(sqls[4], "DROP TABLE __staged");
    }

    #[test]
    fn emit_per_group_recompute_is_key_restricted() {
        let group = emit_per_group_recompute(
            "main.customer_totals",
            "__staged",
            &key(),
            "SELECT customer_id FROM delta",
            "SELECT customer_id, SUM(amount) AS total FROM orders_slice GROUP BY customer_id",
            MaintenanceDialect::DuckDb,
        );
        let delete = &group.statements[2].sql;
        let insert = &group.statements[3].sql;
        assert!(
            delete.contains("__smelt_affected") && delete.contains("customer_id"),
            "DELETE must be predicated on the affected-key relation: {delete}"
        );
        assert!(
            insert.contains("__smelt_affected") && insert.contains("customer_id"),
            "INSERT must be predicated on the affected-key relation: {insert}"
        );
        for sql in [delete.as_str(), insert.as_str()] {
            assert!(
                sql.contains("__smelt_affected"),
                "every write statement touching main.customer_totals must be restricted to the \
                 affected-key relation, never unrestricted: {sql}"
            );
        }
    }

    #[test]
    fn emit_per_group_recompute_repeats_identically() {
        let build = || {
            emit_per_group_recompute(
                "main.customer_totals",
                "__staged",
                &key(),
                "SELECT customer_id FROM delta",
                "SELECT customer_id, SUM(amount) AS total FROM orders_slice GROUP BY customer_id",
                MaintenanceDialect::DuckDb,
            )
        };
        assert_eq!(build(), build());
    }

    #[test]
    #[should_panic(expected = "requires a non-empty row identity")]
    fn emit_per_group_recompute_panics_on_empty_key() {
        emit_per_group_recompute(
            "main.customer_totals",
            "__staged",
            &[],
            "SELECT customer_id FROM delta",
            "SELECT customer_id FROM orders_slice",
            MaintenanceDialect::DuckDb,
        );
    }
}
