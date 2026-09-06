use super::*;

/// The region DELETE+INSERT family (`IncrementalStrategy::DeleteInsert`):
/// every statement `execute_project` actually sends to the DuckDB
/// connection for a timeseries-partitioned batched model must be
/// byte-identical to `emit_delete_insert` called directly with that batch's
/// own inputs (table, partition column, region, compiled SQL).
#[tokio::test]
async fn region_recompute_statements_come_from_the_emitter() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project_dir = tmp.path();
    std::fs::create_dir_all(project_dir.join("models")).unwrap();

    // Self-contained: no upstream ref/source needed to exercise the region
    // DELETE+INSERT family — the output clamp wraps the model's own SELECT
    // regardless of where its data comes from.
    write_model(
        project_dir,
        "daily_events",
        "---\n\
         materialization: table\n\
         refresh: incremental\n\
         grain: partition\n\
         timeseries:\n\
         \x20\x20partition_column: event_date\n\
         \x20\x20event_time_column: event_date\n\
         \x20\x20granularity: day\n\
         ---\n\
         SELECT * FROM (VALUES (DATE '2024-01-01', 10), (DATE '2024-01-02', 20)) AS t(event_date, amount)",
    );

    let db_path = project_dir.join("run.duckdb");
    let smelt_yml = format!(
        "name: statement_parity_test\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: {db}\n    schema: main\ndefault_materialization: table\ntarget: dev\n",
        db = db_path.display()
    );
    std::fs::write(project_dir.join("smelt.yml"), &smelt_yml).unwrap();

    let config = Arc::new(Config::load(project_dir).expect("load config"));

    // Run 1: the table does not exist yet — this run always hits the
    // `create_table_as` first-run path, never `delete_and_insert_transactional`.
    // Statement parity for this family is a *second-run* concern.
    {
        let (db, graph) = build_db_and_graph(project_dir, &config);
        let backend_slot: Arc<Mutex<Option<Arc<RecordingBackend>>>> = Arc::new(Mutex::new(None));
        let factory = RecordingBackendFactory {
            db_path: db_path.clone(),
            backend: Arc::clone(&backend_slot),
        };
        execute_project(
            "statement-parity-run-1".to_string(),
            make_request("dev", "2024-01-01", "2024-01-02"),
            Arc::clone(&config),
            graph,
            db,
            project_dir,
            &factory,
            &smelt_runtime::NoOpReporter,
            CancellationToken::new(),
        )
        .await
        .expect("execute_project run 1 (first-run create)");
    }

    // Run 2: the table exists — this run must dispatch `IncrementalStrategy::
    // DeleteInsert`, and its statements are what this test asserts against.
    let (db, graph) = build_db_and_graph(project_dir, &config);
    let backend_slot: Arc<Mutex<Option<Arc<RecordingBackend>>>> = Arc::new(Mutex::new(None));
    let factory = RecordingBackendFactory {
        db_path: db_path.clone(),
        backend: Arc::clone(&backend_slot),
    };

    let request = make_request("dev", "2024-01-01", "2024-01-03");
    let cancel = CancellationToken::new();
    let outcome = execute_project(
        "statement-parity-run-2".to_string(),
        request,
        Arc::clone(&config),
        graph,
        db,
        project_dir,
        &factory,
        &smelt_runtime::NoOpReporter,
        cancel,
    )
    .await
    .expect("execute_project run 2 (incremental)");

    assert!(
        outcome.models.contains_key("daily_events"),
        "daily_events must have run: {:?}",
        outcome.models.keys().collect::<Vec<_>>()
    );

    let backend = backend_slot
        .lock()
        .unwrap()
        .clone()
        .expect("backend recorded");
    let groups = backend.recorded_groups();
    assert!(
        !groups.is_empty(),
        "at least one DELETE+INSERT group must have executed"
    );

    for group in &groups {
        assert!(
            group.transactional,
            "region DELETE+INSERT must be transactional"
        );
        assert_eq!(group.statements.len(), 2);
        assert!(group.statements[0]
            .sql
            .starts_with("DELETE FROM main.daily_events WHERE"));
        assert!(group.statements[1]
            .sql
            .starts_with("INSERT INTO main.daily_events "));

        // Re-derive the same group directly from the emitter, from the
        // executed statements' own region literals (parsed back out of the
        // DELETE text) plus the INSERT's own body — proving the executed
        // text is exactly what the emitter produces, not merely
        // emitter-shaped.
        let delete_sql = &group.statements[0].sql;
        let where_clause = delete_sql
            .strip_prefix("DELETE FROM main.daily_events WHERE ")
            .expect("delete shape");
        // where_clause: "event_date >= 'START' AND event_date < 'END'"
        let parts: Vec<&str> = where_clause.split(" AND ").collect();
        let start_lit = parts[0]
            .strip_prefix("event_date >= ")
            .expect("start literal");
        let end_lit = parts[1].strip_prefix("event_date < ").expect("end literal");
        let body = group.statements[1]
            .sql
            .strip_prefix("INSERT INTO main.daily_events ")
            .expect("insert shape");

        let region = Region {
            start: start_lit.to_string(),
            end: end_lit.to_string(),
        };
        let expected = emit_delete_insert(
            "main.daily_events",
            "event_date",
            &region,
            body,
            MaintenanceDialect::DuckDb,
        );
        assert_eq!(
            &expected, group,
            "executed group must be byte-identical to a direct emitter call over the same inputs"
        );
    }

    // Result-equivalence: the region DELETE+INSERT statements the run
    // actually executed must leave `daily_events` multiset-equal to a full
    // refresh of the model's own SQL — the technique the plan describes
    // (`docs/specs/incremental_models.md` §"Statement emission (single
    // owner)") reproduces a full refresh, not merely emitter-shaped text.
    let full_refresh_sql = "SELECT * FROM (VALUES (DATE '2024-01-01', 10), \
                             (DATE '2024-01-02', 20)) AS t(event_date, amount)";
    assert!(
        multiset_equal(
            backend.as_ref(),
            "SELECT * FROM main.daily_events",
            full_refresh_sql
        )
        .await,
        "the DELETE+INSERT statements execute_project actually ran must reproduce a full refresh"
    );
}

/// State residency (`docs/outcomes/20260904-state-residency/outcome.md`
/// criterion 1): each DuckDB DeleteInsert batch's own reconciliation-ledger
/// reset — the idempotent `_smelt_ledger` DDL plus this batch's own
/// `[start, end)` region-recompute reset — must be sent to the connection
/// as raw SQL byte-identical to `generate_ledger_table_ddl`/
/// `generate_ledger_recompute_reset_sqls`'s own output, and must never
/// appear inside the write's own `StatementGroup` (bookkeeping never leaks
/// into the emitted write, `docs/specs/incremental_models.md` §"Statement
/// emission (single owner)").
#[tokio::test]
async fn ledger_recompute_reset_statements_come_from_the_state_builder() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project_dir = tmp.path();
    std::fs::create_dir_all(project_dir.join("models")).unwrap();

    write_model(
        project_dir,
        "daily_events",
        "---\n\
         materialization: table\n\
         refresh: incremental\n\
         grain: partition\n\
         timeseries:\n\
         \x20\x20partition_column: event_date\n\
         \x20\x20event_time_column: event_date\n\
         \x20\x20granularity: day\n\
         ---\n\
         SELECT * FROM (VALUES (DATE '2024-01-01', 10), (DATE '2024-01-02', 20)) AS t(event_date, amount)",
    );

    let db_path = project_dir.join("run.duckdb");
    let smelt_yml = format!(
        "name: ledger_reset_test\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: {db}\n    schema: main\ndefault_materialization: table\ntarget: dev\n",
        db = db_path.display()
    );
    std::fs::write(project_dir.join("smelt.yml"), &smelt_yml).unwrap();

    let config = Arc::new(Config::load(project_dir).expect("load config"));

    // Run 1: first-run create — no ledger reset yet (the target doesn't
    // exist, so this run never reaches the DeleteInsert branch).
    {
        let (db, graph) = build_db_and_graph(project_dir, &config);
        let backend_slot: Arc<Mutex<Option<Arc<RecordingBackend>>>> = Arc::new(Mutex::new(None));
        let factory = RecordingBackendFactory {
            db_path: db_path.clone(),
            backend: Arc::clone(&backend_slot),
        };
        execute_project(
            "ledger-reset-run-1".to_string(),
            make_request("dev", "2024-01-01", "2024-01-02"),
            Arc::clone(&config),
            graph,
            db,
            project_dir,
            &factory,
            &smelt_runtime::NoOpReporter,
            CancellationToken::new(),
        )
        .await
        .expect("run 1 (first-run create)");
    }

    // Run 2: the table exists — two daily batches dispatch `IncrementalStrategy::
    // DeleteInsert`, each recording its own ledger reset.
    let (db, graph) = build_db_and_graph(project_dir, &config);
    let backend_slot: Arc<Mutex<Option<Arc<RecordingBackend>>>> = Arc::new(Mutex::new(None));
    let factory = RecordingBackendFactory {
        db_path: db_path.clone(),
        backend: Arc::clone(&backend_slot),
    };
    execute_project(
        "ledger-reset-run-2".to_string(),
        make_request("dev", "2024-01-01", "2024-01-03"),
        Arc::clone(&config),
        graph,
        db,
        project_dir,
        &factory,
        &smelt_runtime::NoOpReporter,
        CancellationToken::new(),
    )
    .await
    .expect("run 2 (incremental)");

    let backend = backend_slot
        .lock()
        .unwrap()
        .clone()
        .expect("backend recorded");
    let sql_log = backend.recorded_sql();

    let ensure_ddl = smelt_state::ddl_duckdb::generate_ledger_table_ddl("main");
    assert!(
        sql_log.iter().any(|s| s == &ensure_ddl),
        "the ledger's idempotent ensure DDL must be sent as raw SQL byte-identical to \
         `generate_ledger_table_ddl`: {sql_log:?}"
    );

    // No `batch_size_days` is set, so the whole `[start, end)` request range
    // runs as a single batch, not one batch per day.
    let expected_reset = smelt_state::ddl_duckdb::generate_ledger_recompute_reset_sqls(
        "main",
        "daily_events",
        "{*}",
        "2024-01-01",
        "2024-01-03",
        "self",
        "2024-01-03",
    );
    for stmt in &expected_reset {
        assert!(
            sql_log.contains(stmt),
            "the batch must record its ledger reset statement byte-identical to \
             `generate_ledger_recompute_reset_sqls`: {stmt}\nrecorded: {sql_log:?}"
        );
    }

    // Bookkeeping never leaks into the emitted write's own StatementGroup.
    let groups = backend.recorded_groups();
    for group in &groups {
        for stmt in &group.statements {
            assert!(
                !stmt.sql.contains("_smelt_ledger"),
                "ledger bookkeeping must never appear inside a maintenance StatementGroup: {}",
                stmt.sql
            );
        }
    }
}

/// First-run bootstrap for a **self-referential** partition-grain model
/// (`docs/specs/incremental_shapes.md` §"First-run and backfill" — "First-run
/// bootstrap for a self-referential model"): building from scratch (no
/// pre-seeded target table) must emit exactly ONE statement group before
/// any region `DELETE`+`INSERT` — a plain `CREATE TABLE main.running_balance
/// (…)` with no `SELECT` — byte-identical to a direct call of
/// `emit_create_empty_table` with the same table name/columns/dialect.
/// Every batch's own region `DELETE`+`INSERT` group after it must still
/// match `emit_delete_insert`, exactly like the non-self-referential family
/// above — the bootstrap only replaces the otherwise-impossible first-run
/// `CREATE TABLE … AS SELECT …`, it does not change any later batch's
/// technique.
#[tokio::test]
async fn self_referential_bootstrap_statements_come_from_the_emitter() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project_dir = tmp.path();
    std::fs::create_dir_all(project_dir.join("models/sources")).unwrap();

    write_model(
        project_dir,
        "running_balance",
        "---\n\
         materialization: table\n\
         refresh: incremental\n\
         grain: partition\n\
         timeseries:\n\
         \x20\x20partition_column: d\n\
         \x20\x20event_time_column: d\n\
         \x20\x20granularity: day\n\
         maintenance:\n\
         \x20\x20scan_bounds:\n\
         \x20\x20\x20\x20per_source:\n\
         \x20\x20\x20\x20\x20\x20transactions:\n\
         \x20\x20\x20\x20\x20\x20\x20\x20allow_full_scan: true\n\
         ---\n\
         SELECT d, balance FROM (\n\
         \x20\x20SELECT\n\
         \x20\x20\x20\x20t.d AS d,\n\
         \x20\x20\x20\x20COALESCE(bal.balance, 0) + SUM(t.amt) AS balance\n\
         \x20\x20FROM smelt.sources.transactions t\n\
         \x20\x20LEFT JOIN smelt.running_balance bal\n\
         \x20\x20\x20\x20ON bal.d >= t.d - INTERVAL '1 day' AND bal.d < t.d\n\
         \x20\x20GROUP BY t.d, bal.balance\n\
         ) inner_balance",
    );
    std::fs::write(
        project_dir.join("models/sources/transactions.yml"),
        "description: statement-parity self-ref source.\n\
         mutation_profile: append_only\n\
         columns:\n\
         \x20\x20- name: d\n\
         \x20\x20\x20\x20type: DATE\n\
         \x20\x20- name: amt\n\
         \x20\x20\x20\x20type: DOUBLE\n",
    )
    .unwrap();

    let db_path = project_dir.join("run.duckdb");
    let smelt_yml = format!(
        "name: statement_parity_self_ref_test\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: {db}\n    schema: main\ndefault_materialization: table\ntarget: dev\n",
        db = db_path.display()
    );
    std::fs::write(project_dir.join("smelt.yml"), &smelt_yml).unwrap();

    // Seed the source table only — deliberately NO pre-created
    // `main.running_balance` target, proving the bootstrap builds it.
    {
        let conn = duckdb::Connection::open(&db_path).expect("open duckdb");
        conn.execute_batch(
            "CREATE SCHEMA IF NOT EXISTS main;\n\
             CREATE TABLE main.sources_transactions (d DATE, amt DOUBLE);\n\
             INSERT INTO main.sources_transactions VALUES \
             (DATE '2024-01-01', 10.0), (DATE '2024-01-02', 5.0);",
        )
        .expect("seed source table");
    }

    let config = Arc::new(Config::load(project_dir).expect("load config"));
    let (db, graph) = build_db_and_graph(project_dir, &config);
    let backend_slot: Arc<Mutex<Option<Arc<RecordingBackend>>>> = Arc::new(Mutex::new(None));
    let factory = RecordingBackendFactory {
        db_path: db_path.clone(),
        backend: Arc::clone(&backend_slot),
    };

    execute_project(
        "statement-parity-self-ref-run".to_string(),
        make_request("dev", "2024-01-01", "2024-01-03"),
        Arc::clone(&config),
        graph,
        db,
        project_dir,
        &factory,
        &smelt_runtime::NoOpReporter,
        CancellationToken::new(),
    )
    .await
    .expect("execute_project self-referential from-scratch run");

    let backend = backend_slot
        .lock()
        .unwrap()
        .clone()
        .expect("backend recorded");
    let groups = backend.recorded_groups();
    assert!(
        !groups.is_empty(),
        "at least the bootstrap CREATE TABLE group must have executed"
    );

    // First group: the bootstrap, non-transactional, exactly one
    // `CREATE TABLE main.running_balance (…)` statement with no `SELECT`.
    let bootstrap = &groups[0];
    assert!(
        !bootstrap.transactional,
        "the bootstrap CREATE TABLE is not a DELETE+INSERT pair"
    );
    assert_eq!(bootstrap.statements.len(), 1);
    let bootstrap_sql = &bootstrap.statements[0].sql;
    assert!(
        bootstrap_sql.starts_with("CREATE TABLE main.running_balance ("),
        "bootstrap statement: {bootstrap_sql}"
    );
    assert!(
        !bootstrap_sql.contains("SELECT"),
        "the bootstrap must be a plain empty CREATE TABLE, not a CREATE TABLE … AS SELECT: \
         {bootstrap_sql}"
    );

    // Re-derive the same statement directly from the emitter over the
    // columns parsed back out of the executed DDL text, proving byte
    // parity rather than merely emitter-shaped text.
    let col_defs = bootstrap_sql
        .strip_prefix("CREATE TABLE main.running_balance (")
        .and_then(|s| s.strip_suffix(')'))
        .expect("bootstrap DDL shape");
    let columns: Vec<(String, smelt_types::DataType)> = col_defs
        .split(", ")
        .map(|col| {
            let (name, ty) = col.split_once(' ').expect("column definition shape");
            (
                name.to_string(),
                smelt_types::parse_type(ty).expect("column type text"),
            )
        })
        .collect();
    let expected = smelt_logical::maintenance::emit::emit_create_empty_table(
        "main.running_balance",
        &columns,
        MaintenanceDialect::DuckDb,
    );
    assert_eq!(
        &expected, bootstrap,
        "executed bootstrap group must be byte-identical to a direct emitter call over the \
         same table/columns"
    );

    // Every subsequent group is the ordinary region DELETE+INSERT family —
    // the bootstrap only replaces the impossible first-run CTAS, it does
    // not change any later batch's technique.
    for group in &groups[1..] {
        assert!(
            group.transactional,
            "region DELETE+INSERT must be transactional"
        );
        assert_eq!(group.statements.len(), 2);
        assert!(group.statements[0]
            .sql
            .starts_with("DELETE FROM main.running_balance WHERE"));
        assert!(group.statements[1]
            .sql
            .starts_with("INSERT INTO main.running_balance "));
    }

    // Result-equivalence: the maintained trajectory must equal a full
    // sequential re-derivation from the source's current contents.
    assert!(
        multiset_equal(
            backend.as_ref(),
            "SELECT balance FROM main.running_balance WHERE d = DATE '2024-01-01'",
            "SELECT 10.0 AS balance",
        )
        .await,
        "day 1 balance must equal the sequential expectation"
    );
    assert!(
        multiset_equal(
            backend.as_ref(),
            "SELECT balance FROM main.running_balance WHERE d = DATE '2024-01-02'",
            "SELECT 15.0 AS balance",
        )
        .await,
        "day 2 balance must equal the sequential expectation"
    );
}

/// The keyed fold family (`refresh: keyed`, `grain: key`): every statement
/// `execute_project` sends for the windowed-keyed-maintenance driver's
/// steps — the first-run `CREATE TABLE … AS` and each following step's
/// `MERGE` — must be byte-identical to `emit_create_table_as`/
/// `emit_keyed_fold` called directly with that step's own inputs.
///
/// The fixture uses only `MIN`/`MAX` aggregator columns (no `SUM`), so the
/// cell grades `Grade::Idempotent`
/// (`WindowedKeyedRule::ledger_grade` — "additive iff any combiner is
/// `Sum`") and every step's create-or-merge action routes through
/// `Backend::execute_statement_group`, the same funnel the region family
/// uses — the `Grade::Additive` ledger-interleaved path
/// (`Backend::fold_ledger_delta`) is untouched by this phase
/// (`docs/plans/20260710-emit-unification.md` Phase 2 implementation
/// shape).
#[tokio::test]
async fn keyed_fold_statements_come_from_the_emitter() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project_dir = tmp.path();
    std::fs::create_dir_all(project_dir.join("models")).unwrap();

    write_model(
        project_dir,
        "events",
        "---\n\
         materialization: table\n\
         timeseries:\n\
         \x20\x20partition_column: event_date\n\
         \x20\x20event_time_column: event_date\n\
         \x20\x20granularity: day\n\
         ---\n\
         SELECT * FROM (VALUES \
         (DATE '2024-01-01', 1, TIMESTAMP '2024-01-01 01:00:00'), \
         (DATE '2024-01-02', 1, TIMESTAMP '2024-01-02 02:00:00'), \
         (DATE '2024-01-02', 2, TIMESTAMP '2024-01-02 03:00:00')) \
         AS t(event_date, device_id, event_ts)",
    );
    write_model(
        project_dir,
        "device_user_edges",
        "---\n\
         materialization: table\n\
         refresh: incremental\n\
         grain: key\n\
         ---\n\
         SELECT device_id, MIN(event_ts) AS first_seen, MAX(event_ts) AS last_seen \
         FROM smelt.events GROUP BY device_id",
    );

    let db_path = project_dir.join("run.duckdb");
    let smelt_yml = format!(
        "name: keyed_statement_parity_test\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: {db}\n    schema: main\ndefault_materialization: table\ntarget: dev\n",
        db = db_path.display()
    );
    std::fs::write(project_dir.join("smelt.yml"), &smelt_yml).unwrap();

    let config = Arc::new(Config::load(project_dir).expect("load config"));

    let (db, graph) = build_db_and_graph(project_dir, &config);
    let backend_slot: Arc<Mutex<Option<Arc<RecordingBackend>>>> = Arc::new(Mutex::new(None));
    let factory = RecordingBackendFactory {
        db_path: db_path.clone(),
        backend: Arc::clone(&backend_slot),
    };

    // One window covering both driving-source partitions: step 1
    // (2024-01-01) hits the first-run CREATE arm; step 2 (2024-01-02) hits
    // the MERGE arm.
    let request = make_request("dev", "2024-01-01", "2024-01-03");
    let outcome = execute_project(
        "keyed-statement-parity-run".to_string(),
        request,
        Arc::clone(&config),
        graph,
        db,
        project_dir,
        &factory,
        &smelt_runtime::NoOpReporter,
        CancellationToken::new(),
    )
    .await
    .expect("execute_project (keyed)");

    assert!(
        outcome.models.contains_key("device_user_edges"),
        "device_user_edges must have run: {:?}",
        outcome.models.keys().collect::<Vec<_>>()
    );

    let backend = backend_slot
        .lock()
        .unwrap()
        .clone()
        .expect("backend recorded");
    let groups = backend.recorded_groups();
    assert_eq!(
        groups.len(),
        2,
        "two steps must each execute exactly one statement group: {:?}",
        groups
    );

    // Step 1: first-run CREATE TABLE ... AS.
    let create_sql = &groups[0].statements[0].sql;
    assert_eq!(groups[0].statements.len(), 1);
    assert!(
        create_sql.starts_with("CREATE TABLE main.device_user_edges AS "),
        "unexpected create statement: {create_sql}"
    );
    let create_select = create_sql
        .strip_prefix("CREATE TABLE main.device_user_edges AS ")
        .expect("create shape");
    let expected_create = emit_create_table_as(
        "main.device_user_edges",
        create_select,
        MaintenanceDialect::DuckDb,
    );
    assert_eq!(
        &expected_create, &groups[0],
        "executed CREATE group must be byte-identical to a direct emitter call"
    );

    // Step 2: combiner-aware MERGE. `first_seen`/`last_seen` are both
    // Comparable (MIN/MAX are registry-backed deterministic functions) over
    // a proven `device_id` key, so a real run now resolves `Suppressed`
    // (`docs/plans/20260715-composed-axes-conditional-maintenance.md`
    // Phase C6 — `resolve_cumulative_write_suppression`, wired into
    // `execute_cumulative_aggregate`) — the matched arm carries an `IS
    // DISTINCT FROM` guard over both fold columns.
    let merge_sql = &groups[1].statements[0].sql;
    assert_eq!(groups[1].statements.len(), 1);
    let prefix = "MERGE INTO main.device_user_edges AS target USING (";
    let suffix = ") AS delta ON target.device_id = delta.device_id \
                  WHEN MATCHED AND (target.first_seen IS DISTINCT FROM (LEAST(target.first_seen, \
                  delta.first_seen)) OR target.last_seen IS DISTINCT FROM (GREATEST(target.\
                  last_seen, delta.last_seen))) THEN UPDATE SET first_seen = LEAST(target.\
                  first_seen, delta.first_seen), last_seen = GREATEST(target.last_seen, \
                  delta.last_seen) WHEN NOT MATCHED THEN INSERT *";
    assert!(
        merge_sql.starts_with(prefix) && merge_sql.ends_with(suffix),
        "unexpected merge statement: {merge_sql}"
    );
    let delta_select = &merge_sql[prefix.len()..merge_sql.len() - suffix.len()];
    let expected_merge = emit_keyed_fold_suppressed(
        "main.device_user_edges",
        &["device_id".to_string()],
        &[
            (
                "first_seen".to_string(),
                "LEAST(target.first_seen, delta.first_seen)".to_string(),
            ),
            (
                "last_seen".to_string(),
                "GREATEST(target.last_seen, delta.last_seen)".to_string(),
            ),
        ],
        delta_select,
        None,
        &["first_seen".to_string(), "last_seen".to_string()],
        MaintenanceDialect::DuckDb,
    );
    assert_eq!(
        &expected_merge, &groups[1],
        "executed MERGE group must be byte-identical to a direct emitter call"
    );

    // Result-equivalence: the CREATE + MERGE statements the run actually
    // executed must leave `device_user_edges` multiset-equal to a full
    // refresh of the model's own aggregation over the driving source's
    // materialized output.
    assert!(
        multiset_equal(
            backend.as_ref(),
            "SELECT * FROM main.device_user_edges",
            "SELECT device_id, MIN(event_ts) AS first_seen, MAX(event_ts) AS last_seen \
             FROM main.events GROUP BY device_id"
        )
        .await,
        "the CREATE+MERGE statements execute_project actually ran must reproduce a full refresh"
    );
}

/// A `write: staged_candidate` pin (`docs/outcomes/
/// 20260815-definition-delta-migrate/phases/27g-plan.md`) on a `refresh:
/// keyed` model's driving-source cell must dispatch the merge-less
/// staged-candidate mechanism at run time instead of the ordinary `MERGE` —
/// even on a `MERGE`-capable backend (DuckDB), since an explicit pin is
/// never second-guessed. Same fixture as
/// `keyed_fold_statements_come_from_the_emitter` above, with one added
/// `maintenance.cells[]` pin.
#[tokio::test]
async fn staged_candidate_keyed_fold_statements_come_from_the_emitter() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project_dir = tmp.path();
    std::fs::create_dir_all(project_dir.join("models")).unwrap();

    write_model(
        project_dir,
        "events",
        "---\n\
         materialization: table\n\
         timeseries:\n\
         \x20\x20partition_column: event_date\n\
         \x20\x20event_time_column: event_date\n\
         \x20\x20granularity: day\n\
         ---\n\
         SELECT * FROM (VALUES \
         (DATE '2024-01-01', 1, TIMESTAMP '2024-01-01 01:00:00'), \
         (DATE '2024-01-02', 1, TIMESTAMP '2024-01-02 02:00:00'), \
         (DATE '2024-01-02', 2, TIMESTAMP '2024-01-02 03:00:00')) \
         AS t(event_date, device_id, event_ts)",
    );
    write_model(
        project_dir,
        "device_user_edges",
        "---\n\
         materialization: table\n\
         refresh: incremental\n\
         grain: key\n\
         maintenance:\n\
         \x20\x20cells:\n\
         \x20\x20\x20\x20- on: smelt.events\n\
         \x20\x20\x20\x20\x20\x20columns: []\n\
         \x20\x20\x20\x20\x20\x20write: staged_candidate\n\
         ---\n\
         SELECT device_id, MIN(event_ts) AS first_seen, MAX(event_ts) AS last_seen \
         FROM smelt.events GROUP BY device_id",
    );

    let db_path = project_dir.join("run.duckdb");
    let smelt_yml = format!(
        "name: staged_candidate_keyed_statement_parity_test\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: {db}\n    schema: main\ndefault_materialization: table\ntarget: dev\n",
        db = db_path.display()
    );
    std::fs::write(project_dir.join("smelt.yml"), &smelt_yml).unwrap();

    let config = Arc::new(Config::load(project_dir).expect("load config"));

    let (db, graph) = build_db_and_graph(project_dir, &config);
    let backend_slot: Arc<Mutex<Option<Arc<RecordingBackend>>>> = Arc::new(Mutex::new(None));
    let factory = RecordingBackendFactory {
        db_path: db_path.clone(),
        backend: Arc::clone(&backend_slot),
    };

    let request = make_request("dev", "2024-01-01", "2024-01-03");
    let outcome = execute_project(
        "staged-candidate-keyed-statement-parity-run".to_string(),
        request,
        Arc::clone(&config),
        graph,
        db,
        project_dir,
        &factory,
        &smelt_runtime::NoOpReporter,
        CancellationToken::new(),
    )
    .await
    .expect("execute_project (keyed, staged_candidate pin)");

    assert!(
        outcome.models.contains_key("device_user_edges"),
        "device_user_edges must have run: {:?}",
        outcome.models.keys().collect::<Vec<_>>()
    );

    let backend = backend_slot
        .lock()
        .unwrap()
        .clone()
        .expect("backend recorded");
    let groups = backend.recorded_groups();
    assert_eq!(
        groups.len(),
        2,
        "two steps must each execute exactly one statement group: {:?}",
        groups
    );

    // Step 1: unaffected by the pin — no target table yet, so the driver
    // still takes the plain create branch.
    assert_eq!(groups[0].statements.len(), 1);
    assert!(groups[0].statements[0]
        .sql
        .starts_with("CREATE TABLE main.device_user_edges AS "));

    // Step 2: the pin selects the merge-less staged-candidate group — five
    // statements, transactional as one unit — never the MERGE.
    let group = &groups[1];
    assert_eq!(
        group.statements.len(),
        5,
        "staged-candidate pin must yield a 5-statement group: {:?}",
        group
    );
    assert!(group.transactional);
    assert!(group.statements[0].sql.starts_with("CREATE TEMP TABLE"));
    assert!(!group.statements.iter().any(|s| s.sql.contains("MERGE")));

    let insert_candidates_sql = &group.statements[1].sql;
    let candidate_select = insert_candidates_sql
        .strip_prefix("INSERT INTO __smelt_staged_device_user_edges ")
        .expect("insert-candidates shape");

    let folds = vec![
        (
            "first_seen".to_string(),
            "LEAST(target.first_seen, delta.first_seen)".to_string(),
        ),
        (
            "last_seen".to_string(),
            "GREATEST(target.last_seen, delta.last_seen)".to_string(),
        ),
    ];
    // Recover the step's own compiled delta SELECT from the candidate
    // SELECT's own `FROM (<delta_sql>) AS delta LEFT JOIN` shape (templated
    // with a placeholder so the surrounding prefix/suffix are derived from
    // the single-owner emitter itself, never hand-duplicated), then rebuild
    // the exact group a direct emitter call over that delta produces.
    let placeholder = "__PLACEHOLDER_DELTA__";
    let templated = smelt_logical::maintenance::emit::keyed_fold_candidate_select(
        "main.device_user_edges",
        &["device_id".to_string()],
        &folds,
        placeholder,
        MaintenanceDialect::DuckDb,
    );
    let (prefix, suffix) = templated.split_once(placeholder).unwrap();
    let actual_delta_sql = candidate_select
        .strip_prefix(prefix)
        .and_then(|s| s.strip_suffix(suffix))
        .expect("candidate_select must match keyed_fold_candidate_select's own shape");

    let expected_candidate_select = smelt_logical::maintenance::emit::keyed_fold_candidate_select(
        "main.device_user_edges",
        &["device_id".to_string()],
        &folds,
        actual_delta_sql,
        MaintenanceDialect::DuckDb,
    );
    assert_eq!(
        candidate_select, expected_candidate_select,
        "the executed candidate SELECT must be byte-identical to a direct \
         keyed_fold_candidate_select call over the step's own delta"
    );
    let expected_group = smelt_logical::maintenance::emit::emit_staged_candidate_conditional(
        "main.device_user_edges",
        "__smelt_staged_device_user_edges",
        &["device_id".to_string()],
        &expected_candidate_select,
        &["first_seen".to_string(), "last_seen".to_string()],
        MaintenanceDialect::DuckDb,
    );
    assert_eq!(
        group, &expected_group,
        "executed staged-candidate group must be byte-identical to a direct emitter call"
    );

    // Result-equivalence: the staged-candidate write path must still
    // reproduce a full refresh of the model's own aggregation.
    assert!(
        multiset_equal(
            backend.as_ref(),
            "SELECT * FROM main.device_user_edges",
            "SELECT device_id, MIN(event_ts) AS first_seen, MAX(event_ts) AS last_seen \
             FROM main.events GROUP BY device_id"
        )
        .await,
        "the staged-candidate statements execute_project actually ran must reproduce a full \
         refresh"
    );
}
