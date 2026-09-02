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
use smelt_logical::maintenance::derive::ModelEdge;
use smelt_logical::maintenance::SourceFacts;
use smelt_runtime::maintenance_driver::resolve_incremental_strategy;

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
    }
}

fn clockless_edge() -> ModelEdge {
    ModelEdge {
        name: "silver.events_deduped".to_string(),
        clock_col: None,
        clock_col_aliases: vec![],
        unique_key: vec![],
        output_shape: None,
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
    )
    .expect(
        "a clockless model edge must not refuse the whole run when another creation-trigger \
         cell (the plain clocked source) is available",
    );
    assert_eq!(strategy, IncrementalStrategy::DeleteInsert);
}
