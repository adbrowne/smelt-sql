//! Phase 6 (`docs/outcomes/20260809-output-delta-typing/phases/06-plan.md`):
//! key-addressed model-edge cells. An upstream maintained model whose own
//! derived output-delta shape is `KeyedUpsert` contributes a **key-addressed**
//! edge (`docs/specs/incremental_models.md` §"Upstream model edges") —
//! `Technique::PerGroupRecompute` over an affected key set, not a
//! partition-interval scan — regardless of whether the upstream declares a
//! `timeseries:` clock, and regardless of whether the downstream itself has
//! a partition axis to clamp against.

use smelt_logical::maintenance::derive::{append_model_edge_cells, ModelEdge};
use smelt_logical::maintenance::{MaintenancePlan, PartitionLocal, Refusal, Technique};
use smelt_logical::OutputDelta;

fn keyed_edge(name: &str, keys: &[&str]) -> ModelEdge {
    ModelEdge {
        name: name.to_string(),
        clock_col: None,
        clock_col_aliases: vec![],
        unique_key: vec![],
        output_shape: Some(OutputDelta::KeyedUpsert {
            keys: keys.iter().map(|s| s.to_string()).collect(),
        }),
    }
}

#[test]
fn clockless_keyed_upstream_yields_a_key_addressed_cell() {
    let mut plan = MaintenancePlan::default();
    let edges = vec![keyed_edge("silver.agg", &["user_id"])];
    append_model_edge_cells(
        &mut plan,
        "SELECT user_id, total FROM smelt.silver.agg",
        Some("d"),
        &edges,
        &["user_id".to_string()],
    );

    assert!(
        plan.refusals.is_empty(),
        "expected no refusals, got {:?}",
        plan.refusals
    );
    assert_eq!(
        plan.cells.len(),
        1,
        "expected one cell, got {:?}",
        plan.cells
    );
    let cell = &plan.cells[0];
    assert_eq!(cell.technique, Technique::PerGroupRecompute);
    let key_scope = cell
        .key_scope
        .as_ref()
        .expect("key-addressed cell must carry a key_scope");
    assert_eq!(key_scope.keys, vec!["user_id".to_string()]);
    assert_eq!(key_scope.from, "silver.agg");
}

#[test]
fn keyed_consumer_of_keyed_upstream_yields_a_cell() {
    let mut plan = MaintenancePlan::default();
    let edges = vec![keyed_edge("silver.agg", &["user_id"])];
    // `output_partition_col: None` — a keyed-grain downstream. Before this
    // phase, `append_model_edge_cells` returned immediately here, swallowing
    // every model edge (keyed or not) without deriving a cell or a refusal.
    append_model_edge_cells(
        &mut plan,
        "SELECT user_id, total FROM smelt.silver.agg",
        None,
        &edges,
        &["user_id".to_string()],
    );

    assert!(
        plan.refusals.is_empty(),
        "expected no refusals, got {:?}",
        plan.refusals
    );
    assert_eq!(
        plan.cells.len(),
        1,
        "expected one cell, got {:?}",
        plan.cells
    );
    assert_eq!(plan.cells[0].technique, Technique::PerGroupRecompute);
}

#[test]
fn clockless_non_keyed_upstream_still_refuses() {
    let mut plan = MaintenancePlan::default();
    let edges = vec![ModelEdge {
        name: "silver.agg".to_string(),
        clock_col: None,
        clock_col_aliases: vec![],
        unique_key: vec![],
        output_shape: Some(OutputDelta::AppendOnlyWindow {
            axis: "d".to_string(),
        }),
    }];
    append_model_edge_cells(
        &mut plan,
        "SELECT user_id, total FROM smelt.silver.agg",
        Some("d"),
        &edges,
        &["user_id".to_string()],
    );

    assert!(
        plan.cells.is_empty(),
        "expected no cells, got {:?}",
        plan.cells
    );
    assert_eq!(plan.refusals.len(), 1);
    match &plan.refusals[0] {
        Refusal::ReachNotDerivable { edge, .. } => assert_eq!(edge, "silver.agg"),
        other => panic!("expected ReachNotDerivable, got {other:?}"),
    }
}

#[test]
fn key_addressed_cell_claims_no_interval_scan() {
    let mut plan = MaintenancePlan::default();
    let edges = vec![keyed_edge("silver.agg", &["user_id"])];
    append_model_edge_cells(
        &mut plan,
        "SELECT user_id, total FROM smelt.silver.agg",
        Some("d"),
        &edges,
        &["user_id".to_string()],
    );

    let cell = &plan.cells[0];
    assert!(
        cell.scans.is_empty(),
        "a key-addressed cell must claim no partition-interval scan, got {:?}",
        cell.scans
    );
    match &cell.partition_local {
        PartitionLocal::No { source, .. } => assert_eq!(source, "silver.agg"),
        other => panic!("expected PartitionLocal::No naming key addressing, got {other:?}"),
    }
}

#[test]
fn consumer_not_carrying_upstream_keys_is_refused() {
    let mut plan = MaintenancePlan::default();
    // The upstream's change-feed identity is `user_id`; the downstream's own
    // grain is `order_id`, resolved through an opaque (registry-unrecognised)
    // function — neither the upstream-keyed route (order_id isn't among the
    // upstream's own key columns) nor the grain-over-upstream route (the
    // grain expression is opaque, so `fingerprint_projection` also fails
    // closed to `FullRow`) can resolve it.
    let edges = vec![keyed_edge("silver.agg", &["user_id"])];
    append_model_edge_cells(
        &mut plan,
        "SELECT custom_udf(order_id) AS order_id, total FROM smelt.silver.agg",
        Some("d"),
        &edges,
        &["order_id".to_string()],
    );

    assert!(
        plan.cells.is_empty(),
        "expected no cells, got {:?}",
        plan.cells
    );
    assert_eq!(plan.refusals.len(), 1);
    match &plan.refusals[0] {
        Refusal::RepairKeysNotDiscoverable { source, why } => {
            assert_eq!(source, "silver.agg");
            assert!(
                why.contains("opaque"),
                "refusal should name the unresolvable grain expression, got: {why}"
            );
        }
        other => panic!("expected RepairKeysNotDiscoverable, got {other:?}"),
    }
}

#[test]
fn grain_over_upstream_columns_is_admitted() {
    let mut plan = MaintenancePlan::default();
    // The upstream's own change-feed identity is `event_id`; the downstream
    // instead regroups the upstream's rows onto `device_id, user_id` — real
    // columns of the upstream relation, just not its own key columns.
    let edges = vec![keyed_edge("silver.agg", &["event_id"])];
    append_model_edge_cells(
        &mut plan,
        "SELECT device_id, user_id, COUNT(*) AS n FROM smelt.silver.agg GROUP BY device_id, \
         user_id",
        Some("d"),
        &edges,
        &[],
    );

    assert!(
        plan.refusals.is_empty(),
        "expected no refusals, got {:?}",
        plan.refusals
    );
    assert_eq!(
        plan.cells.len(),
        1,
        "expected one cell, got {:?}",
        plan.cells
    );
    let key_scope = plan.cells[0]
        .key_scope
        .as_ref()
        .expect("key-addressed cell must carry a key_scope");
    assert_eq!(
        key_scope.keys,
        vec!["device_id".to_string(), "user_id".to_string()]
    );
    assert_eq!(
        key_scope.discovery,
        smelt_logical::maintenance::KeyDiscovery::DownstreamGrainOverUpstream
    );
}

#[test]
fn grain_from_another_relation_is_still_refused() {
    let mut plan = MaintenancePlan::default();
    // The downstream's declared grain (`order_id`) is a column of a JOINED
    // relation, not the keyed upstream `silver.agg` — the upstream side of
    // the join declares its own unique key (`user_id`) so the join itself
    // proves one-to-one, isolating the refusal to grain provenance rather
    // than fan-out ambiguity (see `fan_out_join_blocks_the_grain_route`
    // below for that separate case).
    let edges = vec![ModelEdge {
        name: "silver.agg".to_string(),
        clock_col: None,
        clock_col_aliases: vec![],
        unique_key: vec!["user_id".to_string()],
        output_shape: Some(OutputDelta::KeyedUpsert {
            keys: vec!["user_id".to_string()],
        }),
    }];
    append_model_edge_cells(
        &mut plan,
        "SELECT o.order_id, a.total FROM smelt.other.orders o JOIN smelt.silver.agg a ON \
         o.user_id = a.user_id",
        Some("d"),
        &edges,
        &["order_id".to_string()],
    );

    assert!(
        plan.cells.is_empty(),
        "expected no cells, got {:?}",
        plan.cells
    );
    assert_eq!(plan.refusals.len(), 1);
    match &plan.refusals[0] {
        Refusal::RepairKeysNotDiscoverable { source, why } => {
            assert_eq!(source, "silver.agg");
            assert!(
                why.contains("order_id") && why.contains("independent"),
                "refusal should name the grain column as independent of the upstream, got: {why}"
            );
        }
        other => panic!("expected RepairKeysNotDiscoverable, got {other:?}"),
    }
}

#[test]
fn fan_out_join_blocks_the_grain_route() {
    let mut plan = MaintenancePlan::default();
    // The downstream's grain columns (`device_id`, `user_id`) ARE columns of
    // the keyed upstream `silver.agg`, read straight off it — but the query
    // also joins in an unrelated relation with no declared unique key, so
    // the walk cannot prove the whole scope free of row multiplication. The
    // grain-over-upstream route refuses explicitly rather than admitting a
    // key set that a fan-out join could have widened.
    let edges = vec![keyed_edge("silver.agg", &["event_id"])];
    append_model_edge_cells(
        &mut plan,
        "SELECT a.device_id, a.user_id, COUNT(*) AS n FROM smelt.silver.agg a JOIN \
         smelt.other.enrich e ON a.device_id = e.device_id GROUP BY a.device_id, a.user_id",
        Some("d"),
        &edges,
        &[],
    );

    assert!(
        plan.cells.is_empty(),
        "expected no cells, got {:?}",
        plan.cells
    );
    assert_eq!(plan.refusals.len(), 1);
    match &plan.refusals[0] {
        Refusal::RepairKeysNotDiscoverable { source, why } => {
            assert_eq!(source, "silver.agg");
            assert!(
                why.contains("fan-out") || why.contains("no proven grain"),
                "refusal should name the fan-out block or the resulting ungrained SQL, got: {why}"
            );
        }
        other => panic!("expected RepairKeysNotDiscoverable, got {other:?}"),
    }
}
