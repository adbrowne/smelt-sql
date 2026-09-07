//! The staged-candidate conditional `DELETE`+`INSERT` families — the
//! merge-less realisation of the no-op-write-elimination licence, in keyed,
//! recompute and keyless shapes.

use super::types::*;

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

/// The staged-candidate conditional `DELETE`+`INSERT`, **keyless (whole-row)**
/// variant (`docs/outcomes/20260815-definition-delta-migrate/phases/27c-plan.md`):
/// the [`RowIdentity::WholeRow`] realisation [`emit_staged_candidate_conditional`]'s
/// own doc comment names as unbuilt. Without a proven row identity there is no
/// row address a multiset difference could delete with multiplicity in
/// portable SQL, so suppression here is **region-grained, not row-grained**:
/// a two-way `EXCEPT ALL` diff between stored state (optionally restricted by
/// `region_predicate`) and the staged candidate is materialised once, before
/// either write statement runs, into a 1-row-max sentinel relation; the
/// region's own unconditional `DELETE`+`INSERT` is then guarded on that
/// sentinel being non-empty. Diff empty ⇒ neither write statement touches a
/// row; diff non-empty ⇒ byte-identical to the unconditional region rewrite
/// (`docs/specs/model_transforms.md` §"Change-suppressed MERGE and the
/// staged-candidate conditional DELETE+INSERT").
///
/// The sentinel is a `CREATE TEMP TABLE ... AS SELECT ...`, not an `EXISTS`
/// re-evaluated after the `DELETE` — evaluating the diff after the target has
/// already been mutated is order-dependent reasoning; a materialised sentinel
/// computed up front is not.
///
/// 1. `CREATE TEMP TABLE <staged_relation> AS <candidate_select> LIMIT 0`
/// 2. `INSERT INTO <staged_relation> <candidate_select>`
/// 3. `CREATE TEMP TABLE <sentinel_relation> AS SELECT 1 FROM ((stored EXCEPT
///    ALL staged) UNION ALL (staged EXCEPT ALL stored)) LIMIT 1` — the stored
///    side carries `region_predicate` when given, so the diff is scoped to
///    exactly the same region the candidate covers.
/// 4. `DELETE FROM <table> WHERE [<region_predicate> AND] EXISTS (SELECT 1
///    FROM <sentinel_relation>)` — the whole region, guarded.
/// 5. `INSERT INTO <table> SELECT * FROM <staged_relation> WHERE EXISTS
///    (SELECT 1 FROM <sentinel_relation>)` — the whole staged candidate,
///    guarded by the same sentinel.
/// 6. `DROP TABLE <staged_relation>`
/// 7. `DROP TABLE <sentinel_relation>`
///
/// One transaction, same contract as every other staged-candidate emitter in
/// this module.
///
/// **No observed delta is recorded on this path.** The observed-delta table
/// (T5) is keyed by the row identity's key columns; a keyless write has none
/// to record — callers must not synthesize a fake key to force a recording.
///
/// # Panics
/// Panics if `candidate_select` is empty — a vacuous candidate has no sound
/// diff shape to emit.
pub fn emit_staged_candidate_conditional_keyless(
    table: &str,
    staged_relation: &str,
    sentinel_relation: &str,
    region_predicate: Option<&str>,
    candidate_select: &str,
    _dialect: MaintenanceDialect,
) -> StatementGroup {
    assert!(
        !candidate_select.trim().is_empty(),
        "emit_staged_candidate_conditional_keyless requires a non-empty candidate select for \
         {table}"
    );

    let create = format!(
        "CREATE TEMP TABLE {staged_relation} AS SELECT * FROM ({candidate_select}) AS \
         __smelt_staged_shape LIMIT 0"
    );
    let insert_candidates = format!("INSERT INTO {staged_relation} {candidate_select}");

    let stored_side = match region_predicate {
        Some(pred) => format!("SELECT * FROM {table} WHERE {pred}"),
        None => format!("SELECT * FROM {table}"),
    };
    let sentinel = format!(
        "CREATE TEMP TABLE {sentinel_relation} AS SELECT 1 AS __smelt_diff FROM (({stored_side} \
         EXCEPT ALL SELECT * FROM {staged_relation}) UNION ALL (SELECT * FROM {staged_relation} \
         EXCEPT ALL {stored_side})) AS __smelt_diff_rows LIMIT 1"
    );

    let delete = match region_predicate {
        Some(pred) => format!(
            "DELETE FROM {table} WHERE {pred} AND EXISTS (SELECT 1 FROM {sentinel_relation})"
        ),
        None => {
            format!("DELETE FROM {table} WHERE EXISTS (SELECT 1 FROM {sentinel_relation})")
        }
    };
    let insert = format!(
        "INSERT INTO {table} SELECT * FROM {staged_relation} WHERE EXISTS (SELECT 1 FROM \
         {sentinel_relation})"
    );
    let drop_staged = format!("DROP TABLE {staged_relation}");
    let drop_sentinel = format!("DROP TABLE {sentinel_relation}");

    StatementGroup {
        statements: vec![
            MaintenanceStatement::new(create),
            MaintenanceStatement::new(insert_candidates),
            MaintenanceStatement::new(sentinel),
            MaintenanceStatement::new(delete),
            MaintenanceStatement::new(insert),
            MaintenanceStatement::new(drop_staged),
            MaintenanceStatement::new(drop_sentinel),
        ],
        transactional: true,
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
mod staged_candidate_keyless_tests {
    use super::*;

    #[test]
    fn keyless_group_stages_diffs_and_guards_both_write_legs() {
        let group = emit_staged_candidate_conditional_keyless(
            "main.events_region",
            "__smelt_staged_events_region",
            "__smelt_sentinel_events_region",
            None,
            "SELECT event_id, event_date, payload FROM source_delta",
            MaintenanceDialect::DuckDb,
        );

        assert!(group.transactional);
        assert_eq!(group.statements.len(), 7);
        assert_eq!(
            group.statements[0].sql,
            "CREATE TEMP TABLE __smelt_staged_events_region AS SELECT * FROM (SELECT event_id, \
             event_date, payload FROM source_delta) AS __smelt_staged_shape LIMIT 0"
        );
        assert_eq!(
            group.statements[1].sql,
            "INSERT INTO __smelt_staged_events_region SELECT event_id, event_date, payload FROM \
             source_delta"
        );
        assert!(group.statements[2]
            .sql
            .starts_with("CREATE TEMP TABLE __smelt_sentinel_events_region AS"));
        assert!(group.statements[2].sql.contains("EXCEPT ALL"));
        // Both directions of the whole-row diff appear.
        assert_eq!(group.statements[2].sql.matches("EXCEPT ALL").count(), 2);
        assert_eq!(
            group.statements[3].sql,
            "DELETE FROM main.events_region WHERE EXISTS (SELECT 1 FROM \
             __smelt_sentinel_events_region)"
        );
        assert_eq!(
            group.statements[4].sql,
            "INSERT INTO main.events_region SELECT * FROM __smelt_staged_events_region WHERE \
             EXISTS (SELECT 1 FROM __smelt_sentinel_events_region)"
        );
        assert_eq!(
            group.statements[5].sql,
            "DROP TABLE __smelt_staged_events_region"
        );
        assert_eq!(
            group.statements[6].sql,
            "DROP TABLE __smelt_sentinel_events_region"
        );
    }

    #[test]
    fn keyless_region_predicate_bounds_both_the_diff_and_the_delete() {
        let with_region = emit_staged_candidate_conditional_keyless(
            "main.events_region",
            "__smelt_staged_events_region",
            "__smelt_sentinel_events_region",
            Some("main.events_region.event_date >= '2026-08-01'"),
            "SELECT event_id, event_date FROM source_delta",
            MaintenanceDialect::DuckDb,
        );
        assert!(with_region.statements[2]
            .sql
            .contains("main.events_region.event_date >= '2026-08-01'"));
        assert!(with_region.statements[3]
            .sql
            .contains("main.events_region.event_date >= '2026-08-01'"));

        let without_region = emit_staged_candidate_conditional_keyless(
            "main.events_region",
            "__smelt_staged_events_region",
            "__smelt_sentinel_events_region",
            None,
            "SELECT event_id, event_date FROM source_delta",
            MaintenanceDialect::DuckDb,
        );
        assert!(!without_region.statements[2].sql.contains("event_date >= "));
        assert!(!without_region.statements[3].sql.contains("event_date >= "));
    }

    #[test]
    #[should_panic(expected = "requires a non-empty candidate select")]
    fn keyless_emitter_needs_no_key_but_refuses_an_empty_candidate_select() {
        emit_staged_candidate_conditional_keyless(
            "main.events_region",
            "__smelt_staged_events_region",
            "__smelt_sentinel_events_region",
            None,
            "   ",
            MaintenanceDialect::DuckDb,
        );
    }
}
