//! Declared-granularity checking (`incremental_models.md` §Design "Grain is
//! declared"; §"The graph layer" — "the classifier only *checks* the
//! declaration, e.g. against a `date_trunc` grouping"): a model's declared
//! `timeseries.granularity` must agree with the truncation/grid unit its own
//! `partition_column` SELECT-list projection actually derives to. This is a
//! thin `smelt-db` Salsa wrapper around the pure leaf classifier in
//! `crates/smelt-logical/src/maintenance/granularity.rs`.

use std::fs;

use smelt_core::workspace::load_workspace;
use smelt_db::{workspace_ingest::ingest_loaded_workspace, DiagnosticCode};

/// Build a real on-disk workspace under a fresh tempdir, ingest it into a
/// Salsa `Database`, and return the diagnostics for `model_file` (relative
/// to `models/`, without extension). Mirrors
/// `crates/smelt-db/tests/maintenance_diagnostics.rs`'s harness.
fn diagnostics_for(files: &[(&str, &str)], model_file: &str) -> Vec<smelt_db::Diagnostic> {
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

    smelt_db::file_diagnostics(&db, ws, file)
}

const SMELT_YML: &str = r#"
name: grain_check_fixture
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

const ORDERS_SOURCE: &str = r#"
description: Orders, append-only, clocked on order_date.
mutation_profile: append_only
columns:
  - { name: order_id, type: INTEGER, nullable: false }
  - { name: order_date, type: TIMESTAMP, nullable: false }
  - { name: amount, type: DOUBLE, nullable: false }
"#;

/// A model declares `granularity: hour` but its own `partition_column`
/// projection only truncates on `date_trunc('day', …)` — the declaration
/// **narrows** past what the SQL's own grouping can actually distinguish
/// (per widen-never-narrow, `incremental_models.md` §Design) and must error,
/// never silently keep the mismatched declaration (`docs/specs/models.md`:
/// declared-grain contradictions are hard errors, never silent).
#[test]
fn declared_granularity_contradicted_by_grouping_errors() {
    let model = r#"---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: order_date
  partition_column: order_date
  granularity: hour
---
SELECT
    date_trunc('day', o.order_date) AS order_date,
    SUM(o.amount) AS total
FROM smelt.sources.orders o
GROUP BY 1
"#;

    let diags = diagnostics_for(
        &[
            ("smelt.yml", SMELT_YML),
            ("models/sources/orders.yml", ORDERS_SOURCE),
            ("models/revenue.sql", model),
        ],
        "revenue",
    );

    let mismatches: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::MaintenanceGranularityMismatch))
        .collect();
    assert_eq!(
        mismatches.len(),
        1,
        "expected exactly one MaintenanceGranularityMismatch, got {diags:?}"
    );
    assert!(mismatches[0].message.contains("hour"));
    assert!(mismatches[0].message.contains("day"));
}

/// A model whose declared granularity is **coarser** than its own
/// `date_trunc` grouping is a safe widen, never an error
/// (`incremental_models.md` §Design "Widen-never-narrow").
#[test]
fn declared_granularity_coarser_than_grouping_is_a_safe_widen() {
    let model = r#"---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: order_date
  partition_column: order_date
  granularity: week
---
SELECT
    date_trunc('day', o.order_date) AS order_date,
    SUM(o.amount) AS total
FROM smelt.sources.orders o
GROUP BY 1
"#;

    let diags = diagnostics_for(
        &[
            ("smelt.yml", SMELT_YML),
            ("models/sources/orders.yml", ORDERS_SOURCE),
            ("models/revenue.sql", model),
        ],
        "revenue",
    );

    assert!(
        diags
            .iter()
            .all(|d| d.code != Some(DiagnosticCode::MaintenanceGranularityMismatch)),
        "a coarser-than-actual declared granularity is a safe widen, got {diags:?}"
    );
}

/// A model whose declared granularity agrees with its own `date_trunc`
/// grouping must never raise `MaintenanceGranularityMismatch`.
#[test]
fn declared_granularity_matching_grouping_is_clean() {
    let model = r#"---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: order_date
  partition_column: order_date
  granularity: day
---
SELECT
    date_trunc('day', o.order_date) AS order_date,
    SUM(o.amount) AS total
FROM smelt.sources.orders o
GROUP BY 1
"#;

    let diags = diagnostics_for(
        &[
            ("smelt.yml", SMELT_YML),
            ("models/sources/orders.yml", ORDERS_SOURCE),
            ("models/revenue.sql", model),
        ],
        "revenue",
    );

    assert!(
        diags
            .iter()
            .all(|d| d.code != Some(DiagnosticCode::MaintenanceGranularityMismatch)),
        "matching declared/derived granularity should not raise a mismatch, got {diags:?}"
    );
}
