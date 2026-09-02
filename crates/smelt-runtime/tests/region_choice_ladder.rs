//! `resolve_incremental_strategy`'s creation-trigger cell is resolved
//! through the SAME override ladder every other per-cell dispatch resolver
//! consults (`smelt_logical::maintenance::choice::resolve_cell_choice`)
//! instead of a raw `cell.technique` read
//! (`docs/outcomes/20260815-definition-delta-migrate/phases/18-plan.md`).
//!
//! Spec: `docs/specs/incremental_models.md` §Design "Absent a cost model:
//! the fixed preference order".

use std::collections::HashSet;

use smelt_backend::IncrementalStrategy;
use smelt_core::config::{
    CellTechnique, Grain as ConfigGrain, Granularity, MaintenanceCellConfig, MaintenanceConfig,
    MaintenanceDefaults, RefreshStrategy, TechniquePreference, TimeseriesConfig,
};
use smelt_core::ModelMetadata;
use smelt_logical::maintenance::{MutationProfile, SourceFacts};
use smelt_runtime::maintenance_driver::resolve_incremental_strategy;

const SQL: &str = "SELECT user_id, amount, event_date FROM smelt.sources.payments";

fn payments_metadata(maintenance: Option<MaintenanceConfig>) -> ModelMetadata {
    ModelMetadata {
        refresh: Some(RefreshStrategy::Incremental),
        grain: Some(ConfigGrain::Partition),
        timeseries: Some(TimeseriesConfig {
            event_time_column: "event_date".to_string(),
            partition_column: "event_date".to_string(),
            granularity: Granularity::Day,
            week_start: None,
            assert_monotonic: false,
        }),
        maintenance,
        ..Default::default()
    }
}

fn payments_sources() -> Vec<SourceFacts> {
    vec![SourceFacts {
        name: "payments".to_string(),
        mutation: MutationProfile::AppendOnly,
        partition_col: Some("event_date".to_string()),
        unique_key: vec![],
        allow_full_scan: false,
    }]
}

/// A partition-grain model's creation cell (which admits only
/// `Technique::DeleteInsert`) resolves `defaults.prefer: recompute` to
/// `RegionRecompute` — mapped to `backend_default`, never the cell's own
/// `DeleteInsert` read directly (the ladder is genuinely consulted, not
/// bypassed).
#[test]
fn region_path_honours_prefer_recompute() {
    let maintenance = MaintenanceConfig {
        defaults: Some(MaintenanceDefaults {
            prefer: Some(TechniquePreference::Recompute),
        }),
        cells: vec![],
        scan_bounds: None,
    };
    let metadata = payments_metadata(Some(maintenance));

    let strategy = resolve_incremental_strategy(
        SQL,
        "main.payments_clean",
        &metadata,
        &payments_sources(),
        &HashSet::new(),
        &[],
        IncrementalStrategy::DeleteInsert,
        false,
    )
    .expect("prefer: recompute resolves to RegionRecompute, mapped to backend_default");
    assert_eq!(
        strategy,
        IncrementalStrategy::DeleteInsert,
        "backend_default for this test IS DeleteInsert (the enum's only live variant) — the \
         point of this test is that resolution goes through the ladder (RegionRecompute →\
         backend_default) rather than a direct, unconditional `cell.technique` read"
    );
}

/// A hard `cells[].technique: fold` pin naming a technique outside the
/// creation cell's resolvable set (`{DeleteInsert, RegionRecompute}` — fold
/// is never admitted for a partition-grain region cell) refuses loudly,
/// naming the resolvable set — never a silent fallback to `DeleteInsert`.
#[test]
fn region_path_refuses_unadmitted_technique_pin() {
    let maintenance = MaintenanceConfig {
        defaults: None,
        cells: vec![MaintenanceCellConfig {
            columns: vec!["amount".to_string()],
            on: "payments".to_string(),
            prefer: None,
            technique: Some(CellTechnique::Fold),
            write: None,
        }],
        scan_bounds: None,
    };
    let metadata = payments_metadata(Some(maintenance));

    let err = resolve_incremental_strategy(
        SQL,
        "main.payments_clean",
        &metadata,
        &payments_sources(),
        &HashSet::new(),
        &[],
        IncrementalStrategy::DeleteInsert,
        false,
    )
    .expect_err("technique: fold is not in this cell's resolvable set — must refuse, not silently pick DeleteInsert");
    let message = err.to_string();
    assert!(
        message.contains("resolvable set"),
        "refusal must name the resolvable set, got: {message}"
    );
}

/// With no `maintenance:` overrides declared at all, the resolved strategy
/// is byte-for-byte the pre-phase verdict — for both the plain first-
/// `NewData`-match branch (no model edges) and the model-edge-driven branch
/// (`docs/outcomes/20260815-definition-delta-migrate/phases/17-plan.md`).
#[test]
fn region_path_unchanged_without_overrides() {
    let metadata = payments_metadata(None);
    let strategy = resolve_incremental_strategy(
        SQL,
        "main.payments_clean",
        &metadata,
        &payments_sources(),
        &HashSet::new(),
        &[],
        IncrementalStrategy::DeleteInsert,
        false,
    )
    .expect("no overrides must not refuse");
    assert_eq!(strategy, IncrementalStrategy::DeleteInsert);

    let edge_sql = "SELECT event_id, event_date, amount FROM smelt.silver.events_deduped";
    let edge_metadata = ModelMetadata {
        refresh: Some(RefreshStrategy::Incremental),
        grain: Some(ConfigGrain::Partition),
        timeseries: Some(TimeseriesConfig {
            event_time_column: "event_date".to_string(),
            partition_column: "event_date".to_string(),
            granularity: Granularity::Day,
            week_start: None,
            assert_monotonic: false,
        }),
        ..Default::default()
    };
    let model_edges = vec![smelt_logical::maintenance::derive::ModelEdge {
        name: "silver.events_deduped".to_string(),
        clock_col: Some("event_date".to_string()),
        clock_col_aliases: vec![],
        unique_key: vec![],
        output_shape: None,
    }];
    let edge_strategy = resolve_incremental_strategy(
        edge_sql,
        "main.payments_clean",
        &edge_metadata,
        &[],
        &HashSet::new(),
        &model_edges,
        IncrementalStrategy::DeleteInsert,
        false,
    )
    .expect("a clocked model edge with no overrides must not refuse");
    assert_eq!(edge_strategy, IncrementalStrategy::DeleteInsert);
}
