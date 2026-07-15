//! Phase MP15 (`docs/plans/20260707-maintenance-plan-impl.md`): promoting
//! the forward-propagation graph from the tracer's hand-typed `Edge` lists
//! (`crates/smelt-logical/tests/maintenance_tracer_propagation.rs`, which
//! stays green as the regression floor for the pure composition math) to a
//! real per-workspace assembly: `smelt_runtime::propagation::
//! build_forward_graph` derives the `Edge` list from each model's own
//! `MaintenancePlan` scan clamps, exactly like the maintenance SQL itself is
//! sized, over real fixture models on disk — never a hand-typed clamp
//! number.

use std::path::Path;

use smelt_core::{discover_source_infos, ModelDiscovery};
use smelt_runtime::propagation::{build_forward_graph, plan_since_upstream, SourceDelta};

fn write(dir: &Path, rel: &str, content: &str) {
    let path = dir.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

fn smelt_yml(dir: &Path) {
    write(
        dir,
        "smelt.yml",
        "name: since_upstream_ws\nversion: 1\npaths:\n  - models\n\
         targets:\n  dev:\n    type: duckdb\n    schema: main\n\
         default_materialization: view\n",
    );
}

/// A single driving source (clocked, append-only) feeding one incremental
/// model — the same shape `derived_conversions_clamp_drives_the_propagation`
/// (the tracer suite) already promotes for a single model; this proves the
/// SAME real-derivation path composes across a real on-disk workspace
/// assembled by `build_forward_graph`.
#[test]
fn single_clocked_source_derives_a_real_edge_and_propagates() {
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
    assert_eq!(
        edges.len(),
        1,
        "expected exactly one derived edge: {edges:?}"
    );
    assert_eq!(edges[0].upstream, "bronze");
    assert_eq!(edges[0].downstream, "silver");

    let order = vec!["silver".to_string()];
    let deltas = vec![SourceDelta {
        source: "bronze".to_string(),
        landed: smelt_logical::maintenance::propagate::DayInterval::new(20, 21),
    }];
    let plan = plan_since_upstream(&models, &source_infos, &order, &deltas).expect("plan");
    assert_eq!(plan.runs.len(), 1);
    assert_eq!(plan.runs[0].model, "silver");
    assert!(plan.dirty_set_report.contains("silver <- bronze"));
}

/// A source nothing in the workspace declares a delta for contributes no
/// dirt — no implicit whole-table or recorded-state fallback
/// (`incremental_models.md` §CLI: "a source named without a matching
/// `--landed` delta propagates nothing for that invocation").
#[test]
fn source_without_a_delta_propagates_nothing() {
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

    let order = vec!["silver".to_string()];
    let plan = plan_since_upstream(&models, &source_infos, &order, &[]).expect("plan");
    assert!(plan.runs.is_empty(), "no delta => nothing propagated");
}

/// A maintained model is a delta origin of the same standing as a source
/// (`incremental_models.md` §"Upstream model edges"): a `--source
/// <model-address>` landed delta propagates to that model's downstreams,
/// and the origin model itself is **not** re-run (its landed delta is the
/// window a completed run already wrote for it). The `silver -> gold` edge
/// is derived through the SAME edge-aware derivation `smelt explain` uses
/// (a maintained-model creation cell), so the propagation clamp equals the
/// creation cell's clamp — here a zero-margin passthrough.
#[test]
fn model_delta_origin_propagates_to_downstreams_without_rerunning_origin() {
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
    write(
        root,
        "models/gold.sql",
        "---\nmaterialization: table\nrefresh: incremental\ngrain: partition\n\
         timeseries:\n  partition_column: d\n  event_time_column: d\n  granularity: day\n---\n\
         SELECT id, d FROM smelt.silver\n",
    );

    let discovery = ModelDiscovery::new(root.to_path_buf(), vec!["models".to_string()]);
    let models = discovery.discover_models().expect("discover models");
    let source_infos = discover_source_infos(root, &["models".to_string()]);

    // The model->model edge is present and derived (not a hand-typed clamp):
    // a passthrough read is a zero-margin same-axis edge.
    let edges = build_forward_graph(&models, &source_infos).expect("build graph");
    let edge = edges
        .iter()
        .find(|e| e.upstream == "silver" && e.downstream == "gold")
        .unwrap_or_else(|| panic!("expected a silver -> gold model edge: {edges:?}"));
    assert_eq!(
        edge.before_days, 0,
        "passthrough read is zero-margin: {edge:?}"
    );
    assert_eq!(
        edge.after_days, 0,
        "passthrough read is zero-margin: {edge:?}"
    );

    let order = vec!["silver".to_string(), "gold".to_string()];
    let deltas = vec![SourceDelta {
        source: "silver".to_string(),
        landed: smelt_logical::maintenance::propagate::DayInterval::new(20, 21),
    }];
    let plan = plan_since_upstream(&models, &source_infos, &order, &deltas).expect("plan");

    assert!(
        plan.runs.iter().any(|r| r.model == "gold"),
        "silver's landed delta must propagate to gold: {:?}",
        plan.runs
    );
    assert!(
        !plan.runs.iter().any(|r| r.model == "silver"),
        "the delta origin model must NOT be re-run — its landed delta is already written: {:?}",
        plan.runs
    );
    assert!(
        plan.dirty_set_report.contains("gold <- silver"),
        "the dirty set must show the model edge: {}",
        plan.dirty_set_report
    );
    assert!(
        !plan.dirty_set_report.contains("RUN silver"),
        "the origin model must not appear as a scheduled run: {}",
        plan.dirty_set_report
    );
}

/// A self-referential model (a ref to its own address) refuses fail-loud
/// before any interval math runs — `MaintenanceGraphUnsupportedNode`
/// (`incremental_models.md` §"The graph layer" — refusals).
#[test]
fn self_referential_model_refuses() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path();
    smelt_yml(root);
    write(
        root,
        "models/rolling.sql",
        "---\nmaterialization: table\nrefresh: incremental\ngrain: partition\n\
         timeseries:\n  partition_column: d\n  event_time_column: d\n  granularity: day\n---\n\
         SELECT id, d FROM smelt.rolling\n",
    );

    let discovery = ModelDiscovery::new(root.to_path_buf(), vec!["models".to_string()]);
    let models = discovery.discover_models().expect("discover models");
    let source_infos = discover_source_infos(root, &["models".to_string()]);

    let err = build_forward_graph(&models, &source_infos).expect_err("self-ref must refuse");
    assert!(err.to_string().contains("MaintenanceGraphUnsupportedNode"));
    assert!(err.to_string().contains("self-referential"));
}

/// A keyed-grain model's `PlanCell` never carries a `ScanClamp` (a keyed
/// end-state has no partition axis to bound — `derive_new_data`'s
/// `Grain::Key` arm always derives `scans: vec![]`), so `build_forward_graph`
/// structurally never routes interval dirt through a keyed node: this is
/// itself the honest safety property the graph layer's refusal exists to
/// guarantee. The pure `propagate`/`required_inputs` refusal
/// (`MaintenanceGraphUnsupportedNode`, for the hypothetical case a keyed
/// node's grain reaches the graph through some other path) is exhaustively
/// covered at the pure-math level by
/// `crates/smelt-logical/tests/maintenance_tracer_propagation.rs::
/// s12_keyed_node_refuses_in_both_directions` — this test only pins that the
/// real per-workspace assembly never manufactures such an edge in the first
/// place.
#[test]
fn keyed_grain_model_never_derives_an_edge() {
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
        "models/lifetime_spend.sql",
        "---\nmaterialization: table\nrefresh: incremental\ngrain: key\n---\n\
         SELECT user_id, SUM(amount) AS total FROM smelt.sources.payments GROUP BY user_id\n",
    );

    let discovery = ModelDiscovery::new(root.to_path_buf(), vec!["models".to_string()]);
    let models = discovery.discover_models().expect("discover models");
    let source_infos = discover_source_infos(root, &["models".to_string()]);

    let graph = build_forward_graph(&models, &source_infos).expect("build graph");
    assert!(
        !graph.iter().any(|e| e.downstream == "lifetime_spend"),
        "a keyed-grain model must never derive a propagation edge: {graph:?}"
    );
}

/// A `grain: key_per_partition` model has a genuine partition axis (it
/// requires `timeseries:`, unlike `grain: key`) — when a downstream
/// `grain: partition` model reads it, `model_grain` must route it through
/// `granularity_grain(ts.granularity)`, the SAME path as `grain: partition`,
/// not to `PartitionGrain::Keyed`. Before the fix, `model_grain` grouped
/// `KeyPerPartition` together with `Key`, so the upstream node was
/// classified `Keyed` and `propagate`'s `refuse_keyed_nodes` fail-loud
/// refused the whole `--since-upstream` plan even though the node has a real
/// day axis. This pins that a `key_per_partition` node participates in
/// propagation by its declared granularity instead.
#[test]
fn key_per_partition_upstream_propagates_by_granularity_not_refused_as_keyed() {
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
        "models/trajectory.sql",
        "---\nmaterialization: table\nrefresh: incremental\ngrain: key_per_partition\n\
         timeseries:\n  partition_column: d\n  event_time_column: d\n  granularity: day\n---\n\
         SELECT user_id, d, amount FROM smelt.sources.payments\n",
    );
    write(
        root,
        "models/daily_totals.sql",
        "---\nmaterialization: table\nrefresh: incremental\ngrain: partition\n\
         timeseries:\n  partition_column: d\n  event_time_column: d\n  granularity: day\n---\n\
         SELECT user_id, d, amount FROM smelt.trajectory\n",
    );

    let discovery = ModelDiscovery::new(root.to_path_buf(), vec!["models".to_string()]);
    let models = discovery.discover_models().expect("discover models");
    let source_infos = discover_source_infos(root, &["models".to_string()]);

    let edges = build_forward_graph(&models, &source_infos).expect("build graph");
    let edge = edges
        .iter()
        .find(|e| e.upstream == "trajectory" && e.downstream == "daily_totals")
        .unwrap_or_else(|| panic!("expected trajectory -> daily_totals edge: {edges:?}"));
    assert_eq!(
        edge.upstream_grain,
        smelt_logical::maintenance::propagate::PartitionGrain::Day,
        "a key_per_partition node has a genuine day axis, not PartitionGrain::Keyed"
    );

    let order = vec!["trajectory".to_string(), "daily_totals".to_string()];
    let deltas = vec![SourceDelta {
        source: "trajectory".to_string(),
        landed: smelt_logical::maintenance::propagate::DayInterval::new(20, 21),
    }];
    let plan = plan_since_upstream(&models, &source_infos, &order, &deltas)
        .expect("key_per_partition upstream must not be refused as a keyed node");
    assert!(
        plan.runs.iter().any(|r| r.model == "daily_totals"),
        "expected daily_totals to be propagated from the trajectory delta: {:?}",
        plan.runs
    );
}
