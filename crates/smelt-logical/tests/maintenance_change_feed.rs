//! Phase 28c (`docs/outcomes/20260815-definition-delta-migrate`): a source
//! declaring `mutation_profile: change_feed` gets an `UpstreamMutation` cell
//! like every other mutation-sensitive posture (`docs/specs/
//! incremental_models.md` §"Which changed inputs get a mutation cell"),
//! admitted conservatively as full-input re-derivation — never a
//! column-scoped merge, never the fingerprint-sidecar repair family (no
//! live fold over the feed's own delta shape exists yet, §Known
//! Divergences).

use std::collections::{BTreeSet, HashSet};

use smelt_logical::maintenance::derive::{
    derive_maintenance_plan, derive_triggers, FoldSpec, ModelInputs,
};
use smelt_logical::maintenance::{
    ColumnGroup, Corner, Grain, MutationProfile, OutputSpec, PartitionLocal, Refusal, SourceFacts,
    Technique, Trigger,
};
use smelt_types::SqlFunction;

fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn change_feed_source(name: &str, allow_full_scan: bool) -> SourceFacts {
    SourceFacts {
        name: name.to_string(),
        mutation: MutationProfile::ChangeFeed,
        partition_col: None,
        unique_key: vec![],
        allow_full_scan,
    }
}

/// A `ChangeFeed` source gets a derived `Trigger::UpstreamMutation`, exactly
/// like an explicitly-declared `MutableSnapshot` — but the declaration
/// alone suffices; no `explicitly_mutable` entry is needed (a change feed
/// can only ever arise from an explicit declaration, so there is no
/// fail-closed default to guard against).
#[test]
fn change_feed_source_derives_upstream_mutation_trigger() {
    let sources = vec![change_feed_source("feed", true)];
    let triggers = derive_triggers(&sources, &[], &HashSet::new(), &[]);
    assert!(
        triggers
            .iter()
            .any(|t| matches!(t, Trigger::UpstreamMutation { source } if source == "feed")),
        "expected an UpstreamMutation trigger for the change_feed source, got {triggers:?}"
    );
}

fn base_inputs(sources: Vec<SourceFacts>, column_groups: Vec<ColumnGroup>) -> ModelInputs<'static> {
    ModelInputs {
        sql: "SELECT id, amount FROM smelt.sources.feed",
        output: OutputSpec {
            table: "widget".to_string(),
            grain: Grain::Key {
                unique_key: strings(&["id"]),
            },
            skeleton_columns: set(&["id"]),
        },
        sources,
        column_groups,
        fold: None,
        old_columns: Vec::new(),
        old_sql: None,
        keyed_time_axis: None,
        old_partition_col: None,
    }
}

/// The derived `UpstreamMutation` cell for a change-feed-sensitive group is
/// clamped to full-input re-derivation: `RecomputeRegion`/`DeleteInsert`,
/// never `ColumnScopedMerge`, and — since a change feed carries no clock in
/// this fixture — no partition-local clamp either.
#[test]
fn change_feed_cell_takes_full_input_rederivation() {
    let sources = vec![change_feed_source("feed", true)];
    let column_groups = vec![ColumnGroup {
        columns: strings(&["amount"]),
        mutation_sensitivity: set(&["feed"]),
        membership_sensitivity: BTreeSet::new(),
    }];
    let inputs = base_inputs(sources, column_groups);
    let trigger = Trigger::UpstreamMutation {
        source: "feed".to_string(),
    };
    let plan = derive_maintenance_plan(&inputs, std::slice::from_ref(&trigger));

    assert!(
        plan.refusals.is_empty(),
        "expected the cell to be admitted, got refusals {:?}",
        plan.refusals
    );
    let cell = plan
        .cell_for(&trigger)
        .unwrap_or_else(|| panic!("expected an UpstreamMutation cell: {plan:#?}"));
    assert_eq!(cell.corner, Corner::RecomputeRegion);
    assert_eq!(cell.technique, Technique::DeleteInsert);
    assert!(
        !matches!(cell.partition_local, PartitionLocal::Yes),
        "an unclocked change_feed source clamps to no partition-local scan, got {:?}",
        cell.partition_local
    );
    assert!(
        !plan
            .cells
            .iter()
            .any(|c| c.technique == Technique::ColumnScopedMerge),
        "no ColumnScopedMerge cell should exist for a change_feed-sensitive group, got {:?}",
        plan.cells
    );
}

/// `derive_column_groups` treats a `ChangeFeed` leaf the same as a
/// `MutableSnapshot` one for value sensitivity: a plain (non-aggregate)
/// column reference still contributes sensitivity, unlike an `AppendOnly`
/// reference which contributes only when aggregated.
#[test]
fn change_feed_group_is_value_sensitive_like_mutable_snapshot() {
    use smelt_logical::maintenance::grouping::derive_column_groups;

    let sources = vec![change_feed_source("feed", true)];
    let sql = "SELECT id, amount FROM smelt.sources.feed";
    let skeleton = set(&["id"]);
    let result = derive_column_groups(sql, &sources, &skeleton);

    assert!(
        result.degenerate.is_empty(),
        "degenerate: {:?}",
        result.degenerate
    );
    let amount_group = result
        .groups
        .iter()
        .find(|g| g.columns.contains(&"amount".to_string()))
        .expect("amount is grouped");
    assert_eq!(
        amount_group.mutation_sensitivity,
        set(&["feed"]),
        "a plain reference to a change_feed source must contribute sensitivity, same as \
         mutable_snapshot, got {:?}",
        amount_group.mutation_sensitivity
    );
}

const CLOCKED_KEYED_SQL: &str = "SELECT customer_id, MAX(amount) AS max_amount \
     FROM smelt.sources.orders \
     WHERE order_date BETWEEN CURRENT_DATE - INTERVAL '1 day' AND CURRENT_DATE \
     GROUP BY customer_id";

fn orders_change_feed_source(partition_col: Option<&str>, unique_key: &[&str]) -> SourceFacts {
    SourceFacts {
        name: "orders".to_string(),
        mutation: MutationProfile::ChangeFeed,
        partition_col: partition_col.map(|c| c.to_string()),
        unique_key: strings(unique_key),
        allow_full_scan: false,
    }
}

/// A shape that would, over a `MutableSnapshot` source, admit
/// `Technique::PerGroupRecompute` via the fingerprint-sidecar repair family
/// (`repair_wiring.rs`'s `keyed_fold_over_mutable_source_derives_a_per_group_
/// recompute_cell`) instead refuses fail-loud over a `ChangeFeed` source,
/// naming the source — no sidecar diff exists for a change feed's delta
/// shape, so falling through to one would silently assume machinery that
/// isn't there.
#[test]
fn change_feed_repair_cell_is_refused_not_silently_admitted() {
    let inputs = ModelInputs {
        sql: CLOCKED_KEYED_SQL,
        output: OutputSpec {
            table: "customer_max_amount".to_string(),
            grain: Grain::Key {
                unique_key: strings(&["customer_id"]),
            },
            skeleton_columns: set(&["customer_id"]),
        },
        sources: vec![orders_change_feed_source(Some("order_date"), &["order_id"])],
        column_groups: vec![],
        fold: Some(FoldSpec {
            add_columns: vec![("max_amount".to_string(), SqlFunction::Max)],
        }),
        old_columns: Vec::new(),
        old_sql: None,
        keyed_time_axis: None,
        old_partition_col: None,
    };
    let trigger = Trigger::NewData {
        source: "orders".to_string(),
    };
    let plan = derive_maintenance_plan(&inputs, std::slice::from_ref(&trigger));

    assert!(
        plan.cell_for(&trigger).is_none(),
        "no repair cell should be admitted over a change_feed source, got {:?}",
        plan.cell_for(&trigger)
    );
    assert!(
        !plan
            .cells
            .iter()
            .any(|c| c.technique == Technique::PerGroupRecompute),
        "no PerGroupRecompute cell should be reachable for a change_feed source, got {:?}",
        plan.cells
    );
    assert!(
        plan.refusals.iter().any(|r| matches!(
            r,
            Refusal::NoAdmissibleTechnique { why, .. }
            if why.contains("orders") && why.contains("change_feed")
        )),
        "expected a refusal naming the change_feed source, got {:?}",
        plan.refusals
    );
}
