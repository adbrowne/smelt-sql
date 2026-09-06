
use crate::support::build_report_for;
use std::fs;

const SMELT_YML: &str = r#"
name: write_pin_explain_fixture
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

/// A `grain: partition` model (no `unique_key:`, so identity-free) with
/// a `write: region` pin on its backfill cell. `region` requires only
/// the declared partition axis this output has, so it resolves cleanly
/// — the report must show it as the active pin and list it inside the
/// admissible set.
const MODEL_WITH_VALID_PIN: &str = r#"---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: d
  partition_column: d
  granularity: day
maintenance:
  cells:
    - columns: [amount]
      on: backfill
      write: region
---
SELECT d, amount FROM smelt.sources.payments
"#;

const PAYMENTS_SOURCE: &str = r#"
columns:
  - { name: d, type: DATE, nullable: false }
  - { name: amount, type: DOUBLE, nullable: false }
"#;

/// Phase R1 (`docs/plans/20260715-composed-axes-conditional-maintenance.md`):
/// `smelt explain <model>` prints, per cell, the open write-pattern
/// registry's admissible pattern-name set and the active `write:` pin (if
/// any) — `docs/specs/incremental_models.md` §"Per-cell write addressing".
/// Built as a self-contained tempdir workspace (not one of the shared
/// `examples/` fixtures) so the `write:` pin doesn't perturb any other
/// example-based test or the `example_diagnostics`/LSP parity gates.
#[test]
fn explain_prints_admissible_set_and_active_pin() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();
    fs::write(root.join("smelt.yml"), SMELT_YML).unwrap();
    fs::create_dir_all(root.join("models")).unwrap();
    fs::create_dir_all(root.join("models/sources")).unwrap();
    fs::write(root.join("models/revenue.sql"), MODEL_WITH_VALID_PIN).unwrap();
    fs::write(root.join("models/sources/payments.yml"), PAYMENTS_SOURCE).unwrap();

    let report = build_report_for(root, "revenue").expect("revenue has a maintenance plan");

    assert!(
        report.contains("admissible write patterns:"),
        "expected an admissible-write-patterns row per cell: {report}"
    );
    assert!(
        report.contains("write pin: region"),
        "expected the active `write: region` pin to be printed: {report}"
    );
    // The admissible set must actually list the pinned pattern — a pin
    // never widens the set, and here it should already be a member.
    let admissible_line = report
        .lines()
        .find(|l| l.contains("admissible write patterns:"))
        .expect("admissible-set line present");
    assert!(
        admissible_line.contains("region"),
        "the admissible set must include the pinned pattern 'region': {admissible_line}"
    );
}

/// A model that declares no `maintenance.cells[]` at all still prints
/// the admissible-set row per cell, with `write pin: (none)` — the row
/// is unconditional, not only shown when a pin exists.
#[test]
fn explain_prints_none_pin_when_no_cells_declared() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();
    let model_no_pin = r#"---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: d
  partition_column: d
  granularity: day
---
SELECT d, amount FROM smelt.sources.payments
"#;
    fs::write(root.join("smelt.yml"), SMELT_YML).unwrap();
    fs::create_dir_all(root.join("models")).unwrap();
    fs::create_dir_all(root.join("models/sources")).unwrap();
    fs::write(root.join("models/revenue.sql"), model_no_pin).unwrap();
    fs::write(root.join("models/sources/payments.yml"), PAYMENTS_SOURCE).unwrap();

    let report = build_report_for(root, "revenue").expect("revenue has a maintenance plan");

    assert!(
        report.contains("write pin: (none)"),
        "expected an unconditional 'write pin: (none)' row absent any cells[] entry: {report}"
    );
}
