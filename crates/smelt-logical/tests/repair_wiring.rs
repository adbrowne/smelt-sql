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
    ColumnGroup, Corner, Grain, MutationProfile, OutputSpec, Refusal, RowIdentity, SourceFacts,
    Technique, Trigger,
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
        keyed_time_axis: None,
        old_partition_col: None,
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

// ── `KeyedRetractableContribution` (`incremental_shapes.md` §"Enrichment
// joins") ─────────────────────────────────────────────────────────────────
//
// A `grain: key` model that folds a value off a JOINED `mutable_snapshot`
// dimension, rather than off the driving source itself — the enrichment-join
// shape `join_shape::join_contribution_monotone` proves against, composed
// here with `repair::admit_per_group_recompute`'s own admission for the
// dimension's `NewData` trigger.

fn customers_source(partition_col: Option<&str>, unique_key: &[&str]) -> SourceFacts {
    SourceFacts {
        name: "customers".to_string(),
        mutation: MutationProfile::MutableSnapshot,
        partition_col: partition_col.map(|c| c.to_string()),
        unique_key: strings(unique_key),
        allow_full_scan: false,
    }
}

fn enrichment_inputs(
    sql: &'static str,
    customers: SourceFacts,
    combiner: SqlFunction,
    fold_column: &str,
) -> ModelInputs<'static> {
    ModelInputs {
        sql,
        output: OutputSpec {
            table: "customer_totals".to_string(),
            grain: Grain::Key {
                unique_key: strings(&["customer_id"]),
            },
            skeleton_columns: set(&["customer_id"]),
        },
        sources: vec![
            SourceFacts {
                name: "orders".to_string(),
                mutation: MutationProfile::AppendOnly,
                partition_col: None,
                unique_key: strings(&["order_id"]),
                allow_full_scan: false,
            },
            customers,
        ],
        column_groups: vec![ColumnGroup {
            columns: strings(&[fold_column]),
            mutation_sensitivity: set(&["customers"]),
            membership_sensitivity: BTreeSet::new(),
        }],
        fold: Some(FoldSpec {
            add_columns: vec![(fold_column.to_string(), combiner)],
        }),
        old_columns: Vec::new(),
        old_sql: None,
        keyed_time_axis: None,
        old_partition_col: None,
    }
}

fn customers_new_data_trigger() -> Trigger {
    Trigger::NewData {
        source: "customers".to_string(),
    }
}

const SUM_ENRICHMENT_SQL: &str = "SELECT o.customer_id, SUM(c.discount) AS total_discount \
     FROM smelt.sources.orders o \
     JOIN smelt.sources.customers c ON o.customer_id = c.customer_id \
     GROUP BY o.customer_id";

const MAX_ENRICHMENT_SQL: &str = "SELECT o.customer_id, MAX(c.priority) AS max_priority \
     FROM smelt.sources.orders o \
     JOIN smelt.sources.customers c ON o.customer_id = c.customer_id \
     GROUP BY o.customer_id";

#[test]
fn retractable_enrichment_contribution_refuses_by_name() {
    // `customers` declares no `unique_key` — the join's fan-out cannot be
    // proven one-to-one, and `SUM` is a decrementing aggregate (a monoid
    // with an inverse), so the enrichment contribution is retractable. The
    // same missing `unique_key` also makes the delta unkeyed, so repair's
    // affected-key discovery fails closed too.
    let inputs = enrichment_inputs(
        SUM_ENRICHMENT_SQL,
        customers_source(None, &[]),
        SqlFunction::Sum,
        "total_discount",
    );
    let trigger = customers_new_data_trigger();
    let plan = derive_maintenance_plan(&inputs, std::slice::from_ref(&trigger));

    assert!(
        plan.refusals
            .iter()
            .any(|r| matches!(r, Refusal::NoAdmissibleTechnique { .. })),
        "the pre-existing posture refusal must still be pushed, got {:?}",
        plan.refusals
    );
    assert!(
        plan.refusals
            .iter()
            .any(|r| matches!(r, Refusal::RepairKeysNotDiscoverable { source, .. } if source == "customers")),
        "expected the pre-existing RepairKeysNotDiscoverable refusal, got {:?}",
        plan.refusals
    );
    let retractable = plan.refusals.iter().find_map(|r| match r {
        Refusal::KeyedRetractableContribution {
            source,
            columns,
            why,
        } => Some((source, columns, why)),
        _ => None,
    });
    let (source, columns, why) = retractable.unwrap_or_else(|| {
        panic!("expected an additive KeyedRetractableContribution refusal, got {plan:#?}")
    });
    assert_eq!(source, "customers");
    assert_eq!(columns, &vec!["total_discount".to_string()]);
    assert!(
        why.contains("decrementing aggregate"),
        "expected the join-contribution reason to name the decrementing aggregate, got {why}"
    );
    assert!(
        why.contains("per-row identity"),
        "expected the failing repair obligation's own reason verbatim, got {why}"
    );
}

#[test]
fn monotone_enrichment_contribution_emits_no_retractable_refusal() {
    // `customers` now declares `unique_key: [customer_id]`, matching the
    // join's equality condition — the fan-out is provably one-to-one, and
    // `MAX` is value-monotone, so `join_contribution_monotone` proves the
    // contribution monotone. Repair still refuses (affected-key discovery
    // has no join context to project the output grain's `customer_id`
    // through to `customers`' own delta — a pre-existing, independent
    // limitation of `admit_per_group_recompute`, not something this phase
    // touches), but that refusal is never widened into
    // `KeyedRetractableContribution`: only the composed cardinality +
    // combiner-algebra proof decides this, never the mere presence of a
    // join.
    let inputs = enrichment_inputs(
        MAX_ENRICHMENT_SQL,
        customers_source(None, &["customer_id"]),
        SqlFunction::Max,
        "max_priority",
    );
    let trigger = customers_new_data_trigger();
    let plan = derive_maintenance_plan(&inputs, std::slice::from_ref(&trigger));

    assert!(
        plan.refusals
            .iter()
            .any(|r| matches!(r, Refusal::NoAdmissibleTechnique { .. })),
        "the pre-existing posture refusal must still be pushed, got {:?}",
        plan.refusals
    );
    assert!(
        plan.refusals
            .iter()
            .any(|r| matches!(r, Refusal::RepairKeysNotDiscoverable { source, .. } if source == "customers")),
        "expected the pre-existing RepairKeysNotDiscoverable refusal, got {:?}",
        plan.refusals
    );
    assert!(
        !plan
            .refusals
            .iter()
            .any(|r| matches!(r, Refusal::KeyedRetractableContribution { .. })),
        "a provably monotone enrichment-join contribution must never emit \
         KeyedRetractableContribution, got {:?}",
        plan.refusals
    );
}

#[test]
fn admitted_repair_emits_no_retractable_refusal() {
    // The driving source itself (`orders`, not a joined dimension) admits
    // repair exactly as `keyed_fold_over_mutable_source_derives_a_per_group_
    // recompute_cell` does — clocked and keyed, so `admit_per_group_
    // recompute` returns `Ok`. Its combiner is `SUM` (algebraically the
    // same "decrementing aggregate" shape `retractable_enrichment_
    // contribution_refuses_by_name` names), but there is no JOIN against
    // `orders` in this SQL at all — `join_shape::join_alias_for_source`
    // returns `None`, so the new check's `if let Some(alias)` never even
    // runs. This regresses the "the `Err` arm this refusal derives from is
    // never reached" guarantee at the structural level: even a combiner
    // shape that would be judged retractable through a join never gets
    // `KeyedRetractableContribution` when `admit_per_group_recompute`
    // itself admits.
    let mut source = orders_source(Some("order_date"), &["order_id"]);
    source.mutation = MutationProfile::MutableSnapshot;
    let inputs = ModelInputs {
        fold: Some(FoldSpec {
            add_columns: vec![("max_amount".to_string(), SqlFunction::Sum)],
        }),
        ..inputs(CLOCKED_KEYED_SQL, source)
    };
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
    assert!(
        !plan
            .refusals
            .iter()
            .any(|r| matches!(r, Refusal::KeyedRetractableContribution { .. })),
        "admitted repair must never carry KeyedRetractableContribution, got {:?}",
        plan.refusals
    );
}
