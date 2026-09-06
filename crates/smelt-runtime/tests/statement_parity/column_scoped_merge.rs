use super::*;

/// Copy `examples/timeseries` into a scratch directory so the run's
/// `.smelt/` state never lands inside the checked-in example (mirrors
/// `crates/smelt-runtime/tests/technique_lowering.rs`'s
/// `column_scoped_merge_e2e::copy_dir_recursive`).
pub(super) fn copy_dir_recursive(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).expect("create dst dir");
    for entry in std::fs::read_dir(src).expect("read src dir") {
        let entry = entry.expect("dir entry");
        let file_type = entry.file_type().expect("file type");
        let dst_path = dst.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_recursive(&entry.path(), &dst_path);
        } else {
            std::fs::copy(entry.path(), &dst_path).expect("copy file");
        }
    }
}

pub(super) fn select_request(target: &str, model: &str, start: &str, end: &str) -> ExecuteRequest {
    ExecuteRequest {
        target: target.to_string(),
        select: vec![model.to_string()],
        exclude: vec![],
        start: Some(start.to_string()),
        end: Some(end.to_string()),
        batch_size_days: None,
        per_partition: false,
        full_refresh: false,
        rebuild: false,
        dry_run: false,
        enforce_safety: false,
        allow_column_removal: false,
        allow_full_refresh: false,
        ephemeral_seed_ctes: vec![],
        run_checks: false,
        checks: vec![],
        jobs: None,
        retry_max: None,
        retry_backoff_ms: None,
        resume: false,
        technique_overrides: vec![],
    }
}

/// The column-scoped `MERGE` family (`Technique::ColumnScopedMerge`, MP11).
///
/// **Reachability note** (`docs/plans/20260808-membership-sensitivity.md`
/// Phase 2): before that plan, `examples/timeseries/daily_events_enriched`'s
/// `raw.users` mutation drove the `{user_name}` cell's live column-scoped
/// MERGE, and this test drove it end to end through `execute_project`. Phase
/// 1 of that plan derives membership sensitivity directly from the join's
/// `ON e.user_id = u.user_id` predicate (a row-admission read), which makes
/// `{user_name}` — and every other column group that same join admits —
/// membership-sensitive, so the cell now admits `Technique::DeleteInsert`,
/// never `ColumnScopedMerge`
/// (`technique_lowering.rs::real_fixture_examples_timeseries_admits_
/// membership_recompute_cell` proves the derivation). No fixture in this
/// workspace reaches `ColumnScopedMerge` today: value sensitivity alone,
/// without any row-admission read of the SAME mutable source, has no
/// currently-shipped shape (every `mutation_profile: mutable_snapshot`
/// dimension example workspaces ship is also the driving join's own
/// partner). `ColumnScopedMerge`'s emitter parity is therefore proven the
/// same way the family's OTHER legs in this file prove theirs when no real
/// fixture reaches them — a direct call of the single production dispatch
/// function ([`execute_column_scoped_merge_full`]) against a `RecordingBackend`,
/// asserting the executed `MERGE` is byte-identical to a direct
/// `emit_column_scoped_merge` call over the same inputs. Tracked as a real
/// reachability gap, not silently worked around: `docs/plans/
/// 20260808-membership-sensitivity.md`'s Deferred section and
/// `incremental_models.md` §Known Divergences (Phase 4 of that plan).
#[tokio::test]
async fn column_scoped_merge_statements_come_from_the_emitter() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("test.duckdb");
    let inner = DuckDbBackend::new(&db_path, "main")
        .await
        .expect("open duckdb");
    let backend = RecordingBackend::new(inner);

    backend
        .execute_sql(
            "CREATE TABLE main.daily_events_enriched (event_id INTEGER, user_id INTEGER, \
             user_name VARCHAR)",
        )
        .await
        .expect("create target table");
    backend
        .execute_sql("INSERT INTO main.daily_events_enriched VALUES (1, 1, 'Alice'), (2, 2, 'Bob')")
        .await
        .expect("seed target table");
    backend
        .execute_sql(
            "CREATE TABLE main.sources_raw_users (event_id INTEGER, user_id INTEGER, user_name \
             VARCHAR)",
        )
        .await
        .expect("create dim/source table");
    backend
        .execute_sql("INSERT INTO main.sources_raw_users VALUES (1, 1, 'Alicia'), (2, 2, 'Bob')")
        .await
        .expect("seed source table (user 1 mutated)");

    let dimension_batch_sql = "SELECT event_id, user_id, user_name FROM main.sources_raw_users";
    let suppression = smelt_logical::maintenance::choice::WriteSuppression::Unconditional {
        why: "unit-level parity probe — the family's Unconditional variant is exercised, the \
              Suppressed one by `suppressed_column_scoped_merge_statements_come_from_the_emitter`"
            .to_string(),
    };
    let window = smelt_backend::PartitionRange {
        column: String::new(),
        start: "2025-01-10".to_string(),
        end: "2025-01-11".to_string(),
        axis: smelt_backend::PartitionAxis::Calendar,
    };
    smelt_runtime::maintenance_driver::execute_column_scoped_merge_full(
        &backend,
        "main",
        "daily_events_enriched",
        &["event_id".to_string()],
        dimension_batch_sql,
        &[],
        &suppression,
        &window,
        &no_retry_policy(),
    )
    .await
    .expect("column-scoped merge must succeed");

    let groups = backend.recorded_groups();
    let merge_groups: Vec<_> = groups
        .iter()
        .filter(|g| g.statements[0].sql.starts_with("MERGE INTO"))
        .collect();
    assert_eq!(
        merge_groups.len(),
        1,
        "exactly one column-scoped MERGE group must have executed: {:?}",
        groups
    );

    let group = merge_groups[0];
    assert!(
        !group.transactional,
        "a single-statement group needs no transaction wrapper"
    );
    assert_eq!(group.statements.len(), 1);

    let expected = emit_column_scoped_merge(
        "main.daily_events_enriched",
        &["event_id".to_string()],
        dimension_batch_sql,
        &[],
        MaintenanceDialect::DuckDb,
    );
    assert_eq!(
        &expected, group,
        "executed MERGE group must be byte-identical to a direct emitter call over the same inputs"
    );

    // Result-equivalence: the column-scoped MERGE actually executed must
    // leave the target multiset-equal to a full refresh of the source.
    assert!(
        multiset_equal(
            &backend,
            "SELECT * FROM main.daily_events_enriched",
            "SELECT event_id, user_id, user_name FROM main.sources_raw_users"
        )
        .await,
        "the column-scoped MERGE actually executed must reproduce a full refresh"
    );
}

/// Phase C4 (`docs/plans/20260715-composed-axes-conditional-maintenance.md`)
/// — the change-suppressed column-scoped MERGE (T1) dispatches through
/// `maintenance_driver::execute_column_scoped_merge_full` exactly like the
/// unconditional variant above, but building its `StatementGroup` via
/// `emit_column_scoped_merge_suppressed` and handing it straight to
/// `Backend::execute_statement_group` — never `Backend::merge_into` (which
/// would route back through the unconditional emitter). This proves the
/// EXECUTED statement text is byte-identical to a direct call of
/// `emit_column_scoped_merge_suppressed` over the same inputs, the same
/// property `column_scoped_merge_statements_come_from_the_emitter` proves
/// for the unconditional variant.
#[tokio::test]
async fn suppressed_column_scoped_merge_statements_come_from_the_emitter() {
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
        .execute_sql("INSERT INTO main.dim_users VALUES (1, 'bronze'), (2, 'silver')")
        .await
        .expect("seed target table");
    backend
        .execute_sql("CREATE TABLE main.sources_users (user_id BIGINT, tier VARCHAR)")
        .await
        .expect("create dim table");
    backend
        .execute_sql("INSERT INTO main.sources_users VALUES (1, 'gold'), (2, 'silver')")
        .await
        .expect("seed dim table (user_id=1 mutated)");

    let dimension_batch_sql = "SELECT u.user_id, u.tier FROM main.sources_users u";
    let suppression = smelt_logical::maintenance::choice::WriteSuppression::Suppressed {
        compared_columns: vec!["tier".to_string()],
    };

    let window = smelt_backend::PartitionRange {
        column: String::new(),
        start: "2026-01-01".to_string(),
        end: "2026-01-02".to_string(),
        axis: smelt_backend::PartitionAxis::Calendar,
    };
    smelt_runtime::maintenance_driver::execute_column_scoped_merge_full(
        &backend,
        "main",
        "dim_users",
        &["user_id".to_string()],
        dimension_batch_sql,
        &[],
        &suppression,
        &window,
        &no_retry_policy(),
    )
    .await
    .expect("suppressed column-scoped merge must succeed");

    let groups = backend.recorded_groups();
    let merge_groups: Vec<_> = groups
        .iter()
        .filter(|g| g.statements[0].sql.starts_with("MERGE INTO"))
        .collect();
    assert_eq!(merge_groups.len(), 1, "exactly one MERGE group: {groups:?}");
    let group = merge_groups[0];
    assert!(!group.transactional);
    assert_eq!(group.statements.len(), 1);

    let expected = smelt_logical::maintenance::emit::emit_column_scoped_merge_suppressed(
        "main.dim_users",
        &["user_id".to_string()],
        dimension_batch_sql,
        &["tier".to_string()],
        &[],
        MaintenanceDialect::DuckDb,
    );
    assert_eq!(
        &expected, group,
        "executed suppressed MERGE group must be byte-identical to a direct emitter call over \
         the same inputs"
    );

    // Result-equivalence: the same full-refresh oracle property the
    // unconditional leg proves.
    assert!(
        multiset_equal(
            &backend,
            "SELECT * FROM main.dim_users",
            "SELECT user_id, tier FROM main.sources_users"
        )
        .await,
        "the suppressed MERGE must reproduce a full refresh"
    );
}

/// The keyed membership-recompute family
/// (`docs/plans/20260808-membership-sensitivity.md` Phase 2): drives
/// `examples/timeseries` with an added `grain: key` model (mirrors
/// `technique_lowering.rs::keyed_membership_recompute_e2e`'s fixture — a
/// `COUNT`-folded fact inner-joined to a `mutation_profile: mutable_snapshot`
/// dimension purely for row admission) through `execute_project` twice: a
/// creation run, then a dimension mutation that makes the `{event_count}`
/// cell's `Trigger::UpstreamMutation` live. Asserts the executed staged-
/// candidate `DELETE`+`INSERT` group is byte-identical to a direct
/// `emit_staged_candidate_conditional` call over the same table/key/
/// candidate-select/compared-columns.
#[tokio::test]
async fn delete_insert_suppressed_keyed_membership_statements_come_from_the_emitter() {
    const MODEL_SQL: &str = "SELECT t.user_id AS user_id, COUNT(t.transaction_id) AS \
         event_count FROM smelt.sources.raw.transactions t \
         JOIN smelt.sources.raw.users u ON t.user_id = u.user_id \
         GROUP BY t.user_id";
    const MODEL_FILE: &str = "---\n\
         materialization: table\n\
         refresh: incremental\n\
         grain: key\n\
         unique_key: user_id\n\
         maintenance:\n  \
           scan_bounds:\n    \
             per_source:\n      \
               raw.users:\n        \
                 allow_full_scan: true\n      \
               raw.transactions:\n        \
                 allow_full_scan: true\n\
         ---\n";

    let source_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/timeseries")
        .canonicalize()
        .expect("examples/timeseries exists");

    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().join("project");
    copy_dir_recursive(&source_dir, &project_dir);
    std::fs::write(
        project_dir.join("models/user_lifetime_status.sql"),
        format!("{MODEL_FILE}{MODEL_SQL}\n"),
    )
    .expect("write keyed model fixture");

    let db_path = tmp.path().join("run.duckdb");
    let config = Arc::new(Config::load(&project_dir).expect("load smelt.yml"));

    {
        let backend = DuckDbBackend::new(&db_path, "main")
            .await
            .expect("open duckdb");
        backend
            .execute_sql(
                "CREATE TABLE main.sources_raw_transactions (transaction_id INTEGER, user_id \
                 INTEGER, amount DECIMAL(10,2), transaction_timestamp TIMESTAMP, \
                 transaction_type VARCHAR)",
            )
            .await
            .expect("create transactions source table");
        backend
            .execute_sql(
                "INSERT INTO main.sources_raw_transactions VALUES \
                 (1, 1, 10.00, TIMESTAMP '2025-01-10 08:00:00', 'purchase'), \
                 (2, 2, 20.00, TIMESTAMP '2025-01-10 09:00:00', 'purchase')",
            )
            .await
            .expect("seed transactions");
        backend
            .execute_sql(
                "CREATE TABLE main.sources_raw_users (user_id INTEGER, user_name VARCHAR, \
                 signup_date DATE)",
            )
            .await
            .expect("create users source table");
        backend
            .execute_sql(
                "INSERT INTO main.sources_raw_users VALUES \
                 (1, 'Alice', DATE '2025-01-01'), (2, 'Bob', DATE '2025-01-02')",
            )
            .await
            .expect("seed users");
    }

    // Run 1: creation — never the membership-recompute path.
    {
        let (db, graph) = build_db_and_graph(&project_dir, &config);
        execute_project(
            "keyed-membership-parity-run-1".to_string(),
            select_request("dev", "user_lifetime_status", "2025-01-10", "2025-01-11"),
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
        .expect("first run (create) must succeed");
    }

    // Mutate the dimension in place, making the `{event_count}` cell live.
    {
        let backend = DuckDbBackend::new(&db_path, "main")
            .await
            .expect("reopen duckdb");
        backend
            .execute_sql("UPDATE main.sources_raw_users SET user_name = 'Alicia' WHERE user_id = 1")
            .await
            .expect("mutate dimension");
    }

    // Run 2: the dimension mutation dispatches the staged-candidate
    // membership recompute.
    let (db, graph) = build_db_and_graph(&project_dir, &config);
    let backend_slot: Arc<Mutex<Option<Arc<RecordingBackend>>>> = Arc::new(Mutex::new(None));
    let factory = RecordingBackendFactory {
        db_path: db_path.clone(),
        backend: Arc::clone(&backend_slot),
    };
    let outcome = execute_project(
        "keyed-membership-parity-run-2".to_string(),
        select_request("dev", "user_lifetime_status", "2025-01-11", "2025-01-12"),
        Arc::clone(&config),
        graph,
        db,
        &project_dir,
        &factory,
        &smelt_runtime::NoOpReporter,
        CancellationToken::new(),
    )
    .await
    .expect("second run (membership recompute) must succeed");

    let record = outcome
        .models
        .get("user_lifetime_status")
        .expect("user_lifetime_status ran");
    assert_eq!(
        record.strategy, "delete_insert_suppressed",
        "the dimension mutation must dispatch the staged-candidate membership-recompute \
         technique"
    );

    let backend = backend_slot
        .lock()
        .unwrap()
        .clone()
        .expect("backend recorded");
    let groups = backend.recorded_groups();
    let staged_groups: Vec<_> = groups
        .iter()
        .filter(|g| {
            g.statements
                .first()
                .is_some_and(|s| s.sql.starts_with("CREATE TEMP TABLE"))
        })
        .collect();
    assert_eq!(
        staged_groups.len(),
        1,
        "exactly one staged-candidate group must have executed: {:?}",
        groups
    );
    let group = staged_groups[0];
    assert!(
        group.transactional,
        "the staged-candidate group is transactional"
    );
    assert_eq!(group.statements.len(), 6);

    // Recover the caller-composed `candidate_select` from the recorded
    // INSERT statement (statement index 1: `INSERT INTO {staged} {select}`)
    // and the staged relation name from statement 0's `CREATE TEMP TABLE
    // {name} AS SELECT * FROM ({select}) AS __smelt_staged_shape LIMIT 0`.
    let insert_sql = &group.statements[1].sql;
    let staged_relation = "__smelt_staged_user_lifetime_status";
    let candidate_prefix = format!("INSERT INTO {staged_relation} ");
    assert!(
        insert_sql.starts_with(&candidate_prefix),
        "unexpected staged-candidate INSERT statement: {insert_sql}"
    );
    let candidate_select = &insert_sql[candidate_prefix.len()..];

    let expected = smelt_logical::maintenance::emit::emit_staged_candidate_conditional_recompute(
        "main.user_lifetime_status",
        staged_relation,
        &["user_id".to_string()],
        candidate_select,
        &["event_count".to_string()],
        MaintenanceDialect::DuckDb,
    );
    assert_eq!(
        &expected, group,
        "executed staged-candidate group must be byte-identical to a direct emitter call over \
         the same inputs (the full-recompute variant — this cell's candidate_select is always \
         the model's own full unwindowed recompute, so a departed key must be genuinely \
         deleted, not merely left untouched)"
    );

    // Result-equivalence: the staged-candidate recompute actually executed
    // must leave the target multiset-equal to a full refresh of the model.
    assert!(
        multiset_equal(
            backend.as_ref(),
            "SELECT user_id, event_count FROM main.user_lifetime_status",
            "SELECT t.user_id, COUNT(t.transaction_id) AS event_count FROM \
             main.sources_raw_transactions t JOIN main.sources_raw_users u ON t.user_id = \
             u.user_id GROUP BY t.user_id"
        )
        .await,
        "the staged-candidate recompute actually executed must reproduce a full refresh"
    );
}
