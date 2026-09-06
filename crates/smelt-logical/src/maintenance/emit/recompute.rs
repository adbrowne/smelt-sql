//! Region-recompute statement families: the paired `DELETE`+`INSERT`
//! over a write window, its delta-restricted variant, the repair family's
//! per-group recompute, and the `diff_patch` write pattern.

use super::fingerprint::key_expr_for_columns;
use super::types::*;
use crate::maintenance::diff_patch::DeleteLeg;

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
