//! The generated 2-3 node DAG family (chain/diamond/leak/keyed-sink) — the
//! target-parametrized twin of `maintenance_conformance/dags.rs`, driven
//! end-to-end through the real `smelt_runtime::execute_project` pipeline.
//! Staging, row insertion, and read-back all route through
//! [`smelt_backend::Backend`] (`dag::stage_dag_for_target`/
//! `insert_rows_via_backend`/`fetch_node_multiset_via_backend`), never a raw
//! host connection.

use anyhow::Result;
use chrono::{Datelike, NaiveDate};
use proptest::strategy::{Strategy, ValueTree};
use proptest::test_runner::TestRunner;
use tempfile::TempDir;

use smelt_core::{discover_source_infos, ModelDiscovery};
use smelt_logical::maintenance::propagate::{day_ordinal, DayInterval};
use smelt_runtime::propagation::{
    build_forward_graph, plan_since_upstream, resolve_build_plan, SourceDelta,
};

use crate::dag::{
    chain_dag, classify_node, diamond_dag, fetch_node_multiset_via_backend,
    insert_rows_via_backend, keyed_sink_dag, leak_dag, stage_dag_for_target, DagRecipe,
};
use crate::families::ConformanceBackend;
use crate::link_c_harness::{base_request, LinkCProject};
use crate::recipe::arb_payload_value;
use crate::verdict::Verdict;

fn base_day() -> NaiveDate {
    NaiveDate::from_ymd_opt(2024, 1, 1).expect("valid base date")
}

#[derive(Debug, Clone)]
struct WindowRows {
    start: NaiveDate,
    end: NaiveDate,
    rows: Vec<(NaiveDate, i64, i64)>,
}

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
    project_dir: &std::path::Path,
) -> Result<(
    Vec<smelt_core::ModelFile>,
    Vec<smelt_core::sources::SourceInfo>,
)> {
    let config = smelt_core::config::Config::load(project_dir)?;
    let discovery = ModelDiscovery::new(project_dir.to_path_buf(), config.paths.clone());
    let models = discovery.discover_models()?;
    let source_infos = discover_source_infos(project_dir, &config.paths);
    Ok((models, source_infos))
}

fn stage_pair_for(
    b: &dyn ConformanceBackend,
    dag: &DagRecipe,
    tmp: &TempDir,
    case: usize,
) -> Result<(LinkCProject, LinkCProject)> {
    let inc_dir = tmp.path().join(format!("inc-{case}"));
    let inc_db = inc_dir.join("unused.duckdb");
    std::fs::create_dir_all(&inc_dir)?;
    let inc = stage_dag_for_target(dag, &inc_dir, &inc_db, b.target(case))?;

    let full_dir = tmp.path().join(format!("full-{case}"));
    let full_db = full_dir.join("unused.duckdb");
    std::fs::create_dir_all(&full_dir)?;
    let full = stage_dag_for_target(dag, &full_dir, &full_db, b.target(case))?;

    Ok((inc, full))
}

async fn assert_every_node_equal_for(
    target: crate::recipe::ConformanceTarget,
    schema: &str,
    dag: &DagRecipe,
    inc: &LinkCProject,
    full: &LinkCProject,
    case: usize,
    context: &str,
) -> Result<()> {
    let inc_backend = inc.backend_for_target(target.clone()).await?;
    let full_backend = full.backend_for_target(target).await?;
    for idx in 0..dag.nodes.len() {
        let inc_rows =
            fetch_node_multiset_via_backend(inc_backend.as_ref(), dag, idx, None, schema)
                .await
                .map_err(|e| anyhow::anyhow!("fetch inc node {idx}: {e}"))?;
        let full_rows =
            fetch_node_multiset_via_backend(full_backend.as_ref(), dag, idx, None, schema)
                .await
                .map_err(|e| anyhow::anyhow!("fetch full node {idx}: {e}"))?;
        anyhow::ensure!(
            inc_rows == full_rows,
            "case {case} ({context}): node {:?} diverged from the full-refresh oracle \
             (incremental left, full-refresh right; {} vs {} rows)",
            dag.nodes[idx].name,
            inc_rows.len(),
            full_rows.len(),
        );
    }
    Ok(())
}

/// `chain_since_upstream_dirty_set_suffices` — the twin of
/// `maintenance_conformance::dags::chain_since_upstream_dirty_set_suffices`.
pub async fn run_chain_since_upstream_dirty_set_suffices(
    b: &dyn ConformanceBackend,
    n: usize,
) -> Result<()> {
    let mut runner = TestRunner::deterministic();
    let strat = (arb_window(0, 2), arb_window(1, 2));

    for i in 0..n {
        let (w1, w2) = strat
            .new_tree(&mut runner)
            .expect("generate case")
            .current();
        let dag = chain_dag();
        let tmp = TempDir::new().expect("tempdir");
        let (inc, full) = stage_pair_for(b, &dag, &tmp, i).expect("stage pair");
        let target = b.target(i);

        let inc_backend = inc
            .backend_for_target(target.clone())
            .await
            .expect("open inc backend");
        b.before_step().await;
        insert_rows_via_backend(inc_backend.as_ref(), &dag, &w1.rows, &b.schema(i))
            .await
            .expect("insert w1 into inc");
        drop(inc_backend);

        b.before_step().await;
        inc.run_with_target(
            target.clone(),
            &format!("{}-chain-{i}-init", b.engine_name()),
            base_request(b.engine_name()),
            &smelt_runtime::NoOpReporter,
        )
        .await
        .unwrap_or_else(|e| panic!("case {i}: initial inc build failed: {e}"));

        let inc_backend = inc
            .backend_for_target(target.clone())
            .await
            .expect("open inc backend");
        b.before_step().await;
        insert_rows_via_backend(inc_backend.as_ref(), &dag, &w2.rows, &b.schema(i))
            .await
            .expect("insert w2 into inc");
        drop(inc_backend);

        let (models, source_infos) = discover(&inc.project_dir).expect("discover inc");
        let order = dag.order();
        let deltas = vec![landed_delta(&dag.source.name, &w2)];
        let plan = plan_since_upstream(&models, &source_infos, &order, &deltas)
            .unwrap_or_else(|e| panic!("case {i}: plan_since_upstream failed: {e}"));
        anyhow::ensure!(
            !plan.runs.is_empty(),
            "case {i}: expected at least dag_chain_a to be scheduled by the landed delta: {}",
            plan.dirty_set_report
        );

        for run in &plan.runs {
            let mut req = base_request(b.engine_name());
            req.start = run.start.clone();
            req.end = run.end.clone();
            b.before_step().await;
            inc.run_with_target(
                target.clone(),
                &format!("{}-chain-{i}-prop-{}", b.engine_name(), run.model),
                req,
                &smelt_runtime::NoOpReporter,
            )
            .await
            .unwrap_or_else(|e| panic!("case {i}: propagated run for {} failed: {e}", run.model));
        }

        let full_backend = full
            .backend_for_target(target.clone())
            .await
            .expect("open full backend");
        b.before_step().await;
        insert_rows_via_backend(full_backend.as_ref(), &dag, &w1.rows, &b.schema(i))
            .await
            .expect("insert w1 into full");
        insert_rows_via_backend(full_backend.as_ref(), &dag, &w2.rows, &b.schema(i))
            .await
            .expect("insert w2 into full");
        drop(full_backend);
        b.before_step().await;
        full.run_with_target(
            target.clone(),
            &format!("{}-chain-{i}-full", b.engine_name()),
            base_request(b.engine_name()),
            &smelt_runtime::NoOpReporter,
        )
        .await
        .unwrap_or_else(|e| panic!("case {i}: full-refresh oracle build failed: {e}"));

        assert_every_node_equal_for(target, &b.schema(i), &dag, &inc, &full, i, "chain")
            .await
            .expect("compare nodes");
    }
    Ok(())
}

/// `diamond_propagation_suffices` — the twin of
/// `maintenance_conformance::dags::diamond_propagation_suffices`.
pub async fn run_diamond_propagation_suffices(b: &dyn ConformanceBackend, n: usize) -> Result<()> {
    let mut runner = TestRunner::deterministic();
    let strat = (arb_window(0, 3), arb_window(1, 3));

    for i in 0..n {
        let (w1, w2) = strat
            .new_tree(&mut runner)
            .expect("generate case")
            .current();
        let dag = diamond_dag();
        let tmp = TempDir::new().expect("tempdir");
        let (inc, full) = stage_pair_for(b, &dag, &tmp, i).expect("stage pair");
        let target = b.target(i);

        let inc_backend = inc
            .backend_for_target(target.clone())
            .await
            .expect("open inc backend");
        b.before_step().await;
        insert_rows_via_backend(inc_backend.as_ref(), &dag, &w1.rows, &b.schema(i))
            .await
            .expect("insert w1 into inc");
        drop(inc_backend);
        b.before_step().await;
        inc.run_with_target(
            target.clone(),
            &format!("{}-diamond-{i}-init", b.engine_name()),
            base_request(b.engine_name()),
            &smelt_runtime::NoOpReporter,
        )
        .await
        .unwrap_or_else(|e| panic!("case {i}: initial inc build failed: {e}"));

        let inc_backend = inc
            .backend_for_target(target.clone())
            .await
            .expect("open inc backend");
        b.before_step().await;
        insert_rows_via_backend(inc_backend.as_ref(), &dag, &w2.rows, &b.schema(i))
            .await
            .expect("insert w2 into inc");
        drop(inc_backend);

        let (models, source_infos) = discover(&inc.project_dir).expect("discover inc");
        let order = dag.order();
        let deltas = vec![landed_delta(&dag.source.name, &w2)];
        let plan = plan_since_upstream(&models, &source_infos, &order, &deltas)
            .unwrap_or_else(|e| panic!("case {i}: plan_since_upstream failed: {e}"));
        anyhow::ensure!(
            !plan.runs.is_empty(),
            "case {i}: expected the landed delta to propagate through the diamond: {}",
            plan.dirty_set_report
        );
        anyhow::ensure!(
            plan.runs.iter().any(|r| r.model == "dag_diamond_a")
                && plan.runs.iter().any(|r| r.model == "dag_diamond_b"),
            "case {i}: expected BOTH diamond branches to be scheduled: {:?}",
            plan.runs
        );

        for run in &plan.runs {
            let mut req = base_request(b.engine_name());
            req.start = run.start.clone();
            req.end = run.end.clone();
            b.before_step().await;
            inc.run_with_target(
                target.clone(),
                &format!("{}-diamond-{i}-prop-{}", b.engine_name(), run.model),
                req,
                &smelt_runtime::NoOpReporter,
            )
            .await
            .unwrap_or_else(|e| panic!("case {i}: propagated run for {} failed: {e}", run.model));
        }

        let full_backend = full
            .backend_for_target(target.clone())
            .await
            .expect("open full backend");
        b.before_step().await;
        insert_rows_via_backend(full_backend.as_ref(), &dag, &w1.rows, &b.schema(i))
            .await
            .expect("insert w1 into full");
        insert_rows_via_backend(full_backend.as_ref(), &dag, &w2.rows, &b.schema(i))
            .await
            .expect("insert w2 into full");
        drop(full_backend);
        b.before_step().await;
        full.run_with_target(
            target.clone(),
            &format!("{}-diamond-{i}-full", b.engine_name()),
            base_request(b.engine_name()),
            &smelt_runtime::NoOpReporter,
        )
        .await
        .unwrap_or_else(|e| panic!("case {i}: full-refresh oracle build failed: {e}"));

        assert_every_node_equal_for(target, &b.schema(i), &dag, &inc, &full, i, "diamond")
            .await
            .expect("compare nodes");
    }
    Ok(())
}

/// `upstream_payload_in_downstream_skeleton_position` — the twin of
/// `maintenance_conformance::dags::upstream_payload_in_downstream_skeleton_position`
/// — the leak family must either refuse loudly or uphold full equivalence
/// under propagation, never silently diverge.
pub async fn run_upstream_payload_in_downstream_skeleton_position(
    b: &dyn ConformanceBackend,
    n: usize,
) -> Result<()> {
    let mut runner = TestRunner::deterministic();
    let strat = (arb_window(0, 2), arb_window(1, 2));

    let mut saw_admitted = false;
    let mut saw_refused = false;

    for i in 0..n {
        let (w1, w2) = strat
            .new_tree(&mut runner)
            .expect("generate case")
            .current();
        let dag = leak_dag();
        let tmp = TempDir::new().expect("tempdir");
        let (inc, full) = stage_pair_for(b, &dag, &tmp, i).expect("stage pair");
        let target = b.target(i);

        let inc_backend = inc
            .backend_for_target(target.clone())
            .await
            .expect("open inc backend");
        b.before_step().await;
        insert_rows_via_backend(inc_backend.as_ref(), &dag, &w1.rows, &b.schema(i))
            .await
            .expect("insert w1 into inc");
        drop(inc_backend);
        b.before_step().await;
        inc.run_with_target(
            target.clone(),
            &format!("{}-leak-{i}-init", b.engine_name()),
            base_request(b.engine_name()),
            &smelt_runtime::NoOpReporter,
        )
        .await
        .unwrap_or_else(|e| panic!("case {i}: initial inc build failed: {e}"));

        match classify_node(&inc, "dag_leak_b") {
            Ok(Verdict::Refused(diags)) => {
                saw_refused = true;
                anyhow::ensure!(
                    !diags.is_empty(),
                    "case {i}: a Refused verdict must carry at least one named diagnostic"
                );
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

        let inc_backend = inc
            .backend_for_target(target.clone())
            .await
            .expect("open inc backend");
        b.before_step().await;
        insert_rows_via_backend(inc_backend.as_ref(), &dag, &w2.rows, &b.schema(i))
            .await
            .expect("insert w2 into inc");
        drop(inc_backend);
        let (models, source_infos) = discover(&inc.project_dir).expect("discover inc");
        let order = dag.order();
        let deltas = vec![landed_delta(&dag.source.name, &w2)];
        let plan = plan_since_upstream(&models, &source_infos, &order, &deltas)
            .unwrap_or_else(|e| panic!("case {i}: plan_since_upstream failed: {e}"));
        for run in &plan.runs {
            let mut req = base_request(b.engine_name());
            req.start = run.start.clone();
            req.end = run.end.clone();
            b.before_step().await;
            inc.run_with_target(
                target.clone(),
                &format!("{}-leak-{i}-prop-{}", b.engine_name(), run.model),
                req,
                &smelt_runtime::NoOpReporter,
            )
            .await
            .unwrap_or_else(|e| panic!("case {i}: propagated run for {} failed: {e}", run.model));
        }

        let full_backend = full
            .backend_for_target(target.clone())
            .await
            .expect("open full backend");
        b.before_step().await;
        insert_rows_via_backend(full_backend.as_ref(), &dag, &w1.rows, &b.schema(i))
            .await
            .expect("insert w1 into full");
        insert_rows_via_backend(full_backend.as_ref(), &dag, &w2.rows, &b.schema(i))
            .await
            .expect("insert w2 into full");
        drop(full_backend);
        b.before_step().await;
        full.run_with_target(
            target.clone(),
            &format!("{}-leak-{i}-full", b.engine_name()),
            base_request(b.engine_name()),
            &smelt_runtime::NoOpReporter,
        )
        .await
        .unwrap_or_else(|e| panic!("case {i}: full-refresh oracle build failed: {e}"));

        assert_every_node_equal_for(target, &b.schema(i), &dag, &inc, &full, i, "leak")
            .await
            .expect("compare nodes");
    }

    anyhow::ensure!(
        saw_admitted || saw_refused,
        "generator health: the deterministic sample never classified dag_leak_b at all"
    );
    Ok(())
}

/// `include_upstreams_resolved_slices_suffice` — the twin of
/// `maintenance_conformance::dags::include_upstreams_resolved_slices_suffice`
/// — staging EXACTLY the backward-resolved slices and building bottom-up
/// over the generated chain yields a target period equal to a build over
/// complete history.
pub async fn run_include_upstreams_resolved_slices_suffice(
    b: &dyn ConformanceBackend,
    n: usize,
) -> Result<()> {
    let mut runner = TestRunner::deterministic();
    let strat = (arb_window(5, 2), arb_window(9, 2));

    for i in 0..n {
        let (period, outside) = strat
            .new_tree(&mut runner)
            .expect("generate case")
            .current();
        let dag = chain_dag();
        let tmp = TempDir::new().expect("tempdir");
        let (partial, full) = stage_pair_for(b, &dag, &tmp, i).expect("stage pair");
        let target = b.target(i);

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
        anyhow::ensure!(
            !resolved.build_order.is_empty(),
            "case {i}: expected a non-empty resolved build order: {}",
            resolved.report
        );
        anyhow::ensure!(
            resolved.build_order[0].model == "dag_chain_a",
            "case {i}: ancestor-first — dag_chain_a must build before dag_chain_b: {}",
            resolved.report
        );

        let partial_backend = partial
            .backend_for_target(target.clone())
            .await
            .expect("open partial backend");
        b.before_step().await;
        insert_rows_via_backend(partial_backend.as_ref(), &dag, &period.rows, &b.schema(i))
            .await
            .expect("insert period rows into partial");
        drop(partial_backend);
        for run in &resolved.build_order {
            let mut req = base_request(b.engine_name());
            req.start = run.start.clone();
            req.end = run.end.clone();
            b.before_step().await;
            partial
                .run_with_target(
                    target.clone(),
                    &format!("{}-resolve-{i}-{}", b.engine_name(), run.model),
                    req,
                    &smelt_runtime::NoOpReporter,
                )
                .await
                .unwrap_or_else(|e| {
                    panic!("case {i}: resolved build for {} failed: {e}", run.model)
                });
        }

        let full_backend = full
            .backend_for_target(target.clone())
            .await
            .expect("open full backend");
        b.before_step().await;
        insert_rows_via_backend(full_backend.as_ref(), &dag, &period.rows, &b.schema(i))
            .await
            .expect("insert period rows into full");
        insert_rows_via_backend(full_backend.as_ref(), &dag, &outside.rows, &b.schema(i))
            .await
            .expect("insert outside rows into full");
        drop(full_backend);
        b.before_step().await;
        full.run_with_target(
            target.clone(),
            &format!("{}-resolve-{i}-full", b.engine_name()),
            base_request(b.engine_name()),
            &smelt_runtime::NoOpReporter,
        )
        .await
        .unwrap_or_else(|e| panic!("case {i}: full-refresh oracle build failed: {e}"));

        let where_clause = format!("d = DATE '{}'", period.start.format("%Y-%m-%d"));
        let partial_backend = partial
            .backend_for_target(target.clone())
            .await
            .expect("open partial backend for read-back");
        let full_backend = full
            .backend_for_target(target.clone())
            .await
            .expect("open full backend for read-back");
        let schema = b.schema(i);
        for idx in 0..dag.nodes.len() {
            let partial_rows = fetch_node_multiset_via_backend(
                partial_backend.as_ref(),
                &dag,
                idx,
                Some(&where_clause),
                &schema,
            )
            .await
            .expect("fetch partial node rows");
            let full_rows = fetch_node_multiset_via_backend(
                full_backend.as_ref(),
                &dag,
                idx,
                Some(&where_clause),
                &schema,
            )
            .await
            .expect("fetch full node rows");
            assert_eq!(
                partial_rows, full_rows,
                "case {i}: node {:?} over the requested period diverged between the \
                 --include-upstreams-resolved partial build and the full-history oracle",
                dag.nodes[idx].name
            );
        }
    }
    Ok(())
}

/// `keyed_grain_node_excluded_from_generated_graph` — the twin of
/// `maintenance_conformance::dags::keyed_grain_node_excluded_from_generated_graph`
/// — `build_forward_graph` is pure static analysis over discovered SQL files
/// (never touches a backend), so this only needs the model staged, never a
/// run.
pub async fn run_keyed_grain_node_excluded_from_generated_graph(
    b: &dyn ConformanceBackend,
) -> Result<()> {
    let dag = keyed_sink_dag();
    let tmp = TempDir::new().expect("tempdir");
    let dir = tmp.path().join("proj");
    let db = dir.join("unused.duckdb");
    std::fs::create_dir_all(&dir).expect("create project dir");
    stage_dag_for_target(&dag, &dir, &db, b.target(0)).expect("stage keyed sink dag");

    let (models, source_infos) = discover(&dir).expect("discover");
    let graph = build_forward_graph(&models, &source_infos).expect("build forward graph");
    anyhow::ensure!(
        !graph.iter().any(|e| e.downstream == "dag_keyed_sink"),
        "a keyed-grain node must never derive a propagation edge in a GENERATED graph: {graph:?}"
    );
    Ok(())
}
