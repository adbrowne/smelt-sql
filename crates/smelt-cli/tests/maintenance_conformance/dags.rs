//! Generated DAGs (`docs/plans/20260712-generative-maintenance-conformance.md`
//! Phase 10; `docs/specs/incremental_models.md` §"The equivalence invariant").
//!
//! Extends the single-model conformance gate (`gate.rs`) to small generated
//! graphs of models: a two-hop chain, a diamond (fan-out + confluence), and
//! a payload-leak pair, each staged through
//! `smelt_maintenance_testkit::dag` and driven end-to-end through the real
//! `smelt_runtime::execute_project` pipeline (the Link-C rule — never
//! `run_incremental_sequence`, never a hand-injected `WHERE`).
//!
//! Every case compares an INCREMENTALLY-PROPAGATED project (an initial run,
//! then a landed delta propagated via `smelt_runtime::propagation::
//! plan_since_upstream`, or resolved via `resolve_build_plan`) against an
//! INDEPENDENTLY-STAGED full-refresh twin over the same combined history —
//! never the same project compared against itself, and never just the sink:
//! every node's materialized table is compared
//! (`smelt_maintenance_testkit::dag::fetch_node_multiset`).

use std::path::Path;

use chrono::{Datelike, NaiveDate};
use proptest::strategy::{Strategy, ValueTree};
use proptest::test_runner::TestRunner;
use tempfile::TempDir;

use smelt_core::{discover_source_infos, ModelDiscovery};
use smelt_logical::maintenance::edge_type::Addressing;
use smelt_logical::maintenance::propagate::{day_ordinal, DayInterval};
use smelt_maintenance_testkit::dag::{
    chain_dag, classify_node, diamond_dag, fetch_node_multiset, insert_rows, keyed_chain_dag,
    keyed_partition_sink_dag, keyed_sink_dag, leak_dag, stage_dag, DagRecipe,
};
use smelt_maintenance_testkit::link_c_harness::base_request;
use smelt_maintenance_testkit::recipe::arb_payload_value;
use smelt_maintenance_testkit::verdict::Verdict;
use smelt_runtime::propagation::{
    build_forward_graph, plan_since_upstream, resolve_build_plan, SourceDelta,
};

/// Deterministic case count — DAG cases stage TWO independent projects and
/// drive `execute_project` multiple times each, roughly 2-3x a single-model
/// `gate.rs` case, so the default is smaller; `SMELT_CONFORMANCE_CASES`
/// (the same env override `gate.rs` reads) still scales it up for a deep
/// local/soak pass.
const DEFAULT_CASES: usize = 6;

fn case_count() -> usize {
    std::env::var("SMELT_CONFORMANCE_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_CASES)
}

fn base_day() -> NaiveDate {
    NaiveDate::from_ymd_opt(2024, 1, 1).expect("valid base date")
}

/// One generated one-day window of `(d, id, val)` rows, `id` chosen
/// deterministically (by window offset + row position, so it never collides
/// across windows) and `val` drawn from [`arb_payload_value`] (design §5's
/// integer-valued, bounded numeric-payload discipline).
fn arb_window(offset_days: i64, n_rows: usize) -> impl Strategy<Value = WindowRows> {
    proptest::collection::vec(arb_payload_value(), n_rows).prop_map(move |vals| {
        let start = base_day() + chrono::Duration::days(offset_days);
        let end = start + chrono::Duration::days(1);
        let rows = vals
            .into_iter()
            .enumerate()
            .map(|(i, val)| (start, offset_days * 1000 + i as i64, val))
            .collect();
        WindowRows { start, end, rows }
    })
}

#[derive(Debug, Clone)]
struct WindowRows {
    start: NaiveDate,
    end: NaiveDate,
    rows: Vec<(NaiveDate, i64, i64)>,
}

fn landed_delta(source: &str, w: &WindowRows) -> SourceDelta {
    SourceDelta {
        source: source.to_string(),
        landed: DayInterval::new(
            day_ordinal(w.start.year() as i64, w.start.month(), w.start.day()),
            day_ordinal(w.end.year() as i64, w.end.month(), w.end.day()),
        ),
    }
}

fn discover(
    project_dir: &Path,
) -> anyhow::Result<(
    Vec<smelt_core::ModelFile>,
    Vec<smelt_core::sources::SourceInfo>,
)> {
    let config = smelt_core::config::Config::load(project_dir)?;
    let discovery = ModelDiscovery::new(project_dir.to_path_buf(), config.paths.clone());
    let models = discovery.discover_models()?;
    let source_infos = discover_source_infos(project_dir, &config.paths);
    Ok((models, source_infos))
}

/// Stage `dag` twice (an "inc" project driven incrementally, a "full"
/// project rebuilt from scratch over the combined history) under fresh
/// subdirectories of `tmp`.
fn stage_pair(
    dag: &DagRecipe,
    tmp: &TempDir,
    case: usize,
) -> anyhow::Result<(
    smelt_maintenance_testkit::link_c_harness::LinkCProject,
    smelt_maintenance_testkit::link_c_harness::LinkCProject,
)> {
    let inc_dir = tmp.path().join(format!("inc-{case}"));
    let inc_db = inc_dir.join("db.duckdb");
    std::fs::create_dir_all(&inc_dir)?;
    let inc = stage_dag(dag, &inc_dir, &inc_db)?;

    let full_dir = tmp.path().join(format!("full-{case}"));
    let full_db = full_dir.join("db.duckdb");
    std::fs::create_dir_all(&full_dir)?;
    let full = stage_dag(dag, &full_dir, &full_db)?;

    Ok((inc, full))
}

/// Assert every node in `dag` compares equal (as a sorted multiset) between
/// `inc` and `full` — never just the sink.
fn assert_every_node_equal(
    dag: &DagRecipe,
    inc: &smelt_maintenance_testkit::link_c_harness::LinkCProject,
    full: &smelt_maintenance_testkit::link_c_harness::LinkCProject,
    case: usize,
    context: &str,
) -> anyhow::Result<()> {
    let inc_conn = inc.connect()?;
    let full_conn = full.connect()?;
    for idx in 0..dag.nodes.len() {
        let inc_rows = fetch_node_multiset(&inc_conn, dag, idx, None);
        let full_rows = fetch_node_multiset(&full_conn, dag, idx, None);
        assert_eq!(
            inc_rows,
            full_rows,
            "case {case} ({context}): node {:?} diverged from the full-refresh oracle \
             (incremental left, full-refresh right; {} vs {} rows)",
            dag.nodes[idx].name,
            inc_rows.len(),
            full_rows.len(),
        );
    }
    Ok(())
}

/// `chain_since_upstream_dirty_set_suffices` (Phase 10 TDD list): generated
/// two-hop chain (`dag_chain_a` -> `dag_chain_b`); for generated landed
/// deltas, running exactly what `plan_since_upstream` schedules leaves EVERY
/// node multiset-equal to a from-scratch full refresh over the combined
/// history — executes-and-compares, which
/// `crates/smelt-runtime/tests/since_upstream_propagation.rs` (planning
/// only) does not.
#[test]
fn chain_since_upstream_dirty_set_suffices() {
    let mut runner = TestRunner::deterministic();
    let strat = (arb_window(0, 2), arb_window(1, 2));
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");

    for i in 0..case_count() {
        let (w1, w2) = strat
            .new_tree(&mut runner)
            .expect("generate case")
            .current();
        let dag = chain_dag();
        let tmp = TempDir::new().expect("tempdir");
        let (inc, full) = stage_pair(&dag, &tmp, i).expect("stage pair");

        // Initial build over w1 only: `start`/`end` both `None` triggers
        // full-refresh over whatever the source currently holds
        // (`ExecuteRequest::start`'s own doc comment).
        {
            let conn = inc.connect().expect("connect inc");
            insert_rows(&conn, &dag, &w1.rows).expect("insert w1 into inc");
        }
        rt.block_on(inc.run_quiet(&format!("chain-{i}-init"), base_request("dev")))
            .unwrap_or_else(|e| panic!("case {i}: initial inc build failed: {e}"));

        // The landed delta: w2 lands on the source AFTER the initial build.
        {
            let conn = inc.connect().expect("connect inc");
            insert_rows(&conn, &dag, &w2.rows).expect("insert w2 into inc");
        }

        let (models, source_infos) = discover(&inc.project_dir).expect("discover inc");
        let order = dag.order();
        let deltas = vec![landed_delta(&dag.source.name, &w2)];
        let plan = plan_since_upstream(&models, &source_infos, &order, &deltas)
            .unwrap_or_else(|e| panic!("case {i}: plan_since_upstream failed: {e}"));
        assert!(
            !plan.runs.is_empty(),
            "case {i}: expected at least dag_chain_a to be scheduled by the landed delta: \
             {}",
            plan.dirty_set_report
        );

        for run in &plan.runs {
            let mut req = base_request("dev");
            req.start = run.start.clone();
            req.end = run.end.clone();
            rt.block_on(inc.run_quiet(&format!("chain-{i}-prop-{}", run.model), req))
                .unwrap_or_else(|e| {
                    panic!("case {i}: propagated run for {} failed: {e}", run.model)
                });
        }

        // The independent full-refresh oracle: the SAME combined history,
        // one from-scratch build.
        {
            let conn = full.connect().expect("connect full");
            insert_rows(&conn, &dag, &w1.rows).expect("insert w1 into full");
            insert_rows(&conn, &dag, &w2.rows).expect("insert w2 into full");
        }
        rt.block_on(full.run_quiet(&format!("chain-{i}-full"), base_request("dev")))
            .unwrap_or_else(|e| panic!("case {i}: full-refresh oracle build failed: {e}"));

        assert_every_node_equal(&dag, &inc, &full, i, "chain").expect("compare nodes");
    }
}

/// `diamond_propagation_suffices` (Phase 10 TDD list): the same
/// executes-and-compares check over a generated diamond (`dag_diamond_a`/
/// `dag_diamond_b` fan out from the source on disjoint id parities,
/// `dag_diamond_c` confluences both via `UNION ALL`) — every node compared,
/// not just the confluence.
#[test]
fn diamond_propagation_suffices() {
    let mut runner = TestRunner::deterministic();
    let strat = (arb_window(0, 3), arb_window(1, 3));
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");

    for i in 0..case_count() {
        let (w1, w2) = strat
            .new_tree(&mut runner)
            .expect("generate case")
            .current();
        let dag = diamond_dag();
        let tmp = TempDir::new().expect("tempdir");
        let (inc, full) = stage_pair(&dag, &tmp, i).expect("stage pair");

        {
            let conn = inc.connect().expect("connect inc");
            insert_rows(&conn, &dag, &w1.rows).expect("insert w1 into inc");
        }
        rt.block_on(inc.run_quiet(&format!("diamond-{i}-init"), base_request("dev")))
            .unwrap_or_else(|e| panic!("case {i}: initial inc build failed: {e}"));

        {
            let conn = inc.connect().expect("connect inc");
            insert_rows(&conn, &dag, &w2.rows).expect("insert w2 into inc");
        }

        let (models, source_infos) = discover(&inc.project_dir).expect("discover inc");
        let order = dag.order();
        let deltas = vec![landed_delta(&dag.source.name, &w2)];
        let plan = plan_since_upstream(&models, &source_infos, &order, &deltas)
            .unwrap_or_else(|e| panic!("case {i}: plan_since_upstream failed: {e}"));
        assert!(
            !plan.runs.is_empty(),
            "case {i}: expected the landed delta to propagate through the diamond: {}",
            plan.dirty_set_report
        );
        // Both fan-out branches must be dirtied by a single source delta —
        // the propagation-sufficiency claim this test exists to check.
        assert!(
            plan.runs.iter().any(|r| r.model == "dag_diamond_a")
                && plan.runs.iter().any(|r| r.model == "dag_diamond_b"),
            "case {i}: expected BOTH diamond branches to be scheduled: {:?}",
            plan.runs
        );

        for run in &plan.runs {
            let mut req = base_request("dev");
            req.start = run.start.clone();
            req.end = run.end.clone();
            rt.block_on(inc.run_quiet(&format!("diamond-{i}-prop-{}", run.model), req))
                .unwrap_or_else(|e| {
                    panic!("case {i}: propagated run for {} failed: {e}", run.model)
                });
        }

        {
            let conn = full.connect().expect("connect full");
            insert_rows(&conn, &dag, &w1.rows).expect("insert w1 into full");
            insert_rows(&conn, &dag, &w2.rows).expect("insert w2 into full");
        }
        rt.block_on(full.run_quiet(&format!("diamond-{i}-full"), base_request("dev")))
            .unwrap_or_else(|e| panic!("case {i}: full-refresh oracle build failed: {e}"));

        assert_every_node_equal(&dag, &inc, &full, i, "diamond").expect("compare nodes");
    }
}

/// `include_upstreams_resolved_slices_suffice` (Phase 10 TDD list): staging
/// EXACTLY the backward-resolved slices (`resolve_build_plan`, the
/// `smelt build --include-upstreams` v1 contract: an explicit `--period`,
/// never inferred) and building bottom-up over the generated chain yields a
/// target period equal to a build over complete history. Mirrors
/// `crates/smelt-cli/tests/include_upstreams.rs::resolved_slices_suffice`'s
/// own pattern (seed ONLY the resolved slice; if the resolution ever
/// under-computed the required interval, the omitted rows would make this
/// fail rather than pass vacuously).
#[test]
fn include_upstreams_resolved_slices_suffice() {
    let mut runner = TestRunner::deterministic();
    // `period`: the single day the target (`dag_chain_b`) is requested over.
    // `outside`: an extra day OUTSIDE the period, seeded only in the full
    // oracle, proving the partial build's resolved slice does not need it.
    let strat = (arb_window(5, 2), arb_window(9, 2));
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");

    for i in 0..case_count() {
        let (period, outside) = strat
            .new_tree(&mut runner)
            .expect("generate case")
            .current();
        let dag = chain_dag();
        let tmp = TempDir::new().expect("tempdir");
        let (partial, full) = stage_pair(&dag, &tmp, i).expect("stage pair");

        let (models, source_infos) = discover(&partial.project_dir).expect("discover partial");
        let period_interval = DayInterval::new(
            day_ordinal(
                period.start.year() as i64,
                period.start.month(),
                period.start.day(),
            ),
            day_ordinal(
                period.end.year() as i64,
                period.end.month(),
                period.end.day(),
            ),
        );
        let resolved = resolve_build_plan(&models, &source_infos, "dag_chain_b", period_interval)
            .unwrap_or_else(|e| panic!("case {i}: resolve_build_plan failed: {e}"));
        assert!(
            !resolved.build_order.is_empty(),
            "case {i}: expected a non-empty resolved build order: {}",
            resolved.report
        );
        assert_eq!(
            resolved.build_order[0].model, "dag_chain_a",
            "case {i}: ancestor-first — dag_chain_a must build before dag_chain_b: {}",
            resolved.report
        );

        // Seed the partial project with ONLY the period's own rows — if
        // `resolve_build_plan` ever under-resolved the required source
        // interval, the run below would be missing data it needs and this
        // case would fail rather than pass vacuously.
        {
            let conn = partial.connect().expect("connect partial");
            insert_rows(&conn, &dag, &period.rows).expect("insert period rows into partial");
        }
        for run in &resolved.build_order {
            let mut req = base_request("dev");
            req.start = run.start.clone();
            req.end = run.end.clone();
            rt.block_on(partial.run_quiet(&format!("resolve-{i}-{}", run.model), req))
                .unwrap_or_else(|e| {
                    panic!("case {i}: resolved build for {} failed: {e}", run.model)
                });
        }

        // The full oracle: the SAME period rows, plus an extra day outside
        // the requested period, built in one from-scratch pass.
        {
            let conn = full.connect().expect("connect full");
            insert_rows(&conn, &dag, &period.rows).expect("insert period rows into full");
            insert_rows(&conn, &dag, &outside.rows).expect("insert outside rows into full");
        }
        rt.block_on(full.run_quiet(&format!("resolve-{i}-full"), base_request("dev")))
            .unwrap_or_else(|e| panic!("case {i}: full-refresh oracle build failed: {e}"));

        let where_clause = format!("d = DATE '{}'", period.start.format("%Y-%m-%d"));
        let partial_conn = partial.connect().expect("connect partial for read-back");
        let full_conn = full.connect().expect("connect full for read-back");
        for idx in 0..dag.nodes.len() {
            let partial_rows = fetch_node_multiset(&partial_conn, &dag, idx, Some(&where_clause));
            let full_rows = fetch_node_multiset(&full_conn, &dag, idx, Some(&where_clause));
            assert_eq!(
                partial_rows, full_rows,
                "case {i}: node {:?} over the requested period diverged between the \
                 --include-upstreams-resolved partial build and the full-history oracle",
                dag.nodes[idx].name
            );
        }
    }
}

/// `upstream_payload_in_downstream_skeleton_position` (Phase 10 TDD list):
/// the leak family — `dag_leak_b` groups by `dag_leak_a`'s own payload
/// column `val` (a skeleton-position leak). The pair must either refuse
/// loudly (a named `Maintenance*`/admission diagnostic,
/// `classify_node`'s own fail-loud check) or uphold full equivalence under
/// propagation — NEVER silently diverge.
#[test]
fn upstream_payload_in_downstream_skeleton_position() {
    let mut runner = TestRunner::deterministic();
    let strat = (arb_window(0, 2), arb_window(1, 2));
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");

    let mut saw_admitted = false;
    let mut saw_refused = false;

    for i in 0..case_count() {
        let (w1, w2) = strat
            .new_tree(&mut runner)
            .expect("generate case")
            .current();
        let dag = leak_dag();
        let tmp = TempDir::new().expect("tempdir");
        let (inc, full) = stage_pair(&dag, &tmp, i).expect("stage pair");

        {
            let conn = inc.connect().expect("connect inc");
            insert_rows(&conn, &dag, &w1.rows).expect("insert w1 into inc");
        }
        rt.block_on(inc.run_quiet(&format!("leak-{i}-init"), base_request("dev")))
            .unwrap_or_else(|e| panic!("case {i}: initial inc build failed: {e}"));

        match classify_node(&inc, "dag_leak_b") {
            Ok(Verdict::Refused(diags)) => {
                saw_refused = true;
                assert!(
                    !diags.is_empty(),
                    "case {i}: a Refused verdict must carry at least one named diagnostic \
                     (classify_node's own fail-loud check enforces this; a bare assertion here \
                     as a second, independent check)"
                );
                // Refused is a safe, loud outcome — no equivalence claim is
                // made for a refused cell, so nothing further to drive for
                // this case.
                continue;
            }
            Ok(Verdict::Admitted(_)) => {
                saw_admitted = true;
            }
            Err(e) => panic!(
                "case {i}: classify_node returned neither a clean Admitted nor a fail-loud-backed \
                 Refused verdict: {e}"
            ),
        }

        // Admitted: the leak's claimed technique must actually uphold
        // equivalence under propagation, never silently diverge.
        {
            let conn = inc.connect().expect("connect inc");
            insert_rows(&conn, &dag, &w2.rows).expect("insert w2 into inc");
        }
        let (models, source_infos) = discover(&inc.project_dir).expect("discover inc");
        let order = dag.order();
        let deltas = vec![landed_delta(&dag.source.name, &w2)];
        let plan = plan_since_upstream(&models, &source_infos, &order, &deltas)
            .unwrap_or_else(|e| panic!("case {i}: plan_since_upstream failed: {e}"));
        for run in &plan.runs {
            let mut req = base_request("dev");
            req.start = run.start.clone();
            req.end = run.end.clone();
            rt.block_on(inc.run_quiet(&format!("leak-{i}-prop-{}", run.model), req))
                .unwrap_or_else(|e| {
                    panic!("case {i}: propagated run for {} failed: {e}", run.model)
                });
        }

        {
            let conn = full.connect().expect("connect full");
            insert_rows(&conn, &dag, &w1.rows).expect("insert w1 into full");
            insert_rows(&conn, &dag, &w2.rows).expect("insert w2 into full");
        }
        rt.block_on(full.run_quiet(&format!("leak-{i}-full"), base_request("dev")))
            .unwrap_or_else(|e| panic!("case {i}: full-refresh oracle build failed: {e}"));

        assert_every_node_equal(&dag, &inc, &full, i, "leak").expect("compare nodes");
    }

    assert!(
        saw_admitted || saw_refused,
        "generator health: the deterministic sample never classified dag_leak_b at all"
    );
}

/// A [`keyed_chain_dag`] case: an initial batch of rows over distinct ids,
/// then a delta batch that lands MORE rows for a nonempty proper subset of
/// those same ids (mutating that subset's `SUM(val)` without touching the
/// rest) — the shape `keyed_chain_fold_matches_full_refresh_oracle` and
/// `keyed_chain_maintains_only_the_changed_keys` both need: a real subset
/// of keys touched, a real subset left alone.
#[derive(Debug)]
struct KeyedCase {
    initial_rows: Vec<(NaiveDate, i64, i64)>,
    delta_rows: Vec<(NaiveDate, i64, i64)>,
    touched_ids: Vec<i64>,
    untouched_ids: Vec<i64>,
}

fn arb_keyed_case(n_ids: usize) -> impl Strategy<Value = KeyedCase> {
    assert!(
        n_ids >= 2,
        "arb_keyed_case needs at least 2 ids to guarantee a touched/untouched split"
    );
    (
        proptest::collection::vec(arb_payload_value(), n_ids),
        proptest::collection::vec(arb_payload_value(), n_ids),
        proptest::collection::vec(proptest::bool::ANY, n_ids),
    )
        .prop_map(move |(initial_vals, delta_vals, mut touch_mask)| {
            if !touch_mask.iter().any(|&t| t) {
                touch_mask[0] = true;
            }
            if touch_mask.iter().all(|&t| t) {
                touch_mask[1] = false;
            }
            let d = base_day();
            let initial_rows: Vec<(NaiveDate, i64, i64)> = initial_vals
                .into_iter()
                .enumerate()
                .map(|(i, v)| (d, i as i64, v))
                .collect();
            let delta_rows: Vec<(NaiveDate, i64, i64)> = delta_vals
                .into_iter()
                .zip(touch_mask.iter())
                .enumerate()
                .filter_map(|(i, (v, &touched))| touched.then_some((d, i as i64, v)))
                .collect();
            let touched_ids: Vec<i64> = touch_mask
                .iter()
                .enumerate()
                .filter_map(|(i, &t)| t.then_some(i as i64))
                .collect();
            let untouched_ids: Vec<i64> = touch_mask
                .iter()
                .enumerate()
                .filter_map(|(i, &t)| (!t).then_some(i as i64))
                .collect();
            KeyedCase {
                initial_rows,
                delta_rows,
                touched_ids,
                untouched_ids,
            }
        })
}

/// `keyed_chain_derives_a_typed_keyed_edge` (phase 8 plan test 1): over the
/// generated [`keyed_chain_dag`], `build_forward_graph` derives an edge
/// `dag_kchain_a -> dag_kchain_b` carrying a `KeyedUpsert` component
/// addressed `Addressing::Keyed` — the narrowed refusal admits it, unlike
/// the append-only-upstream case `keyed_grain_node_excluded_from_generated_graph`
/// still pins below (unchanged).
#[test]
fn keyed_chain_derives_a_typed_keyed_edge() {
    let dag = keyed_chain_dag();
    let tmp = TempDir::new().expect("tempdir");
    let dir = tmp.path().join("proj");
    let db = dir.join("db.duckdb");
    std::fs::create_dir_all(&dir).expect("create project dir");
    stage_dag(&dag, &dir, &db).expect("stage keyed chain dag");

    let (models, source_infos) = discover(&dir).expect("discover");
    let graph = build_forward_graph(&models, &source_infos).expect("build forward graph");
    let edge = graph
        .iter()
        .find(|e| e.upstream == "dag_kchain_a" && e.downstream == "dag_kchain_b")
        .unwrap_or_else(|| {
            panic!(
                "expected an edge dag_kchain_a -> dag_kchain_b in the generated graph: {graph:?}"
            )
        });
    assert!(
        edge.components.iter().any(|c| matches!(
            (&c.shape, &c.addressing),
            (
                smelt_logical::analysis::output_delta::OutputDelta::KeyedUpsert { .. },
                Addressing::Keyed { .. }
            )
        )),
        "expected a KeyedUpsert component addressed Addressing::Keyed on the edge into \
         dag_kchain_b: {edge:?}"
    );
}

/// `keyed_chain_fold_matches_full_refresh_oracle` (phase 8 plan test 2):
/// stage [`keyed_chain_dag`] twice, land an initial batch then a delta
/// touching a subset of the same ids, drive the inc project through
/// `execute_project` with no run window at all (a model-selecting run —
/// `base_request`'s `select: vec![]` runs every node, mirroring
/// `key_addressed_model_edge_lowering.rs::select_request`'s "these chains
/// have no clock" posture), and compare against an independently staged
/// full-refresh oracle over the combined history.
#[test]
fn keyed_chain_fold_matches_full_refresh_oracle() {
    let mut runner = TestRunner::deterministic();
    let strat = arb_keyed_case(4);
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");

    for i in 0..case_count() {
        let kcase = strat
            .new_tree(&mut runner)
            .expect("generate case")
            .current();
        let dag = keyed_chain_dag();
        let tmp = TempDir::new().expect("tempdir");
        let (inc, full) = stage_pair(&dag, &tmp, i).expect("stage pair");

        {
            let conn = inc.connect().expect("connect inc");
            insert_rows(&conn, &dag, &kcase.initial_rows).expect("insert initial rows into inc");
        }
        rt.block_on(inc.run_quiet(&format!("kchain-{i}-init"), base_request("dev")))
            .unwrap_or_else(|e| panic!("case {i}: initial inc build failed: {e}"));

        {
            let conn = inc.connect().expect("connect inc");
            insert_rows(&conn, &dag, &kcase.delta_rows).expect("insert delta rows into inc");
        }
        rt.block_on(inc.run_quiet(&format!("kchain-{i}-repair"), base_request("dev")))
            .unwrap_or_else(|e| panic!("case {i}: repair run failed: {e}"));

        {
            let conn = full.connect().expect("connect full");
            insert_rows(&conn, &dag, &kcase.initial_rows).expect("insert initial rows into full");
            insert_rows(&conn, &dag, &kcase.delta_rows).expect("insert delta rows into full");
        }
        rt.block_on(full.run_quiet(&format!("kchain-{i}-full"), base_request("dev")))
            .unwrap_or_else(|e| panic!("case {i}: full-refresh oracle build failed: {e}"));

        assert_every_node_equal(&dag, &inc, &full, i, "keyed chain").expect("compare nodes");
    }
}

/// `keyed_chain_maintains_only_the_changed_keys` (phase 8 plan test 3): the
/// gate-level analogue of phase 7's "no full-input rescan" — after the
/// repair run, an untouched id's row on `dag_kchain_b` is bit-identical to
/// its pre-repair value, while a touched id's row moved.
#[test]
fn keyed_chain_maintains_only_the_changed_keys() {
    let mut runner = TestRunner::deterministic();
    let strat = arb_keyed_case(4);
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");

    for i in 0..case_count() {
        let kcase = strat
            .new_tree(&mut runner)
            .expect("generate case")
            .current();
        let dag = keyed_chain_dag();
        let id_col = dag.source.key_column.clone();
        let tmp = TempDir::new().expect("tempdir");
        let dir = tmp.path().join(format!("inc-{i}"));
        let db = dir.join("db.duckdb");
        std::fs::create_dir_all(&dir).expect("create project dir");
        let inc = stage_dag(&dag, &dir, &db).expect("stage inc");

        {
            let conn = inc.connect().expect("connect inc");
            insert_rows(&conn, &dag, &kcase.initial_rows).expect("insert initial rows");
        }
        rt.block_on(inc.run_quiet(&format!("kchain-only-{i}-init"), base_request("dev")))
            .unwrap_or_else(|e| panic!("case {i}: initial inc build failed: {e}"));

        let conn = inc.connect().expect("connect inc for before-snapshot");
        let before: std::collections::BTreeMap<i64, Vec<Vec<String>>> = kcase
            .touched_ids
            .iter()
            .chain(kcase.untouched_ids.iter())
            .map(|&id| {
                let rows = fetch_node_multiset(&conn, &dag, 1, Some(&format!("{id_col} = {id}")));
                (id, rows)
            })
            .collect();
        drop(conn);

        {
            let conn = inc.connect().expect("connect inc");
            insert_rows(&conn, &dag, &kcase.delta_rows).expect("insert delta rows");
        }
        rt.block_on(inc.run_quiet(&format!("kchain-only-{i}-repair"), base_request("dev")))
            .unwrap_or_else(|e| panic!("case {i}: repair run failed: {e}"));

        let conn = inc.connect().expect("connect inc for after-snapshot");
        for &id in &kcase.untouched_ids {
            let after = fetch_node_multiset(&conn, &dag, 1, Some(&format!("{id_col} = {id}")));
            assert_eq!(
                before[&id], after,
                "case {i}: untouched id {id}'s row on dag_kchain_b must be bit-identical \
                 before/after the repair run"
            );
        }
        for &id in &kcase.touched_ids {
            let after = fetch_node_multiset(&conn, &dag, 1, Some(&format!("{id_col} = {id}")));
            assert_ne!(
                before[&id], after,
                "case {i}: touched id {id}'s row on dag_kchain_b must have moved after the \
                 repair run — its delta rows landed but the fold left it unchanged"
            );
        }
    }
}

/// `keyed_upstream_partition_downstream_matches_oracle` (phase 8 plan test
/// 4; incrementality assertion added by the scheduler-delta-signatures
/// outcome's phase 2,
/// `docs/outcomes/20260816-scheduler-delta-signatures/phases/02-plan.md`):
/// the [`keyed_partition_sink_dag`] combination — multiset-equal to the
/// full-refresh oracle (correctness), AND the repair run's own manifest
/// records `dag_kpart_b` maintaining via `per_group_recompute`
/// (incrementality — the run loop actually dispatches the derived
/// key-addressed model-edge cell, `docs/specs/incremental_models.md`
/// §"Upstream model edges" / §"Dispatch — from propagated components to
/// run units", rather than falling back to the ordinary correct-but-full
/// route).
#[test]
fn keyed_upstream_partition_downstream_matches_oracle() {
    let mut runner = TestRunner::deterministic();
    let strat = arb_keyed_case(4);
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");

    for i in 0..case_count() {
        let kcase = strat
            .new_tree(&mut runner)
            .expect("generate case")
            .current();
        let dag = keyed_partition_sink_dag();
        let tmp = TempDir::new().expect("tempdir");
        let (inc, full) = stage_pair(&dag, &tmp, i).expect("stage pair");

        {
            let conn = inc.connect().expect("connect inc");
            insert_rows(&conn, &dag, &kcase.initial_rows).expect("insert initial rows into inc");
        }
        rt.block_on(inc.run_quiet(&format!("kpart-{i}-init"), base_request("dev")))
            .unwrap_or_else(|e| panic!("case {i}: initial inc build failed: {e}"));

        {
            let conn = inc.connect().expect("connect inc");
            insert_rows(&conn, &dag, &kcase.delta_rows).expect("insert delta rows into inc");
        }
        let repair_outcome = rt
            .block_on(inc.run_quiet(&format!("kpart-{i}-repair"), base_request("dev")))
            .unwrap_or_else(|e| panic!("case {i}: repair run failed: {e}"));
        let repair_record = repair_outcome.models.get("dag_kpart_b").unwrap_or_else(|| {
            panic!("case {i}: repair run has no manifest entry for dag_kpart_b")
        });
        assert_eq!(
            repair_record.strategy, "per_group_recompute",
            "case {i}: dag_kpart_b (grain: partition) must dispatch the key-addressed model-\
             edge cell's repair family on a repair run, not the ordinary route — got strategy \
             '{}'",
            repair_record.strategy
        );

        {
            let conn = full.connect().expect("connect full");
            insert_rows(&conn, &dag, &kcase.initial_rows).expect("insert initial rows into full");
            insert_rows(&conn, &dag, &kcase.delta_rows).expect("insert delta rows into full");
        }
        rt.block_on(full.run_quiet(&format!("kpart-{i}-full"), base_request("dev")))
            .unwrap_or_else(|e| panic!("case {i}: full-refresh oracle build failed: {e}"));

        assert_every_node_equal(&dag, &inc, &full, i, "keyed partition sink").expect("compare");
    }
}

/// Review checklist ("keyed-grain nodes excluded from generated graphs"):
/// a keyed sink in a GENERATED graph must never derive a propagation edge,
/// mirroring
/// `crates/smelt-runtime/tests/since_upstream_propagation.rs::
/// keyed_grain_model_never_derives_an_edge`'s hand-typed-fixture assertion
/// over `smelt_maintenance_testkit::dag::keyed_sink_dag`'s generated
/// counterpart.
#[test]
fn keyed_grain_node_excluded_from_generated_graph() {
    let dag = keyed_sink_dag();
    let tmp = TempDir::new().expect("tempdir");
    let dir = tmp.path().join("proj");
    let db = dir.join("db.duckdb");
    std::fs::create_dir_all(&dir).expect("create project dir");
    stage_dag(&dag, &dir, &db).expect("stage keyed sink dag");

    let (models, source_infos) = discover(&dir).expect("discover");
    let graph = build_forward_graph(&models, &source_infos).expect("build forward graph");
    assert!(
        !graph.iter().any(|e| e.downstream == "dag_keyed_sink"),
        "a keyed-grain node must never derive a propagation edge in a GENERATED graph: {graph:?}"
    );
}
