use super::*;

/// T5 (`docs/plans/20260715-composed-axes-conditional-maintenance.md` Phase
/// D2) — structural no-authoring gate extension for observed-delta
/// recording. `docs/specs/incremental_models.md` §"The graph layer" —
/// "Observed deltas on model edges" places the recorded delta in the SAME
/// warehouse-resident, "bookkeeping" class as the reconciliation ledger
/// (`smelt_state::ddl_duckdb::generate_ledger_table_ddl`/
/// `generate_ledger_insert_sql`): D1's ruling is that this is smelt-state
/// storage for a run's own byproduct, not a maintenance statement the run's
/// *write* executes, so `smelt_logical::maintenance::emit`'s single-owner
/// rule does not apply to it, and `no_maintenance_statement_authoring_
/// outside_the_emitter` above is not extended with an allowlist entry (the
/// recording query is a `SELECT ... LEFT JOIN`, not one of that gate's
/// forbidden `DELETE FROM `/`MERGE INTO `/`CREATE TABLE {}.{} AS`/
/// `CREATE TEMP TABLE ` shapes — confirmed by that test's own green run,
/// unmodified by this phase).
///
/// What IS asserted here, as the phase's own structural gate: the "one
/// comparison, two consumers" claim
/// (`crate::maintenance_driver::changed_row_predicate`'s doc comment) — the
/// observed-delta recording query's `IS DISTINCT FROM` guard must be
/// BYTE-IDENTICAL to the suppressed MERGE's own matched-arm guard over the
/// same `compared_columns`, so change-suppression and delta-recording can
/// never silently diverge on what counts as "changed".
#[test]
fn observed_delta_predicate_matches_suppressed_merge_guard_byte_for_byte() {
    let compared_columns = vec!["tier".to_string(), "email".to_string()];

    let merge_group = smelt_logical::maintenance::emit::emit_column_scoped_merge_suppressed(
        "main.dim_users",
        &["user_id".to_string()],
        "SELECT * FROM main.sources_users",
        &compared_columns,
        &[],
        MaintenanceDialect::DuckDb,
    );
    let merge_sql = &merge_group.statements[0].sql;

    let record_sql = smelt_runtime::maintenance_driver::changed_row_predicate(
        "target",
        "source",
        &compared_columns,
    );

    assert!(
        merge_sql.contains(&record_sql),
        "the recorded-delta predicate must appear byte-identical inside the suppressed MERGE's \
         own matched-arm guard — predicate: {record_sql:?}, MERGE: {merge_sql:?}"
    );

    // The recording query built off the SAME predicate carries it verbatim
    // too — a second cross-check at the query-assembly level, not just the
    // bare predicate.
    let changed_keys_query = smelt_runtime::maintenance_driver::changed_keys_select(
        "main.dim_users",
        &["user_id".to_string()],
        "SELECT * FROM main.sources_users",
        &compared_columns,
        None,
    );
    assert!(
        changed_keys_query.contains(&record_sql),
        "changed_keys_select must carry the identical predicate text, got: {changed_keys_query:?}"
    );
}

/// Phase 16 (`docs/outcomes/20260815-definition-delta-migrate/phases/
/// 16-plan.md`): the keyed-fold observed-delta recording's `IS DISTINCT
/// FROM` guard must be BYTE-IDENTICAL to `emit_keyed_fold_suppressed`'s own
/// matched-arm guard — one comparison (over the fold's own combine
/// expression, not the raw delta column), two consumers.
#[test]
fn keyed_fold_changed_key_select_matches_the_merge_guard() {
    let compared_columns = vec!["score".to_string()];
    let folds = vec![(
        "score".to_string(),
        "GREATEST(target.score, delta.score)".to_string(),
    )];

    let merge_group = smelt_logical::maintenance::emit::emit_keyed_fold_suppressed(
        "main.dim_scores",
        &["user_id".to_string()],
        &folds,
        "SELECT user_id, score FROM main.src_scores",
        None,
        &compared_columns,
        MaintenanceDialect::DuckDb,
    );
    let merge_sql = &merge_group.statements[0].sql;

    let record_predicate = smelt_runtime::maintenance_driver::keyed_fold_changed_row_predicate(
        &compared_columns,
        &folds,
    );

    assert!(
        merge_sql.contains(&record_predicate),
        "the recorded-delta predicate must appear byte-identical inside the suppressed keyed \
         fold's own matched-arm guard — predicate: {record_predicate:?}, MERGE: {merge_sql:?}"
    );

    let changed_keys_query = smelt_runtime::maintenance_driver::keyed_fold_changed_keys_select(
        "main.dim_scores",
        &["user_id".to_string()],
        "SELECT user_id, score FROM main.src_scores",
        &compared_columns,
        &folds,
        None,
    );
    assert!(
        changed_keys_query.contains(&record_predicate),
        "keyed_fold_changed_keys_select must carry the identical predicate text, got: \
         {changed_keys_query:?}"
    );
}

/// Phase C5 (`docs/plans/20260715-composed-axes-conditional-maintenance.md`)
/// — the change-suppressed keyed-fold `MERGE` (T1 for `refresh: keyed`
/// models): `emit_keyed_fold_suppressed` carries the same suppression
/// predicate machinery as C4's `emit_column_scoped_merge_suppressed`, but
/// compares the stored value against the fold's own combine expression.
/// This is a direct-dispatch leg (no `execute_project` model pipeline
/// involved, matching this phase's "runtime e2e" test — the abstract
/// `MaintenancePlan`/`choice::resolve_keyed_write_mechanism` this phase adds
/// is not yet wired into the live `refresh: keyed` per-partition loop
/// (`smelt_runtime::cumulative`); that wiring is out of this phase's file
/// scope). It proves two things over a real DuckDB connection:
///
/// - The executed `StatementGroup` is byte-identical to a direct
///   `emit_keyed_fold_suppressed` call over the same inputs.
/// - A `run_marker` fold column — only ever overwritten when the matched
///   arm's `UPDATE SET` actually fires — proves the suppressed row was
///   never written at all (not merely that it landed on the same bits): a
///   device whose delta contributes zero new events (`event_count`
///   unchanged after the additive combine) keeps its **prior** run's
///   marker, while a device whose combined result differs gets the new
///   run's marker, and a brand-new device is inserted with it.
#[tokio::test]
async fn suppressed_keyed_fold_statements_come_from_the_emitter() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("test.duckdb");
    let inner = DuckDbBackend::new(&db_path, "main")
        .await
        .expect("open duckdb");
    let backend = RecordingBackend::new(inner);

    backend
        .execute_sql(
            "CREATE TABLE main.device_daily (device_id BIGINT, event_count BIGINT, run_marker \
             VARCHAR)",
        )
        .await
        .expect("create target table");
    backend
        .execute_sql("INSERT INTO main.device_daily VALUES (1, 5, 'run1'), (2, 3, 'run1')")
        .await
        .expect("seed target table");

    // Device 1's delta contributes zero new events (an unchanged-effect
    // re-run); device 2's delta genuinely adds events; device 3 is brand
    // new.
    let delta_select = "SELECT * FROM (VALUES (1, 0, 'run2'), (2, 4, 'run2'), (3, 10, 'run2')) AS \
                         t(device_id, event_count, run_marker)";
    let folds = vec![
        (
            "event_count".to_string(),
            "target.event_count + delta.event_count".to_string(),
        ),
        ("run_marker".to_string(), "delta.run_marker".to_string()),
    ];
    let key = vec!["device_id".to_string()];
    let compared_columns = vec!["event_count".to_string()];

    let group = emit_keyed_fold_suppressed(
        "main.device_daily",
        &key,
        &folds,
        delta_select,
        None,
        &compared_columns,
        MaintenanceDialect::DuckDb,
    );
    backend
        .execute_statement_group(&group)
        .await
        .expect("suppressed keyed-fold merge must succeed");

    let recorded = backend.recorded_groups();
    assert_eq!(recorded.len(), 1);
    assert_eq!(&recorded[0], &group);
    let expected = emit_keyed_fold_suppressed(
        "main.device_daily",
        &key,
        &folds,
        delta_select,
        None,
        &compared_columns,
        MaintenanceDialect::DuckDb,
    );
    assert_eq!(
        &expected, &recorded[0],
        "executed suppressed keyed-fold group must be byte-identical to a direct emitter call \
         over the same inputs"
    );

    let rows = backend
        .execute_sql(
            "SELECT device_id, event_count, run_marker FROM main.device_daily ORDER BY device_id",
        )
        .await
        .expect("read back target");
    let batch = &rows[0];
    let markers: Vec<String> = {
        let col = batch
            .column(2)
            .as_any()
            .downcast_ref::<arrow::array::StringArray>()
            .expect("run_marker is a string column");
        (0..col.len()).map(|i| col.value(i).to_string()).collect()
    };
    assert_eq!(
        markers,
        vec!["run1".to_string(), "run2".to_string(), "run2".to_string()],
        "device 1's suppressed row must keep its prior run's marker (never written); device 2 \
         (changed) and device 3 (new) must carry the new run's marker"
    );

    assert!(
        multiset_equal(
            &backend,
            "SELECT device_id, event_count FROM main.device_daily",
            "SELECT device_id, event_count FROM (VALUES (1, 5), (2, 7), (3, 10)) AS \
             t(device_id, event_count)"
        )
        .await,
        "the suppressed keyed-fold merge must reproduce the full-refresh oracle's combined state"
    );
}

/// Phase C5 — the staged-candidate conditional `DELETE`+`INSERT` (T2): the
/// merge-less keyed-shaped realisation. Proves the executed `StatementGroup`
/// is byte-identical to a direct `emit_staged_candidate_conditional` call,
/// that the same `run_marker` technique proves an unchanged row is never
/// touched (its prior marker survives), and that a mid-group failure rolls
/// back the whole transaction — including the staged temp relation's own
/// `CREATE` — leaving no temp relation behind.
#[tokio::test]
async fn staged_candidate_conditional_statements_come_from_the_emitter() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("test.duckdb");
    let inner = DuckDbBackend::new(&db_path, "main")
        .await
        .expect("open duckdb");
    let backend = RecordingBackend::new(inner);

    backend
        .execute_sql(
            "CREATE TABLE main.dim_users (user_id BIGINT, tier VARCHAR, run_marker VARCHAR)",
        )
        .await
        .expect("create target table");
    backend
        .execute_sql(
            "INSERT INTO main.dim_users VALUES (1, 'bronze', 'run1'), (2, 'silver', 'run1'), (3, \
             'gold', 'run1')",
        )
        .await
        .expect("seed target table");

    // user 1: unchanged tier ('bronze' -> 'bronze'); user 2: changed tier;
    // user 4: brand new. user 3 is absent from the candidate set (out of
    // this run's touched region) and must be left untouched entirely.
    let candidate_select = "SELECT * FROM (VALUES (1, 'bronze', 'run2'), (2, 'platinum', \
                             'run2'), (4, 'new', 'run2')) AS t(user_id, tier, run_marker)";
    let key = vec!["user_id".to_string()];
    let compared_columns = vec!["tier".to_string()];

    let group = smelt_logical::maintenance::emit::emit_staged_candidate_conditional(
        "main.dim_users",
        "__smelt_staged_dim_users",
        &key,
        candidate_select,
        &compared_columns,
        MaintenanceDialect::DuckDb,
    );
    backend
        .execute_statement_group(&group)
        .await
        .expect("staged-candidate conditional write must succeed");

    let recorded = backend.recorded_groups();
    assert_eq!(recorded.len(), 1);
    let expected = smelt_logical::maintenance::emit::emit_staged_candidate_conditional(
        "main.dim_users",
        "__smelt_staged_dim_users",
        &key,
        candidate_select,
        &compared_columns,
        MaintenanceDialect::DuckDb,
    );
    assert_eq!(
        &expected, &recorded[0],
        "executed staged-candidate group must be byte-identical to a direct emitter call over \
         the same inputs"
    );

    let rows = backend
        .execute_sql("SELECT user_id, tier, run_marker FROM main.dim_users ORDER BY user_id")
        .await
        .expect("read back target");
    let batch = &rows[0];
    let tiers: Vec<String> = {
        let col = batch
            .column(1)
            .as_any()
            .downcast_ref::<arrow::array::StringArray>()
            .expect("tier is a string column");
        (0..col.len()).map(|i| col.value(i).to_string()).collect()
    };
    let markers: Vec<String> = {
        let col = batch
            .column(2)
            .as_any()
            .downcast_ref::<arrow::array::StringArray>()
            .expect("run_marker is a string column");
        (0..col.len()).map(|i| col.value(i).to_string()).collect()
    };
    assert_eq!(
        tiers,
        vec![
            "bronze".to_string(),
            "platinum".to_string(),
            "gold".to_string(),
            "new".to_string()
        ]
    );
    assert_eq!(
        markers,
        vec![
            "run1".to_string(), // user 1: suppressed, never deleted/reinserted
            "run2".to_string(), // user 2: changed, deleted+reinserted
            "run1".to_string(), // user 3: absent from candidate set, untouched
            "run2".to_string(), // user 4: new, inserted
        ],
        "an unchanged staged candidate must never delete/reinsert its row (prior marker \
         survives); a changed or new row must carry the new run's marker"
    );

    assert!(
        multiset_equal(
            &backend,
            "SELECT user_id, tier FROM main.dim_users",
            "SELECT user_id, tier FROM (VALUES (1, 'bronze'), (2, 'platinum'), (3, 'gold'), (4, \
             'new')) AS t(user_id, tier)"
        )
        .await,
        "the staged-candidate conditional write must reproduce the full-refresh oracle"
    );

    let staged_relations = backend
        .execute_sql(
            "SELECT count(*) FROM duckdb_tables() WHERE table_name = \
             '__smelt_staged_dim_users'",
        )
        .await
        .expect("query duckdb_tables");
    let count = staged_relations[0]
        .column(0)
        .as_any()
        .downcast_ref::<Int64Array>()
        .expect("count(*) is Int64")
        .value(0);
    assert_eq!(
        count, 0,
        "the staged temp relation must be dropped by the end of a successful group"
    );
}

/// The full-recompute variant (`docs/plans/20260808-membership-sensitivity.md`
/// Phase 3): the executed `StatementGroup` is byte-identical to a direct
/// `emit_staged_candidate_conditional_recompute` call, and — unlike its
/// region-scoped sibling above — a row whose key is entirely absent from the
/// candidate (user 3) is genuinely DELETED, never merely left untouched:
/// this variant's `candidate_select` always represents the model's own full
/// current state, so absence means departure. A matched-but-unchanged row
/// (user 1) is still suppressed (never deleted/reinserted), proving the
/// extra departed-key `DELETE` is a no-op over still-present keys.
#[tokio::test]
async fn staged_candidate_conditional_recompute_deletes_departed_keys() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("test.duckdb");
    let inner = DuckDbBackend::new(&db_path, "main")
        .await
        .expect("open duckdb");
    let backend = RecordingBackend::new(inner);

    backend
        .execute_sql(
            "CREATE TABLE main.dim_users (user_id BIGINT, tier VARCHAR, run_marker VARCHAR)",
        )
        .await
        .expect("create target table");
    backend
        .execute_sql(
            "INSERT INTO main.dim_users VALUES (1, 'bronze', 'run1'), (2, 'silver', 'run1'), (3, \
             'gold', 'run1')",
        )
        .await
        .expect("seed target table");

    // user 1: unchanged tier; user 2: changed tier; user 4: brand new. user
    // 3 is genuinely departed — the model's own full recompute no longer
    // produces a row for it at all (e.g. the dimension row a fact joined on
    // was deleted).
    let candidate_select = "SELECT * FROM (VALUES (1, 'bronze', 'run2'), (2, 'platinum', \
                             'run2'), (4, 'new', 'run2')) AS t(user_id, tier, run_marker)";
    let key = vec!["user_id".to_string()];
    let compared_columns = vec!["tier".to_string()];

    let group = emit_staged_candidate_conditional_recompute(
        "main.dim_users",
        "__smelt_staged_dim_users",
        &key,
        candidate_select,
        &compared_columns,
        MaintenanceDialect::DuckDb,
    );
    backend
        .execute_statement_group(&group)
        .await
        .expect("staged-candidate recompute write must succeed");

    let recorded = backend.recorded_groups();
    assert_eq!(recorded.len(), 1);
    let expected = emit_staged_candidate_conditional_recompute(
        "main.dim_users",
        "__smelt_staged_dim_users",
        &key,
        candidate_select,
        &compared_columns,
        MaintenanceDialect::DuckDb,
    );
    assert_eq!(
        &expected, &recorded[0],
        "executed staged-candidate recompute group must be byte-identical to a direct emitter \
         call over the same inputs"
    );

    let rows = backend
        .execute_sql("SELECT user_id, tier, run_marker FROM main.dim_users ORDER BY user_id")
        .await
        .expect("read back target");
    let batch = &rows[0];
    let ids = batch
        .column(0)
        .as_any()
        .downcast_ref::<Int64Array>()
        .expect("user_id is Int64");
    assert_eq!(
        ids.len(),
        3,
        "user 3 (departed — absent from the full-recompute candidate) must be deleted, leaving \
         exactly users 1, 2, 4"
    );
    let markers: Vec<String> = {
        let col = batch
            .column(2)
            .as_any()
            .downcast_ref::<arrow::array::StringArray>()
            .expect("run_marker is a string column");
        (0..col.len()).map(|i| col.value(i).to_string()).collect()
    };
    assert_eq!(
        markers,
        vec!["run1".to_string(), "run2".to_string(), "run2".to_string()],
        "user 1 (unchanged, suppressed) keeps its prior marker; user 2 (changed) and user 4 \
         (new) carry the new run's marker"
    );

    assert!(
        multiset_equal(
            &backend,
            "SELECT user_id, tier FROM main.dim_users",
            "SELECT user_id, tier FROM (VALUES (1, 'bronze'), (2, 'platinum'), (4, 'new')) AS \
             t(user_id, tier)"
        )
        .await,
        "the staged-candidate recompute write must reproduce the full-refresh oracle — no \
         departed row survives"
    );
}

/// A mid-group failure (the candidate `INSERT`'s projection does not match
/// the staged relation's own `CREATE`-derived shape — a column-count
/// mismatch DuckDB rejects) must roll back the **entire** transaction,
/// including the temp relation's own `CREATE`: no temp relation is left
/// behind, and the target table is completely untouched.
#[tokio::test]
async fn staged_candidate_interrupted_run_leaves_no_temp_relation_behind() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("test.duckdb");
    let inner = DuckDbBackend::new(&db_path, "main")
        .await
        .expect("open duckdb");
    let backend = RecordingBackend::new(inner);

    backend
        .execute_sql("CREATE TABLE main.dim_users (user_id BIGINT, tier VARCHAR)")
        .await
        .expect("create target table");
    backend
        .execute_sql("INSERT INTO main.dim_users VALUES (1, 'bronze')")
        .await
        .expect("seed target table");

    // Hand-build a group whose CREATE (shape-only, `LIMIT 0` over a 2-column
    // projection) disagrees with its own INSERT (a 3-column projection) —
    // the same shape `emit_staged_candidate_conditional` would build if a
    // caller ever violated its full-row-projection contract. DuckDB rejects
    // the INSERT with a column-count mismatch mid-transaction.
    let mut group = smelt_logical::maintenance::emit::emit_staged_candidate_conditional(
        "main.dim_users",
        "__smelt_staged_broken",
        &["user_id".to_string()],
        "SELECT user_id, tier FROM (VALUES (1, 'bronze')) AS t(user_id, tier)",
        &["tier".to_string()],
        MaintenanceDialect::DuckDb,
    );
    group.statements[1] = smelt_logical::maintenance::emit::MaintenanceStatement {
        sql: "INSERT INTO __smelt_staged_broken SELECT user_id, tier, 'extra' FROM (VALUES (1, \
              'bronze')) AS t(user_id, tier, junk)"
            .to_string(),
    };

    let result = backend.execute_statement_group(&group).await;
    assert!(
        result.is_err(),
        "the deliberately-broken INSERT must fail: {result:?}"
    );

    let staged_relations = backend
        .execute_sql(
            "SELECT count(*) FROM duckdb_tables() WHERE table_name = '__smelt_staged_broken'",
        )
        .await
        .expect("query duckdb_tables");
    let count = staged_relations[0]
        .column(0)
        .as_any()
        .downcast_ref::<Int64Array>()
        .expect("count(*) is Int64")
        .value(0);
    assert_eq!(
        count, 0,
        "a rolled-back transaction must leave no staged temp relation behind — its own CREATE \
         is part of the same failed transaction"
    );

    assert!(
        multiset_equal(
            &backend,
            "SELECT user_id, tier FROM main.dim_users",
            "SELECT user_id, tier FROM (VALUES (1, 'bronze')) AS t(user_id, tier)"
        )
        .await,
        "the target table must be completely untouched by a rolled-back staged-candidate group"
    );
}

/// `docs/plans/20260715-composed-axes-conditional-maintenance.md` Phase C6's
/// own real-fixture requirement: `examples/web_analytics`'s
/// `silver.events_deduped` (the flagship composed shape — key-addressed via
/// `event_id`, time-partitioned via `first_seen_date`, admitted through
/// route 3's declared `key_recurrence`) driven through the SAME real model
/// text `smelt run` executes for that example, via `execute_project` and a
/// `RecordingBackend` so the executed SQL can be inspected directly — the
/// real-fixture counterpart to `keyed_fold_slice_predicated_merge_
/// statements_come_from_the_emitter`'s synthetic composed fixture above.
///
/// Only the two files this model actually needs
/// (`models/sources/raw/events.yml`, `models/silver/events_deduped.sql`)
/// are copied byte-for-byte off disk — not the whole example (which also
/// needs `smelt-datagen`-generated Parquet + the `functions/` dir neither
/// model here calls) — into a fresh scratch project, seeded directly via
/// `raw.events` INSERTs rather than a full datagen run.
///
/// Day 1 seeds `event_id` 1; day 2 redelivers the SAME `event_id` 1 with
/// byte-identical payload fields (only `arrival_time`, a column this model
/// never selects, would differ in a real redelivery — irrelevant here) —
/// exactly `datagen.yaml`'s `redelivery:` storm, collapsed to one pair —
/// alongside a genuinely new `event_id` 2. Day 2's `MERGE` step must carry
/// **both** predicates (the route-3 `RecurrenceBounded` slice on the target
/// read, `IS DISTINCT FROM` suppression on the matched arm) and must write
/// zero rows for the redelivered key while still inserting the new one.
#[tokio::test]
async fn events_deduped_composed_suppression_storm_rerun_writes_zero_rows() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project_dir = tmp.path();
    std::fs::create_dir_all(project_dir.join("models/sources/raw")).unwrap();
    std::fs::create_dir_all(project_dir.join("models/silver")).unwrap();

    let fixture_root = super::structural_and_ledger::repo_root().join("examples/web_analytics");
    let events_yml = std::fs::read_to_string(fixture_root.join("models/sources/raw/events.yml"))
        .expect("read examples/web_analytics/models/sources/raw/events.yml");
    let events_deduped_sql =
        std::fs::read_to_string(fixture_root.join("models/silver/events_deduped.sql"))
            .expect("read examples/web_analytics/models/silver/events_deduped.sql");
    std::fs::write(
        project_dir.join("models/sources/raw/events.yml"),
        &events_yml,
    )
    .unwrap();
    std::fs::write(
        project_dir.join("models/silver/events_deduped.sql"),
        &events_deduped_sql,
    )
    .unwrap();

    let db_path = project_dir.join("run.duckdb");
    let smelt_yml = format!(
        "name: events_deduped_storm_test\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: {db}\n    schema: main\ndefault_materialization: table\ntarget: dev\n",
        db = db_path.display()
    );
    std::fs::write(project_dir.join("smelt.yml"), &smelt_yml).unwrap();

    {
        let conn = duckdb::Connection::open(&db_path).expect("open duckdb");
        conn.execute_batch(
            "CREATE SCHEMA IF NOT EXISTS raw;\n\
             CREATE TABLE raw.events (\n\
               event_id BIGINT, device_id INTEGER, user_id INTEGER, seconds_in_day INTEGER,\n\
               event_time VARCHAR, arrival_time VARCHAR, utm_campaign VARCHAR, payload VARCHAR,\n\
               event_date VARCHAR\n\
             );\n\
             -- Day 1: event_id 1 first seen.\n\
             INSERT INTO raw.events VALUES (\n\
               1, 10, NULL, 100, '2026-04-01T00:01:40', '2026-04-01T00:01:41', NULL,\n\
               '{\"event_name\": \"page_view\", \"platform\": \"web\", \"url\": \"/home\"}',\n\
               '2026-04-01'\n\
             );\n\
             -- Day 2: event_id 1 redelivered (byte-identical payload fields — only\n\
             -- arrival_time, never selected by this model, differs), plus a\n\
             -- genuinely new event_id 2.\n\
             INSERT INTO raw.events VALUES (\n\
               1, 10, NULL, 100, '2026-04-01T00:01:40', '2026-04-02T00:01:41', NULL,\n\
               '{\"event_name\": \"page_view\", \"platform\": \"web\", \"url\": \"/home\"}',\n\
               '2026-04-01'\n\
             );\n\
             INSERT INTO raw.events VALUES (\n\
               2, 11, NULL, 200, '2026-04-02T00:03:20', '2026-04-02T00:03:21', NULL,\n\
               '{\"event_name\": \"page_view\", \"platform\": \"web\", \"url\": \"/pricing\"}',\n\
               '2026-04-02'\n\
             );",
        )
        .expect("seed raw.events");
    }

    let config = Arc::new(Config::load(project_dir).expect("load config"));

    // Window 1: day 1 alone — first-run CREATE, no MERGE yet.
    {
        let (db, graph) = build_db_and_graph(project_dir, &config);
        let backend_slot: Arc<Mutex<Option<Arc<RecordingBackend>>>> = Arc::new(Mutex::new(None));
        let factory = RecordingBackendFactory {
            db_path: db_path.clone(),
            backend: Arc::clone(&backend_slot),
        };
        execute_project(
            "events-deduped-storm-run-1".to_string(),
            make_request("dev", "2026-04-01", "2026-04-02"),
            Arc::clone(&config),
            graph,
            db,
            project_dir,
            &factory,
            &smelt_runtime::NoOpReporter,
            CancellationToken::new(),
        )
        .await
        .expect("day 1 (create) must run");
    }

    // Window 2: day 2 — the redelivery-storm step, a single MERGE.
    let (db, graph) = build_db_and_graph(project_dir, &config);
    let backend_slot: Arc<Mutex<Option<Arc<RecordingBackend>>>> = Arc::new(Mutex::new(None));
    let factory = RecordingBackendFactory {
        db_path: db_path.clone(),
        backend: Arc::clone(&backend_slot),
    };
    execute_project(
        "events-deduped-storm-run-2".to_string(),
        make_request("dev", "2026-04-02", "2026-04-03"),
        Arc::clone(&config),
        graph,
        db,
        project_dir,
        &factory,
        &smelt_runtime::NoOpReporter,
        CancellationToken::new(),
    )
    .await
    .expect("day 2 (redelivery-storm merge) must run");

    let backend = backend_slot
        .lock()
        .unwrap()
        .clone()
        .expect("backend recorded");
    let groups = backend.recorded_groups();
    let merge_group = groups
        .iter()
        .find(|g| g.statements[0].sql.starts_with("MERGE INTO"))
        .expect("day 2 must execute exactly one MERGE group");
    let merge_sql = &merge_group.statements[0].sql;

    assert!(
        merge_sql.contains("BETWEEN"),
        "the composed model's merge must carry the route-3 recurrence-bounded slice on the \
         target read: {merge_sql}"
    );
    assert!(
        merge_sql.contains("IS DISTINCT FROM"),
        "the composed model's merge must carry the suppression arm: {merge_sql}"
    );

    // Zero-write proof: reissue the exact statement text the run recorded
    // — DuckDB's own `MERGE` returns the count of rows it actually
    // modified (`crates/smelt-runtime/tests/technique_lowering.rs::
    // merge_affected_row_count`'s own technique). The run already brought
    // the target to its converged state, so replaying the identical
    // statement now must match every row (`event_id` 1's redelivered
    // duplicate, `event_id` 2 already inserted) but write none of them.
    let replay = backend
        .execute_sql(merge_sql)
        .await
        .expect("replaying the recorded merge must succeed");
    let batch = replay.first().expect("MERGE returns one Count row");
    let affected = batch
        .column(0)
        .as_any()
        .downcast_ref::<Int64Array>()
        .expect("Count column is Int64")
        .value(0);
    assert_eq!(
        affected, 0,
        "replaying day 2's already-converged merge must write zero rows — the redelivery \
         storm's unchanged payload must be fully suppressed"
    );

    // Result-equivalence: the maintained state must still equal a full
    // refresh of the model's own MIN-fold dedup over every seeded row.
    assert!(
        multiset_equal(
            backend.as_ref(),
            "SELECT event_id, device_id, user_id, first_seen_date FROM main.silver_events_deduped",
            "SELECT event_id, MIN(device_id) AS device_id, MIN(user_id) AS user_id, \
             MIN(CAST(event_date AS DATE)) AS first_seen_date FROM raw.events GROUP BY event_id"
        )
        .await,
        "the composed suppressed-merge run must still reproduce a full refresh"
    );
}

// =============================================================================
// T3 — delta-restricted region recompute over a model edge (`docs/plans/
// 20260715-composed-axes-conditional-maintenance.md` Phase E3): the
// statements `maintenance_driver::execute_delete_insert_with_delta_
// restriction` actually executes must be byte-identical to a direct call of
// `emit_delete_insert_delta_restricted`/`emit_delete_insert` with the same
// inputs — the same proof shape as the suppressed-MERGE and staged-
// candidate legs above.
// =============================================================================
