//! Phase 4 (`docs/outcomes/20260809-contract-lattice-v1/phases/04-plan.md`):
//! `contract.deferral` clock-admissibility diagnostic
//! (`docs/specs/incremental_models.md` §"Contract relaxations
//! (`contract:`)") — a thin `smelt-db` Salsa wrapper around the pure
//! `smelt_logical::contract::deferral::validate_deferral` validator. Mirrors
//! `crates/smelt-db/tests/contract_frozen_horizon_diagnostics.rs`'s harness.

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
name: contract_deferral_fixture
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

const SNAPSHOT_SOURCE: &str = r#"
description: A mutable snapshot with no clock.
mutation_profile: mutable_snapshot
columns:
  - { name: customer_id, type: INTEGER, nullable: false }
  - { name: tier, type: VARCHAR, nullable: false }
"#;

const SMELT_YML_INTERVALS: &str = r#"
name: contract_deferral_fixture
version: 1

paths:
  - models

targets:
  dev:
    type: duckdb
    database: target/dev.duckdb
    schema: main

default_materialization: view

state:
  mode: intervals
"#;

const CLOCKED_DEFERRAL_MODEL: &str = r#"---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: order_date
  partition_column: order_date
  granularity: day
contract:
  deferral: '6 hours'
---
SELECT
    date_trunc('day', o.order_date) AS order_date,
    SUM(o.amount) AS total
FROM smelt.sources.orders o
GROUP BY 1
"#;

/// `deferral:` declared on a model with no `timeseries:` clock is refused —
/// there is no frontier to measure lag against.
#[test]
fn deferral_without_a_clock_raises_diagnostic() {
    let model = r#"---
materialization: table
refresh: incremental
grain: key
unique_key: [order_id]
contract:
  deferral: '6 hours'
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
        .filter(|d| d.code == Some(DiagnosticCode::ContractDeferralInvalid))
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "expected exactly one ContractDeferralInvalid, got {diags:?}"
    );
    assert!(
        matches[0].message.contains("model"),
        "message must name the offending model, got: {}",
        matches[0].message
    );
}

/// `deferral:` declared on a model with a `timeseries:` clock is clean.
#[test]
fn deferral_with_a_clock_is_clean() {
    let model = r#"---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: order_date
  partition_column: order_date
  granularity: day
contract:
  deferral: '6 hours'
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
        .filter(|d| d.code == Some(DiagnosticCode::ContractDeferralInvalid))
        .collect();
    assert!(
        matches.is_empty(),
        "expected no ContractDeferralInvalid, got {diags:?}"
    );
}

/// A `contract.cells[]` entry whose `on:` trigger is a `mutable_snapshot`
/// source has no interval representation — refused.
#[test]
fn cell_deferral_on_mutable_snapshot_trigger_raises_diagnostic() {
    let model = r#"---
materialization: table
refresh: incremental
grain: key
unique_key: [order_id]
contract:
  cells:
    - columns: [tier]
      on: customers
      deferral: '6 hours'
---
SELECT
    o.order_id,
    o.order_date,
    o.amount,
    c.tier
FROM smelt.sources.orders o
JOIN smelt.sources.customers c ON c.customer_id = o.order_id
"#;

    let diags = diagnostics_for(
        &[
            ("smelt.yml", SMELT_YML),
            ("models/sources/orders.yml", ORDERS_SOURCE),
            ("models/sources/customers.yml", SNAPSHOT_SOURCE),
            ("models/revenue.sql", model),
        ],
        "revenue",
    );

    let matches: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::ContractDeferralInvalid))
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "expected exactly one ContractDeferralInvalid, got {diags:?}"
    );
}

/// A `contract.cells[]` entry declaring `on: backfill` has no clock at all —
/// refused.
#[test]
fn cell_deferral_on_backfill_raises_diagnostic() {
    let model = r#"---
materialization: table
refresh: incremental
grain: key
unique_key: [order_id]
contract:
  cells:
    - columns: [amount]
      on: backfill
      deferral: '6 hours'
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
        .filter(|d| d.code == Some(DiagnosticCode::ContractDeferralInvalid))
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "expected exactly one ContractDeferralInvalid, got {diags:?}"
    );
}

/// `docs/outcomes/20260816-state-residency/phases/06-plan.md`: a
/// model-level `contract.deferral` under the project's default
/// `state.mode: stateless` posture is refused — the declared lag can never
/// be measured without the interval ledger and landed-delta record
/// (`docs/specs/state.md` §"Declarations stay fail-loud").
#[test]
fn deferral_under_stateless_posture_is_refused() {
    let diags = diagnostics_for(
        &[
            ("smelt.yml", SMELT_YML),
            ("models/sources/orders.yml", ORDERS_SOURCE),
            ("models/revenue.sql", CLOCKED_DEFERRAL_MODEL),
        ],
        "revenue",
    );

    let matches: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::DeclaredContractRequiresState))
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "expected exactly one DeclaredContractRequiresState, got {diags:?}"
    );
    assert!(
        matches[0].message.contains("contract.deferral"),
        "message must name the offending declaration, got: {}",
        matches[0].message
    );
    assert_eq!(matches[0].severity, smelt_db::DiagnosticSeverity::Error);
}

/// The same declaration under `state.mode: intervals` is clean — the
/// posture supplies the interval ledger and landed-delta record the
/// declaration's lag is measured against.
#[test]
fn deferral_under_intervals_posture_is_clean() {
    let diags = diagnostics_for(
        &[
            ("smelt.yml", SMELT_YML_INTERVALS),
            ("models/sources/orders.yml", ORDERS_SOURCE),
            ("models/revenue.sql", CLOCKED_DEFERRAL_MODEL),
        ],
        "revenue",
    );

    let matches: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::DeclaredContractRequiresState))
        .collect();
    assert!(
        matches.is_empty(),
        "expected no DeclaredContractRequiresState, got {diags:?}"
    );
}

/// A model that narrows its own `state:` block to `stateless` under an
/// `intervals` project is still refused — the check reads the *effective*
/// (narrowest) posture, not the project's alone (D-47 narrowing,
/// `docs/specs/state.md` §"`state.mode` and what each posture provides").
#[test]
fn model_narrowing_to_stateless_refuses_declared_deferral() {
    let model = r#"---
materialization: table
refresh: incremental
grain: partition
state:
  mode: stateless
timeseries:
  event_time_column: order_date
  partition_column: order_date
  granularity: day
contract:
  deferral: '6 hours'
---
SELECT
    date_trunc('day', o.order_date) AS order_date,
    SUM(o.amount) AS total
FROM smelt.sources.orders o
GROUP BY 1
"#;

    let diags = diagnostics_for(
        &[
            ("smelt.yml", SMELT_YML_INTERVALS),
            ("models/sources/orders.yml", ORDERS_SOURCE),
            ("models/revenue.sql", model),
        ],
        "revenue",
    );

    let matches: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::DeclaredContractRequiresState))
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "expected exactly one DeclaredContractRequiresState, got {diags:?}"
    );
}

/// A `contract.cells[].deferral` entry is refused per-cell under the
/// `stateless` posture, same as the model-level declaration.
#[test]
fn cell_deferral_under_stateless_posture_is_refused() {
    let model = r#"---
materialization: table
refresh: incremental
grain: key
unique_key: [order_id]
contract:
  cells:
    - columns: [amount]
      on: orders
      deferral: '1 day'
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
        .filter(|d| d.code == Some(DiagnosticCode::DeclaredContractRequiresState))
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "expected exactly one DeclaredContractRequiresState, got {diags:?}"
    );
    assert!(
        matches[0].message.contains("contract.cells"),
        "message must name the offending cell declaration, got: {}",
        matches[0].message
    );
}
