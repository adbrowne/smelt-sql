//! `maintenance_plan_report`'s `state_columns` field
//! (`docs/outcomes/20260809-rung2-state-shapes` row 9): a `grain: key`
//! model with a presented column that folds through hidden decomposed state
//! (`AVG` → `(sum, count)`) reports it; a plain additive-fold column does
//! not.

use std::fs;

use smelt_core::workspace::load_workspace;
use smelt_db::queries::maintenance::MaintenancePlanResult;
use smelt_db::workspace_ingest::ingest_loaded_workspace;

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
name: maintenance_plan_state_columns_fixture
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

const EVENTS_SOURCE: &str = r#"
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

/// A `grain: key` model whose only presented column is `AVG(amount)` folds
/// through hidden `(sum, count)` state — `state_columns` names both hidden
/// columns and the presentation expression.
#[test]
fn maintenance_plan_report_carries_state_columns() {
    let model = r#"---
materialization: table
refresh: incremental
grain: key
---
SELECT device_id, AVG(amount) AS avg_amount
FROM smelt.sources.events
GROUP BY device_id
"#;

    let plan = plan_for(
        &[
            ("smelt.yml", SMELT_YML),
            ("models/sources/events.yml", EVENTS_SOURCE),
            ("models/avg_amount.sql", model),
        ],
        "avg_amount",
    );

    assert_eq!(
        plan.state_columns,
        vec![smelt_logical::StateColumnSummary {
            presented_column: "avg_amount".to_string(),
            state_columns: vec![
                "avg_amount__sum".to_string(),
                "avg_amount__count".to_string()
            ],
            presentation_expr: "avg_amount__sum / avg_amount__count".to_string(),
        }],
        "expected AVG's decomposed state to be reported: {:?}",
        plan.state_columns
    );
}

/// A `grain: key` model whose only presented column is a plain `SUM` folds
/// its own presented value directly — no decomposed state, so
/// `state_columns` is empty.
#[test]
fn maintenance_plan_report_omits_state_columns_for_stateless_model() {
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
            ("models/sources/events.yml", EVENTS_SOURCE),
            ("models/total_amount.sql", model),
        ],
        "total_amount",
    );

    assert!(
        plan.state_columns.is_empty(),
        "expected no decomposed state for a plain SUM column: {:?}",
        plan.state_columns
    );
}
