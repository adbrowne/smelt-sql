//! Fact-violation recipe pool — the standing conformance-gate coverage for
//! `docs/specs/model_properties.md` §"Probe obligation"
//! (`docs/outcomes/20260809-probe-backed-facts/outcome.md` phase 7,
//! success criterion 6): one recipe per `built` registry row, each staging a
//! real project/backend, feeding it conforming data (the run succeeds and
//! matches the full-refresh oracle) and then violating data (the run fails
//! with the registry's named diagnostic, *before* any write). A
//! spec-parsed coverage test (`every_built_registry_row_has_a_violation_recipe`)
//! fails whenever a `built` row has no recipe here, or a recipe here names a
//! row that is not `built` — the pool cannot silently fall behind the
//! registry.
//!
//! Criterion 6's "caught by its probe, **not** by wrong output" gets a third
//! leg: for a recipe whose violation is end-state observable, the same
//! violating feed under `probes: {cadence: off}` must write output that
//! *differs* from the full-refresh oracle — proving the probe is load-bearing
//! rather than decorative. Three of the six lifted/full-refresh-shaped
//! recipes are **not** end-state observable in the shape staged here (each
//! reason is a real, checked property of the staged recipe, not a cop-out):
//!
//! - `functional_dependencies` (`DeclaredFunctionalDependencyViolated`): the
//!   staged model is a plain full-refresh `table` passthrough. The FD
//!   declaration only ever licenses the once-write `COALESCE`
//!   family for *incremental* maintenance (`model_properties.md`
//!   §"Model-scoped declarations"); a full-refresh CTAS reads the same rows
//!   whether or not the key→column constancy holds, so the write is
//!   bit-identical either way.
//! - `bounded_domain` (`DeclaredBoundedDomainExceeded`): the staged model is
//!   a per-day passthrough with no holistic aggregate (`MEDIAN`/`MODE`/exact
//!   `COUNT(DISTINCT)`). `bounded_domain:` only licenses a different
//!   technique for that aggregate family; a plain `SELECT` ignores the cap
//!   entirely, so the write reflects whatever the source holds regardless.
//! - `mutation_profile.kind: append_only` posture
//!   (`SourceMutationProfileViolated`): the staged model has no
//!   `refresh: incremental` — every run is a full-refresh `CREATE TABLE AS`
//!   from the source's *current* contents, so the write trivially equals a
//!   full-refresh oracle whether or not the append-only posture holds; the
//!   posture only matters for an incremental delta-restricted read, which
//!   this recipe's full-refresh shape never takes.
//! - `timeseries.assert_monotonic` (`DeclaredMonotonicityViolated`): this
//!   recipe dispatches the model-scoped probe directly
//!   (`smelt_runtime::model_probes`) over an unconditional `CREATE TABLE AS`
//!   the harness performs itself, regardless of whether the trace was
//!   Undecidable — there is no scan-bound pushdown in this harness for the
//!   declaration to have licensed differently, so the write is the same
//!   `SELECT` either way. A genuinely divergent shape needs an
//!   Undecidable-trace incremental model whose bound derivation depends on
//!   the declaration; out of scope for this recipe.
//!
//! The remaining two recipes (`referential_integrity` /
//! `SourceCountPreservationViolated`, `key_recurrence` /
//! `KeyedRecurrenceBoundViolated`) exercise real narrowing techniques
//! (a delta-restricted recompute, a recurrence-bounded checked merge) and
//! are end-state observable: with the probe off, the narrowed technique
//! silently leaves stale/incomplete state that a full recompute would not.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;

use smelt_backend::{Backend, MaintenanceDialect};
use smelt_backend_duckdb::DuckDbBackend;
use smelt_core::config::{Granularity, ProbeCadence, TimeseriesConfig};
use smelt_logical::analysis::source_bounds::Seconds;
use smelt_logical::maintenance::choice::WriteSuppression;
use smelt_logical::maintenance::locality::LocalitySlice;
use smelt_logical::maintenance::{RowPreservation, SkeletonSourceClosure};
use smelt_maintenance_testkit::link_c_harness::{base_request, LinkCProject};
use smelt_maintenance_testkit::oracle::multiset_equal_via_backend;
use smelt_planner::{
    AggregatorColumn, CrossPartitionCombiner, CumulativeClassification, DrivingSource,
};
use smelt_runtime::maintenance_driver::{
    driving_steps, execute_delete_insert_with_delta_restriction, run_windowed_keyed_maintenance,
    RestrictionDeltaSource,
};
use smelt_runtime::model_probes::{declared_model_probes, dispatch_declared_model_probes};
use smelt_runtime::probes::ProbePolicy;

type BoxFut = Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send>>;

/// Whether a recipe's violation is observable in its maintained OUTPUT once
/// the probe stops checking (`probes: {cadence: off}`) — see the module doc
/// comment for the per-recipe reasoning.
enum Observability {
    /// The violating feed, probes off, writes and the write differs from the
    /// full-refresh oracle.
    Observable(fn() -> BoxFut),
    /// The declaration's staged shape licenses no technique divergence; the
    /// reason is printed as an explicit skip, never silently omitted.
    NotObservable(&'static str),
}

struct Recipe {
    name: &'static str,
    diagnostic: &'static str,
    conforming: fn() -> BoxFut,
    violated: fn() -> BoxFut,
    observability: Observability,
}

fn recipes() -> Vec<Recipe> {
    vec![
        Recipe {
            name: "functional_dependencies",
            diagnostic: "DeclaredFunctionalDependencyViolated",
            conforming: || Box::pin(fd_conforming()),
            violated: || Box::pin(fd_violated()),
            observability: Observability::NotObservable(
                "plain full-refresh `table` passthrough — the once-write COALESCE family FD \
                 licenses only applies to incremental maintenance; the write is bit-identical \
                 with or without the declaration holding",
            ),
        },
        Recipe {
            name: "bounded_domain",
            diagnostic: "DeclaredBoundedDomainExceeded",
            conforming: || Box::pin(bounded_domain_conforming()),
            violated: || Box::pin(bounded_domain_violated()),
            observability: Observability::NotObservable(
                "the staged model has no holistic aggregate (MEDIAN/MODE/exact COUNT(DISTINCT)) \
                 for bounded_domain to license a technique for — a plain per-day passthrough \
                 ignores the cap entirely",
            ),
        },
        Recipe {
            name: "mutation_profile.kind: append_only",
            diagnostic: "SourceMutationProfileViolated",
            conforming: || Box::pin(append_only_conforming()),
            violated: || Box::pin(append_only_violated()),
            observability: Observability::NotObservable(
                "the staged model has no `refresh: incremental` — every run is a full-refresh \
                 CREATE TABLE AS from the source's current contents, trivially matching the \
                 oracle regardless of posture",
            ),
        },
        Recipe {
            name: "timeseries.assert_monotonic",
            diagnostic: "DeclaredMonotonicityViolated",
            conforming: || Box::pin(monotonicity_conforming()),
            violated: || Box::pin(monotonicity_violated()),
            observability: Observability::NotObservable(
                "this recipe dispatches the model-scoped probe directly over an unconditional \
                 CREATE TABLE AS the harness performs itself — there is no scan-bound pushdown \
                 in this harness for the declaration to have licensed differently",
            ),
        },
        Recipe {
            name: "referential_integrity",
            diagnostic: "SourceCountPreservationViolated",
            conforming: || Box::pin(count_preservation_conforming()),
            violated: || Box::pin(count_preservation_violated()),
            observability: Observability::Observable(|| {
                Box::pin(count_preservation_violated_probes_off())
            }),
        },
        Recipe {
            name: "key_recurrence",
            diagnostic: "KeyedRecurrenceBoundViolated",
            conforming: || Box::pin(recurrence_conforming()),
            violated: || Box::pin(recurrence_violated()),
            observability: Observability::Observable(|| Box::pin(recurrence_violated_probes_off())),
        },
    ]
}

// ---------------------------------------------------------------------
// Shared staging helpers
// ---------------------------------------------------------------------

fn write_file(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, content).unwrap();
}

fn no_retry_policy() -> smelt_runtime::RetryPolicy<'static> {
    const NO_OP_REPORTER: smelt_runtime::NoOpReporter = smelt_runtime::NoOpReporter;
    smelt_runtime::RetryPolicy {
        retry_max: 0,
        base_backoff_ms: 0,
        run_id: "fact-violations-gate",
        model_name: "fact-violations-gate",
        reporter: &NO_OP_REPORTER,
    }
}

// ---------------------------------------------------------------------
// Recipe: functional_dependencies (`e2e/declared_fact_probe_firing.rs::stage_fd_workspace`)
// ---------------------------------------------------------------------

fn stage_fd_project(tmp: &tempfile::TempDir) -> anyhow::Result<LinkCProject> {
    let root = tmp.path().join("fd_probe");
    let db_path = root.join("target/dev.duckdb");
    write_file(
        &root.join("smelt.yml"),
        "name: fd_probe\n\
         version: 1\n\
         paths:\n  - models\n\
         targets:\n  dev:\n    type: duckdb\n    database: target/dev.duckdb\n    schema: main\n\
         default_materialization: table\n\
         probes:\n  cadence: per_run\n",
    );
    write_file(
        &root.join("models/sources/raw/subs.yml"),
        "description: Raw subscription rows; pre-loaded by the conformance gate\n\
         name: raw.subs\n\
         columns:\n\
         \x20 - name: customer_id\n    type: INTEGER\n\
         \x20 - name: region\n    type: VARCHAR\n",
    );
    write_file(
        &root.join("models/subscriptions.sql"),
        "---\n\
         materialization: table\n\
         functional_dependencies:\n\
         \x20 - key: [customer_id]\n    determines: region\n\
         ---\n\
         SELECT customer_id, region FROM smelt.sources.raw.subs\n",
    );
    std::fs::create_dir_all(db_path.parent().unwrap())?;
    LinkCProject::load(root, db_path)
}

fn seed_subs(db_path: &Path, violating: bool) -> anyhow::Result<()> {
    let conn = duckdb::Connection::open(db_path)?;
    conn.execute_batch("CREATE SCHEMA IF NOT EXISTS raw;")?;
    if violating {
        conn.execute_batch(
            "CREATE TABLE raw.subs (customer_id INTEGER, region VARCHAR); \
             INSERT INTO raw.subs VALUES (1, 'us'), (1, 'eu'), (2, 'us');",
        )?;
    } else {
        conn.execute_batch(
            "CREATE TABLE raw.subs (customer_id INTEGER, region VARCHAR); \
             INSERT INTO raw.subs VALUES (1, 'us'), (1, 'us'), (2, 'eu');",
        )?;
    }
    Ok(())
}

async fn fd_conforming() -> anyhow::Result<()> {
    let tmp = tempfile::TempDir::new()?;
    let project = stage_fd_project(&tmp)?;
    seed_subs(&project.db_path, false)?;
    project
        .run_quiet("fd-conforming", base_request("dev"))
        .await?;

    let backend = project.backend().await?;
    let maintained = "SELECT customer_id, region FROM main.subscriptions";
    let oracle = "SELECT customer_id, region FROM raw.subs";
    let equal = multiset_equal_via_backend(backend.as_ref(), maintained, oracle).await?;
    anyhow::ensure!(
        equal,
        "fd conforming: maintained output does not match the full-refresh oracle"
    );
    Ok(())
}

async fn fd_violated() -> anyhow::Result<()> {
    let tmp = tempfile::TempDir::new()?;
    let project = stage_fd_project(&tmp)?;
    seed_subs(&project.db_path, true)?;
    let err = project
        .run_quiet("fd-violated", base_request("dev"))
        .await
        .expect_err("a violated functional dependency must fail the run");
    let message = format!("{err:#}");
    anyhow::ensure!(
        message.contains("DeclaredFunctionalDependencyViolated"),
        "expected the named diagnostic, got: {message}"
    );

    let backend = project.backend().await?;
    let exists = backend.table_exists("main", "subscriptions").await?;
    anyhow::ensure!(
        !exists,
        "the probe must fire before any write — main.subscriptions must not exist"
    );
    Ok(())
}

// ---------------------------------------------------------------------
// Recipe: bounded_domain (`e2e/declared_fact_probe_firing.rs::stage_bounded_domain_workspace`)
// ---------------------------------------------------------------------

fn stage_bounded_domain_project(tmp: &tempfile::TempDir) -> anyhow::Result<LinkCProject> {
    let root = tmp.path().join("bounded_domain_probe");
    let db_path = root.join("target/dev.duckdb");
    write_file(
        &root.join("smelt.yml"),
        "name: bounded_domain_probe\n\
         version: 1\n\
         paths:\n  - models\n\
         targets:\n  dev:\n    type: duckdb\n    database: target/dev.duckdb\n    schema: main\n\
         default_materialization: table\n\
         probes:\n  cadence: per_run\n",
    );
    write_file(
        &root.join("models/sources/raw/events.yml"),
        "description: Raw event rows; pre-loaded by the conformance gate\n\
         name: raw.events\n\
         columns:\n\
         \x20 - name: event_ts\n    type: TIMESTAMP\n\
         \x20 - name: country_code\n    type: VARCHAR\n",
    );
    write_file(
        &root.join("models/daily_countries.sql"),
        "---\n\
         materialization: table\n\
         refresh: incremental\n\
         grain: partition\n\
         bounded_domain:\n\
         \x20 column: country_code\n  max_cardinality: 2\n\
         timeseries:\n\
         \x20 event_time_column: event_ts\n  partition_column: event_date\n  granularity: day\n\
         ---\n\
         SELECT CAST(event_ts AS DATE) AS event_date, country_code\n\
         FROM smelt.sources.raw.events\n",
    );
    std::fs::create_dir_all(db_path.parent().unwrap())?;
    LinkCProject::load(root, db_path)
}

fn seed_events(db_path: &Path, violating: bool) -> anyhow::Result<()> {
    let conn = duckdb::Connection::open(db_path)?;
    conn.execute_batch("CREATE SCHEMA IF NOT EXISTS raw;")?;
    conn.execute_batch(
        "CREATE TABLE raw.events (event_ts TIMESTAMP, country_code VARCHAR); \
         INSERT INTO raw.events VALUES \
           (TIMESTAMP '2026-01-01 00:00:00', 'US'), \
           (TIMESTAMP '2026-01-01 01:00:00', 'CA');",
    )?;
    if violating {
        conn.execute_batch(
            "INSERT INTO raw.events VALUES \
               (TIMESTAMP '2026-01-02 00:00:00', 'US'), \
               (TIMESTAMP '2026-01-02 01:00:00', 'CA'), \
               (TIMESTAMP '2026-01-02 02:00:00', 'MX'), \
               (TIMESTAMP '2026-01-02 03:00:00', 'FR');",
        )?;
    } else {
        conn.execute_batch(
            "INSERT INTO raw.events VALUES \
               (TIMESTAMP '2026-01-02 00:00:00', 'US'), \
               (TIMESTAMP '2026-01-02 01:00:00', 'CA');",
        )?;
    }
    Ok(())
}

async fn bounded_domain_conforming() -> anyhow::Result<()> {
    let tmp = tempfile::TempDir::new()?;
    let project = stage_bounded_domain_project(&tmp)?;
    seed_events(&project.db_path, false)?;

    let mut r1 = base_request("dev");
    r1.start = Some("2026-01-01".to_string());
    r1.end = Some("2026-01-02".to_string());
    project.run_quiet("bounded-domain-conforming-1", r1).await?;

    let mut r2 = base_request("dev");
    r2.start = Some("2026-01-02".to_string());
    r2.end = Some("2026-01-03".to_string());
    project.run_quiet("bounded-domain-conforming-2", r2).await?;

    let backend = project.backend().await?;
    let maintained = "SELECT CAST(event_date AS VARCHAR), country_code FROM main.daily_countries";
    let oracle = "SELECT CAST(CAST(event_ts AS DATE) AS VARCHAR), country_code FROM raw.events";
    let equal = multiset_equal_via_backend(backend.as_ref(), maintained, oracle).await?;
    anyhow::ensure!(
        equal,
        "bounded_domain conforming: maintained output does not match the full-refresh oracle"
    );
    Ok(())
}

async fn bounded_domain_violated() -> anyhow::Result<()> {
    let tmp = tempfile::TempDir::new()?;
    let project = stage_bounded_domain_project(&tmp)?;
    seed_events(&project.db_path, true)?;

    let mut r1 = base_request("dev");
    r1.start = Some("2026-01-01".to_string());
    r1.end = Some("2026-01-02".to_string());
    project.run_quiet("bounded-domain-violated-1", r1).await?;

    let backend = project.backend().await?;
    let before = "SELECT CAST(event_date AS VARCHAR), country_code FROM main.daily_countries";
    let before_rows: Vec<Vec<String>> = read_text_rows(backend.as_ref(), before).await?;

    let mut r2 = base_request("dev");
    r2.start = Some("2026-01-02".to_string());
    r2.end = Some("2026-01-03".to_string());
    let err = project
        .run_quiet("bounded-domain-violated-2", r2)
        .await
        .expect_err("a violated bounded domain must fail the batch");
    let message = format!("{err:#}");
    anyhow::ensure!(
        message.contains("DeclaredBoundedDomainExceeded"),
        "expected the named diagnostic, got: {message}"
    );

    let after_rows: Vec<Vec<String>> = read_text_rows(backend.as_ref(), before).await?;
    anyhow::ensure!(
        before_rows == after_rows,
        "the target's pre-run contents must be unchanged after a refused batch"
    );
    Ok(())
}

async fn read_text_rows(backend: &dyn Backend, sql: &str) -> anyhow::Result<Vec<Vec<String>>> {
    let batches = backend
        .execute_sql(sql)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let rows = smelt_runtime::check_runner::batches_to_rows(&batches);
    let mut out: Vec<Vec<String>> = rows
        .into_iter()
        .map(|r| {
            let mut vals: Vec<(String, String)> = r.into_iter().collect();
            vals.sort_by(|a, b| a.0.cmp(&b.0));
            vals.into_iter().map(|(_, v)| v).collect()
        })
        .collect();
    out.sort();
    Ok(out)
}

// ---------------------------------------------------------------------
// Recipe: mutation_profile.kind: append_only
// (`e2e/declared_fact_probe_firing.rs::stage_append_only_workspace`)
// ---------------------------------------------------------------------

fn stage_append_only_project(tmp: &tempfile::TempDir) -> anyhow::Result<LinkCProject> {
    let root = tmp.path().join("append_only_probe");
    let db_path = root.join("target/dev.duckdb");
    write_file(
        &root.join("smelt.yml"),
        "name: append_only_probe\n\
         version: 1\n\
         paths:\n  - models\n\
         targets:\n  dev:\n    type: duckdb\n    database: target/dev.duckdb\n    schema: main\n\
         default_materialization: table\n\
         probes:\n  cadence: per_run\n",
    );
    write_file(
        &root.join("models/sources/raw/clicks.yml"),
        "description: Raw append-only clickstream rows; pre-loaded by the conformance gate\n\
         name: raw.clicks\n\
         mutation_profile:\n  kind: append_only\n\
         timeseries:\n\
         \x20 event_time_column: click_ts\n  partition_column: click_date\n  granularity: day\n\
         columns:\n\
         \x20 - name: click_ts\n    type: TIMESTAMP\n\
         \x20 - name: click_date\n    type: DATE\n\
         \x20 - name: payload\n    type: VARCHAR\n",
    );
    write_file(
        &root.join("models/clicks_summary.sql"),
        "---\n\
         materialization: table\n\
         ---\n\
         SELECT click_date, payload FROM smelt.sources.raw.clicks\n",
    );
    std::fs::create_dir_all(db_path.parent().unwrap())?;
    LinkCProject::load(root, db_path)
}

fn seed_clicks(db_path: &Path) -> anyhow::Result<()> {
    let conn = duckdb::Connection::open(db_path)?;
    conn.execute_batch("CREATE SCHEMA IF NOT EXISTS raw;")?;
    conn.execute_batch(
        "CREATE TABLE raw.clicks (click_ts TIMESTAMP, click_date DATE, payload VARCHAR); \
         INSERT INTO raw.clicks VALUES \
           (TIMESTAMP '2026-01-01 00:00:00', DATE '2026-01-01', 'a'), \
           (TIMESTAMP '2026-01-02 00:00:00', DATE '2026-01-02', 'b');",
    )?;
    Ok(())
}

fn mutate_closed_partition(db_path: &Path) -> anyhow::Result<()> {
    let conn = duckdb::Connection::open(db_path)?;
    conn.execute_batch(
        "UPDATE raw.clicks SET payload = 'mutated' WHERE click_date = DATE '2026-01-01';",
    )?;
    Ok(())
}

async fn append_only_conforming() -> anyhow::Result<()> {
    let tmp = tempfile::TempDir::new()?;
    let project = stage_append_only_project(&tmp)?;
    seed_clicks(&project.db_path)?;

    // First run: no recorded baseline yet — establishes it.
    project
        .run_quiet("append-only-conforming-1", base_request("dev"))
        .await?;
    // Second run: nothing mutated — the recorded baseline verifies clean.
    project
        .run_quiet("append-only-conforming-2", base_request("dev"))
        .await?;

    let backend = project.backend().await?;
    let maintained = "SELECT CAST(click_date AS VARCHAR), payload FROM main.clicks_summary";
    let oracle = "SELECT CAST(click_date AS VARCHAR), payload FROM raw.clicks";
    let equal = multiset_equal_via_backend(backend.as_ref(), maintained, oracle).await?;
    anyhow::ensure!(
        equal,
        "append_only conforming: maintained output does not match the full-refresh oracle"
    );
    Ok(())
}

async fn append_only_violated() -> anyhow::Result<()> {
    let tmp = tempfile::TempDir::new()?;
    let project = stage_append_only_project(&tmp)?;
    seed_clicks(&project.db_path)?;

    project
        .run_quiet("append-only-violated-1", base_request("dev"))
        .await?;

    let backend = project.backend().await?;
    let snapshot_sql = "SELECT CAST(click_date AS VARCHAR), payload FROM main.clicks_summary";
    let after_first = read_text_rows(backend.as_ref(), snapshot_sql).await?;

    mutate_closed_partition(&project.db_path)?;

    let err = project
        .run_quiet("append-only-violated-2", base_request("dev"))
        .await
        .expect_err("a mutated closed partition must fail the second run");
    let message = format!("{err:#}");
    anyhow::ensure!(
        message.contains("SourceMutationProfileViolated"),
        "expected the named diagnostic, got: {message}"
    );

    let after_second = read_text_rows(backend.as_ref(), snapshot_sql).await?;
    anyhow::ensure!(
        after_first == after_second,
        "the model table must be unchanged after a refused second run"
    );
    Ok(())
}

// ---------------------------------------------------------------------
// Recipe: timeseries.assert_monotonic
// (starting point: `crates/smelt-runtime/tests/model_probes.rs`)
// ---------------------------------------------------------------------

fn monotonic_timeseries() -> TimeseriesConfig {
    TimeseriesConfig {
        event_time_column: "event_time".to_string(),
        // `user_id` — a real, constant-per-key output column, not
        // `event_time` itself. `declared_model_probes` falls back to
        // `[ts.partition_column]` as the monotonicity probe's `PARTITION
        // BY` key when no `unique_key` is declared; naming the event-time
        // column itself here would make every row its own singleton
        // partition (each row's own event_time value differs), so no `LAG`
        // predecessor within a partition would ever exist and a genuine
        // out-of-order violation would go undetected.
        partition_column: "user_id".to_string(),
        granularity: Granularity::Day,
        week_start: None,
        assert_monotonic: true,
    }
}

async fn monotonicity_conforming() -> anyhow::Result<()> {
    let tmp = tempfile::TempDir::new()?;
    let db_path = tmp.path().join("monotonic.duckdb");
    let backend = DuckDbBackend::new(&db_path, "main")
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    backend
        .execute_sql(
            "CREATE TABLE main.events_raw (user_id INT, event_time TIMESTAMP); \
             INSERT INTO main.events_raw VALUES \
               (1, TIMESTAMP '2026-01-01 00:00:00'), \
               (1, TIMESTAMP '2026-01-01 01:00:00');",
        )
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    let ts = monotonic_timeseries();
    let select = "SELECT user_id, event_time FROM main.events_raw";
    let probes = declared_model_probes(
        "main.monotonic_events",
        "main.monotonic_events creation",
        None,
        Some(&ts),
        select,
        MaintenanceDialect::DuckDb,
    );
    dispatch_declared_model_probes(&backend, &ProbePolicy::per_run(), &probes)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    backend
        .execute_sql(&format!("CREATE TABLE main.monotonic_events AS {select}"))
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    let maintained = "SELECT user_id, event_time FROM main.monotonic_events";
    let equal = multiset_equal_via_backend(&backend, maintained, select).await?;
    anyhow::ensure!(
        equal,
        "monotonicity conforming: maintained output does not match the full-refresh oracle"
    );
    Ok(())
}

async fn monotonicity_violated() -> anyhow::Result<()> {
    let tmp = tempfile::TempDir::new()?;
    let db_path = tmp.path().join("monotonic.duckdb");
    let backend = DuckDbBackend::new(&db_path, "main")
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    backend
        .execute_sql(
            "CREATE TABLE main.events_raw (user_id INT, event_time TIMESTAMP); \
             INSERT INTO main.events_raw VALUES \
               (1, TIMESTAMP '2026-01-01 01:00:00'), \
               (1, TIMESTAMP '2026-01-01 00:00:00');",
        )
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    let ts = monotonic_timeseries();
    let select = "SELECT user_id, event_time FROM main.events_raw";
    let probes = declared_model_probes(
        "main.monotonic_events",
        "main.monotonic_events creation",
        None,
        Some(&ts),
        select,
        MaintenanceDialect::DuckDb,
    );
    let err = dispatch_declared_model_probes(&backend, &ProbePolicy::per_run(), &probes)
        .await
        .expect_err("a violated monotonicity declaration must fail loud");
    anyhow::ensure!(
        err.to_string().contains("DeclaredMonotonicityViolated"),
        "expected the named diagnostic, got: {err}"
    );

    let exists = Backend::table_exists(&backend, "main", "monotonic_events")
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    anyhow::ensure!(
        !exists,
        "the probe must fire before any write — main.monotonic_events must not exist"
    );
    Ok(())
}

// ---------------------------------------------------------------------
// Recipe: referential_integrity
// (starting point: `crates/smelt-runtime/tests/delta_restricted_recompute.rs`,
// `crates/smelt-cli/tests/e2e/events_deduped_redelivery_equivalence.rs`)
//
// The declared-`referential_integrity` route's only live production
// dispatch site is `execute_delete_insert_with_delta_restriction`'s
// model-edge delta restriction (`docs/outcomes/20260809-probe-backed-facts/
// outcome.md` phase 3 decision log: "Runtime dispatch reachability for the
// declared route stays scoped to the model-edge call site"). This recipe
// drives that function directly against a real DuckDB backend, mirroring
// `delta_restricted_recompute.rs`'s own fixture shape, rather than staging a
// full `smelt.yml` project — the same accommodation
// `crates/smelt-runtime/tests/locality_route3_recurrence_check.rs` makes for
// `key_recurrence` below.
// ---------------------------------------------------------------------

const CP_UPSTREAM_MODEL: &str = "silver.fact";
const CP_WINDOW_START: &str = "2026-07-01";
const CP_WINDOW_END: &str = "2026-07-02";

async fn cp_setup(matched_all: bool) -> anyhow::Result<(tempfile::TempDir, DuckDbBackend)> {
    let tmp = tempfile::TempDir::new()?;
    let db_path = tmp.path().join("cp.duckdb");
    let backend = DuckDbBackend::new(&db_path, "main")
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    // Baseline maintained output: ev-1/ev-2 stale ('OLD', about to be
    // touched by this run's delta); ev-3/ev-4 already converged ('NEW').
    backend
        .execute_sql(
            "CREATE TABLE main.enriched (event_id VARCHAR, event_date DATE, tier VARCHAR); \
             INSERT INTO main.enriched VALUES \
               ('ev-1', '2026-07-01', 'OLD'), \
               ('ev-2', '2026-07-01', 'OLD'), \
               ('ev-3', '2026-07-01', 'NEW'), \
               ('ev-4', '2026-07-01', 'NEW');",
        )
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    backend
        .execute_sql(
            "CREATE TABLE main.fact_recompute (event_id VARCHAR, event_date DATE); \
             INSERT INTO main.fact_recompute VALUES \
               ('ev-1', '2026-07-01'), ('ev-2', '2026-07-01'), \
               ('ev-3', '2026-07-01'), ('ev-4', '2026-07-01');",
        )
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    backend
        .execute_sql("CREATE TABLE main.dim (event_id VARCHAR)")
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    if matched_all {
        backend
            .execute_sql("INSERT INTO main.dim VALUES ('ev-1'), ('ev-2'), ('ev-3'), ('ev-4')")
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;
    } else {
        // ev-3's dimension key is missing anywhere in the table — a
        // globally broken referential_integrity, not merely a stale
        // key inside the delta.
        backend
            .execute_sql("INSERT INTO main.dim VALUES ('ev-1'), ('ev-2'), ('ev-4')")
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;
    }

    Ok((tmp, backend))
}

async fn cp_record_delta(backend: &DuckDbBackend) -> anyhow::Result<()> {
    let ensure = smelt_state::ddl_duckdb::generate_observed_delta_table_ddl("main");
    backend
        .execute_sql(&ensure)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let changed_keys_query =
        "SELECT * FROM (VALUES ('ev-1', NULL), ('ev-2', NULL)) AS t(delta_key, delta_partition)";
    let upsert = smelt_state::ddl_duckdb::generate_observed_delta_upsert_sql(
        "main",
        CP_UPSTREAM_MODEL,
        CP_WINDOW_START,
        CP_WINDOW_END,
        changed_keys_query,
    );
    backend
        .execute_sql(&upsert)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(())
}

fn cp_region() -> smelt_logical::maintenance::emit::Region {
    smelt_logical::maintenance::emit::Region {
        start: format!("'{CP_WINDOW_START}'"),
        end: format!("'{CP_WINDOW_END}'"),
    }
}

fn cp_body() -> &'static str {
    "SELECT f.event_id, f.event_date, 'NEW' AS tier FROM main.fact_recompute f \
     JOIN main.dim d ON f.event_id = d.event_id"
}

fn cp_closure() -> SkeletonSourceClosure {
    SkeletonSourceClosure::Closed {
        row_preservation: RowPreservation::DeclaredReferentialIntegrity {
            source: "dim".to_string(),
        },
    }
}

async fn cp_tiers(backend: &DuckDbBackend) -> anyhow::Result<Vec<(String, String)>> {
    let batches = backend
        .execute_sql("SELECT event_id, tier FROM main.enriched ORDER BY event_id")
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let rows = smelt_runtime::check_runner::batches_to_rows(&batches);
    Ok(rows
        .into_iter()
        .map(|r| {
            (
                r.get("event_id").cloned().unwrap_or_default(),
                r.get("tier").cloned().unwrap_or_default(),
            )
        })
        .collect())
}

async fn count_preservation_conforming() -> anyhow::Result<()> {
    let (_tmp, backend) = cp_setup(true).await?;
    cp_record_delta(&backend).await?;

    execute_delete_insert_with_delta_restriction(
        &backend,
        "main",
        "enriched",
        "event_date",
        &cp_region(),
        cp_body(),
        cp_body(),
        Some("event_id"),
        Some(&cp_closure()),
        RestrictionDeltaSource::ModelEdge {
            upstream_model: CP_UPSTREAM_MODEL,
            window_start: CP_WINDOW_START,
            window_end: CP_WINDOW_END,
        },
        None,
        MaintenanceDialect::DuckDb,
        &no_retry_policy(),
        &ProbePolicy::per_run(),
    )
    .await
    .map_err(|e| anyhow::anyhow!("{e}"))?;

    let maintained = "SELECT event_id, tier FROM main.enriched";
    let oracle =
        "SELECT f.event_id, 'NEW' AS tier FROM main.fact_recompute f JOIN main.dim d ON f.event_id = d.event_id";
    let equal = multiset_equal_via_backend(&backend, maintained, oracle).await?;
    anyhow::ensure!(
        equal,
        "referential_integrity conforming: maintained output does not match the \
         full-refresh oracle"
    );
    Ok(())
}

/// `body`/`probe_body` are the two different strings a real compile
/// produces (`docs/plans/20260819-source-derived-projection.md` Phase 5):
/// `body` here is genuinely type-cast-wrapped, matching production
/// shape (`CompiledModel::sql`, `apply_type_casts`'s
/// `SELECT CAST(..) FROM ( <body> ) _smelt_typed` form) — not the
/// hand-written unwrapped fixture every other recipe in this file feeds.
/// `probe_body` is the pre-wrap body (`CompiledModel::body_sql`) the
/// count-preservation probe reads its enrichment join from. Before this
/// phase, the only body ever threaded to the probe was the wrapped one,
/// which buries the join inside a derived table the probe never looked
/// inside — the probe silently found nothing, dropped the delta
/// restriction, and fell back to the widened scan on every run.
#[tokio::test]
async fn count_preservation_conforming_with_a_cast_wrapped_body_still_restricts(
) -> anyhow::Result<()> {
    let (_tmp, backend) = cp_setup(true).await?;
    cp_record_delta(&backend).await?;

    let wrapped_body = smelt_dialect::wrap_with_type_casts(
        cp_body(),
        &["event_id", "event_date", "tier"],
        &[
            smelt_types::DataType::Text,
            smelt_types::DataType::Date,
            smelt_types::DataType::Text,
        ],
        smelt_dialect::SqlDialect::DuckDB,
    );

    let group = execute_delete_insert_with_delta_restriction(
        &backend,
        "main",
        "enriched",
        "event_date",
        &cp_region(),
        &wrapped_body,
        cp_body(),
        Some("event_id"),
        Some(&cp_closure()),
        RestrictionDeltaSource::ModelEdge {
            upstream_model: CP_UPSTREAM_MODEL,
            window_start: CP_WINDOW_START,
            window_end: CP_WINDOW_END,
        },
        None,
        MaintenanceDialect::DuckDb,
        &no_retry_policy(),
        &ProbePolicy::per_run(),
    )
    .await
    .map_err(|e| anyhow::anyhow!("{e}"))?;

    // The delta restriction was actually applied — a declared
    // `referential_integrity` closure whose count-preservation probe never
    // located the join falls back silently to the widened scan instead,
    // which carries no `IN (...)` restriction predicate at all. Asserting
    // on the emitted statement (not merely on the resulting row contents,
    // which a widened scan over this conforming fixture would also get
    // right) is what actually pins the probe having found the join.
    let insert = group
        .statements
        .iter()
        .find(|s| s.sql.starts_with("INSERT INTO main.enriched"))
        .expect("an INSERT statement was emitted");
    assert!(
        insert.sql.contains("event_id IN ("),
        "expected the delta restriction to be applied (a count-preservation probe that found \
         the join inside the cast-wrapped body), got: {}",
        insert.sql
    );
    assert!(insert.sql.contains("'ev-1'"), "{}", insert.sql);
    assert!(insert.sql.contains("'ev-2'"), "{}", insert.sql);

    let maintained = "SELECT event_id, tier FROM main.enriched";
    let oracle =
        "SELECT f.event_id, 'NEW' AS tier FROM main.fact_recompute f JOIN main.dim d ON f.event_id = d.event_id";
    let equal = multiset_equal_via_backend(&backend, maintained, oracle).await?;
    anyhow::ensure!(
        equal,
        "referential_integrity conforming (cast-wrapped body): maintained output does not \
         match the full-refresh oracle"
    );
    Ok(())
}

async fn count_preservation_violated() -> anyhow::Result<()> {
    let (_tmp, backend) = cp_setup(false).await?;
    cp_record_delta(&backend).await?;
    let before = cp_tiers(&backend).await?;

    let err = execute_delete_insert_with_delta_restriction(
        &backend,
        "main",
        "enriched",
        "event_date",
        &cp_region(),
        cp_body(),
        cp_body(),
        Some("event_id"),
        Some(&cp_closure()),
        RestrictionDeltaSource::ModelEdge {
            upstream_model: CP_UPSTREAM_MODEL,
            window_start: CP_WINDOW_START,
            window_end: CP_WINDOW_END,
        },
        None,
        MaintenanceDialect::DuckDb,
        &no_retry_policy(),
        &ProbePolicy::per_run(),
    )
    .await
    .expect_err("a broken referential_integrity must refuse the delta-restricted recompute");
    let message = err.to_string();
    anyhow::ensure!(
        message.contains("SourceCountPreservationViolated"),
        "expected the named diagnostic, got: {message}"
    );

    let after = cp_tiers(&backend).await?;
    anyhow::ensure!(
        before == after,
        "the target's pre-run contents must be unchanged after a refused delta restriction"
    );
    Ok(())
}

async fn count_preservation_violated_probes_off() -> anyhow::Result<()> {
    let (_tmp, backend) = cp_setup(false).await?;
    cp_record_delta(&backend).await?;

    execute_delete_insert_with_delta_restriction(
        &backend,
        "main",
        "enriched",
        "event_date",
        &cp_region(),
        cp_body(),
        cp_body(),
        Some("event_id"),
        Some(&cp_closure()),
        RestrictionDeltaSource::ModelEdge {
            upstream_model: CP_UPSTREAM_MODEL,
            window_start: CP_WINDOW_START,
            window_end: CP_WINDOW_END,
        },
        None,
        MaintenanceDialect::DuckDb,
        &no_retry_policy(),
        &ProbePolicy::new(ProbeCadence::Off, 0),
    )
    .await
    .map_err(|e| anyhow::anyhow!("{e}"))
    .expect("probes: {cadence: off} must let the delta-restricted recompute write");

    let maintained = "SELECT event_id, tier FROM main.enriched";
    let oracle =
        "SELECT f.event_id, 'NEW' AS tier FROM main.fact_recompute f JOIN main.dim d ON f.event_id = d.event_id";
    let equal = multiset_equal_via_backend(&backend, maintained, oracle).await?;
    anyhow::ensure!(
        !equal,
        "expected the probes-off write to diverge from the full-refresh oracle (a stale, \
         no-longer-matching ev-3 row left behind by the untouched restriction) — got equal \
         output, meaning the declaration was not actually load-bearing for this recipe"
    );
    Ok(())
}

// ---------------------------------------------------------------------
// Recipe: key_recurrence
// (starting point: `crates/smelt-runtime/tests/locality_route3_recurrence_check.rs`)
// ---------------------------------------------------------------------

fn kr_unconditional() -> WriteSuppression {
    WriteSuppression::Unconditional {
        why: "conformance recipe exercises route-3 checked-merge behaviour, not suppression"
            .to_string(),
    }
}

fn kr_timeseries() -> TimeseriesConfig {
    TimeseriesConfig {
        event_time_column: "event_ts".to_string(),
        partition_column: "event_date".to_string(),
        granularity: Granularity::Day,
        week_start: None,
        assert_monotonic: false,
    }
}

fn kr_classification() -> CumulativeClassification {
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
            timeseries: Some(kr_timeseries()),
        },
    }
}

fn kr_checked_slice() -> LocalitySlice {
    LocalitySlice::RecurrenceBounded {
        partition_column: "last_seen_date".to_string(),
        margin_before: Seconds::days(3),
        margin_after: Seconds::ZERO,
        r: Seconds::days(3),
    }
}

async fn kr_setup_backend(db_path: &Path) -> anyhow::Result<DuckDbBackend> {
    let conn = duckdb::Connection::open(db_path)?;
    conn.execute_batch(
        "CREATE SCHEMA IF NOT EXISTS main; \
         CREATE TABLE main.raw_events (event_id INTEGER, event_ts TIMESTAMP, event_date DATE);",
    )?;
    drop(conn);
    DuckDbBackend::new(db_path, "main")
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))
}

async fn kr_insert(backend: &DuckDbBackend, event_id: i64, date: &str) -> anyhow::Result<()> {
    backend
        .execute_sql(&format!(
            "INSERT INTO main.raw_events VALUES ({event_id}, TIMESTAMP '{date} 00:00:00', DATE '{date}')"
        ))
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(())
}

fn kr_compile_step(
    step: &smelt_runtime::maintenance_driver::MaintenanceStep,
) -> anyhow::Result<String> {
    Ok(format!(
        "SELECT event_id, MAX(event_date) AS last_seen_date FROM main.raw_events \
         WHERE event_date = '{}' GROUP BY event_id",
        step.partition_value
    ))
}

async fn kr_last_seen(backend: &DuckDbBackend, event_id: i64) -> anyhow::Result<Option<String>> {
    let batches = backend
        .execute_sql(&format!(
            "SELECT CAST(last_seen_date AS VARCHAR) AS v FROM main.events_last_seen \
             WHERE event_id = {event_id}"
        ))
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let rows = smelt_runtime::check_runner::batches_to_rows(&batches);
    Ok(rows.first().and_then(|r| r.get("v")).cloned())
}

async fn recurrence_conforming() -> anyhow::Result<()> {
    let tmp = tempfile::TempDir::new()?;
    let db_path = tmp.path().join("dev.duckdb");
    let backend = kr_setup_backend(&db_path).await?;

    kr_insert(&backend, 1, "2026-01-01").await?;
    kr_insert(&backend, 1, "2026-01-02").await?; // in-bound: 1 day, r=3 days

    let steps = driving_steps("2026-01-01", "2026-01-03", &Granularity::Day)?;
    run_windowed_keyed_maintenance(
        &backend,
        "events_last_seen",
        "main",
        "events_last_seen",
        &steps,
        &kr_classification(),
        Some(&kr_checked_slice()),
        &kr_unconditional(),
        None,
        kr_compile_step,
        &no_retry_policy(),
        &ProbePolicy::per_run(),
    )
    .await
    .map_err(|e| anyhow::anyhow!("{e}"))?;

    let maintained =
        "SELECT event_id, CAST(last_seen_date AS VARCHAR) AS v FROM main.events_last_seen";
    let oracle =
        "SELECT event_id, CAST(MAX(event_date) AS VARCHAR) AS v FROM main.raw_events GROUP BY event_id";
    let equal = multiset_equal_via_backend(&backend, maintained, oracle).await?;
    anyhow::ensure!(
        equal,
        "key_recurrence conforming: maintained output does not match the full-refresh oracle"
    );
    Ok(())
}

async fn recurrence_violated() -> anyhow::Result<()> {
    let tmp = tempfile::TempDir::new()?;
    let db_path = tmp.path().join("dev.duckdb");
    let backend = kr_setup_backend(&db_path).await?;

    kr_insert(&backend, 1, "2026-01-01").await?;
    let create_steps = driving_steps("2026-01-01", "2026-01-02", &Granularity::Day)?;
    run_windowed_keyed_maintenance(
        &backend,
        "events_last_seen",
        "main",
        "events_last_seen",
        &create_steps,
        &kr_classification(),
        Some(&kr_checked_slice()),
        &kr_unconditional(),
        None,
        kr_compile_step,
        &no_retry_policy(),
        &ProbePolicy::per_run(),
    )
    .await
    .map_err(|e| anyhow::anyhow!("{e}"))?;

    kr_insert(&backend, 1, "2026-01-06").await?; // 5 days after day 1, outside r=3
    let violating_steps = driving_steps("2026-01-06", "2026-01-07", &Granularity::Day)?;
    let err = run_windowed_keyed_maintenance(
        &backend,
        "events_last_seen",
        "main",
        "events_last_seen",
        &violating_steps,
        &kr_classification(),
        Some(&kr_checked_slice()),
        &kr_unconditional(),
        None,
        kr_compile_step,
        &no_retry_policy(),
        &ProbePolicy::per_run(),
    )
    .await
    .expect_err("an out-of-bound redelivery must refuse the run");
    let message = err.to_string();
    anyhow::ensure!(
        message.contains("KeyedRecurrenceBoundViolated"),
        "expected the named diagnostic, got: {message}"
    );

    let after = kr_last_seen(&backend, 1).await?;
    anyhow::ensure!(
        after.as_deref() == Some("2026-01-01"),
        "the target must be unchanged after the checked probe refuses the run, got {after:?}"
    );
    Ok(())
}

async fn recurrence_violated_probes_off() -> anyhow::Result<()> {
    let tmp = tempfile::TempDir::new()?;
    let db_path = tmp.path().join("dev.duckdb");
    let backend = kr_setup_backend(&db_path).await?;

    kr_insert(&backend, 1, "2026-01-01").await?;
    let create_steps = driving_steps("2026-01-01", "2026-01-02", &Granularity::Day)?;
    run_windowed_keyed_maintenance(
        &backend,
        "events_last_seen",
        "main",
        "events_last_seen",
        &create_steps,
        &kr_classification(),
        Some(&kr_checked_slice()),
        &kr_unconditional(),
        None,
        kr_compile_step,
        &no_retry_policy(),
        &ProbePolicy::per_run(),
    )
    .await
    .map_err(|e| anyhow::anyhow!("{e}"))?;

    kr_insert(&backend, 1, "2026-01-06").await?;
    let violating_steps = driving_steps("2026-01-06", "2026-01-07", &Granularity::Day)?;
    run_windowed_keyed_maintenance(
        &backend,
        "events_last_seen",
        "main",
        "events_last_seen",
        &violating_steps,
        &kr_classification(),
        Some(&kr_checked_slice()),
        &kr_unconditional(),
        None,
        kr_compile_step,
        &no_retry_policy(),
        &ProbePolicy::new(ProbeCadence::Off, 0),
    )
    .await
    .expect("probes: {cadence: off} must let the out-of-bound redelivery merge write");

    let maintained = kr_last_seen(&backend, 1).await?;
    let oracle_batches = backend
        .execute_sql(
            "SELECT CAST(MAX(event_date) AS VARCHAR) AS v FROM main.raw_events WHERE event_id = 1",
        )
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let oracle_rows = smelt_runtime::check_runner::batches_to_rows(&oracle_batches);
    let oracle = oracle_rows.first().and_then(|r| r.get("v")).cloned();

    anyhow::ensure!(
        maintained != oracle,
        "expected the probes-off write to diverge from the full-refresh oracle (a stale \
         last_seen_date left behind by the recurrence-slice-restricted merge) — got maintained \
         == oracle == {maintained:?}, meaning the declaration was not actually load-bearing for \
         this recipe"
    );
    Ok(())
}

// ---------------------------------------------------------------------
// The four standing tests over the pool
// ---------------------------------------------------------------------

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/smelt-cli has a parent dir")
        .parent()
        .expect("crates/ has a parent dir")
        .to_path_buf()
}

fn read(rel: &str) -> String {
    let path = repo_root().join(rel);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()))
}

/// Same extraction shape as `crates/smelt-logical/tests/probe_obligation.rs`.
fn probe_obligation_section(model_properties: &str) -> &str {
    let start = model_properties
        .find("### Probe obligation")
        .expect("model_properties.md must have a §\"Probe obligation\" heading");
    let after_heading = &model_properties[start..];
    let body_start = after_heading
        .find('\n')
        .map(|i| i + 1)
        .unwrap_or(after_heading.len());
    let body = &after_heading[body_start..];
    let end = body
        .find("\n## ")
        .or_else(|| body.find("\n### "))
        .unwrap_or(body.len());
    &body[..end]
}

fn registry_rows(section: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut seen_separator = false;
    for line in section.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with('|') {
            continue;
        }
        if trimmed.chars().all(|c| "|-: ".contains(c)) {
            seen_separator = true;
            continue;
        }
        if !seen_separator {
            continue;
        }
        let cells: Vec<String> = trimmed
            .trim_matches('|')
            .split('|')
            .map(|c| c.trim().to_string())
            .collect();
        rows.push(cells);
    }
    rows
}

/// Every `built` row's Diagnostic cell, from the live spec table.
fn built_diagnostics() -> Vec<String> {
    let model_properties = read("docs/specs/model_properties.md");
    let section = probe_obligation_section(&model_properties);
    registry_rows(section)
        .into_iter()
        .filter(|row| row.last().map(String::as_str) == Some("built"))
        .map(|row| {
            row.get(4)
                .map(|d| d.trim_matches('`').to_string())
                .unwrap_or_default()
        })
        .filter(|d| !d.is_empty())
        .collect()
}

#[test]
fn every_built_registry_row_has_a_violation_recipe() {
    let spec_diagnostics = built_diagnostics();
    let pool = recipes();
    let pool_diagnostics: Vec<&str> = pool.iter().map(|r| r.diagnostic).collect();

    for diagnostic in &spec_diagnostics {
        assert!(
            pool_diagnostics.contains(&diagnostic.as_str()),
            "registry row with diagnostic `{diagnostic}` is `built` but has no recipe in \
             fact_violations.rs's pool: {pool_diagnostics:?}"
        );
    }
    for diagnostic in &pool_diagnostics {
        assert!(
            spec_diagnostics.iter().any(|d| d == diagnostic),
            "recipe names diagnostic `{diagnostic}`, which is not a `built` row in \
             docs/specs/model_properties.md §\"Probe obligation\": {spec_diagnostics:?}"
        );
    }
}

#[tokio::test]
async fn conforming_data_runs_clean_and_matches_the_oracle() {
    for recipe in recipes() {
        (recipe.conforming)()
            .await
            .unwrap_or_else(|e| panic!("recipe {:?} conforming leg failed: {e:#}", recipe.name));
    }
}

#[tokio::test]
async fn violated_fact_fails_before_any_write() {
    for recipe in recipes() {
        (recipe.violated)()
            .await
            .unwrap_or_else(|e| panic!("recipe {:?} violated leg failed: {e:#}", recipe.name));
    }
}

#[tokio::test]
async fn violation_is_end_state_observable_when_probes_are_off() {
    let mut observed = 0;
    let mut skipped = 0;
    for recipe in recipes() {
        match recipe.observability {
            Observability::Observable(f) => {
                f().await.unwrap_or_else(|e| {
                    panic!(
                        "recipe {:?} probes-off observability leg failed: {e:#}",
                        recipe.name
                    )
                });
                observed += 1;
            }
            Observability::NotObservable(reason) => {
                eprintln!(
                    "SKIP {:?} (diagnostic {:?}): not end-state observable in this recipe's \
                     staged shape — {reason}",
                    recipe.name, recipe.diagnostic
                );
                skipped += 1;
            }
        }
    }
    assert!(
        observed > 0,
        "expected at least one recipe to be end-state observable — got {observed} observed, \
         {skipped} skipped"
    );
}
