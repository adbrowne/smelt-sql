//! Spark twin of `maintenance_conformance/dags.rs`. Thin wrapper over
//! `smelt_maintenance_testkit::families::dags`
//! (`docs/plans/20260817-bigquery-generative-conformance.md` Phase 4).

use smelt_maintenance_testkit::families::{dags, ConformanceBackend};

use crate::backend::SparkConformanceBackend;

/// Default deterministic case count — smaller than the DuckDB leg's default
/// (6): each Spark case stages TWO independent projects and drives
/// `execute_project` multiple times each over a live Spark Connect server.
const DEFAULT_CASES: usize = 3;

fn case_count() -> usize {
    std::env::var("SMELT_CONFORMANCE_SPARK_DAG_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_CASES)
}

/// `dags_oracle_flags_a_seeded_divergence_on_spark` — this family's
/// non-vacuity self-check. Proves the per-node equality assertion is capable
/// of FAILING on Spark, which it was not while a case's incremental project
/// and its full-refresh twin shared one schema.
#[test]
fn dags_oracle_flags_a_seeded_divergence_on_spark() {
    let b = SparkConformanceBackend;
    if let Some(reason) = b.skip_reason() {
        eprintln!("{reason} — skipping dags_oracle_flags_a_seeded_divergence_on_spark");
        return;
    }
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(dags::run_oracle_flags_a_seeded_divergence(&b))
        .expect("the dags oracle failed to flag a seeded divergence on Spark");
}

/// `chain_since_upstream_dirty_set_suffices_on_spark`.
#[test]
fn chain_since_upstream_dirty_set_suffices_on_spark() {
    let b = SparkConformanceBackend;
    if let Some(reason) = b.skip_reason() {
        eprintln!("{reason} — skipping chain_since_upstream_dirty_set_suffices_on_spark");
        return;
    }
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(dags::run_chain_since_upstream_dirty_set_suffices(
        &b,
        case_count(),
    ))
    .expect("chain propagation check failed on Spark");
}

/// `diamond_propagation_suffices_on_spark`.
#[test]
fn diamond_propagation_suffices_on_spark() {
    let b = SparkConformanceBackend;
    if let Some(reason) = b.skip_reason() {
        eprintln!("{reason} — skipping diamond_propagation_suffices_on_spark");
        return;
    }
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(dags::run_diamond_propagation_suffices(&b, case_count()))
        .expect("diamond propagation check failed on Spark");
}

/// `upstream_payload_in_downstream_skeleton_position_on_spark`.
#[test]
fn upstream_payload_in_downstream_skeleton_position_on_spark() {
    let b = SparkConformanceBackend;
    if let Some(reason) = b.skip_reason() {
        eprintln!("{reason} — skipping upstream_payload_in_downstream_skeleton_position_on_spark");
        return;
    }
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(dags::run_upstream_payload_in_downstream_skeleton_position(
        &b,
        case_count(),
    ))
    .expect("leak-family propagation check failed on Spark");
}

/// `include_upstreams_resolved_slices_suffice_on_spark`.
#[test]
fn include_upstreams_resolved_slices_suffice_on_spark() {
    let b = SparkConformanceBackend;
    if let Some(reason) = b.skip_reason() {
        eprintln!("{reason} — skipping include_upstreams_resolved_slices_suffice_on_spark");
        return;
    }
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(dags::run_include_upstreams_resolved_slices_suffice(
        &b,
        case_count(),
    ))
    .expect("backward-resolved slice check failed on Spark");
}

/// `keyed_grain_node_excluded_from_generated_graph_on_spark`.
#[test]
fn keyed_grain_node_excluded_from_generated_graph_on_spark() {
    let b = SparkConformanceBackend;
    if let Some(reason) = b.skip_reason() {
        eprintln!("{reason} — skipping keyed_grain_node_excluded_from_generated_graph_on_spark");
        return;
    }
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(dags::run_keyed_grain_node_excluded_from_generated_graph(&b))
        .expect("keyed-grain exclusion check failed on Spark");
}
