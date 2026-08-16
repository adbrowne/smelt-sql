//! Phase 3 (`docs/outcomes/20260816-scheduler-delta-signatures/outcome.md`):
//! key-valued dirt-sets through the graph layer. `KeyedDirt` gains a
//! resolved-values payload distinct from the merely symbolic key-column
//! marker, `propagate_with_keys` accepts keyed seeds as pure input, and
//! composition projects an upstream's key values onto each consumer's own
//! key scope — widening to whole-model dirt when the projection does not
//! resolve (`incremental_models.md` §"Keyed dirt-sets and the narrowed
//! refusal", §"Composition rules", §"Unresolved seeds").
//!
//! Pure math only — no CLI, no runtime, no on-disk workspace, mirroring
//! `maintenance_propagation_adjoint.rs`'s style.

use std::collections::BTreeMap;

use smelt_logical::analysis::output_delta::OutputDelta;
use smelt_logical::maintenance::edge_type::{Addressing, EdgeComponent};
use smelt_logical::maintenance::propagate::{
    propagate, propagate_with_keys, DayInterval, Edge, KeyValues, KeyedDirt, PartitionGrain,
};

fn iv(start: i64, end: i64) -> DayInterval {
    DayInterval::new(start, end)
}

fn deltas(items: &[(&str, DayInterval)]) -> BTreeMap<String, Vec<DayInterval>> {
    let mut m: BTreeMap<String, Vec<DayInterval>> = BTreeMap::new();
    for (name, interval) in items {
        m.entry(name.to_string()).or_default().push(*interval);
    }
    m
}

fn edge(upstream: &str, downstream: &str) -> Edge {
    Edge {
        upstream: upstream.to_string(),
        downstream: downstream.to_string(),
        before_days: 0,
        after_days: 0,
        upstream_grain: PartitionGrain::Keyed,
        downstream_grain: PartitionGrain::Keyed,
        components: Vec::new(),
        consumer_key_scope: Vec::new(),
    }
}

fn keyed_component(keys: &[&str]) -> EdgeComponent {
    EdgeComponent {
        group: "{id}".to_string(),
        shape: OutputDelta::KeyedUpsert {
            keys: keys.iter().map(|k| k.to_string()).collect(),
        },
        addressing: Addressing::Keyed {
            keys: keys.iter().map(|k| k.to_string()).collect(),
        },
        columns: keys.iter().map(|k| k.to_string()).collect(),
    }
}

fn seeds(items: &[(&str, KeyValues)]) -> BTreeMap<String, KeyValues> {
    items
        .iter()
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect()
}

#[test]
fn seeded_key_values_reach_the_downstream_dirt_set() {
    let mut e = edge("agg", "consumer");
    e.components = vec![keyed_component(&["user_id"])];
    e.consumer_key_scope = vec!["user_id".to_string()];
    let edges = vec![e];

    let seeded = seeds(&[(
        "agg",
        KeyValues::Resolved(vec!["u1".to_string(), "u2".to_string()]),
    )]);
    let result =
        propagate_with_keys(&edges, &BTreeMap::new(), &seeded).expect("propagate with keys");

    let per_edge = result
        .per_edge_keys
        .get(&("consumer".to_string(), "agg".to_string()))
        .expect("keyed channel entry must exist");
    assert_eq!(
        per_edge,
        &vec![KeyedDirt {
            keys: vec!["user_id".to_string()],
            from: "agg".to_string(),
            values: KeyValues::Resolved(vec!["u1".to_string(), "u2".to_string()]),
        }]
    );
    let merged = result
        .keyed_dirty
        .get("consumer")
        .expect("consumer must be keyed-dirty");
    assert_eq!(merged, per_edge);
}

#[test]
fn empty_resolved_seed_is_not_an_unresolved_seed() {
    let mut e = edge("agg", "consumer");
    e.components = vec![keyed_component(&["user_id"])];
    e.consumer_key_scope = vec!["user_id".to_string()];
    let edges = vec![e];

    let resolved_empty = seeds(&[("agg", KeyValues::Resolved(Vec::new()))]);
    let result = propagate_with_keys(&edges, &BTreeMap::new(), &resolved_empty)
        .expect("propagate with keys");
    let kd = &result.keyed_dirty.get("consumer").expect("keyed-dirty")[0];
    assert_eq!(kd.values, KeyValues::Resolved(Vec::new()));
    assert!(
        !result.dirty.contains_key("consumer"),
        "an empty resolved seed must not add whole-model interval dirt: {:?}",
        result.dirty
    );

    // An absent seed (no entry in the seed map at all) propagates
    // Unresolved with a reason, and also adds no whole-model dirt. The node
    // still needs an interval delta to be visited at all (mirrors every
    // other keyed-origin test in this suite family) — the absence under
    // test is the keyed seed, not the node's own visitation.
    let result = propagate(&edges, &deltas(&[("agg", iv(1, 2))])).expect("propagate");
    let kd = &result.keyed_dirty.get("consumer").expect("keyed-dirty")[0];
    assert!(
        matches!(&kd.values, KeyValues::Unresolved { .. }),
        "an absent seed must propagate Unresolved: {kd:?}"
    );
    assert!(
        !result.dirty.contains_key("consumer"),
        "an absent seed must not add whole-model interval dirt either: {:?}",
        result.dirty
    );
}

#[test]
fn keys_that_do_not_project_onto_the_consumer_key_scope_widen_to_whole_model() {
    let mut e = edge("agg", "consumer");
    e.components = vec![keyed_component(&["user_id"])];
    // Mismatched scope: the consumer restricts by "account_id", not the
    // upstream's admitted "user_id".
    e.consumer_key_scope = vec!["account_id".to_string()];
    let edges = vec![e];

    let seeded = seeds(&[("agg", KeyValues::Resolved(vec!["u1".to_string()]))]);
    let result =
        propagate_with_keys(&edges, &BTreeMap::new(), &seeded).expect("propagate with keys");

    let kd = &result
        .per_edge_keys
        .get(&("consumer".to_string(), "agg".to_string()))
        .expect("keyed channel entry must exist")[0];
    match &kd.values {
        KeyValues::Unresolved { reason } => {
            assert!(reason.contains("user_id"), "{reason}");
            assert!(reason.contains("account_id"), "{reason}");
        }
        other => panic!("expected Unresolved on scope mismatch, got {other:?}"),
    }
    let dirty = result
        .dirty
        .get("consumer")
        .expect("mismatched scope must still widen to whole-model dirt");
    assert!(
        dirty.iter().any(|d| d.is_whole()),
        "expected whole-table dirt on scope mismatch, even for a keyed-grain consumer: {dirty:?}"
    );
}

#[test]
fn key_values_compose_through_a_two_hop_keyed_chain() {
    let mut ab = edge("a", "b");
    ab.components = vec![keyed_component(&["user_id"])];
    ab.consumer_key_scope = vec!["user_id".to_string()];
    let mut bc = edge("b", "c");
    bc.components = vec![keyed_component(&["user_id"])];
    bc.consumer_key_scope = vec!["user_id".to_string()];
    let edges = vec![ab, bc];

    let seeded = seeds(&[("a", KeyValues::Resolved(vec!["u1".to_string()]))]);
    let result =
        propagate_with_keys(&edges, &BTreeMap::new(), &seeded).expect("propagate with keys");

    let kd = &result
        .per_edge_keys
        .get(&("c".to_string(), "b".to_string()))
        .expect("b -> c keyed channel entry must exist")[0];
    assert_eq!(
        kd.values,
        KeyValues::Resolved(vec!["u1".to_string()]),
        "the seeded value must carry through the second hop: {kd:?}"
    );
    assert!(
        !result.dirty.contains_key("c"),
        "a fully-resolved two-hop chain must not widen to whole-model dirt: {:?}",
        result.dirty
    );
}

#[test]
fn propagate_without_keyed_seeds_is_unchanged() {
    let mut e = edge("agg", "consumer");
    e.components = vec![keyed_component(&["user_id"])];
    let edges = vec![e];

    let with_wrapper = propagate(&edges, &deltas(&[("agg", iv(1, 2))])).expect("propagate");
    let with_empty_seeds =
        propagate_with_keys(&edges, &deltas(&[("agg", iv(1, 2))]), &BTreeMap::new())
            .expect("propagate_with_keys");

    assert_eq!(with_wrapper.dirty, with_empty_seeds.dirty);
    assert_eq!(with_wrapper.per_edge, with_empty_seeds.per_edge);
    assert_eq!(with_wrapper.keyed_dirty, with_empty_seeds.keyed_dirty);
    assert_eq!(with_wrapper.per_edge_keys, with_empty_seeds.per_edge_keys);
}
