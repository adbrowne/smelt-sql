//! Real-DuckDB coverage for `ModelRunRecord.probes` population
//! (`docs/specs/run_state.md` §"Run manifest",
//! `docs/outcomes/20260809-probe-backed-facts/phases/08-plan.md`): every
//! live dispatch site (`model_probes`, `source_probes`) writes its held/
//! skipped outcome onto the run's manifest entry rather than leaving it
//! `Vec::new()`. Stages the same fixture shapes
//! `crates/smelt-cli/tests/maintenance_conformance/fact_violations.rs`
//! already proves probe-firing over.

use std::path::Path;

use smelt_maintenance_testkit::link_c_harness::{base_request, LinkCProject};
use smelt_state::ProbeRecordOutcome;

fn write_file(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, content).unwrap();
}

fn stage_fd_project(tmp: &tempfile::TempDir, cadence: &str) -> anyhow::Result<LinkCProject> {
    let root = tmp.path().join("fd_manifest");
    let db_path = root.join("target/dev.duckdb");
    write_file(
        &root.join("smelt.yml"),
        &format!(
            "name: fd_manifest\n\
             version: 1\n\
             paths:\n  - models\n\
             targets:\n  dev:\n    type: duckdb\n    database: target/dev.duckdb\n    schema: main\n\
             default_materialization: table\n\
             probes:\n  cadence: {cadence}\n"
        ),
    );
    write_file(
        &root.join("models/sources/raw/subs.yml"),
        "description: Raw subscription rows; pre-loaded by the test\n\
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

fn seed_subs(db_path: &Path) -> anyhow::Result<()> {
    let conn = duckdb::Connection::open(db_path)?;
    conn.execute_batch(
        "CREATE SCHEMA IF NOT EXISTS raw; \
         CREATE TABLE raw.subs (customer_id INTEGER, region VARCHAR); \
         INSERT INTO raw.subs VALUES (1, 'us'), (1, 'us'), (2, 'eu');",
    )?;
    Ok(())
}

#[tokio::test]
async fn manifest_records_dispatched_model_probes() -> anyhow::Result<()> {
    let tmp = tempfile::TempDir::new()?;
    let project = stage_fd_project(&tmp, "per_run")?;
    seed_subs(&project.db_path)?;

    let outcome = project
        .run_quiet("fd-manifest-dispatched", base_request("dev"))
        .await?;

    let record = outcome
        .models
        .get("subscriptions")
        .expect("subscriptions must have a manifest entry");
    let probe = record
        .probes
        .iter()
        .find(|p| p.fact == "functional_dependencies:")
        .unwrap_or_else(|| {
            panic!(
                "expected a functional_dependencies probe record, got {:?}",
                record.probes
            )
        });
    assert_eq!(probe.probe, "DeclaredFunctionalDependencyViolated");
    assert_eq!(probe.outcome, ProbeRecordOutcome::Dispatched);
    Ok(())
}

#[tokio::test]
async fn manifest_records_skipped_probes_under_cadence_off() -> anyhow::Result<()> {
    let tmp = tempfile::TempDir::new()?;
    let project = stage_fd_project(&tmp, "off")?;
    seed_subs(&project.db_path)?;

    let outcome = project
        .run_quiet("fd-manifest-skipped", base_request("dev"))
        .await?;

    let record = outcome
        .models
        .get("subscriptions")
        .expect("subscriptions must have a manifest entry");
    let probe = record
        .probes
        .iter()
        .find(|p| p.fact == "functional_dependencies:")
        .unwrap_or_else(|| {
            panic!(
                "expected a functional_dependencies probe record, got {:?}",
                record.probes
            )
        });
    assert_eq!(probe.probe, "DeclaredFunctionalDependencyViolated");
    assert_eq!(
        probe.outcome,
        ProbeRecordOutcome::Skipped,
        "cadence off trusts the declaration and records it unverified, not empty"
    );
    Ok(())
}

fn stage_append_only_project(tmp: &tempfile::TempDir) -> anyhow::Result<LinkCProject> {
    let root = tmp.path().join("append_only_manifest");
    let db_path = root.join("target/dev.duckdb");
    write_file(
        &root.join("smelt.yml"),
        "name: append_only_manifest\n\
         version: 1\n\
         paths:\n  - models\n\
         targets:\n  dev:\n    type: duckdb\n    database: target/dev.duckdb\n    schema: main\n\
         default_materialization: table\n\
         probes:\n  cadence: per_run\n",
    );
    write_file(
        &root.join("models/sources/raw/clicks.yml"),
        "description: Raw append-only clickstream rows; pre-loaded by the test\n\
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
    conn.execute_batch(
        "CREATE SCHEMA IF NOT EXISTS raw; \
         CREATE TABLE raw.clicks (click_ts TIMESTAMP, click_date DATE, payload VARCHAR); \
         INSERT INTO raw.clicks VALUES \
           (TIMESTAMP '2026-01-01 00:00:00', DATE '2026-01-01', 'a'), \
           (TIMESTAMP '2026-01-02 00:00:00', DATE '2026-01-02', 'b');",
    )?;
    Ok(())
}

#[tokio::test]
async fn manifest_records_source_posture_probe() -> anyhow::Result<()> {
    let tmp = tempfile::TempDir::new()?;
    let project = stage_append_only_project(&tmp)?;
    seed_clicks(&project.db_path)?;

    // First run: no recorded baseline yet — the posture probe establishes
    // one, still governed by cadence, still worth a manifest entry.
    let outcome = project
        .run_quiet("append-only-manifest-1", base_request("dev"))
        .await?;

    let record = outcome
        .models
        .get("clicks_summary")
        .expect("clicks_summary must have a manifest entry");
    let probe = record
        .probes
        .iter()
        .find(|p| p.fact == "mutation_profile.kind: append_only")
        .unwrap_or_else(|| {
            panic!(
                "expected an append-only posture probe record, got {:?}",
                record.probes
            )
        });
    assert_eq!(probe.probe, "SourceMutationProfileViolated");
    assert_eq!(
        probe.outcome,
        ProbeRecordOutcome::BaselineEstablished,
        "the first observation has no recorded baseline to verify against — it \
         establishes one rather than dispatching a comparison"
    );
    Ok(())
}
