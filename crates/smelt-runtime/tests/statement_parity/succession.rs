use super::*;
use std::path::PathBuf;

/// The succession-patch family (phase 5c, `docs/outcomes/
/// 20260906-scd2-keyed-succession/phases/05c-plan.md`): the event-delta
/// `SELECT`, the clock-tie probe and the patch group `execute_project`
/// actually sends to a real DuckDB connection over `tests/fixtures/
/// succession/` must be byte-identical to the single-owner emitters called
/// directly with the batch's own inputs (`crates/smelt-logical/src/
/// maintenance/emit/succession.rs`).
const SOURCE_TABLE: &str = "main.sources_customer_changes";
const PRESENTED_TABLE: &str = "main.customer_history";

fn succession_fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/succession")
        .canonicalize()
        .expect("tests/fixtures/succession exists")
}

fn stage_succession_source(db_path: &Path) {
    let conn = duckdb::Connection::open(db_path).expect("open duckdb");
    conn.execute_batch(&format!(
        "CREATE TABLE {SOURCE_TABLE} (customer_id INTEGER, changed_at TIMESTAMP, arrival_date \
         DATE, tier VARCHAR)"
    ))
    .expect("create source table");
}

fn insert_succession_event(
    db_path: &Path,
    id: i64,
    changed_at: &str,
    arrival_date: &str,
    tier: &str,
) {
    let conn = duckdb::Connection::open(db_path).expect("reopen duckdb");
    conn.execute_batch(&format!(
        "INSERT INTO {SOURCE_TABLE} VALUES ({id}, TIMESTAMP '{changed_at}', DATE \
         '{arrival_date}', '{tier}')"
    ))
    .expect("insert event");
}

/// Test 7: `succession_patch_executed_statements_match_the_emitters` — the
/// clock-tie probe and patch group recorded by [`RecordingBackend`] during a
/// real `execute_project` run over one window are byte-identical to direct
/// emitter calls built from the SAME known batch inputs (the fixture's own
/// recipe: `customer_id` key, `changed_at` clock, `tier` payload, `valid_to`
/// `{lead}`-derived, no delete filter).
#[tokio::test]
async fn succession_patch_executed_statements_match_the_emitters() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().join("project");
    column_scoped_merge::copy_dir_recursive(&succession_fixture_dir(), &project_dir);
    let db_path = tmp.path().join("run.duckdb");
    stage_succession_source(&db_path);
    insert_succession_event(&db_path, 1, "2026-01-01 08:00:00", "2026-01-01", "gold");

    let config = Arc::new(Config::load(&project_dir).expect("load smelt.yml"));
    let backend_slot: Arc<Mutex<Option<Arc<RecordingBackend>>>> = Arc::new(Mutex::new(None));
    let factory = RecordingBackendFactory {
        db_path: db_path.clone(),
        backend: Arc::clone(&backend_slot),
    };
    let (db, graph) = build_db_and_graph(&project_dir, &config);
    execute_project(
        "succession-statement-parity-run-1".to_string(),
        column_scoped_merge::select_request("dev", "customer_history", "2026-01-01", "2026-01-02"),
        Arc::clone(&config),
        graph,
        db,
        &project_dir,
        &factory,
        &smelt_runtime::NoOpReporter,
        CancellationToken::new(),
    )
    .await
    .expect("succession-patch run must succeed");

    let backend = backend_slot
        .lock()
        .unwrap()
        .clone()
        .expect("backend recorded");
    let groups = backend.recorded_groups();
    assert_eq!(
        groups.len(),
        1,
        "exactly one transactional patch group must have executed: {groups:?}"
    );
    let patch_group = &groups[0];

    let sql_log = backend.recorded_sql();
    let probe_sql = sql_log
        .iter()
        .find(|s| s.contains("violation_count"))
        .expect("the clock-tie probe must have executed via execute_sql");

    let window_predicate = "arrival_date >= DATE '2026-01-01' AND arrival_date < DATE '2026-01-02'";
    let projection = vec![
        ("customer_id".to_string(), "customer_id".to_string()),
        ("changed_at".to_string(), "changed_at".to_string()),
        ("tier".to_string(), "tier".to_string()),
    ];
    let expected_event_delta = smelt_logical::maintenance::emit::emit_succession_event_delta(
        SOURCE_TABLE,
        &projection,
        None,
        window_predicate,
    );

    let expected_probe = smelt_logical::maintenance::emit::emit_succession_clock_tie_probe(
        PRESENTED_TABLE,
        &["customer_id".to_string()],
        "changed_at",
        &["tier".to_string()],
        None,
        &expected_event_delta.sql,
        smelt_logical::maintenance::emit::MaintenanceDialect::DuckDb,
    );
    assert_eq!(probe_sql, &expected_probe.sql);

    let expected_patch_group = smelt_logical::maintenance::emit::emit_succession_patch(
        PRESENTED_TABLE,
        &["customer_id".to_string()],
        "changed_at",
        &["tier".to_string()],
        &[("valid_to".to_string(), "{lead}".to_string())],
        &[],
        None,
        &expected_event_delta.sql,
        smelt_logical::maintenance::emit::MaintenanceDialect::DuckDb,
    );
    assert_eq!(
        patch_group.statements.len(),
        expected_patch_group.statements.len()
    );
    for (actual, expected) in patch_group
        .statements
        .iter()
        .zip(expected_patch_group.statements.iter())
    {
        assert_eq!(actual.sql, expected.sql);
    }
    assert_eq!(
        patch_group.transactional,
        expected_patch_group.transactional
    );
}

/// Test 8: `succession_full_refresh_executed_statements_match_the_emitters`
/// — the rebuild group recorded under `full_refresh: true` is
/// byte-identical to a direct `emit_succession_full_rebuild` call over the
/// model's own compiled SQL.
#[tokio::test]
async fn succession_full_refresh_executed_statements_match_the_emitters() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().join("project");
    column_scoped_merge::copy_dir_recursive(&succession_fixture_dir(), &project_dir);
    let db_path = tmp.path().join("run.duckdb");
    stage_succession_source(&db_path);
    insert_succession_event(&db_path, 1, "2026-01-01 08:00:00", "2026-01-01", "gold");
    insert_succession_event(&db_path, 1, "2026-01-02 08:00:00", "2026-01-02", "silver");

    let config = Arc::new(Config::load(&project_dir).expect("load smelt.yml"));

    // Run 1: bootstrap via the ordinary patch loop over both windows, so
    // the full-refresh run below rebuilds a table that already exists.
    {
        let (db, graph) = build_db_and_graph(&project_dir, &config);
        execute_project(
            "succession-full-refresh-bootstrap".to_string(),
            column_scoped_merge::select_request(
                "dev",
                "customer_history",
                "2026-01-01",
                "2026-01-03",
            ),
            Arc::clone(&config),
            graph,
            db,
            &project_dir,
            &RecordingBackendFactory {
                db_path: db_path.clone(),
                backend: Arc::new(Mutex::new(None)),
            },
            &smelt_runtime::NoOpReporter,
            CancellationToken::new(),
        )
        .await
        .expect("bootstrap run must succeed");
    }

    let backend_slot: Arc<Mutex<Option<Arc<RecordingBackend>>>> = Arc::new(Mutex::new(None));
    let factory = RecordingBackendFactory {
        db_path: db_path.clone(),
        backend: Arc::clone(&backend_slot),
    };
    let (db, graph) = build_db_and_graph(&project_dir, &config);
    let mut request =
        column_scoped_merge::select_request("dev", "customer_history", "2026-01-01", "2026-01-03");
    request.full_refresh = true;
    execute_project(
        "succession-full-refresh-run".to_string(),
        request,
        Arc::clone(&config),
        graph,
        db,
        &project_dir,
        &factory,
        &smelt_runtime::NoOpReporter,
        CancellationToken::new(),
    )
    .await
    .expect("full-refresh run must succeed");

    let backend = backend_slot
        .lock()
        .unwrap()
        .clone()
        .expect("backend recorded");
    let groups = backend.recorded_groups();
    assert_eq!(
        groups.len(),
        1,
        "exactly one transactional rebuild group must have executed: {groups:?}"
    );
    let rebuild_group = &groups[0];

    // Recover the compiled SELECT the run actually used, from the executed
    // `CREATE TABLE ... AS <select>` statement's own text — the same
    // reconstruct-from-what-executed idiom `extract_affected_keys_select`
    // uses elsewhere in this suite, proving the emitter reproduces the
    // exact SQL a real run compiled and sent, cast-wrapping included.
    let create_prefix = format!("CREATE TABLE {PRESENTED_TABLE} AS ");
    let model_select_sql = rebuild_group.statements[0]
        .sql
        .strip_prefix(&create_prefix)
        .expect("rebuild group's first statement is the presented CREATE TABLE AS");

    let expected_group = smelt_logical::maintenance::emit::emit_succession_full_rebuild(
        PRESENTED_TABLE,
        model_select_sql,
        SOURCE_TABLE,
        &["customer_id".to_string()],
        "changed_at",
        None,
        "FALSE",
        smelt_logical::maintenance::emit::MaintenanceDialect::DuckDb,
    );
    assert_eq!(
        rebuild_group.statements.len(),
        expected_group.statements.len()
    );
    for (actual, expected) in rebuild_group
        .statements
        .iter()
        .zip(expected_group.statements.iter())
    {
        assert_eq!(actual.sql, expected.sql);
    }
    assert_eq!(rebuild_group.transactional, expected_group.transactional);
}

/// Test 9: `succession_patch_result_equals_full_refresh` — the Link-C
/// `multiset_equal` leg every other family in this suite carries: the
/// presented table a succession-patch run over two windows produces equals
/// the model SQL's own full-refresh oracle.
#[tokio::test]
async fn succession_patch_result_equals_full_refresh() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().join("project");
    column_scoped_merge::copy_dir_recursive(&succession_fixture_dir(), &project_dir);
    let db_path = tmp.path().join("run.duckdb");
    stage_succession_source(&db_path);
    insert_succession_event(&db_path, 1, "2026-01-01 08:00:00", "2026-01-01", "gold");
    insert_succession_event(&db_path, 1, "2026-01-02 08:00:00", "2026-01-02", "silver");

    let config = Arc::new(Config::load(&project_dir).expect("load smelt.yml"));
    let backend_slot: Arc<Mutex<Option<Arc<RecordingBackend>>>> = Arc::new(Mutex::new(None));
    let factory = RecordingBackendFactory {
        db_path: db_path.clone(),
        backend: Arc::clone(&backend_slot),
    };
    let (db, graph) = build_db_and_graph(&project_dir, &config);
    execute_project(
        "succession-result-parity-run-1".to_string(),
        column_scoped_merge::select_request("dev", "customer_history", "2026-01-01", "2026-01-02"),
        Arc::clone(&config),
        graph,
        db,
        &project_dir,
        &factory,
        &smelt_runtime::NoOpReporter,
        CancellationToken::new(),
    )
    .await
    .expect("run 1 must succeed");

    let (db, graph) = build_db_and_graph(&project_dir, &config);
    execute_project(
        "succession-result-parity-run-2".to_string(),
        column_scoped_merge::select_request("dev", "customer_history", "2026-01-02", "2026-01-03"),
        Arc::clone(&config),
        graph,
        db,
        &project_dir,
        &factory,
        &smelt_runtime::NoOpReporter,
        CancellationToken::new(),
    )
    .await
    .expect("run 2 must succeed");

    let backend = backend_slot
        .lock()
        .unwrap()
        .clone()
        .expect("backend recorded");
    let oracle = format!(
        "SELECT customer_id, changed_at, tier, LEAD(changed_at) OVER (PARTITION BY customer_id \
         ORDER BY changed_at) AS valid_to FROM {SOURCE_TABLE}"
    );
    assert!(
        multiset_equal(
            &*backend,
            "SELECT customer_id, changed_at, tier, valid_to FROM main.customer_history",
            &oracle,
        )
        .await,
        "the succession-patch run's result must equal the model SQL's own full-refresh oracle"
    );
}
