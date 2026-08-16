//! Real-DuckDB coverage for `smelt_runtime::probes::dispatch_probe` — the
//! single-owner probe dispatch helper (`docs/specs/model_properties.md`
//! §"Probe cadence"): cadence decision, backend execution, the shared
//! `violation_count`/`sample_keys` contract, and the named firing
//! diagnostic. Also covers the two re-routed dispatch sites in
//! `maintenance_driver.rs` (recurrence-bound probe) end-to-end for the
//! cadence-skip and probe-unavailable cases `dispatch_probe`'s own unit
//! coverage cannot reach.

use std::path::Path;

use smelt_backend::Backend;
use smelt_backend_duckdb::DuckDbBackend;
use smelt_core::config::{Granularity, ProbeCadence, TimeseriesConfig};
use smelt_logical::maintenance::choice::WriteSuppression;
use smelt_logical::maintenance::emit::TargetSlicePredicate;
use smelt_logical::maintenance::locality::LocalitySlice;
use smelt_planner::{
    AggregatorColumn, CrossPartitionCombiner, CumulativeClassification, DrivingSource,
};
use smelt_runtime::maintenance_driver::{
    driving_steps, run_windowed_keyed_maintenance, MaintenanceStep,
};
use smelt_runtime::probes::{dispatch_probe, ProbeContext, ProbePolicy, ProbeVerdict};
use smelt_state::reconciliation::Grade;

async fn backend(tmp: &tempfile::TempDir) -> DuckDbBackend {
    let db_path = tmp.path().join("probe_dispatch.duckdb");
    DuckDbBackend::new(&db_path, "main")
        .await
        .expect("open duckdb")
}

fn ctx() -> ProbeContext {
    ProbeContext {
        probe_code: "TestProbeViolated".to_string(),
        fact: "bounded_domain".to_string(),
        model: "main.t".to_string(),
        cell: "main.t creation".to_string(),
        remedy: "correct the source data or widen the declared domain".to_string(),
    }
}

#[tokio::test]
async fn holding_probe_reports_held_and_no_error() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let backend = backend(&tmp).await;
    let sql = "SELECT 0 AS violation_count, NULL AS sample_keys";
    let verdict = dispatch_probe(&backend, &ProbePolicy::per_run(), &ctx(), sql)
        .await
        .expect("a holding probe must not error");
    assert!(matches!(verdict, ProbeVerdict::Held));
}

#[tokio::test]
async fn violating_probe_fails_with_named_diagnostic_fact_cell_and_remedy() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let backend = backend(&tmp).await;
    let sql = "SELECT 3 AS violation_count, 'k1,k2' AS sample_keys";
    let context = ctx();
    let verdict = dispatch_probe(&backend, &ProbePolicy::per_run(), &context, sql)
        .await
        .expect("dispatch itself must not error — only the caller decides to fail the run");
    let (count, sample_keys) = match verdict {
        ProbeVerdict::Violated { count, sample_keys } => (count, sample_keys),
        other => panic!("expected Violated, got {other:?}"),
    };
    let message = format!(
        "{}: model '{}' declares `{}`, but {} row(s) violate it. Sample keys: {}.{}",
        context.probe_code,
        context.model,
        context.fact,
        count,
        sample_keys,
        smelt_runtime::probes::probe_violation_suffix(&context)
    );
    assert!(message.contains("TestProbeViolated"), "{message}");
    assert!(message.contains("bounded_domain"), "{message}");
    assert!(message.contains("k1,k2"), "{message}");
    assert!(message.contains("main.t creation"), "{message}");
    assert!(
        message.contains("correct the source data or widen the declared domain"),
        "{message}"
    );
}

#[tokio::test]
async fn probe_result_missing_violation_count_fails_loud() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let backend = backend(&tmp).await;
    let sql = "SELECT 'oops' AS not_violation_count";
    let err = dispatch_probe(&backend, &ProbePolicy::per_run(), &ctx(), sql)
        .await
        .expect_err("a malformed probe row must refuse, not read as \"no violation\"");
    assert!(err.to_string().contains("violation_count"), "{err}");
}

#[tokio::test]
async fn off_cadence_skips_dispatch_and_executes_no_sql() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let backend = backend(&tmp).await;
    let policy = ProbePolicy::new(ProbeCadence::Off, 0);
    // Invalid SQL: if `dispatch_probe` ever reached the backend call under
    // `Off`, this would error instead of returning `Skipped`.
    let sql = "THIS IS NOT VALID SQL AT ALL";
    let verdict = dispatch_probe(&backend, &policy, &ctx(), sql)
        .await
        .expect("Off cadence must skip before touching the backend");
    assert!(matches!(
        verdict,
        ProbeVerdict::Skipped(smelt_logical::maintenance::SkipReason::CadenceOff)
    ));
}

// ── Driver-level coverage: the recurrence-bound probe re-routed through
// `dispatch_probe` ──────────────────────────────────────────────────────

const NO_OP_REPORTER: smelt_runtime::NoOpReporter = smelt_runtime::NoOpReporter;
fn no_retry_policy() -> smelt_runtime::RetryPolicy<'static> {
    smelt_runtime::RetryPolicy {
        retry_max: 0,
        base_backoff_ms: 0,
        run_id: "probe-dispatch-test",
        model_name: "probe-dispatch-test",
        reporter: &NO_OP_REPORTER,
    }
}

fn unconditional() -> WriteSuppression {
    WriteSuppression::Unconditional {
        why: "test exercises probe dispatch, not suppression".to_string(),
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

fn checked_slice() -> LocalitySlice {
    LocalitySlice::RecurrenceBounded {
        partition_column: "last_seen_date".to_string(),
        margin_before: smelt_logical::analysis::source_bounds::Seconds::days(3),
        margin_after: smelt_logical::analysis::source_bounds::Seconds::ZERO,
        r: smelt_logical::analysis::source_bounds::Seconds::days(3),
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

async fn insert_event(backend: &DuckDbBackend, event_id: i64, date: &str) {
    backend
        .execute_sql(&format!(
            "INSERT INTO main.raw_events VALUES ({event_id}, TIMESTAMP '{date} 00:00:00', DATE '{date}')"
        ))
        .await
        .expect("insert event");
}

fn compile_step(step: &MaintenanceStep) -> anyhow::Result<String> {
    Ok(format!(
        "SELECT event_id, MAX(event_date) AS last_seen_date FROM main.raw_events \
         WHERE event_date = '{}' GROUP BY event_id",
        step.partition_value
    ))
}

/// A declared, checked route-3 bound is violated (a redelivery further
/// apart than `r`), but `probes.cadence: off` trusts the declaration:
/// the merge completes instead of refusing, and the run records the
/// declaration as unverified rather than checking it
/// (`docs/specs/model_properties.md` §"Probe cadence").
#[tokio::test]
async fn recurrence_probe_under_off_cadence_runs_without_dispatch() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("dev.duckdb");
    let backend = setup_backend(&db_path).await;

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
        compile_step,
        &no_retry_policy(),
        &ProbePolicy::per_run(),
    )
    .await
    .expect("day 1 create must succeed");

    // Day 6 redelivery — 5 days after day 1, further apart than r=3 days:
    // a genuine violation under `per_run`, but `off` trusts it.
    insert_event(&backend, 1, "2026-01-06").await;
    let violating_steps =
        driving_steps("2026-01-06", "2026-01-07", &Granularity::Day).expect("steps");
    run_windowed_keyed_maintenance(
        &backend,
        "events_last_seen",
        "main",
        "events_last_seen",
        &violating_steps,
        &classification(),
        Some(&checked_slice()),
        &unconditional(),
        compile_step,
        &no_retry_policy(),
        &ProbePolicy::new(ProbeCadence::Off, 0),
    )
    .await
    .expect("Off cadence must trust the declaration and let the merge complete");
}

/// A rule that never overrides `recurrence_probe_sql` (the trait default,
/// `None`) cannot build a checked-route probe. The policy-skip path must
/// not weaken this — the driver still refuses fail-closed even under
/// `per_run`, the cadence that would otherwise dispatch.
struct RuleWithNoProbe;

impl smelt_runtime::maintenance_driver::WindowedKeyedRule for RuleWithNoProbe {
    fn refuse(&self) -> Option<String> {
        None
    }
    fn merge_sql(
        &self,
        _schema: &str,
        _table: &str,
        _delta_sql: &str,
        _slice: Option<&TargetSlicePredicate>,
        _suppression: &WriteSuppression,
    ) -> String {
        "SELECT 1".to_string()
    }
    fn ledger_grade(&self) -> Grade {
        Grade::Idempotent
    }
}

#[tokio::test]
async fn unbuildable_probe_still_fails_closed_under_per_run() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("dev.duckdb");
    let backend = setup_backend(&db_path).await;

    insert_event(&backend, 1, "2026-01-01").await;
    let create_steps = driving_steps("2026-01-01", "2026-01-02", &Granularity::Day).expect("steps");
    run_windowed_keyed_maintenance(
        &backend,
        "events_last_seen",
        "main",
        "events_last_seen",
        &create_steps,
        &RuleWithNoProbe,
        Some(&checked_slice()),
        &unconditional(),
        compile_step,
        &no_retry_policy(),
        &ProbePolicy::per_run(),
    )
    .await
    .expect("day 1 create has no target table yet, so no probe is consulted");

    insert_event(&backend, 1, "2026-01-06").await;
    let merge_steps = driving_steps("2026-01-06", "2026-01-07", &Granularity::Day).expect("steps");
    let err = run_windowed_keyed_maintenance(
        &backend,
        "events_last_seen",
        "main",
        "events_last_seen",
        &merge_steps,
        &RuleWithNoProbe,
        Some(&checked_slice()),
        &unconditional(),
        compile_step,
        &no_retry_policy(),
        &ProbePolicy::per_run(),
    )
    .await
    .expect_err("a rule with no recurrence-bound probe must refuse fail-closed");
    assert!(err.to_string().contains("recurrence-bound probe"), "{err}");
}
