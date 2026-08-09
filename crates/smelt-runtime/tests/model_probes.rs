//! Real-DuckDB coverage for `smelt_runtime::model_probes` — the pure
//! declared-probe builder plus the shared dispatch executor for the three
//! model-scoped probes (`docs/specs/model_properties.md` §"Probe
//! obligation"): `timeseries.assert_monotonic`, `functional_dependencies:`,
//! `bounded_domain:`. Mirrors `probe_dispatch.rs`'s harness.

use smelt_backend::{Backend, MaintenanceDialect};
use smelt_backend_duckdb::DuckDbBackend;
use smelt_core::config::{
    BoundedDomain, FunctionalDependency, Granularity, ProbeCadence, TimeseriesConfig,
};
use smelt_core::metadata::ModelMetadata;
use smelt_runtime::model_probes::{declared_model_probes, dispatch_declared_model_probes};
use smelt_runtime::probes::ProbePolicy;

async fn backend(tmp: &tempfile::TempDir) -> DuckDbBackend {
    let db_path = tmp.path().join("model_probes.duckdb");
    DuckDbBackend::new(&db_path, "main")
        .await
        .expect("open duckdb")
}

fn timeseries(assert_monotonic: bool) -> TimeseriesConfig {
    TimeseriesConfig {
        event_time_column: "event_time".to_string(),
        partition_column: "event_date".to_string(),
        granularity: Granularity::Day,
        week_start: None,
        assert_monotonic,
    }
}

fn metadata_with_fd(key: &[&str], determines: &str) -> ModelMetadata {
    ModelMetadata {
        functional_dependencies: vec![FunctionalDependency {
            key: key.iter().map(|s| s.to_string()).collect(),
            determines: determines.to_string(),
        }],
        ..Default::default()
    }
}

fn metadata_with_bounded_domain(column: &str, max_cardinality: u64) -> ModelMetadata {
    ModelMetadata {
        bounded_domain: Some(BoundedDomain {
            column: column.to_string(),
            max_cardinality,
        }),
        ..Default::default()
    }
}

#[tokio::test]
async fn fd_probe_fires_named_diagnostic() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let backend = backend(&tmp).await;
    backend
        .execute_sql(
            "CREATE TABLE subscriptions (customer_id INT, region TEXT); \
             INSERT INTO subscriptions VALUES (1, 'us'), (1, 'eu'), (2, 'us');",
        )
        .await
        .expect("stage");

    let metadata = metadata_with_fd(&["customer_id"], "region");
    let probes = declared_model_probes(
        "main.subscriptions",
        "main.subscriptions creation",
        Some(&metadata),
        None,
        "SELECT customer_id, region FROM subscriptions",
        MaintenanceDialect::DuckDb,
    );
    assert_eq!(probes.len(), 1);

    let err = dispatch_declared_model_probes(&backend, &ProbePolicy::per_run(), &probes)
        .await
        .expect_err("a violated functional dependency must fail loud");
    let message = err.to_string();
    assert!(
        message.contains("DeclaredFunctionalDependencyViolated"),
        "{message}"
    );
    assert!(message.contains("main.subscriptions creation"), "{message}");
    assert!(message.contains('1'), "{message}");
}

#[tokio::test]
async fn fd_probe_holds_on_conforming_data() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let backend = backend(&tmp).await;
    backend
        .execute_sql(
            "CREATE TABLE subscriptions (customer_id INT, region TEXT); \
             INSERT INTO subscriptions VALUES (1, 'us'), (1, 'us'), (2, 'eu');",
        )
        .await
        .expect("stage");

    let metadata = metadata_with_fd(&["customer_id"], "region");
    let probes = declared_model_probes(
        "main.subscriptions",
        "main.subscriptions creation",
        Some(&metadata),
        None,
        "SELECT customer_id, region FROM subscriptions",
        MaintenanceDialect::DuckDb,
    );

    let records = dispatch_declared_model_probes(&backend, &ProbePolicy::per_run(), &probes)
        .await
        .expect("conforming data must not error");
    assert_eq!(records.len(), 1);
}

#[tokio::test]
async fn bounded_domain_probe_fires_named_diagnostic() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let backend = backend(&tmp).await;
    backend
        .execute_sql(
            "CREATE TABLE shipments (country_code TEXT); \
             INSERT INTO shipments VALUES ('US'), ('CA'), ('MX'), ('FR');",
        )
        .await
        .expect("stage");

    let metadata = metadata_with_bounded_domain("country_code", 2);
    let probes = declared_model_probes(
        "main.shipments",
        "main.shipments creation",
        Some(&metadata),
        None,
        "SELECT country_code FROM shipments",
        MaintenanceDialect::DuckDb,
    );

    let err = dispatch_declared_model_probes(&backend, &ProbePolicy::per_run(), &probes)
        .await
        .expect_err("a bounded domain exceeded must fail loud");
    assert!(
        err.to_string().contains("DeclaredBoundedDomainExceeded"),
        "{err}"
    );
}

#[tokio::test]
async fn monotonicity_probe_fires_named_diagnostic() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let backend = backend(&tmp).await;
    backend
        .execute_sql(
            "CREATE TABLE events (user_id INT, event_time TIMESTAMP); \
             INSERT INTO events VALUES \
               (1, TIMESTAMP '2026-01-01 01:00:00'), \
               (1, TIMESTAMP '2026-01-01 00:00:00');",
        )
        .await
        .expect("stage");

    let ts = timeseries(true);
    let probes = declared_model_probes(
        "main.events",
        "main.events creation",
        None,
        Some(&ts),
        "SELECT user_id, event_time FROM events",
        MaintenanceDialect::DuckDb,
    );
    assert_eq!(probes.len(), 1);

    let err = dispatch_declared_model_probes(&backend, &ProbePolicy::per_run(), &probes)
        .await
        .expect_err("a violated monotonicity declaration must fail loud");
    assert!(
        err.to_string().contains("DeclaredMonotonicityViolated"),
        "{err}"
    );
}

#[tokio::test]
async fn undeclared_model_dispatches_no_probes() {
    let metadata = ModelMetadata::default();
    let ts = timeseries(false);
    let probes = declared_model_probes(
        "main.plain",
        "main.plain creation",
        Some(&metadata),
        Some(&ts),
        "SELECT 1",
        MaintenanceDialect::DuckDb,
    );
    assert!(probes.is_empty());

    let tmp = tempfile::TempDir::new().expect("tempdir");
    let backend = backend(&tmp).await;
    let records = dispatch_declared_model_probes(&backend, &ProbePolicy::per_run(), &probes)
        .await
        .expect("no probes to dispatch");
    assert!(records.is_empty());
}

#[tokio::test]
async fn assert_monotonic_false_dispatches_no_monotonicity_probe() {
    let ts = timeseries(false);
    let probes = declared_model_probes(
        "main.events",
        "main.events creation",
        None,
        Some(&ts),
        "SELECT user_id, event_time FROM events",
        MaintenanceDialect::DuckDb,
    );
    assert!(probes.is_empty());
}

#[tokio::test]
async fn cadence_off_skips_declared_probes() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let backend = backend(&tmp).await;
    backend
        .execute_sql(
            "CREATE TABLE shipments (country_code TEXT); \
             INSERT INTO shipments VALUES ('US'), ('CA'), ('MX'), ('FR');",
        )
        .await
        .expect("stage");

    let metadata = metadata_with_bounded_domain("country_code", 2);
    let probes = declared_model_probes(
        "main.shipments",
        "main.shipments creation",
        Some(&metadata),
        None,
        "SELECT country_code FROM shipments",
        MaintenanceDialect::DuckDb,
    );

    let policy = ProbePolicy::new(ProbeCadence::Off, 0);
    let records = dispatch_declared_model_probes(&backend, &policy, &probes)
        .await
        .expect("Off cadence must trust the declaration, not fail the run");
    assert_eq!(records.len(), 1);
}

#[tokio::test]
async fn multiple_functional_dependencies_each_dispatch() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let backend = backend(&tmp).await;
    backend
        .execute_sql(
            "CREATE TABLE t (a INT, b TEXT, c TEXT); \
             INSERT INTO t VALUES (1, 'x', 'p'), (1, 'x', 'q');",
        )
        .await
        .expect("stage");

    let metadata = ModelMetadata {
        functional_dependencies: vec![
            FunctionalDependency {
                key: vec!["a".to_string()],
                determines: "b".to_string(),
            },
            FunctionalDependency {
                key: vec!["a".to_string()],
                determines: "c".to_string(),
            },
        ],
        ..Default::default()
    };
    let probes = declared_model_probes(
        "main.t",
        "main.t creation",
        Some(&metadata),
        None,
        "SELECT a, b, c FROM t",
        MaintenanceDialect::DuckDb,
    );
    assert_eq!(probes.len(), 2);

    let err = dispatch_declared_model_probes(&backend, &ProbePolicy::per_run(), &probes)
        .await
        .expect_err("the second FD (a -> c) is violated and must fire");
    assert!(
        err.to_string()
            .contains("DeclaredFunctionalDependencyViolated"),
        "{err}"
    );
}
