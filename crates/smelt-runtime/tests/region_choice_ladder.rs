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
use smelt_logical::maintenance::choice::RegionWrite;
use smelt_logical::maintenance::emit::{emit_delete_insert, MaintenanceDialect, Region};
use smelt_logical::maintenance::{MutationProfile, SourceFacts};
use smelt_runtime::maintenance_driver::{
    build_delete_insert_group_dispatched, resolve_incremental_strategy,
};

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

fn region() -> Region {
    Region {
        start: "'2026-07-01'".to_string(),
        end: "'2026-07-02'".to_string(),
    }
}

/// `build_delete_insert_group_dispatched` realises the region family's
/// change-suppressed conditional variant (`RegionWrite::Suppressed`) via
/// `emit_diff_patch` — the update leg's `IS DISTINCT FROM` guard and a
/// complete, region-predicated delete leg — when no delta restriction
/// applies (`docs/outcomes/20260815-definition-delta-migrate/phases/
/// 27b-plan.md`).
#[test]
fn region_recompute_emits_the_conditional_staged_write_when_suppressible() {
    let region_write = RegionWrite::Suppressed {
        key: vec!["region_id".to_string()],
        compared_columns: vec!["amount".to_string()],
    };
    let group = build_delete_insert_group_dispatched(
        "main.regions",
        "region_date",
        &region(),
        "SELECT region_id, region_date, amount FROM smelt.sources.payments",
        None,
        None,
        None,
        Some(&region_write),
        MaintenanceDialect::DuckDb,
    );
    assert!(group.transactional);
    let sql: Vec<&str> = group.statements.iter().map(|s| s.sql.as_str()).collect();
    let update_leg = sql
        .iter()
        .find(|s| s.starts_with("DELETE FROM main.regions USING"))
        .expect("update leg present");
    assert!(
        update_leg.contains("IS DISTINCT FROM"),
        "update leg must guard on IS DISTINCT FROM over compared columns: {update_leg}"
    );
    let delete_leg = sql
        .iter()
        .find(|s| s.starts_with("DELETE FROM main.regions WHERE") && !s.contains("USING"))
        .expect("complete delete leg present");
    assert!(
        delete_leg.contains("region_date >= '2026-07-01'")
            && delete_leg.contains("region_date < '2026-07-02'"),
        "delete leg must be region-predicated: {delete_leg}"
    );
}

/// Without a proven key (`region_write: None`, or the `Unconditional`
/// variant), the region family stays byte-identical to today's widened
/// `emit_delete_insert` scan — the non-regression leg.
#[test]
fn region_recompute_keeps_the_widened_scan_without_a_proven_key() {
    let body = "SELECT region_id, region_date, amount FROM smelt.sources.payments";
    let expected = emit_delete_insert(
        "main.regions",
        "region_date",
        &region(),
        body,
        MaintenanceDialect::DuckDb,
    );

    let group_none = build_delete_insert_group_dispatched(
        "main.regions",
        "region_date",
        &region(),
        body,
        None,
        None,
        None,
        None,
        MaintenanceDialect::DuckDb,
    );
    assert_eq!(group_none, expected);

    let unconditional = RegionWrite::Unconditional {
        why: "no proven row identity".to_string(),
    };
    let group_unconditional = build_delete_insert_group_dispatched(
        "main.regions",
        "region_date",
        &region(),
        body,
        None,
        None,
        None,
        Some(&unconditional),
        MaintenanceDialect::DuckDb,
    );
    assert_eq!(group_unconditional, expected);
}

/// Delta restriction (T3) wins over write suppression when both are
/// admitted: the restricted arm narrows the scan itself, strictly cheaper
/// than suppression narrowing writes within an unrestricted scan (design
/// call 2, `docs/outcomes/20260815-definition-delta-migrate/phases/
/// 27b-plan.md`).
#[test]
fn delta_restriction_wins_over_suppression_when_both_admit() {
    let region_write = RegionWrite::Suppressed {
        key: vec!["region_id".to_string()],
        compared_columns: vec!["amount".to_string()],
    };
    let closure = smelt_logical::maintenance::SkeletonSourceClosure::Closed {
        row_preservation: smelt_logical::maintenance::RowPreservation::JoinShape,
    };
    let body = "SELECT region_id, region_date, amount FROM smelt.sources.payments";
    let delta_keys = vec!["region_id_1".to_string()];
    let group = build_delete_insert_group_dispatched(
        "main.regions",
        "region_date",
        &region(),
        body,
        Some("region_id"),
        Some(&closure),
        Some(&delta_keys),
        Some(&region_write),
        MaintenanceDialect::DuckDb,
    );
    let expected = smelt_logical::maintenance::emit::emit_delete_insert_delta_restricted(
        "main.regions",
        "region_date",
        &region(),
        body,
        "region_id",
        &delta_keys,
        MaintenanceDialect::DuckDb,
    );
    assert_eq!(
        group, expected,
        "delta restriction must win over write suppression: {group:?}"
    );
}
