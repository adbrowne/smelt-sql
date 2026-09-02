//! Phase MP15 (`docs/plans/20260707-maintenance-plan-impl.md`): promoting
//! the forward-propagation graph from the tracer's hand-typed `Edge` lists
//! (`crates/smelt-logical/tests/maintenance_tracer_propagation.rs`, which
//! stays green as the regression floor for the pure composition math) to a
//! real per-workspace assembly: `smelt_runtime::propagation::
//! build_forward_graph` derives the `Edge` list from each model's own
//! `MaintenancePlan` scan clamps, exactly like the maintenance SQL itself is
//! sized, over real fixture models on disk — never a hand-typed clamp
//! number.

use std::collections::BTreeSet;
use std::path::Path;

use arrow::array::RecordBatch;
use arrow::datatypes::SchemaRef;
use async_trait::async_trait;

use smelt_backend::{Backend, BackendCapabilities, BackendError, PartitionRange, SqlDialect};
use smelt_core::{discover_source_infos, ModelDiscovery};
use smelt_runtime::propagation::{
    build_forward_graph, load_observed_delta_lookup, plan_since_upstream,
    plan_since_upstream_with_observed_deltas, resolve_build_plan, ObservedDeltaLookup, SourceDelta,
};
use smelt_state::ddl_duckdb::ObservedDelta;

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

/// A self-referential model whose self-read carries no backward margin (a
/// same-partition self-read — reads exactly the window it is itself
/// writing, not strictly time-backward) still refuses fail-loud —
/// `MaintenanceGraphUnsupportedNode` — but the refusal now happens at
/// `propagate`/`required_inputs` time (`self_edges`'s `before_days <= 0`
/// gate), not at `build_forward_graph` — a **provably backward-bounded**
/// self-edge (`before_days > 0`) is a real day-unrolled edge
/// (`incremental_models.md` §"Time-unrolled self-edges"), so
/// `build_forward_graph` itself no longer refuses every self-reference on
/// sight; it defers the strictly-time-backward check to the shared
/// [`smelt_logical::maintenance::propagate::propagate`]/`required_inputs`
/// gate both directions share.
#[test]
fn same_partition_self_referential_model_refuses() {
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

    let edges = build_forward_graph(&models, &source_infos)
        .expect("build_forward_graph itself only refuses a non-time-backward self-edge later");
    assert!(
        edges
            .iter()
            .any(|e| e.upstream == "rolling" && e.downstream == "rolling"),
        "expected a self-edge to have been assembled: {edges:?}"
    );

    let deltas = vec![SourceDelta {
        source: "rolling".to_string(),
        landed: smelt_logical::maintenance::propagate::DayInterval::new(10, 11),
    }];
    let err = plan_since_upstream(&models, &source_infos, &["rolling".to_string()], &deltas)
        .expect_err("a same-partition (before_days == 0) self-edge must still refuse");
    assert!(err.to_string().contains("MaintenanceGraphUnsupportedNode"));
    assert!(err.to_string().contains("rolling"));
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
        "---\nmaterialization: table\nrefresh: incremental\nunique_key: [user_id, d]\n\
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

/// Build a throwaway `smelt_db::Database`/`Workspace` over `project_dir` and
/// return the `SourceFile` handle for `models/<model_name>.sql`, so a test
/// can call `smelt_db::maintenance_plan_report` — the SAME non-Salsa
/// derivation `smelt explain` reads — exactly like production does. Mirrors
/// `smelt-maintenance-testkit::verdict::build_db_and_target`'s Salsa setup
/// (out of this phase's critical files, so inlined here rather than taken as
/// a new dev-dependency).
fn build_db_and_target(
    project_dir: &Path,
    model_name: &str,
) -> (
    smelt_db::Database,
    smelt_db::Workspace,
    smelt_db::SourceFile,
) {
    let discovery = ModelDiscovery::new(project_dir.to_path_buf(), vec!["models".to_string()]);
    let sql_models = discovery.discover_models().expect("discover models");
    let target_path = project_dir.join(format!("models/{model_name}.sql"));

    let mut db = smelt_db::Database::default();
    let project = db.set_project_input(project_dir.to_path_buf(), String::new());
    let mut target: Option<smelt_db::SourceFile> = None;
    let source_files: Vec<_> = sql_models
        .iter()
        .map(|m| {
            let file =
                db.set_source_file(m.path.clone(), m.content.clone(), project_dir.to_path_buf());
            if m.path == target_path {
                target = Some(file);
            }
            file
        })
        .collect();
    db.set_workspace(source_files, vec![project]);
    let workspace = db.workspace();
    let target = target.unwrap_or_else(|| {
        panic!(
            "staged model {model_name:?} (expected at {}) not found among discovered models",
            target_path.display()
        )
    });
    (db, workspace, target)
}

/// Phase 6 (`docs/plans/20260719-prod-w8-composed-axes-followups.md`): the
/// recursive composed-driving-source case — a `grain: key` + `timeseries:`
/// model whose driving ref is *another maintained model's own* locality-
/// admitted composed output, not a declared `sources.*` entry.
/// `examples/timeseries` already carries this shape:
/// `raw.transactions -> user_daily_spend [grain: key, route 1] ->
/// user_spend_running_total [grain: key, route 1]` — `user_spend_running_total`
/// reads `smelt.user_daily_spend` (itself a composed node) as its own
/// driving source. Before this phase, `build_forward_graph`'s driving-source
/// granularity candidate set only ever looked at declared `sources.*` refs
/// (see the now-removed doc comment at the old call site), so
/// `user_spend_running_total` could never resolve a `driving_source_granularity`
/// and its own key-temporal-locality gate refused — it fell back to a bare
/// `PartitionGrain::Keyed` node in the graph even though `smelt explain`
/// (via `smelt_db::maintenance_plan_report`, which already threads a
/// model's own composed-output candidates via `model_source_granularities`)
/// admits it. This test asserts parity between the two: `smelt explain`'s
/// verdict for `user_spend_running_total` is "locality admitted, Day
/// granularity", and `build_forward_graph` must construct the
/// `user_daily_spend -> user_spend_running_total` edge at that SAME declared
/// granularity — never re-deriving the admission itself.
#[test]
fn recursive_composed_driving_source_reaches_the_same_verdict_as_smelt_explain() {
    let project_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/timeseries")
        .canonicalize()
        .expect("examples/timeseries exists");

    // The `smelt explain` verdict: the real Salsa-backed derivation
    // `maintenance_plan_report` calls, never re-derived by this test.
    let (db, workspace, target) = build_db_and_target(&project_dir, "user_spend_running_total");
    let report = smelt_db::maintenance_plan_report(&db, workspace, target).unwrap_or_else(|| {
        panic!(
            "user_spend_running_total must derive a maintenance plan (refresh: incremental, \
             grain: key)"
        )
    });
    let key_locality = report.plan.key_locality.as_ref().unwrap_or_else(|| {
        panic!(
            "smelt explain must admit key temporal locality for user_spend_running_total via \
             route 1 (key-embedded) over its composed driving source user_daily_spend: {:?}",
            report.plan
        )
    });
    assert!(
        matches!(
            key_locality.slice,
            smelt_logical::maintenance::locality::LocalitySlice::Window { .. }
        ),
        "user_spend_running_total's own (user_id, spend_date) GROUP BY admits route 1 \
         (key-embedded, exact projection — LocalitySlice::Window), not a recurrence-bounded \
         route: {:?}",
        key_locality.slice
    );

    // The graph-layer verdict: `build_forward_graph` must construct the
    // SAME edge, at the SAME (Day) granularity — not refuse the node as a
    // bare `PartitionGrain::Keyed` hop.
    let discovery = ModelDiscovery::new(project_dir.clone(), vec!["models".to_string()]);
    let models = discovery.discover_models().expect("discover models");
    let source_infos = discover_source_infos(&project_dir, &["models".to_string()]);
    let edges = build_forward_graph(&models, &source_infos).expect(
        "the graph must build without MaintenanceGraphUnsupportedNode: user_spend_running_total \
         is a locality-admitted composed node driven by another composed node's output, not a \
         bare keyed refusal",
    );

    let inbound = edges
        .iter()
        .find(|e| e.upstream == "user_daily_spend" && e.downstream == "user_spend_running_total")
        .unwrap_or_else(|| {
            panic!(
                "expected an edge from the composed driving source user_daily_spend into \
                 user_spend_running_total: {edges:?}"
            )
        });
    assert_eq!(
        inbound.downstream_grain,
        smelt_logical::maintenance::propagate::PartitionGrain::Day,
        "user_spend_running_total must classify by its OWN declared Day granularity — the same \
         granularity smelt explain's admitted verdict carries — never PartitionGrain::Keyed: \
         {inbound:?}"
    );

    // The whole graph propagates a delta through both composed hops without
    // refusing.
    let order: Vec<String> = models
        .iter()
        .map(|m| m.canonical_path())
        .filter(|a| a == "user_daily_spend" || a == "user_spend_running_total")
        .collect();
    let deltas = vec![SourceDelta {
        source: "raw.transactions".to_string(),
        landed: smelt_logical::maintenance::propagate::DayInterval::new(20, 21),
    }];
    plan_since_upstream(&models, &source_infos, &order, &deltas).expect(
        "a delta on raw.transactions must propagate through both composed hops \
         (user_daily_spend then user_spend_running_total)",
    );
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
    let err = plan_since_upstream(&models, &source_infos, &order, &[]).expect_err(
        "a bare keyed node whose inbound edge reads a mutable-snapshot source must \
                     still refuse — its own GROUP BY key includes a mutable-snapshot column \
                     ('region'), so the derived shape degrades to General rather than admitting \
                     a Keyed component (phase 5, docs/outcomes/20260809-output-delta-typing)",
    );
    let msg = err.to_string();
    assert!(msg.contains("MaintenanceGraphUnsupportedNode"), "{msg}");
    assert!(msg.contains("bare_keyed"), "{msg}");
    assert!(
        msg.contains("degraded to general"),
        "the refusal must name the General degrade, not the old bare-keyed wording: {msg}"
    );
    assert!(
        msg.contains("dims"),
        "the refusal must name the degrading source ('dims' is a mutable snapshot): {msg}"
    );
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

/// Phase D3 (`docs/plans/20260715-composed-axes-conditional-maintenance.md`):
/// a composed model edge's recorded observed delta narrows forward
/// propagation to exactly the touched partitions (widened only by the
/// downstream edge's own real derived margin — `user_spend_rollup`'s
/// 3-day pushdown lookback, unrelated to D3's own projection) instead of
/// the whole declared `--landed` window. `examples/timeseries`'s
/// `user_daily_spend` (route 1, key-embedded — `LocalitySlice::Window`
/// with zero widening) is reused, matching `composed_model_as_source`'s
/// own fixture and substitution rationale.
#[test]
fn observed_delta_narrows_composed_edge_to_recorded_partitions() {
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

    // A wide declared window (20 days) — the pre-D3 behaviour would dirty
    // `user_spend_rollup` over this entire window (mod the edge's own
    // margin). The recorded observed delta only touched 3 far-apart
    // partitions within it.
    let window = smelt_logical::maintenance::propagate::DayInterval::new(20, 40);
    let deltas = vec![SourceDelta {
        source: "user_daily_spend".to_string(),
        landed: window,
    }];

    let mut observed: ObservedDeltaLookup = ObservedDeltaLookup::new();
    observed.insert(
        (
            "user_daily_spend".to_string(),
            smelt_logical::maintenance::propagate::ordinal_to_iso(window.start),
            smelt_logical::maintenance::propagate::ordinal_to_iso(window.end),
        ),
        ObservedDelta {
            changed_keys: vec!["u1".to_string(), "u2".to_string(), "u3".to_string()],
            partitions: vec![
                smelt_logical::maintenance::propagate::ordinal_to_iso(21),
                smelt_logical::maintenance::propagate::ordinal_to_iso(30),
                smelt_logical::maintenance::propagate::ordinal_to_iso(39),
            ],
        },
    );

    let plan = plan_since_upstream_with_observed_deltas(
        &models,
        &source_infos,
        &order,
        &deltas,
        &observed,
        "2026-01-01",
    )
    .expect("an observed delta on a composed origin must project and propagate");

    // `user_spend_rollup`'s own inbound edge derives a real 3-day backward
    // pushdown margin (its WHERE clause's `INTERVAL '3 days'` lookback) —
    // each of the 3 exact observed partitions widens by that margin. The
    // three widened regions ([18,22), [27,31), [36,40)) stay disjoint
    // (spaced far enough apart) and are collectively much narrower than
    // the declared 20-day window.
    let rollup_runs: Vec<_> = plan
        .runs
        .iter()
        .filter(|r| r.model == "user_spend_rollup")
        .collect();
    assert!(
        !rollup_runs.is_empty(),
        "the observed delta must still propagate: {:?}",
        plan.runs
    );
    let total_days: i64 = rollup_runs
        .iter()
        .map(|r| {
            let start = r.start.as_deref().expect("bounded run");
            let end = r.end.as_deref().expect("bounded run");
            let s = chrono::NaiveDate::parse_from_str(start, "%Y-%m-%d").unwrap();
            let e = chrono::NaiveDate::parse_from_str(end, "%Y-%m-%d").unwrap();
            (e - s).num_days()
        })
        .sum();
    assert!(
        total_days < 20,
        "observed-delta projection must be narrower than the full 20-day declared window: \
         got {total_days} total dirtied days across {rollup_runs:?}"
    );
    // Every one of the 3 observed partitions must still be covered
    // (widen-never-narrow) — spot-check the middle one.
    let day_30_iso = smelt_logical::maintenance::propagate::ordinal_to_iso(30);
    assert!(
        rollup_runs
            .iter()
            .any(|r| r.start.as_deref().unwrap() <= day_30_iso.as_str()
                && day_30_iso.as_str() < r.end.as_deref().unwrap()),
        "the observed partition {day_30_iso} must be covered by some dirtied run: {rollup_runs:?}"
    );
}

/// D3: a **present-and-empty** recorded observed delta (a fully-suppressed
/// conditional write — nothing changed) propagates **nothing** downstream —
/// the graph half of the no-op cascade.
#[test]
fn empty_observed_delta_schedules_zero_downstream_regions() {
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

    let window = smelt_logical::maintenance::propagate::DayInterval::new(20, 21);
    let deltas = vec![SourceDelta {
        source: "user_daily_spend".to_string(),
        landed: window,
    }];

    let mut observed: ObservedDeltaLookup = ObservedDeltaLookup::new();
    observed.insert(
        (
            "user_daily_spend".to_string(),
            smelt_logical::maintenance::propagate::ordinal_to_iso(window.start),
            smelt_logical::maintenance::propagate::ordinal_to_iso(window.end),
        ),
        ObservedDelta {
            changed_keys: vec![],
            partitions: vec![],
        },
    );

    let plan = plan_since_upstream_with_observed_deltas(
        &models,
        &source_infos,
        &order,
        &deltas,
        &observed,
        "2026-01-01",
    )
    .expect("a present-and-empty observed delta must not be a refusal");

    assert!(
        plan.runs.is_empty(),
        "an empty recorded delta must schedule zero downstream regions: {:?}",
        plan.runs
    );
    assert!(
        !plan
            .dirty_set_report
            .contains("user_spend_rollup <- user_daily_spend"),
        "an empty recorded delta must not appear as dirt in the report: {}",
        plan.dirty_set_report
    );
}

/// D3: an **absent** observed-delta record (e.g. a run that predates
/// conditional-write recording) falls back to the declared `--landed`
/// window unchanged — the D1 widen-never-narrow rule. Same fixture and
/// deltas as `composed_model_as_source`, but driven through
/// `plan_since_upstream_with_observed_deltas` with an empty lookup, proving
/// the fallback path is identical to `plan_since_upstream`'s own behaviour.
#[test]
fn absent_observed_delta_falls_back_to_the_written_window() {
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

    let no_observed = ObservedDeltaLookup::new();
    let with_lookup = plan_since_upstream_with_observed_deltas(
        &models,
        &source_infos,
        &order,
        &deltas,
        &no_observed,
        "2026-01-01",
    )
    .expect("absent record must fall back, not error");
    let baseline = plan_since_upstream(&models, &source_infos, &order, &deltas)
        .expect("baseline plan_since_upstream");

    assert_eq!(
        with_lookup.runs, baseline.runs,
        "an absent observed-delta record must propagate identically to the no-observed-delta \
         baseline"
    );
    assert!(
        with_lookup
            .runs
            .iter()
            .any(|r| r.model == "user_spend_rollup"),
        "the declared window must still propagate downstream: {:?}",
        with_lookup.runs
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

// ---------------------------------------------------------------------------
// Phase D3 real-fixture e2e: `examples/web_analytics`'s flagship composed
// model, `silver.events_deduped` (route 3, declared recurrence bound) ->
// `silver.sessions`. Proves the D2 write-side (`smelt_state::ddl_duckdb`'s
// real DDL/DML, executed against a real DuckDB backend — the SAME warehouse
// round trip `crates/smelt-runtime/tests/observed_delta.rs` and
// `crates/smelt-backend-duckdb/src/lib.rs`'s own tests already prove for the
// write side) and this phase's read+project+propagate side compose
// end to end over the actual flagship fixture: a fully-suppressed
// `events_deduped` run (an empty recorded delta, exactly what a
// change-suppressed conditional write records when nothing differs —
// `docs/plans/20260715-composed-axes-conditional-maintenance.md` Phase D2)
// schedules zero downstream `silver.sessions` regions under
// `--since-upstream`.
#[tokio::test]
async fn web_analytics_events_deduped_fully_suppressed_schedules_no_downstream_sessions() {
    let project_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/web_analytics")
        .canonicalize()
        .expect("examples/web_analytics exists");

    let discovery = ModelDiscovery::new(project_dir.clone(), vec!["models".to_string()]);
    let models: Vec<_> = discovery.discover_models().expect("discover models");
    let source_infos = discover_source_infos(&project_dir, &["models".to_string()]);

    // `events_deduped` (route 3, declared recurrence bound via
    // `sources.raw.events`'s `key_recurrence`) must be a real, locality-
    // admitted graph node whose outbound edge reaches `silver.sessions` —
    // the same real per-workspace derivation `smelt explain` uses.
    let edges = build_forward_graph(&models, &source_infos).expect(
        "the real web_analytics graph must build: events_deduped is locality-admitted \
         (route 3, declared)",
    );
    assert!(
        edges
            .iter()
            .any(|e| e.upstream == "silver.events_deduped" && e.downstream == "silver.sessions"),
        "expected a real outbound edge silver.events_deduped -> silver.sessions: {edges:?}"
    );

    let order: Vec<String> = models
        .iter()
        .map(|m| m.canonical_path())
        .filter(|a| a == "silver.events_deduped" || a == "silver.sessions")
        .collect();

    let window = smelt_logical::maintenance::propagate::DayInterval::new(100, 101);
    let window_start = smelt_logical::maintenance::propagate::ordinal_to_iso(window.start);
    let window_end = smelt_logical::maintenance::propagate::ordinal_to_iso(window.end);

    // The real D2 write-side round trip: a real DuckDB backend, the real
    // `_smelt_observed_delta` DDL, and a real upsert recording a
    // fully-suppressed run (a change-suppressed conditional write whose
    // `IS DISTINCT FROM` guard matched zero rows — nothing to record but the
    // schema itself, `docs/specs/incremental_models.md` §"The graph layer":
    // "a fully-suppressed run's changed-keys query returns zero rows ...
    // still inserts a row — present-and-empty, not absent").
    let tmp = tempfile::TempDir::new().unwrap();
    let db_path = tmp.path().join("test.duckdb");
    let backend = smelt_backend_duckdb::DuckDbBackend::new(&db_path, "main")
        .await
        .expect("open duckdb backend");
    let ddl = smelt_state::ddl_duckdb::generate_observed_delta_table_ddl("main");
    backend.execute_sql(&ddl).await.expect("create delta table");
    let upsert = smelt_state::ddl_duckdb::generate_observed_delta_upsert_sql(
        "main",
        "silver.events_deduped",
        &window_start,
        &window_end,
        "SELECT NULL::VARCHAR AS delta_key, NULL::VARCHAR AS delta_partition WHERE FALSE",
    );
    backend
        .execute_sql(&upsert)
        .await
        .expect("record the fully-suppressed run's empty delta");

    // The real D3 read side: `generate_observed_delta_select_sql` against
    // the SAME real backend, decoded into an `ObservedDelta`.
    let select = smelt_state::ddl_duckdb::generate_observed_delta_select_sql(
        "main",
        "silver.events_deduped",
        &window_start,
        &window_end,
    );
    let batches = backend.execute_sql(&select).await.expect("read delta row");
    assert_eq!(
        batches.iter().map(|b| b.num_rows()).sum::<usize>(),
        1,
        "the fully-suppressed run's row must be present (empty, not absent)"
    );
    let observed_delta = decode_observed_delta_row(&batches);
    assert!(
        observed_delta.is_empty(),
        "a fully-suppressed run's recorded delta must be present-and-empty: {observed_delta:?}"
    );

    let mut observed: ObservedDeltaLookup = ObservedDeltaLookup::new();
    observed.insert(
        (
            "silver.events_deduped".to_string(),
            window_start,
            window_end,
        ),
        observed_delta,
    );

    let deltas = vec![SourceDelta {
        source: "silver.events_deduped".to_string(),
        landed: window,
    }];
    // Phase 16: the settle-bound × observed-delta composition's reporting
    // leg (`docs/specs/incremental_models.md` §"Observed deltas on model
    // edges"). `window.end` (day ordinal 101) is deep in 1970 — far behind
    // ANY real `now`, so it reports a settled no-op; `now` pinned to the
    // window's own end reports the same empty delta as merely unsettled.
    // Both legs must schedule the IDENTICAL (empty) run set — this is a
    // reporting distinction only, never extra pruning.
    let window_end_iso = smelt_logical::maintenance::propagate::ordinal_to_iso(window.end);
    let plan_settled = plan_since_upstream_with_observed_deltas(
        &models,
        &source_infos,
        &order,
        &deltas,
        &observed,
        "2026-01-01",
    )
    .expect("a present-and-empty observed delta must not be a refusal");
    let plan_unsettled = plan_since_upstream_with_observed_deltas(
        &models,
        &source_infos,
        &order,
        &deltas,
        &observed,
        &window_end_iso,
    )
    .expect("a present-and-empty observed delta must not be a refusal");

    for plan in [&plan_settled, &plan_unsettled] {
        assert!(
            !plan.runs.iter().any(|r| r.model == "silver.sessions"),
            "a fully-suppressed events_deduped run must schedule zero downstream \
             silver.sessions regions: {:?}",
            plan.runs
        );
        assert!(
            !plan
                .dirty_set_report
                .contains("silver.sessions <- silver.events_deduped"),
            "the dirty set must show no dirt on the events_deduped -> sessions edge: {}",
            plan.dirty_set_report
        );
    }
    assert_eq!(
        plan_settled.runs, plan_unsettled.runs,
        "the settled/unsettled distinction is reporting-only — the scheduled run set must be \
         identical either way"
    );
    assert!(
        plan_settled
            .dirty_set_report
            .contains("settled no-op (behind the settle bound)"),
        "a window far behind the settle bound must report a settled no-op: {}",
        plan_settled.dirty_set_report
    );
    assert!(
        plan_unsettled
            .dirty_set_report
            .contains("empty this run (not yet settled)"),
        "a window still within the settle bound must report merely empty-this-run: {}",
        plan_unsettled.dirty_set_report
    );
}

// ---------------------------------------------------------------------------
// Phase 22 (`docs/outcomes/20260815-definition-delta-migrate/phases/22-plan.md`):
// `examples/web_analytics`'s own `silver.sessions_chained` — a deliberately
// self-referential (root-anchored session cut) model — builds a real
// day-unrolled self-edge in the WHOLE unfiltered workspace graph, instead of
// refusing the whole-workspace graph as a table-graph cycle.
// ---------------------------------------------------------------------------

/// `build_forward_graph` over the full, unfiltered `examples/web_analytics`
/// model set succeeds and contains a self-edge for `silver.sessions_chained`
/// with `after_days == 0` and a positive derived backward reach.
#[test]
fn web_analytics_self_referential_model_builds_a_self_edge() {
    let project_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/web_analytics")
        .canonicalize()
        .expect("examples/web_analytics exists");

    let discovery = ModelDiscovery::new(project_dir.clone(), vec!["models".to_string()]);
    let models = discovery.discover_models().expect("discover models");
    let source_infos = discover_source_infos(&project_dir, &["models".to_string()]);

    let edges = build_forward_graph(&models, &source_infos).expect(
        "the whole unfiltered web_analytics graph must build: sessions_chained's self-edge is \
         a provably backward-bounded time-unrolled self-edge, not a table-graph cycle refusal",
    );

    let self_edge = edges
        .iter()
        .find(|e| {
            e.upstream == "silver.sessions_chained" && e.downstream == "silver.sessions_chained"
        })
        .expect("expected a self-edge for silver.sessions_chained");
    assert_eq!(
        self_edge.after_days, 0,
        "the self-edge must carry no forward reach: {self_edge:?}"
    );
    assert!(
        self_edge.before_days >= 2,
        "the self-edge's backward reach must be at least the model's own 2-day root-anchored \
         cutoff: {self_edge:?}"
    );
}

/// A delta on `silver.sessions_chained`'s own upstream (`silver.events_deduped`)
/// schedules an open-ended `PropagatedRun` for it (`start: Some(_), end:
/// None`), and the dirty-set report renders both the self-edge line and the
/// `[<date>, →)` open-ended form.
#[tokio::test]
async fn self_referential_model_schedules_an_open_ended_run() {
    let project_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/web_analytics")
        .canonicalize()
        .expect("examples/web_analytics exists");

    let discovery = ModelDiscovery::new(project_dir.clone(), vec!["models".to_string()]);
    let models = discovery.discover_models().expect("discover models");
    let source_infos = discover_source_infos(&project_dir, &["models".to_string()]);

    let order: Vec<String> = models
        .iter()
        .map(|m| m.canonical_path())
        .filter(|a| a == "silver.events_deduped" || a == "silver.sessions_chained")
        .collect();

    let window = smelt_logical::maintenance::propagate::DayInterval::new(100, 101);
    let deltas = vec![SourceDelta {
        source: "silver.events_deduped".to_string(),
        landed: window,
    }];

    let plan =
        plan_since_upstream(&models, &source_infos, &order, &deltas).expect("plan propagates");

    let chained_run = plan
        .runs
        .iter()
        .find(|r| r.model == "silver.sessions_chained")
        .expect("expected a scheduled run for silver.sessions_chained");
    assert!(
        chained_run.start.is_some(),
        "an open-ended run must still carry a finite start: {chained_run:?}"
    );
    assert!(
        chained_run.end.is_none(),
        "a self-referential model's own dirt must widen open-ended (no finite end): \
         {chained_run:?}"
    );
    assert!(
        plan.dirty_set_report
            .contains("silver.sessions_chained <-(self, unrolled) silver.sessions_chained"),
        "the dirty-set report must render the self-edge line: {}",
        plan.dirty_set_report
    );
    assert!(
        plan.dirty_set_report.contains(", →)"),
        "the dirty-set report must render the open-ended interval form: {}",
        plan.dirty_set_report
    );
}

/// Decode the single `(changed_keys, partitions)` row
/// `generate_observed_delta_select_sql` projects — the two `VARCHAR[]`
/// columns — into an [`ObservedDelta`]. Mirrors
/// `crates/smelt-runtime/tests/observed_delta.rs`'s own
/// `batches_to_string_lists` decode helper (this phase's read side is the
/// consumer of that exact same D2-written table).
fn decode_observed_delta_row(batches: &[arrow::array::RecordBatch]) -> ObservedDelta {
    use arrow::array::{Array, ListArray, StringArray};
    for batch in batches {
        if batch.num_rows() == 0 {
            continue;
        }
        let changed = batch
            .column(0)
            .as_any()
            .downcast_ref::<ListArray>()
            .expect("changed_keys is a LIST column");
        let partitions = batch
            .column(1)
            .as_any()
            .downcast_ref::<ListArray>()
            .expect("partitions is a LIST column");
        let changed_keys: Vec<String> = changed
            .value(0)
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("changed_keys elements are VARCHAR")
            .iter()
            .map(|v| v.unwrap_or_default().to_string())
            .collect();
        let parts: Vec<String> = partitions
            .value(0)
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("partitions elements are VARCHAR")
            .iter()
            .map(|v| v.unwrap_or_default().to_string())
            .collect();
        return ObservedDelta {
            changed_keys,
            partitions: parts,
        };
    }
    panic!("expected exactly one row in the observed-delta batches");
}

/// Regression for the reviewer finding on Phase 6
/// (`docs/plans/20260719-prod-w8-composed-axes-followups.md`): `derive_clamp_
/// and_locality`'s fixed-point loop over composed-source candidates
/// documents an "N maintained models converges within N passes" argument
/// that assumes an acyclic model-ref graph. `build_forward_graph` only
/// refuses a literal self-reference (a model naming itself) before this
/// call, not a longer cycle among maintained `grain: key` composed models —
/// and this call site runs before `DependencyGraph::execution_order()` (the
/// real cycle detector). Two `grain: key` composed models whose driving refs
/// point at each other, each with its own directly-clocked declared source
/// at a DIFFERENT granularity than the other's admitted output, oscillate
/// forever: each pass, admitting `a` (Day) makes `b`'s candidate set
/// ambiguous (`raw_b`: Month + `a`: Day) so `b` stops admitting, which drops
/// `a` from `b`'s candidates and lets `b` re-admit (Month) next pass, which
/// in turn makes `a`'s own candidate set ambiguous — a period-2 oscillation
/// the consecutive-state equality check can never observe as convergence.
/// This must return an `Err` (the iteration cap) rather than hang.
#[test]
fn cyclic_composed_model_refs_return_an_error_instead_of_hanging() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path();
    smelt_yml(root);
    write(
        root,
        "models/sources/raw_a.yml",
        "description: raw_a\ncolumns:\n- name: k\n  type: INTEGER\n- name: d\n  type: DATE\n\
         - name: v\n  type: INTEGER\n\
         mutation_profile:\n  kind: append_only\n\
         timeseries:\n  partition_column: d\n  event_time_column: d\n  granularity: day\n",
    );
    write(
        root,
        "models/sources/raw_b.yml",
        "description: raw_b\ncolumns:\n- name: k\n  type: INTEGER\n- name: d\n  type: DATE\n\
         - name: v\n  type: INTEGER\n\
         mutation_profile:\n  kind: append_only\n\
         timeseries:\n  partition_column: d\n  event_time_column: d\n  granularity: month\n",
    );
    write(
        root,
        "models/a.sql",
        "---\nmaterialization: table\nrefresh: incremental\ngrain: key\n\
         timeseries:\n  partition_column: d\n  event_time_column: d\n  granularity: day\n---\n\
         SELECT k, d, SUM(v) AS v\nFROM smelt.sources.raw_a\n\
         WHERE k IN (SELECT k FROM smelt.b)\nGROUP BY 1, 2\n",
    );
    write(
        root,
        "models/b.sql",
        "---\nmaterialization: table\nrefresh: incremental\ngrain: key\n\
         timeseries:\n  partition_column: d\n  event_time_column: d\n  granularity: month\n---\n\
         SELECT k, d, SUM(v) AS v\nFROM smelt.sources.raw_b\n\
         WHERE k IN (SELECT k FROM smelt.a)\nGROUP BY 1, 2\n",
    );

    let discovery = ModelDiscovery::new(root.to_path_buf(), vec!["models".to_string()]);
    let models = discovery.discover_models().expect("discover models");
    let source_infos = discover_source_infos(root, &["models".to_string()]);

    let result = build_forward_graph(&models, &source_infos);
    let err = result.expect_err(
        "a cyclic model-ref graph between two mutually-referencing grain: key composed models \
         must fail loud (the iteration cap), not silently succeed or hang",
    );
    let message = err.to_string();
    assert!(
        message.contains("did not converge") || message.contains("cycle"),
        "expected the iteration-cap error to name non-convergence/cycle, got: {message}"
    );
}

/// Phase 5 (`docs/outcomes/20260809-output-delta-typing/phases/05-plan.md`):
/// `refuse_bare_keyed_origins` consults the model's own derived output-delta
/// verdict — a `--source` delta naming a bare keyed model whose derived
/// shape is `KeyedUpsert` is admitted; the same origin with a `General`
/// shape (a mutable-snapshot join key) still refuses, narrowed from the old
/// blanket "any bare keyed origin refuses" rule.
#[test]
fn bare_keyed_origin_refusal_narrows_to_general() {
    fn payments(root: &Path) {
        write(
            root,
            "models/sources/payments.yml",
            "description: payments\ncolumns:\n- name: user_id\n  type: INTEGER\n\
             - name: amount\n  type: DECIMAL(10,2)\n- name: d\n  type: DATE\n\
             mutation_profile:\n  kind: append_only\n\
             timeseries:\n  partition_column: d\n  event_time_column: d\n  granularity: day\n",
        );
    }

    // A bare keyed origin whose derived shape is KeyedUpsert (a plain
    // GROUP BY over one append-only source, no other edge in the graph to
    // poison the call) must be admitted.
    let admitted_tmp = tempfile::TempDir::new().unwrap();
    let admitted_root = admitted_tmp.path();
    smelt_yml(admitted_root);
    payments(admitted_root);
    write(
        admitted_root,
        "models/keyed_upsert_agg.sql",
        "---\nmaterialization: table\nrefresh: incremental\ngrain: key\n---\n\
         SELECT user_id, SUM(amount) AS total FROM smelt.sources.payments GROUP BY user_id\n",
    );
    let admitted_discovery =
        ModelDiscovery::new(admitted_root.to_path_buf(), vec!["models".to_string()]);
    let admitted_models = admitted_discovery
        .discover_models()
        .expect("discover models");
    let admitted_source_infos = discover_source_infos(admitted_root, &["models".to_string()]);
    let admitted = plan_since_upstream(
        &admitted_models,
        &admitted_source_infos,
        &[],
        &[SourceDelta {
            source: "keyed_upsert_agg".to_string(),
            landed: smelt_logical::maintenance::propagate::DayInterval::new(1, 2),
        }],
    );
    assert!(
        admitted.is_ok(),
        "a bare keyed origin whose derived shape is KeyedUpsert must be admitted: {admitted:?}"
    );

    // The same shape of origin, but its own derived shape is General (its
    // GROUP BY key includes a column read off a mutable-snapshot join) —
    // must still refuse, narrowed to name the degrading operator.
    let refused_tmp = tempfile::TempDir::new().unwrap();
    let refused_root = refused_tmp.path();
    smelt_yml(refused_root);
    payments(refused_root);
    write(
        refused_root,
        "models/sources/dims.yml",
        "description: dims\ncolumns:\n- name: user_id\n  type: INTEGER\n\
         - name: region\n  type: VARCHAR\n\
         mutation_profile:\n  kind: mutable_snapshot\n",
    );
    write(
        refused_root,
        "models/general_agg.sql",
        "---\nmaterialization: table\nrefresh: incremental\ngrain: key\n---\n\
         SELECT p.user_id, SUM(p.amount) AS total, d.region\n\
         FROM smelt.sources.payments p\n\
         JOIN smelt.sources.dims d ON p.user_id = d.user_id\n\
         GROUP BY p.user_id, d.region\n",
    );
    let refused_discovery =
        ModelDiscovery::new(refused_root.to_path_buf(), vec!["models".to_string()]);
    let refused_models = refused_discovery
        .discover_models()
        .expect("discover models");
    let refused_source_infos = discover_source_infos(refused_root, &["models".to_string()]);
    let refused = plan_since_upstream(
        &refused_models,
        &refused_source_infos,
        &[],
        &[SourceDelta {
            source: "general_agg".to_string(),
            landed: smelt_logical::maintenance::propagate::DayInterval::new(1, 2),
        }],
    )
    .expect_err("a bare keyed origin whose derived shape is General must still refuse");
    let msg = refused.to_string();
    assert!(msg.contains("MaintenanceGraphUnsupportedNode"), "{msg}");
    assert!(msg.contains("general_agg"), "{msg}");
}

/// Phase 15 (`docs/outcomes/20260815-definition-delta-migrate/phases/
/// 15-plan.md`): [`load_observed_delta_lookup`] builds the read-side lookup
/// [`plan_since_upstream_with_observed_deltas`] consults, keyed exactly
/// `(model, iso(start), iso(end))`, for a model-address delta origin — and
/// skips a raw-source delta origin entirely (never a valid observed-delta
/// key).
#[tokio::test]
async fn load_observed_delta_lookup_keys_by_model_and_window() {
    let project_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/timeseries")
        .canonicalize()
        .expect("examples/timeseries exists");

    let discovery = ModelDiscovery::new(project_dir.clone(), vec!["models".to_string()]);
    let models = discovery.discover_models().expect("discover models");
    let model_names: BTreeSet<String> = models.iter().map(|m| m.canonical_path()).collect();

    let window = smelt_logical::maintenance::propagate::DayInterval::new(20, 21);
    let window_start = smelt_logical::maintenance::propagate::ordinal_to_iso(window.start);
    let window_end = smelt_logical::maintenance::propagate::ordinal_to_iso(window.end);

    let tmp = tempfile::TempDir::new().unwrap();
    let db_path = tmp.path().join("test.duckdb");
    let backend = smelt_backend_duckdb::DuckDbBackend::new(&db_path, "main")
        .await
        .expect("open duckdb backend");
    let ddl = smelt_state::ddl_duckdb::generate_observed_delta_table_ddl("main");
    backend.execute_sql(&ddl).await.expect("create delta table");
    let upsert = smelt_state::ddl_duckdb::generate_observed_delta_upsert_sql(
        "main",
        "user_daily_spend",
        &window_start,
        &window_end,
        "SELECT 'u1'::VARCHAR AS delta_key, NULL::VARCHAR AS delta_partition",
    );
    backend
        .execute_sql(&upsert)
        .await
        .expect("record a delta for user_daily_spend");

    let deltas = vec![
        SourceDelta {
            source: "user_daily_spend".to_string(),
            landed: window,
        },
        SourceDelta {
            source: "bronze".to_string(),
            landed: window,
        },
    ];

    let lookup = load_observed_delta_lookup(&backend, "main", &deltas, &model_names)
        .await
        .expect("load succeeds");

    assert_eq!(
        lookup.len(),
        1,
        "only the model-address delta origin is looked up, the raw source is skipped: \
         {lookup:?}"
    );
    let key = (
        "user_daily_spend".to_string(),
        window_start.clone(),
        window_end.clone(),
    );
    assert_eq!(
        lookup.get(&key).map(|od| od.changed_keys.clone()),
        Some(vec!["u1".to_string()]),
        "the lookup must be keyed exactly (model, iso(start), iso(end)): {lookup:?}"
    );
}

/// A fake [`Backend`] reporting a non-DuckDB dialect — every method beyond
/// `dialect()` panics if called, since [`load_observed_delta_lookup`] must
/// never reach a backend call for a non-DuckDB target (the read-side
/// fallback is unconditional, `read_observed_delta`'s own dialect guard).
struct NonDuckDbBackend;

#[async_trait]
impl Backend for NonDuckDbBackend {
    async fn execute_sql(&self, _sql: &str) -> Result<Vec<RecordBatch>, BackendError> {
        unimplemented!("must not be called for a non-DuckDB target")
    }
    async fn create_table_as(&self, _: &str, _: &str, _: &str) -> Result<(), BackendError> {
        unimplemented!()
    }
    async fn create_view_as(&self, _: &str, _: &str, _: &str) -> Result<(), BackendError> {
        unimplemented!()
    }
    async fn drop_table_if_exists(&self, _: &str, _: &str) -> Result<(), BackendError> {
        unimplemented!()
    }
    async fn drop_view_if_exists(&self, _: &str, _: &str) -> Result<(), BackendError> {
        unimplemented!()
    }
    async fn get_row_count(&self, _: &str, _: &str) -> Result<usize, BackendError> {
        unimplemented!()
    }
    async fn get_preview(
        &self,
        _: &str,
        _: &str,
        _: usize,
    ) -> Result<Vec<RecordBatch>, BackendError> {
        unimplemented!()
    }
    async fn table_exists(&self, _: &str, _: &str) -> Result<bool, BackendError> {
        unimplemented!()
    }
    async fn ensure_schema(&self, _: &str) -> Result<(), BackendError> {
        unimplemented!()
    }
    fn dialect(&self) -> SqlDialect {
        SqlDialect::SparkSQL
    }
    fn capabilities(&self) -> BackendCapabilities {
        unimplemented!()
    }
    async fn load_table(
        &self,
        _: &str,
        _: &str,
        _: SchemaRef,
        _: Vec<RecordBatch>,
    ) -> Result<(), BackendError> {
        unimplemented!()
    }
    async fn delete_partitions(
        &self,
        _: &str,
        _: &str,
        _: &PartitionRange,
    ) -> Result<(), BackendError> {
        unimplemented!()
    }
    async fn insert_into_from_query(&self, _: &str, _: &str, _: &str) -> Result<(), BackendError> {
        unimplemented!()
    }
    async fn insert_overwrite(
        &self,
        _: &str,
        _: &str,
        _: &str,
        _: &PartitionRange,
    ) -> Result<(), BackendError> {
        unimplemented!()
    }
}

/// [`load_observed_delta_lookup`] on a non-DuckDB target reads back an
/// EMPTY lookup — never an error — matching every other observed-delta
/// read's fallback posture.
#[tokio::test]
async fn load_observed_delta_lookup_is_empty_on_a_non_duckdb_target() {
    let project_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/timeseries")
        .canonicalize()
        .expect("examples/timeseries exists");
    let discovery = ModelDiscovery::new(project_dir.clone(), vec!["models".to_string()]);
    let models = discovery.discover_models().expect("discover models");
    let model_names: BTreeSet<String> = models.iter().map(|m| m.canonical_path()).collect();

    let window = smelt_logical::maintenance::propagate::DayInterval::new(20, 21);
    let deltas = vec![SourceDelta {
        source: "user_daily_spend".to_string(),
        landed: window,
    }];

    let backend = NonDuckDbBackend;
    let lookup = load_observed_delta_lookup(&backend, "main", &deltas, &model_names)
        .await
        .expect("a non-DuckDB target must not error");
    assert!(
        lookup.is_empty(),
        "a non-DuckDB target's lookup must be empty, not an error: {lookup:?}"
    );
}

// ---------------------------------------------------------------------------
// Phase 21 (`docs/outcomes/20260815-definition-delta-migrate/outcome.md`):
// the keyed dirt-set channel reaches the real per-workspace assembly — a
// chain of two bare `grain: key` models (`keyed_a`, admitted `KeyedUpsert`
// over an append-only source; `keyed_b`, reading `keyed_a`'s own output)
// feeding a Day-grain reader. Before this phase, `keyed_b` (dirtied only
// through the keyed channel — both its own inbound edge's endpoints are
// keyed-grain) was a one-hop dead end at the pure-math layer
// (`smelt_logical::maintenance::propagate::propagate`), so `reader` never
// saw any dirt at all and `plan_since_upstream` scheduled nothing.
// ---------------------------------------------------------------------------

fn write_bare_keyed_chain_workspace(root: &std::path::Path) {
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
        "models/keyed_a.sql",
        "---\nmaterialization: table\nrefresh: incremental\ngrain: key\n---\n\
         SELECT user_id, SUM(amount) AS total FROM smelt.sources.payments GROUP BY user_id\n",
    );
    write(
        root,
        "models/keyed_b.sql",
        "---\nmaterialization: table\nrefresh: incremental\ngrain: key\n---\n\
         SELECT user_id, SUM(total) AS grand_total FROM smelt.keyed_a GROUP BY user_id\n",
    );
    write(
        root,
        "models/reader.sql",
        "---\nmaterialization: table\nrefresh: incremental\ngrain: partition\n\
         unique_key: [user_id]\n\
         timeseries:\n  partition_column: d\n  event_time_column: d\n  granularity: day\n---\n\
         SELECT user_id, grand_total, CURRENT_DATE AS d FROM smelt.keyed_b\n",
    );
}

/// A bare `grain: key` model (`keyed_a`) whose landed delta cascades through
/// a second bare keyed model (`keyed_b`) to a Day-grain reader:
/// `plan_since_upstream` must schedule BOTH `keyed_b` (keyed-only dirt, no
/// interval axis — a whole-table run) and `reader` (widened to whole-table
/// by the keyed-to-clocked edge), never refusing with
/// `MaintenanceGraphUnsupportedNode`. The origin (`keyed_a`) is not itself
/// re-run — its landed delta is the window a completed run already wrote.
#[test]
fn bare_keyed_model_with_readers_is_scheduled() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path();
    write_bare_keyed_chain_workspace(root);

    let discovery = ModelDiscovery::new(root.to_path_buf(), vec!["models".to_string()]);
    let models = discovery.discover_models().expect("discover models");
    let source_infos = discover_source_infos(root, &["models".to_string()]);

    let edges = build_forward_graph(&models, &source_infos).expect(
        "the graph must build: keyed_a and keyed_b both admit a KeyedUpsert output-delta \
         shape over their GROUP BY user_id",
    );
    assert!(
        edges
            .iter()
            .any(|e| e.upstream == "keyed_a" && e.downstream == "keyed_b"),
        "expected a keyed_a -> keyed_b edge: {edges:?}"
    );
    assert!(
        edges
            .iter()
            .any(|e| e.upstream == "keyed_b" && e.downstream == "reader"),
        "expected a keyed_b -> reader edge: {edges:?}"
    );

    let order = vec![
        "keyed_a".to_string(),
        "keyed_b".to_string(),
        "reader".to_string(),
    ];
    let deltas = vec![SourceDelta {
        source: "keyed_a".to_string(),
        landed: smelt_logical::maintenance::propagate::DayInterval::new(20, 21),
    }];
    let plan = plan_since_upstream(&models, &source_infos, &order, &deltas).expect(
        "a landed delta on a bare keyed origin must cascade past keyed_b to reader without \
         refusing",
    );

    assert!(
        !plan.runs.iter().any(|r| r.model == "keyed_a"),
        "the delta origin must NOT be re-run: {:?}",
        plan.runs
    );
    assert!(
        plan.runs.iter().any(|r| r.model == "keyed_b"),
        "keyed_b must be scheduled — it carries keyed dirt cascaded from keyed_a: {:?}",
        plan.runs
    );
    assert!(
        plan.runs.iter().any(|r| r.model == "reader"),
        "reader must be scheduled — a clocked reader of a keyed-dirty node gets whole-table \
         dirt: {:?}",
        plan.runs
    );
    // Dependency order: keyed_b before reader.
    let keyed_b_pos = plan.runs.iter().position(|r| r.model == "keyed_b").unwrap();
    let reader_pos = plan.runs.iter().position(|r| r.model == "reader").unwrap();
    assert!(
        keyed_b_pos < reader_pos,
        "keyed_b must be scheduled before reader: {:?}",
        plan.runs
    );
}

/// The rendered dirty-set report names a keyed-only-dirty node distinctly
/// from an interval `RUN` line, naming its key columns and upstream.
#[test]
fn keyed_dirt_appears_in_the_dirty_set_report() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path();
    write_bare_keyed_chain_workspace(root);

    let discovery = ModelDiscovery::new(root.to_path_buf(), vec!["models".to_string()]);
    let models = discovery.discover_models().expect("discover models");
    let source_infos = discover_source_infos(root, &["models".to_string()]);

    let order = vec![
        "keyed_a".to_string(),
        "keyed_b".to_string(),
        "reader".to_string(),
    ];
    let deltas = vec![SourceDelta {
        source: "keyed_a".to_string(),
        landed: smelt_logical::maintenance::propagate::DayInterval::new(20, 21),
    }];
    let plan = plan_since_upstream(&models, &source_infos, &order, &deltas).expect("plan");

    assert!(
        plan.dirty_set_report.contains("keyed_b <-(keyed) keyed_a"),
        "the dirty set must name the keyed edge distinctly from an interval line: {}",
        plan.dirty_set_report
    );
    assert!(
        plan.dirty_set_report.contains("user_id"),
        "the dirty set must name the affected key column: {}",
        plan.dirty_set_report
    );
    assert!(
        plan.dirty_set_report.contains("RUN keyed_b: keyed"),
        "keyed_b's own scheduled run must be reported as a keyed (not interval) run: {}",
        plan.dirty_set_report
    );
    assert!(
        plan.dirty_set_report.contains("RUN reader:"),
        "reader's scheduled run must still be reported: {}",
        plan.dirty_set_report
    );
}
