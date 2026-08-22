//! T3 real-fixture coverage over the `examples/web_analytics` shape
//! (`docs/plans/20260715-composed-axes-conditional-maintenance.md` Phase
//! E3): `silver.events_deduped` → an event-grain enrichment join against
//! `silver.sessions`, styled after `examples/web_analytics/models/silver/
//! events_enriched.sql`'s real column names (`event_id`, `device_id`,
//! `session_id`) and upstream model names, but with the enrichment join
//! reshaped as a bare LEFT JOIN (payload-only projection, no membership
//! predicate) so P1 skeleton-source closure actually proves `Closed` — the
//! shipped `events_enriched.sql` states its session-cap bound in `WHERE`
//! (a load-bearing Form B propagation clamp for a different, already-tested
//! concern; `models.md`'s Relation Contract doesn't require every consumer
//! shape to also close P1), so this test exercises the SAME production
//! `smelt-logical`/`smelt-runtime` code path — `append_model_edge_cells`,
//! `resolve_recompute_restriction`, `execute_delete_insert_with_delta_
//! restriction` — over a query shape that does.
//!
//! Drives the full pipeline from real model SQL text through a real DuckDB
//! backend: a single event (`ev-42`) is redelivered with a changed
//! `utm_campaign`; `silver.events_deduped` records the changed key via the
//! observed-delta table (T5); this model's own creation-trigger cell
//! restricts its region recompute to exactly that event's row.

use smelt_backend::{Backend, BackendCapabilities, BackendError, PartitionRange, SqlDialect};
use smelt_backend_duckdb::DuckDbBackend;
use smelt_logical::maintenance::derive::{append_model_edge_cells, ModelEdge};
use smelt_logical::maintenance::emit::{MaintenanceDialect, Region};
use smelt_logical::maintenance::{MaintenancePlan, Trigger};
use smelt_runtime::maintenance_driver::execute_delete_insert_with_delta_restriction;
use tempfile::TempDir;

/// A retry policy that never retries — this test exercises the T3
/// delta-restricted DeleteInsert dispatch directly against a real DuckDB
/// backend, outside `execute_project`, so there is no `ExecuteRequest`/run
/// reporter to derive one from (`docs/plans/20260719-prod-w2-operability.md`
/// Phase 6).
const NO_OP_REPORTER: smelt_runtime::NoOpReporter = smelt_runtime::NoOpReporter;
fn no_retry_policy() -> smelt_runtime::RetryPolicy<'static> {
    smelt_runtime::RetryPolicy {
        retry_max: 0,
        base_backoff_ms: 0,
        run_id: "web-analytics-session-delta-restriction-test",
        model_name: "web-analytics-session-delta-restriction-test",
        reporter: &NO_OP_REPORTER,
    }
}

/// Real model SQL shape (event-grain enrichment over a session dimension),
/// column/table names matching `examples/web_analytics/models/silver/
/// events_enriched.sql` (`event_id`, `device_id`, `session_id`, both upstream
/// model addresses) — the only departure is the LEFT JOIN with no
/// membership predicate, so this scope's own P1 closure proves `Closed`.
const EVENTS_ENRICHED_SQL: &str = "SELECT \
     e.event_id, e.device_id, e.event_date, e.utm_campaign AS event_utm_campaign, \
     s.session_id, s.utm_campaign AS session_utm_campaign \
     FROM smelt.silver.events_deduped e \
     LEFT JOIN smelt.silver.sessions s ON e.device_id = s.device_id";

fn model_edges() -> Vec<ModelEdge> {
    vec![
        ModelEdge {
            name: "silver.events_deduped".to_string(),
            clock_col: Some("event_date".to_string()),
            clock_col_aliases: vec![],
            unique_key: vec!["event_id".to_string()],
            output_shape: None,
        },
        ModelEdge {
            name: "silver.sessions".to_string(),
            clock_col: Some("event_date".to_string()),
            clock_col_aliases: vec![],
            unique_key: vec!["device_id".to_string()],
            output_shape: None,
        },
    ]
}

#[tokio::test]
async fn a_single_redelivered_then_changed_event_recomputes_only_its_own_row() {
    // ── Derive the real plan cell over the real model SQL shape ─────────
    let mut plan = MaintenancePlan::default();
    append_model_edge_cells(
        &mut plan,
        EVENTS_ENRICHED_SQL,
        Some("event_date"),
        &model_edges(),
        &[],
    );
    let cell = plan
        .cell_for(&Trigger::NewData {
            source: "silver.events_deduped".to_string(),
        })
        .expect("events_deduped edge cell present");
    assert_eq!(
        cell.skeleton_source_closure.as_ref().map(|c| c.is_closed()),
        Some(true),
        "the LEFT JOIN payload-only enrichment must prove P1 Closed; got {:?}",
        cell.skeleton_source_closure
    );

    // ── Real DuckDB backend: seed events_enriched with 3 baseline rows ──
    let tmp = TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("test.duckdb");
    let backend = DuckDbBackend::new(&db_path, "main")
        .await
        .expect("open duckdb");

    backend
        .execute_sql(
            "CREATE TABLE main.events_enriched (event_id VARCHAR, device_id VARCHAR, \
             event_date DATE, event_utm_campaign VARCHAR, session_id VARCHAR, \
             session_utm_campaign VARCHAR)",
        )
        .await
        .unwrap();
    backend
        .execute_sql(
            "INSERT INTO main.events_enriched VALUES \
             ('ev-1', 'dev-a', '2026-07-01', 'spring_sale', 'sess-A', 'spring_sale'), \
             ('ev-42', 'dev-b', '2026-07-01', 'winter_sale', 'sess-B', 'winter_sale'), \
             ('ev-99', 'dev-c', '2026-07-01', 'organic', 'sess-C', 'organic')",
        )
        .await
        .unwrap();

    // The recompute source: `ev-42`'s redelivery changed its
    // `utm_campaign` from 'winter_sale' to 'flash_sale' — every other
    // event's recomputed value is byte-identical to its stored row.
    backend
        .execute_sql(
            "CREATE TABLE main.events_enriched_recompute (event_id VARCHAR, device_id VARCHAR, \
             event_date DATE, event_utm_campaign VARCHAR, session_id VARCHAR, \
             session_utm_campaign VARCHAR)",
        )
        .await
        .unwrap();
    backend
        .execute_sql(
            "INSERT INTO main.events_enriched_recompute VALUES \
             ('ev-1', 'dev-a', '2026-07-01', 'spring_sale', 'sess-A', 'spring_sale'), \
             ('ev-42', 'dev-b', '2026-07-01', 'flash_sale', 'sess-B', 'winter_sale'), \
             ('ev-99', 'dev-c', '2026-07-01', 'organic', 'sess-C', 'organic')",
        )
        .await
        .unwrap();

    // events_deduped's own conditional write (C4) recorded the observed
    // delta: exactly `ev-42` changed in this window (T5, Group D).
    let ensure_sql = smelt_state::ddl_duckdb::generate_observed_delta_table_ddl("main");
    backend.execute_sql(&ensure_sql).await.unwrap();
    let upsert_sql = smelt_state::ddl_duckdb::generate_observed_delta_upsert_sql(
        "main",
        "silver.events_deduped",
        "2026-07-01",
        "2026-07-02",
        "SELECT * FROM (VALUES ('ev-42', NULL)) AS t(delta_key, delta_partition)",
    );
    backend.execute_sql(&upsert_sql).await.unwrap();

    // ── Execute the real creation-trigger cell's recompute ──────────────
    let region = Region {
        start: "'2026-07-01'".to_string(),
        end: "'2026-07-02'".to_string(),
    };
    let body = "SELECT event_id, device_id, event_date, event_utm_campaign, session_id, \
                session_utm_campaign FROM main.events_enriched_recompute";
    let group = execute_delete_insert_with_delta_restriction(
        &backend,
        "main",
        "events_enriched",
        "event_date",
        &region,
        body,
        body,
        Some("event_id"),
        cell.skeleton_source_closure.as_ref(),
        "silver.events_deduped",
        "2026-07-01",
        "2026-07-02",
        MaintenanceDialect::DuckDb,
        &no_retry_policy(),
        &smelt_runtime::probes::ProbePolicy::per_run(),
    )
    .await
    .expect("delta-restricted recompute executes");

    assert!(group.statements[0].sql.contains("event_id IN ('ev-42')"));
    assert!(group.statements[1].sql.contains("event_id IN ('ev-42')"));

    let batches = backend
        .execute_sql(
            "SELECT event_id, event_utm_campaign FROM main.events_enriched ORDER BY event_id",
        )
        .await
        .unwrap();
    let mut rows = Vec::new();
    for batch in &batches {
        use arrow::array::{Array, StringArray};
        let ids = batch
            .column(0)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        let campaigns = batch
            .column(1)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        for i in 0..batch.num_rows() {
            rows.push((ids.value(i).to_string(), campaigns.value(i).to_string()));
        }
    }
    assert_eq!(
        rows,
        vec![
            ("ev-1".to_string(), "spring_sale".to_string()),
            ("ev-42".to_string(), "flash_sale".to_string()),
            ("ev-99".to_string(), "organic".to_string()),
        ],
        "only ev-42's row (the redelivered-then-changed event) must recompute; ev-1/ev-99 must \
         be byte-identical to their stored rows"
    );
}

// =============================================================================
// Live-run wiring: the same restriction machinery driven through
// `execute_project` itself (`docs/plans/20260715-composed-axes-conditional-
// maintenance.md` Phase E3's central deliverable — the live per-batch loop
// in `smelt_runtime::execute` must actually dispatch through
// `resolve_live_delta_restriction_facts`/`execute_delete_insert_with_delta_
// restriction`, not just prove the helper works when called directly, which
// every other T3 test — including the one above — did until this one).
//
// **Why this fixture's shape differs from the direct-call test above.**
// Proving this through a REAL, validated model hits two structural limits
// neither of which this phase's own scope licenses touching:
//   - A model whose creation cell contributes model-edge cells at all must
//     resolve `grain: partition` (`append_model_edge_cells`'s
//     `output_partition_col` gate) — and `models.md`'s own `validate_
//     timeseries` rejects declaring a top-level `unique_key:` alongside a
//     clock, since together they'd derive `grain: key`/`key_per_partition`,
//     never `partition` (`GrainAssertionMismatch`). So a model-edge cell's
//     row identity can ONLY ever come from the walk's OWN proof
//     (`analysis::walk::model_property_vector`'s grain), never a declared
//     fact — by construction, for every partition-grain model, not just
//     this one.
//   - The walk only proves a grain key via `GROUP BY` or `SELECT DISTINCT`
//     over the full output — but `skeleton_source_closure`'s v1 scope
//     restriction refuses (`Open`) any scope with a `GROUP BY` above the
//     enrichment join outright ("join-below-aggregation changes the
//     skeleton question"). `SELECT DISTINCT` survives that restriction, but
//     establishes the FULL output column list as the key — and `timeseries.
//     partition_column` must itself be a selected output column
//     (`MalformedTimeseries` otherwise), so any additional payload column
//     makes the proven key composite, never the single column `emit_
//     delete_insert_delta_restricted`'s `restrict_column` needs.
//
// The one shape that satisfies both simultaneously is `SELECT DISTINCT
// <partition_column>` with no other output column — a degenerate
// "which partition values exist" projection. `events_enriched` here uses
// exactly that (over a LEFT JOIN to `sessions`, still a genuine
// model-edge-sourced, P1-closed enrichment join), and the recorded delta
// names a changed **date value** rather than an entity id — an unusual but
// mechanically valid instance of the same restriction machinery (the
// emitter has no opinion on what the key values mean). Because there is no
// payload column left to observe a "did this row's value change" effect on,
// this test proves restriction through the **executed statement text**
// (captured via a recording backend, the same technique `statement_parity.
// rs`/`delta_restricted_recompute.rs` already use), not row-content
// equality — reverting `execute.rs`'s wiring to call `emit_delete_insert`
// unconditionally would drop the `... AND event_date IN (...)` predicate
// this test asserts on.

use smelt_core::config::{Config, Target};
use smelt_core::graph::DependencyGraph;
use smelt_core::ModelDiscovery;
use smelt_runtime::execute::{execute_project, BackendFactory, BackendFuture};
use smelt_runtime::types::ExecuteRequest;
use std::path::Path;
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;

/// Wraps a real [`DuckDbBackend`], delegating every call, but recording the
/// [`smelt_backend::StatementGroup`] passed to `execute_statement_group` —
/// the single point every emitted maintenance statement flows through
/// (`docs/specs/incremental_models.md` §"Statement emission (single
/// owner)"). Mirrors `statement_parity.rs`'s own `RecordingBackend`
/// (duplicated here rather than shared, matching this crate's existing
/// per-test-binary convention for these harnesses).
struct RecordingBackend {
    inner: DuckDbBackend,
    groups: Mutex<Vec<smelt_backend::StatementGroup>>,
}

impl RecordingBackend {
    fn new(inner: DuckDbBackend) -> Self {
        Self {
            inner,
            groups: Mutex::new(Vec::new()),
        }
    }

    fn recorded_groups(&self) -> Vec<smelt_backend::StatementGroup> {
        self.groups.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl Backend for RecordingBackend {
    async fn execute_sql(&self, sql: &str) -> Result<Vec<arrow::array::RecordBatch>, BackendError> {
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
    ) -> Result<Vec<arrow::array::RecordBatch>, BackendError> {
        self.inner.get_preview(schema, name, limit).await
    }
    async fn table_exists(&self, schema: &str, name: &str) -> Result<bool, BackendError> {
        self.inner.table_exists(schema, name).await
    }
    async fn ensure_schema(&self, schema: &str) -> Result<(), BackendError> {
        self.inner.ensure_schema(schema).await
    }
    fn dialect(&self) -> SqlDialect {
        self.inner.dialect()
    }
    fn capabilities(&self) -> BackendCapabilities {
        self.inner.capabilities()
    }
    async fn load_table(
        &self,
        schema: &str,
        name: &str,
        arrow_schema: arrow::datatypes::SchemaRef,
        batches: Vec<arrow::array::RecordBatch>,
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
    async fn execute_statement_group(
        &self,
        group: &smelt_backend::StatementGroup,
    ) -> Result<(), BackendError> {
        self.groups.lock().unwrap().push(group.clone());
        self.inner.execute_statement_group(group).await
    }
}

/// Thin `Backend` forwarder over an `Arc<RecordingBackend>` — see
/// `statement_parity.rs`'s identical precedent for why this exists (the
/// factory needs to hand `execute_project` ownership of a `Box<dyn
/// Backend>` while the test keeps its own handle to read the recording
/// back).
struct ArcBackend(Arc<RecordingBackend>);

#[async_trait::async_trait]
impl Backend for ArcBackend {
    async fn execute_sql(&self, sql: &str) -> Result<Vec<arrow::array::RecordBatch>, BackendError> {
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
    ) -> Result<Vec<arrow::array::RecordBatch>, BackendError> {
        self.0.get_preview(schema, name, limit).await
    }
    async fn table_exists(&self, schema: &str, name: &str) -> Result<bool, BackendError> {
        self.0.table_exists(schema, name).await
    }
    async fn ensure_schema(&self, schema: &str) -> Result<(), BackendError> {
        self.0.ensure_schema(schema).await
    }
    fn dialect(&self) -> SqlDialect {
        self.0.dialect()
    }
    fn capabilities(&self) -> BackendCapabilities {
        self.0.capabilities()
    }
    async fn load_table(
        &self,
        schema: &str,
        name: &str,
        arrow_schema: arrow::datatypes::SchemaRef,
        batches: Vec<arrow::array::RecordBatch>,
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
    async fn execute_statement_group(
        &self,
        group: &smelt_backend::StatementGroup,
    ) -> Result<(), BackendError> {
        self.0.execute_statement_group(group).await
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

fn write_live_model(project_dir: &Path, relative_name: &str, content: &str) {
    let path = project_dir
        .join("models")
        .join(format!("{relative_name}.sql"));
    std::fs::create_dir_all(path.parent().expect("model path has a parent")).unwrap();
    std::fs::write(path, content).expect("write model file");
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
    db.set_active_target(config.target.clone().map(|t| Arc::from(t.as_str())));

    let graph = DependencyGraph::build(sql_models, None).expect("build graph");

    (
        Arc::new(tokio::sync::Mutex::new(db)),
        Arc::new(tokio::sync::Mutex::new(graph)),
    )
}

fn live_request(start: &str, end: &str) -> ExecuteRequest {
    ExecuteRequest {
        target: "dev".to_string(),
        select: vec![],
        exclude: vec![],
        start: Some(start.to_string()),
        end: Some(end.to_string()),
        batch_size_days: None,
        per_partition: false,
        full_refresh: false,
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

const LIVE_EVENTS_DEDUPED: &str = "---\n\
     materialization: table\n\
     refresh: incremental\n\
     grain: partition\n\
     timeseries:\n\
     \x20\x20partition_column: event_date\n\
     \x20\x20event_time_column: event_date\n\
     \x20\x20granularity: day\n\
     ---\n\
     SELECT * FROM (VALUES \n\
     \x20\x20(CAST('ev-1' AS VARCHAR), CAST('dev-a' AS VARCHAR), DATE '2026-07-01'),\n\
     \x20\x20(CAST('ev-42' AS VARCHAR), CAST('dev-b' AS VARCHAR), DATE '2026-07-02'),\n\
     \x20\x20(CAST('ev-99' AS VARCHAR), CAST('dev-c' AS VARCHAR), DATE '2026-07-03')\n\
     ) AS t(event_id, device_id, event_date)";

/// `grain: key` (a plain, real cumulative aggregate — the SAME `GROUP BY` +
/// combiner shape `examples/web_analytics/models/silver/
/// device_user_edges.sql` already ships, reading from the SAME clocked
/// driving source `events_deduped` a `refresh: keyed` snapshot-reconcile
/// posture requires) declaring a top-level `unique_key` with NO
/// `timeseries:` block of its own, so it carries no clock but DOES
/// contribute a declared `unique_key` fact `ModelEdge` picks up — the fact
/// the enrichment join's one-to-one conjunct (P1) and the row-identity
/// fan-out check (P2) both need.
const LIVE_SESSIONS: &str = "---\n\
     materialization: table\n\
     refresh: incremental\n\
     grain: key\n\
     unique_key: [device_id]\n\
     ---\n\
     SELECT device_id, MAX(UPPER(device_id)) AS session_id\n\
     FROM smelt.silver.events_deduped\n\
     GROUP BY device_id";

/// `SELECT DISTINCT event_date` — see this section's own doc comment for
/// why this degenerate single-column projection is the one shape that
/// simultaneously closes P1 (a LEFT JOIN, no `GROUP BY`, no membership
/// predicate) and lets the walk prove a single-column P2 row identity.
const LIVE_EVENTS_ENRICHED: &str = "---\n\
     materialization: table\n\
     refresh: incremental\n\
     grain: partition\n\
     timeseries:\n\
     \x20\x20partition_column: event_date\n\
     \x20\x20event_time_column: event_date\n\
     \x20\x20granularity: day\n\
     ---\n\
     SELECT DISTINCT e.event_date\n\
     FROM smelt.silver.events_deduped e\n\
     LEFT JOIN smelt.silver.sessions s ON e.device_id = s.device_id";

/// The flagship live-path proof: a real two-run `execute_project` sequence
/// over the SAME window, with `silver.events_deduped`'s own observed delta
/// pre-seeded between the runs (standing in for T5's own recording, a
/// different track's scope — Phase E3's own pre-conditions name "D2–D3 (the
/// exact delta and its projection exist)" as a precondition, not something
/// this phase re-derives).
#[tokio::test]
async fn a_real_execute_project_run_dispatches_through_the_delta_restricted_emitter() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project_dir = tmp.path();

    write_live_model(project_dir, "silver/events_deduped", LIVE_EVENTS_DEDUPED);
    write_live_model(project_dir, "silver/sessions", LIVE_SESSIONS);
    write_live_model(project_dir, "silver/events_enriched", LIVE_EVENTS_ENRICHED);

    let db_path = project_dir.join("run.duckdb");
    let smelt_yml = format!(
        "name: web_analytics_live_wiring_test\nversion: 1\npaths:\n  - models\ntargets:\n  \
         dev:\n    type: duckdb\n    database: {db}\n    schema: main\n\
         default_materialization: table\ntarget: dev\n",
        db = db_path.display()
    );
    std::fs::write(project_dir.join("smelt.yml"), &smelt_yml).unwrap();
    let config = Arc::new(Config::load(project_dir).expect("load config"));

    // ── Run 1: bootstrap all 3 tables over the whole 3-day window (a
    // single batch — the model carries no lookback construct, so batch
    // safety folds the full requested range into one chunk). ───────────
    {
        let (db, graph) = build_db_and_graph(project_dir, &config);
        let backend_slot: Arc<Mutex<Option<Arc<RecordingBackend>>>> = Arc::new(Mutex::new(None));
        let factory = RecordingBackendFactory {
            db_path: db_path.clone(),
            backend: Arc::clone(&backend_slot),
        };
        execute_project(
            "live-wiring-run-1".to_string(),
            live_request("2026-07-01", "2026-07-04"),
            Arc::clone(&config),
            graph,
            db,
            project_dir,
            &factory,
            &smelt_runtime::NoOpReporter,
            CancellationToken::new(),
        )
        .await
        .expect("execute_project run 1 (bootstrap)");
    }

    // ── Between runs: pre-seed the observed-delta table exactly as T5
    // would have (a different track's scope — this phase consumes an
    // already-recorded delta per its own pre-conditions), over the SAME
    // window a single-batch run 2 will read back with: `2026-07-02` is the
    // only changed partition value. ────────────────────────────────────
    {
        let seed_backend = DuckDbBackend::new(&db_path, "main")
            .await
            .expect("open duckdb to seed the observed-delta table");
        let ensure_sql = smelt_state::ddl_duckdb::generate_observed_delta_table_ddl("main");
        seed_backend.execute_sql(&ensure_sql).await.unwrap();
        let upsert_sql = smelt_state::ddl_duckdb::generate_observed_delta_upsert_sql(
            "main",
            "silver.events_deduped",
            "2026-07-01",
            "2026-07-04",
            "SELECT * FROM (VALUES ('2026-07-02', NULL)) AS t(delta_key, delta_partition)",
        );
        seed_backend.execute_sql(&upsert_sql).await.unwrap();
    }

    // ── Run 2: the real live wiring under test. `events_enriched`'s
    // creation cell is model-edge-sourced and P1-closed; if `execute.rs`'s
    // live per-batch loop actually dispatches through
    // `resolve_live_delta_restriction_facts`/`execute_delete_insert_with_
    // delta_restriction` (rather than calling `emit_delete_insert`
    // unconditionally), its executed DELETE/INSERT must carry the
    // `event_date IN ('2026-07-02')` semi-join predicate. ───────────────
    let (db, graph) = build_db_and_graph(project_dir, &config);
    let backend_slot: Arc<Mutex<Option<Arc<RecordingBackend>>>> = Arc::new(Mutex::new(None));
    let factory = RecordingBackendFactory {
        db_path: db_path.clone(),
        backend: Arc::clone(&backend_slot),
    };
    let outcome = execute_project(
        "live-wiring-run-2".to_string(),
        live_request("2026-07-01", "2026-07-04"),
        Arc::clone(&config),
        graph,
        db,
        project_dir,
        &factory,
        &smelt_runtime::NoOpReporter,
        CancellationToken::new(),
    )
    .await
    .expect("execute_project run 2 (live delta-restricted recompute)");
    assert!(outcome.models.contains_key("silver.events_enriched"));

    let backend = backend_slot
        .lock()
        .unwrap()
        .clone()
        .expect("backend recorded");
    let groups = backend.recorded_groups();
    let enriched_groups: Vec<_> = groups
        .iter()
        .filter(|g| {
            g.statements[0]
                .sql
                .starts_with("DELETE FROM main.silver_events_enriched")
        })
        .collect();
    assert_eq!(
        enriched_groups.len(),
        1,
        "exactly one events_enriched DELETE+INSERT group this run: {groups:?}"
    );
    let group = enriched_groups[0];
    let delete_sql = &group.statements[0].sql;
    let insert_sql = &group.statements[1].sql;

    assert!(
        delete_sql.contains("event_date IN ('2026-07-02')"),
        "the live run must restrict the DELETE to the recorded delta's changed partition value \
         — this predicate is only present when execute.rs actually dispatches through \
         execute_delete_insert_with_delta_restriction rather than calling emit_delete_insert \
         unconditionally; got: {delete_sql}"
    );
    assert!(
        insert_sql.contains("_smelt_delta_scope.event_date IN ('2026-07-02')"),
        "the live run's INSERT must carry the matching semi-join restriction; got: {insert_sql}"
    );
    assert!(
        !delete_sql.contains("event_date IN ('2026-07-01'")
            && !delete_sql.contains("event_date IN ('2026-07-03'"),
        "the semi-join restriction predicate must name ONLY the recorded delta's key \
         ('2026-07-02'), not the other (unchanged) partition values in this batch's region \
         (the region clamp's own '2026-07-01'/'2026-07-04' literals are expected and \
         unrelated); got: {delete_sql}"
    );

    // Byte-for-byte parity with a direct call of the same emitter over the
    // batch's own region + body, reconstructed from the executed DELETE's
    // own region literals (the same reconstruction technique `statement_
    // parity.rs::region_recompute_statements_come_from_the_emitter` uses) —
    // proves the live-executed statement is exactly the emitter's output,
    // not merely emitter-shaped text.
    let where_clause = delete_sql
        .strip_prefix("DELETE FROM main.silver_events_enriched WHERE ")
        .expect("delete shape");
    let parts: Vec<&str> = where_clause.split(" AND ").collect();
    let start_lit = parts[0]
        .strip_prefix("event_date >= ")
        .expect("start literal");
    let end_lit = parts[1].strip_prefix("event_date < ").expect("end literal");
    let body = insert_sql
        .strip_prefix("INSERT INTO main.silver_events_enriched SELECT * FROM (")
        .and_then(|s| {
            s.strip_suffix(
                ") AS _smelt_delta_scope WHERE _smelt_delta_scope.event_date IN ('2026-07-02')",
            )
        })
        .expect("insert shape");
    let region = smelt_logical::maintenance::emit::Region {
        start: start_lit.to_string(),
        end: end_lit.to_string(),
    };
    let expected = smelt_logical::maintenance::emit::emit_delete_insert_delta_restricted(
        "main.silver_events_enriched",
        "event_date",
        &region,
        body,
        "event_date",
        &["2026-07-02".to_string()],
        smelt_logical::maintenance::emit::MaintenanceDialect::DuckDb,
    );
    assert_eq!(
        &expected, group,
        "the executed group must be byte-identical to a direct call of \
         emit_delete_insert_delta_restricted over the same inputs"
    );
}
