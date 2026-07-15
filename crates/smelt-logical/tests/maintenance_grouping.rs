//! TDD tests for `smelt_logical::maintenance::grouping` — the
//! mutation-sensitivity column-grouping derivation
//! (`docs/specs/incremental_models.md` §Design "Factoring by
//! mutation-sensitivity"; `docs/specs/model_properties.md` §"Per-column
//! mutation-sensitivity / column provenance").

use std::collections::BTreeSet;

use smelt_logical::maintenance::grouping::derive_column_groups;
use smelt_logical::maintenance::{MutationProfile, SourceFacts};

fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn source(
    name: &str,
    mutation: MutationProfile,
    partition_col: Option<&str>,
    unique_key: &[&str],
) -> SourceFacts {
    SourceFacts {
        name: name.to_string(),
        mutation,
        partition_col: partition_col.map(|s| s.to_string()),
        unique_key: unique_key.iter().map(|s| s.to_string()).collect(),
        allow_full_scan: false,
    }
}

/// The load-bearing append-only case (`incremental_models.md` §Design
/// "Factoring by mutation-sensitivity"): a column that reads only an
/// append-only source, without aggregating over it, is immutable at
/// creation — the source can only ever add new rows, never revise the one
/// this column already read. `tier`, which joins the mutable `customers`
/// dimension, must still land in its own sensitive group so the test also
/// proves the append-only reference does not drag the whole row along.
#[test]
fn immutable_at_creation_reference_contributes_no_sensitivity() {
    let sources = vec![
        source(
            "orders",
            MutationProfile::AppendOnly,
            Some("order_date"),
            &[],
        ),
        source(
            "customers",
            MutationProfile::MutableSnapshot,
            None,
            &["user_id"],
        ),
    ];
    let sql = "SELECT o.order_id, o.order_date, o.amount, c.tier \
               FROM smelt.sources.orders o \
               JOIN smelt.sources.customers c ON c.user_id = o.user_id";
    let skeleton = set(&["order_id", "order_date"]);
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
    assert!(
        amount_group.mutation_sensitivity.is_empty(),
        "an append-only, non-aggregated reference must contribute no sensitivity: {:?}",
        amount_group.mutation_sensitivity
    );

    let tier_group = result
        .groups
        .iter()
        .find(|g| g.columns.contains(&"tier".to_string()))
        .expect("tier is grouped");
    assert_eq!(tier_group.mutation_sensitivity, set(&["customers"]));
    // The two must be different groups: the append-only reference did not
    // drag `amount` into `tier`'s mutation-sensitive group.
    assert_ne!(amount_group.columns, tier_group.columns);
}

/// A column that reaches two independently mutation-sensitive sources must
/// carry the *union* of both — never silently keep just one (fail-closed) —
/// and that union forms its own group, distinct from either single-source
/// group.
#[test]
fn two_source_projection_merges_groups_fail_closed() {
    let sources = vec![
        source(
            "orders",
            MutationProfile::AppendOnly,
            Some("order_date"),
            &[],
        ),
        source(
            "customers",
            MutationProfile::MutableSnapshot,
            None,
            &["user_id"],
        ),
        source(
            "warehouses",
            MutationProfile::MutableSnapshot,
            None,
            &["warehouse_id"],
        ),
    ];
    let sql = "SELECT o.order_id, o.order_date, c.tier, w.region, \
               c.tier || w.region AS tier_region \
               FROM smelt.sources.orders o \
               JOIN smelt.sources.customers c ON c.user_id = o.user_id \
               JOIN smelt.sources.warehouses w ON w.warehouse_id = o.warehouse_id";
    let skeleton = set(&["order_id", "order_date"]);
    let result = derive_column_groups(sql, &sources, &skeleton);

    assert!(
        result.degenerate.is_empty(),
        "degenerate: {:?}",
        result.degenerate
    );

    let tier_region_group = result
        .groups
        .iter()
        .find(|g| g.columns.contains(&"tier_region".to_string()))
        .expect("tier_region is grouped");
    assert_eq!(
        tier_region_group.mutation_sensitivity,
        set(&["customers", "warehouses"]),
        "must carry both sources' sensitivity, never drop one"
    );

    let tier_group = result
        .groups
        .iter()
        .find(|g| g.columns == vec!["tier".to_string()])
        .expect("tier has its own single-source group");
    assert_eq!(tier_group.mutation_sensitivity, set(&["customers"]));
    let region_group = result
        .groups
        .iter()
        .find(|g| g.columns == vec!["region".to_string()])
        .expect("region has its own single-source group");
    assert_eq!(region_group.mutation_sensitivity, set(&["warehouses"]));

    // Three distinct sensitivity sets ⇒ three distinct groups — the merged
    // column never silently joins either single-source group.
    assert_ne!(tier_region_group.columns, tier_group.columns);
    assert_ne!(tier_region_group.columns, region_group.columns);
}

/// An unqualified column reference ambiguous among more than one joined
/// source cannot be resolved to a single provenance — the derivation must
/// fail closed (collapse to the whole model's sources) and *surface* the
/// collapse, never silently drop coverage or silently pick one source.
#[test]
fn degenerate_collapse_is_surfaced() {
    let sources = vec![
        source(
            "orders",
            MutationProfile::AppendOnly,
            Some("order_date"),
            &[],
        ),
        source(
            "customers",
            MutationProfile::MutableSnapshot,
            None,
            &["user_id"],
        ),
    ];
    // `amount` is unqualified with two joined inputs in scope — ambiguous.
    let sql = "SELECT o.order_id, o.order_date, amount \
               FROM smelt.sources.orders o \
               JOIN smelt.sources.customers c ON c.user_id = o.user_id";
    let skeleton = set(&["order_id", "order_date"]);
    let result = derive_column_groups(sql, &sources, &skeleton);

    assert!(
        !result.degenerate.is_empty(),
        "the collapse must be named, never silent"
    );
    assert!(
        result.degenerate.iter().any(|d| d.column == "amount"),
        "degenerate: {:?}",
        result.degenerate
    );

    // Fail-closed: still produces a plan (widened, never dropped) — one
    // group over every declared source ("the whole table").
    assert_eq!(result.groups.len(), 1);
    assert_eq!(
        result.groups[0].mutation_sensitivity,
        set(&["orders", "customers"])
    );
    assert!(result.groups[0].columns.contains(&"amount".to_string()));
}
