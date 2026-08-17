//! BigQuery twin of `maintenance_conformance/dags.rs`. Thin wrapper over
//! `smelt_maintenance_testkit::families::dags`
//! (`docs/plans/20260817-bigquery-generative-conformance.md` Phase 5).
//!
//! Live run (2026-08-17): 1/5 passed
//! (`keyed_grain_node_excluded_from_generated_graph_on_bigquery`, which
//! stages only one project). The other 4 fail `409 Already Exists` —
//! `families::dags`'s two-project-per-case staging calls `b.target(case)`
//! twice with the SAME `case` for what should be two distinct projects,
//! colliding on BigQuery's per-case-dataset design. See `main.rs`'s doc
//! comment point 3.

use smelt_maintenance_testkit::families::{dags, ConformanceBackend};

use crate::backend::BigQueryConformanceBackend;

/// Default deterministic case count — smaller than the Spark leg's default
/// (3): each BigQuery case stages TWO independent projects (each its own
/// fresh dataset) and drives `execute_project` multiple times each, with
/// pacing between every write-ish step.
const DEFAULT_CASES: usize = 2;

fn case_count() -> usize {
    std::env::var("SMELT_CONFORMANCE_BQ_DAG_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_CASES)
}

/// `chain_since_upstream_dirty_set_suffices_on_bigquery`.
#[test]
fn chain_since_upstream_dirty_set_suffices_on_bigquery() {
    let b = BigQueryConformanceBackend::new("dags_chain");
    if let Some(reason) = b.skip_reason() {
        eprintln!("{reason} — skipping chain_since_upstream_dirty_set_suffices_on_bigquery");
        return;
    }
    b.preflight_or_panic();
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(dags::run_chain_since_upstream_dirty_set_suffices(
        &b,
        case_count(),
    ))
    .expect("chain propagation check failed on BigQuery");
}

/// `diamond_propagation_suffices_on_bigquery`.
#[test]
fn diamond_propagation_suffices_on_bigquery() {
    let b = BigQueryConformanceBackend::new("dags_diamond");
    if let Some(reason) = b.skip_reason() {
        eprintln!("{reason} — skipping diamond_propagation_suffices_on_bigquery");
        return;
    }
    b.preflight_or_panic();
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(dags::run_diamond_propagation_suffices(&b, case_count()))
        .expect("diamond propagation check failed on BigQuery");
}

/// `upstream_payload_in_downstream_skeleton_position_on_bigquery`.
#[test]
fn upstream_payload_in_downstream_skeleton_position_on_bigquery() {
    let b = BigQueryConformanceBackend::new("dags_leak");
    if let Some(reason) = b.skip_reason() {
        eprintln!(
            "{reason} — skipping upstream_payload_in_downstream_skeleton_position_on_bigquery"
        );
        return;
    }
    b.preflight_or_panic();
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(dags::run_upstream_payload_in_downstream_skeleton_position(
        &b,
        case_count(),
    ))
    .expect("leak-family propagation check failed on BigQuery");
}

/// `include_upstreams_resolved_slices_suffice_on_bigquery`.
#[test]
fn include_upstreams_resolved_slices_suffice_on_bigquery() {
    let b = BigQueryConformanceBackend::new("dags_include_upstreams");
    if let Some(reason) = b.skip_reason() {
        eprintln!("{reason} — skipping include_upstreams_resolved_slices_suffice_on_bigquery");
        return;
    }
    b.preflight_or_panic();
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(dags::run_include_upstreams_resolved_slices_suffice(
        &b,
        case_count(),
    ))
    .expect("backward-resolved slice check failed on BigQuery");
}

/// `keyed_grain_node_excluded_from_generated_graph_on_bigquery`.
#[test]
fn keyed_grain_node_excluded_from_generated_graph_on_bigquery() {
    let b = BigQueryConformanceBackend::new("dags_keyed_grain");
    if let Some(reason) = b.skip_reason() {
        eprintln!("{reason} — skipping keyed_grain_node_excluded_from_generated_graph_on_bigquery");
        return;
    }
    b.preflight_or_panic();
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(dags::run_keyed_grain_node_excluded_from_generated_graph(&b))
        .expect("keyed-grain exclusion check failed on BigQuery");
}
