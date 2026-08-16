//! Real-DuckDB coverage for `smelt_runtime::source_probes` — the pure
//! append-only posture probe builder plus the dispatch/refresh driver
//! (`docs/specs/model_properties.md` §"Probe obligation", row
//! `mutation_profile.kind: append_only`). Mirrors `model_probes.rs`'s
//! harness.

use smelt_backend::{Backend, MaintenanceDialect};
use smelt_backend_duckdb::DuckDbBackend;
use smelt_core::config::{Granularity, ProbeCadence, TimeseriesConfig};
use smelt_core::sources::{
    MutationProfile, SourceColumn, SourceInfo, SourceMutationProfile, SourceNameOverride,
};
use smelt_core::ModelFile;
use smelt_runtime::probes::ProbePolicy;
use smelt_runtime::reporter::{NoOpReporter, RunReporter};
use smelt_runtime::source_probes::{
    append_only_posture_probes, dispatch_and_record_append_only_postures,
};
use smelt_state::source_postures::{SourcePosturePartition, SourcePostureStore};
use smelt_state::ProbeRecordOutcome;
use smelt_types::DataType;
use std::sync::Mutex;

/// Captures `probe_advisory` calls for assertion — the other `RunReporter`
/// callbacks are irrelevant here, so every other method inherits the
/// trait's no-op default.
#[derive(Default)]
struct RecordingReporter {
    advisories: Mutex<Vec<(String, String)>>,
}

impl RunReporter for RecordingReporter {
    fn probe_advisory(&self, _run_id: &str, model: &str, code: &str, message: &str) {
        self.advisories
            .lock()
            .unwrap()
            .push((model.to_string(), format!("{code}: {message}")));
    }
}

async fn backend(tmp: &tempfile::TempDir) -> DuckDbBackend {
    let db_path = tmp.path().join("source_probes.duckdb");
    DuckDbBackend::new(&db_path, "main")
        .await
        .expect("open duckdb")
}

fn make_model(name: &str, sql: &str) -> ModelFile {
    let parse = smelt_parser::parse(sql);
    let refs = smelt_parser::ast::File::cast(parse.syntax())
        .map(|f| smelt_core::extract_refs(&f))
        .unwrap_or_default();
    let path = std::path::PathBuf::from(format!("models/{name}.sql"));
    ModelFile {
        name: name.to_string(),
        path: path.clone(),
        content: sql.to_string(),
        refs,
        parse_errors: Vec::new(),
        metadata: None,
        kind: smelt_core::ModelKind::Sql,
        model_id: smelt_core::ModelId::from_path(path),
        address_segments: vec![name.to_string()],
    }
}

fn day_ts(partition_column: &str) -> TimeseriesConfig {
    TimeseriesConfig {
        event_time_column: format!("{partition_column}_ts"),
        partition_column: partition_column.to_string(),
        granularity: Granularity::Day,
        week_start: None,
        assert_monotonic: false,
    }
}

fn append_only_source(segments: &[&str], partition_column: &str) -> SourceInfo {
    SourceInfo {
        path: std::path::PathBuf::from("/tmp/fake.yml"),
        address_segments: segments.iter().map(|s| s.to_string()).collect(),
        columns: vec![SourceColumn {
            name: "payload".to_string(),
            data_type: DataType::Text,
            nullable: true,
            description: None,
        }],
        description: None,
        name_override: Some(SourceNameOverride::Literal("raw.events".to_string())),
        tags: vec![],
        timeseries: Some(day_ts(partition_column)),
        mutation_profile: Some(SourceMutationProfile::from_kind(
            MutationProfile::AppendOnly,
        )),
        source_lateness: None,
        watermark: None,
        unique_key: None,
        retention: None,
        referential_integrity: None,
    }
}

#[tokio::test]
async fn first_run_has_no_baseline_so_builds_no_probe_and_records_one() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let backend = backend(&tmp).await;
    backend
        .execute_sql(
            "CREATE SCHEMA IF NOT EXISTS raw; \
             CREATE TABLE raw.events (event_date DATE, payload TEXT); \
             INSERT INTO raw.events VALUES (DATE '2026-01-01', 'a'), (DATE '2026-01-02', 'b');",
        )
        .await
        .expect("stage");

    let model = make_model("m", "SELECT * FROM smelt.sources.raw.events");
    let source = append_only_source(&["sources", "raw", "events"], "event_date");
    let empty_baselines = SourcePostureStore::default();

    let probes = append_only_posture_probes(
        "m",
        "m creation",
        &model,
        &[source],
        &empty_baselines,
        "dev",
        "raw",
        MaintenanceDialect::DuckDb,
    );
    assert_eq!(
        probes.len(),
        1,
        "an eligible source builds an Establish action"
    );
    assert!(
        matches!(
            probes[0].action,
            smelt_runtime::source_probes::SourcePostureAction::Establish { .. }
        ),
        "a source with no recorded baseline must build an Establish action, not a Verify"
    );

    let (refreshed, records) = dispatch_and_record_append_only_postures(
        &backend,
        &ProbePolicy::per_run(),
        &probes,
        &NoOpReporter,
        "run-1",
    )
    .await
    .expect("establishing a first baseline never violates");
    assert_eq!(
        refreshed.len(),
        1,
        "the first observation must be recorded as the baseline"
    );
    assert_eq!(refreshed[0].source_address, "raw.events");
    assert_eq!(refreshed[0].partitions.len(), 2);
    assert_eq!(records.len(), 1);
}

#[tokio::test]
async fn second_run_over_mutated_source_fires_with_the_registry_code_and_remedy() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let backend = backend(&tmp).await;
    backend
        .execute_sql(
            "CREATE SCHEMA IF NOT EXISTS raw; \
             CREATE TABLE raw.events (event_date DATE, payload TEXT); \
             INSERT INTO raw.events VALUES \
               (DATE '2026-01-01', 'a'), (DATE '2026-01-02', 'b');",
        )
        .await
        .expect("stage");

    let mut baselines = SourcePostureStore::default();
    baselines.record(
        "raw.events",
        vec![
            SourcePosturePartition {
                partition_value: "2026-01-01".to_string(),
                recorded_count: 1,
                // Deliberately wrong fingerprint relative to current
                // content, to prove a closed partition's mismatch fires.
                recorded_fingerprint: "not-the-real-fingerprint".to_string(),
            },
            SourcePosturePartition {
                partition_value: "2026-01-02".to_string(),
                recorded_count: 1,
                recorded_fingerprint: "irrelevant-for-the-open-partition".to_string(),
            },
        ],
    );

    let model = make_model("m", "SELECT * FROM smelt.sources.raw.events");
    let source = append_only_source(&["sources", "raw", "events"], "event_date");

    let probes = append_only_posture_probes(
        "m",
        "m creation",
        &model,
        &[source],
        &baselines,
        "dev",
        "raw",
        MaintenanceDialect::DuckDb,
    );
    assert_eq!(probes.len(), 1);

    let err = dispatch_and_record_append_only_postures(
        &backend,
        &ProbePolicy::per_run(),
        &probes,
        &NoOpReporter,
        "run-1",
    )
    .await
    .expect_err("a mismatched closed-partition fingerprint must fail loud");
    let message = err.to_string();
    assert!(
        message.contains("SourceMutationProfileViolated"),
        "{message}"
    );
    assert!(message.contains("raw.events"), "{message}");
    assert!(message.contains("Remedy:"), "{message}");
}

#[tokio::test]
async fn second_run_over_appended_source_holds_and_refreshes_the_baseline() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let backend = backend(&tmp).await;
    backend
        .execute_sql(
            "CREATE SCHEMA IF NOT EXISTS raw; \
             CREATE TABLE raw.events (event_date DATE, payload TEXT); \
             INSERT INTO raw.events VALUES (DATE '2026-01-01', 'a'), (DATE '2026-01-02', 'b');",
        )
        .await
        .expect("stage");

    // Snapshot the real baseline via the emitter's own snapshot query so
    // the recorded fingerprint matches production exactly.
    let snapshot_sql = smelt_logical::maintenance::emit::emit_append_only_baseline_snapshot(
        "raw.events",
        "event_date",
        &["payload".to_string()],
        MaintenanceDialect::DuckDb,
    )
    .sql;
    let batches = backend.execute_sql(&snapshot_sql).await.expect("snapshot");
    let rows = smelt_runtime::check_runner::batches_to_rows(&batches);
    let mut baselines = SourcePostureStore::default();
    baselines.record(
        "raw.events",
        rows.iter()
            .map(|r| SourcePosturePartition {
                partition_value: r.get("partition_value").cloned().unwrap_or_default(),
                recorded_count: r
                    .get("current_count")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0),
                recorded_fingerprint: r.get("current_fingerprint").cloned().unwrap_or_default(),
            })
            .collect(),
    );

    // A legitimate append into the still-open (max) partition.
    backend
        .execute_sql("INSERT INTO raw.events VALUES (DATE '2026-01-02', 'c');")
        .await
        .expect("append");

    let model = make_model("m", "SELECT * FROM smelt.sources.raw.events");
    let source = append_only_source(&["sources", "raw", "events"], "event_date");

    let probes = append_only_posture_probes(
        "m",
        "m creation",
        &model,
        &[source],
        &baselines,
        "dev",
        "raw",
        MaintenanceDialect::DuckDb,
    );
    assert_eq!(probes.len(), 1);

    let (refreshed, records) = dispatch_and_record_append_only_postures(
        &backend,
        &ProbePolicy::per_run(),
        &probes,
        &NoOpReporter,
        "run-1",
    )
    .await
    .expect("an append into the open partition must hold");
    assert_eq!(refreshed.len(), 1);
    assert_eq!(refreshed[0].source_address, "raw.events");
    let refreshed_max = refreshed[0]
        .partitions
        .iter()
        .find(|p| p.partition_value == "2026-01-02")
        .expect("refreshed baseline must include the open partition");
    assert_eq!(
        refreshed_max.recorded_count, 2,
        "the refreshed baseline must reflect the new row"
    );
    assert_eq!(records.len(), 1);
}

#[tokio::test]
async fn cadence_off_skips_dispatch_and_leaves_the_baseline_untouched() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let backend = backend(&tmp).await;
    backend
        .execute_sql(
            "CREATE SCHEMA IF NOT EXISTS raw; \
             CREATE TABLE raw.events (event_date DATE, payload TEXT); \
             INSERT INTO raw.events VALUES (DATE '2026-01-01', 'a'), (DATE '2026-01-02', 'b');",
        )
        .await
        .expect("stage");

    let mut baselines = SourcePostureStore::default();
    baselines.record(
        "raw.events",
        vec![
            SourcePosturePartition {
                partition_value: "2026-01-01".to_string(),
                recorded_count: 1,
                recorded_fingerprint: "not-the-real-fingerprint".to_string(),
            },
            SourcePosturePartition {
                partition_value: "2026-01-02".to_string(),
                recorded_count: 1,
                recorded_fingerprint: "irrelevant-for-the-open-partition".to_string(),
            },
        ],
    );

    let model = make_model("m", "SELECT * FROM smelt.sources.raw.events");
    let source = append_only_source(&["sources", "raw", "events"], "event_date");

    let probes = append_only_posture_probes(
        "m",
        "m creation",
        &model,
        &[source],
        &baselines,
        "dev",
        "raw",
        MaintenanceDialect::DuckDb,
    );
    assert_eq!(probes.len(), 1);

    let policy = ProbePolicy::new(ProbeCadence::Off, 0);
    let (refreshed, records) = dispatch_and_record_append_only_postures(
        &backend,
        &policy,
        &probes,
        &NoOpReporter,
        "run-1",
    )
    .await
    .expect("Off cadence must trust the declaration, not fail the run");
    assert!(
        refreshed.is_empty(),
        "a cadence-skipped run must not refresh the baseline"
    );
    assert_eq!(records.len(), 1);
}

#[tokio::test]
async fn first_observation_records_baseline_established_and_reports_advisory() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let backend = backend(&tmp).await;
    backend
        .execute_sql(
            "CREATE SCHEMA IF NOT EXISTS raw; \
             CREATE TABLE raw.events (event_date DATE, payload TEXT); \
             INSERT INTO raw.events VALUES (DATE '2026-01-01', 'a'), (DATE '2026-01-02', 'b');",
        )
        .await
        .expect("stage");

    let model = make_model("m", "SELECT * FROM smelt.sources.raw.events");
    let source = append_only_source(&["sources", "raw", "events"], "event_date");
    let empty_baselines = SourcePostureStore::default();

    let probes = append_only_posture_probes(
        "m",
        "m creation",
        &model,
        &[source],
        &empty_baselines,
        "dev",
        "raw",
        MaintenanceDialect::DuckDb,
    );
    assert_eq!(probes.len(), 1);

    let reporter = RecordingReporter::default();
    let (_refreshed, records) = dispatch_and_record_append_only_postures(
        &backend,
        &ProbePolicy::per_run(),
        &probes,
        &reporter,
        "run-1",
    )
    .await
    .expect("establishing a first baseline never violates");

    assert_eq!(records.len(), 1);
    assert_eq!(records[0].outcome, ProbeRecordOutcome::BaselineEstablished);

    let advisories = reporter.advisories.lock().unwrap();
    assert_eq!(
        advisories.len(),
        1,
        "an establishing run must report exactly one advisory"
    );
    let (model, message) = &advisories[0];
    assert_eq!(model, "m");
    assert!(message.contains("ProbeBaselineUnavailable"), "{message}");
    assert!(message.contains("raw.events"), "{message}");
}

#[tokio::test]
async fn second_run_verifies_and_reports_no_advisory() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let backend = backend(&tmp).await;
    backend
        .execute_sql(
            "CREATE SCHEMA IF NOT EXISTS raw; \
             CREATE TABLE raw.events (event_date DATE, payload TEXT); \
             INSERT INTO raw.events VALUES (DATE '2026-01-01', 'a'), (DATE '2026-01-02', 'b');",
        )
        .await
        .expect("stage");

    let snapshot_sql = smelt_logical::maintenance::emit::emit_append_only_baseline_snapshot(
        "raw.events",
        "event_date",
        &["payload".to_string()],
        MaintenanceDialect::DuckDb,
    )
    .sql;
    let batches = backend.execute_sql(&snapshot_sql).await.expect("snapshot");
    let rows = smelt_runtime::check_runner::batches_to_rows(&batches);
    let mut baselines = SourcePostureStore::default();
    baselines.record(
        "raw.events",
        rows.iter()
            .map(|r| SourcePosturePartition {
                partition_value: r.get("partition_value").cloned().unwrap_or_default(),
                recorded_count: r
                    .get("current_count")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0),
                recorded_fingerprint: r.get("current_fingerprint").cloned().unwrap_or_default(),
            })
            .collect(),
    );

    let model = make_model("m", "SELECT * FROM smelt.sources.raw.events");
    let source = append_only_source(&["sources", "raw", "events"], "event_date");

    let probes = append_only_posture_probes(
        "m",
        "m creation",
        &model,
        &[source],
        &baselines,
        "dev",
        "raw",
        MaintenanceDialect::DuckDb,
    );
    assert_eq!(probes.len(), 1);
    assert!(
        matches!(
            probes[0].action,
            smelt_runtime::source_probes::SourcePostureAction::Verify { .. }
        ),
        "a source with a recorded baseline must build a Verify action"
    );

    let reporter = RecordingReporter::default();
    let (_refreshed, records) = dispatch_and_record_append_only_postures(
        &backend,
        &ProbePolicy::per_run(),
        &probes,
        &reporter,
        "run-2",
    )
    .await
    .expect("an unchanged closed partition must hold");

    assert_eq!(records.len(), 1);
    assert_eq!(records[0].outcome, ProbeRecordOutcome::Dispatched);
    assert!(
        reporter.advisories.lock().unwrap().is_empty(),
        "a verifying run against a recorded baseline must not report the advisory"
    );
}
