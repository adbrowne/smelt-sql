//! Real-fixture, DuckDB-backed coverage for key temporal locality's
//! **route 3** (recurrence-bounded, declared `r`) transactional runtime
//! check (`docs/specs/incremental_shapes.md` §"Key temporal locality (the
//! time-partitioned output)"; `docs/plans/20260715-composed-axes-
//! conditional-maintenance.md` Phase A4).
//!
//! Route 3's flagship shape (an extremal-fold `MAX(event_date)` partition
//! column, `unique_key: [event_id]`) hits the same pre-existing blocker
//! `docs/specs/incremental_models.md` §Known Divergences documents for
//! route 2's own real-fixture coverage: every extremal aggregate
//! (`MIN`/`MAX`) is inferred nullable unconditionally by the type-inference
//! registry regardless of its argument's own nullability, which trips the
//! unrelated NOT-NULL diagnostic `execute_project`'s pre-execution
//! diagnostic gate (`gate_diagnostics`) enforces — independent of locality
//! admission. This file therefore drives the windowed-keyed-maintenance
//! driver (`smelt_runtime::maintenance_driver::run_windowed_keyed_
//! maintenance`) directly against a **real** `DuckDbBackend`, with a
//! manually-constructed `CumulativeClassification` (the same classification
//! shape `classify_cumulative` would produce for the flagship SQL), rather
//! than through the full `execute_project` pipeline — this still exercises
//! the real emitted SQL (`smelt_logical::maintenance::emit::
//! emit_recurrence_bound_probe` + `emit_keyed_fold`) against a real
//! database, proving the actual runtime check, not just its unit-level
//! SQL-shape assertions (`crates/smelt-logical/tests/emit_statements.rs`)
//! or its pure-gate admission assertions
//! (`crates/smelt-logical/src/maintenance/locality.rs`'s own tests).

use std::path::Path;

use smelt_backend::Backend;
use smelt_backend_duckdb::DuckDbBackend;
use smelt_core::config::{Granularity, TimeseriesConfig};
use smelt_logical::maintenance::choice::WriteSuppression;
use smelt_logical::maintenance::locality::LocalitySlice;
use smelt_planner::{
    AggregatorColumn, CrossPartitionCombiner, CumulativeClassification, DrivingSource,
};
use smelt_runtime::maintenance_driver::{driving_steps, run_windowed_keyed_maintenance};

/// A retry policy that never retries — these tests exercise the
/// windowed-keyed-maintenance driver directly against a real DuckDB
/// backend, outside `execute_project`, so there is no `ExecuteRequest`/run
/// reporter to derive one from (`docs/plans/20260719-prod-w2-operability.md`
/// Phase 6).
const NO_OP_REPORTER: smelt_runtime::NoOpReporter = smelt_runtime::NoOpReporter;
fn no_retry_policy() -> smelt_runtime::RetryPolicy<'static> {
    smelt_runtime::RetryPolicy {
        retry_max: 0,
        base_backoff_ms: 0,
        run_id: "locality-route3-recurrence-check-test",
        model_name: "locality-route3-recurrence-check-test",
        reporter: &NO_OP_REPORTER,
    }
}

/// This file exercises route-3 checked-merge behaviour, not suppression —
/// every call site below passes the plain unconditional matched arm.
fn unconditional() -> WriteSuppression {
    WriteSuppression::Unconditional {
        why: "test exercises route-3 checked-merge behaviour, not suppression".to_string(),
    }
}

fn timeseries() -> TimeseriesConfig {
    TimeseriesConfig {
        event_time_column: "event_ts".to_string(),
        partition_column: "event_date".to_string(),
        granularity: Granularity::Day,
        week_start: None,
        assert_monotonic: false,
    }
}

/// The `events_last_seen` classification: `unique_key: [event_id]`,
/// `last_seen_date = MAX(event_date)` — an extremal-fold combiner, the
/// shape route 3 exists for (route 1/2 both fail on it: not a key column,
/// and not once-write-provable).
fn classification() -> CumulativeClassification {
    CumulativeClassification {
        unique_key: vec!["event_id".to_string()],
        aggregator_columns: vec![AggregatorColumn {
            output_name: "last_seen_date".to_string(),
            per_partition_agg: "MAX".to_string(),
            cross_partition_combiner: CrossPartitionCombiner::Max,
            state: None,
        }],
        driving_source: DrivingSource {
            name: "smelt.sources.raw.events".to_string(),
            timeseries: Some(timeseries()),
        },
    }
}

/// A declared, checked route-3 slice: `r = 3 days`, no additional margin.
fn checked_slice() -> LocalitySlice {
    LocalitySlice::RecurrenceBounded {
        partition_column: "last_seen_date".to_string(),
        margin_before: smelt_logical::analysis::source_bounds::Seconds::days(3),
        margin_after: smelt_logical::analysis::source_bounds::Seconds::ZERO,
        r: smelt_logical::analysis::source_bounds::Seconds::days(3),
    }
}

/// A **derived** (unchecked) route-3 slice — structurally identical to
/// route 1's `Window`, since a statically-derived `r` is proof-backed and
/// never triggers the probe.
fn derived_slice() -> LocalitySlice {
    LocalitySlice::Window {
        partition_column: "last_seen_date".to_string(),
        margin_before: smelt_logical::analysis::source_bounds::Seconds::days(3),
        margin_after: smelt_logical::analysis::source_bounds::Seconds::ZERO,
        recurrence_bounded: true,
    }
}

async fn setup_backend(db_path: &Path) -> DuckDbBackend {
    let conn = duckdb::Connection::open(db_path).expect("open duckdb");
    conn.execute_batch(
        r#"
        CREATE SCHEMA IF NOT EXISTS main;
        CREATE TABLE main.raw_events (
            event_id INTEGER,
            event_ts TIMESTAMP,
            event_date DATE
        );
        "#,
    )
    .expect("create raw_events");
    drop(conn);
    DuckDbBackend::new(db_path, "main")
        .await
        .expect("open backend")
}

/// Inserts through the same live `Backend` connection the driver itself
/// uses — a second, independently-opened `duckdb::Connection` to the same
/// file would not share the backend's in-process connection state.
async fn insert_event(backend: &DuckDbBackend, event_id: i64, date: &str) {
    backend
        .execute_sql(&format!(
            "INSERT INTO main.raw_events VALUES ({event_id}, TIMESTAMP '{date} 00:00:00', DATE '{date}')"
        ))
        .await
        .expect("insert event");
}

/// One step's delta: `event_id, MAX(event_date) AS last_seen_date` for
/// rows landing on that step's own date — matching `classification()`'s
/// aggregator shape exactly (`unique_key` + one fold column, nothing else).
fn compile_step(
    step: &smelt_runtime::maintenance_driver::MaintenanceStep,
) -> anyhow::Result<String> {
    Ok(format!(
        "SELECT event_id, MAX(event_date) AS last_seen_date FROM main.raw_events \
         WHERE event_date = '{}' GROUP BY event_id",
        step.partition_value
    ))
}

async fn last_seen_date_for(backend: &DuckDbBackend, event_id: i64) -> Option<String> {
    let batches = backend
        .execute_sql(&format!(
            "SELECT CAST(last_seen_date AS VARCHAR) AS v FROM main.events_last_seen \
             WHERE event_id = {event_id}"
        ))
        .await
        .expect("query last_seen_date");
    let rows = smelt_runtime::check_runner::batches_to_rows(&batches);
    rows.first().and_then(|r| r.get("v")).cloned()
}

/// An in-bound redelivery (within the declared `r`) merges cleanly: the
/// checked probe finds no violation (the stored row lies within the
/// slice), and the merge updates the key's `last_seen_date` normally.
#[tokio::test]
async fn checked_route3_in_bound_redelivery_merges_cleanly() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("dev.duckdb");
    let backend = setup_backend(&db_path).await;

    // Day 1: event_id 1 first seen.
    insert_event(&backend, 1, "2026-01-01").await;
    // Day 2: event_id 1 redelivered — 1 day after day 1, well within r=3 days.
    insert_event(&backend, 1, "2026-01-02").await;

    let steps = driving_steps("2026-01-01", "2026-01-03", &Granularity::Day).expect("steps");
    let classification = classification();
    let slice = checked_slice();
    run_windowed_keyed_maintenance(
        &backend,
        "events_last_seen",
        "main",
        "events_last_seen",
        &steps,
        &classification,
        Some(&slice),
        &unconditional(),
        None,
        compile_step,
        &no_retry_policy(),
        &smelt_runtime::probes::ProbePolicy::per_run(),
    )
    .await
    .expect("in-bound redelivery must merge cleanly, not refuse");

    assert_eq!(
        last_seen_date_for(&backend, 1).await.as_deref(),
        Some("2026-01-02"),
        "the merge must still apply — last_seen_date updates to the later date"
    );
}

/// An out-of-bound redelivery (further apart than the declared `r`) trips
/// the check: the run refuses with `KeyedRecurrenceBoundViolated`, naming
/// the violation count and sample key, and the target table is unchanged
/// (still holds day 1's original value) — the probe runs before the merge,
/// so a violation never reaches the write path.
#[tokio::test]
async fn checked_route3_out_of_bound_redelivery_rolls_back_with_violation() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("dev.duckdb");
    let backend = setup_backend(&db_path).await;

    // Day 1: event_id 1 first seen — creates the target table.
    insert_event(&backend, 1, "2026-01-01").await;
    let create_steps = driving_steps("2026-01-01", "2026-01-02", &Granularity::Day).expect("steps");
    run_windowed_keyed_maintenance(
        &backend,
        "events_last_seen",
        "main",
        "events_last_seen",
        &create_steps,
        &classification(),
        Some(&checked_slice()),
        &unconditional(),
        None,
        compile_step,
        &no_retry_policy(),
        &smelt_runtime::probes::ProbePolicy::per_run(),
    )
    .await
    .expect("day 1 create must succeed");
    assert_eq!(
        last_seen_date_for(&backend, 1).await.as_deref(),
        Some("2026-01-01")
    );

    // Day 6: event_id 1 redelivered — 5 days after day 1, further apart
    // than the declared r=3 days. The checked probe's slice lower bound
    // for this step is 2026-01-06 - 3 days = 2026-01-03; the stored row
    // (2026-01-01) lies before it — a violation.
    insert_event(&backend, 1, "2026-01-06").await;
    let violating_steps =
        driving_steps("2026-01-06", "2026-01-07", &Granularity::Day).expect("steps");
    let err = run_windowed_keyed_maintenance(
        &backend,
        "events_last_seen",
        "main",
        "events_last_seen",
        &violating_steps,
        &classification(),
        Some(&checked_slice()),
        &unconditional(),
        None,
        compile_step,
        &no_retry_policy(),
        &smelt_runtime::probes::ProbePolicy::per_run(),
    )
    .await
    .expect_err("an out-of-bound redelivery must refuse the run");

    let message = err.to_string();
    assert!(
        message.contains("KeyedRecurrenceBoundViolated"),
        "message must carry the diagnostic code: {message}"
    );
    assert!(
        message.contains('1'),
        "message must report the violation count: {message}"
    );

    // Target unchanged after the refusal: the probe ran read-only, before
    // any write — day 1's original value must still be exactly what is
    // stored (the merge never ran).
    assert_eq!(
        last_seen_date_for(&backend, 1).await.as_deref(),
        Some("2026-01-01"),
        "the target must be unchanged after the checked probe refuses the run"
    );
}

/// A thin recording wrapper over a real `DuckDbBackend`, capturing every
/// SQL statement handed to `execute_sql` — used only to prove a **derived**
/// route-3 slice never emits the out-of-slice match probe (no statement
/// containing the probe's own `__recurrence_violations` marker runs).
struct RecordingBackend {
    inner: DuckDbBackend,
    executed_sql: std::sync::Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl Backend for RecordingBackend {
    async fn execute_sql(
        &self,
        sql: &str,
    ) -> Result<Vec<arrow::array::RecordBatch>, smelt_backend::BackendError> {
        self.executed_sql.lock().unwrap().push(sql.to_string());
        self.inner.execute_sql(sql).await
    }
    async fn create_table_as(
        &self,
        schema: &str,
        name: &str,
        sql: &str,
    ) -> Result<(), smelt_backend::BackendError> {
        self.inner.create_table_as(schema, name, sql).await
    }
    async fn create_view_as(
        &self,
        schema: &str,
        name: &str,
        sql: &str,
    ) -> Result<(), smelt_backend::BackendError> {
        self.inner.create_view_as(schema, name, sql).await
    }
    async fn drop_table_if_exists(
        &self,
        schema: &str,
        name: &str,
    ) -> Result<(), smelt_backend::BackendError> {
        self.inner.drop_table_if_exists(schema, name).await
    }
    async fn drop_view_if_exists(
        &self,
        schema: &str,
        name: &str,
    ) -> Result<(), smelt_backend::BackendError> {
        self.inner.drop_view_if_exists(schema, name).await
    }
    async fn get_row_count(
        &self,
        schema: &str,
        name: &str,
    ) -> Result<usize, smelt_backend::BackendError> {
        self.inner.get_row_count(schema, name).await
    }
    async fn get_preview(
        &self,
        schema: &str,
        name: &str,
        limit: usize,
    ) -> Result<Vec<arrow::array::RecordBatch>, smelt_backend::BackendError> {
        self.inner.get_preview(schema, name, limit).await
    }
    async fn table_exists(
        &self,
        schema: &str,
        name: &str,
    ) -> Result<bool, smelt_backend::BackendError> {
        self.inner.table_exists(schema, name).await
    }
    async fn ensure_schema(&self, schema: &str) -> Result<(), smelt_backend::BackendError> {
        self.inner.ensure_schema(schema).await
    }
    fn dialect(&self) -> smelt_backend::SqlDialect {
        self.inner.dialect()
    }
    fn capabilities(&self) -> smelt_backend::BackendCapabilities {
        self.inner.capabilities()
    }
    async fn load_table(
        &self,
        schema: &str,
        name: &str,
        arrow_schema: arrow::datatypes::SchemaRef,
        batches: Vec<arrow::array::RecordBatch>,
    ) -> Result<(), smelt_backend::BackendError> {
        self.inner
            .load_table(schema, name, arrow_schema, batches)
            .await
    }
    async fn delete_partitions(
        &self,
        schema: &str,
        name: &str,
        partition: &smelt_backend::PartitionRange,
    ) -> Result<(), smelt_backend::BackendError> {
        self.inner.delete_partitions(schema, name, partition).await
    }
    async fn insert_into_from_query(
        &self,
        schema: &str,
        name: &str,
        sql: &str,
    ) -> Result<(), smelt_backend::BackendError> {
        self.inner.insert_into_from_query(schema, name, sql).await
    }
    async fn insert_overwrite(
        &self,
        schema: &str,
        table: &str,
        sql: &str,
        partition: &smelt_backend::PartitionRange,
    ) -> Result<(), smelt_backend::BackendError> {
        self.inner
            .insert_overwrite(schema, table, sql, partition)
            .await
    }
}

/// A **derived** (statically-proven) `r` never emits the check: it renders
/// as an ordinary `Window` slice (structurally identical to route 1's), so
/// an in-bound redelivery merges cleanly with **no** probe statement ever
/// executed — proving `LocalitySlice::Window` never triggers the
/// out-of-slice match probe (only `LocalitySlice::RecurrenceBounded` does).
#[tokio::test]
async fn derived_route3_bound_never_emits_the_check() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("dev.duckdb");
    let inner = setup_backend(&db_path).await;
    let backend = RecordingBackend {
        inner,
        executed_sql: std::sync::Mutex::new(Vec::new()),
    };

    // Day 1 (create), then day 2's in-bound redelivery (merge) — both under
    // the derived slice.
    backend
        .execute_sql(
            "INSERT INTO main.raw_events VALUES (1, TIMESTAMP '2026-01-01 00:00:00', DATE \
             '2026-01-01')",
        )
        .await
        .expect("insert day 1");
    let create_steps = driving_steps("2026-01-01", "2026-01-02", &Granularity::Day).expect("steps");
    run_windowed_keyed_maintenance(
        &backend,
        "events_last_seen",
        "main",
        "events_last_seen",
        &create_steps,
        &classification(),
        Some(&derived_slice()),
        &unconditional(),
        None,
        compile_step,
        &no_retry_policy(),
        &smelt_runtime::probes::ProbePolicy::per_run(),
    )
    .await
    .expect("day 1 create must succeed");

    backend
        .execute_sql(
            "INSERT INTO main.raw_events VALUES (1, TIMESTAMP '2026-01-02 00:00:00', DATE \
             '2026-01-02')",
        )
        .await
        .expect("insert day 2");
    let steps = driving_steps("2026-01-02", "2026-01-03", &Granularity::Day).expect("steps");
    run_windowed_keyed_maintenance(
        &backend,
        "events_last_seen",
        "main",
        "events_last_seen",
        &steps,
        &classification(),
        Some(&derived_slice()),
        &unconditional(),
        None,
        compile_step,
        &no_retry_policy(),
        &smelt_runtime::probes::ProbePolicy::per_run(),
    )
    .await
    .expect("a derived slice must merge cleanly");

    let executed = backend.executed_sql.lock().unwrap();
    assert!(
        !executed
            .iter()
            .any(|sql| sql.contains("__recurrence_violations")),
        "a derived (unchecked) route-3 slice must never execute the out-of-slice match \
         probe: {:?}",
        *executed
    );
}
