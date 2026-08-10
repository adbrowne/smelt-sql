//! Phase 4 (`docs/outcomes/20260809-output-delta-typing/phases/04-plan.md`):
//! `build_forward_graph` wires the derived output-delta shapes onto real
//! propagation edges via `type_edge` — `Edge.components` is non-empty for a
//! real workspace, not just in `smelt-logical`'s own unit tests.

use std::path::Path;

use smelt_core::{discover_source_infos, ModelDiscovery};
use smelt_logical::maintenance::edge_type::Addressing;
use smelt_logical::maintenance::propagate::{propagate, DayInterval};
use smelt_runtime::propagation::build_forward_graph;

fn write(dir: &Path, rel: &str, content: &str) {
    let path = dir.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

fn smelt_yml(dir: &Path) {
    write(
        dir,
        "smelt.yml",
        "name: typed_edge_ws\nversion: 1\npaths:\n  - models\n\
         targets:\n  dev:\n    type: duckdb\n    schema: main\n\
         default_materialization: view\n",
    );
}

#[test]
fn forward_graph_edge_from_window_source_carries_window_component() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path();
    smelt_yml(root);
    write(
        root,
        "models/sources/bronze.yml",
        "description: bronze\ncolumns:\n- name: id\n  type: INTEGER\n- name: d\n  type: DATE\n\
         mutation_profile:\n  kind: append_only\n\
         timeseries:\n  partition_column: d\n  event_time_column: d\n  granularity: day\n",
    );
    write(
        root,
        "models/silver.sql",
        "---\nmaterialization: table\nrefresh: incremental\ngrain: partition\n\
         timeseries:\n  partition_column: d\n  event_time_column: d\n  granularity: day\n---\n\
         SELECT id, d FROM smelt.sources.bronze\n",
    );

    let discovery = ModelDiscovery::new(root.to_path_buf(), vec!["models".to_string()]);
    let models = discovery.discover_models().expect("discover models");
    let source_infos = discover_source_infos(root, &["models".to_string()]);

    let edges = build_forward_graph(&models, &source_infos).expect("build graph");
    assert_eq!(edges.len(), 1, "{edges:?}");
    let edge = &edges[0];
    assert_eq!(edge.upstream, "bronze");
    assert!(
        !edge.components.is_empty(),
        "a sources.* upstream edge must carry typed components: {edge:?}"
    );
    assert!(
        edge.components
            .iter()
            .any(|c| matches!(&c.addressing, Addressing::Window { axis } if axis == "d")),
        "expected a Window{{axis: \"d\"}} component from the append-only source, got {:?}",
        edge.components
    );
}

/// A keyed-upsert upstream model (`GROUP BY` over an append-only source)
/// feeding a downstream model via a `smelt.models.*` ref — the model-edge
/// change-feed case: the edge from the upstream model must carry a
/// `Keyed` component, not an empty vector.
#[test]
fn forward_graph_edge_from_model_upstream_carries_components() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path();
    smelt_yml(root);
    write(
        root,
        "models/sources/bronze.yml",
        "description: bronze\ncolumns:\n- name: user_id\n  type: INTEGER\n- name: d\n  type: DATE\n\
         - name: amount\n  type: DOUBLE\n\
         mutation_profile:\n  kind: append_only\n\
         timeseries:\n  partition_column: d\n  event_time_column: d\n  granularity: day\n",
    );
    write(
        root,
        "models/agg.sql",
        "---\nmaterialization: table\nrefresh: incremental\ngrain: partition\n\
         timeseries:\n  partition_column: d\n  event_time_column: d\n  granularity: day\n---\n\
         SELECT user_id, d, SUM(amount) AS total FROM smelt.sources.bronze GROUP BY user_id, d\n",
    );
    write(
        root,
        "models/downstream.sql",
        "---\nmaterialization: table\nrefresh: incremental\ngrain: partition\n\
         timeseries:\n  partition_column: d\n  event_time_column: d\n  granularity: day\n---\n\
         SELECT user_id, d, total FROM smelt.models.agg\n",
    );

    let discovery = ModelDiscovery::new(root.to_path_buf(), vec!["models".to_string()]);
    let models = discovery.discover_models().expect("discover models");
    let source_infos = discover_source_infos(root, &["models".to_string()]);

    let edges = build_forward_graph(&models, &source_infos).expect("build graph");
    let model_edge = edges
        .iter()
        .find(|e| e.upstream == "agg" && e.downstream == "downstream")
        .unwrap_or_else(|| panic!("expected an agg -> downstream edge, got {edges:?}"));
    assert!(
        !model_edge.components.is_empty(),
        "a model upstream edge must carry typed components: {model_edge:?}"
    );
    assert!(
        model_edge
            .components
            .iter()
            .any(|c| matches!(&c.addressing, Addressing::Keyed { .. })),
        "expected a Keyed component from the keyed-upsert upstream model, got {:?}",
        model_edge.components
    );
}

/// Populating `Edge.components` is advisory: `propagate` over the typed
/// graph returns the same interval dirt as over the same graph with
/// `components` cleared (the widen-never-narrow invariant phase 3
/// established, re-pinned here against the real graph builder).
#[test]
fn component_population_does_not_change_interval_dirt() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path();
    smelt_yml(root);
    write(
        root,
        "models/sources/bronze.yml",
        "description: bronze\ncolumns:\n- name: id\n  type: INTEGER\n- name: d\n  type: DATE\n\
         mutation_profile:\n  kind: append_only\n\
         timeseries:\n  partition_column: d\n  event_time_column: d\n  granularity: day\n",
    );
    write(
        root,
        "models/silver.sql",
        "---\nmaterialization: table\nrefresh: incremental\ngrain: partition\n\
         timeseries:\n  partition_column: d\n  event_time_column: d\n  granularity: day\n---\n\
         SELECT id, d FROM smelt.sources.bronze\n",
    );

    let discovery = ModelDiscovery::new(root.to_path_buf(), vec!["models".to_string()]);
    let models = discovery.discover_models().expect("discover models");
    let source_infos = discover_source_infos(root, &["models".to_string()]);

    let typed_edges = build_forward_graph(&models, &source_infos).expect("build graph");
    let mut untyped_edges = typed_edges.clone();
    for e in &mut untyped_edges {
        e.components.clear();
    }

    let mut deltas = std::collections::BTreeMap::new();
    deltas.insert("bronze".to_string(), vec![DayInterval::new(20, 21)]);

    let typed = propagate(&typed_edges, &deltas).expect("propagate typed");
    let untyped = propagate(&untyped_edges, &deltas).expect("propagate untyped");
    assert_eq!(
        typed.dirty, untyped.dirty,
        "typed components must not change propagated dirt"
    );
    assert_eq!(typed.per_edge, untyped.per_edge);
}

/// Phase 5 (`docs/outcomes/20260809-output-delta-typing/phases/05-plan.md`):
/// a bare (not locality-admitted) `grain: key` model whose own derived
/// output-delta shape is `KeyedUpsert` feeds a downstream consumer without
/// tripping the propagation graph's keyed-grain refusal — the keyed dirt-set
/// channel admits it instead.
#[test]
fn keyed_upstream_model_propagates_on_the_real_graph() {
    use smelt_logical::maintenance::propagate::propagate;

    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path();
    smelt_yml(root);
    write(
        root,
        "models/sources/payments.yml",
        "description: payments\ncolumns:\n- name: user_id\n  type: INTEGER\n\
         - name: amount\n  type: DECIMAL(10,2)\n- name: d\n  type: DATE\n\
         mutation_profile:\n  kind: append_only\n\
         timeseries:\n  partition_column: d\n  event_time_column: d\n  granularity: day\n",
    );
    write(
        root,
        "models/agg.sql",
        "---\nmaterialization: table\nrefresh: incremental\ngrain: key\n\
         timeseries:\n  partition_column: d\n  event_time_column: d\n  granularity: day\n---\n\
         SELECT user_id, SUM(amount) AS total, MAX(d) AS d\n\
         FROM smelt.sources.payments\n\
         GROUP BY user_id\n",
    );
    write(
        root,
        "models/downstream.sql",
        "---\nmaterialization: table\nrefresh: incremental\ngrain: partition\n\
         timeseries:\n  partition_column: d\n  event_time_column: d\n  granularity: day\n---\n\
         SELECT user_id, total, d FROM smelt.models.agg\n",
    );

    let discovery = ModelDiscovery::new(root.to_path_buf(), vec!["models".to_string()]);
    let models = discovery.discover_models().expect("discover models");
    let source_infos = discover_source_infos(root, &["models".to_string()]);

    let edges = build_forward_graph(&models, &source_infos).expect("build graph");
    let agg_edge = edges
        .iter()
        .find(|e| e.upstream == "agg")
        .unwrap_or_else(|| panic!("expected an edge with agg as upstream, got {edges:?}"));
    assert!(
        agg_edge
            .components
            .iter()
            .any(|c| matches!(&c.addressing, Addressing::Keyed { .. })),
        "expected a Keyed component from the keyed-upsert 'agg' model, got {:?}",
        agg_edge.components
    );

    let mut deltas = std::collections::BTreeMap::new();
    deltas.insert("agg".to_string(), vec![DayInterval::new(1, 2)]);
    propagate(&edges, &deltas).expect("an admitted keyed edge must propagate without refusing");
}
