//! T3 — delta-restricted compute over model edges (`docs/plans/
//! 20260715-composed-axes-conditional-maintenance.md` Phase E3).
//!
//! Exercises the full pipeline a downstream model's creation-trigger cell
//! walks for a model-edge enrichment join: `append_model_edge_cells` derives
//! the shared P1 skeleton-source-closure verdict onto every edge's cell
//! (`derive.rs`), `choice::resolve_recompute_restriction` decides whether an
//! exact observed delta licenses restriction, and `emit::
//! emit_delete_insert_delta_restricted` produces the semi-joined statement —
//! versus `emit::emit_delete_insert`'s byte-identical unrestricted form when
//! either factor is absent (the review checklist's "no partial restriction"
//! rule).

use smelt_logical::maintenance::choice::{resolve_recompute_restriction, RecomputeRestriction};
use smelt_logical::maintenance::derive::{append_model_edge_cells, ModelEdge};
use smelt_logical::maintenance::emit::{
    emit_delete_insert, emit_delete_insert_delta_restricted, MaintenanceDialect, Region,
};
use smelt_logical::maintenance::{MaintenancePlan, Trigger};

/// A fact + dimension enrichment, LEFT JOIN, payload-only projection, the
/// dimension's `unique_key: [id]` declared — the closure-admissible shape
/// (mirrors `skeleton_closure.rs`'s own `left_join_payload_only_closes`
/// fixture, but through the model-edge derivation path instead of a direct
/// `skeleton_source_closure` call).
const CLOSED_SQL: &str = "SELECT fact.event_id, fact.event_date, dim.tier \
     FROM smelt.silver.fact fact \
     LEFT JOIN smelt.silver.dim dim ON fact.dim_id = dim.id";

/// Same join shape, but `INNER JOIN` with no declared `unique_key` on the
/// dimension edge — conjuncts 3 (one-to-one) and 4 (row preservation) both
/// fail to prove, so closure stays `Open`.
const OPEN_SQL: &str = "SELECT fact.event_id, fact.event_date, dim.tier \
     FROM smelt.silver.fact fact \
     JOIN smelt.silver.dim dim ON fact.dim_id = dim.id";

fn fact_edge() -> ModelEdge {
    ModelEdge {
        name: "silver.fact".to_string(),
        clock_col: Some("event_date".to_string()),
        unique_key: vec![],
    }
}

fn dim_edge_with_key() -> ModelEdge {
    ModelEdge {
        name: "silver.dim".to_string(),
        clock_col: Some("event_date".to_string()),
        unique_key: vec!["id".to_string()],
    }
}

fn dim_edge_without_key() -> ModelEdge {
    ModelEdge {
        name: "silver.dim".to_string(),
        clock_col: Some("event_date".to_string()),
        unique_key: vec![],
    }
}

#[test]
fn left_join_with_declared_unique_key_closes_every_edge_cell() {
    let mut plan = MaintenancePlan::default();
    append_model_edge_cells(
        &mut plan,
        CLOSED_SQL,
        Some("event_date"),
        &[fact_edge(), dim_edge_with_key()],
        &[],
    );
    assert_eq!(plan.cells.len(), 2, "{plan:?}");
    for cell in &plan.cells {
        assert_eq!(
            cell.skeleton_source_closure.as_ref().map(|c| c.is_closed()),
            Some(true),
            "cell for {:?} must carry a Closed skeleton-source-closure verdict; got {:?}",
            cell.trigger,
            cell.skeleton_source_closure
        );
    }
}

#[test]
fn inner_join_without_unique_key_stays_open_on_every_edge_cell() {
    let mut plan = MaintenancePlan::default();
    append_model_edge_cells(
        &mut plan,
        OPEN_SQL,
        Some("event_date"),
        &[fact_edge(), dim_edge_without_key()],
        &[],
    );
    assert_eq!(plan.cells.len(), 2, "{plan:?}");
    for cell in &plan.cells {
        assert_eq!(
            cell.skeleton_source_closure.as_ref().map(|c| c.is_closed()),
            Some(false),
            "cell for {:?} must NOT be Closed; got {:?}",
            cell.trigger,
            cell.skeleton_source_closure
        );
    }
}

#[test]
fn single_model_edge_with_no_enrichment_join_carries_no_closure_verdict() {
    // A model whose only model edge is the driving (FROM) source has no
    // enrichment join to close over at all — `None`, not `Open`
    // (`PlanCell::skeleton_source_closure`'s documented "common case").
    let sql = "SELECT fact.event_id, fact.event_date FROM smelt.silver.fact fact";
    let mut plan = MaintenancePlan::default();
    append_model_edge_cells(&mut plan, sql, Some("event_date"), &[fact_edge()], &[]);
    assert_eq!(plan.cells.len(), 1);
    assert_eq!(plan.cells[0].skeleton_source_closure, None);
}

#[test]
fn closed_cell_with_an_exact_delta_restricts_the_emitted_statement() {
    let mut plan = MaintenancePlan::default();
    append_model_edge_cells(
        &mut plan,
        CLOSED_SQL,
        Some("event_date"),
        &[fact_edge(), dim_edge_with_key()],
        &[],
    );
    let cell = plan
        .cell_for(&Trigger::NewData {
            source: "silver.fact".to_string(),
        })
        .expect("fact edge cell present");

    let delta_keys = vec!["ev-1".to_string(), "ev-2".to_string()];
    let restriction =
        resolve_recompute_restriction(cell.skeleton_source_closure.as_ref(), Some(&delta_keys));
    assert_eq!(
        restriction,
        RecomputeRestriction::Restricted {
            delta_keys: delta_keys.clone()
        }
    );

    let region = Region {
        start: "'2026-07-01'".to_string(),
        end: "'2026-07-02'".to_string(),
    };
    let body = "SELECT fact.event_id, fact.event_date, dim.tier FROM enrichment_recompute";
    let RecomputeRestriction::Restricted { delta_keys } = restriction else {
        unreachable!();
    };
    let restricted_group = emit_delete_insert_delta_restricted(
        "main.enriched",
        "event_date",
        &region,
        body,
        "event_id",
        &delta_keys,
        MaintenanceDialect::DuckDb,
    );
    let unrestricted_group = emit_delete_insert(
        "main.enriched",
        "event_date",
        &region,
        body,
        MaintenanceDialect::DuckDb,
    );
    assert_ne!(
        restricted_group, unrestricted_group,
        "a licensed restriction must actually change the emitted statement text"
    );
    assert!(restricted_group.statements[0].sql.contains("event_id IN"));
    assert!(restricted_group.statements[1].sql.contains("event_id IN"));
}

#[test]
fn open_cell_never_restricts_even_with_an_exact_delta_present() {
    let mut plan = MaintenancePlan::default();
    append_model_edge_cells(
        &mut plan,
        OPEN_SQL,
        Some("event_date"),
        &[fact_edge(), dim_edge_without_key()],
        &[],
    );
    let cell = plan
        .cell_for(&Trigger::NewData {
            source: "silver.fact".to_string(),
        })
        .expect("fact edge cell present");

    let delta_keys = vec!["ev-1".to_string()];
    let restriction =
        resolve_recompute_restriction(cell.skeleton_source_closure.as_ref(), Some(&delta_keys));
    assert!(matches!(
        restriction,
        RecomputeRestriction::Unrestricted { .. }
    ));

    // The fallback path calls `emit_delete_insert` directly — byte-identical
    // to the same call made with no restriction machinery involved at all.
    let region = Region {
        start: "'2026-07-01'".to_string(),
        end: "'2026-07-02'".to_string(),
    };
    let body = "SELECT fact.event_id, fact.event_date, dim.tier FROM enrichment_recompute";
    let a = emit_delete_insert(
        "main.enriched",
        "event_date",
        &region,
        body,
        MaintenanceDialect::DuckDb,
    );
    let b = emit_delete_insert(
        "main.enriched",
        "event_date",
        &region,
        body,
        MaintenanceDialect::DuckDb,
    );
    assert_eq!(
        a, b,
        "unrestricted fallback must be byte-identical every time"
    );
}

#[test]
fn absent_observed_delta_falls_back_to_the_widened_scan() {
    // Fallback: a pre-D2 upstream (or a window never recorded) records no
    // delta at all — `None`, distinct from a present-but-empty one — must
    // never restrict, even under a Closed verdict (widen-never-narrow).
    let mut plan = MaintenancePlan::default();
    append_model_edge_cells(
        &mut plan,
        CLOSED_SQL,
        Some("event_date"),
        &[fact_edge(), dim_edge_with_key()],
        &[],
    );
    let cell = plan
        .cell_for(&Trigger::NewData {
            source: "silver.fact".to_string(),
        })
        .expect("fact edge cell present");
    assert_eq!(
        cell.skeleton_source_closure.as_ref().map(|c| c.is_closed()),
        Some(true)
    );

    let restriction = resolve_recompute_restriction(cell.skeleton_source_closure.as_ref(), None);
    assert!(matches!(
        restriction,
        RecomputeRestriction::Unrestricted { .. }
    ));
}
