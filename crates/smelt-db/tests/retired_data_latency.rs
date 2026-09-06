//! The per-column `data_latency:` frontmatter key is retired outright
//! (`docs/specs/models.md` §Diagnostics): declared lateness is expressed
//! once on the source as `mutation_profile.lateness`, never per column.
//! `file_diagnostics()` surfaces the hard error naming the replacement,
//! mirroring `crates/smelt-db/tests/partition_grain_forbids_metrics.rs`'s
//! harness.

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
name: retired_data_latency_fixture
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
fn per_column_data_latency_is_a_hard_error_naming_mutation_profile_lateness() {
    let model = r#"---
materialization: table
columns:
  order_date:
    data_latency: "3 days"
---
SELECT order_date, amount FROM smelt.sources.orders
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
        .filter(|d| d.code == Some(DiagnosticCode::YamlParseError))
        .collect();
    assert_eq!(
        hits.len(),
        1,
        "expected exactly one YamlParseError, got {diags:?}"
    );
    assert_eq!(hits[0].severity, smelt_db::DiagnosticSeverity::Error);
    assert!(
        hits[0].message.contains("mutation_profile.lateness"),
        "expected fix-it naming `mutation_profile.lateness`, got: {}",
        hits[0].message
    );
}

#[test]
fn no_per_column_data_latency_is_clean() {
    let model = r#"---
materialization: table
---
SELECT order_date, amount FROM smelt.sources.orders
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
            .all(|d| d.code != Some(DiagnosticCode::YamlParseError)),
        "no per-column data_latency must not surface a YamlParseError, got {diags:?}"
    );
}
