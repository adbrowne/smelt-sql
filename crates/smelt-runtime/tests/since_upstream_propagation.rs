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
use smelt_runtime::propagation::{
    build_forward_graph, plan_since_upstream, resolve_build_plan, SourceDelta,
};

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

/// Phase B1 (`docs/plans/20260715-composed-axes-conditional-maintenance.md`):
/// a **locality-admitted composed node** (`grain: key` + `timeseries:`,
/// admitted key temporal locality) sits *inside* a real propagation chain
/// instead of terminating it (`incremental_models.md` §"The graph layer": "A
/// locality-admitted time-partitioned keyed output is not refused").
///
/// This phase's own flagship fixture (`examples/web_analytics/models/
/// silver/events_deduped.sql`) is not yet buildable — its
/// extremal-fold (`MIN`/`MAX`) `timeseries.partition_column` hits a
/// pre-existing, out-of-scope NOT-NULL inference gap (`docs/plans/
/// 20260715-composed-axes-conditional-maintenance.md` §"Blocked phases",
/// 2026-07-18, W1). `examples/timeseries` already carries an equivalent,
/// already-landed composed shape wired for exactly this scenario
/// (`user_daily_spend.sql`'s own doc comment cites this plan's Phase A5):
/// `raw.transactions -> user_daily_spend` (`grain: key` + `timeseries:`,
/// route 1 key-embedded) `-> user_spend_rollup` (`grain: partition`).
#[test]
fn composed_node_in_the_chain() {
    let project_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/timeseries")
        .canonicalize()
        .expect("examples/timeseries exists");

    let discovery = ModelDiscovery::new(project_dir.clone(), vec!["models".to_string()]);
    let models = discovery.discover_models().expect("discover models");
    let source_infos = discover_source_infos(&project_dir, &["models".to_string()]);

    let edges = build_forward_graph(&models, &source_infos).expect(
        "the graph must build without MaintenanceGraphUnsupportedNode: user_daily_spend is a \
         locality-admitted composed node, not a bare keyed refusal",
    );

    // Inbound: the composed node's own driving-source edge. Phase B2
    // ("Key→partition dirt projection through composed nodes") derives this
    // margin for real from the model's admitted `KeyLocality` verdict
    // (`locality_margin_days`) rather than a placeholder — it genuinely IS
    // `(0, 0)` here because `user_daily_spend`'s SQL reads no lookback
    // window against its driving source (route 1 key-embedded, zero
    // derived read margin; see the model's own doc comment).
    let inbound = edges
        .iter()
        .find(|e| e.upstream == "raw.transactions" && e.downstream == "user_daily_spend")
        .unwrap_or_else(|| panic!("expected an inbound edge into the composed node: {edges:?}"));
    assert_eq!(
        inbound.downstream_grain,
        smelt_logical::maintenance::propagate::PartitionGrain::Day,
        "the composed node classifies by its declared granularity, not PartitionGrain::Keyed"
    );
    assert_eq!(
        (inbound.before_days, inbound.after_days),
        (0, 0),
        "user_daily_spend's real derived margin is genuinely zero: {inbound:?}"
    );

    // Outbound: the composed node feeds an ordinary downstream consumer.
    let outbound = edges
        .iter()
        .find(|e| e.upstream == "user_daily_spend" && e.downstream == "user_spend_rollup")
        .unwrap_or_else(|| panic!("expected an outbound edge from the composed node: {edges:?}"));
    assert_eq!(
        outbound.upstream_grain,
        smelt_logical::maintenance::propagate::PartitionGrain::Day,
        "the composed node's own outbound edge must not be PartitionGrain::Keyed"
    );

    // The whole graph — including the bare Day-grain neighbours — still
    // propagates end to end through the composed node without refusing.
    let order: Vec<String> = models
        .iter()
        .map(|m| m.canonical_path())
        .filter(|a| a == "user_daily_spend" || a == "user_spend_rollup")
        .collect();
    let deltas = vec![SourceDelta {
        source: "raw.transactions".to_string(),
        landed: smelt_logical::maintenance::propagate::DayInterval::new(20, 21),
    }];
    plan_since_upstream(&models, &source_infos, &order, &deltas)
        .expect("a delta on the composed node's driving source must propagate through it");
}

/// Phase B2: a locality-admitted composed node whose own SQL reads a
/// **nonzero** lookback window against its driving source (route 1
/// key-embedded, mirroring `examples/timeseries`'s own `user_spend_rollup.sql`
/// pushdown-margin construct — a `WHERE col >= CAST(col AS DATE) - INTERVAL
/// '…days'` Form B relation — but here applied directly on the composed
/// model's own driving-source read) must derive a genuinely nonzero
/// `before_days` on its inbound edge, proving `locality_margin_days` pulls a
/// real margin through end to end rather than only ever seeing the
/// (coincidentally zero) `user_daily_spend` case.
#[test]
fn composed_node_with_a_lookback_window_derives_a_nonzero_inbound_margin() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path();
    smelt_yml(root);
    write(
        root,
        "models/sources/bronze.yml",
        "description: bronze\ncolumns:\n- name: user_id\n  type: INTEGER\n\
         - name: amount\n  type: DECIMAL(10,2)\n- name: ts\n  type: TIMESTAMP\n\
         mutation_profile:\n  kind: append_only\n\
         timeseries:\n  partition_column: ts\n  event_time_column: ts\n  granularity: day\n",
    );
    write(
        root,
        "models/composed.sql",
        "---\nmaterialization: table\nrefresh: incremental\ngrain: key\n\
         timeseries:\n  partition_column: d\n  event_time_column: d\n  granularity: day\n---\n\
         SELECT\n    user_id,\n    CAST(ts AS DATE) AS d,\n    SUM(amount) AS total\n\
         FROM smelt.sources.bronze\n\
         WHERE ts >= CAST(ts AS DATE) - INTERVAL '3 days'\n\
         GROUP BY 1, 2\n",
    );

    let discovery = ModelDiscovery::new(root.to_path_buf(), vec!["models".to_string()]);
    let models = discovery.discover_models().expect("discover models");
    let source_infos = discover_source_infos(root, &["models".to_string()]);

    let edges = build_forward_graph(&models, &source_infos).expect(
        "the graph must build: composed's WHERE clause admits route 1 with a derived \
                 nonzero read margin",
    );
    let inbound = edges
        .iter()
        .find(|e| e.upstream == "bronze" && e.downstream == "composed")
        .unwrap_or_else(|| panic!("expected an inbound edge into composed: {edges:?}"));
    assert!(
        inbound.before_days > 0,
        "a 3-day lookback WHERE clause must derive a nonzero before_days margin: {inbound:?}"
    );
}

/// A **bare** keyed model (no `timeseries:` declared at all) that another
/// model reads still refuses fail-loud in the real per-workspace assembly
/// — `MaintenanceGraphUnsupportedNode`, naming the missing time axis — the
/// same safety property `keyed_grain_model_never_derives_an_edge` pins for
/// the model's own creation edge, but exercised here through the edge an
/// explicitly-mutable **unclocked** dimension source contributes (the one
/// real path that reaches a bare keyed node as a graph node today).
#[test]
fn bare_keyed_upstream_still_refuses() {
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
        "models/sources/dims.yml",
        "description: dims\ncolumns:\n- name: user_id\n  type: INTEGER\n\
         - name: region\n  type: VARCHAR\n\
         mutation_profile:\n  kind: mutable_snapshot\n",
    );
    write(
        root,
        "models/bare_keyed.sql",
        "---\nmaterialization: table\nrefresh: incremental\ngrain: key\n---\n\
         SELECT p.user_id, SUM(p.amount) AS total, d.region\n\
         FROM smelt.sources.payments p\n\
         JOIN smelt.sources.dims d ON p.user_id = d.user_id\n\
         GROUP BY p.user_id, d.region\n",
    );

    let discovery = ModelDiscovery::new(root.to_path_buf(), vec!["models".to_string()]);
    let models = discovery.discover_models().expect("discover models");
    let source_infos = discover_source_infos(root, &["models".to_string()]);

    let order = vec!["bare_keyed".to_string()];
    let err = plan_since_upstream(&models, &source_infos, &order, &[])
        .expect_err("a bare keyed node reached by the graph must still refuse");
    let msg = err.to_string();
    assert!(msg.contains("MaintenanceGraphUnsupportedNode"), "{msg}");
    assert!(msg.contains("without an admitted time axis"), "{msg}");
    assert!(msg.contains("bare_keyed"), "{msg}");
}

/// Phase B3 (`docs/plans/20260715-composed-axes-conditional-maintenance.md`):
/// a **locality-admitted composed node** itself — not its driving source —
/// is the `--source` delta origin. `examples/timeseries`'s already-landed
/// composed chain (`raw.transactions -> user_daily_spend [grain: key +
/// timeseries, route 1] -> user_spend_rollup`, the same fixture
/// `composed_node_in_the_chain` uses mid-chain) is reused here with the
/// delta seeded directly on `user_daily_spend`'s own declared output axis
/// (`spend_date`) instead of on `raw.transactions`: its landed window
/// reflects through the real (zero-margin) outbound edge to
/// `user_spend_rollup`, and the composed origin itself is never re-run —
/// exactly the same "origin is not re-run" contract
/// `model_delta_origin_propagates_to_downstreams_without_rerunning_origin`
/// pins for a bare `grain: partition` origin, now exercised for a composed
/// keyed+timeseries origin.
#[test]
fn composed_model_as_source() {
    let project_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/timeseries")
        .canonicalize()
        .expect("examples/timeseries exists");

    let discovery = ModelDiscovery::new(project_dir.clone(), vec!["models".to_string()]);
    let models = discovery.discover_models().expect("discover models");
    let source_infos = discover_source_infos(&project_dir, &["models".to_string()]);

    let order: Vec<String> = models
        .iter()
        .map(|m| m.canonical_path())
        .filter(|a| a == "user_daily_spend" || a == "user_spend_rollup")
        .collect();
    let deltas = vec![SourceDelta {
        source: "user_daily_spend".to_string(),
        landed: smelt_logical::maintenance::propagate::DayInterval::new(20, 21),
    }];
    let plan = plan_since_upstream(&models, &source_infos, &order, &deltas).expect(
        "a landed delta declared directly on a locality-admitted composed model must \
         propagate through its outbound edge",
    );

    assert!(
        plan.runs.iter().any(|r| r.model == "user_spend_rollup"),
        "user_daily_spend's landed delta must propagate to user_spend_rollup: {:?}",
        plan.runs
    );
    assert!(
        !plan.runs.iter().any(|r| r.model == "user_daily_spend"),
        "the composed origin itself must NOT be re-run — its landed delta is already \
         written: {:?}",
        plan.runs
    );
    assert!(
        plan.dirty_set_report
            .contains("user_spend_rollup <- user_daily_spend"),
        "the dirty set must show the composed model edge: {}",
        plan.dirty_set_report
    );
    assert!(
        !plan.dirty_set_report.contains("RUN user_daily_spend"),
        "the composed origin must not appear as a scheduled run: {}",
        plan.dirty_set_report
    );
}

/// Phase B3: `resolve_build_plan` (the `smelt build --include-upstreams`
/// backward-resolution assembly) walks *through* a locality-admitted
/// composed ancestor exactly like any other clocked model — the composed
/// node appears in the build order (built before its consumer,
/// `user_spend_rollup`, and after its own upstream source), and its
/// required slice is a real bounded region, not a whole-table fallback.
/// Shares the graph layer's "one edge object, both directions" law with
/// [`composed_node_in_the_chain`] (forward) and B2's pure-math
/// `required_inputs_resolves_route_aware_through_a_composed_edge`
/// (`crates/smelt-logical/tests/maintenance_tracer_propagation.rs`) — this
/// test is the real per-workspace assembly's own leg of that same coverage.
#[test]
fn resolve_build_plan_walks_through_a_composed_ancestor() {
    let project_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/timeseries")
        .canonicalize()
        .expect("examples/timeseries exists");

    let discovery = ModelDiscovery::new(project_dir.clone(), vec!["models".to_string()]);
    let models = discovery.discover_models().expect("discover models");
    let source_infos = discover_source_infos(&project_dir, &["models".to_string()]);

    let period = smelt_logical::maintenance::propagate::DayInterval::new(20, 21);
    let resolved = resolve_build_plan(&models, &source_infos, "user_spend_rollup", period)
        .expect("backward resolution must walk through the composed ancestor without refusing");

    let names: Vec<&str> = resolved
        .build_order
        .iter()
        .map(|r| r.model.as_str())
        .collect();
    assert!(
        names.contains(&"user_daily_spend"),
        "the composed ancestor must be a build step, not silently skipped: {names:?}"
    );
    assert!(
        names.contains(&"user_spend_rollup"),
        "the target itself must be a build step: {names:?}"
    );
    let composed_pos = names.iter().position(|n| *n == "user_daily_spend").unwrap();
    let target_pos = names
        .iter()
        .position(|n| *n == "user_spend_rollup")
        .unwrap();
    assert!(
        composed_pos < target_pos,
        "the composed ancestor must build before its consumer: {names:?}"
    );
    assert!(
        resolved.report.contains("BUILD user_daily_spend"),
        "the report must show the composed ancestor as a build step: {}",
        resolved.report
    );
}
