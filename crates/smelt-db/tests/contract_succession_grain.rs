//! Phase 3 (`docs/outcomes/20260906-scd2-keyed-succession/phases/03-plan.md`):
//! contract-lattice posture for the keyed-succession grain — `frozen_horizon`/
//! `retain_departed` are refused by the existing grain rules, now naming the
//! succession grain rather than the pre-succession `Key` fallback;
//! `deferral` is admitted unchanged, since a succession model always carries
//! a classifier-derived clock. Mirrors `contract_frozen_horizon_diagnostics.rs`'s
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
name: contract_succession_grain_fixture
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

const CUSTOMER_CHANGES_SOURCE: &str = r#"
description: Customer change events, append-only, clocked on changed_at.
mutation_profile: append_only
timeseries:
  event_time_column: changed_at
  partition_column: changed_at
  granularity: day
columns:
  - { name: customer_id, type: INTEGER, nullable: false }
  - { name: changed_at, type: TIMESTAMP, nullable: false }
  - { name: name, type: VARCHAR, nullable: true }
"#;

const SUCCESSION_MODEL_SQL: &str = "\
    customer_id,\n\
    changed_at,\n\
    name,\n\
    LEAD(changed_at) OVER (PARTITION BY customer_id ORDER BY changed_at) AS next_changed_at\n\
FROM smelt.sources.customer_changes\n";

fn succession_files(contract_block: &str) -> Vec<(&'static str, String)> {
    let model = format!(
        "---\nmaterialization: table\nrefresh: incremental\ncontract:\n{contract_block}\n---\nSELECT\n{SUCCESSION_MODEL_SQL}"
    );
    vec![
        ("smelt.yml", SMELT_YML.to_string()),
        (
            "models/sources/customer_changes.yml",
            CUSTOMER_CHANGES_SOURCE.to_string(),
        ),
        ("models/customer_history.sql", model),
    ]
}

/// `frozen_horizon:` declared on an undeclared-grain model that classifies
/// as keyed-succession is refused, naming the succession grain — not the
/// pre-succession `Key` fallback `metadata.grain.unwrap_or(Grain::Key)`
/// produced (which happened to also refuse, just with a misleading message).
#[test]
fn frozen_horizon_refused_on_a_succession_model() {
    let files = succession_files("  frozen_horizon: '90 days'");
    let files_ref: Vec<(&str, &str)> = files.iter().map(|(p, c)| (*p, c.as_str())).collect();
    let diags = diagnostics_for(&files_ref, "customer_history");

    let matches: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::ContractFrozenHorizonInvalid))
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "expected exactly one ContractFrozenHorizonInvalid, got {diags:?}"
    );
    assert!(
        matches[0].message.contains("succession"),
        "message must name the succession grain, got: {}",
        matches[0].message
    );
}

/// `retain_departed:` declared on a succession model is refused, naming the
/// succession grain.
#[test]
fn retain_departed_refused_on_a_succession_model() {
    let files = succession_files("  retain_departed: true");
    let files_ref: Vec<(&str, &str)> = files.iter().map(|(p, c)| (*p, c.as_str())).collect();
    let diags = diagnostics_for(&files_ref, "customer_history");

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
        matches[0].message.contains("succession"),
        "message must name the succession grain, got: {}",
        matches[0].message
    );
}

/// `deferral:` declared on a succession model is admitted — a succession
/// model always carries a classifier-derived clock, even though it declares
/// no `timeseries:` block of its own.
#[test]
fn deferral_admitted_on_a_succession_model() {
    let files = succession_files("  deferral: '6 hours'");
    let files_ref: Vec<(&str, &str)> = files.iter().map(|(p, c)| (*p, c.as_str())).collect();
    let diags = diagnostics_for(&files_ref, "customer_history");

    let matches: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::ContractDeferralInvalid))
        .collect();
    assert!(
        matches.is_empty(),
        "expected no ContractDeferralInvalid, got {diags:?}"
    );
}
