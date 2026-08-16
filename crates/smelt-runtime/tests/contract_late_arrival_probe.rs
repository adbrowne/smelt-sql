//! Real-DuckDB coverage for `smelt_runtime::contract_probes` — the pure
//! frozen-horizon late-arrival probe builder plus the dispatch/refresh
//! driver (`docs/specs/incremental_models.md` §"The contract lattice",
//! frozen horizon; `docs/outcomes/20260809-contract-lattice-v1/phases/
//! 03-plan.md`). Mirrors `tests/source_probes.rs`'s harness.

use chrono::NaiveDate;
use smelt_backend::{Backend, MaintenanceDialect};
use smelt_backend_duckdb::DuckDbBackend;
use smelt_core::config::{ContractConfig, Granularity, TimeseriesConfig};
use smelt_core::metadata::ModelMetadata;
use smelt_core::sources::{SourceColumn, SourceInfo, SourceNameOverride};
use smelt_core::ModelFile;
use smelt_runtime::contract_probes::{
    dispatch_and_record_frozen_horizon_probes, frozen_horizon_probes,
};
use smelt_runtime::probes::ProbePolicy;
use smelt_runtime::reporter::{NoOpReporter, RunReporter};
use smelt_state::frozen_band_baselines::FrozenBandBaselineStore;
use smelt_state::ProbeRecordOutcome;
use smelt_types::DataType;
use std::sync::Mutex;

async fn backend(tmp: &tempfile::TempDir) -> DuckDbBackend {
    let db_path = tmp.path().join("contract_probes.duckdb");
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

fn clocked_source(segments: &[&str], partition_column: &str) -> SourceInfo {
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
        mutation_profile: None,
        source_lateness: None,
        watermark: None,
        unique_key: None,
        retention: None,
        referential_integrity: None,
    }
}

fn frozen_horizon_metadata(days: u32) -> ModelMetadata {
    ModelMetadata {
        contract: Some(ContractConfig {
            frozen_horizon: smelt_core::config::DataLatency::parse(&format!("{days} days")),
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn deferral_metadata(days: u32) -> ModelMetadata {
    ModelMetadata {
        contract: Some(ContractConfig {
            deferral: smelt_core::config::DataLatency::parse(&format!("{days} days")),
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// Captures `probe_advisory` calls for assertion — mirrors
/// `tests/source_probes.rs`'s `RecordingReporter`.
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

#[tokio::test]
async fn first_run_establishes_frozen_band_baseline_without_violating() {
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
    let source = clocked_source(&["sources", "raw", "events"], "event_date");
    let metadata = frozen_horizon_metadata(1);
    let end_date = NaiveDate::from_ymd_opt(2026, 2, 1).unwrap();
    let empty_baselines = FrozenBandBaselineStore::default();

    let probes = frozen_horizon_probes(
        "m",
        "m creation",
        &model,
        Some(&metadata),
        &[source],
        end_date,
        "dev",
        "raw",
        MaintenanceDialect::DuckDb,
    );
    assert_eq!(
        probes.len(),
        1,
        "a declared frozen_horizon builds one probe"
    );

    let result = dispatch_and_record_frozen_horizon_probes(
        &backend,
        &ProbePolicy::per_run(),
        &probes,
        &empty_baselines,
        &NoOpReporter,
        "run-1",
    )
    .await
    .expect("establishing a first baseline never errors");
    assert!(
        result.violations.is_empty(),
        "the first observation must never violate"
    );
    assert_eq!(result.refreshed.len(), 1);
    assert_eq!(result.refreshed[0].source_address, "raw.events");
    assert_eq!(result.records.len(), 1);
}

#[tokio::test]
async fn late_row_in_frozen_band_raises_contract_late_arrival_outside_horizon() {
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
    let source = clocked_source(&["sources", "raw", "events"], "event_date");
    let metadata = frozen_horizon_metadata(1);
    // H = 1 day, end = 2026-01-03 -> frozen band is everything before
    // 2026-01-02, so 2026-01-01 is frozen.
    let end_date = NaiveDate::from_ymd_opt(2026, 1, 3).unwrap();

    let probes = frozen_horizon_probes(
        "m",
        "m creation",
        &model,
        Some(&metadata),
        std::slice::from_ref(&source),
        end_date,
        "dev",
        "raw",
        MaintenanceDialect::DuckDb,
    );
    assert_eq!(probes.len(), 1);

    // Establish the baseline first.
    let empty_baselines = FrozenBandBaselineStore::default();
    let first = dispatch_and_record_frozen_horizon_probes(
        &backend,
        &ProbePolicy::per_run(),
        &probes,
        &empty_baselines,
        &NoOpReporter,
        "run-1",
    )
    .await
    .expect("establish");
    assert!(first.violations.is_empty());
    let mut baselines = FrozenBandBaselineStore::default();
    for r in first.refreshed {
        baselines.record(&r.source_address, r.partitions);
    }

    // A genuinely late arrival into the already-frozen 2026-01-01 partition.
    backend
        .execute_sql("INSERT INTO raw.events VALUES (DATE '2026-01-01', 'late');")
        .await
        .expect("late insert");

    let probes = frozen_horizon_probes(
        "m",
        "m creation",
        &model,
        Some(&metadata),
        &[source],
        end_date,
        "dev",
        "raw",
        MaintenanceDialect::DuckDb,
    );
    let second = dispatch_and_record_frozen_horizon_probes(
        &backend,
        &ProbePolicy::per_run(),
        &probes,
        &baselines,
        &NoOpReporter,
        "run-2",
    )
    .await
    .expect("dispatch must not error even on a violation");

    assert_eq!(second.violations.len(), 1, "the late row must be flagged");
    let violation = &second.violations[0];
    assert!(
        violation
            .message
            .contains("ContractLateArrivalOutsideHorizon"),
        "{}",
        violation.message
    );
    assert!(
        violation.message.contains("2026-01-01"),
        "{}",
        violation.message
    );
    assert!(
        violation.message.contains("H = 1 day"),
        "{}",
        violation.message
    );
    // The baseline must still be refreshed so the same late row is not
    // re-reported on a subsequent run.
    assert_eq!(second.refreshed.len(), 1);
}

#[tokio::test]
async fn no_frozen_horizon_declaration_dispatches_no_contract_probe() {
    let model = make_model("m", "SELECT * FROM smelt.sources.raw.events");
    let source = clocked_source(&["sources", "raw", "events"], "event_date");
    let end_date = NaiveDate::from_ymd_opt(2026, 2, 1).unwrap();

    let probes = frozen_horizon_probes(
        "m",
        "m creation",
        &model,
        None,
        &[source],
        end_date,
        "dev",
        "raw",
        MaintenanceDialect::DuckDb,
    );
    assert!(
        probes.is_empty(),
        "absent a `contract.frozen_horizon` declaration the probe is opt-in only"
    );
}

#[tokio::test]
async fn frozen_band_first_observation_reports_baseline_unavailable() {
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
    let source = clocked_source(&["sources", "raw", "events"], "event_date");
    let metadata = frozen_horizon_metadata(1);
    let end_date = NaiveDate::from_ymd_opt(2026, 2, 1).unwrap();
    let empty_baselines = FrozenBandBaselineStore::default();

    let probes = frozen_horizon_probes(
        "m",
        "m creation",
        &model,
        Some(&metadata),
        &[source],
        end_date,
        "dev",
        "raw",
        MaintenanceDialect::DuckDb,
    );
    assert_eq!(probes.len(), 1);

    let reporter = RecordingReporter::default();
    let result = dispatch_and_record_frozen_horizon_probes(
        &backend,
        &ProbePolicy::per_run(),
        &probes,
        &empty_baselines,
        &reporter,
        "run-1",
    )
    .await
    .expect("establishing a first baseline never errors");

    assert!(
        result.violations.is_empty(),
        "the absent-baseline branch must not manufacture a LateArrivalViolation"
    );
    assert_eq!(result.records.len(), 1);
    assert_eq!(
        result.records[0].outcome,
        ProbeRecordOutcome::BaselineEstablished
    );

    let advisories = reporter.advisories.lock().unwrap();
    assert_eq!(advisories.len(), 1);
    let (model, message) = &advisories[0];
    assert_eq!(model, "m");
    assert!(message.contains("ProbeBaselineUnavailable"), "{message}");
    assert!(message.contains("raw.events"), "{message}");
}

#[test]
fn deferral_never_reports_baseline_unavailable() {
    // `contract.deferral`'s refusal is phase 5's `DeclaredContractRequiresState`,
    // not an absent-baseline advisory here — `evaluate_deferral` compares two
    // already-recorded ledger frontiers and has no baseline concept at all, so
    // it never records `BaselineEstablished` regardless of whether the
    // frontiers are present.
    let metadata = deferral_metadata(1);
    let probes =
        smelt_runtime::contract_probes::deferral_probes("m", "m creation", Some(&metadata));
    assert_eq!(probes.len(), 1);

    let policy = ProbePolicy::per_run();
    let interval_store = smelt_state::intervals::IntervalStore::default();
    let landed_deltas = smelt_state::landed_deltas::LandedDeltaStore::default();

    let (records, _violations) = smelt_runtime::contract_probes::evaluate_deferral(
        &policy,
        &probes,
        "m",
        &[],
        &interval_store,
        &landed_deltas,
    );

    assert_eq!(records.len(), 1);
    assert_ne!(
        records[0].outcome,
        ProbeRecordOutcome::BaselineEstablished,
        "deferral has no baseline to establish"
    );
}
