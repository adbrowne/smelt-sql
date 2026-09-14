//! `statement_parity`'s Trino leg (`docs/outcomes/20260913-trino-incremental/
//! phases/03h-plan.md`): the whole-row `MERGE` upsert (keyed-fold) family's
//! executed statements, captured off a real `TrinoBackend` during a real
//! `execute_project` run, must be byte-identical to a direct
//! `emit_keyed_fold_suppressed` call with the batch's own inputs — the same
//! shape `structural_and_ledger.rs::snapshot_reconcile_delete_leg_parity`
//! proves for DuckDB.
//!
//! Local env gate (`common::trino_env`, mirroring `crates/smelt-cli/tests/
//! common/mod.rs::trino_env`) — skips with an explicit `Skipping …` line
//! rather than silently no-op'ing. Per 3e's isolation rule this test gets
//! its own process-unique schema (`common::trino_schema`), dropped
//! unconditionally at the end.

use super::*;
use smelt_backend_trino::{TrinoBackend, TrinoClientConfig};

use crate::common::TrinoEnv;

struct TrinoRecordingBackendFactory {
    base_url: String,
    user: String,
    catalog: String,
    backend: Arc<Mutex<Option<Arc<RecordingBackend>>>>,
}

impl BackendFactory for TrinoRecordingBackendFactory {
    fn create<'a>(
        &'a self,
        _target_name: &'a str,
        target_config: &'a Target,
        _project_dir: &'a Path,
    ) -> BackendFuture<'a> {
        let base_url = self.base_url.clone();
        let user = self.user.clone();
        let catalog = self.catalog.clone();
        let schema = target_config.schema.clone();
        let slot = Arc::clone(&self.backend);
        Box::pin(async move {
            let inner = TrinoBackend::new(TrinoClientConfig {
                base_url,
                user,
                catalog,
                schema,
                password: None,
            });
            let recording = Arc::new(RecordingBackend::new(Box::new(inner)));
            *slot.lock().unwrap() = Some(Arc::clone(&recording));
            Ok(Box::new(ArcBackend(recording)) as Box<dyn Backend>)
        })
    }
}

fn stage_keyed_fold_project(project_dir: &Path, env: &TrinoEnv, schema: &str) {
    std::fs::create_dir_all(project_dir.join("models/sources")).unwrap();

    std::fs::write(
        project_dir.join("models/sources/events.yml"),
        "description: Clocked per-device events.\n\
         columns:\n\
         \x20\x20- name: device_id\n\
         \x20\x20\x20\x20type: INTEGER\n\
         \x20\x20- name: event_date\n\
         \x20\x20\x20\x20type: DATE\n\
         \x20\x20- name: amount\n\
         \x20\x20\x20\x20type: DOUBLE\n\
         timeseries:\n\
         \x20\x20event_time_column: event_date\n\
         \x20\x20partition_column: event_date\n\
         \x20\x20granularity: day\n\
         mutation_profile:\n\
         \x20\x20kind: append_only\n",
    )
    .unwrap();

    write_model(
        project_dir,
        "device_agg",
        "---\n\
         materialization: table\n\
         refresh: incremental\n\
         grain: key\n\
         maintenance:\n\
         \x20\x20scan_bounds:\n\
         \x20\x20\x20\x20per_source:\n\
         \x20\x20\x20\x20\x20\x20events:\n\
         \x20\x20\x20\x20\x20\x20\x20\x20allow_full_scan: true\n\
         ---\n\
         SELECT device_id, MIN(amount) AS agg_amount FROM smelt.sources.events GROUP BY 1",
    );

    let smelt_yml = format!(
        "name: trino_statement_parity_keyed_fold\nversion: 1\npaths:\n  - models\n\
         targets:\n  dev:\n    type: trino\n    host: {host}\n    port: {port}\n    \
         user: {user}\n    catalog: {catalog}\n    schema: {schema}\n    tls: {tls}\n\
         default_materialization: table\ntarget: dev\n",
        host = env.host,
        port = env.port,
        user = env.user,
        catalog = env.catalog,
        tls = env.tls,
    );
    std::fs::write(project_dir.join("smelt.yml"), smelt_yml).unwrap();
}

/// Test 4 (`docs/outcomes/20260913-trino-incremental/phases/03h-plan.md`):
/// a `RecordingBackend` wrapping a real `TrinoBackend` captures the
/// `StatementGroup`s a real `execute_project` run sends for the whole-row
/// `MERGE` upsert (keyed-fold) family's idempotent (`MIN`-combiner) shape —
/// one window spanning both driving-source partitions, so step 1 hits the
/// first-run `CREATE TABLE … AS` arm and step 2 hits the `MERGE` arm,
/// mirroring `region_and_keyed_fold.rs::keyed_fold_statements_come_from_the_emitter`'s
/// DuckDB shape. The captured `MERGE` group is asserted byte-identical to a
/// direct `emit_keyed_fold_suppressed` call with the batch's own inputs.
#[tokio::test]
async fn keyed_fold_parity_on_trino() {
    let Some(env) = common::trino_env() else {
        eprintln!("SMELT_TRINO_URL unset — skipping keyed_fold_parity_on_trino");
        return;
    };
    let schema = common::trino_schema("keyed_fold");

    let backend = common::trino_backend(&schema);
    backend
        .execute_sql(&format!("CREATE SCHEMA IF NOT EXISTS {schema}"))
        .await
        .expect("create schema");
    backend
        .execute_sql(&format!(
            "CREATE TABLE {schema}.sources_events (device_id INTEGER, event_date DATE, \
             amount DOUBLE)"
        ))
        .await
        .expect("create source table");
    backend
        .execute_sql(&format!(
            "INSERT INTO {schema}.sources_events VALUES \
             (1, DATE '2026-01-01', 50.0), (2, DATE '2026-01-01', 20.0), \
             (1, DATE '2026-01-02', 5.0), (3, DATE '2026-01-02', 30.0)"
        ))
        .await
        .expect("seed source table");

    let tmp = tempfile::tempdir().expect("tempdir");
    let project_dir = tmp.path();
    stage_keyed_fold_project(project_dir, &env, &schema);

    let config = Arc::new(Config::load(project_dir).expect("load config"));
    let (db, graph) = build_db_and_graph(project_dir, &config);
    let backend_slot: Arc<Mutex<Option<Arc<RecordingBackend>>>> = Arc::new(Mutex::new(None));
    let scheme = if env.tls { "https" } else { "http" };
    let factory = TrinoRecordingBackendFactory {
        base_url: format!("{scheme}://{}:{}", env.host, env.port),
        user: env.user.clone(),
        catalog: env.catalog.clone(),
        backend: Arc::clone(&backend_slot),
    };

    // One window covering both driving-source partitions: step 1
    // (2026-01-01) hits the first-run CREATE arm; step 2 (2026-01-02) hits
    // the MERGE arm — the leg this test asserts against.
    let request = make_request("dev", "2026-01-01", "2026-01-03");
    let outcome = execute_project(
        "trino-keyed-fold-statement-parity".to_string(),
        request,
        Arc::clone(&config),
        graph,
        db,
        project_dir,
        &factory,
        &smelt_runtime::NoOpReporter,
        CancellationToken::new(),
    )
    .await;

    if let Err(e) = &outcome {
        common::drop_trino_schema(&schema).await;
        panic!("execute_project (Trino keyed fold) failed: {e}");
    }
    let outcome = outcome.unwrap();
    assert!(
        outcome.models.contains_key("device_agg"),
        "device_agg must have run: {:?}",
        outcome.models.keys().collect::<Vec<_>>()
    );

    let recorded = backend_slot
        .lock()
        .unwrap()
        .clone()
        .expect("backend recorded");
    let groups = recorded.recorded_groups();
    assert_eq!(
        groups.len(),
        2,
        "two steps must each execute exactly one statement group: {groups:?}"
    );

    let merge_sql = &groups[1].statements[0].sql;
    let prefix = format!("MERGE INTO {schema}.device_agg AS target USING (");
    let using_start = merge_sql.find("USING (").map(|i| i + "USING (".len());
    let delta_end = merge_sql.rfind(") AS delta ON");

    let (using_start, delta_end) = match (using_start, delta_end) {
        (Some(s), Some(e)) => (s, e),
        _ => {
            common::drop_trino_schema(&schema).await;
            panic!("unexpected merge statement shape: {merge_sql}");
        }
    };
    assert!(
        merge_sql.starts_with(&prefix),
        "unexpected merge statement: {merge_sql}"
    );
    let delta_select = &merge_sql[using_start..delta_end];

    let expected_merge = emit_keyed_fold_suppressed(
        &format!("{schema}.device_agg"),
        &["device_id".to_string()],
        &[(
            "agg_amount".to_string(),
            "LEAST(target.agg_amount, delta.agg_amount)".to_string(),
        )],
        delta_select,
        None,
        &["agg_amount".to_string()],
        MaintenanceDialect::Trino,
    );

    let matches = expected_merge == groups[1];
    if !matches {
        common::drop_trino_schema(&schema).await;
    }
    assert_eq!(
        expected_merge, groups[1],
        "executed MERGE group must be byte-identical to a direct emit_keyed_fold_suppressed call"
    );

    common::drop_trino_schema(&schema).await;
}

fn stage_delete_insert_project(project_dir: &Path, env: &TrinoEnv, schema: &str) {
    std::fs::create_dir_all(project_dir.join("models")).unwrap();

    // Self-contained: no upstream ref/source needed to exercise the region
    // DELETE+INSERT family — the output clamp wraps the model's own SELECT
    // regardless of where its data comes from (`region_and_keyed_fold.rs`'s
    // DuckDB precedent). The explicit outer `CAST(event_date AS DATE)`
    // (rather than a bare `SELECT *` over the VALUES literal) is what lets
    // smelt's own type inference resolve `event_date` as a genuine `Date`
    // column — `stage_calendar_partition_project`'s precedent in
    // `crates/smelt-cli/tests/trino_incremental_families.rs` — so the
    // injected output-clamp literal renders `DATE '…'`-typed (gap 1's fix)
    // rather than a bare quoted string a strict engine refuses.
    write_model(
        project_dir,
        "daily_mart",
        "---\n\
         materialization: table\n\
         refresh: incremental\n\
         grain: partition\n\
         timeseries:\n\
         \x20\x20partition_column: event_date\n\
         \x20\x20event_time_column: event_date\n\
         \x20\x20granularity: day\n\
         ---\n\
         SELECT CAST(event_date AS DATE) AS event_date, amount FROM (VALUES \
         (DATE '2026-01-01', 10), (DATE '2026-01-02', 20)) AS t(event_date, amount)",
    );

    let smelt_yml = format!(
        "name: trino_statement_parity_delete_insert\nversion: 1\npaths:\n  - models\n\
         targets:\n  dev:\n    type: trino\n    host: {host}\n    port: {port}\n    \
         user: {user}\n    catalog: {catalog}\n    schema: {schema}\n    tls: {tls}\n\
         default_materialization: table\ntarget: dev\n",
        host = env.host,
        port = env.port,
        user = env.user,
        catalog = env.catalog,
        tls = env.tls,
    );
    std::fs::write(project_dir.join("smelt.yml"), smelt_yml).unwrap();
}

/// Test 3 (`docs/outcomes/20260913-trino-incremental/phases/04-plan.md`):
/// the region `DELETE`+`INSERT` family's (`Technique::DeleteInsert`)
/// executed statements, captured by a `RecordingBackend` wrapping a real
/// `TrinoBackend` during a real `execute_project` run, are byte-identical
/// to a direct `emit_delete_insert` call with the batch's own inputs — the
/// same shape `region_and_keyed_fold.rs::
/// region_recompute_statements_come_from_the_emitter` proves for DuckDB.
/// Also asserts criterion 4's "asserted directly": the `DELETE`'s two
/// literals are exactly the batch's own bounds, the same two literals the
/// `INSERT` body's injected output clamp carries (recovered from the
/// executed text itself, not independently rederived).
#[tokio::test]
async fn delete_insert_parity_on_trino() {
    let Some(env) = common::trino_env() else {
        eprintln!("SMELT_TRINO_URL unset — skipping delete_insert_parity_on_trino");
        return;
    };
    let schema = common::trino_schema("delete_insert");

    let tmp = tempfile::tempdir().expect("tempdir");
    let project_dir = tmp.path();
    stage_delete_insert_project(project_dir, &env, &schema);

    let config = Arc::new(Config::load(project_dir).expect("load config"));
    let scheme = if env.tls { "https" } else { "http" };

    // Run 1: the table does not exist yet — this run always hits the
    // first-run `create_table_as` arm, never `delete_and_insert_transactional`.
    {
        let (db, graph) = build_db_and_graph(project_dir, &config);
        let backend_slot: Arc<Mutex<Option<Arc<RecordingBackend>>>> = Arc::new(Mutex::new(None));
        let factory = TrinoRecordingBackendFactory {
            base_url: format!("{scheme}://{}:{}", env.host, env.port),
            user: env.user.clone(),
            catalog: env.catalog.clone(),
            backend: Arc::clone(&backend_slot),
        };
        let outcome = execute_project(
            "trino-delete-insert-statement-parity-run-1".to_string(),
            make_request("dev", "2026-01-01", "2026-01-02"),
            Arc::clone(&config),
            graph,
            db,
            project_dir,
            &factory,
            &smelt_runtime::NoOpReporter,
            CancellationToken::new(),
        )
        .await;
        if let Err(e) = &outcome {
            common::drop_trino_schema(&schema).await;
            panic!("execute_project run 1 (first-run create) failed: {e}");
        }
    }

    // Run 2: the table exists — this run must dispatch `IncrementalStrategy::
    // DeleteInsert`, and its statements are what this test asserts against.
    let (db, graph) = build_db_and_graph(project_dir, &config);
    let backend_slot: Arc<Mutex<Option<Arc<RecordingBackend>>>> = Arc::new(Mutex::new(None));
    let factory = TrinoRecordingBackendFactory {
        base_url: format!("{scheme}://{}:{}", env.host, env.port),
        user: env.user.clone(),
        catalog: env.catalog.clone(),
        backend: Arc::clone(&backend_slot),
    };

    let outcome = execute_project(
        "trino-delete-insert-statement-parity-run-2".to_string(),
        make_request("dev", "2026-01-01", "2026-01-03"),
        Arc::clone(&config),
        graph,
        db,
        project_dir,
        &factory,
        &smelt_runtime::NoOpReporter,
        CancellationToken::new(),
    )
    .await;

    if let Err(e) = &outcome {
        common::drop_trino_schema(&schema).await;
        panic!("execute_project (Trino delete+insert) failed: {e}");
    }
    let outcome = outcome.unwrap();
    assert!(
        outcome.models.contains_key("daily_mart"),
        "daily_mart must have run: {:?}",
        outcome.models.keys().collect::<Vec<_>>()
    );

    let recorded = backend_slot
        .lock()
        .unwrap()
        .clone()
        .expect("backend recorded");
    let groups = recorded.recorded_groups();
    assert_eq!(
        groups.len(),
        1,
        "one DELETE+INSERT group must have executed: {groups:?}"
    );

    let group = &groups[0];
    assert!(
        group.transactional,
        "region DELETE+INSERT must be transactional"
    );
    assert_eq!(group.statements.len(), 2);

    // `execute_model_incremental_with_bookkeeping`'s `IncrementalStrategy::
    // DeleteInsert` arm (`crates/smelt-backend/src/lib.rs`) — the real
    // dispatch path a `refresh: incremental`/`grain: partition` model's
    // steady-state run takes — builds the group via the dialect-agnostic
    // `build_delete_insert_group(schema, name, ...)` with a bare, unquoted
    // `schema.table` name; it never routes through `Backend::
    // delete_and_insert_transactional`, so the executed text is not
    // catalog-qualified here (that override's own target, `insert_overwrite`,
    // has no production caller today — the same "unreached" situation
    // `delete_partitions` is in — and is proved separately by this file's
    // unit test in `crates/smelt-backend-trino/src/backend.rs`).
    let table_name = format!("{schema}.daily_mart");
    let delete_prefix = format!("DELETE FROM {table_name} WHERE ");
    let insert_prefix = format!("INSERT INTO {table_name} ");

    let delete_sql = &group.statements[0].sql;
    let insert_sql = &group.statements[1].sql;
    assert!(
        delete_sql.starts_with(&delete_prefix),
        "unexpected delete shape: {delete_sql}"
    );
    assert!(
        insert_sql.starts_with(&insert_prefix),
        "unexpected insert shape: {insert_sql}"
    );

    // Recover the region literals from the executed DELETE's own WHERE
    // clause — proving the executed text is exactly what the emitter
    // produces, and that the DELETE's two literals are the same ones the
    // INSERT body's injected output clamp carries (they must be, since both
    // come from the same batch's PartitionRange, but this is the direct
    // assertion criterion 4 asks for).
    let where_clause = delete_sql
        .strip_prefix(&delete_prefix)
        .expect("delete shape");
    let parts: Vec<&str> = where_clause.split(" AND ").collect();
    let start_lit = parts[0]
        .strip_prefix("event_date >= ")
        .expect("start literal");
    let end_lit = parts[1].strip_prefix("event_date < ").expect("end literal");
    assert!(
        insert_sql.contains(start_lit) && insert_sql.contains(end_lit),
        "the INSERT body's injected output clamp must carry the same two literals the DELETE \
         covers: delete={delete_sql} insert={insert_sql}"
    );

    let body = insert_sql
        .strip_prefix(&insert_prefix)
        .expect("insert shape");
    let region = Region {
        start: start_lit.to_string(),
        end: end_lit.to_string(),
    };
    let expected = emit_delete_insert(
        &table_name,
        "event_date",
        &region,
        body,
        MaintenanceDialect::Trino,
    );
    let matches = &expected == group;
    if !matches {
        common::drop_trino_schema(&schema).await;
    }
    assert_eq!(
        &expected, group,
        "executed group must be byte-identical to a direct emitter call over the same inputs"
    );

    // Result-equivalence, mirroring region_and_keyed_fold.rs's DuckDB proof:
    // the DELETE+INSERT statements actually executed must leave `daily_mart`
    // multiset-equal to a full refresh of the model's own SQL.
    let full_refresh_sql = "SELECT * FROM (VALUES (DATE '2026-01-01', 10), \
                             (DATE '2026-01-02', 20)) AS t(event_date, amount)";
    let equal = multiset_equal(
        recorded.as_ref(),
        &format!("SELECT * FROM {table_name}"),
        full_refresh_sql,
    )
    .await;
    if !equal {
        common::drop_trino_schema(&schema).await;
    }
    assert!(
        equal,
        "the DELETE+INSERT statements execute_project actually ran must reproduce a full refresh"
    );

    common::drop_trino_schema(&schema).await;
}
