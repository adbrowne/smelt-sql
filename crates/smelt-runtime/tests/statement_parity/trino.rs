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
