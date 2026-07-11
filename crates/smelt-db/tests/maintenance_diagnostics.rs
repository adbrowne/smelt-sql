//! `Maintenance*` diagnostics: the thin `maintenance_plan` Salsa query
//! (`crates/smelt-db/src/queries/maintenance.rs`) folds the derived plan's
//! admission refusals, plus the `maintenance.cells[]` column-group-span
//! check, into `file_diagnostics()`.
//!
//! Spec: `docs/specs/maintenance_plan.md` §Diagnostics, §Semantics
//! "Partition-local maintenance (the K8 guardrail)"; `docs/specs/models.md`
//! "Declared grain contradicted by the derived plan" (Constraint violations
//! table).

use std::fs;

use smelt_core::workspace::load_workspace;
use smelt_db::{workspace_ingest::ingest_loaded_workspace, DiagnosticCode};

/// Build a real on-disk workspace under a fresh tempdir, ingest it into a
/// Salsa `Database`, and return the diagnostics for `model_file` (relative
/// to `models/`, without extension).
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
name: maintenance_diagnostics_fixture
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

/// A cross-axis, unclocked enrichment source with no derivable predicate to
/// the output's partition axis must refuse with `MaintenanceScanUnbounded`
/// under the K8 default (`require: partition_local`, `on_violation:
/// error`); a declared `allow_full_scan: true` for that source clears it.
#[test]
fn unbounded_scan_refuses_by_default() {
    let orders_source = r#"
description: Orders, append-only, clocked on order_date.
mutation_profile: append_only
columns:
  - { name: order_id, type: INTEGER, nullable: false }
  - { name: order_date, type: DATE, nullable: false }
  - { name: customer_id, type: INTEGER, nullable: false }
"#;
    let enrichment_source = r#"
description: Customer enrichment lookup, mutable snapshot, unclocked.
mutation_profile: mutable_snapshot
columns:
  - { name: customer_id, type: INTEGER, nullable: false }
  - { name: category, type: VARCHAR, nullable: true }
"#;
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
    o.order_id,
    o.order_date,
    e.category AS enrichment_category
FROM smelt.sources.orders o
JOIN smelt.sources.enrichment e ON o.customer_id = e.customer_id
"#;

    let diags = diagnostics_for(
        &[
            ("smelt.yml", SMELT_YML),
            ("models/sources/orders.yml", orders_source),
            ("models/sources/enrichment.yml", enrichment_source),
            ("models/revenue.sql", model),
        ],
        "revenue",
    );

    let scan_unbounded: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::MaintenanceScanUnbounded))
        .collect();
    assert_eq!(
        scan_unbounded.len(),
        1,
        "expected exactly one MaintenanceScanUnbounded, got {diags:?}"
    );

    // `allow_full_scan: true` for the enrichment source clears the refusal.
    let model_allowed = model.replacen(
        "grain: partition\n",
        "grain: partition\nmaintenance:\n  scan_bounds:\n    per_source:\n      enrichment:\n        allow_full_scan: true\n",
        1,
    );
    let diags_allowed = diagnostics_for(
        &[
            ("smelt.yml", SMELT_YML),
            ("models/sources/orders.yml", orders_source),
            ("models/sources/enrichment.yml", enrichment_source),
            ("models/revenue.sql", &model_allowed),
        ],
        "revenue",
    );
    assert!(
        diags_allowed
            .iter()
            .all(|d| d.code != Some(DiagnosticCode::MaintenanceScanUnbounded)),
        "allow_full_scan: true should clear MaintenanceScanUnbounded, got {diags_allowed:?}"
    );
}

/// A `grain: key` model whose body never aggregates has no fold candidate —
/// the plan-shaped read is a partition-shaped (row-per-event) body under a
/// key-addressed declaration, and the derivation refuses honestly
/// (`MaintenanceNoAdmissibleTechnique`) rather than silently keeping the
/// mismatched declaration (`docs/specs/models.md`: "Declared grain
/// contradicted by the derived plan ... Hard error").
#[test]
fn grain_mismatch_is_error_never_silent() {
    let payments_source = r#"
description: Payments, append-only.
mutation_profile: append_only
columns:
  - { name: user_id, type: INTEGER, nullable: false }
  - { name: amount, type: DOUBLE, nullable: false }
"#;
    let model = r#"---
materialization: table
refresh: incremental
grain: key
---
SELECT user_id, amount FROM smelt.sources.payments
"#;

    let diags = diagnostics_for(
        &[
            ("smelt.yml", SMELT_YML),
            ("models/sources/payments.yml", payments_source),
            ("models/lifetime_value.sql", model),
        ],
        "lifetime_value",
    );

    let refusals: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::MaintenanceNoAdmissibleTechnique))
        .collect();
    assert_eq!(
        refusals.len(),
        1,
        "expected exactly one MaintenanceNoAdmissibleTechnique, got {diags:?}"
    );
    assert_eq!(
        refusals[0].severity,
        smelt_db::DiagnosticSeverity::Error,
        "grain mismatch must be an Error, never silent"
    );
}

/// `maintenance.cells[].columns` naming members of two different derived
/// column groups is an error — it would silently re-partition the plan.
#[test]
fn cells_columns_spanning_groups_error() {
    let payments_source = r#"
description: Payments, append-only.
mutation_profile: append_only
columns:
  - { name: order_id, type: INTEGER, nullable: false }
  - { name: order_date, type: DATE, nullable: false }
  - { name: amount, type: DOUBLE, nullable: false }
"#;
    let shipments_source = r#"
description: Shipments, mutable snapshot.
mutation_profile: mutable_snapshot
columns:
  - { name: order_id, type: INTEGER, nullable: false }
  - { name: shipped_flag, type: BOOLEAN, nullable: true }
"#;
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
    - columns: [amount, shipped_flag]
      on: payments
---
SELECT
    p.order_id,
    p.order_date,
    p.amount,
    s.shipped_flag
FROM smelt.sources.payments p
JOIN smelt.sources.shipments s ON p.order_id = s.order_id
"#;

    let diags = diagnostics_for(
        &[
            ("smelt.yml", SMELT_YML),
            ("models/sources/payments.yml", payments_source),
            ("models/sources/shipments.yml", shipments_source),
            ("models/orders_enriched.sql", model),
        ],
        "orders_enriched",
    );

    let violations: Vec<_> = diags
        .iter()
        .filter(|d| {
            d.code == Some(DiagnosticCode::MaintenanceNoAdmissibleTechnique)
                && d.message.contains("spans")
        })
        .collect();
    assert_eq!(
        violations.len(),
        1,
        "expected exactly one cells[].columns-spans-groups violation, got {diags:?}"
    );
}
