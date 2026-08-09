//! The repair family's per-group recompute technique — admission and cell
//! derivation (`docs/specs/incremental_models.md` §"The repair family"):
//! [`admit_per_group_recompute`] discharges the three repair obligations
//! (grain, bounded slice, affected-key discovery) fail-closed, and
//! [`derive_repair_cell`] builds the `ColumnMerge`/`PerGroupRecompute`
//! `PlanCell` for an admitted verdict. Standalone machinery this phase —
//! not yet wired into `derive_maintenance_plan` (that lands with the
//! runtime lowering, a later phase's scope).

use std::collections::HashMap;

use smelt_logical::analysis::affected_keys::DeltaShape;
use smelt_logical::analysis::footprint::FootprintResult;
use smelt_logical::analysis::source_bounds::{BoundResult, CrossAxisLink, Seconds};
use smelt_logical::maintenance::derive::LocalityInputs;
use smelt_logical::maintenance::repair::{
    admit_per_group_recompute, derive_repair_cell, RepairRefusal,
};
use smelt_logical::maintenance::{Corner, MutationProfile, RowIdentity, SourceFacts, Technique};

fn orders_source() -> SourceFacts {
    SourceFacts {
        name: "orders".to_string(),
        mutation: MutationProfile::MutableSnapshot,
        partition_col: Some("order_date".to_string()),
        unique_key: vec!["order_id".to_string()],
        allow_full_scan: false,
    }
}

fn keyed_delta(columns: &[&str]) -> DeltaShape {
    DeltaShape {
        source: "orders".to_string(),
        columns: columns.iter().map(|c| c.to_string()).collect(),
        keyed: true,
    }
}

#[allow(clippy::type_complexity)]
fn bounded_loc(
    before: Seconds,
    after: Seconds,
) -> (
    HashMap<String, BoundResult>,
    HashMap<String, FootprintResult>,
    HashMap<String, CrossAxisLink>,
) {
    let mut bounds = HashMap::new();
    bounds.insert(
        "orders".to_string(),
        BoundResult::Bounded {
            source_partition_col: "order_date".to_string(),
            before,
            after,
        },
    );
    (bounds, HashMap::new(), HashMap::new())
}

#[test]
fn repair_admits_keyed_non_invertible_mutation() {
    let sql = "SELECT customer_id, MAX(amount) AS max_amount FROM smelt.sources.orders \
                GROUP BY customer_id";
    let (bounds, footprints, links) = bounded_loc(Seconds::hours(1), Seconds::ZERO);
    let loc = LocalityInputs {
        bounds: &bounds,
        footprints: &footprints,
        links: &links,
    };
    let delta = keyed_delta(&["customer_id", "amount"]);
    let admitted = admit_per_group_recompute(
        sql,
        &["customer_id".to_string()],
        &orders_source(),
        None,
        &loc,
        &delta,
    )
    .expect("expected admission");
    assert_eq!(admitted.key, vec!["customer_id".to_string()]);
    assert_eq!(admitted.slice.source, "orders");
    assert_eq!(admitted.slice.column, "order_date");
    assert_eq!(admitted.slice.before, Seconds::hours(1));
    assert!(admitted.over_approximated);
}

#[test]
fn repair_cell_lands_in_column_merge_corner() {
    let sql = "SELECT customer_id, MAX(amount) AS max_amount FROM smelt.sources.orders \
                GROUP BY customer_id";
    let (bounds, footprints, links) = bounded_loc(Seconds::hours(1), Seconds::ZERO);
    let loc = LocalityInputs {
        bounds: &bounds,
        footprints: &footprints,
        links: &links,
    };
    let delta = keyed_delta(&["customer_id", "amount"]);
    let admitted = admit_per_group_recompute(
        sql,
        &["customer_id".to_string()],
        &orders_source(),
        None,
        &loc,
        &delta,
    )
    .expect("expected admission");
    let cell = derive_repair_cell(&admitted, "orders", "{max_amount}".to_string());
    assert_eq!(cell.corner, Corner::ColumnMerge);
    assert_eq!(cell.technique, Technique::PerGroupRecompute);
    assert_eq!(
        cell.row_identity.identity,
        RowIdentity::Key(vec!["customer_id".to_string()])
    );
    assert_eq!(cell.scans, vec![admitted.slice.clone()]);
}

#[test]
fn repair_refuses_when_affected_keys_not_discoverable() {
    let sql = "SELECT customer_id, MAX(amount) AS max_amount FROM smelt.sources.orders \
                GROUP BY customer_id";
    let (bounds, footprints, links) = bounded_loc(Seconds::hours(1), Seconds::ZERO);
    let loc = LocalityInputs {
        bounds: &bounds,
        footprints: &footprints,
        links: &links,
    };
    let delta = DeltaShape {
        source: "orders".to_string(),
        columns: ["customer_id".to_string()].into_iter().collect(),
        keyed: false,
    };
    let err = admit_per_group_recompute(
        sql,
        &["customer_id".to_string()],
        &orders_source(),
        None,
        &loc,
        &delta,
    )
    .expect_err("expected refusal");
    match err {
        RepairRefusal::KeysNotDiscoverable { source, why } => {
            assert_eq!(source, "orders");
            assert!(why.contains("unkeyed"), "{why}");
        }
        other => panic!("expected KeysNotDiscoverable, got {other:?}"),
    }
}

#[test]
fn repair_refuses_when_grain_not_derivable() {
    let sql = "SELECT customer_id, amount FROM smelt.sources.orders";
    let (bounds, footprints, links) = bounded_loc(Seconds::hours(1), Seconds::ZERO);
    let loc = LocalityInputs {
        bounds: &bounds,
        footprints: &footprints,
        links: &links,
    };
    let delta = keyed_delta(&["customer_id", "amount"]);
    let err = admit_per_group_recompute(sql, &[], &orders_source(), None, &loc, &delta)
        .expect_err("expected refusal");
    match err {
        RepairRefusal::KeysNotDiscoverable { why, .. } => {
            assert!(why.contains("no proven grain"), "{why}");
        }
        other => panic!("expected KeysNotDiscoverable, got {other:?}"),
    }
}

#[test]
fn repair_refuses_when_slice_unbounded() {
    let sql = "SELECT customer_id, MAX(amount) AS max_amount FROM smelt.sources.orders \
                GROUP BY customer_id";
    let mut bounds = HashMap::new();
    bounds.insert("orders".to_string(), BoundResult::NotDerivable);
    let footprints = HashMap::new();
    let links = HashMap::new();
    let loc = LocalityInputs {
        bounds: &bounds,
        footprints: &footprints,
        links: &links,
    };
    let delta = keyed_delta(&["customer_id", "amount"]);
    let err = admit_per_group_recompute(
        sql,
        &["customer_id".to_string()],
        &orders_source(),
        None,
        &loc,
        &delta,
    )
    .expect_err("expected refusal");
    assert!(matches!(err, RepairRefusal::SliceUnbounded { .. }));
}

#[test]
fn repair_refuses_when_every_grain_column_independent_of_delta_source() {
    let sql = "SELECT 'ALL' AS bucket, SUM(amount) AS total FROM smelt.sources.orders \
                GROUP BY 'ALL'";
    let (bounds, footprints, links) = bounded_loc(Seconds::hours(1), Seconds::ZERO);
    let loc = LocalityInputs {
        bounds: &bounds,
        footprints: &footprints,
        links: &links,
    };
    let delta = keyed_delta(&["amount"]);
    let err = admit_per_group_recompute(
        sql,
        &["bucket".to_string()],
        &orders_source(),
        None,
        &loc,
        &delta,
    )
    .expect_err("expected refusal");
    match err {
        RepairRefusal::KeysNotDiscoverable { why, .. } => {
            assert!(why.contains("independent of source"), "{why}");
        }
        other => panic!("expected KeysNotDiscoverable, got {other:?}"),
    }
}

#[test]
fn repair_over_approximated_keys_are_admitted() {
    let sql = "SELECT customer_id, MAX(amount) AS max_amount FROM smelt.sources.orders \
                GROUP BY customer_id";
    let (bounds, footprints, links) = bounded_loc(Seconds::hours(1), Seconds::ZERO);
    let loc = LocalityInputs {
        bounds: &bounds,
        footprints: &footprints,
        links: &links,
    };
    let delta = keyed_delta(&["customer_id", "amount", "region", "created_at"]);
    let admitted = admit_per_group_recompute(
        sql,
        &["customer_id".to_string()],
        &orders_source(),
        None,
        &loc,
        &delta,
    )
    .expect("a key superset in the delta's own row shape must still admit");
    assert!(admitted.over_approximated);
}
