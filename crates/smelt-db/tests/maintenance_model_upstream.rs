//! Upstream model edges (`incremental_models.md` §"Upstream model edges"): a
//! maintained model's ref to **another maintained model in the same project**
//! is a plan edge of the same standing as a `sources.*` ref. The upstream's
//! own `timeseries:` clock supplies the creation-trigger cell's clamp; an
//! upstream whose clock cannot be derived is a recorded refusal
//! (`MaintenanceReachNotDerivable`, naming the edge), never a silent drop; a
//! `full`-mode or view upstream contributes no creation cell and no refusal.

use std::fs;

use smelt_core::workspace::load_workspace;
use smelt_db::queries::maintenance::MaintenancePlanResult;
use smelt_db::workspace_ingest::ingest_loaded_workspace;
use smelt_logical::maintenance::{Refusal, Trigger};

/// Build a real on-disk workspace under a fresh tempdir, ingest it into a
/// Salsa `Database`, and return the derived maintenance plan for `model_file`
/// (relative to `models/`, without extension).
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

    smelt_db::maintenance_plan_report(&db, ws, file)
        .unwrap_or_else(|| panic!("model {model_file} has no maintenance plan"))
}

const SMELT_YML: &str = r#"
name: maintenance_model_upstream_fixture
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

/// A two-model chain: an upstream `grain: partition` model that declares a
/// `timeseries:` clock, and a downstream model that refs it. The downstream's
/// plan must contain a **creation cell** for the model edge, clamped on the
/// upstream's own clock column.
#[test]
fn model_upstream_derives_creation_cell() {
    let upstream = r#"---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
---
SELECT event_id, event_date FROM smelt.sources.events
"#;
    let events_source = r#"
description: Raw events, append-only, clocked on event_date.
mutation_profile: append_only
columns:
  - { name: event_id, type: INTEGER, nullable: false }
  - { name: event_date, type: DATE, nullable: false }
"#;
    let downstream = r#"---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
---
SELECT event_id, event_date FROM smelt.silver_events
"#;

    let plan = plan_for(
        &[
            ("smelt.yml", SMELT_YML),
            ("models/sources/events.yml", events_source),
            ("models/silver_events.sql", upstream),
            ("models/gold_events.sql", downstream),
        ],
        "gold_events",
    );

    let creation = plan
        .plan
        .cells
        .iter()
        .find(|c| matches!(&c.trigger, Trigger::NewData { source } if source == "silver_events"))
        .unwrap_or_else(|| {
            panic!(
                "expected a NewData creation cell for the model edge 'silver_events'; cells: {:?}",
                plan.plan.cells
            )
        });

    assert!(
        creation
            .scans
            .iter()
            .any(|s| s.source == "silver_events" && s.column == "event_date"),
        "the model-edge creation cell must be clamped on the upstream's clock column \
         'event_date'; scans: {:?}",
        creation.scans
    );

    assert!(
        !plan.plan.refusals.iter().any(
            |r| matches!(r, Refusal::ReachNotDerivable { edge, .. } if edge == "silver_events")
        ),
        "a derivable clock must not record a refusal: {:?}",
        plan.plan.refusals
    );
}

/// An upstream maintained model with no derivable clock (a `grain: key` model
/// that declares no `timeseries:`) records a `MaintenanceReachNotDerivable`
/// refusal naming the edge — the cell is refused, never silently absent.
#[test]
fn model_upstream_without_clock_records_refusal() {
    let keyed_upstream = r#"---
materialization: table
refresh: incremental
grain: key
---
SELECT user_id, COUNT(*) AS n FROM smelt.sources.events GROUP BY user_id
"#;
    let events_source = r#"
description: Raw events, append-only.
mutation_profile: append_only
columns:
  - { name: user_id, type: INTEGER, nullable: false }
  - { name: event_date, type: DATE, nullable: false }
"#;
    let downstream = r#"---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
---
SELECT event_date, n FROM smelt.keyed_upstream
"#;

    let plan = plan_for(
        &[
            ("smelt.yml", SMELT_YML),
            ("models/sources/events.yml", events_source),
            ("models/keyed_upstream.sql", keyed_upstream),
            ("models/gold_events.sql", downstream),
        ],
        "gold_events",
    );

    assert!(
        plan.plan.refusals.iter().any(
            |r| matches!(r, Refusal::ReachNotDerivable { edge, .. } if edge == "keyed_upstream")
        ),
        "an underivable upstream clock must record a ReachNotDerivable refusal naming the \
         edge, never a silent drop; refusals: {:?}",
        plan.plan.refusals
    );

    assert!(
        !plan.plan.cells.iter().any(
            |c| matches!(&c.trigger, Trigger::NewData { source } if source == "keyed_upstream")
        ),
        "the refused edge must not also produce a creation cell: {:?}",
        plan.plan.cells
    );
}

/// A view / `full`-mode upstream delivers no incremental delta, so a ref to it
/// contributes no creation cell and no refusal — it participates in
/// mutation/backfill triggers only.
#[test]
fn view_upstream_derives_no_creation_cell() {
    let view_upstream = r#"---
materialization: view
---
SELECT event_id, event_date FROM smelt.sources.events
"#;
    let events_source = r#"
description: Raw events, append-only, clocked on event_date.
mutation_profile: append_only
columns:
  - { name: event_id, type: INTEGER, nullable: false }
  - { name: event_date, type: DATE, nullable: false }
"#;
    let downstream = r#"---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
---
SELECT event_id, event_date FROM smelt.raw_view
"#;

    let plan = plan_for(
        &[
            ("smelt.yml", SMELT_YML),
            ("models/sources/events.yml", events_source),
            ("models/raw_view.sql", view_upstream),
            ("models/gold_events.sql", downstream),
        ],
        "gold_events",
    );

    assert!(
        !plan
            .plan
            .cells
            .iter()
            .any(|c| matches!(&c.trigger, Trigger::NewData { source } if source == "raw_view")),
        "a view/full upstream must contribute no creation cell: {:?}",
        plan.plan.cells
    );
    assert!(
        !plan
            .plan
            .refusals
            .iter()
            .any(|r| matches!(r, Refusal::ReachNotDerivable { edge, .. } if edge == "raw_view")),
        "a view/full upstream must contribute no refusal: {:?}",
        plan.plan.refusals
    );
}
