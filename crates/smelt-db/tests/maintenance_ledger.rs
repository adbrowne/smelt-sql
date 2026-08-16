//! `maintenance_plan_report`'s `column_determinism` field
//! (`docs/specs/incremental_models.md` §Surface "CLI" — the per-column
//! guarantee ledger, phase 10 of
//! `docs/outcomes/20260816-scheduler-delta-signatures/outcome.md`): the
//! per-column determinism verdict `smelt_logical::maintenance::ledger::
//! derive_guarantee_ledger` reads to print a volatile column's determinism
//! exemption in place of an equivalence contract.

use std::fs;

use smelt_core::workspace::load_workspace;
use smelt_db::queries::maintenance::MaintenancePlanResult;
use smelt_db::workspace_ingest::ingest_loaded_workspace;
use smelt_logical::analysis::walk::Determinism;

/// Build a real on-disk workspace under a fresh tempdir, ingest it into a
/// Salsa `Database`, and return the derived maintenance plan for `model_file`
/// (relative to `models/`, without extension) — mirrors
/// `maintenance_signature.rs::plan_for`.
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
name: maintenance_ledger_fixture
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

/// A model that projects a run-nondeterministic column (`CURRENT_DATE`)
/// alongside a clean, deterministic one: `column_determinism` must carry the
/// `Run` verdict for the volatile column and `Clean` for the plain one — the
/// non-report construction path (`finish_plan_result`) leaves this field
/// empty, so a non-empty result here is only reachable through
/// `maintenance_plan_report`'s own population.
#[test]
fn plan_report_populates_column_determinism() {
    let model = r#"---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
---
SELECT device_id, event_date, SUM(amount) AS daily_amount, now() AS loaded_on
FROM smelt.sources.events
GROUP BY device_id, event_date
"#;

    let plan = plan_for(
        &[
            ("smelt.yml", SMELT_YML),
            ("models/sources/events.yml", EVENTS_SOURCE),
            ("models/daily_amount.sql", model),
        ],
        "daily_amount",
    );

    assert!(
        !plan.column_determinism.is_empty(),
        "expected a derived determinism verdict per output column: {plan:?}"
    );
    let loaded_on = plan
        .column_determinism
        .iter()
        .find(|d| d.output.eq_ignore_ascii_case("loaded_on"))
        .unwrap_or_else(|| panic!("no determinism verdict for loaded_on: {plan:?}"));
    assert_eq!(loaded_on.level, Determinism::Run);
    let daily_amount = plan
        .column_determinism
        .iter()
        .find(|d| d.output.eq_ignore_ascii_case("daily_amount"))
        .unwrap_or_else(|| panic!("no determinism verdict for daily_amount: {plan:?}"));
    assert_eq!(daily_amount.level, Determinism::Clean);
}

/// A `grain: key` model with no declared `unique_key`, grouping by a
/// projected alias (`date_trunc('day', event_date) AS d`), must still prove
/// `RowIdentity::Key(["d"])` through the full Salsa path — not fail closed to
/// `WholeRow` because the grain factory couldn't resolve `GROUP BY d` against
/// the alias (`docs/outcomes/20260816-scheduler-delta-signatures/outcome.md`
/// phase 11).
#[test]
fn alias_grouped_model_proves_row_identity() {
    let model = r#"---
materialization: table
refresh: incremental
grain: key
---
SELECT date_trunc('day', event_date) AS d, SUM(amount) AS total_amount
FROM smelt.sources.events
GROUP BY d
"#;

    let plan = plan_for(
        &[
            ("smelt.yml", SMELT_YML),
            ("models/sources/events.yml", EVENTS_SOURCE),
            ("models/total_amount.sql", model),
        ],
        "total_amount",
    );

    let identity = plan
        .plan
        .cells
        .iter()
        .find_map(|cell| match &cell.row_identity.identity {
            smelt_logical::maintenance::RowIdentity::Key(cols) => Some(cols.clone()),
            smelt_logical::maintenance::RowIdentity::WholeRow => None,
        })
        .unwrap_or_else(|| panic!("expected a Key row identity, got WholeRow only: {plan:?}"));
    assert_eq!(
        identity,
        vec!["d".to_string()],
        "alias-grouped key must resolve to the output column 'd'"
    );
}
