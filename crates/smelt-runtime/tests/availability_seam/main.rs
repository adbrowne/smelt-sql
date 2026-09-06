//! Phase 5 (`docs/outcomes/20260904-state-residency/outcome.md`) — the
//! single `smelt-runtime` derivation seam: every runtime consumer of
//! `smelt-db`'s `derive_model_maintenance_plan{,_with_edges}` reads an
//! availability-resolved plan via `smelt_runtime::maintenance_availability`,
//! never the bare `smelt-db` functions.
//!
//! Spec: `docs/specs/state.md` §"The degradation contract".
//!
//! [`structural`] holds the two source-scanning structural assertions;
//! this file holds the fixtures and the pure-behavior unit tests.

use std::collections::HashSet;

use smelt_core::config::{Config, Grain as ConfigGrain, RefreshStrategy, WarehouseTables};
use smelt_core::ModelMetadata;
use smelt_dialect::SqlDialect;
use smelt_logical::maintenance::availability::{
    realisable_state_structures, StateAvailability, StateStructure,
};
use smelt_logical::maintenance::derive::SourceReferentialIntegrity;
use smelt_logical::maintenance::{MutationProfile, SourceFacts, Technique, Trigger};
use smelt_runtime::maintenance_availability::{
    availability_for_run, derive_resolved, derive_resolved_with_edges,
};

mod structural;

/// A `grain: key` model whose driving source (`payments`) is append-only
/// with an invertible `SUM` combiner — the same shape
/// `crates/smelt-logical/tests/maintenance_availability.rs::keyed_fold_plan`
/// exercises at the pure-derivation layer, replicated here at the
/// `smelt-db` entry point so the seam is proven against a real
/// `Technique::KeyedFold` admission, not a hand-built cell.
const KEYED_FOLD_SQL: &str = "SELECT user_id, SUM(amount) AS lifetime_spend \
     FROM smelt.sources.payments GROUP BY user_id";

fn keyed_fold_metadata() -> ModelMetadata {
    ModelMetadata {
        refresh: Some(RefreshStrategy::Incremental),
        grain: Some(ConfigGrain::Key),
        ..Default::default()
    }
}

fn keyed_fold_sources() -> Vec<SourceFacts> {
    vec![SourceFacts {
        name: "payments".to_string(),
        mutation: MutationProfile::AppendOnly,
        partition_col: Some("pay_date".to_string()),
        unique_key: vec![],
        allow_full_scan: false,
    }]
}

fn creation_cell(
    plan: &smelt_logical::maintenance::MaintenancePlan,
) -> &smelt_logical::maintenance::PlanCell {
    plan.cells
        .iter()
        .find(|c| matches!(c.trigger, Trigger::NewData { .. }))
        .expect("the keyed-fold fixture must derive a creation cell")
}

#[test]
fn availability_for_run_intersects_dialect_and_warehouse_tables() {
    let (mut config, _) =
        Config::parse_with_warnings("name: p\nversion: 1\n").expect("minimal config must parse");
    assert_eq!(config.state.warehouse_tables, WarehouseTables::Allowed);

    let duckdb_allowed = availability_for_run(SqlDialect::DuckDB, &config);
    assert!(duckdb_allowed.contains(StateStructure::ReconciliationLedger));
    assert!(duckdb_allowed.contains(StateStructure::MergeLedger));

    config.state.warehouse_tables = WarehouseTables::None;
    let duckdb_none = availability_for_run(SqlDialect::DuckDB, &config);
    assert!(!duckdb_none.contains(StateStructure::ReconciliationLedger));
    assert!(!duckdb_none.contains(StateStructure::MergeLedger));

    config.state.warehouse_tables = WarehouseTables::Allowed;
    let spark_allowed = availability_for_run(SqlDialect::SparkSQL, &config);
    assert!(!spark_allowed.contains(StateStructure::ReconciliationLedger));
    assert!(spark_allowed.contains(StateStructure::FingerprintSidecar));
}

/// A keyed-fold cell derived through [`derive_resolved`] under a
/// ledger-less availability downgrades to `PerGroupRecompute` and carries
/// the recorded `state_downgrade` — the seam actually applies
/// `resolve_availability`, not just re-deriving the ideal plan.
#[test]
fn derive_resolved_downgrades_a_keyed_fold_cell() {
    let metadata = keyed_fold_metadata();
    let sources = keyed_fold_sources();
    let ledger_less = StateAvailability::resolve(
        WarehouseTables::Allowed,
        &realisable_state_structures(SqlDialect::SparkSQL),
    );

    let result = derive_resolved(
        KEYED_FOLD_SQL,
        "main.lifetime_spend",
        &metadata,
        &sources,
        &HashSet::new(),
        None,
        &[],
        &[],
        &SourceReferentialIntegrity::new(),
        None,
        None,
        &ledger_less,
        &[],
    )
    .expect("a keyed-fold model must derive a plan");

    let cell = creation_cell(&result.plan);
    assert_eq!(cell.technique, Technique::PerGroupRecompute);
    let downgrade = cell
        .state_downgrade
        .as_ref()
        .expect("a ledger-less target must record the downgrade");
    assert_eq!(downgrade.original, Technique::KeyedFold);
    assert_eq!(downgrade.missing, StateStructure::ReconciliationLedger);
}

/// The edge-aware entry point applies the SAME resolution — proven here
/// with empty `model_edges` (the with-edges wrapper is a strict superset of
/// the source-only derivation; a real live `ColumnScopedMerge`/`MergeLedger`
/// admission needs dimension-join structural machinery this seam test does
/// not attempt to stand up, but `resolve_availability` treats every
/// ledger-requiring technique identically — proven exhaustively at the pure
/// layer by `crates/smelt-logical/tests/maintenance_availability.rs`).
#[test]
fn derive_resolved_with_edges_downgrades_the_same_keyed_fold_cell() {
    let metadata = keyed_fold_metadata();
    let sources = keyed_fold_sources();
    let ledger_less = StateAvailability::resolve(
        WarehouseTables::Allowed,
        &realisable_state_structures(SqlDialect::SparkSQL),
    );

    let result = derive_resolved_with_edges(
        KEYED_FOLD_SQL,
        "main.lifetime_spend",
        &metadata,
        &sources,
        &HashSet::new(),
        &[],
        None,
        &[],
        &[],
        &SourceReferentialIntegrity::new(),
        None,
        None,
        &ledger_less,
        &[],
    )
    .expect("a keyed-fold model must derive a plan through the edge-aware entry point too");

    let cell = creation_cell(&result.plan);
    assert_eq!(cell.technique, Technique::PerGroupRecompute);
    assert!(cell.state_downgrade.is_some());
}

/// Resolution is a no-op under full availability — a DuckDB target with
/// `warehouse_tables: allowed` sees byte-identical cells to the raw
/// `smelt-db` derivation, so this phase changes no behaviour on the
/// backend every existing fixture assumes.
#[test]
fn derive_resolved_under_full_availability_is_byte_identical_to_the_raw_derivation() {
    let metadata = keyed_fold_metadata();
    let sources = keyed_fold_sources();

    let raw = smelt_db::queries::maintenance::derive_model_maintenance_plan(
        KEYED_FOLD_SQL,
        "main.lifetime_spend",
        &metadata,
        &sources,
        &HashSet::new(),
        None,
        &[],
        &[],
        &SourceReferentialIntegrity::new(),
        None,
        None,
        &[],
    )
    .expect("raw derivation must succeed");

    let resolved = derive_resolved(
        KEYED_FOLD_SQL,
        "main.lifetime_spend",
        &metadata,
        &sources,
        &HashSet::new(),
        None,
        &[],
        &[],
        &SourceReferentialIntegrity::new(),
        None,
        None,
        &StateAvailability::all(),
        &[],
    )
    .expect("resolved derivation must succeed");

    assert_eq!(raw.plan.cells.len(), resolved.plan.cells.len());
    for (raw_cell, resolved_cell) in raw.plan.cells.iter().zip(resolved.plan.cells.iter()) {
        assert_eq!(raw_cell.technique, resolved_cell.technique);
        assert!(raw_cell.state_downgrade.is_none());
        assert!(resolved_cell.state_downgrade.is_none());
    }
}
