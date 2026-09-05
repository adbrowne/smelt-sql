//! `PartitionGrainForbidsMetrics` (`docs/specs/incremental_shapes.md`
//! §"Functions inside partition-grain bodies"): a `grain: partition` model's
//! body calling `smelt.metric(...)` refuses ahead of execution — the
//! composition of metric expansion with time-filter injection is
//! deliberately unspecified. Salsa-direct `file_diagnostics()` coverage,
//! mirroring `crates/smelt-db/tests/grain_check.rs`'s harness.

use std::fs;

use smelt_core::workspace::load_workspace;
use smelt_db::{workspace_ingest::ingest_loaded_workspace, DiagnosticCode};

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
name: partition_grain_forbids_metrics_fixture
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

#[test]
fn partition_grain_metric_call_file_diagnostic() {
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
    order_date,
    smelt.metric('revenue') AS revenue
FROM smelt.sources.orders
"#;

    let diags = diagnostics_for(
        &[
            ("smelt.yml", SMELT_YML),
            ("models/sources/orders.yml", ORDERS_SOURCE),
            ("models/revenue.sql", model),
        ],
        "revenue",
    );

    let hits: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::PartitionGrainForbidsMetrics))
        .collect();
    assert_eq!(
        hits.len(),
        1,
        "expected exactly one PartitionGrainForbidsMetrics, got {diags:?}"
    );
    assert_eq!(hits[0].severity, smelt_db::DiagnosticSeverity::Error);
    // Rule diagnostics are anchored at the model's SQL body start (`lib.rs`'s
    // `detect_builtin_rules` loop), not offset 0 of the file (frontmatter
    // precedes the body) — a nonzero start confirms it is in-range.
    assert!(
        u32::from(hits[0].range.start()) > 0,
        "diagnostic must be anchored past the frontmatter, got {:?}",
        hits[0].range
    );
}

#[test]
fn partition_grain_without_metric_call_is_clean() {
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
    order_date,
    SUM(amount) AS total
FROM smelt.sources.orders
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
            .all(|d| d.code != Some(DiagnosticCode::PartitionGrainForbidsMetrics)),
        "no smelt.metric() call must not surface PartitionGrainForbidsMetrics, got {diags:?}"
    );
}
