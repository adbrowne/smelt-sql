//! The enrichment-keyed model-edge route (`docs/specs/incremental_models.md`
//! §"Upstream model edges"): a clockless keyed upstream model edge read in
//! **value-enrichment** position (a join whose `ON` predicate matches the
//! edge's own declared `unique_key`, contributing payload columns) by a
//! **partition-addressed** downstream contributes an `UpstreamMutation` cell
//! with `Technique::ColumnScopedMerge`, addressed by the join key the
//! downstream itself projects — parity with the `ColumnScopedMerge`
//! technique already derived for a declared `mutation_profile:
//! mutable_snapshot` source dimension. Attempted only after the key-addressed
//! route (`repair::admit_key_addressed_recompute`) declines with
//! `KeysNotDiscoverable` — the real shape a `grain: partition` downstream
//! with no derivable key grain hits (`docs/outcomes/
//! 20260906-bigquery-correctness/phases/04-plan.md`).

use smelt_logical::analysis::output_delta::OutputDelta;
use smelt_logical::maintenance::derive::{append_model_edge_cells, ModelEdge};
use smelt_logical::maintenance::{
    Corner, KeyDiscovery, MaintenancePlan, PartitionLocal, Refusal, Technique, Trigger,
};

/// A partition-addressed fact enriched by a clockless keyed dimension —
/// `f`'s own columns (`event_id`, `repo_id`) are passthrough, `dim`'s
/// `current_repo_name` is the only value-enrichment-sensitive column. The
/// join's `ON` predicate matches `dim`'s own declared `unique_key`
/// (`repo_id`) and is a `LEFT JOIN`, so the enrichment join proves
/// one-to-one and row-preserving.
const ENRICHMENT_SQL: &str = "SELECT f.event_id, f.event_date, f.repo_id, \
     dim.current_repo_name AS current_repo_name \
     FROM smelt.silver.events f \
     LEFT JOIN smelt.gold.repo_dim dim ON f.repo_id = dim.repo_id";

/// Same shape, but the downstream never projects `repo_id` at all — the
/// enrichment join's own local key column is not available to address a
/// merge write by.
const ENRICHMENT_SQL_KEY_NOT_PROJECTED: &str = "SELECT f.event_id, f.event_date, \
     dim.current_repo_name AS current_repo_name \
     FROM smelt.silver.events f \
     LEFT JOIN smelt.gold.repo_dim dim ON f.repo_id = dim.repo_id";

/// Same shape, but an `INNER JOIN` with no declared `referential_integrity` —
/// row preservation is unproven, so the enrichment join is a row-admission
/// (membership) read, not a pure value-enrichment one.
const MEMBERSHIP_SQL: &str = "SELECT f.event_id, f.event_date, f.repo_id, \
     dim.current_repo_name AS current_repo_name \
     FROM smelt.silver.events f \
     INNER JOIN smelt.gold.repo_dim dim ON f.repo_id = dim.repo_id";

fn repo_dim_edge(allow_full_scan: bool) -> ModelEdge {
    ModelEdge {
        name: "gold.repo_dim".to_string(),
        clock_col: None,
        clock_col_aliases: vec![],
        unique_key: vec!["repo_id".to_string()],
        output_shape: Some(OutputDelta::KeyedUpsert {
            keys: vec!["repo_id".to_string()],
        }),
        allow_full_scan,
    }
}

fn clocked_edge() -> ModelEdge {
    ModelEdge {
        name: "gold.repo_dim".to_string(),
        clock_col: Some("updated_at".to_string()),
        clock_col_aliases: vec![],
        unique_key: vec!["repo_id".to_string()],
        output_shape: Some(OutputDelta::KeyedUpsert {
            keys: vec!["repo_id".to_string()],
        }),
        allow_full_scan: false,
    }
}

#[test]
fn clockless_keyed_dimension_yields_column_scoped_mutation_cell() {
    let mut plan = MaintenancePlan::default();
    append_model_edge_cells(
        &mut plan,
        ENRICHMENT_SQL,
        Some("event_date"),
        &[repo_dim_edge(true)],
        &[],
        &[],
        &Default::default(),
    );
    assert!(
        plan.refusals.is_empty(),
        "expected no refusals, got {:?}",
        plan.refusals
    );
    let cells: Vec<_> = plan
        .cells
        .iter()
        .filter(|c| matches!(&c.trigger, Trigger::UpstreamMutation { source } if source == "gold.repo_dim"))
        .collect();
    assert_eq!(cells.len(), 1, "expected exactly one cell, got {plan:?}");
    let cell = cells[0];
    assert_eq!(cell.technique, Technique::ColumnScopedMerge);
    assert_eq!(cell.corner, Corner::ColumnMerge);
}

#[test]
fn mutation_cell_group_is_only_the_edge_provenanced_columns() {
    let mut plan = MaintenancePlan::default();
    append_model_edge_cells(
        &mut plan,
        ENRICHMENT_SQL,
        Some("event_date"),
        &[repo_dim_edge(true)],
        &[],
        &[],
        &Default::default(),
    );
    let cell = plan
        .cells
        .iter()
        .find(|c| matches!(&c.trigger, Trigger::UpstreamMutation { .. }))
        .unwrap_or_else(|| panic!("expected an UpstreamMutation cell, got {plan:?}"));
    assert_eq!(cell.group, "{current_repo_name}");
}

#[test]
fn mutation_cell_key_scope_is_the_join_key_carried_in_the_output() {
    let mut plan = MaintenancePlan::default();
    append_model_edge_cells(
        &mut plan,
        ENRICHMENT_SQL,
        Some("event_date"),
        &[repo_dim_edge(true)],
        &[],
        &[],
        &Default::default(),
    );
    let cell = plan
        .cells
        .iter()
        .find(|c| matches!(&c.trigger, Trigger::UpstreamMutation { .. }))
        .unwrap_or_else(|| panic!("expected an UpstreamMutation cell, got {plan:?}"));
    let key_scope = cell
        .key_scope
        .as_ref()
        .unwrap_or_else(|| panic!("expected a key_scope on {cell:?}"));
    assert_eq!(key_scope.keys, vec!["repo_id".to_string()]);
    assert_eq!(key_scope.from, "gold.repo_dim");
    assert_eq!(key_scope.discovery, KeyDiscovery::EnrichmentKeyed);
    assert!(matches!(cell.partition_local, PartitionLocal::No { .. }));
}

#[test]
fn enrichment_route_refuses_when_the_join_key_is_not_projected() {
    let mut plan = MaintenancePlan::default();
    append_model_edge_cells(
        &mut plan,
        ENRICHMENT_SQL_KEY_NOT_PROJECTED,
        Some("event_date"),
        &[repo_dim_edge(true)],
        &[],
        &[],
        &Default::default(),
    );
    assert!(
        !plan
            .cells
            .iter()
            .any(|c| matches!(&c.trigger, Trigger::UpstreamMutation { .. })),
        "expected no UpstreamMutation cell, got {plan:?}"
    );
    assert!(
        plan.refusals.iter().any(|r| matches!(
            r,
            Refusal::RepairKeysNotDiscoverable { source, .. } if source == "gold.repo_dim"
        )),
        "expected a RepairKeysNotDiscoverable refusal naming the edge, got {:?}",
        plan.refusals
    );
}

#[test]
fn enrichment_route_refuses_an_unbounded_scan_without_allow_full_scan() {
    let mut plan = MaintenancePlan::default();
    append_model_edge_cells(
        &mut plan,
        ENRICHMENT_SQL,
        Some("event_date"),
        &[repo_dim_edge(false)],
        &[],
        &[],
        &Default::default(),
    );
    assert!(
        !plan
            .cells
            .iter()
            .any(|c| matches!(&c.trigger, Trigger::UpstreamMutation { .. })),
        "expected no UpstreamMutation cell, got {plan:?}"
    );
    assert!(
        plan.refusals.iter().any(|r| matches!(
            r,
            Refusal::ScanUnbounded { source, .. } if source == "gold.repo_dim"
        )),
        "expected a ScanUnbounded refusal naming the edge, got {:?}",
        plan.refusals
    );
}

#[test]
fn clocked_edge_keeps_todays_delete_insert_route() {
    let mut plan = MaintenancePlan::default();
    append_model_edge_cells(
        &mut plan,
        ENRICHMENT_SQL,
        Some("event_date"),
        &[clocked_edge()],
        &[],
        &[],
        &Default::default(),
    );
    assert!(
        !plan.refusals.iter().any(|r| matches!(
            r,
            Refusal::RepairKeysNotDiscoverable { .. } | Refusal::ScanUnbounded { .. }
        )),
        "expected no repair-family refusal for a clocked edge, got {:?}",
        plan.refusals
    );
    let cell = plan
        .cells
        .iter()
        .find(|c| matches!(&c.trigger, Trigger::NewData { source } if source == "gold.repo_dim"))
        .unwrap_or_else(|| panic!("expected a NewData creation cell, got {plan:?}"));
    assert_eq!(cell.technique, Technique::DeleteInsert);
}

#[test]
fn membership_position_edge_gets_no_column_scoped_cell() {
    let mut plan = MaintenancePlan::default();
    append_model_edge_cells(
        &mut plan,
        MEMBERSHIP_SQL,
        Some("event_date"),
        &[repo_dim_edge(true)],
        &[],
        &[],
        &Default::default(),
    );
    assert!(
        !plan
            .cells
            .iter()
            .any(|c| matches!(&c.trigger, Trigger::UpstreamMutation { .. })),
        "a membership-position (row-admission) edge must never get a column-scoped merge \
         cell, got {plan:?}"
    );
    assert!(
        plan.refusals.iter().any(|r| matches!(
            r,
            Refusal::RepairKeysNotDiscoverable { source, .. } if source == "gold.repo_dim"
        )),
        "expected the original RepairKeysNotDiscoverable refusal to survive, got {:?}",
        plan.refusals
    );
}
