//! `resolve_incremental_strategy` becomes edge-aware
//! (`docs/outcomes/20260815-definition-delta-migrate/phases/17-plan.md`):
//! the partition-addressed maintained-model creation cell
//! (`Trigger::NewData { source: <upstream model> }`) gains a real execution
//! technique, read from the SAME edge-aware derivation
//! (`derive_model_maintenance_plan_with_edges`) `resolve_live_delta_
//! restriction_facts` already uses — never a second derivation — and an
//! upstream edge refused `ReachNotDerivable` (no derivable clock) with no
//! other creation cell to fall back on is a fail-loud run refusal rather
//! than a silent region-recompute under `backend_default`.
//!
//! Spec: `docs/specs/incremental_models.md` §"Upstream model edges".

use std::collections::HashSet;

use smelt_backend::IncrementalStrategy;
use smelt_core::config::{Grain as ConfigGrain, Granularity, RefreshStrategy, TimeseriesConfig};
use smelt_core::ModelMetadata;
use smelt_logical::analysis::output_delta::OutputDelta;
use smelt_logical::maintenance::derive::ModelEdge;
use smelt_logical::maintenance::SourceFacts;
use smelt_runtime::maintenance_driver::{
    decide_column_merge_dispatch, resolve_incremental_strategy, resolve_live_column_scoped_cell,
};

fn partition_metadata() -> ModelMetadata {
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
        ..Default::default()
    }
}

fn clocked_edge() -> ModelEdge {
    ModelEdge {
        name: "silver.events_deduped".to_string(),
        clock_col: Some("event_date".to_string()),
        clock_col_aliases: vec![],
        unique_key: vec![],
        output_shape: None,
        allow_full_scan: false,
    }
}

fn clockless_edge() -> ModelEdge {
    ModelEdge {
        name: "silver.events_deduped".to_string(),
        clock_col: None,
        clock_col_aliases: vec![],
        unique_key: vec![],
        output_shape: None,
        allow_full_scan: false,
    }
}

/// A model whose ONLY creation-trigger input is a clocked upstream
/// maintained-model edge (no plain `sources:` at all) resolves its strategy
/// from the edge's own `Trigger::NewData` cell — the cell `append_model_
/// edge_cells` derives — rather than falling back to `backend_default` for
/// lack of any cell (the pre-phase-17 behaviour, since the model's own
/// source-only derivation contributes no cells when `sources` is empty).
#[test]
fn model_edge_creation_cell_drives_the_incremental_strategy() {
    let sql = "SELECT event_id, event_date, amount FROM smelt.silver.events_deduped";
    let metadata = partition_metadata();
    let model_edges = vec![clocked_edge()];

    let strategy = resolve_incremental_strategy(
        sql,
        "main.payments_clean",
        &metadata,
        &[],
        &HashSet::new(),
        &model_edges,
        IncrementalStrategy::DeleteInsert,
        false,
        &smelt_logical::maintenance::availability::StateAvailability::all(),
    )
    .expect("a clocked model edge must admit DeleteInsert, not refuse");
    assert_eq!(strategy, IncrementalStrategy::DeleteInsert);
}

/// A maintained upstream with no `timeseries:` and no `KeyedUpsert` output
/// shape (so neither the clock-based nor the key-addressed route in
/// `append_model_edge_cells` admits anything) records a `Refusal::
/// ReachNotDerivable` naming the edge — and with no OTHER `Trigger::NewData`
/// cell to fall back on (no plain `sources:`), `resolve_incremental_strategy`
/// must fail loud rather than silently return `backend_default` (which would
/// execute a region-recompute technique the plan never actually admitted for
/// this trigger).
#[test]
fn clockless_maintained_upstream_refuses_instead_of_silently_region_recomputing() {
    let sql = "SELECT event_id, event_date, amount FROM smelt.silver.events_deduped";
    let metadata = partition_metadata();
    let model_edges = vec![clockless_edge()];

    let err = resolve_incremental_strategy(
        sql,
        "main.payments_clean",
        &metadata,
        &[],
        &HashSet::new(),
        &model_edges,
        IncrementalStrategy::DeleteInsert,
        false,
        &smelt_logical::maintenance::availability::StateAvailability::all(),
    )
    .expect_err("a clockless model edge with no fallback cell must refuse, not silently default");
    let message = err.to_string();
    assert!(
        message.contains("silver.events_deduped"),
        "refusal must name the edge, got: {message}"
    );
}

/// The same clockless upstream edge alongside a plain, clocked `sources:`
/// entry that DOES admit its own `Trigger::NewData` cell — the refusal must
/// be narrow: since another creation-trigger cell is available, the run
/// still proceeds (falls through to that cell) instead of refusing the
/// whole model over an edge it does not actually need for this trigger.
#[test]
fn clockless_upstream_alongside_a_clocked_source_still_runs() {
    let sql = "SELECT event_id, event_date, amount FROM smelt.sources.payments \
               LEFT JOIN smelt.silver.events_deduped USING (event_id)";
    let metadata = partition_metadata();
    let model_edges = vec![clockless_edge()];
    let sources = vec![SourceFacts {
        name: "payments".to_string(),
        mutation: smelt_logical::maintenance::MutationProfile::AppendOnly,
        partition_col: Some("event_date".to_string()),
        unique_key: vec![],
        allow_full_scan: false,
    }];

    let strategy = resolve_incremental_strategy(
        sql,
        "main.payments_clean",
        &metadata,
        &sources,
        &HashSet::new(),
        &model_edges,
        IncrementalStrategy::DeleteInsert,
        false,
        &smelt_logical::maintenance::availability::StateAvailability::all(),
    )
    .expect(
        "a clockless model edge must not refuse the whole run when another creation-trigger \
         cell (the plain clocked source) is available",
    );
    assert_eq!(strategy, IncrementalStrategy::DeleteInsert);
}

// ── Phase 5 (`docs/outcomes/20260906-bigquery-correctness/phases/
// 05-plan.md`): the enrichment-keyed cell on the run path ─────────────────

/// A partition-addressed fact enriched by a clockless keyed dimension — same
/// fixture shape as `crates/smelt-logical/tests/model_edge_enrichment_
/// mutation.rs`'s `ENRICHMENT_SQL`/`repo_dim_edge`.
const ENRICHMENT_SQL: &str = "SELECT f.event_id, f.event_date, f.repo_id, \
     dim.current_repo_name AS current_repo_name \
     FROM smelt.silver.events f \
     LEFT JOIN smelt.gold.repo_dim dim ON f.repo_id = dim.repo_id";

fn repo_dim_edge() -> ModelEdge {
    ModelEdge {
        name: "gold.repo_dim".to_string(),
        clock_col: None,
        clock_col_aliases: vec![],
        unique_key: vec!["repo_id".to_string()],
        output_shape: Some(OutputDelta::KeyedUpsert {
            keys: vec!["repo_id".to_string()],
        }),
        // `derive_model_maintenance_plan_with_edges` re-derives each edge's
        // effective `allow_full_scan` from THIS downstream's own declared
        // `maintenance.scan_bounds.per_source` (a fact of the downstream, not
        // the edge) — this field is overridden regardless of what is set
        // here, so `enrichment_metadata()` below is what actually licenses
        // the full scan.
        allow_full_scan: false,
    }
}

/// `partition_metadata()` plus a declared `maintenance.scan_bounds.
/// per_source["gold.repo_dim"].allow_full_scan: true` — the enrichment-keyed
/// route's own precondition (`admit_enrichment_keyed_merge` refuses
/// `ScanUnbounded` without it).
fn enrichment_metadata() -> ModelMetadata {
    let mut per_source = std::collections::HashMap::new();
    per_source.insert(
        "gold.repo_dim".to_string(),
        smelt_core::config::PerSourceScanBounds {
            max_lookback: None,
            allow_full_scan: true,
        },
    );
    ModelMetadata {
        maintenance: Some(smelt_core::config::MaintenanceConfig {
            defaults: None,
            cells: vec![],
            scan_bounds: Some(smelt_core::config::ScanBoundsConfig {
                require: None,
                on_violation: None,
                per_source,
            }),
        }),
        ..partition_metadata()
    }
}

/// Test 1: with `model_edges` supplied, `resolve_live_column_scoped_cell`
/// sees the enrichment-keyed cell `append_model_edge_cells` derives for a
/// clockless keyed upstream model edge in value-enrichment position — the
/// resolver's source-only derivation (no `model_edges` at all, the
/// pre-phase-5 behaviour) never sees it.
#[test]
fn resolve_live_column_scoped_cell_sees_an_enrichment_keyed_edge_cell() {
    let metadata = enrichment_metadata();
    let model_edges = vec![repo_dim_edge()];

    let resolved = resolve_live_column_scoped_cell(
        ENRICHMENT_SQL,
        "main.events_enriched",
        &metadata,
        &[],
        &HashSet::new(),
        &model_edges,
        true,
        &[],
        &smelt_logical::maintenance::availability::StateAvailability::all(),
    )
    .expect("resolution must not error")
    .expect("an enrichment-keyed cell must resolve live once model_edges is supplied");

    let (source, cell, _suppression) = resolved;
    assert_eq!(source, "gold.repo_dim");
    assert_eq!(
        cell.technique,
        smelt_logical::maintenance::Technique::ColumnScopedMerge
    );
    let key_scope = cell
        .key_scope
        .as_ref()
        .unwrap_or_else(|| panic!("expected a key_scope on {cell:?}"));
    assert_eq!(
        key_scope.discovery,
        smelt_logical::maintenance::KeyDiscovery::EnrichmentKeyed
    );

    // With an EMPTY edge slice (the pre-phase-5 call shape), the same cell
    // is invisible.
    let resolved_without_edges = resolve_live_column_scoped_cell(
        ENRICHMENT_SQL,
        "main.events_enriched",
        &metadata,
        &[],
        &HashSet::new(),
        &[],
        true,
        &[],
        &smelt_logical::maintenance::availability::StateAvailability::all(),
    )
    .expect("resolution must not error");
    assert!(
        resolved_without_edges.is_none(),
        "an enrichment-keyed edge cell must not resolve without model_edges: \
         {resolved_without_edges:?}"
    );
}

/// Test 2: `decide_column_merge_dispatch` excludes the `EnrichmentKeyed` cell
/// from the per-batch window-scoped corner entirely — its write is addressed
/// by its own join key, not a partition interval, so no batch may ever
/// window-scope it (`execute_enrichment_keyed_heal` is its only run path).
#[test]
fn an_enrichment_keyed_cell_is_excluded_from_the_window_scoped_dispatch() {
    let metadata = enrichment_metadata();
    let model_edges = vec![repo_dim_edge()];

    let (_source, cell, _suppression) = resolve_live_column_scoped_cell(
        ENRICHMENT_SQL,
        "main.events_enriched",
        &metadata,
        &[],
        &HashSet::new(),
        &model_edges,
        true,
        &[],
        &smelt_logical::maintenance::availability::StateAvailability::all(),
    )
    .expect("resolution must not error")
    .expect("an enrichment-keyed cell must resolve live");

    let dispatch = decide_column_merge_dispatch(
        &cell,
        "gold.repo_dim",
        true,
        true,
        &smelt_logical::analysis::join_shape::ContributionVerdict::Monotone,
    );
    assert_eq!(
        dispatch, None,
        "an EnrichmentKeyed cell must never resolve a window-scoped dispatch corner, got \
         {dispatch:?}"
    );
}
