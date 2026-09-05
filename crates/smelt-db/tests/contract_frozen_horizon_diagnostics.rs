//! Phase 2 (`docs/outcomes/20260809-contract-lattice-v1/phases/02-plan.md`):
//! `contract.frozen_horizon` grain-admissibility diagnostic
//! (`docs/specs/incremental_models.md` §"Contract relaxations
//! (`contract:`)") — a thin `smelt-db` Salsa wrapper around the pure
//! `smelt_logical::contract::frozen_horizon::validate_frozen_horizon`
//! validator. Mirrors `crates/smelt-db/tests/grain_check.rs`'s harness.

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
name: contract_frozen_horizon_fixture
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

const MUTABLE_ORDERS_SOURCE: &str = r#"
description: Orders, mutable snapshot.
mutation_profile: mutable_snapshot
columns:
  - { name: order_id, type: INTEGER, nullable: false }
  - { name: order_date, type: TIMESTAMP, nullable: false }
  - { name: amount, type: DOUBLE, nullable: false }
"#;

const MUTABLE_DIMENSION_SOURCE: &str = r#"
description: A mutable dimension joined, not driving.
mutation_profile: mutable_snapshot
columns:
  - { name: customer_id, type: INTEGER, nullable: false }
  - { name: segment, type: VARCHAR, nullable: false }
"#;

/// `frozen_horizon:` declared on a `grain: key` model is refused — the
/// declaration is admitted only on a partition-grain model.
#[test]
fn frozen_horizon_on_key_grain_model_raises_diagnostic() {
    let model = r#"---
materialization: table
refresh: incremental
grain: key
unique_key: [order_id]
contract:
  frozen_horizon: '90 days'
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
        .filter(|d| d.code == Some(DiagnosticCode::ContractFrozenHorizonInvalid))
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "expected exactly one ContractFrozenHorizonInvalid, got {diags:?}"
    );
}

/// `frozen_horizon:` declared on a `grain: partition` model is clean.
#[test]
fn frozen_horizon_on_partition_grain_model_is_clean() {
    let model = r#"---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: order_date
  partition_column: order_date
  granularity: day
contract:
  frozen_horizon: '90 days'
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
        .filter(|d| d.code == Some(DiagnosticCode::ContractFrozenHorizonInvalid))
        .collect();
    assert!(
        matches.is_empty(),
        "expected no ContractFrozenHorizonInvalid, got {diags:?}"
    );
}

/// `frozen_horizon:` declared on a partition-grain model driven by a
/// `mutation_profile: mutable_snapshot` source is refused — the late-arrival
/// probe's row-count comparison is blind under a non-append-only posture
/// (`docs/specs/incremental_models.md` §"The contract lattice").
#[test]
fn frozen_horizon_on_mutable_snapshot_source_raises_diagnostic() {
    let model = r#"---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: order_date
  partition_column: order_date
  granularity: day
contract:
  frozen_horizon: '90 days'
---
SELECT
    date_trunc('day', o.order_date) AS order_date,
    SUM(o.amount) AS total
FROM smelt.sources.mutable_orders o
GROUP BY 1
"#;

    let diags = diagnostics_for(
        &[
            ("smelt.yml", SMELT_YML),
            ("models/sources/mutable_orders.yml", MUTABLE_ORDERS_SOURCE),
            ("models/revenue.sql", model),
        ],
        "revenue",
    );

    let matches: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::ContractFrozenHorizonInvalid))
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "expected exactly one ContractFrozenHorizonInvalid, got {diags:?}"
    );
}

/// `frozen_horizon:` declared on a partition-grain model driven by the
/// existing append-only `orders` source is clean.
#[test]
fn frozen_horizon_on_append_only_source_is_clean() {
    let model = r#"---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: order_date
  partition_column: order_date
  granularity: day
contract:
  frozen_horizon: '90 days'
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
        .filter(|d| d.code == Some(DiagnosticCode::ContractFrozenHorizonInvalid))
        .collect();
    assert!(
        matches.is_empty(),
        "expected no ContractFrozenHorizonInvalid, got {diags:?}"
    );
}

/// A partition-grain model driven by the append-only `orders` source that
/// *joins* a `mutable_snapshot` dimension is clean — only the driving
/// relation (the FROM clause's first entry) is judged.
#[test]
fn frozen_horizon_joined_dimension_posture_ignored() {
    let model = r#"---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: order_date
  partition_column: order_date
  granularity: day
contract:
  frozen_horizon: '90 days'
---
SELECT
    date_trunc('day', o.order_date) AS order_date,
    SUM(o.amount) AS total
FROM smelt.sources.orders o
JOIN smelt.sources.customers c ON o.order_id = c.customer_id
GROUP BY 1
"#;

    let diags = diagnostics_for(
        &[
            ("smelt.yml", SMELT_YML),
            ("models/sources/orders.yml", ORDERS_SOURCE),
            ("models/sources/customers.yml", MUTABLE_DIMENSION_SOURCE),
            ("models/revenue.sql", model),
        ],
        "revenue",
    );

    let matches: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::ContractFrozenHorizonInvalid))
        .collect();
    assert!(
        matches.is_empty(),
        "expected no ContractFrozenHorizonInvalid, got {diags:?}"
    );
}
