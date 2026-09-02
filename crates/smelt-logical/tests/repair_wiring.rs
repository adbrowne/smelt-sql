//! Refusal narrowing at the derive layer (`docs/specs/incremental_models.md`
//! §"The repair family"): a keyed fold over a mutable/retraction source
//! derives a `Technique::PerGroupRecompute` cell where the faithful-fold
//! source-posture obligation would otherwise refuse outright — narrowed
//! exactly at `derive_new_data`'s key-grain posture-failure branch,
//! additive with the pre-existing `NoAdmissibleTechnique` refusal when
//! repair admission itself fails.

use std::collections::BTreeSet;

use smelt_logical::analysis::source_bounds::Seconds;
use smelt_logical::maintenance::derive::{derive_maintenance_plan, FoldSpec, ModelInputs};
use smelt_logical::maintenance::{
    Corner, Grain, MutationProfile, OutputSpec, Refusal, RowIdentity, SourceFacts, Technique,
    Trigger,
};
use smelt_types::SqlFunction;

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| s.to_string()).collect()
}

const CLOCKED_KEYED_SQL: &str = "SELECT customer_id, MAX(amount) AS max_amount \
     FROM smelt.sources.orders \
     WHERE order_date BETWEEN CURRENT_DATE - INTERVAL '1 day' AND CURRENT_DATE \
     GROUP BY customer_id";

const UNCLOCKED_KEYED_SQL: &str =
    "SELECT customer_id, MAX(amount) AS max_amount FROM smelt.sources.orders \
     GROUP BY customer_id";

fn orders_source(partition_col: Option<&str>, unique_key: &[&str]) -> SourceFacts {
    SourceFacts {
        name: "orders".to_string(),
        mutation: MutationProfile::MutableSnapshot,
        partition_col: partition_col.map(|c| c.to_string()),
        unique_key: strings(unique_key),
        allow_full_scan: false,
    }
}

fn inputs(sql: &'static str, source: SourceFacts) -> ModelInputs<'static> {
    ModelInputs {
        sql,
        output: OutputSpec {
            table: "customer_max_amount".to_string(),
            grain: Grain::Key {
                unique_key: strings(&["customer_id"]),
            },
            skeleton_columns: set(&["customer_id"]),
        },
        sources: vec![source],
        column_groups: vec![],
        fold: Some(FoldSpec {
            add_columns: vec![("max_amount".to_string(), SqlFunction::Max)],
        }),
        old_columns: Vec::new(),
        old_sql: None,
    }
}

fn new_data_trigger() -> Trigger {
    Trigger::NewData {
        source: "orders".to_string(),
    }
}

#[test]
fn keyed_fold_over_mutable_source_derives_a_per_group_recompute_cell() {
    let inputs = inputs(
        CLOCKED_KEYED_SQL,
        orders_source(Some("order_date"), &["order_id"]),
    );
    let trigger = new_data_trigger();
    let plan = derive_maintenance_plan(&inputs, std::slice::from_ref(&trigger));

    assert!(
        plan.refusals.is_empty(),
        "admitted repair should carry no refusal, got {:?}",
        plan.refusals
    );
    let cell = plan
        .cell_for(&trigger)
        .unwrap_or_else(|| panic!("expected a repair cell for NewData: {plan:#?}"));
    assert_eq!(cell.corner, Corner::ColumnMerge);
    assert_eq!(cell.technique, Technique::PerGroupRecompute);
}

#[test]
fn repair_cell_carries_the_affected_key_and_the_bounded_slice() {
    let inputs = inputs(
        CLOCKED_KEYED_SQL,
        orders_source(Some("order_date"), &["order_id"]),
    );
    let trigger = new_data_trigger();
    let plan = derive_maintenance_plan(&inputs, std::slice::from_ref(&trigger));
    let cell = plan
        .cell_for(&trigger)
        .unwrap_or_else(|| panic!("expected a repair cell for NewData: {plan:#?}"));
    assert_eq!(
        cell.row_identity.identity,
        RowIdentity::Key(vec!["customer_id".to_string()])
    );
    assert!(!cell.scans.is_empty(), "expected a non-empty scan clamp");
    let clamp = &cell.scans[0];
    assert_eq!(clamp.source, "orders");
    assert_eq!(clamp.column, "order_date");
    assert_eq!(clamp.before, Seconds::days(1));
}

#[test]
fn undiscoverable_affected_keys_refuses_repair_keys_not_discoverable() {
    // No declared `unique_key` on the source — the delta carries no per-row
    // identity, so affected-key discovery fails closed.
    let inputs = inputs(CLOCKED_KEYED_SQL, orders_source(Some("order_date"), &[]));
    let trigger = new_data_trigger();
    let plan = derive_maintenance_plan(&inputs, std::slice::from_ref(&trigger));

    assert!(
        plan.cell_for(&trigger).is_none(),
        "no cell should be admitted, got {:?}",
        plan.cell_for(&trigger)
    );
    assert!(
        plan.refusals
            .iter()
            .any(|r| matches!(r, Refusal::NoAdmissibleTechnique { .. })),
        "the pre-existing posture refusal must still be pushed, got {:?}",
        plan.refusals
    );
    assert!(
        plan.refusals.iter().any(|r| matches!(
            r,
            Refusal::RepairKeysNotDiscoverable { source, .. } if source == "orders"
        )),
        "expected an additive RepairKeysNotDiscoverable refusal naming the source, got {:?}",
        plan.refusals
    );
}

#[test]
fn unclocked_mutable_source_refuses_repair_slice_unbounded() {
    let inputs = inputs(UNCLOCKED_KEYED_SQL, orders_source(None, &["order_id"]));
    let trigger = new_data_trigger();
    let plan = derive_maintenance_plan(&inputs, std::slice::from_ref(&trigger));

    assert!(plan.cell_for(&trigger).is_none());
    assert!(
        plan.refusals.iter().any(
            |r| matches!(r, Refusal::RepairSliceUnbounded { source, .. } if source == "orders")
        ),
        "expected RepairSliceUnbounded — never a widened whole-table repair, got {:?}",
        plan.refusals
    );
}

#[test]
fn append_only_fold_still_derives_the_unchanged_fold_cell() {
    let mut source = orders_source(Some("order_date"), &["order_id"]);
    source.mutation = MutationProfile::AppendOnly;
    let inputs = inputs(CLOCKED_KEYED_SQL, source);
    let trigger = new_data_trigger();
    let plan = derive_maintenance_plan(&inputs, std::slice::from_ref(&trigger));

    assert!(
        plan.refusals.is_empty(),
        "append-only posture must not refuse, got {:?}",
        plan.refusals
    );
    let cell = plan
        .cell_for(&trigger)
        .unwrap_or_else(|| panic!("expected the ordinary KeyedFold cell: {plan:#?}"));
    assert_eq!(
        cell.technique,
        Technique::KeyedFold,
        "no repair narrowing should fire once the posture obligation already passes"
    );
}
