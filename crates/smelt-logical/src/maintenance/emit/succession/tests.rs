use super::*;

fn keys() -> Vec<String> {
    vec!["customer_id".to_string()]
}

#[test]
fn tombstone_table_name_appends_the_reserved_suffix() {
    assert_eq!(
        tombstone_table_name("main.customer_history"),
        "main.customer_history__tombstones"
    );
}

#[test]
fn event_delta_select_projects_row_local_columns_and_the_delete_flag_with_no_window_function() {
    let projection = vec![
        ("customer_id".to_string(), "customer_id".to_string()),
        ("changed_at".to_string(), "changed_at".to_string()),
        ("tier".to_string(), "tier".to_string()),
        ("is_deleted".to_string(), "is_deleted".to_string()),
    ];
    let stmt = emit_succession_event_delta(
        "raw.customer_changes",
        &projection,
        Some("ingested_at < changed_at + INTERVAL '7 days'"),
        "ingested_date >= DATE '2026-01-01' AND ingested_date < DATE '2026-01-02'",
    );
    assert!(!stmt.sql.contains("OVER ("), "{}", stmt.sql);
    assert!(
        stmt.sql
            .contains("ingested_at < changed_at + INTERVAL '7 days'"),
        "{}",
        stmt.sql
    );
    assert!(
        stmt.sql
            .contains("ingested_date >= DATE '2026-01-01' AND ingested_date < DATE '2026-01-02'"),
        "{}",
        stmt.sql
    );
    assert!(stmt.sql.starts_with(
        "SELECT customer_id AS customer_id, changed_at AS changed_at, tier AS tier, \
             is_deleted AS is_deleted FROM raw.customer_changes WHERE"
    ));
}

#[test]
fn patch_group_is_transactional_and_records_tombstones_before_the_presented_merge() {
    let group = emit_succession_patch(
        "main.customer_history",
        &keys(),
        "changed_at",
        &["tier".to_string()],
        &[("valid_to".to_string(), "{lead}".to_string())],
        &[],
        Some("is_deleted"),
        "SELECT customer_id, changed_at, tier, is_deleted FROM raw.customer_changes",
        MaintenanceDialect::DuckDb,
    )
    .expect("a realisable succession dialect");
    assert!(group.transactional);
    assert_eq!(group.statements.len(), 2);
    assert!(
        group.statements[0]
            .sql
            .starts_with("INSERT INTO main.customer_history__tombstones"),
        "{}",
        group.statements[0].sql
    );
    assert!(
        group.statements[1].sql.starts_with("MERGE INTO"),
        "{}",
        group.statements[1].sql
    );
}

#[test]
fn patch_merge_neighbour_domain_unions_presented_ledger_and_batch() {
    let group = emit_succession_patch(
        "main.customer_history",
        &keys(),
        "changed_at",
        &["tier".to_string()],
        &[("valid_to".to_string(), "{lead}".to_string())],
        &[],
        None,
        "SELECT customer_id, changed_at, tier FROM raw.customer_changes",
        MaintenanceDialect::DuckDb,
    )
    .expect("a realisable succession dialect");
    let merge_sql = &group.statements[1].sql;
    assert!(
        merge_sql.contains("FROM main.customer_history WHERE"),
        "{merge_sql}"
    );
    assert!(
        merge_sql.contains("FROM main.customer_history__tombstones WHERE"),
        "{merge_sql}"
    );
    assert!(
        merge_sql.contains(
            "FROM (SELECT customer_id, changed_at, tier FROM \
             raw.customer_changes) AS __smelt_batch"
        ),
        "{merge_sql}"
    );
    assert!(
        merge_sql.contains(
            "LEAD(__smelt_t) OVER (PARTITION BY customer_id ORDER BY \
             __smelt_t)"
        ),
        "{merge_sql}"
    );
    assert!(
        !merge_sql.contains(
            "LEAD(__smelt_t) OVER (PARTITION BY customer_id ORDER BY \
             __smelt_t) AS __smelt_lead_t FROM main.customer_history"
        ),
        "the LEAD/LAG recomputation must run over the domain union, not the presented table \
             alone: {merge_sql}"
    );
}

#[test]
fn patch_merge_keys_on_key_columns_and_the_clock() {
    let group = emit_succession_patch(
        "main.customer_history",
        &keys(),
        "changed_at",
        &["tier".to_string()],
        &[("valid_to".to_string(), "{lead}".to_string())],
        &[],
        None,
        "SELECT customer_id, changed_at, tier FROM raw.customer_changes",
        MaintenanceDialect::DuckDb,
    )
    .expect("a realisable succession dialect");
    let merge_sql = &group.statements[1].sql;
    assert!(
        merge_sql.contains(
            "ON target.customer_id = source.customer_id AND target.changed_at = \
                 source.__smelt_t"
        ),
        "{merge_sql}"
    );
}

#[test]
fn ledger_rebuild_select_is_key_and_clock_of_delete_flagged_rows_passing_the_pre_filter() {
    let stmt = emit_succession_ledger_rebuild_select(
        "raw.customer_changes",
        &keys(),
        "changed_at",
        Some("ingested_at < changed_at + INTERVAL '7 days'"),
        "is_deleted",
        None,
    );
    assert_eq!(
        stmt.sql,
        "SELECT customer_id, changed_at FROM raw.customer_changes WHERE (ingested_at < \
             changed_at + INTERVAL '7 days') AND (is_deleted)"
    );
}

#[test]
fn clock_tie_probe_selects_key_clock_and_a_sample_for_non_identical_collisions() {
    let stmt = emit_succession_clock_tie_probe(
        "main.customer_history",
        &keys(),
        "changed_at",
        &["tier".to_string()],
        None,
        "SELECT customer_id, changed_at, tier FROM raw.customer_changes",
        MaintenanceDialect::DuckDb,
    );
    assert!(stmt.sql.contains("violation_count"), "{}", stmt.sql);
    assert!(stmt.sql.contains("sample_keys"), "{}", stmt.sql);
    assert!(stmt.sql.contains("HAVING COUNT(DISTINCT"), "{}", stmt.sql);
}

/// Runs [`emit_succession_clock_tie_probe`] against a real in-memory
/// DuckDB, over a `main.customer_history` presented table and its
/// `__tombstones` ledger sibling, both pre-created by the caller, and
/// returns the probe's `violation_count`.
fn clock_tie_violation_count(conn: &duckdb::Connection, event_delta_select: &str) -> i64 {
    let stmt = emit_succession_clock_tie_probe(
        "main.customer_history",
        &keys(),
        "changed_at",
        &["tier".to_string()],
        Some("is_deleted"),
        event_delta_select,
        MaintenanceDialect::DuckDb,
    );
    conn.query_row(&stmt.sql, [], |row| row.get::<_, i64>(0))
        .expect("clock tie probe query")
}

fn conn_with_empty_presented_and_ledger() -> duckdb::Connection {
    let conn = duckdb::Connection::open_in_memory().expect("open in-memory duckdb");
    conn.execute_batch(
        "CREATE TABLE main.customer_history (customer_id INTEGER, changed_at TIMESTAMP, \
             tier VARCHAR); CREATE TABLE main.customer_history__tombstones (customer_id \
             INTEGER, changed_at TIMESTAMP);",
    )
    .expect("create presented and ledger tables");
    conn
}

/// The red test: replaying a tombstoned delete (same `(k, t)` as an
/// existing ledger row, delete flag set) must be silent — the spec's
/// rule that "against a stored tombstone only the delete flag is
/// comparable, since the ledger carries no row-local content"
/// (`docs/specs/incremental_shapes.md` §"Run shape and late events").
/// Before the fix, the ledger row's NULL payload and the replayed
/// event's real payload compared unequal, so this fired a spurious
/// `SuccessionClockTie` on every refold of a window containing a
/// delete.
#[test]
fn clock_tie_probe_is_silent_when_a_tombstoned_delete_is_replayed() {
    let conn = conn_with_empty_presented_and_ledger();
    conn.execute_batch(
        "INSERT INTO main.customer_history__tombstones VALUES (1, TIMESTAMP \
             '2024-01-01 00:00:00');",
    )
    .expect("seed ledger row");
    let count = clock_tie_violation_count(
        &conn,
        "SELECT 1 AS customer_id, TIMESTAMP '2024-01-01 00:00:00' AS changed_at, 'bronze' \
             AS tier, TRUE AS is_deleted",
    );
    assert_eq!(count, 0);
}

#[test]
fn clock_tie_probe_still_fires_for_a_delete_and_an_insert_at_one_clock_value() {
    let conn = conn_with_empty_presented_and_ledger();
    conn.execute_batch(
        "INSERT INTO main.customer_history VALUES (1, TIMESTAMP '2024-01-01 00:00:00', \
             'silver');",
    )
    .expect("seed presented row");
    let count = clock_tie_violation_count(
        &conn,
        "SELECT 1 AS customer_id, TIMESTAMP '2024-01-01 00:00:00' AS changed_at, NULL AS \
             tier, TRUE AS is_deleted",
    );
    assert_eq!(count, 1);
}

#[test]
fn clock_tie_probe_still_fires_for_two_non_identical_inserts() {
    let conn = conn_with_empty_presented_and_ledger();
    let count = clock_tie_violation_count(
        &conn,
        "SELECT 1 AS customer_id, TIMESTAMP '2024-01-01 00:00:00' AS changed_at, 'bronze' \
             AS tier, FALSE AS is_deleted \
             UNION ALL \
             SELECT 1 AS customer_id, TIMESTAMP '2024-01-01 00:00:00' AS changed_at, 'gold' AS \
             tier, FALSE AS is_deleted",
    );
    assert_eq!(count, 1);
}

/// Two deletes colliding at one `(k, t)` are indistinguishable by
/// construction once a delete row's signature is its flag alone, so
/// they must stay silent (the spec's "identical ⇒ re-presentation"
/// rule) — distinct from the tombstone-replay case above in that
/// neither delete comes from the ledger.
#[test]
fn clock_tie_probe_is_silent_for_two_identical_deletes_at_one_clock_value() {
    let conn = conn_with_empty_presented_and_ledger();
    let count = clock_tie_violation_count(
        &conn,
        "SELECT 1 AS customer_id, TIMESTAMP '2024-01-01 00:00:00' AS changed_at, 'bronze' \
             AS tier, TRUE AS is_deleted \
             UNION ALL \
             SELECT 1 AS customer_id, TIMESTAMP '2024-01-01 00:00:00' AS changed_at, NULL AS \
             tier, TRUE AS is_deleted",
    );
    assert_eq!(count, 0);
}

#[test]
fn full_rebuild_group_is_transactional_and_replaces_the_ledger() {
    let model_select_sql = "SELECT customer_id, changed_at, tier, LEAD(changed_at) OVER \
                                 (PARTITION BY customer_id ORDER BY changed_at) AS valid_to FROM \
                                 raw.customer_changes";
    let output_columns = vec![
        "customer_id".to_string(),
        "changed_at".to_string(),
        "tier".to_string(),
        "valid_to".to_string(),
    ];
    let lead_derived = vec![("valid_to".to_string(), "{lead}".to_string())];
    let group = emit_succession_full_rebuild(
        "main.customer_history",
        model_select_sql,
        "raw.customer_changes",
        &keys(),
        "changed_at",
        &output_columns,
        &lead_derived,
        &[],
        None,
        "FALSE",
        MaintenanceDialect::DuckDb,
    )
    .expect("a realisable succession dialect");
    assert!(group.transactional);
    assert_eq!(group.statements.len(), 3);
    assert_eq!(
        group.statements[0].sql,
        format!(
            "CREATE TABLE main.customer_history AS SELECT customer_id, changed_at, tier, \
                 valid_to FROM (SELECT *, ROW_NUMBER() OVER (PARTITION BY customer_id, \
                 changed_at ORDER BY (CASE WHEN valid_to = changed_at THEN 1 ELSE 0 END) ASC) \
                 AS __smelt_rn FROM ({model_select_sql}) AS __smelt_model) AS __smelt_ranked \
                 WHERE __smelt_rn = 1"
        )
    );
    assert_eq!(
        group.statements[1].sql,
        "DELETE FROM main.customer_history__tombstones"
    );
    let expected_select = emit_succession_ledger_rebuild_select(
        "raw.customer_changes",
        &keys(),
        "changed_at",
        None,
        "FALSE",
        None,
    );
    assert_eq!(
        group.statements[2].sql,
        format!(
            "INSERT INTO main.customer_history__tombstones (customer_id, changed_at) {}",
            expected_select.sql
        )
    );
}

#[test]
fn emit_succession_full_rebuild_ledgerless_emits_only_the_presented_arm() {
    let model_select_sql = "SELECT customer_id, changed_at, tier, LEAD(changed_at) OVER \
                                 (PARTITION BY customer_id ORDER BY changed_at) AS valid_to FROM \
                                 raw.customer_changes";
    let output_columns = vec![
        "customer_id".to_string(),
        "changed_at".to_string(),
        "tier".to_string(),
        "valid_to".to_string(),
    ];
    let lead_derived = vec![("valid_to".to_string(), "{lead}".to_string())];

    // The presented arm's fold shape must not drift between the two
    // callers of `presented_arm_statement` — compared at the same dialect,
    // since `emit_create_table_as`'s per-dialect `USING DELTA` clause
    // (needed by the window-forward `MERGE` loop, never issued for a
    // downgraded cell) is orthogonal to what this test is proving.
    let ledgerless_duckdb = emit_succession_full_rebuild_ledgerless(
        "main.customer_history",
        model_select_sql,
        &keys(),
        "changed_at",
        &output_columns,
        &lead_derived,
        &[],
        MaintenanceDialect::DuckDb,
    );
    let ledger_bearing = emit_succession_full_rebuild(
        "main.customer_history",
        model_select_sql,
        "raw.customer_changes",
        &keys(),
        "changed_at",
        &output_columns,
        &lead_derived,
        &[],
        None,
        "FALSE",
        MaintenanceDialect::DuckDb,
    )
    .expect("a realisable succession dialect");
    assert_eq!(
        ledgerless_duckdb.statements[0].sql,
        ledger_bearing.statements[0].sql
    );

    let ledgerless_spark = emit_succession_full_rebuild_ledgerless(
        "main.customer_history",
        model_select_sql,
        &keys(),
        "changed_at",
        &output_columns,
        &lead_derived,
        &[],
        MaintenanceDialect::Spark,
    );
    assert!(!ledgerless_spark.transactional);
    assert_eq!(ledgerless_spark.statements.len(), 1);
    assert!(
        !ledgerless_spark.statements[0].sql.contains("__tombstones"),
        "{}",
        ledgerless_spark.statements[0].sql
    );
}

#[test]
fn emit_succession_full_rebuild_ledgerless_accepts_every_dialect() {
    let model_select_sql = "SELECT customer_id, changed_at FROM raw.customer_changes";
    let output_columns = vec!["customer_id".to_string(), "changed_at".to_string()];
    for dialect in [
        MaintenanceDialect::DuckDb,
        MaintenanceDialect::Spark,
        MaintenanceDialect::BigQuery,
    ] {
        let group = emit_succession_full_rebuild_ledgerless(
            "main.customer_history",
            model_select_sql,
            &keys(),
            "changed_at",
            &output_columns,
            &[],
            &[],
            dialect,
        );
        assert_eq!(group.statements.len(), 1);
    }
}

/// The refusal is now *typed* rather than a panic (`CLAUDE.md` §"Fail-loud
/// discipline"), and it names only the dialect that genuinely has no
/// realisation: Spark, where Delta's lack of a cross-table transaction
/// leaves the tombstone record and the presented write unable to land
/// together. BigQuery is no longer refused.
#[test]
fn emit_succession_full_rebuild_refuses_only_the_dialect_with_no_realisation() {
    let rebuild = |dialect| {
        emit_succession_full_rebuild(
            "main.customer_history",
            "SELECT 1",
            "raw.customer_changes",
            &keys(),
            "changed_at",
            &[],
            &[],
            &[],
            None,
            "FALSE",
            dialect,
        )
    };
    let err = rebuild(MaintenanceDialect::Spark).expect_err("Spark has no tombstone ledger");
    assert_eq!(err.dialect, "spark");
    assert!(err.to_string().contains("tombstone ledger"), "{err}");
    rebuild(MaintenanceDialect::DuckDb).expect("DuckDB realises the tombstone ledger");
    rebuild(MaintenanceDialect::BigQuery).expect("BigQuery realises the tombstone ledger");
}

#[test]
fn full_rebuild_fold_is_identity_with_no_extra_columns() {
    let model_select_sql = "SELECT customer_id, changed_at FROM raw.customer_changes";
    let output_columns = vec!["customer_id".to_string(), "changed_at".to_string()];
    let group = emit_succession_full_rebuild(
        "main.customer_history",
        model_select_sql,
        "raw.customer_changes",
        &keys(),
        "changed_at",
        &output_columns,
        &[],
        &[],
        None,
        "FALSE",
        MaintenanceDialect::DuckDb,
    )
    .expect("a realisable succession dialect");
    assert_eq!(
        group.statements[0].sql,
        format!(
            "CREATE TABLE main.customer_history AS SELECT customer_id, changed_at FROM \
                 (SELECT *, ROW_NUMBER() OVER (PARTITION BY customer_id, changed_at ORDER BY 1 \
                 ASC) AS __smelt_rn FROM ({model_select_sql}) AS __smelt_model) AS \
                 __smelt_ranked WHERE __smelt_rn = 1"
        )
    );
}

#[test]
fn full_rebuild_folds_on_key_and_clock_with_no_bare_passthrough() {
    let model_select_sql = "SELECT customer_id, changed_at, tier FROM raw.customer_changes";
    let output_columns = vec![
        "customer_id".to_string(),
        "changed_at".to_string(),
        "tier".to_string(),
    ];
    let group = emit_succession_full_rebuild(
        "main.customer_history",
        model_select_sql,
        "raw.customer_changes",
        &keys(),
        "changed_at",
        &output_columns,
        &[],
        &[],
        None,
        "FALSE",
        MaintenanceDialect::DuckDb,
    )
    .expect("a realisable succession dialect");
    let presented_sql = &group.statements[0].sql;
    assert_ne!(
        presented_sql,
        &format!("CREATE TABLE main.customer_history AS {model_select_sql}"),
        "the presented rebuild must not be a bare passthrough of the model select: \
             {presented_sql}"
    );
    assert!(
        presented_sql.contains("PARTITION BY customer_id, changed_at"),
        "{presented_sql}"
    );
    assert!(
        presented_sql.contains("WHERE __smelt_rn = 1"),
        "{presented_sql}"
    );
}

/// Same shape as the rebuild's refusal test: Spark only, typed, and the two
/// realising dialects both produce a group.
#[test]
fn emit_succession_patch_refuses_only_the_dialect_with_no_realisation() {
    let patch = |dialect| {
        emit_succession_patch(
            "main.customer_history",
            &keys(),
            "changed_at",
            &["tier".to_string()],
            &[("valid_to".to_string(), "{lead}".to_string())],
            &[],
            None,
            "SELECT customer_id, changed_at, tier FROM raw.customer_changes",
            dialect,
        )
    };
    let err = patch(MaintenanceDialect::Spark).expect_err("Spark has no tombstone ledger");
    assert_eq!(err.dialect, "spark");
    patch(MaintenanceDialect::DuckDb).expect("DuckDB realises the tombstone ledger");
    patch(MaintenanceDialect::BigQuery).expect("BigQuery realises the tombstone ledger");
}

// ── GoogleSQL spellings ────────────────────────────────────────────────────
//
// (`docs/specs/state.md` §"Which dialects realise which structure".) Each of
// these asserts the BigQuery text *verbatim*, because the risk this phase
// carries is a plausible-looking translation the engine rejects on a path no
// offline test covers — a shape assertion would not have caught the two
// constructs that actually needed rewriting.

fn two_keys() -> Vec<String> {
    vec!["customer_id".to_string(), "region".to_string()]
}

fn bq_patch(key_cols: &[String]) -> StatementGroup {
    emit_succession_patch(
        "smelt_dogfood.customer_history",
        key_cols,
        "changed_at",
        &["tier".to_string()],
        &[("valid_to".to_string(), "{lead}".to_string())],
        &[],
        Some("is_deleted"),
        "SELECT customer_id, region, changed_at, tier, is_deleted FROM raw.customer_changes",
        MaintenanceDialect::BigQuery,
    )
    .expect("BigQuery realises the tombstone ledger")
}

/// The BigQuery patch group's text, verbatim. The tombstone insert needed
/// no translation at all — `INSERT … SELECT … WHERE NOT EXISTS (correlated)`
/// is the same in both dialects — so it is asserted identical to DuckDB's
/// rather than restated, which is the strongest form of "this construct did
/// not need a dialect branch".
#[test]
fn bigquery_patch_group_text_is_verbatim() {
    let group = bq_patch(&two_keys());
    assert!(group.transactional);
    assert_eq!(group.statements.len(), 2);
    assert_eq!(group.statements[0].sql, BQ_TOMBSTONE_INSERT);
    assert_eq!(group.statements[1].sql, BQ_PATCH_MERGE);
}

/// The idempotent tombstone insert is dialect-identical; only the neighbour
/// domain and the dedup relation needed translating.
#[test]
fn the_tombstone_insert_needed_no_dialect_branch() {
    let bq = bq_patch(&two_keys());
    let duck = emit_succession_patch(
        "smelt_dogfood.customer_history",
        &two_keys(),
        "changed_at",
        &["tier".to_string()],
        &[("valid_to".to_string(), "{lead}".to_string())],
        &[],
        Some("is_deleted"),
        "SELECT customer_id, region, changed_at, tier, is_deleted FROM raw.customer_changes",
        MaintenanceDialect::DuckDb,
    )
    .expect("DuckDB realises the tombstone ledger");
    assert_eq!(bq.statements[0].sql, duck.statements[0].sql);
    // …and the MERGE genuinely differs, so the test above is not vacuous.
    assert_ne!(bq.statements[1].sql, duck.statements[1].sql);
}

/// The three GoogleSQL constructs this phase had to avoid, asserted as
/// absences on the BigQuery path and as *presences* on DuckDB's — so a
/// regression that reunified the two spellings fails here rather than at the
/// warehouse.
#[test]
fn the_bigquery_patch_avoids_the_constructs_googlesql_lacks() {
    let bq = bq_patch(&two_keys()).statements[1].sql.clone();
    let duck = emit_succession_patch(
        "smelt_dogfood.customer_history",
        &two_keys(),
        "changed_at",
        &["tier".to_string()],
        &[("valid_to".to_string(), "{lead}".to_string())],
        &[],
        Some("is_deleted"),
        "SELECT customer_id, region, changed_at, tier, is_deleted FROM raw.customer_changes",
        MaintenanceDialect::DuckDb,
    )
    .expect("DuckDB realises the tombstone ledger")
    .statements[1]
        .sql
        .clone();

    // 1. No row-constructor `IN`: `(a, b)` is not a tuple in GoogleSQL.
    assert!(!bq.contains(") IN (SELECT"), "{bq}");
    assert!(duck.contains("(customer_id, region) IN (SELECT"), "{duck}");
    // 2. No `QUALIFY` — its preconditions on BigQuery cannot be settled
    //    offline, and an explicit `ROW_NUMBER() … WHERE rn = 1` is exact.
    assert!(!bq.contains("QUALIFY"), "{bq}");
    assert!(duck.contains("QUALIFY"), "{duck}");
    // 3. No `WITH` inside the MERGE's `USING` subquery.
    assert!(!bq.contains("WITH __smelt_domain"), "{bq}");
    assert!(duck.contains("WITH __smelt_domain"), "{duck}");
    // And no DuckDB-only spellings leak through.
    assert!(!bq.contains("VARCHAR"), "{bq}");
    assert!(!bq.contains('"'), "{bq}");
}

/// The full-rebuild group in GoogleSQL: the presented `CREATE TABLE … AS`
/// carries no format clause, and the ledger truncation carries the `WHERE
/// TRUE` GoogleSQL requires of every `DELETE`.
#[test]
fn bigquery_full_rebuild_text_is_verbatim() {
    let group = emit_succession_full_rebuild(
        "smelt_dogfood.customer_history",
        "SELECT customer_id, changed_at, valid_to FROM raw.customer_changes",
        "raw.customer_changes",
        &keys(),
        "changed_at",
        &[
            "customer_id".to_string(),
            "changed_at".to_string(),
            "valid_to".to_string(),
        ],
        &[("valid_to".to_string(), "{lead}".to_string())],
        &[],
        None,
        "is_deleted",
        MaintenanceDialect::BigQuery,
    )
    .expect("BigQuery realises the tombstone ledger");
    assert!(group.transactional);
    assert_eq!(group.statements.len(), 3);
    assert_eq!(
        group.statements[0].sql,
        "CREATE TABLE smelt_dogfood.customer_history AS SELECT customer_id, changed_at, \
         valid_to FROM (SELECT *, ROW_NUMBER() OVER (PARTITION BY customer_id, changed_at ORDER \
         BY (CASE WHEN valid_to = changed_at THEN 1 ELSE 0 END) ASC) AS __smelt_rn FROM (SELECT \
         customer_id, changed_at, valid_to FROM raw.customer_changes) AS __smelt_model) AS \
         __smelt_ranked WHERE __smelt_rn = 1"
    );
    assert_eq!(
        group.statements[1].sql,
        "DELETE FROM smelt_dogfood.customer_history__tombstones WHERE TRUE"
    );
    assert_eq!(
        group.statements[2].sql,
        "INSERT INTO smelt_dogfood.customer_history__tombstones (customer_id, changed_at) \
         SELECT customer_id, changed_at FROM raw.customer_changes WHERE is_deleted"
    );
}

/// DuckDB's rebuild keeps its bare `DELETE` — the `WHERE TRUE` is a
/// GoogleSQL requirement, not a shared change.
#[test]
fn duckdb_full_rebuild_keeps_the_bare_delete() {
    let group = emit_succession_full_rebuild(
        "main.customer_history",
        "SELECT customer_id, changed_at FROM raw.customer_changes",
        "raw.customer_changes",
        &keys(),
        "changed_at",
        &["customer_id".to_string(), "changed_at".to_string()],
        &[],
        &[],
        None,
        "is_deleted",
        MaintenanceDialect::DuckDb,
    )
    .expect("DuckDB realises the tombstone ledger");
    assert_eq!(
        group.statements[1].sql,
        "DELETE FROM main.customer_history__tombstones"
    );
}

/// The clock-tie probe already dispatched on `dialect` for its string cast
/// and its sample aggregate; what changed is the neighbour domain beneath it,
/// which must now carry BigQuery's `EXISTS` scoping too.
#[test]
fn the_bigquery_clock_tie_probe_scopes_with_exists_and_casts_to_string() {
    let stmt = emit_succession_clock_tie_probe(
        "smelt_dogfood.customer_history",
        &two_keys(),
        "changed_at",
        &["tier".to_string()],
        Some("is_deleted"),
        "SELECT customer_id, region, changed_at, tier, is_deleted FROM raw.customer_changes",
        MaintenanceDialect::BigQuery,
    );
    assert!(stmt.sql.contains("CAST(tier AS STRING)"), "{}", stmt.sql);
    assert!(!stmt.sql.contains("VARCHAR"), "{}", stmt.sql);
    assert!(
        stmt.sql
            .contains("FROM smelt_dogfood.customer_history AS __smelt_presented WHERE EXISTS ("),
        "{}",
        stmt.sql
    );
    assert!(!stmt.sql.contains(") IN (SELECT"), "{}", stmt.sql);
}

const BQ_TOMBSTONE_INSERT: &str = "INSERT INTO smelt_dogfood.customer_history__tombstones (customer_id, region, changed_at) SELECT customer_id, region, changed_at FROM (SELECT customer_id, region, changed_at, tier, is_deleted FROM raw.customer_changes) AS __smelt_batch WHERE is_deleted AND NOT EXISTS (SELECT 1 FROM smelt_dogfood.customer_history__tombstones AS __smelt_existing WHERE __smelt_existing.customer_id = __smelt_batch.customer_id AND __smelt_existing.region = __smelt_batch.region AND __smelt_existing.changed_at = __smelt_batch.changed_at)";

const BQ_PATCH_MERGE: &str = "MERGE INTO smelt_dogfood.customer_history AS target USING (SELECT customer_id, region, __smelt_t, tier, __smelt_is_delete, __smelt_lead_t AS valid_to FROM (SELECT customer_id, region, __smelt_t, tier, __smelt_is_delete, LEAD(__smelt_t) OVER (PARTITION BY customer_id, region ORDER BY __smelt_t) AS __smelt_lead_t, LAG(__smelt_t) OVER (PARTITION BY customer_id, region ORDER BY __smelt_t) AS __smelt_lag_t FROM (SELECT customer_id, region, __smelt_t, tier, __smelt_is_delete FROM (SELECT customer_id, region, __smelt_t, tier, __smelt_is_delete, ROW_NUMBER() OVER (PARTITION BY customer_id, region, __smelt_t ORDER BY __smelt_is_delete ASC) AS __smelt_dedup_rn FROM (SELECT customer_id, region, changed_at AS __smelt_t, tier, FALSE AS __smelt_is_delete FROM smelt_dogfood.customer_history AS __smelt_presented WHERE EXISTS (SELECT 1 FROM (SELECT customer_id, region, changed_at, tier, is_deleted FROM raw.customer_changes) AS __smelt_touched_keys WHERE __smelt_touched_keys.customer_id = __smelt_presented.customer_id AND __smelt_touched_keys.region = __smelt_presented.region) UNION ALL SELECT customer_id, region, changed_at AS __smelt_t, NULL AS tier, TRUE AS __smelt_is_delete FROM smelt_dogfood.customer_history__tombstones AS __smelt_tombstones WHERE EXISTS (SELECT 1 FROM (SELECT customer_id, region, changed_at, tier, is_deleted FROM raw.customer_changes) AS __smelt_touched_keys WHERE __smelt_touched_keys.customer_id = __smelt_tombstones.customer_id AND __smelt_touched_keys.region = __smelt_tombstones.region) UNION ALL SELECT customer_id, region, changed_at AS __smelt_t, tier, is_deleted AS __smelt_is_delete FROM (SELECT customer_id, region, changed_at, tier, is_deleted FROM raw.customer_changes) AS __smelt_batch) AS __smelt_domain) AS __smelt_dedup_ranked WHERE __smelt_dedup_rn = 1) AS __smelt_dedup) AS __smelt_windowed) AS source ON target.customer_id = source.customer_id AND target.region = source.region AND target.changed_at = source.__smelt_t WHEN MATCHED THEN UPDATE SET tier = source.tier, valid_to = source.valid_to WHEN NOT MATCHED AND NOT source.__smelt_is_delete THEN INSERT (customer_id, region, changed_at, tier, valid_to) VALUES (source.customer_id, source.region, source.__smelt_t, source.tier, source.valid_to)";
