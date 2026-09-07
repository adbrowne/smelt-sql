//! 27e (`docs/outcomes/20260815-definition-delta-migrate/phases/27e-plan.md`):
//! the fingerprint sidecar's synthesized changed-key delta for an external
//! `mutable_snapshot` source reaches LIVE delta-restriction dispatch through
//! `execute_project` itself — not only the direct-driver-call proof in
//! `crates/smelt-runtime/tests/technique_lowering.rs`'s
//! `external_source_point_lookup_recompute` module, which states outright
//! that it drives the mechanism "directly rather than through
//! `execute_project`".
//!
//! Fixture: a custom model (`probe_model.sql`, written into a copy of
//! `examples/timeseries`) joining the append-only `raw.events` fact to the
//! `mutable_snapshot` `raw.users` dimension (declared `unique_key: [user_id]`
//! and `referential_integrity: [user_id]`) — the SAME shape
//! `daily_events_enriched.sql` uses, plus one extra `RANDOM() AS jitter`
//! output column with no P3 comparability proof. That extra column is
//! deliberate: it makes the phase-27c keyless whole-row staged-candidate
//! mechanism (`resolve_live_membership_recompute_cell`'s `StagedKeyless` arm)
//! refuse fail-closed (`WriteSuppression::Unconditional`, "column(s) jitter
//! are not proven comparable"), which otherwise ALWAYS wins the same
//! `UpstreamMutation` cell ahead of this phase's external-sidecar dispatch
//! for any `grain: partition` model (`RowIdentity` is unconditionally
//! `WholeRow` there) — so this is the fixture where the branch this phase
//! adds is actually the one that executes, not merely wired-but-dead code.
//! `jitter` is excluded from every oracle/result comparison below (it is
//! intentionally non-deterministic).

use std::path::Path;
use std::sync::{Arc, Mutex};

use arrow::array::{Array, RecordBatch, StringArray};
use arrow::datatypes::SchemaRef;
use async_trait::async_trait;
use smelt_backend::{Backend, BackendCapabilities, BackendError, PartitionRange, StatementGroup};
use smelt_backend_duckdb::DuckDbBackend;
use smelt_core::config::{Config, Target};
use smelt_core::graph::DependencyGraph;
use smelt_core::ModelDiscovery;
use smelt_runtime::execute::{execute_project, BackendFactory, BackendFuture};
use smelt_runtime::types::ExecuteRequest;
use smelt_runtime::NoOpReporter;
use tokio_util::sync::CancellationToken;

/// A thin `Backend` wrapper recording every `StatementGroup` handed to
/// `execute_statement_group` — the DELETE+INSERT the external-sidecar branch
/// dispatches through — mirroring `tests/statement_parity.rs`'s own
/// `RecordingBackend`/`ArcBackend` pair.
struct RecordingBackend {
    inner: DuckDbBackend,
    groups: Mutex<Vec<StatementGroup>>,
}

impl RecordingBackend {
    fn new(inner: DuckDbBackend) -> Self {
        Self {
            inner,
            groups: Mutex::new(Vec::new()),
        }
    }

    fn recorded_groups(&self) -> Vec<StatementGroup> {
        self.groups.lock().unwrap().clone()
    }
}

#[async_trait]
impl Backend for RecordingBackend {
    async fn execute_sql(&self, sql: &str) -> Result<Vec<RecordBatch>, BackendError> {
        self.inner.execute_sql(sql).await
    }
    async fn create_table_as(
        &self,
        schema: &str,
        name: &str,
        sql: &str,
    ) -> Result<(), BackendError> {
        self.inner.create_table_as(schema, name, sql).await
    }
    async fn create_view_as(
        &self,
        schema: &str,
        name: &str,
        sql: &str,
    ) -> Result<(), BackendError> {
        self.inner.create_view_as(schema, name, sql).await
    }
    async fn drop_table_if_exists(&self, schema: &str, name: &str) -> Result<(), BackendError> {
        self.inner.drop_table_if_exists(schema, name).await
    }
    async fn drop_view_if_exists(&self, schema: &str, name: &str) -> Result<(), BackendError> {
        self.inner.drop_view_if_exists(schema, name).await
    }
    async fn get_row_count(&self, schema: &str, name: &str) -> Result<usize, BackendError> {
        self.inner.get_row_count(schema, name).await
    }
    async fn get_preview(
        &self,
        schema: &str,
        name: &str,
        limit: usize,
    ) -> Result<Vec<RecordBatch>, BackendError> {
        self.inner.get_preview(schema, name, limit).await
    }
    async fn table_exists(&self, schema: &str, name: &str) -> Result<bool, BackendError> {
        self.inner.table_exists(schema, name).await
    }
    async fn ensure_schema(&self, schema: &str) -> Result<(), BackendError> {
        self.inner.ensure_schema(schema).await
    }
    fn dialect(&self) -> smelt_backend::SqlDialect {
        self.inner.dialect()
    }
    fn capabilities(&self) -> BackendCapabilities {
        self.inner.capabilities()
    }
    async fn load_table(
        &self,
        schema: &str,
        name: &str,
        arrow_schema: SchemaRef,
        batches: Vec<RecordBatch>,
    ) -> Result<(), BackendError> {
        self.inner
            .load_table(schema, name, arrow_schema, batches)
            .await
    }
    async fn delete_partitions(
        &self,
        schema: &str,
        name: &str,
        partition: &PartitionRange,
    ) -> Result<(), BackendError> {
        self.inner.delete_partitions(schema, name, partition).await
    }
    async fn insert_into_from_query(
        &self,
        schema: &str,
        name: &str,
        sql: &str,
    ) -> Result<(), BackendError> {
        self.inner.insert_into_from_query(schema, name, sql).await
    }
    async fn insert_overwrite(
        &self,
        schema: &str,
        table: &str,
        sql: &str,
        partition: &PartitionRange,
    ) -> Result<(), BackendError> {
        self.inner
            .insert_overwrite(schema, table, sql, partition)
            .await
    }
    async fn execute_statement_group(&self, group: &StatementGroup) -> Result<(), BackendError> {
        self.groups.lock().unwrap().push(group.clone());
        self.inner.execute_statement_group(group).await
    }
}

struct RecordingBackendFactory {
    db_path: std::path::PathBuf,
    backend: Arc<Mutex<Option<Arc<RecordingBackend>>>>,
}

impl BackendFactory for RecordingBackendFactory {
    fn create<'a>(
        &'a self,
        _target_name: &'a str,
        target_config: &'a Target,
        _project_dir: &'a Path,
    ) -> BackendFuture<'a> {
        let path = self.db_path.clone();
        let schema = target_config.schema.clone();
        let slot = Arc::clone(&self.backend);
        Box::pin(async move {
            let inner = DuckDbBackend::new(&path, &schema)
                .await
                .map_err(|e| anyhow::anyhow!("DuckDB init failed: {}", e))?;
            let recording = Arc::new(RecordingBackend::new(inner));
            *slot.lock().unwrap() = Some(Arc::clone(&recording));
            Ok(Box::new(ArcBackend(recording)) as Box<dyn Backend>)
        })
    }
}

/// Thin `Backend` forwarder over an `Arc<RecordingBackend>` so the same
/// instance can be returned to `execute_project` (which needs ownership)
/// while the test keeps its own handle to read the recording back.
struct ArcBackend(Arc<RecordingBackend>);

#[async_trait]
impl Backend for ArcBackend {
    async fn execute_sql(&self, sql: &str) -> Result<Vec<RecordBatch>, BackendError> {
        self.0.execute_sql(sql).await
    }
    async fn create_table_as(
        &self,
        schema: &str,
        name: &str,
        sql: &str,
    ) -> Result<(), BackendError> {
        self.0.create_table_as(schema, name, sql).await
    }
    async fn create_view_as(
        &self,
        schema: &str,
        name: &str,
        sql: &str,
    ) -> Result<(), BackendError> {
        self.0.create_view_as(schema, name, sql).await
    }
    async fn drop_table_if_exists(&self, schema: &str, name: &str) -> Result<(), BackendError> {
        self.0.drop_table_if_exists(schema, name).await
    }
    async fn drop_view_if_exists(&self, schema: &str, name: &str) -> Result<(), BackendError> {
        self.0.drop_view_if_exists(schema, name).await
    }
    async fn get_row_count(&self, schema: &str, name: &str) -> Result<usize, BackendError> {
        self.0.get_row_count(schema, name).await
    }
    async fn get_preview(
        &self,
        schema: &str,
        name: &str,
        limit: usize,
    ) -> Result<Vec<RecordBatch>, BackendError> {
        self.0.get_preview(schema, name, limit).await
    }
    async fn table_exists(&self, schema: &str, name: &str) -> Result<bool, BackendError> {
        self.0.table_exists(schema, name).await
    }
    async fn ensure_schema(&self, schema: &str) -> Result<(), BackendError> {
        self.0.ensure_schema(schema).await
    }
    fn dialect(&self) -> smelt_backend::SqlDialect {
        self.0.dialect()
    }
    fn capabilities(&self) -> BackendCapabilities {
        self.0.capabilities()
    }
    async fn load_table(
        &self,
        schema: &str,
        name: &str,
        arrow_schema: SchemaRef,
        batches: Vec<RecordBatch>,
    ) -> Result<(), BackendError> {
        self.0.load_table(schema, name, arrow_schema, batches).await
    }
    async fn delete_partitions(
        &self,
        schema: &str,
        name: &str,
        partition: &PartitionRange,
    ) -> Result<(), BackendError> {
        self.0.delete_partitions(schema, name, partition).await
    }
    async fn insert_into_from_query(
        &self,
        schema: &str,
        name: &str,
        sql: &str,
    ) -> Result<(), BackendError> {
        self.0.insert_into_from_query(schema, name, sql).await
    }
    async fn insert_overwrite(
        &self,
        schema: &str,
        table: &str,
        sql: &str,
        partition: &PartitionRange,
    ) -> Result<(), BackendError> {
        self.0.insert_overwrite(schema, table, sql, partition).await
    }
    async fn execute_statement_group(&self, group: &StatementGroup) -> Result<(), BackendError> {
        self.0.execute_statement_group(group).await
    }
}

fn copy_dir_recursive(src: &Path, dst: &Path) {
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

fn build_db_and_graph(
    project_dir: &Path,
    config: &Config,
) -> (
    Arc<tokio::sync::Mutex<smelt_db::Database>>,
    Arc<tokio::sync::Mutex<DependencyGraph>>,
) {
    let discovery = ModelDiscovery::new(project_dir.to_path_buf(), config.paths.clone());
    let sql_models = discovery.discover_models().expect("discover_models");

    let mut db = smelt_db::Database::default();
    let project = db.set_project_input(project_dir.to_path_buf(), String::new());
    let source_files: Vec<_> = sql_models
        .iter()
        .map(|m| db.set_source_file(m.path.clone(), m.content.clone(), project_dir.to_path_buf()))
        .collect();
    db.set_workspace(source_files, vec![project]);
    db.set_active_target(Some(std::sync::Arc::from("dev")));

    let graph = DependencyGraph::build(sql_models, None).expect("build graph");

    (
        Arc::new(tokio::sync::Mutex::new(db)),
        Arc::new(tokio::sync::Mutex::new(graph)),
    )
}

fn request_for_day(start: &str, end: &str) -> ExecuteRequest {
    ExecuteRequest {
        target: "dev".to_string(),
        select: vec!["probe_model".to_string()],
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

async fn seed(backend: &dyn Backend) {
    backend
        .execute_sql(
            "CREATE TABLE main.sources_raw_events (event_id INTEGER, user_id INTEGER, \
             event_type VARCHAR, event_timestamp TIMESTAMP)",
        )
        .await
        .expect("create events source table");
    backend
        .execute_sql(
            "INSERT INTO main.sources_raw_events VALUES \
             (1, 1, 'login', TIMESTAMP '2025-01-10 08:00:00'), \
             (2, 1, 'click', TIMESTAMP '2025-01-10 09:00:00'), \
             (3, 2, 'login', TIMESTAMP '2025-01-10 10:00:00'), \
             (4, 2, 'click', TIMESTAMP '2025-01-10 11:00:00'), \
             (5, 3, 'login', TIMESTAMP '2025-01-10 12:00:00'), \
             (6, 3, 'click', TIMESTAMP '2025-01-10 13:00:00')",
        )
        .await
        .expect("seed events");
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
             (1, 'Alice', DATE '2025-01-01'), \
             (2, 'Bob', DATE '2025-01-02'), \
             (3, 'Carol', DATE '2025-01-03')",
        )
        .await
        .expect("seed users");
}

async fn user_names(backend: &dyn Backend) -> Vec<(i32, String)> {
    let batches = backend
        .execute_sql("SELECT user_id, user_name FROM main.probe_model ORDER BY user_id, event_id")
        .await
        .expect("read maintained table");
    let mut out = Vec::new();
    for batch in &batches {
        use arrow::array::Int32Array;
        let ids = batch
            .column(0)
            .as_any()
            .downcast_ref::<Int32Array>()
            .expect("user_id is INTEGER");
        let names = batch
            .column(1)
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("user_name is VARCHAR");
        for i in 0..batch.num_rows() {
            out.push((ids.value(i), names.value(i).to_string()));
        }
    }
    out
}

/// The `RANDOM() AS jitter` extra output column is what keeps the pre-
/// existing phase-27c keyless whole-row staged-candidate mechanism
/// (`resolve_live_membership_recompute_cell`) from claiming this model's
/// `raw.users` `UpstreamMutation` cell ahead of this phase's own dispatch —
/// see the module doc comment for why that matters.
const PROBE_MODEL_FILE: &str = "---\n\
     materialization: table\n\
     refresh: incremental\n\
     grain: partition\n\
     timeseries:\n  \
       partition_column: event_date\n  \
       event_time_column: event_timestamp\n  \
       granularity: day\n\
     maintenance:\n  \
       scan_bounds:\n    \
         per_source:\n      \
           raw.users:\n        \
             allow_full_scan: true\n\
     ---\n\
     SELECT\n    \
         e.event_id,\n    \
         CAST(e.event_timestamp AS DATE) AS event_date,\n    \
         e.user_id,\n    \
         e.event_type,\n    \
         u.user_name,\n    \
         RANDOM() AS jitter\n\
     FROM smelt.sources.raw.events e\n\
     JOIN smelt.sources.raw.users u ON e.user_id = u.user_id\n";

fn setup_project() -> (
    tempfile::TempDir,
    std::path::PathBuf,
    std::path::PathBuf,
    Arc<Config>,
) {
    let source_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/timeseries")
        .canonicalize()
        .expect("examples/timeseries exists");
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().join("project");
    copy_dir_recursive(&source_dir, &project_dir);
    std::fs::write(project_dir.join("models/probe_model.sql"), PROBE_MODEL_FILE)
        .expect("write probe_model fixture");
    let db_path = tmp.path().join("run.duckdb");
    let config = Arc::new(Config::load(&project_dir).expect("load smelt.yml"));
    (tmp, project_dir, db_path, config)
}

/// One renamed user out of three: the THIRD run over the SAME already-
/// materialized day (after a second run has already populated the sidecar
/// baseline against the pre-rename content) must dispatch the external-
/// sidecar-restricted DELETE+INSERT — the recorded statement group carries
/// the `user_id IN (...)` semi-join predicate naming exactly the renamed
/// user — and the maintained table ends up byte-equal to a from-scratch
/// full-refresh oracle.
#[tokio::test]
async fn mutation_of_one_dimension_row_restricts_the_recompute() {
    let (_tmp, project_dir, db_path, config) = setup_project();
    let backend_slot: Arc<Mutex<Option<Arc<RecordingBackend>>>> = Arc::new(Mutex::new(None));
    let factory = RecordingBackendFactory {
        db_path: db_path.clone(),
        backend: Arc::clone(&backend_slot),
    };

    {
        let backend = DuckDbBackend::new(&db_path, "main")
            .await
            .expect("open duckdb");
        seed(&backend).await;
    }

    // Run 1: creation. The target doesn't exist yet, so the ordinary
    // `CREATE TABLE AS` bootstrap runs — restriction only ever applies to a
    // recompute over an already-materialized target.
    {
        let (db, graph) = build_db_and_graph(&project_dir, &config);
        execute_project(
            "run-1".to_string(),
            request_for_day("2025-01-10", "2025-01-11"),
            Arc::clone(&config),
            graph,
            db,
            &project_dir,
            &factory,
            &NoOpReporter,
            CancellationToken::new(),
        )
        .await
        .expect("first run (creation) must succeed");
    }
    // Drop this run's own connection (held alive by `backend_slot`'s Arc
    // clone, independent of `execute_project`'s own now-dropped copy)
    // before opening another connection to the SAME on-disk DuckDB file —
    // two live connections at once corrupts the file (observed as a
    // DuckDB-internal SIGSEGV in string-storage scanning).
    *backend_slot.lock().unwrap() = None;

    // Run 2: SAME day, target now exists, but no sidecar baseline exists
    // yet — every user is "changed" against the absent baseline (see
    // `absent_sidecar_first_run_takes_the_widened_scan`), and this run's
    // own write refreshes the sidecar to the CURRENT (pre-rename) content —
    // the baseline run 3 below diffs against.
    {
        let (db, graph) = build_db_and_graph(&project_dir, &config);
        execute_project(
            "run-2".to_string(),
            request_for_day("2025-01-10", "2025-01-11"),
            Arc::clone(&config),
            graph,
            db,
            &project_dir,
            &factory,
            &NoOpReporter,
            CancellationToken::new(),
        )
        .await
        .expect("second run (sidecar baseline) must succeed");
    }
    *backend_slot.lock().unwrap() = None;

    // Rename user 1 — the ONLY declared-projection column that changed.
    {
        let backend = DuckDbBackend::new(&db_path, "main")
            .await
            .expect("reopen duckdb");
        backend
            .execute_sql("UPDATE main.sources_raw_users SET user_name = 'Alicia' WHERE user_id = 1")
            .await
            .expect("rename user 1");
    }

    // Run 3: SAME day again — the sidecar now has a real (pre-rename)
    // baseline, so the diff synthesizes exactly the renamed user's key.
    {
        let (db, graph) = build_db_and_graph(&project_dir, &config);
        execute_project(
            "run-3".to_string(),
            request_for_day("2025-01-10", "2025-01-11"),
            Arc::clone(&config),
            graph,
            db,
            &project_dir,
            &factory,
            &NoOpReporter,
            CancellationToken::new(),
        )
        .await
        .expect("third run (restricted recompute) must succeed");
    }

    let backend = backend_slot
        .lock()
        .unwrap()
        .clone()
        .expect("backend recorded");
    let groups = backend.recorded_groups();
    let restricted = groups
        .iter()
        .rev()
        .find(|g| g.statements.iter().any(|s| s.sql.contains("user_id IN")))
        .unwrap_or_else(|| {
            panic!("no recorded statement group carries the restriction predicate: {groups:#?}")
        });
    assert!(
        restricted
            .statements
            .iter()
            .all(|s| !s.sql.contains("user_id IN") || s.sql.contains("user_id IN ('1')")),
        "the restriction predicate must name exactly the renamed user (1): {restricted:#?}"
    );

    let names = user_names(backend.as_ref()).await;
    assert_eq!(
        names,
        vec![
            (1, "Alicia".to_string()),
            (1, "Alicia".to_string()),
            (2, "Bob".to_string()),
            (2, "Bob".to_string()),
            (3, "Carol".to_string()),
            (3, "Carol".to_string()),
        ],
        "only user 1's 2 rows change; users 2 and 3's rows are untouched"
    );

    // End state equals a from-scratch full refresh of the CURRENT source
    // state — `jitter` excluded from both sides (intentionally non-
    // deterministic, see the module doc comment).
    let oracle = "SELECT e.event_id, CAST(e.event_timestamp AS DATE) AS event_date, e.user_id, \
                  e.event_type, u.user_name FROM main.sources_raw_events e JOIN \
                  main.sources_raw_users u ON e.user_id = u.user_id"
        .to_string();
    let maintained =
        "SELECT event_id, event_date, user_id, event_type, user_name FROM main.probe_model"
            .to_string();
    let left_only = backend
        .execute_sql(&format!(
            "SELECT count(*) FROM (({maintained}) EXCEPT ALL ({oracle})) AS d"
        ))
        .await
        .expect("except all query");
    let right_only = backend
        .execute_sql(&format!(
            "SELECT count(*) FROM (({oracle}) EXCEPT ALL ({maintained})) AS d"
        ))
        .await
        .expect("except all query");
    use arrow::array::Int64Array;
    let count = |batches: &[RecordBatch]| -> i64 {
        batches[0]
            .column(0)
            .as_any()
            .downcast_ref::<Int64Array>()
            .expect("COUNT(*) is BIGINT")
            .value(0)
    };
    assert_eq!(
        (count(&left_only), count(&right_only)),
        (0, 0),
        "the delta-restricted recompute must match a full-refresh oracle exactly"
    );
}

/// No sidecar partition yet on the very first restriction-eligible run over
/// an already-materialized target ⇒ every row "changed" against the absent
/// baseline (`diff_fingerprint_sidecar_changed_keys`'s own documented "First
/// run and `--full-refresh`" posture) ⇒ the recorded statement group's
/// restriction predicate names every user, not a narrower subset — and the
/// sidecar is populated for the next run, so a THIRD run with no further
/// mutation finds a stable, unchanged baseline and its own (empty) diff
/// falls back to the genuinely unrestricted widened scan
/// (`RecomputeRestriction::Unrestricted` — an empty delta is not a
/// restriction licence either).
#[tokio::test]
async fn absent_sidecar_first_run_takes_the_widened_scan() {
    let (_tmp, project_dir, db_path, config) = setup_project();
    let backend_slot: Arc<Mutex<Option<Arc<RecordingBackend>>>> = Arc::new(Mutex::new(None));
    let factory = RecordingBackendFactory {
        db_path: db_path.clone(),
        backend: Arc::clone(&backend_slot),
    };

    {
        let backend = DuckDbBackend::new(&db_path, "main")
            .await
            .expect("open duckdb");
        seed(&backend).await;
    }

    // Run 1: creation.
    {
        let (db, graph) = build_db_and_graph(&project_dir, &config);
        execute_project(
            "run-1".to_string(),
            request_for_day("2025-01-10", "2025-01-11"),
            Arc::clone(&config),
            graph,
            db,
            &project_dir,
            &factory,
            &NoOpReporter,
            CancellationToken::new(),
        )
        .await
        .expect("first run (creation) must succeed");
    }

    // Run 2: SAME day, target exists, but NO source mutation happened and
    // no sidecar partition was ever populated before this run — the first
    // restriction-eligible run always takes the widened scan (every row is
    // "changed" against an absent baseline).
    {
        let (db, graph) = build_db_and_graph(&project_dir, &config);
        execute_project(
            "run-2".to_string(),
            request_for_day("2025-01-10", "2025-01-11"),
            Arc::clone(&config),
            graph,
            db,
            &project_dir,
            &factory,
            &NoOpReporter,
            CancellationToken::new(),
        )
        .await
        .expect("second run must succeed");
    }

    let backend = backend_slot
        .lock()
        .unwrap()
        .clone()
        .expect("backend recorded");
    let groups_after_run_2 = backend.recorded_groups();
    assert!(
        groups_after_run_2
            .iter()
            .any(|g| g.statements.iter().any(|s| s.sql.contains("DELETE"))),
        "run 2 must still execute a DELETE+INSERT recompute: {groups_after_run_2:#?}"
    );
    assert!(
        groups_after_run_2.iter().any(|g| g
            .statements
            .iter()
            .any(|s| s.sql.contains("user_id IN ('1', '2', '3')"))),
        "an absent sidecar baseline makes every user 'changed' — the restriction predicate \
         must name all three, not fall back to a bare unrestricted scan: \
         {groups_after_run_2:#?}"
    );

    // The sidecar is now populated — a THIRD run with no further mutation
    // finds a stable (unchanged) baseline, so its own diff is empty and it
    // too falls back to the widened scan (an empty delta is not a
    // restriction licence either — `RecomputeRestriction::Unrestricted`).
    {
        let (db, graph) = build_db_and_graph(&project_dir, &config);
        execute_project(
            "run-3".to_string(),
            request_for_day("2025-01-10", "2025-01-11"),
            Arc::clone(&config),
            graph,
            db,
            &project_dir,
            &factory,
            &NoOpReporter,
            CancellationToken::new(),
        )
        .await
        .expect("third run must succeed");
    }
    let groups_after_run_3 = backend.recorded_groups();
    let new_groups = &groups_after_run_3[groups_after_run_2.len()..];
    assert!(
        !new_groups
            .iter()
            .any(|g| g.statements.iter().any(|s| s.sql.contains("user_id IN"))),
        "an empty diff against a stable baseline must fall back to the genuinely unrestricted \
         widened scan, never a restriction predicate: {new_groups:#?}"
    );

    let names = user_names(backend.as_ref()).await;
    assert_eq!(
        names,
        vec![
            (1, "Alice".to_string()),
            (1, "Alice".to_string()),
            (2, "Bob".to_string()),
            (2, "Bob".to_string()),
            (3, "Carol".to_string()),
            (3, "Carol".to_string()),
        ],
        "no mutation ever happened — the maintained table stays exactly as first materialized"
    );
}
