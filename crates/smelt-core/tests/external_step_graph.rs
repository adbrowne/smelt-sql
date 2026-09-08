//! DAG-membership tests for external steps (phase 3 of
//! `docs/outcomes/20260906-external-dag-steps`).
//!
//! An external step is a node keyed by its own canonical address, carrying
//! an edge to each source it produces. A model never refs a step directly —
//! consumers are derived from the model's own `smelt.sources.*` refs, never
//! from the step's `produces:` list in reverse. See `docs/specs/sources.md`
//! §"Externally-produced sources (black-box steps)" and
//! `docs/specs/model_selection.md` §"Graph traversal".

use rowan::TextRange;
use smelt_core::discovery::{ModelFile, ModelKind, RefInfo};
use smelt_core::external_step::ExternalStepInfo;
use smelt_core::graph::DependencyGraph;
use smelt_core::model_id::ModelId;
use smelt_core::refs::SmeltRef;
use smelt_core::selector::{SelectionMethod, Selector};
use smelt_core::Config;
use std::collections::HashSet;
use std::path::PathBuf;

fn source_ref(segs: &[&str]) -> RefInfo {
    RefInfo {
        has_named_params: false,
        range: TextRange::default(),
        smelt_ref: SmeltRef::Path(segs.iter().map(|s| s.to_string()).collect()),
    }
}

fn model(name: &str, address: &[&str], refs: Vec<RefInfo>) -> ModelFile {
    let path: PathBuf = format!("models/{}.sql", name).into();
    ModelFile {
        name: name.to_string(),
        model_id: ModelId::from_path(path.clone()),
        path,
        content: String::new(),
        refs,
        parse_errors: Vec::new(),
        metadata: None,
        kind: ModelKind::Sql,
        address_segments: address.iter().map(|s| s.to_string()).collect(),
    }
}

fn step(address: &[&str], produces: &[&str]) -> ExternalStepInfo {
    ExternalStepInfo {
        path: format!("models/{}.yml", address.join("/")).into(),
        address_segments: address.iter().map(|s| s.to_string()).collect(),
        description: None,
        produces: produces.iter().map(|s| s.to_string()).collect(),
        command: vec!["bash".to_string(), "loader.sh".to_string()],
        cadence: None,
    }
}

fn selector(method: SelectionMethod, upstream: bool, downstream: bool) -> Selector {
    Selector {
        method,
        include_upstream: upstream,
        include_downstream: downstream,
    }
}

fn name_selector(name: &str, upstream: bool, downstream: bool) -> Selector {
    selector(
        SelectionMethod::ModelName(name.to_string()),
        upstream,
        downstream,
    )
}

fn test_config() -> Config {
    Config::parse_with_warnings(
        "name: test\nversion: 1\ntargets:\n  dev:\n    type: duckdb\n    database: test.duckdb\n    schema: main\n",
    )
    .unwrap()
    .0
}

/// After `add_external_steps`, the step's canonical address is a node and
/// `producing_step_of_source` answers for every `produces:` entry.
#[test]
fn step_registers_as_node_with_edge_per_produced_source() {
    let mut graph = DependencyGraph::build(vec![], None).unwrap();
    graph.add_external_steps(&[step(
        &["loader"],
        &[
            "smelt.sources.raw.events",
            "smelt.sources.raw.events_arrival",
        ],
    )]);

    let steps: Vec<&str> = graph.iter_external_steps().collect();
    assert_eq!(steps, vec!["loader"]);
    assert_eq!(
        graph.producing_step_of_source("sources.raw.events"),
        Some("loader")
    );
    assert_eq!(
        graph.producing_step_of_source("sources.raw.events_arrival"),
        Some("loader")
    );
    assert_eq!(graph.producing_step_of_source("sources.raw.other"), None);
}

/// Consumers are derived from the model's own `smelt.sources.*` refs, not
/// from the source's YAML.
#[test]
fn model_reading_a_produced_source_is_a_consumer() {
    let consumer = model(
        "consumer",
        &["consumer"],
        vec![source_ref(&["sources", "raw", "events"])],
    );
    let mut graph = DependencyGraph::build(vec![consumer], None).unwrap();
    graph.add_external_steps(&[step(&["loader"], &["smelt.sources.raw.events"])]);

    let config = test_config();
    let selection = graph
        .select_nodes(&[name_selector("loader", false, true)], &config)
        .unwrap();
    assert!(selection.models.contains("consumer"));
}

/// `select_nodes(["+consumer"])` returns the step in `.steps`, transitively
/// through an intermediate model too.
#[test]
fn upstream_selector_on_consumer_includes_the_step() {
    let bronze = model(
        "bronze",
        &["bronze"],
        vec![source_ref(&["sources", "raw", "events"])],
    );
    let silver = model("silver", &["silver"], vec![source_ref(&["bronze"])]);
    let mut graph = DependencyGraph::build(vec![bronze, silver], None).unwrap();
    graph.add_external_steps(&[step(&["loader"], &["smelt.sources.raw.events"])]);

    let config = test_config();
    let selection = graph
        .select_nodes(&[name_selector("silver", true, false)], &config)
        .unwrap();
    assert!(selection.steps.contains("loader"));
    assert!(selection.models.contains("bronze"));
    assert!(selection.models.contains("silver"));
}

/// `select_nodes(["step+"])` returns the consumer and its downstreams in
/// `.models`.
#[test]
fn downstream_selector_on_step_includes_consumers() {
    let bronze = model(
        "bronze",
        &["bronze"],
        vec![source_ref(&["sources", "raw", "events"])],
    );
    let silver = model("silver", &["silver"], vec![source_ref(&["bronze"])]);
    let mut graph = DependencyGraph::build(vec![bronze, silver], None).unwrap();
    graph.add_external_steps(&[step(&["loader"], &["smelt.sources.raw.events"])]);

    let config = test_config();
    let selection = graph
        .select_nodes(&[name_selector("loader", false, true)], &config)
        .unwrap();
    assert!(selection.models.contains("bronze"));
    assert!(selection.models.contains("silver"));
}

/// A bare step selector — no "not found" error; `.models` empty.
#[test]
fn bare_step_selector_selects_only_the_step() {
    let mut graph = DependencyGraph::build(vec![], None).unwrap();
    graph.add_external_steps(&[step(&["loader"], &["smelt.sources.raw.events"])]);

    let config = test_config();
    let selection = graph
        .select_nodes(&[name_selector("loader", false, false)], &config)
        .unwrap();
    assert_eq!(selection.steps, HashSet::from(["loader".to_string()]));
    assert!(selection.models.is_empty());
}

/// Given an arbitrary selected model set (what the run path holds, not a
/// selector), the required step set is exactly the producers of sources
/// those models read.
#[test]
fn steps_required_by_selection() {
    let bronze = model(
        "bronze",
        &["bronze"],
        vec![source_ref(&["sources", "raw", "events"])],
    );
    let other = model("other", &["other"], vec![]);
    let mut graph = DependencyGraph::build(vec![bronze, other], None).unwrap();
    graph.add_external_steps(&[step(&["loader"], &["smelt.sources.raw.events"])]);

    let selected: HashSet<String> = ["bronze".to_string(), "other".to_string()]
        .into_iter()
        .collect();
    assert_eq!(graph.steps_required_by(&selected), vec!["loader"]);

    let just_other: HashSet<String> = ["other".to_string()].into_iter().collect();
    assert!(graph.steps_required_by(&just_other).is_empty());
}

/// The existing model-only API is byte-identical with and without steps
/// registered (no caller breakage).
#[test]
fn select_models_unchanged_when_steps_registered() {
    let bronze = model(
        "bronze",
        &["bronze"],
        vec![source_ref(&["sources", "raw", "events"])],
    );
    let silver = model("silver", &["silver"], vec![source_ref(&["bronze"])]);
    let config = test_config();

    let graph_without = DependencyGraph::build(vec![bronze.clone(), silver.clone()], None).unwrap();
    let without = graph_without
        .select_models(&[name_selector("silver", true, false)], &config)
        .unwrap();

    let mut graph_with = DependencyGraph::build(vec![bronze, silver], None).unwrap();
    graph_with.add_external_steps(&[step(&["loader"], &["smelt.sources.raw.events"])]);
    let with = graph_with
        .select_models(&[name_selector("silver", true, false)], &config)
        .unwrap();

    assert_eq!(without, with);
}
