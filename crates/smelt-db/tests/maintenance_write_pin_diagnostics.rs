//! `maintenance.cells[].write` pin validation, folded into
//! `file_diagnostics()` (`crates/smelt-db/src/queries/maintenance.rs::
//! write_pin_diagnostics`).
//!
//! Spec: `docs/specs/incremental_models.md` §"Per-cell write addressing" →
//! "User pins", "The write-pattern set is open"; `docs/specs/diagnostics.md`
//! catalogue entries for `MaintenanceWritePatternUnavailable` /
//! `MaintenanceWriteAddressingRefused`.

use std::fs;

use smelt_core::workspace::load_workspace;
use smelt_db::{workspace_ingest::ingest_loaded_workspace, DiagnosticCode};

/// Build a real on-disk workspace under a fresh tempdir, ingest it into a
/// Salsa `Database`, and return the diagnostics for `model_file` (relative
/// to `models/`, without extension). Mirrors `maintenance_diagnostics.rs`'s
/// own helper — kept local because integration test files are separate
/// binaries and cannot share a `#[cfg(test)]`-gated helper.
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
name: maintenance_write_pin_fixture
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

const PAYMENTS_SOURCE: &str = r#"
description: Payments fact, append-only.
columns:
  - { name: order_id, type: INTEGER, nullable: false }
  - { name: order_date, type: DATE, nullable: false }
  - { name: amount, type: DOUBLE, nullable: false }
"#;

/// A `write:` pin naming a pattern the registry does not recognise refuses
/// `MaintenanceWritePatternUnavailable`, naming the pattern and the (project
/// default) backend — never a silent downgrade to a substituted technique.
#[test]
fn unrecognised_write_pattern_refuses_pattern_unavailable() {
    let model = r#"---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: order_date
  partition_column: order_date
  granularity: day
maintenance:
  cells:
    - columns: [amount]
      on: backfill
      write: not_a_real_pattern
---
SELECT order_id, order_date, amount FROM smelt.sources.payments
"#;

    let diags = diagnostics_for(
        &[
            ("smelt.yml", SMELT_YML),
            ("models/sources/payments.yml", PAYMENTS_SOURCE),
            ("models/revenue.sql", model),
        ],
        "revenue",
    );

    let hit = diags
        .iter()
        .find(|d| d.code == Some(DiagnosticCode::MaintenanceWritePatternUnavailable))
        .unwrap_or_else(|| {
            panic!("expected a MaintenanceWritePatternUnavailable diagnostic, got: {diags:?}")
        });
    assert!(
        hit.message.contains("not_a_real_pattern"),
        "message must name the refused pattern: {}",
        hit.message
    );
    // Never a silent downgrade: no diagnostic claims a different technique
    // was substituted for the pin.
    assert!(
        !diags
            .iter()
            .any(|d| d.code == Some(DiagnosticCode::MaintenanceWriteAddressingRefused)),
        "an unrecognised pattern must not ALSO refuse as an addressing violation: {diags:?}"
    );
}

/// A `write: keyed` pin on a `grain: partition` model with no declared
/// `unique_key` (an identity-free output) resolves the pattern in the
/// registry and the backend can run `MERGE`, but this cell's own facts
/// cannot uphold the pattern's equivalence obligation — refused
/// `MaintenanceWriteAddressingRefused`, naming the cell, never a silent
/// downgrade to region rewrite.
#[test]
fn keyed_pin_on_identity_free_output_refuses_addressing_refused() {
    let model = r#"---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: order_date
  partition_column: order_date
  granularity: day
maintenance:
  cells:
    - columns: [amount]
      on: backfill
      write: keyed
---
SELECT order_id, order_date, amount FROM smelt.sources.payments
"#;

    let diags = diagnostics_for(
        &[
            ("smelt.yml", SMELT_YML),
            ("models/sources/payments.yml", PAYMENTS_SOURCE),
            ("models/revenue.sql", model),
        ],
        "revenue",
    );

    let hit = diags
        .iter()
        .find(|d| d.code == Some(DiagnosticCode::MaintenanceWriteAddressingRefused))
        .unwrap_or_else(|| {
            panic!("expected a MaintenanceWriteAddressingRefused diagnostic, got: {diags:?}")
        });
    assert!(
        hit.message.contains("keyed"),
        "message must name the refused pattern: {}",
        hit.message
    );
    assert!(
        !diags
            .iter()
            .any(|d| d.code == Some(DiagnosticCode::MaintenanceWritePatternUnavailable)),
        "a facts violation must not ALSO refuse as pattern-unavailable: {diags:?}"
    );
}

/// A `write: region` pin on a `grain: partition` model (which declares the
/// required partition axis) resolves cleanly — no `Maintenance*` write-pin
/// diagnostic at all.
#[test]
fn valid_region_pin_on_partition_grain_model_is_clean() {
    let model = r#"---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: order_date
  partition_column: order_date
  granularity: day
maintenance:
  cells:
    - columns: [amount]
      on: backfill
      write: region
---
SELECT order_id, order_date, amount FROM smelt.sources.payments
"#;

    let diags = diagnostics_for(
        &[
            ("smelt.yml", SMELT_YML),
            ("models/sources/payments.yml", PAYMENTS_SOURCE),
            ("models/revenue.sql", model),
        ],
        "revenue",
    );

    assert!(
        !diags.iter().any(|d| matches!(
            d.code,
            Some(DiagnosticCode::MaintenanceWritePatternUnavailable)
                | Some(DiagnosticCode::MaintenanceWriteAddressingRefused)
        )),
        "a valid `write: region` pin on a partition-grain model must not refuse: {diags:?}"
    );
}
