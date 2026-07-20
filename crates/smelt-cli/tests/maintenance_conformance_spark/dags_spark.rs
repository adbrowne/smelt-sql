//! Spark twin of `maintenance_conformance/dags.rs`
//! (`docs/plans/20260720-prod-w9-spark-conformance-twin.md` Phase 5):
//! the same generated 2-3 node DAGs (chain/diamond/leak/keyed-sink), driven
//! end-to-end through the real `smelt_runtime::execute_project` pipeline
//! against a live Spark/Delta backend instead of DuckDB — staging, row
//! insertion, and read-back all route through `smelt_backend::Backend`
//! (`dag::stage_dag_for_target`/`insert_rows_via_backend`/
//! `fetch_node_multiset_via_backend`), never a raw host connection.

use chrono::{Datelike, NaiveDate};
use proptest::strategy::{Strategy, ValueTree};
use proptest::test_runner::TestRunner;
use tempfile::TempDir;

use smelt_core::{discover_source_infos, ModelDiscovery};
use smelt_logical::maintenance::propagate::{day_ordinal, DayInterval};
use smelt_maintenance_testkit::dag::{
    chain_dag, classify_node, diamond_dag, fetch_node_multiset_via_backend,
    insert_rows_via_backend, keyed_sink_dag, leak_dag, stage_dag_for_target, DagRecipe,
};
use smelt_maintenance_testkit::link_c_harness::base_request;
use smelt_maintenance_testkit::recipe::{arb_payload_value, ConformanceTarget};
use smelt_maintenance_testkit::verdict::Verdict;
use smelt_runtime::propagation::{
    build_forward_graph, plan_since_upstream, resolve_build_plan, SourceDelta,
};

use crate::gate_spark::spark_connect_url;

/// Default deterministic case count — smaller than the DuckDB leg's default
/// (6): each Spark case stages TWO independent projects and drives
/// `execute_project` multiple times each over a live Spark Connect server.
const DEFAULT_CASES: usize = 3;

fn case_count() -> usize {
    std::env::var("SMELT_CONFORMANCE_SPARK_DAG_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_CASES)
}

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

fn stage_pair_spark(
    dag: &DagRecipe,
    tmp: &TempDir,
    case: usize,
) -> anyhow::Result<(
    smelt_maintenance_testkit::link_c_harness::LinkCProject,
    smelt_maintenance_testkit::link_c_harness::LinkCProject,
)> {
    let inc_dir = tmp.path().join(format!("inc-{case}"));
    let inc_db = inc_dir.join("unused.duckdb");
    std::fs::create_dir_all(&inc_dir)?;
    let inc = stage_dag_for_target(dag, &inc_dir, &inc_db, ConformanceTarget::SparkDelta)?;

    let full_dir = tmp.path().join(format!("full-{case}"));
    let full_db = full_dir.join("unused.duckdb");
    std::fs::create_dir_all(&full_dir)?;
    let full = stage_dag_for_target(dag, &full_dir, &full_db, ConformanceTarget::SparkDelta)?;

    Ok((inc, full))
}

async fn assert_every_node_equal_spark(
    dag: &DagRecipe,
    inc: &smelt_maintenance_testkit::link_c_harness::LinkCProject,
    full: &smelt_maintenance_testkit::link_c_harness::LinkCProject,
    case: usize,
    context: &str,
) -> anyhow::Result<()> {
    let inc_backend = inc
        .backend_for_target(ConformanceTarget::SparkDelta)
        .await?;
    let full_backend = full
        .backend_for_target(ConformanceTarget::SparkDelta)
        .await?;
    for idx in 0..dag.nodes.len() {
        let inc_rows = fetch_node_multiset_via_backend(inc_backend.as_ref(), dag, idx, None)
            .await
            .map_err(|e| anyhow::anyhow!("fetch inc node {idx} on Spark: {e}"))?;
        let full_rows = fetch_node_multiset_via_backend(full_backend.as_ref(), dag, idx, None)
            .await
            .map_err(|e| anyhow::anyhow!("fetch full node {idx} on Spark: {e}"))?;
        anyhow::ensure!(
            inc_rows == full_rows,
            "case {case} ({context}): node {:?} diverged from the full-refresh oracle on Spark \
             (incremental left, full-refresh right; {} vs {} rows)",
            dag.nodes[idx].name,
            inc_rows.len(),
            full_rows.len(),
        );
    }
    Ok(())
}

/// `chain_since_upstream_dirty_set_suffices_on_spark` (plan Phase 5 TDD
/// list): the Spark twin of
/// `maintenance_conformance::dags::chain_since_upstream_dirty_set_suffices`.
#[test]
fn chain_since_upstream_dirty_set_suffices_on_spark() {
    let Some(_url) = spark_connect_url() else {
        eprintln!(
            "SPARK_CONNECT_URL unset — skipping \
             chain_since_upstream_dirty_set_suffices_on_spark"
        );
        return;
    };

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
        let (inc, full) = stage_pair_spark(&dag, &tmp, i).expect("stage pair on Spark");

        rt.block_on(async {
            let inc_backend = inc
                .backend_for_target(ConformanceTarget::SparkDelta)
                .await
                .expect("open inc backend");
            insert_rows_via_backend(inc_backend.as_ref(), &dag, &w1.rows)
                .await
                .expect("insert w1 into inc");
        });
        rt.block_on(inc.run_with_target(
            ConformanceTarget::SparkDelta,
            &format!("spark-chain-{i}-init"),
            base_request("spark"),
            &smelt_runtime::NoOpReporter,
        ))
        .unwrap_or_else(|e| panic!("case {i}: initial inc Spark build failed: {e}"));

        rt.block_on(async {
            let inc_backend = inc
                .backend_for_target(ConformanceTarget::SparkDelta)
                .await
                .expect("open inc backend");
            insert_rows_via_backend(inc_backend.as_ref(), &dag, &w2.rows)
                .await
                .expect("insert w2 into inc");
        });

        let (models, source_infos) = discover(&inc.project_dir).expect("discover inc");
        let order = dag.order();
        let deltas = vec![landed_delta(&dag.source.name, &w2)];
        let plan = plan_since_upstream(&models, &source_infos, &order, &deltas)
            .unwrap_or_else(|e| panic!("case {i}: plan_since_upstream failed: {e}"));
        assert!(
            !plan.runs.is_empty(),
            "case {i}: expected at least dag_chain_a to be scheduled by the landed delta: {}",
            plan.dirty_set_report
        );

        for run in &plan.runs {
            let mut req = base_request("spark");
            req.start = run.start.clone();
            req.end = run.end.clone();
            rt.block_on(inc.run_with_target(
                ConformanceTarget::SparkDelta,
                &format!("spark-chain-{i}-prop-{}", run.model),
                req,
                &smelt_runtime::NoOpReporter,
            ))
            .unwrap_or_else(|e| {
                panic!(
                    "case {i}: propagated Spark run for {} failed: {e}",
                    run.model
                )
            });
        }

        rt.block_on(async {
            let full_backend = full
                .backend_for_target(ConformanceTarget::SparkDelta)
                .await
                .expect("open full backend");
            insert_rows_via_backend(full_backend.as_ref(), &dag, &w1.rows)
                .await
                .expect("insert w1 into full");
            insert_rows_via_backend(full_backend.as_ref(), &dag, &w2.rows)
                .await
                .expect("insert w2 into full");
        });
        rt.block_on(full.run_with_target(
            ConformanceTarget::SparkDelta,
            &format!("spark-chain-{i}-full"),
            base_request("spark"),
            &smelt_runtime::NoOpReporter,
        ))
        .unwrap_or_else(|e| panic!("case {i}: full-refresh oracle Spark build failed: {e}"));

        rt.block_on(assert_every_node_equal_spark(&dag, &inc, &full, i, "chain"))
            .expect("compare nodes on Spark");
    }
}

/// `diamond_propagation_suffices_on_spark` (plan Phase 5 TDD list): the Spark
/// twin of `maintenance_conformance::dags::diamond_propagation_suffices`.
#[test]
fn diamond_propagation_suffices_on_spark() {
    let Some(_url) = spark_connect_url() else {
        eprintln!("SPARK_CONNECT_URL unset — skipping diamond_propagation_suffices_on_spark");
        return;
    };

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
        let (inc, full) = stage_pair_spark(&dag, &tmp, i).expect("stage pair on Spark");

        rt.block_on(async {
            let inc_backend = inc
                .backend_for_target(ConformanceTarget::SparkDelta)
                .await
                .expect("open inc backend");
            insert_rows_via_backend(inc_backend.as_ref(), &dag, &w1.rows)
                .await
                .expect("insert w1 into inc");
        });
        rt.block_on(inc.run_with_target(
            ConformanceTarget::SparkDelta,
            &format!("spark-diamond-{i}-init"),
            base_request("spark"),
            &smelt_runtime::NoOpReporter,
        ))
        .unwrap_or_else(|e| panic!("case {i}: initial inc Spark build failed: {e}"));

        rt.block_on(async {
            let inc_backend = inc
                .backend_for_target(ConformanceTarget::SparkDelta)
                .await
                .expect("open inc backend");
            insert_rows_via_backend(inc_backend.as_ref(), &dag, &w2.rows)
                .await
                .expect("insert w2 into inc");
        });

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
        assert!(
            plan.runs.iter().any(|r| r.model == "dag_diamond_a")
                && plan.runs.iter().any(|r| r.model == "dag_diamond_b"),
            "case {i}: expected BOTH diamond branches to be scheduled: {:?}",
            plan.runs
        );

        for run in &plan.runs {
            let mut req = base_request("spark");
            req.start = run.start.clone();
            req.end = run.end.clone();
            rt.block_on(inc.run_with_target(
                ConformanceTarget::SparkDelta,
                &format!("spark-diamond-{i}-prop-{}", run.model),
                req,
                &smelt_runtime::NoOpReporter,
            ))
            .unwrap_or_else(|e| {
                panic!(
                    "case {i}: propagated Spark run for {} failed: {e}",
                    run.model
                )
            });
        }

        rt.block_on(async {
            let full_backend = full
                .backend_for_target(ConformanceTarget::SparkDelta)
                .await
                .expect("open full backend");
            insert_rows_via_backend(full_backend.as_ref(), &dag, &w1.rows)
                .await
                .expect("insert w1 into full");
            insert_rows_via_backend(full_backend.as_ref(), &dag, &w2.rows)
                .await
                .expect("insert w2 into full");
        });
        rt.block_on(full.run_with_target(
            ConformanceTarget::SparkDelta,
            &format!("spark-diamond-{i}-full"),
            base_request("spark"),
            &smelt_runtime::NoOpReporter,
        ))
        .unwrap_or_else(|e| panic!("case {i}: full-refresh oracle Spark build failed: {e}"));

        rt.block_on(assert_every_node_equal_spark(
            &dag, &inc, &full, i, "diamond",
        ))
        .expect("compare nodes on Spark");
    }
}

/// `upstream_payload_in_downstream_skeleton_position_on_spark` (plan Phase 5
/// TDD list): the Spark twin of
/// `maintenance_conformance::dags::upstream_payload_in_downstream_skeleton_position`
/// — the leak family must either refuse loudly or uphold full equivalence
/// under propagation, never silently diverge, on Spark too.
#[test]
fn upstream_payload_in_downstream_skeleton_position_on_spark() {
    let Some(_url) = spark_connect_url() else {
        eprintln!(
            "SPARK_CONNECT_URL unset — skipping \
             upstream_payload_in_downstream_skeleton_position_on_spark"
        );
        return;
    };

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
        let (inc, full) = stage_pair_spark(&dag, &tmp, i).expect("stage pair on Spark");

        rt.block_on(async {
            let inc_backend = inc
                .backend_for_target(ConformanceTarget::SparkDelta)
                .await
                .expect("open inc backend");
            insert_rows_via_backend(inc_backend.as_ref(), &dag, &w1.rows)
                .await
                .expect("insert w1 into inc");
        });
        rt.block_on(inc.run_with_target(
            ConformanceTarget::SparkDelta,
            &format!("spark-leak-{i}-init"),
            base_request("spark"),
            &smelt_runtime::NoOpReporter,
        ))
        .unwrap_or_else(|e| panic!("case {i}: initial inc Spark build failed: {e}"));

        match classify_node(&inc, "dag_leak_b") {
            Ok(Verdict::Refused(diags)) => {
                saw_refused = true;
                assert!(
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

        rt.block_on(async {
            let inc_backend = inc
                .backend_for_target(ConformanceTarget::SparkDelta)
                .await
                .expect("open inc backend");
            insert_rows_via_backend(inc_backend.as_ref(), &dag, &w2.rows)
                .await
                .expect("insert w2 into inc");
        });
        let (models, source_infos) = discover(&inc.project_dir).expect("discover inc");
        let order = dag.order();
        let deltas = vec![landed_delta(&dag.source.name, &w2)];
        let plan = plan_since_upstream(&models, &source_infos, &order, &deltas)
            .unwrap_or_else(|e| panic!("case {i}: plan_since_upstream failed: {e}"));
        for run in &plan.runs {
            let mut req = base_request("spark");
            req.start = run.start.clone();
            req.end = run.end.clone();
            rt.block_on(inc.run_with_target(
                ConformanceTarget::SparkDelta,
                &format!("spark-leak-{i}-prop-{}", run.model),
                req,
                &smelt_runtime::NoOpReporter,
            ))
            .unwrap_or_else(|e| {
                panic!(
                    "case {i}: propagated Spark run for {} failed: {e}",
                    run.model
                )
            });
        }

        rt.block_on(async {
            let full_backend = full
                .backend_for_target(ConformanceTarget::SparkDelta)
                .await
                .expect("open full backend");
            insert_rows_via_backend(full_backend.as_ref(), &dag, &w1.rows)
                .await
                .expect("insert w1 into full");
            insert_rows_via_backend(full_backend.as_ref(), &dag, &w2.rows)
                .await
                .expect("insert w2 into full");
        });
        rt.block_on(full.run_with_target(
            ConformanceTarget::SparkDelta,
            &format!("spark-leak-{i}-full"),
            base_request("spark"),
            &smelt_runtime::NoOpReporter,
        ))
        .unwrap_or_else(|e| panic!("case {i}: full-refresh oracle Spark build failed: {e}"));

        rt.block_on(assert_every_node_equal_spark(&dag, &inc, &full, i, "leak"))
            .expect("compare nodes on Spark");
    }

    assert!(
        saw_admitted || saw_refused,
        "generator health: the deterministic sample never classified dag_leak_b at all on Spark"
    );
}

/// `include_upstreams_resolved_slices_suffice_on_spark` (plan Phase 5 TDD
/// list): the Spark twin of
/// `maintenance_conformance::dags::include_upstreams_resolved_slices_suffice`
/// — staging EXACTLY the backward-resolved slices and building bottom-up
/// over the generated chain yields a target period equal to a build over
/// complete history, on Spark too.
#[test]
fn include_upstreams_resolved_slices_suffice_on_spark() {
    let Some(_url) = spark_connect_url() else {
        eprintln!(
            "SPARK_CONNECT_URL unset — skipping \
             include_upstreams_resolved_slices_suffice_on_spark"
        );
        return;
    };

    let mut runner = TestRunner::deterministic();
    let strat = (arb_window(5, 2), arb_window(9, 2));
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");

    for i in 0..case_count() {
        let (period, outside) = strat
            .new_tree(&mut runner)
            .expect("generate case")
            .current();
        let dag = chain_dag();
        let tmp = TempDir::new().expect("tempdir");
        let (partial, full) = stage_pair_spark(&dag, &tmp, i).expect("stage pair on Spark");

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

        rt.block_on(async {
            let partial_backend = partial
                .backend_for_target(ConformanceTarget::SparkDelta)
                .await
                .expect("open partial backend");
            insert_rows_via_backend(partial_backend.as_ref(), &dag, &period.rows)
                .await
                .expect("insert period rows into partial");
        });
        for run in &resolved.build_order {
            let mut req = base_request("spark");
            req.start = run.start.clone();
            req.end = run.end.clone();
            rt.block_on(partial.run_with_target(
                ConformanceTarget::SparkDelta,
                &format!("spark-resolve-{i}-{}", run.model),
                req,
                &smelt_runtime::NoOpReporter,
            ))
            .unwrap_or_else(|e| {
                panic!(
                    "case {i}: resolved Spark build for {} failed: {e}",
                    run.model
                )
            });
        }

        rt.block_on(async {
            let full_backend = full
                .backend_for_target(ConformanceTarget::SparkDelta)
                .await
                .expect("open full backend");
            insert_rows_via_backend(full_backend.as_ref(), &dag, &period.rows)
                .await
                .expect("insert period rows into full");
            insert_rows_via_backend(full_backend.as_ref(), &dag, &outside.rows)
                .await
                .expect("insert outside rows into full");
        });
        rt.block_on(full.run_with_target(
            ConformanceTarget::SparkDelta,
            &format!("spark-resolve-{i}-full"),
            base_request("spark"),
            &smelt_runtime::NoOpReporter,
        ))
        .unwrap_or_else(|e| panic!("case {i}: full-refresh oracle Spark build failed: {e}"));

        let where_clause = format!("d = DATE '{}'", period.start.format("%Y-%m-%d"));
        rt.block_on(async {
            let partial_backend = partial
                .backend_for_target(ConformanceTarget::SparkDelta)
                .await
                .expect("open partial backend for read-back");
            let full_backend = full
                .backend_for_target(ConformanceTarget::SparkDelta)
                .await
                .expect("open full backend for read-back");
            for idx in 0..dag.nodes.len() {
                let partial_rows = fetch_node_multiset_via_backend(
                    partial_backend.as_ref(),
                    &dag,
                    idx,
                    Some(&where_clause),
                )
                .await
                .expect("fetch partial node rows on Spark");
                let full_rows = fetch_node_multiset_via_backend(
                    full_backend.as_ref(),
                    &dag,
                    idx,
                    Some(&where_clause),
                )
                .await
                .expect("fetch full node rows on Spark");
                assert_eq!(
                    partial_rows, full_rows,
                    "case {i}: node {:?} over the requested period diverged between the \
                     --include-upstreams-resolved partial Spark build and the full-history \
                     oracle",
                    dag.nodes[idx].name
                );
            }
        });
    }
}

/// `keyed_grain_node_excluded_from_generated_graph_on_spark` (plan Phase 5
/// TDD list): the Spark twin of
/// `maintenance_conformance::dags::keyed_grain_node_excluded_from_generated_graph`
/// — `build_forward_graph` is pure static analysis over discovered SQL files
/// (never touches a backend), so this only needs the model staged under a
/// Spark-targeted `smelt.yml`, never a Spark run.
#[test]
fn keyed_grain_node_excluded_from_generated_graph_on_spark() {
    let Some(_url) = spark_connect_url() else {
        eprintln!(
            "SPARK_CONNECT_URL unset — skipping \
             keyed_grain_node_excluded_from_generated_graph_on_spark"
        );
        return;
    };

    let dag = keyed_sink_dag();
    let tmp = TempDir::new().expect("tempdir");
    let dir = tmp.path().join("proj");
    let db = dir.join("unused.duckdb");
    std::fs::create_dir_all(&dir).expect("create project dir");
    stage_dag_for_target(&dag, &dir, &db, ConformanceTarget::SparkDelta)
        .expect("stage keyed sink dag on Spark");

    let (models, source_infos) = discover(&dir).expect("discover");
    let graph = build_forward_graph(&models, &source_infos).expect("build forward graph");
    assert!(
        !graph.iter().any(|e| e.downstream == "dag_keyed_sink"),
        "a keyed-grain node must never derive a propagation edge in a GENERATED graph: {graph:?}"
    );
}
