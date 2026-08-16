//! `maintenance_plan_report`'s `own_output_delta`/`run_shape` fields
//! (`docs/specs/incremental_models.md` §Surface "CLI" **Headline**, phase 9
//! of `docs/outcomes/20260816-scheduler-delta-signatures/outcome.md`): a
//! model's own per-column-group output-delta verdicts, and its derived run
//! shape — snapshot-reconcile for an unclocked keyed model's driving source,
//! window-forward for a clocked one, the window sweep over the partition
//! axis for a `grain: partition` model.

use std::fs;

use smelt_core::workspace::load_workspace;
use smelt_db::queries::maintenance::MaintenancePlanResult;
use smelt_db::workspace_ingest::ingest_loaded_workspace;
use smelt_logical::maintenance::signature::KeyedRunShape;

/// Build a real on-disk workspace under a fresh tempdir, ingest it into a
/// Salsa `Database`, and return the derived maintenance plan for `model_file`
/// (relative to `models/`, without extension) — mirrors
/// `maintenance_model_upstream.rs::plan_for`.
fn plan_for(files: &[(&str, &str)], model_file: &str) -> MaintenancePlanResult {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path().to_path_buf();
    for (rel, content) in files {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, content).unwrap();
    }

    let loaded = load_workspace(&root);
    let mut db = smelt_db::Database::default();
    let ingested = ingest_loaded_workspace(&mut db, &loaded);
    db.set_workspace(ingested.source_files.clone(), vec![ingested.project]);
    let ws = db.workspace();

    let target_path = root.join("models").join(format!("{model_file}.sql"));
    let file = ingested
        .source_files
        .iter()
        .zip(ingested.paths.iter())
        .find(|(_, p)| **p == target_path)
        .map(|(f, _)| *f)
        .unwrap_or_else(|| panic!("model file {target_path:?} not ingested"));

    smelt_db::maintenance_plan_report(&db, ws, file, "duckdb")
        .unwrap_or_else(|| panic!("model {model_file} has no maintenance plan"))
}

const SMELT_YML: &str = r#"
name: maintenance_signature_fixture
version: 1

paths:
  - models

targets:
  dev:
    type: duckdb
    database: target/dev.duckdb
    schema: main

default_materialization: view
"#;

/// A clocked source feeding a `grain: key` model: the driving source's own
/// `timeseries:` block makes the run shape window-forward, never
/// snapshot-reconcile.
const CLOCKED_EVENTS_SOURCE: &str = r#"
description: Raw events, append-only, clocked on event_date.
mutation_profile: append_only
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
columns:
  - { name: device_id, type: INTEGER, nullable: false }
  - { name: event_date, type: DATE, nullable: false }
  - { name: amount, type: DOUBLE, nullable: false }
"#;

#[test]
fn plan_report_carries_own_signature_for_a_clocked_keyed_model() {
    let model = r#"---
materialization: table
refresh: incremental
grain: key
---
SELECT device_id, SUM(amount) AS total_amount
FROM smelt.sources.events
GROUP BY device_id
"#;

    let plan = plan_for(
        &[
            ("smelt.yml", SMELT_YML),
            ("models/sources/events.yml", CLOCKED_EVENTS_SOURCE),
            ("models/total_amount.sql", model),
        ],
        "total_amount",
    );

    assert!(
        !plan.own_output_delta.is_empty(),
        "expected at least one derived per-group verdict: {plan:?}"
    );
    assert_eq!(
        plan.run_shape,
        Some(KeyedRunShape::WindowForward),
        "a clocked driving source must derive window-forward, not snapshot-reconcile"
    );
}

/// An unclocked source (no `timeseries:` block) feeding a `grain: key`
/// model: the driving source has no clock, so the run shape is
/// snapshot-reconcile.
const UNCLOCKED_SIGNUPS_SOURCE: &str = r#"
description: Raw signups, no clock.
columns:
  - { name: user_id, type: INTEGER, nullable: false }
  - { name: amount, type: DOUBLE, nullable: false }
"#;

#[test]
fn plan_report_carries_own_signature_for_an_unclocked_keyed_model() {
    let model = r#"---
materialization: table
refresh: incremental
grain: key
---
SELECT user_id, ANY_VALUE(amount) AS total_amount
FROM smelt.sources.signups
GROUP BY user_id
"#;

    let plan = plan_for(
        &[
            ("smelt.yml", SMELT_YML),
            ("models/sources/signups.yml", UNCLOCKED_SIGNUPS_SOURCE),
            ("models/user_total_amount.sql", model),
        ],
        "user_total_amount",
    );

    assert!(
        !plan.own_output_delta.is_empty(),
        "expected at least one derived per-group verdict: {plan:?}"
    );
    assert_eq!(
        plan.run_shape,
        Some(KeyedRunShape::SnapshotReconcile),
        "an unclocked driving source must derive snapshot-reconcile"
    );
}

/// A `grain: partition` model's run shape is the window sweep over its own
/// declared partition axis, never re-derived from "does it have a clock" at
/// the CLI — read straight off `metadata.timeseries`.
#[test]
fn plan_report_carries_partition_sweep_run_shape() {
    let model = r#"---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
---
SELECT device_id, event_date, SUM(amount) AS daily_amount
FROM smelt.sources.events
GROUP BY device_id, event_date
"#;

    let plan = plan_for(
        &[
            ("smelt.yml", SMELT_YML),
            ("models/sources/events.yml", CLOCKED_EVENTS_SOURCE),
            ("models/daily_amount.sql", model),
        ],
        "daily_amount",
    );

    assert!(
        !plan.own_output_delta.is_empty(),
        "expected at least one derived per-group verdict: {plan:?}"
    );
    assert_eq!(
        plan.run_shape,
        Some(KeyedRunShape::PartitionSweep {
            axis: "event_date".to_string()
        })
    );
}
