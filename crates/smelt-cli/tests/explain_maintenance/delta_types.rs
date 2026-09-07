use std::path::Path;

use crate::support::{build_report_for, stage_delta_type_project, stage_keyed_chain_project};

/// `dag_kchain_a` (clockless, `KeyedUpsert`-shaped via its own `GROUP BY id`
/// over an append-only source) is `dag_kchain_b`'s only inbound edge — its
/// block must print `delta type: keyed upsert`.
#[test]
fn explain_renders_keyed_upsert_edge_delta_type() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project_dir = stage_keyed_chain_project(&tmp);

    let report = build_report_for(&project_dir, "dag_kchain_b")
        .expect("dag_kchain_b has a maintenance plan");

    assert!(
        report.contains("dag_kchain_a (model)"),
        "expected dag_kchain_a as an inbound model edge: {report}"
    );
    assert!(
        report.contains("delta type: keyed upsert"),
        "expected the clockless keyed upstream's edge to be typed keyed upsert: {report}"
    );
}

/// `user_daily_spend` in `examples/timeseries` reads the clocked,
/// `append_only` `sources.raw.transactions` — the common case's edge must
/// still print `delta type: append-only within window` (no regression from
/// the new row).
#[test]
fn explain_renders_append_only_window_edge_delta_type() {
    let project_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/timeseries")
        .canonicalize()
        .expect("examples/timeseries exists");

    let report = build_report_for(&project_dir, "user_daily_spend")
        .expect("user_daily_spend has a maintenance plan");

    assert!(
        report.contains("sources.raw.transactions (source)"),
        "expected sources.raw.transactions as an inbound source edge: {report}"
    );
    assert!(
        report.contains("delta type: append-only within window"),
        "expected the clocked append-only source edge to be typed append-only within window: \
         {report}"
    );
}

/// `windowed_upstream`'s `rn` column is a window-function output — the walk
/// cannot classify it as addressable, so `general_consumer`'s inbound edge to
/// it must print `delta type: general` naming the window-function construct.
#[test]
fn explain_names_construct_that_degraded_edge_delta_type() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project_dir = stage_delta_type_project(&tmp);

    let report = build_report_for(&project_dir, "general_consumer")
        .expect("general_consumer has a maintenance plan");

    assert!(
        report.contains("windowed_upstream (model)"),
        "expected windowed_upstream as an inbound model edge: {report}"
    );
    assert!(
        report.contains("delta type: general (degraded by:") && report.contains("window-function"),
        "expected a general verdict naming the window-function construct: {report}"
    );
}

/// `sources.undeclared` declares no `mutation_profile` — the fail-closed
/// seed must be visible (`general`), not silently skipped, and the reason
/// must name the missing declaration.
#[test]
fn explain_renders_source_edge_delta_type() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project_dir = stage_delta_type_project(&tmp);

    let report = build_report_for(&project_dir, "source_consumer")
        .expect("source_consumer has a maintenance plan");

    assert!(
        report.contains("sources.undeclared (source)"),
        "expected sources.undeclared as an inbound source edge: {report}"
    );
    assert!(
        report.contains("delta type: general (degraded by:")
            && report.contains("declares no mutation_profile"),
        "expected a general verdict naming the missing mutation_profile declaration: {report}"
    );
}

/// `view_upstream` is a plain view (no `refresh: incremental`) — it
/// contributes no [`smelt_db::model_edges_for`] entry, so `view_consumer`'s
/// edge to it must print no `delta type:` row at all rather than a
/// fabricated one.
#[test]
fn explain_edge_without_derived_shape_prints_no_delta_row() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project_dir = stage_delta_type_project(&tmp);

    let report = build_report_for(&project_dir, "view_consumer")
        .expect("view_consumer has a maintenance plan");

    assert!(
        report.contains("view_upstream (model)"),
        "expected view_upstream as an inbound model edge: {report}"
    );
    let edge_block_start = report
        .find("view_upstream (model)")
        .expect("view_upstream edge block present");
    let edge_block = &report[edge_block_start..];
    let edge_block_end = edge_block
        .find("\n\n")
        .map(|i| edge_block_start + i)
        .unwrap_or(report.len());
    assert!(
        !report[edge_block_start..edge_block_end].contains("delta type:"),
        "a non-incremental upstream's edge must print no delta type row: {report}"
    );
}

/// `dag_kchain_b`'s `PerGroupRecompute` cell over the clockless keyed
/// upstream `dag_kchain_a` is key-addressed (`cell.key_scope`) — its repair
/// stanza's affected-key discovery line must name the group-grain
/// fingerprint-sidecar diff over the upstream's own output table, not the
/// declared-source discovery mechanism (`dag_kchain_a` is a model, not a
/// declared source).
#[test]
fn explain_key_addressed_cell_prints_upstream_sidecar_discovery() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project_dir = stage_keyed_chain_project(&tmp);

    let report = build_report_for(&project_dir, "dag_kchain_b")
        .expect("dag_kchain_b has a maintenance plan");

    assert!(
        report.contains("technique: PerGroupRecompute"),
        "expected a key-addressed PerGroupRecompute cell: {report}"
    );
    assert!(
        report.contains(
            "affected-key discovery: group-grain fingerprint-sidecar diff over the upstream's \
             own output table"
        ),
        "expected the upstream-sidecar discovery mechanism, not a declared-source posture: \
         {report}"
    );
}

/// Phase 24b (`docs/outcomes/20260815-definition-delta-migrate/phases/
/// 24b-plan.md`): `silver.device_user_edges` regroups
/// `silver.events_deduped`'s rows onto `device_id, user_id` — real columns
/// of the upstream relation, not `events_deduped`'s own `KeyedUpsert` key
/// (`event_id`). The grain-over-upstream discovery route admits this: the
/// cell must resolve with no `RepairKeysNotDiscoverable` refusal, and the
/// report must name the grain-over-upstream route.
#[test]
fn device_user_edges_admits_a_key_addressed_cell() {
    let project_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/web_analytics")
        .canonicalize()
        .expect("examples/web_analytics exists");

    let report = build_report_for(&project_dir, "silver.device_user_edges")
        .expect("silver.device_user_edges has a maintenance plan");

    assert!(
        !report.contains("RepairKeysNotDiscoverable"),
        "device_user_edges must no longer refuse key-addressed admission: {report}"
    );
    assert!(
        report.contains("technique: PerGroupRecompute"),
        "expected a key-addressed PerGroupRecompute cell: {report}"
    );
    assert!(
        report.contains(
            "affected-key discovery: group-grain fingerprint-sidecar diff over the upstream's \
             own output table (keyed at the downstream's own grain, projected over the upstream \
             relation)"
        ),
        "expected the grain-over-upstream discovery route to be named: {report}"
    );
}
