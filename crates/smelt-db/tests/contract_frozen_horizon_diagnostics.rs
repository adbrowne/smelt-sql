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
