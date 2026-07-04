//! Phase 2 parity tests for the planner safety check + schema evolution flags
//! lifted into `smelt_runtime`.
//!
//! These tests verify that `smelt_runtime::safety` exposes the same refusal
//! behaviour the CLI previously owned inline in `commands/run.rs`:
//!
//! 1. An unsafe incremental model is refused when `enforce_safety = true`.
//! 2. The same model runs with warnings when `enforce_safety = false`.
//! 3. An undefinable temporal bound is refused when `enforce_safety = true`.
//! 4. A schema-evolution `ColumnRemovalBlocked` result propagates as `Err`
//!    unless `allow_column_removal = true`.

use smelt_core::config::TimeseriesConfig;
use smelt_core::{BatchedConfig, BatchedSafetyOverrides, Granularity};
use smelt_planner::{ModelGraph, ModelInfo};
use smelt_runtime::safety::{
    check_bound_derivation, check_planner_safety, should_force_full_refresh,
};
use smelt_runtime::schema_evolution::SchemaEvolutionResult;

// ── helpers ──────────────────────────────────────────────────────────────────

fn make_ts(event_col: &str, partition_col: &str) -> TimeseriesConfig {
    TimeseriesConfig {
        event_time_column: event_col.to_string(),
        partition_column: partition_col.to_string(),
        granularity: Granularity::Day,
        week_start: None,
    }
}

fn make_inc() -> BatchedConfig {
    BatchedConfig {
        unique_key: vec![],
        safety_overrides: BatchedSafetyOverrides::default(),
    }
}

/// A ModelGraph whose only model has `incremental_config` but no
/// `timeseries_config` — the planner's incremental rule requires both.
fn unsafe_incremental_graph() -> ModelGraph {
    let mut graph = ModelGraph::new();
    graph.add_model(ModelInfo {
        name: "unsafe_model".into(),
        sql: "SELECT a, SUM(b) as total FROM events GROUP BY a".into(),
        refs: vec![],
        timeseries_config: None,
        incremental_config: Some(make_inc()),
    });
    graph
}

/// A ModelGraph with a model whose SQL has a bare `LAG()` (no `RANGE BETWEEN
/// INTERVAL`) over a timeseries source, so `derive_model_source_bounds`
/// returns `NotDerivable`.
fn undefinable_bound_graph() -> ModelGraph {
    let mut graph = ModelGraph::new();

    // Upstream timeseries source
    graph.add_model(ModelInfo {
        name: "silver.events".into(),
        sql: "SELECT event_date, user_id FROM raw_events".into(),
        refs: vec![],
        timeseries_config: Some(make_ts("event_date", "event_date")),
        incremental_config: None,
    });

    // Downstream model: bare LAG without RANGE BETWEEN → NotDerivable
    graph.add_model(ModelInfo {
        name: "downstream".into(),
        sql: "SELECT id, ts, LAG(x) OVER (PARTITION BY id ORDER BY ts) AS prev_x \
              FROM silver.events"
            .into(),
        refs: vec!["silver.events".into()],
        timeseries_config: Some(make_ts("ts", "ts")),
        incremental_config: Some(make_inc()),
    });

    graph
}

// ── Test 1 — unsafe incremental refused by default ───────────────────────────

#[test]
fn test_unsafe_incremental_refused_by_default() {
    let graph = unsafe_incremental_graph();
    let result = check_planner_safety(&graph, true);
    assert!(
        result.is_err(),
        "expected Err for model with incremental: but no timeseries:"
    );
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("unsafe_model") || msg.contains("incremental"),
        "error should mention model name or incremental issue: {msg}"
    );
}

// ── Test 2 — enforce_safety = false allows unsafe model (with warning) ────────

#[test]
fn test_unsafe_incremental_allowed_with_enforce_safety_false() {
    let graph = unsafe_incremental_graph();
    let result = check_planner_safety(&graph, false);
    assert!(
        result.is_ok(),
        "enforce_safety=false should return Ok, got: {:?}",
        result
    );
    // Transformations may be empty (planner couldn't plan the broken model),
    // but the call must not error.
}

// ── Test 3 — undefinable bound refused by default ─────────────────────────────

#[test]
fn test_undefinable_bound_refused_by_default() {
    let graph = undefinable_bound_graph();
    let result = check_bound_derivation(&graph, true);
    assert!(
        result.is_err(),
        "expected Err for model with bare LAG (NotDerivable bound)"
    );
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("downstream") || msg.contains("bound") || msg.contains("temporal"),
        "error should mention model or bound issue: {msg}"
    );
}

#[test]
fn test_undefinable_bound_allowed_with_enforce_safety_false() {
    let graph = undefinable_bound_graph();
    let result = check_bound_derivation(&graph, false);
    assert!(
        result.is_ok(),
        "enforce_safety=false should return Ok even for undefinable bound, got: {:?}",
        result
    );
}

// ── Test 4 — schema evolution column removal blocked by default ───────────────

#[test]
fn test_schema_evolution_blocks_column_removal_by_default() {
    let result = SchemaEvolutionResult::ColumnRemovalBlocked {
        columns: vec!["old_col".to_string()],
    };
    let err = should_force_full_refresh(&result, "my_model", false, false);
    assert!(
        err.is_err(),
        "ColumnRemovalBlocked should be Err when allow_column_removal=false"
    );
    let msg = err.unwrap_err().to_string();
    assert!(
        msg.contains("old_col") || msg.contains("allow-column-removal"),
        "error should mention the column or the flag: {msg}"
    );
}

#[test]
fn test_schema_evolution_column_removal_allowed_with_flag() {
    let result = SchemaEvolutionResult::ColumnRemovalBlocked {
        columns: vec!["old_col".to_string()],
    };
    let force = should_force_full_refresh(&result, "my_model", true, false);
    assert!(
        force.unwrap(),
        "ColumnRemovalBlocked with allow_column_removal=true should force full refresh"
    );
}

#[test]
fn test_schema_evolution_full_refresh_blocked_by_default() {
    let result = SchemaEvolutionResult::FullRefreshBlocked {
        reason: "type change".to_string(),
    };
    let err = should_force_full_refresh(&result, "my_model", false, false);
    assert!(
        err.is_err(),
        "FullRefreshBlocked should be Err when allow_full_refresh=false"
    );
    let msg = err.unwrap_err().to_string();
    assert!(
        msg.contains("allow-full-refresh") || msg.contains("full refresh"),
        "error should mention the flag: {msg}"
    );
}

#[test]
fn test_schema_evolution_full_refresh_allowed_with_flag() {
    let result = SchemaEvolutionResult::FullRefreshBlocked {
        reason: "type change".to_string(),
    };
    let force = should_force_full_refresh(&result, "my_model", false, true);
    assert!(
        force.unwrap(),
        "FullRefreshBlocked with allow_full_refresh=true should force full refresh"
    );
}

#[test]
fn test_schema_evolution_no_change_does_not_force_refresh() {
    let result = SchemaEvolutionResult::NoChange;
    assert!(!should_force_full_refresh(&result, "my_model", false, false).unwrap());

    let result = SchemaEvolutionResult::FirstDeployment;
    assert!(!should_force_full_refresh(&result, "my_model", false, false).unwrap());
}

#[test]
fn test_schema_evolution_full_refresh_required_forces_refresh() {
    let result = SchemaEvolutionResult::FullRefreshRequired {
        reason: "column type changed".to_string(),
    };
    assert!(should_force_full_refresh(&result, "my_model", false, false).unwrap());
}
