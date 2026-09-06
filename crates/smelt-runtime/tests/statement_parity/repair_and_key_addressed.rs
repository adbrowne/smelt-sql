use super::*;

/// The keyless (whole-row) realisation (`docs/outcomes/
/// 20260815-definition-delta-migrate/phases/27c-plan.md`): a `grain:
/// partition` output with no `unique_key` and no `GROUP BY` — `RowIdentity::
/// WholeRow` — joined to a `mutation_profile: mutable_snapshot` dimension
/// with no declared `unique_key`/`referential_integrity` of its own (so the
/// join is never closure-pruned, keeping the group genuinely membership-
/// sensitive) must dispatch `MembershipRecomputeWrite::StagedKeyless`, whose
/// executed statements are byte-identical to a direct
/// `emit_staged_candidate_conditional_keyless` call over the batch's own
/// inputs.
#[tokio::test]
async fn staged_candidate_keyless_statements_come_from_the_emitter() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().join("project");
    std::fs::create_dir_all(project_dir.join("models/sources")).expect("mkdir models/sources");
    std::fs::write(
        project_dir.join("smelt.yml"),
        "name: keyless_membership_parity\nversion: 1\npaths:\n  - models\n\
         targets:\n  dev:\n    type: duckdb\n    schema: main\n\
         default_materialization: table\ntarget: dev\n",
    )
    .expect("write smelt.yml");
    std::fs::write(
        project_dir.join("models/sources/facts.yml"),
        "description: facts\ncolumns:\n- name: fact_id\n  type: INTEGER\n\
         - name: dim_id\n  type: INTEGER\n- name: event_date\n  type: DATE\n\
         - name: amount\n  type: INTEGER\n\
         mutation_profile:\n  kind: append_only\n\
         timeseries:\n  partition_column: event_date\n  event_time_column: event_date\n  \
         granularity: day\n",
    )
    .expect("write facts source yml");
    std::fs::write(
        project_dir.join("models/sources/dim.yml"),
        "description: dim\ncolumns:\n- name: dim_id\n  type: INTEGER\n\
         - name: tag\n  type: VARCHAR\n\
         mutation_profile:\n  kind: mutable_snapshot\n",
    )
    .expect("write dim source yml");
    write_model(
        &project_dir,
        "events_by_dim",
        "---\nmaterialization: table\nrefresh: incremental\ngrain: partition\n\
         timeseries:\n  partition_column: event_date\n  event_time_column: event_date\n  \
         granularity: day\nmaintenance:\n  scan_bounds:\n    per_source:\n      \
         dim:\n        allow_full_scan: true\n---\n\
         SELECT f.fact_id AS fact_id, f.event_date AS event_date, f.amount AS amount, d.tag AS \
         tag\nFROM smelt.sources.facts f\nJOIN smelt.sources.dim d ON f.dim_id = d.dim_id\n",
    );

    let db_path = tmp.path().join("run.duckdb");
    let config = Arc::new(Config::load(&project_dir).expect("load smelt.yml"));

    {
        let backend = DuckDbBackend::new(&db_path, "main")
            .await
            .expect("open duckdb");
        backend
            .execute_sql(
                "CREATE TABLE main.sources_facts (fact_id INTEGER, dim_id INTEGER, event_date \
                 DATE, amount INTEGER)",
            )
            .await
            .expect("create facts source table");
        backend
            .execute_sql(
                "INSERT INTO main.sources_facts VALUES \
                 (1, 1, DATE '2025-01-10', 10), (2, 2, DATE '2025-01-10', 20)",
            )
            .await
            .expect("seed facts");
        backend
            .execute_sql("CREATE TABLE main.sources_dim (dim_id INTEGER, tag VARCHAR)")
            .await
            .expect("create dim source table");
        backend
            .execute_sql("INSERT INTO main.sources_dim VALUES (1, 'a'), (2, 'b')")
            .await
            .expect("seed dim");
    }

    // Run 1: creation — never the membership-recompute path.
    {
        let (db, graph) = build_db_and_graph(&project_dir, &config);
        execute_project(
            "keyless-membership-parity-run-1".to_string(),
            super::column_scoped_merge::select_request(
                "dev",
                "events_by_dim",
                "2025-01-10",
                "2025-01-11",
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
        .expect("first run (create) must succeed");
    }

    // Mutate the dimension in place — the `{tag}` cell becomes live.
    {
        let backend = DuckDbBackend::new(&db_path, "main")
            .await
            .expect("reopen duckdb");
        backend
            .execute_sql("UPDATE main.sources_dim SET tag = 'z' WHERE dim_id = 1")
            .await
            .expect("mutate dimension");
    }

    // Run 2: the dimension mutation dispatches the staged-candidate keyless
    // membership recompute.
    let (db, graph) = build_db_and_graph(&project_dir, &config);
    let backend_slot: Arc<Mutex<Option<Arc<RecordingBackend>>>> = Arc::new(Mutex::new(None));
    let factory = RecordingBackendFactory {
        db_path: db_path.clone(),
        backend: Arc::clone(&backend_slot),
    };
    let outcome = execute_project(
        "keyless-membership-parity-run-2".to_string(),
        super::column_scoped_merge::select_request(
            "dev",
            "events_by_dim",
            "2025-01-11",
            "2025-01-12",
        ),
        Arc::clone(&config),
        graph,
        db,
        &project_dir,
        &factory,
        &smelt_runtime::NoOpReporter,
        CancellationToken::new(),
    )
    .await
    .expect("second run (keyless membership recompute) must succeed");

    let record = outcome
        .models
        .get("events_by_dim")
        .expect("events_by_dim ran");
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
        "the staged-candidate keyless group is transactional"
    );
    assert_eq!(group.statements.len(), 7);

    // Recover the caller-composed `candidate_select` from the recorded
    // INSERT statement (statement index 1).
    let staged_relation = "__smelt_staged_events_by_dim";
    let sentinel_relation = "__smelt_sentinel_events_by_dim";
    let insert_sql = &group.statements[1].sql;
    let candidate_prefix = format!("INSERT INTO {staged_relation} ");
    assert!(
        insert_sql.starts_with(&candidate_prefix),
        "unexpected staged-candidate INSERT statement: {insert_sql}"
    );
    let candidate_select = &insert_sql[candidate_prefix.len()..];

    let expected = smelt_logical::maintenance::emit::emit_staged_candidate_conditional_keyless(
        "main.events_by_dim",
        staged_relation,
        sentinel_relation,
        None,
        candidate_select,
        MaintenanceDialect::DuckDb,
    );
    assert_eq!(
        &expected, group,
        "executed staged-candidate keyless group must be byte-identical to a direct emitter \
         call over the same inputs"
    );

    // Result-equivalence: the staged-candidate recompute actually executed
    // must leave the target multiset-equal to a full refresh of the model.
    assert!(
        multiset_equal(
            backend.as_ref(),
            "SELECT fact_id, event_date, amount, tag FROM main.events_by_dim",
            "SELECT f.fact_id, f.event_date, f.amount, d.tag FROM main.sources_facts f JOIN \
             main.sources_dim d ON f.dim_id = d.dim_id"
        )
        .await,
        "the staged-candidate keyless recompute actually executed must reproduce a full refresh"
    );
}

/// The repair family (`docs/specs/incremental_models.md` §"The repair
/// family"): a keyed `MAX` fold over a **clocked, mutable** source refuses
/// the faithful-fold source-posture obligation, so the derived plan admits
/// `Technique::PerGroupRecompute` on the model's own `NewData` trigger
/// instead. The statements a real `execute_project` run sends to the
/// connection must be byte-identical to a direct `emit_per_group_recompute`
/// call over the batch's own inputs — plus the family's result-equivalence
/// leg against a full-refresh oracle.
#[tokio::test]
async fn per_group_recompute_statements_come_from_the_emitter() {
    const ORDERS_SOURCE_YML: &str = r#"description: Mutable order snapshot
columns:
- name: order_id
  type: INTEGER
- name: customer_id
  type: INTEGER
- name: amount
  type: DECIMAL(10,2)
- name: order_date
  type: TIMESTAMP
timeseries:
  event_time_column: order_date
  partition_column: order_date
  granularity: day
unique_key: [order_id]
mutation_profile:
  kind: mutable_snapshot
"#;
    const MODEL_SQL: &str = "SELECT customer_id, MAX(amount) AS max_amount \
         FROM smelt.sources.raw.orders \
         WHERE order_date BETWEEN TIMESTAMP '2025-01-14' - INTERVAL '3 days' AND TIMESTAMP \
         '2025-01-14' \
         GROUP BY customer_id";
    const MODEL_FILE: &str = "---\n\
         materialization: table\n\
         refresh: incremental\n\
         grain: key\n\
         unique_key: customer_id\n\
         ---\n";

    let source_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/timeseries")
        .canonicalize()
        .expect("examples/timeseries exists");

    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().join("project");
    super::column_scoped_merge::copy_dir_recursive(&source_dir, &project_dir);
    std::fs::write(
        project_dir.join("models/sources/raw/orders.yml"),
        ORDERS_SOURCE_YML,
    )
    .expect("write orders source yml");
    std::fs::write(
        project_dir.join("models/customer_max_amount.sql"),
        format!("{MODEL_FILE}{MODEL_SQL}\n"),
    )
    .expect("write repair model fixture");

    let db_path = tmp.path().join("run.duckdb");
    let config = Arc::new(Config::load(&project_dir).expect("load smelt.yml"));

    {
        let backend = DuckDbBackend::new(&db_path, "main")
            .await
            .expect("open duckdb");
        backend
            .execute_sql(
                "CREATE TABLE main.sources_raw_orders (order_id INTEGER, customer_id INTEGER, \
                 amount DECIMAL(10,2), order_date TIMESTAMP)",
            )
            .await
            .expect("create orders source table");
        backend
            .execute_sql(
                "INSERT INTO main.sources_raw_orders VALUES \
                 (1, 1, 100.00, TIMESTAMP '2025-01-13 10:00:00'), \
                 (2, 1, 50.00, TIMESTAMP '2025-01-13 11:00:00'), \
                 (3, 2, 70.00, TIMESTAMP '2025-01-11 10:00:00')",
            )
            .await
            .expect("seed orders");
    }

    // Run 1: creation — nothing to repair yet, the fold's create path runs.
    {
        let (db, graph) = build_db_and_graph(&project_dir, &config);
        execute_project(
            "repair-parity-run-1".to_string(),
            super::column_scoped_merge::select_request(
                "dev",
                "customer_max_amount",
                "2025-01-11",
                "2025-01-14",
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
        .expect("first run (create) must succeed");
    }

    // The retraction `MAX` cannot undo: customer 1's top contribution is
    // corrected downward in place.
    {
        let backend = DuckDbBackend::new(&db_path, "main")
            .await
            .expect("reopen duckdb");
        backend
            .execute_sql("UPDATE main.sources_raw_orders SET amount = 10.00 WHERE order_id = 1")
            .await
            .expect("retract");
    }

    let (db, graph) = build_db_and_graph(&project_dir, &config);
    let backend_slot: Arc<Mutex<Option<Arc<RecordingBackend>>>> = Arc::new(Mutex::new(None));
    let factory = RecordingBackendFactory {
        db_path: db_path.clone(),
        backend: Arc::clone(&backend_slot),
    };
    let outcome = execute_project(
        "repair-parity-run-2".to_string(),
        super::column_scoped_merge::select_request(
            "dev",
            "customer_max_amount",
            "2025-01-16",
            "2025-01-17",
        ),
        Arc::clone(&config),
        graph,
        db,
        &project_dir,
        &factory,
        &smelt_runtime::NoOpReporter,
        CancellationToken::new(),
    )
    .await
    .expect("second run (per-group recompute) must succeed");

    let record = outcome
        .models
        .get("customer_max_amount")
        .expect("customer_max_amount ran");
    assert_eq!(
        record.strategy, "per_group_recompute",
        "the retraction must dispatch the repair family, not the fold"
    );

    let backend = backend_slot
        .lock()
        .unwrap()
        .clone()
        .expect("backend recorded");
    let groups = backend.recorded_groups();
    let repair_groups: Vec<_> = groups
        .iter()
        .filter(|g| {
            g.statements
                .first()
                .is_some_and(|s| s.sql.starts_with("CREATE TEMP TABLE __smelt_repair_"))
        })
        .collect();
    assert_eq!(
        repair_groups.len(),
        1,
        "exactly one per-group-recompute group must have executed: {groups:?}"
    );
    let group = repair_groups[0];
    assert!(group.transactional, "the repair group is transactional");
    assert_eq!(group.statements.len(), 5);

    // Recover the caller-composed `candidate_select` from the recorded
    // `INSERT INTO {staged} {select}` (statement index 1).
    let staged_relation = "__smelt_repair_customer_max_amount";
    let candidate_prefix = format!("INSERT INTO {staged_relation} ");
    let insert_sql = &group.statements[1].sql;
    assert!(
        insert_sql.starts_with(&candidate_prefix),
        "unexpected staged INSERT statement: {insert_sql}"
    );
    let candidate_select = &insert_sql[candidate_prefix.len()..];

    // This is a `MutationProfile::MutableSnapshot` source, so the affected-
    // key relation is the group-grain sidecar diff (P9), not the append-only
    // clamped scan — a backend-state-dependent `VALUES (...)` literal
    // recovered straight from the executed candidate, per
    // `extract_affected_keys_select`'s own doc comment.
    let key = vec!["customer_id".to_string()];
    let affected_keys_select = extract_affected_keys_select(candidate_select);
    assert!(
        affected_keys_select.contains("__smelt_repair_group_keys(delta_key)"),
        "a MutableSnapshot source's affected-key relation must be the sidecar-diff-derived \
         literal keys relation: {affected_keys_select}"
    );

    let expected = emit_per_group_recompute(
        "main.customer_max_amount",
        staged_relation,
        &key,
        &affected_keys_select,
        candidate_select,
        MaintenanceDialect::DuckDb,
    );
    assert_eq!(
        &expected, group,
        "executed per-group-recompute group must be byte-identical to a direct emitter call \
         over the same inputs"
    );

    // Result-equivalence: the repair actually executed must leave the target
    // multiset-equal to a full refresh over the same inputs.
    assert!(
        multiset_equal(
            backend.as_ref(),
            "SELECT customer_id, max_amount FROM main.customer_max_amount",
            "SELECT customer_id, MAX(amount) AS max_amount FROM main.sources_raw_orders WHERE \
             order_date BETWEEN TIMESTAMP '2025-01-14' - INTERVAL '3 days' AND TIMESTAMP \
             '2025-01-14' GROUP BY customer_id"
        )
        .await,
        "the repair actually executed must reproduce a full refresh"
    );
}

/// Phase 7 (`docs/outcomes/20260809-output-delta-typing/phases/07-plan.md`):
/// a key-addressed model-edge cell's `Technique::PerGroupRecompute` group
/// must be byte-identical to a direct [`emit_per_group_recompute`] call —
/// the SAME parity proof `per_group_recompute_statements_come_from_the_
/// emitter` runs for the ordinary declared-source repair route, over a
/// clockless `KeyedUpsert` upstream model edge instead of a
/// `mutation_profile: mutable_snapshot` source.
#[tokio::test]
async fn key_addressed_model_edge_statements_come_from_the_emitter() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().join("project");
    std::fs::create_dir_all(project_dir.join("models/sources")).expect("mkdir models/sources");
    std::fs::write(
        project_dir.join("smelt.yml"),
        "name: key_addressed_parity\nversion: 1\npaths:\n  - models\n\
         targets:\n  dev:\n    type: duckdb\n    schema: main\n\
         default_materialization: view\n",
    )
    .expect("write smelt.yml");
    std::fs::write(
        project_dir.join("models/sources/payments.yml"),
        "description: payments\ncolumns:\n- name: user_id\n  type: INTEGER\n\
         - name: amount\n  type: DECIMAL(10,2)\n- name: d\n  type: DATE\n\
         mutation_profile:\n  kind: append_only\n\
         timeseries:\n  partition_column: d\n  event_time_column: d\n  granularity: day\n",
    )
    .expect("write payments source yml");
    write_model(
        &project_dir,
        "agg",
        "---\nmaterialization: table\nrefresh: incremental\ngrain: key\n\
         unique_key: user_id\nmaintenance:\n  scan_bounds:\n    per_source:\n      \
         payments:\n        allow_full_scan: true\n---\n\
         SELECT user_id, SUM(amount) AS total\nFROM smelt.sources.payments\n\
         GROUP BY user_id\n",
    );
    write_model(
        &project_dir,
        "downstream",
        "---\nmaterialization: table\nrefresh: incremental\ngrain: key\n\
         unique_key: user_id\n---\n\
         SELECT user_id, ANY_VALUE(total) AS total FROM smelt.agg GROUP BY user_id\n",
    );

    let db_path = tmp.path().join("run.duckdb");
    let config = Arc::new(Config::load(&project_dir).expect("load smelt.yml"));

    {
        let backend = DuckDbBackend::new(&db_path, "main")
            .await
            .expect("open duckdb");
        backend
            .execute_sql(
                "CREATE TABLE main.sources_payments (user_id INTEGER, amount DECIMAL(10,2), \
                 d DATE)",
            )
            .await
            .expect("create payments source table");
        backend
            .execute_sql(
                "INSERT INTO main.sources_payments VALUES \
                 (1, 100.00, DATE '2025-01-01'), (1, 50.00, DATE '2025-01-02'), \
                 (2, 70.00, DATE '2025-01-01')",
            )
            .await
            .expect("seed payments");
    }

    // `agg` is a clocked, `grain: key` window-forward model always run
    // unwindowed here — that now refuses without `--full-refresh`
    // (`docs/specs/incremental_shapes.md` §"The key grain"). Harmless for
    // `downstream`: `full_refresh` is only consulted by that one
    // windowless-keyed-run branch, never by the key-addressed model-edge
    // dispatch this test pins.
    let multi_select = |models: &[&str]| ExecuteRequest {
        target: "dev".to_string(),
        select: models.iter().map(|s| s.to_string()).collect(),
        exclude: vec![],
        start: None,
        end: None,
        batch_size_days: None,
        per_partition: false,
        full_refresh: true,
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
    };

    // Run 1: creation — nothing to fold yet.
    {
        let (db, graph) = build_db_and_graph(&project_dir, &config);
        execute_project(
            "key-edge-parity-run-1".to_string(),
            multi_select(&["agg", "downstream"]),
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

    // Mutate user 1's contribution in place.
    {
        let backend = DuckDbBackend::new(&db_path, "main")
            .await
            .expect("reopen duckdb");
        backend
            .execute_sql(
                "UPDATE main.sources_payments SET amount = 200.00 WHERE user_id = 1 AND \
                 amount = 100.00",
            )
            .await
            .expect("mutate payments");
    }

    let (db, graph) = build_db_and_graph(&project_dir, &config);
    let backend_slot: Arc<Mutex<Option<Arc<RecordingBackend>>>> = Arc::new(Mutex::new(None));
    let factory = RecordingBackendFactory {
        db_path: db_path.clone(),
        backend: Arc::clone(&backend_slot),
    };
    let outcome = execute_project(
        "key-edge-parity-run-2".to_string(),
        multi_select(&["agg", "downstream"]),
        Arc::clone(&config),
        graph,
        db,
        &project_dir,
        &factory,
        &smelt_runtime::NoOpReporter,
        CancellationToken::new(),
    )
    .await
    .expect("second run (key-addressed recompute) must succeed");

    let record = outcome.models.get("downstream").expect("downstream ran");
    assert_eq!(
        record.strategy, "per_group_recompute",
        "the upstream's key-addressed fold must dispatch the repair family"
    );

    let backend = backend_slot
        .lock()
        .unwrap()
        .clone()
        .expect("backend recorded");
    let groups = backend.recorded_groups();
    let repair_groups: Vec<_> = groups
        .iter()
        .filter(|g| {
            g.statements
                .first()
                .is_some_and(|s| s.sql.starts_with("CREATE TEMP TABLE __smelt_repair_"))
        })
        .collect();
    assert_eq!(
        repair_groups.len(),
        1,
        "exactly one key-addressed per-group-recompute group must have executed: {groups:?}"
    );
    let group = repair_groups[0];
    assert!(group.transactional, "the repair group is transactional");
    assert_eq!(group.statements.len(), 5);

    let staged_relation = "__smelt_repair_downstream";
    let candidate_prefix = format!("INSERT INTO {staged_relation} ");
    let insert_sql = &group.statements[1].sql;
    assert!(
        insert_sql.starts_with(&candidate_prefix),
        "unexpected staged INSERT statement: {insert_sql}"
    );
    let candidate_select = &insert_sql[candidate_prefix.len()..];

    let key = vec!["user_id".to_string()];
    let affected_keys_select = extract_affected_keys_select(candidate_select);
    assert!(
        affected_keys_select.contains("SELECT DISTINCT"),
        "a key-addressed cell's affected-key relation must be the key-restricted projection \
         over the upstream table: {affected_keys_select}"
    );
    assert!(
        !affected_keys_select
            .to_uppercase()
            .contains("__SMELT_REPAIR_GROUP_KEYS"),
        "a key-addressed cell must not route through the ordinary sidecar-literal-keys \
         relation shape — it reads the upstream table directly: {affected_keys_select}"
    );

    let expected = emit_per_group_recompute(
        "main.downstream",
        staged_relation,
        &key,
        &affected_keys_select,
        candidate_select,
        MaintenanceDialect::DuckDb,
    );
    assert_eq!(
        &expected, group,
        "executed key-addressed per-group-recompute group must be byte-identical to a direct \
         emitter call over the same inputs"
    );

    // Result-equivalence: the key-addressed fold actually executed must
    // leave the downstream equal to a full refresh over agg's current state.
    assert!(
        multiset_equal(
            backend.as_ref(),
            "SELECT user_id, total FROM main.downstream",
            "SELECT user_id, SUM(amount) AS total FROM main.sources_payments GROUP BY user_id"
        )
        .await,
        "the key-addressed fold actually executed must reproduce a full refresh"
    );
}

#[tokio::test]
async fn diff_patch_statements_come_from_the_emitter() {
    const ORDERS_SOURCE_YML: &str = r#"description: Mutable order snapshot
columns:
- name: order_id
  type: INTEGER
- name: customer_id
  type: INTEGER
- name: amount
  type: DECIMAL(10,2)
- name: order_date
  type: TIMESTAMP
timeseries:
  event_time_column: order_date
  partition_column: order_date
  granularity: day
unique_key: [order_id]
mutation_profile:
  kind: mutable_snapshot
"#;
    const MODEL_SQL: &str = "SELECT customer_id, MAX(amount) AS max_amount \
         FROM smelt.sources.raw.orders \
         WHERE order_date BETWEEN TIMESTAMP '2025-01-14' - INTERVAL '3 days' AND TIMESTAMP \
         '2025-01-14' \
         GROUP BY customer_id";
    const MODEL_FILE: &str = "---\n\
         materialization: table\n\
         refresh: incremental\n\
         grain: key\n\
         unique_key: customer_id\n\
         maintenance:\n\
         \x20\x20cells:\n\
         \x20\x20- on: raw.orders\n\
         \x20\x20\x20\x20columns: [max_amount]\n\
         \x20\x20\x20\x20write: diff_patch\n\
         ---\n";

    let source_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/timeseries")
        .canonicalize()
        .expect("examples/timeseries exists");

    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().join("project");
    super::column_scoped_merge::copy_dir_recursive(&source_dir, &project_dir);
    std::fs::write(
        project_dir.join("models/sources/raw/orders.yml"),
        ORDERS_SOURCE_YML,
    )
    .expect("write orders source yml");
    std::fs::write(
        project_dir.join("models/customer_max_amount.sql"),
        format!("{MODEL_FILE}{MODEL_SQL}\n"),
    )
    .expect("write diff_patch model fixture");

    let db_path = tmp.path().join("run.duckdb");
    let config = Arc::new(Config::load(&project_dir).expect("load smelt.yml"));

    {
        let backend = DuckDbBackend::new(&db_path, "main")
            .await
            .expect("open duckdb");
        backend
            .execute_sql(
                "CREATE TABLE main.sources_raw_orders (order_id INTEGER, customer_id INTEGER, \
                 amount DECIMAL(10,2), order_date TIMESTAMP)",
            )
            .await
            .expect("create orders source table");
        backend
            .execute_sql(
                "INSERT INTO main.sources_raw_orders VALUES \
                 (1, 1, 100.00, TIMESTAMP '2025-01-13 10:00:00'), \
                 (2, 1, 50.00, TIMESTAMP '2025-01-13 11:00:00'), \
                 (3, 2, 70.00, TIMESTAMP '2025-01-11 10:00:00')",
            )
            .await
            .expect("seed orders");
    }

    // Run 1: creation — nothing to repair yet, the fold's create path runs.
    {
        let (db, graph) = build_db_and_graph(&project_dir, &config);
        execute_project(
            "diff-patch-parity-run-1".to_string(),
            super::column_scoped_merge::select_request(
                "dev",
                "customer_max_amount",
                "2025-01-11",
                "2025-01-14",
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
        .expect("first run (create) must succeed");
    }

    // The retraction `MAX` cannot undo: customer 1's top contribution is
    // corrected downward in place.
    {
        let backend = DuckDbBackend::new(&db_path, "main")
            .await
            .expect("reopen duckdb");
        backend
            .execute_sql("UPDATE main.sources_raw_orders SET amount = 10.00 WHERE order_id = 1")
            .await
            .expect("retract");
    }

    let (db, graph) = build_db_and_graph(&project_dir, &config);
    let backend_slot: Arc<Mutex<Option<Arc<RecordingBackend>>>> = Arc::new(Mutex::new(None));
    let factory = RecordingBackendFactory {
        db_path: db_path.clone(),
        backend: Arc::clone(&backend_slot),
    };
    let outcome = execute_project(
        "diff-patch-parity-run-2".to_string(),
        super::column_scoped_merge::select_request(
            "dev",
            "customer_max_amount",
            "2025-01-16",
            "2025-01-17",
        ),
        Arc::clone(&config),
        graph,
        db,
        &project_dir,
        &factory,
        &smelt_runtime::NoOpReporter,
        CancellationToken::new(),
    )
    .await
    .expect("second run (diff_patch) must succeed");

    let record = outcome
        .models
        .get("customer_max_amount")
        .expect("customer_max_amount ran");
    assert_eq!(
        record.strategy, "diff_patch",
        "the write: diff_patch pin must dispatch the diff-patch write, not the repair family's \
         own targeted delete+insert"
    );

    let backend = backend_slot
        .lock()
        .unwrap()
        .clone()
        .expect("backend recorded");
    let groups = backend.recorded_groups();
    let diff_patch_groups: Vec<_> = groups
        .iter()
        .filter(|g| {
            g.statements
                .first()
                .is_some_and(|s| s.sql.starts_with("CREATE TEMP TABLE __smelt_diff_patch_"))
        })
        .collect();
    assert_eq!(
        diff_patch_groups.len(),
        1,
        "exactly one diff_patch group must have executed: {groups:?}"
    );
    let group = diff_patch_groups[0];
    assert!(group.transactional, "the diff_patch group is transactional");
    // Update leg + delete leg (PerGroupRecompute's own bounded-slice
    // admission discharges diff_patch's completeness premise, so the delete
    // leg is included) + create/insert-candidates/insert/drop = 6.
    assert_eq!(group.statements.len(), 6);
    assert!(
        group
            .statements
            .iter()
            .any(|s| s.sql.starts_with("DELETE") && s.sql.contains("NOT EXISTS")),
        "the delete leg must be present: {group:?}"
    );

    // Recover the caller-composed `candidate_select` from the recorded
    // `INSERT INTO {staged} {select}` (statement index 1).
    let staged_relation = "__smelt_diff_patch_customer_max_amount";
    let candidate_prefix = format!("INSERT INTO {staged_relation} ");
    let insert_sql = &group.statements[1].sql;
    assert!(
        insert_sql.starts_with(&candidate_prefix),
        "unexpected staged INSERT statement: {insert_sql}"
    );
    let candidate_select = &insert_sql[candidate_prefix.len()..];

    // This is a `MutationProfile::MutableSnapshot` source, so the affected-
    // key relation is the group-grain sidecar diff (P9), not the append-only
    // clamped scan — a backend-state-dependent `VALUES (...)` literal
    // recovered straight from the executed candidate, per
    // `extract_affected_keys_select`'s own doc comment.
    let key = vec!["customer_id".to_string()];
    let affected_keys_select = extract_affected_keys_select(candidate_select);
    assert!(
        affected_keys_select.contains("__smelt_repair_group_keys(delta_key)"),
        "a MutableSnapshot source's affected-key relation must be the sidecar-diff-derived \
         literal keys relation: {affected_keys_select}"
    );

    let slice_predicate = smelt_runtime::maintenance_driver::repair_slice_predicate(
        "customer_max_amount",
        &key,
        &affected_keys_select,
    );
    let expected = emit_diff_patch(
        "main.customer_max_amount",
        staged_relation,
        &key,
        candidate_select,
        &["max_amount".to_string()],
        &slice_predicate,
        &smelt_logical::maintenance::diff_patch::DeleteLeg::Complete,
        MaintenanceDialect::DuckDb,
    );
    assert_eq!(
        &expected, group,
        "executed diff_patch group must be byte-identical to a direct emitter call over the \
         same inputs"
    );

    // Result-equivalence: the diff_patch write actually executed must leave
    // the target multiset-equal to a full refresh over the same inputs.
    assert!(
        multiset_equal(
            backend.as_ref(),
            "SELECT customer_id, max_amount FROM main.customer_max_amount",
            "SELECT customer_id, MAX(amount) AS max_amount FROM main.sources_raw_orders WHERE \
             order_date BETWEEN TIMESTAMP '2025-01-14' - INTERVAL '3 days' AND TIMESTAMP \
             '2025-01-14' GROUP BY customer_id"
        )
        .await,
        "the diff_patch write actually executed must reproduce a full refresh"
    );
}
