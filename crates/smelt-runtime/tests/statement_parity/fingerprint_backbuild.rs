use super::*;

/// Phase F3 (`docs/plans/20260715-composed-axes-conditional-maintenance.md`)
/// — the fingerprint-sidecar diff query is emitter-authored (unlike the T5
/// observed-delta recording query above, which D1 ruled smelt-state
/// bookkeeping): `smelt_runtime::maintenance_driver::
/// diff_fingerprint_sidecar_changed_keys`/`refresh_fingerprint_sidecar` must
/// execute SQL text byte-identical to a direct call of
/// `smelt_logical::maintenance::emit::emit_fingerprint_sidecar_diff`/
/// `emit_fingerprint_digest_select` and
/// `smelt_state::ddl_duckdb::generate_fingerprint_sidecar_refresh_sql`/
/// `_gc_sql` over the same resolved inputs — this is a direct-dispatch leg
/// (no `execute_project` model pipeline involved, matching the precedent
/// `suppressed_keyed_fold_statements_come_from_the_emitter` documents: the
/// sidecar is not yet wired into the live trigger/technique-selection
/// pipeline, that wiring is a later phase's scope).
#[tokio::test]
async fn fingerprint_sidecar_diff_and_refresh_statements_come_from_the_emitter() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("test.duckdb");
    let inner = DuckDbBackend::new(&db_path, "main")
        .await
        .expect("open duckdb");
    let backend = RecordingBackend::new(inner);

    backend
        .execute_sql(
            "CREATE TABLE main.dim_users (id INTEGER, name VARCHAR, tier VARCHAR, notes VARCHAR)",
        )
        .await
        .expect("create source table");
    backend
        .execute_sql(
            "INSERT INTO main.dim_users VALUES \
             (1, 'Alice', 'gold', 'n1'), (2, 'Bob', 'silver', 'n2'), (3, 'Cara', 'gold', 'n3')",
        )
        .await
        .expect("seed source table");

    let projection = smelt_logical::analysis::fingerprint::Projection::Columns(
        ["name".to_string(), "tier".to_string()]
            .into_iter()
            .collect(),
    );
    let source_key = vec!["id".to_string()];
    let all_source_columns = vec![
        "id".to_string(),
        "name".to_string(),
        "tier".to_string(),
        "notes".to_string(),
    ];
    // Phase F4 — the consuming model's SQL text, folded into the sidecar's
    // identity stamp (`compute_fingerprint_sidecar_stamp`).
    let model_sql = "SELECT id, name, tier FROM smelt.sources.dim_users";
    // Phase 4 (`docs/outcomes/20260904-decided-gap-residue`) — the sidecar
    // namespace also includes the CONSUMING model's own address.
    let consumer_address = "smelt.models.consumer_a";

    // Run 1: absent sidecar — every source row is "changed" (whole-table
    // delta), and this diff also creates the sidecar table.
    let changed = smelt_runtime::maintenance_driver::diff_fingerprint_sidecar_changed_keys(
        &backend,
        "main",
        "smelt.sources.dim_users",
        "main.dim_users",
        &source_key,
        &projection,
        &all_source_columns,
        model_sql,
        consumer_address,
    )
    .await
    .expect("first diff against an absent sidecar");
    let mut changed_sorted = changed.clone();
    changed_sorted.sort();
    assert_eq!(
        changed_sorted,
        vec!["1".to_string(), "2".to_string(), "3".to_string()],
        "an absent sidecar must report every current source row as changed"
    );

    // The executed diff SQL must be byte-identical to a direct emitter call
    // over the same resolved inputs.
    let identity = smelt_logical::analysis::fingerprint::projection_identity(&projection);
    let stamp =
        smelt_runtime::maintenance_driver::compute_fingerprint_sidecar_stamp(&identity, model_sql);
    let expected_diff_sql = smelt_logical::maintenance::emit::emit_fingerprint_sidecar_diff(
        "main.dim_users",
        &source_key,
        &["name".to_string(), "tier".to_string()],
        "main._smelt_fingerprint_sidecar",
        "smelt.sources.dim_users",
        &identity,
        consumer_address,
        &stamp,
        MaintenanceDialect::DuckDb,
    );
    let recorded_sql = backend.recorded_sql();
    assert!(
        recorded_sql.contains(&expected_diff_sql),
        "executed diff SQL must be byte-identical to a direct emitter call: {recorded_sql:?}"
    );

    // Refresh: populate the sidecar (a trivial, empty write_group — this
    // leg tests statement byte-identity, not the write/refresh
    // transactionality already covered by
    // `smelt-backend-duckdb`'s own unit tests).
    let empty_write_group = StatementGroup {
        statements: vec![],
        transactional: false,
    };
    smelt_runtime::maintenance_driver::refresh_fingerprint_sidecar(
        &backend,
        "main",
        "smelt.sources.dim_users",
        "main.dim_users",
        &source_key,
        &projection,
        &all_source_columns,
        model_sql,
        consumer_address,
        &empty_write_group,
    )
    .await
    .expect("sidecar refresh");

    let expected_digest_select = smelt_logical::maintenance::emit::emit_fingerprint_digest_select(
        "main.dim_users",
        &source_key,
        &["name".to_string(), "tier".to_string()],
        MaintenanceDialect::DuckDb,
    );
    let expected_refresh_sql = smelt_state::ddl_duckdb::generate_fingerprint_sidecar_refresh_sql(
        "main",
        "smelt.sources.dim_users",
        &identity,
        consumer_address,
        &stamp,
        &expected_digest_select,
    );
    let expected_gc_sql = smelt_state::ddl_duckdb::generate_fingerprint_sidecar_gc_sql(
        "main",
        "smelt.sources.dim_users",
        &identity,
        consumer_address,
        &expected_digest_select,
    );
    let recorded_sql = backend.recorded_sql();
    assert!(
        recorded_sql.contains(&expected_refresh_sql),
        "executed refresh SQL must be byte-identical to a direct emitter/ddl call: {recorded_sql:?}"
    );
    assert!(
        recorded_sql.contains(&expected_gc_sql),
        "executed GC SQL must be byte-identical to a direct emitter/ddl call: {recorded_sql:?}"
    );

    // Run 2: mutate exactly 2 of the 3 rows' projected columns — the diff
    // must report exactly those 2 keys, never the untouched third.
    backend
        .execute_sql("UPDATE main.dim_users SET tier = 'platinum' WHERE id = 1")
        .await
        .expect("mutate row 1");
    backend
        .execute_sql("UPDATE main.dim_users SET name = 'Roberta' WHERE id = 2")
        .await
        .expect("mutate row 2");

    let changed_after_edit =
        smelt_runtime::maintenance_driver::diff_fingerprint_sidecar_changed_keys(
            &backend,
            "main",
            "smelt.sources.dim_users",
            "main.dim_users",
            &source_key,
            &projection,
            &all_source_columns,
            model_sql,
            consumer_address,
        )
        .await
        .expect("second diff after a targeted edit");
    let mut changed_after_edit_sorted = changed_after_edit;
    changed_after_edit_sorted.sort();
    assert_eq!(
        changed_after_edit_sorted,
        vec!["1".to_string(), "2".to_string()],
        "the diff must report exactly the 2 edited keys, never the untouched third"
    );

    // An edit to a column OUTSIDE the P4 projection (`notes`) must yield an
    // EMPTY changed set once the sidecar reflects that edit's siblings.
    smelt_runtime::maintenance_driver::refresh_fingerprint_sidecar(
        &backend,
        "main",
        "smelt.sources.dim_users",
        "main.dim_users",
        &source_key,
        &projection,
        &all_source_columns,
        model_sql,
        consumer_address,
        &empty_write_group,
    )
    .await
    .expect("second sidecar refresh");
    backend
        .execute_sql("UPDATE main.dim_users SET notes = 'edited' WHERE id = 3")
        .await
        .expect("mutate row 3's out-of-projection column");
    let changed_out_of_projection =
        smelt_runtime::maintenance_driver::diff_fingerprint_sidecar_changed_keys(
            &backend,
            "main",
            "smelt.sources.dim_users",
            "main.dim_users",
            &source_key,
            &projection,
            &all_source_columns,
            model_sql,
            consumer_address,
        )
        .await
        .expect("third diff after an out-of-projection edit");
    assert!(
        changed_out_of_projection.is_empty(),
        "an edit outside the P4 projection must never dirty the changed-key set: \
         {changed_out_of_projection:?}"
    );
}

// =============================================================================
// Backbuild statement-parity legs (`crates/smelt-logical/src/backbuild/
// emit.rs`, `docs/outcomes/20260815-definition-delta-migrate/phases/
// 30-plan.md`): the same "executed byte-identical to a direct emitter call"
// proof as the maintenance families above, driven directly through
// `smelt_runtime::definition_delta::{derive_plan, apply_migration}` —
// backbuild's own single dispatch point, mirroring the "drive the single
// dispatch point" rationale
// `recurrence_bound_probe_and_checked_merge_come_from_the_emitters` already
// documents. Plus the same result-equivalence leg (`multiset_equal` against a
// full refresh) the maintenance families carry.
// =============================================================================

/// Shared staging for the three backbuild legs below: writes every model in
/// `models` (each `(name, v1_sql)`), deploys them via a real
/// `execute_project` run so schema tracking records `model_sql`/columns for
/// each, then rewrites `target_model`'s file to `v2_sql`, re-discovers the
/// workspace, and re-derives the migration plan via `definition_delta::
/// derive_plan` — the same single derivation `smelt migrate`/the run gate/
/// `smelt explain` all read. Returns the derived plan and a fresh
/// `RecordingBackend` opened on the same DuckDB file (not yet applied — each
/// leg calls `apply_migration` itself, since the skeleton-change leg applies
/// a hand-built full-refresh plan rather than `derived.plan` itself, whose
/// `statements` are empty for a `SkeletonChange` verdict).
async fn stage_and_migrate(
    target_model: &str,
    models: &[(&str, &str)],
    v2_sql: &str,
) -> (
    smelt_runtime::definition_delta::DerivedPlan,
    RecordingBackend,
    tempfile::TempDir,
) {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project_dir = tmp.path().to_path_buf();
    std::fs::create_dir_all(project_dir.join("models")).unwrap();
    for (name, sql) in models {
        write_model(&project_dir, name, sql);
    }

    let db_path = project_dir.join("run.duckdb");
    let smelt_yml = format!(
        "name: backbuild_statement_parity_test\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: {db}\n    schema: main\ndefault_materialization: table\ntarget: dev\nstate:\n  mode: intervals\n",
        db = db_path.display()
    );
    std::fs::write(project_dir.join("smelt.yml"), &smelt_yml).unwrap();

    let config = Arc::new(Config::load(&project_dir).expect("load config"));

    // Deploy v1 through a real run so schema tracking records every model's
    // `model_sql` and columns.
    {
        let (db, graph) = build_db_and_graph(&project_dir, &config);
        let backend_slot: Arc<Mutex<Option<Arc<RecordingBackend>>>> = Arc::new(Mutex::new(None));
        let factory = RecordingBackendFactory {
            db_path: db_path.clone(),
            backend: backend_slot,
        };
        execute_project(
            "backbuild-parity-deploy".to_string(),
            make_request("dev", "2024-01-01", "2024-01-02"),
            Arc::clone(&config),
            graph,
            db,
            &project_dir,
            &factory,
            &smelt_runtime::NoOpReporter,
            CancellationToken::new(),
        )
        .await
        .expect("execute_project v1 deploy");
    }

    // Rewrite the target model to v2 and re-discover the workspace.
    write_model(&project_dir, target_model, v2_sql);
    let discovery = ModelDiscovery::new(project_dir.clone(), config.paths.clone());
    let sql_models = discovery.discover_models().expect("discover_models v2");

    let mut db2 = smelt_db::Database::default();
    let project = db2.set_project_input(project_dir.clone(), String::new());
    let source_files: Vec<_> = sql_models
        .iter()
        .map(|m| db2.set_source_file(m.path.clone(), m.content.clone(), project_dir.clone()))
        .collect();
    db2.set_workspace(source_files, vec![project]);
    db2.set_active_target(config.target.clone().map(|t| Arc::from(t.as_str())));

    let target = sql_models
        .iter()
        .find(|m| m.name == target_model)
        .expect("target model discovered")
        .clone();

    let file_store = smelt_state::file_store::FileStore::new(&project_dir, "dev");
    let deployed = file_store
        .load_schema(&target.db_name_owned())
        .expect("load deployed schema")
        .expect("the v1 deploy must have recorded a schema");
    let before_sql_raw = deployed
        .model_sql
        .clone()
        .expect("the v1 deploy must have recorded model_sql");

    let derived = derive_plan(
        &file_store,
        &target,
        &sql_models,
        None,
        &db2,
        &before_sql_raw,
        &deployed.columns,
    )
    .expect("derive_plan");

    let inner = DuckDbBackend::new(&db_path, "main")
        .await
        .expect("open duckdb for migration");
    let backend = RecordingBackend::new(inner);

    (derived, backend, tmp)
}

/// B1 (`Technique::SelfDerivedColumnAdd`): a new column that is a pure
/// function of existing stored columns backfills via `ALTER TABLE ADD
/// COLUMN` + an in-place `UPDATE`, both byte-identical to a direct
/// `emit_alter_add_column`/`emit_in_place_update` call over the plan's own
/// derived inputs — never merely emitter-shaped text.
#[tokio::test]
async fn backbuild_in_place_backfill_statements_come_from_the_emitter() {
    const V1: &str = "---\nmaterialization: table\n---\n\
SELECT id, amount, discount FROM (VALUES (1, 100, 20), (2, 200, 50)) AS t(id, amount, discount)\n";
    const V2: &str = "---\nmaterialization: table\n---\n\
SELECT id, amount, discount, amount - discount AS net_amount FROM (VALUES (1, 100, 20), (2, 200, 50)) AS t(id, amount, discount)\n";

    let (derived, backend, _tmp) = stage_and_migrate("net_orders", &[("net_orders", V1)], V2).await;

    assert_eq!(derived.plan.groups.len(), 1, "{:?}", derived.plan.groups);
    let group = &derived.plan.groups[0];
    assert_eq!(group.verdict, MigrationVerdict::BackfillInPlace);
    assert_eq!(group.options.len(), 1);
    assert_eq!(group.options[0].technique, Technique::SelfDerivedColumnAdd);

    let sql_type = derived
        .inputs
        .added_column_types
        .get("net_amount")
        .expect("net_amount type inferred")
        .clone();
    let expected_alter = emit_alter_add_column(&derived.inputs.table, "net_amount", &sql_type);
    let expected_update = emit_in_place_update(
        &derived.inputs.table,
        &[("net_amount".to_string(), "amount - discount".to_string())],
    );
    let expected_statements = vec![expected_alter, expected_update];
    assert_eq!(group.options[0].statements, expected_statements);
    assert_eq!(derived.plan.statements, expected_statements);

    apply_migration(&backend, &derived.plan)
        .await
        .expect("apply_migration");
    assert_eq!(
        backend.recorded_sql(),
        expected_statements,
        "executed SQL must be byte-identical to a direct emitter call over the plan's own inputs"
    );

    assert!(
        multiset_equal(
            &backend,
            &format!("SELECT * FROM {}", derived.inputs.table),
            "SELECT id, amount, discount, amount - discount AS net_amount FROM (VALUES \
             (1, 100, 20), (2, 200, 50)) AS t(id, amount, discount)"
        )
        .await,
        "the backfill statements must reproduce a full refresh of the after-definition"
    );
}

/// A skeleton (grain) change admits no in-place backfill technique
/// (`MigrationVerdict::SkeletonChange`) — the only honest route is the
/// always-present model-level `FullRefresh` baseline, byte-identical to a
/// direct `emit_full_refresh` call.
#[tokio::test]
async fn backbuild_full_refresh_statement_comes_from_the_emitter() {
    const V1: &str = "---\nmaterialization: table\n---\n\
SELECT id, amount, discount FROM (VALUES (1, 100, 20), (2, 200, 50)) AS t(id, amount, discount)\n";
    const V2_SKELETON_CHANGE: &str = "---\nmaterialization: table\n---\n\
SELECT id, amount, discount, count(*) AS n FROM (VALUES (1, 100, 20), (2, 200, 50)) AS t(id, amount, discount) GROUP BY id, amount, discount\n";

    let (derived, backend, _tmp) =
        stage_and_migrate("net_orders", &[("net_orders", V1)], V2_SKELETON_CHANGE).await;

    assert_eq!(derived.plan.groups.len(), 1, "{:?}", derived.plan.groups);
    assert_eq!(
        derived.plan.groups[0].verdict,
        MigrationVerdict::SkeletonChange
    );
    assert!(
        derived.plan.groups[0].options.is_empty(),
        "a skeleton change admits no targeted technique: {:?}",
        derived.plan.groups[0].options
    );

    let expected_full_refresh = emit_full_refresh(&derived.inputs.table, &derived.inputs.after_sql);
    assert_eq!(
        derived.plan.full_refresh.statements,
        vec![expected_full_refresh.clone()]
    );

    // The caller (not `derive_plan`) is the one that decides to fall back to
    // the full-refresh option on a `SkeletonChange` verdict — build that
    // plan explicitly, the same shape `apply_migration_executes_plan_
    // statements_in_order` (`crates/smelt-runtime/src/definition_delta.rs`)
    // hand-builds.
    let full_refresh_plan = MigrationPlan {
        model: derived.plan.model.clone(),
        table: derived.plan.table.clone(),
        groups: vec![],
        full_refresh: derived.plan.full_refresh.clone(),
        statements: derived.plan.full_refresh.statements.clone(),
    };
    apply_migration(&backend, &full_refresh_plan)
        .await
        .expect("apply_migration");
    assert_eq!(
        backend.recorded_sql(),
        vec![expected_full_refresh],
        "executed SQL must be byte-identical to a direct emit_full_refresh call"
    );

    assert!(
        multiset_equal(
            &backend,
            &format!(
                "SELECT id, amount, discount, n FROM {}",
                derived.inputs.table
            ),
            "SELECT id, amount, discount, count(*) AS n FROM (VALUES (1, 100, 20), (2, 200, 50)) \
             AS t(id, amount, discount) GROUP BY id, amount, discount"
        )
        .await,
        "the full-refresh statement must reproduce a full refresh of the after-definition"
    );
}

/// B3 (`Technique::UpstreamPullthrough`): an added column that pulls through
/// an upstream already in the FROM tree, bound via the upstream's declared
/// `unique_key`, backfills via `ALTER TABLE ADD COLUMN` + a column-scoped
/// `UPDATE ... FROM`, byte-identical to a direct `emit_alter_add_column`/
/// `emit_column_backfill_update_from` call.
#[tokio::test]
async fn backbuild_upstream_backfill_statements_come_from_the_emitter() {
    const CUSTOMERS: &str = "---\nmaterialization: table\nunique_key:\n  - customer_id\n---\n\
SELECT customer_id, name FROM (VALUES (1, 'Alice'), (2, 'Bob')) AS t(customer_id, name)\n";
    const ORDERS_V1: &str = "---\nmaterialization: table\n---\n\
SELECT o.order_id AS order_id, o.customer_id AS customer_id, \
customers.customer_id AS customers_customer_id \
FROM (VALUES (1, 1), (2, 2)) AS o(order_id, customer_id) \
JOIN smelt.customers AS customers ON o.customer_id = customers.customer_id\n";
    const ORDERS_V2: &str = "---\nmaterialization: table\n---\n\
SELECT o.order_id AS order_id, o.customer_id AS customer_id, \
customers.customer_id AS customers_customer_id, customers.name AS customer_name \
FROM (VALUES (1, 1), (2, 2)) AS o(order_id, customer_id) \
JOIN smelt.customers AS customers ON o.customer_id = customers.customer_id\n";

    let (derived, backend, _tmp) = stage_and_migrate(
        "orders",
        &[("customers", CUSTOMERS), ("orders", ORDERS_V1)],
        ORDERS_V2,
    )
    .await;

    assert_eq!(derived.plan.groups.len(), 1, "{:?}", derived.plan.groups);
    let group = &derived.plan.groups[0];
    assert_eq!(group.verdict, MigrationVerdict::Rederive);
    assert_eq!(group.options.len(), 1);
    assert_eq!(group.options[0].technique, Technique::UpstreamPullthrough);

    let sql_type = derived
        .inputs
        .added_column_types
        .get("customer_name")
        .expect("customer_name type inferred")
        .clone();
    let expected_alter = emit_alter_add_column(&derived.inputs.table, "customer_name", &sql_type);
    let expected_update = emit_column_backfill_update_from(
        &derived.inputs.table,
        &[("customer_name".to_string(), "u.name".to_string())],
        "customers",
        "u",
        &[(
            "customers_customer_id".to_string(),
            "customer_id".to_string(),
        )],
    );
    let expected_statements = vec![expected_alter, expected_update];
    assert_eq!(group.options[0].statements, expected_statements);
    assert_eq!(derived.plan.statements, expected_statements);

    apply_migration(&backend, &derived.plan)
        .await
        .expect("apply_migration");
    assert_eq!(
        backend.recorded_sql(),
        expected_statements,
        "executed SQL must be byte-identical to a direct emitter call over the plan's own inputs"
    );

    assert!(
        multiset_equal(
            &backend,
            &format!("SELECT * FROM {}", derived.inputs.table),
            "SELECT o.order_id AS order_id, o.customer_id AS customer_id, \
             customers.customer_id AS customers_customer_id, customers.name AS customer_name \
             FROM (VALUES (1, 1), (2, 2)) AS o(order_id, customer_id) \
             JOIN customers ON o.customer_id = customers.customer_id"
        )
        .await,
        "the backfill statements must reproduce a full refresh of the after-definition"
    );
}
