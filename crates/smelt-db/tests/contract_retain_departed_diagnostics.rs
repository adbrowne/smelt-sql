//! Phase 32 (`docs/outcomes/20260815-definition-delta-migrate/phases/32-plan.md`):
//! `contract.retain_departed` posture/tombstone-column admissibility
//! diagnostic (`docs/specs/incremental_models.md` §"Contract relaxations
//! (`contract:`)") — a thin `smelt-db` Salsa wrapper around the pure
//! `smelt_logical::contract::retain_departed::validate` validator. Mirrors
//! `crates/smelt-db/tests/contract_deferral_diagnostics.rs`'s harness.

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
name: contract_retain_departed_fixture
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

const SNAPSHOT_SOURCE: &str = r#"
description: A mutable customer snapshot.
mutation_profile: mutable_snapshot
columns:
  - { name: customer_id, type: INTEGER, nullable: false }
  - { name: tier, type: VARCHAR, nullable: false }
"#;

const ORDERS_SOURCE: &str = r#"
description: Orders, append-only, clocked on order_date.
mutation_profile: append_only
columns:
  - { name: order_id, type: INTEGER, nullable: false }
  - { name: order_date, type: TIMESTAMP, nullable: false }
  - { name: amount, type: DOUBLE, nullable: false }
"#;

/// `retain_departed:` declared on a keyed model consuming a mutable
/// snapshot is clean.
#[test]
fn retain_departed_on_keyed_mutable_snapshot_is_clean() {
    let model = r#"---
materialization: table
refresh: incremental
grain: key
unique_key: [customer_id]
contract:
  retain_departed: true
---
SELECT
    c.customer_id,
    c.tier
FROM smelt.sources.customers c
"#;

    let diags = diagnostics_for(
        &[
            ("smelt.yml", SMELT_YML),
            ("models/sources/customers.yml", SNAPSHOT_SOURCE),
            ("models/dim_customers.sql", model),
        ],
        "dim_customers",
    );

    let matches: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::ContractRetainDepartedInvalid))
        .collect();
    assert!(
        matches.is_empty(),
        "expected no ContractRetainDepartedInvalid, got {diags:?}"
    );
}

/// `retain_departed:` declared on a partition-grain model is refused — the
/// point is admitted only on a keyed shape.
#[test]
fn retain_departed_on_partition_grain_raises_diagnostic() {
    let model = r#"---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: order_date
  partition_column: order_date
  granularity: day
contract:
  retain_departed: true
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

    let matches: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::ContractRetainDepartedInvalid))
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "expected exactly one ContractRetainDepartedInvalid, got {diags:?}"
    );
}

/// `retain_departed:` declared on a keyed model with no mutable_snapshot
/// source is refused.
#[test]
fn retain_departed_over_non_mutable_snapshot_source_raises_diagnostic() {
    let model = r#"---
materialization: table
refresh: incremental
grain: key
unique_key: [order_id]
contract:
  retain_departed: true
---
SELECT
    o.order_id,
    o.order_date,
    o.amount
FROM smelt.sources.orders o
"#;

    let diags = diagnostics_for(
        &[
            ("smelt.yml", SMELT_YML),
            ("models/sources/orders.yml", ORDERS_SOURCE),
            ("models/revenue.sql", model),
        ],
        "revenue",
    );

    let matches: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::ContractRetainDepartedInvalid))
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "expected exactly one ContractRetainDepartedInvalid, got {diags:?}"
    );
}

/// A `{tombstone: <col>}` form naming a column absent from the model's
/// output is refused.
#[test]
fn retain_departed_tombstone_column_absent_from_output_raises_diagnostic() {
    let model = r#"---
materialization: table
refresh: incremental
grain: key
unique_key: [customer_id]
contract:
  retain_departed:
    tombstone: is_departed
---
SELECT
    c.customer_id,
    c.tier
FROM smelt.sources.customers c
"#;

    let diags = diagnostics_for(
        &[
            ("smelt.yml", SMELT_YML),
            ("models/sources/customers.yml", SNAPSHOT_SOURCE),
            ("models/dim_customers.sql", model),
        ],
        "dim_customers",
    );

    let matches: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::ContractRetainDepartedInvalid))
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "expected exactly one ContractRetainDepartedInvalid, got {diags:?}"
    );
    assert!(
        matches[0].message.contains("is_departed"),
        "message must name the missing tombstone column, got: {}",
        matches[0].message
    );
}
