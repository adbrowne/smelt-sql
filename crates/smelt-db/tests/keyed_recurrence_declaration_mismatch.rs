//! `KeyedRecurrenceDeclarationMismatch` (key-grain rule 16,
//! `docs/specs/incremental_shapes.md` §"Key temporal locality"): a declared
//! `key_recurrence` on a driving source that disagrees with route 3's
//! statically-derived recurrence bound refuses at plan time via
//! `file_diagnostics()` — the derived bound is authoritative.

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
name: keyed_recurrence_declaration_mismatch_fixture
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
description: Raw events, append-only, redelivery-prone.
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
mutation_profile:
  kind: append_only
  key_recurrence:
    key: [event_id]
    window: '7 days'
columns:
  - { name: event_id, type: INTEGER, nullable: false }
  - { name: event_date, type: DATE, nullable: false }
"#;

/// The model's own SQL statically proves a 3-day recurrence bound (a
/// Form-B WHERE-filter lookback on the driving source's own clock column),
/// but the source declares `key_recurrence.window: 7 days` over the same
/// key — the declaration disagrees with the derived, authoritative bound.
const MISMATCHED_MODEL: &str = r#"---
materialization: table
refresh: incremental
grain: key
timeseries:
  event_time_column: last_seen_date
  partition_column: last_seen_date
  granularity: day
---
SELECT
    event_id,
    MAX(event_date) AS last_seen_date,
    COUNT(*) AS event_count
FROM smelt.sources.raw.events
WHERE event_date >= CAST(event_date AS DATE) - INTERVAL '3 days'
GROUP BY event_id
"#;

#[test]
fn disagreeing_declared_recurrence_yields_exactly_one_mismatch_diagnostic() {
    let diags = diagnostics_for(
        &[
            ("smelt.yml", SMELT_YML),
            ("models/sources/raw/events.yml", EVENTS_SOURCE),
            ("models/events_last_seen.sql", MISMATCHED_MODEL),
        ],
        "events_last_seen",
    );

    let mismatches: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::KeyedRecurrenceDeclarationMismatch))
        .collect();
    assert_eq!(
        mismatches.len(),
        1,
        "expected exactly one KeyedRecurrenceDeclarationMismatch, got {diags:?}"
    );
    let diag = mismatches[0];
    assert_eq!(diag.severity, smelt_db::DiagnosticSeverity::Error);
    assert!(
        diag.message.contains("3") && diag.message.contains("7"),
        "message must name both the derived and declared values: {}",
        diag.message
    );
}

const AGREEING_SOURCE: &str = r#"
description: Raw events, append-only, redelivery-prone.
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
mutation_profile:
  kind: append_only
  key_recurrence:
    key: [event_id]
    window: '3 days'
columns:
  - { name: event_id, type: INTEGER, nullable: false }
  - { name: event_date, type: DATE, nullable: false }
"#;

#[test]
fn agreeing_declared_recurrence_yields_no_mismatch_diagnostic() {
    let diags = diagnostics_for(
        &[
            ("smelt.yml", SMELT_YML),
            ("models/sources/raw/events.yml", AGREEING_SOURCE),
            ("models/events_last_seen.sql", MISMATCHED_MODEL),
        ],
        "events_last_seen",
    );

    assert!(
        diags
            .iter()
            .all(|d| d.code != Some(DiagnosticCode::KeyedRecurrenceDeclarationMismatch)),
        "an agreeing declaration must not mismatch: {diags:?}"
    );
}
