//! Spark twin of `maintenance_conformance/gate.rs`'s `change_feed`-source
//! admission leg (`docs/plans/20260720-prod-w9-spark-conformance-twin.md`
//! Phase 5): `change_feed_source_admits_recompute_only` only —
//! `feed_declared_source_upholds_equivalence_via_recompute` is NOT ported
//! here (see this plan's "Deferred during implementation" entry): it drives
//! genuine incremental runs interleaved with dimension mutations through
//! `feed::apply_feed_step`/`feed::replay_feed`, both hardcoded to a raw
//! `duckdb::Connection`-based change-log replay oracle; porting that
//! replay machinery to a `Backend`-routed form is real, new work beyond
//! this phase's scope (the plan's own "Implementation shape" names the
//! change-feed fixture as "the one genuinely new fixture" this phase adds —
//! the admission leg below is that fixture's classify-only slice; the
//! execution-driven leg's oracle-replay port is tracked as follow-up).
//!
//! This leg needs no execution at all — `feed::stage_feed_keyed_for_target`
//! stages the physical base+feed tables through `Backend::execute_sql`
//! (never a host-filesystem load path), and `classify_keyed_full` (copied
//! from `gate.rs`, target-agnostic) reads the derived plan straight off the
//! staged project's own files.

use proptest::strategy::{Strategy, ValueTree};
use proptest::test_runner::TestRunner;

use smelt_logical::maintenance::{Technique, Trigger};
use smelt_maintenance_testkit::feed;
use smelt_maintenance_testkit::link_c_harness::LinkCProject;
use smelt_maintenance_testkit::recipe::{arb_keyed_combiner, ConformanceTarget, KeyedRecipe};

use crate::gate_spark::spark_connect_url;

/// Default deterministic case count for
/// `change_feed_source_admits_recompute_only_on_spark` —
/// `SMELT_CONFORMANCE_SPARK_FEED_ADMISSION_CASES` env override, smaller than
/// the DuckDB leg's 10.
const DEFAULT_CASES: usize = 6;

fn case_count() -> usize {
    std::env::var("SMELT_CONFORMANCE_SPARK_FEED_ADMISSION_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_CASES)
}

/// Copied from `gate.rs::classify_keyed_full` (target-agnostic: reads the
/// staged project's own files off disk, never touches a backend).
fn classify_keyed_full(
    project: &LinkCProject,
    recipe: &KeyedRecipe,
) -> anyhow::Result<(
    Option<smelt_logical::maintenance::MaintenancePlan>,
    Vec<smelt_db::Diagnostic>,
)> {
    let config = smelt_core::config::Config::load(&project.project_dir)?;
    let discovery =
        smelt_core::ModelDiscovery::new(project.project_dir.clone(), config.paths.clone());
    let sql_models = discovery.discover_models()?;
    let target_path = project
        .project_dir
        .join(format!("models/{}.sql", recipe.model_name));

    let mut db = smelt_db::Database::default();
    let project_input = db.set_project_input(project.project_dir.clone(), String::new());
    let mut target: Option<smelt_db::SourceFile> = None;
    let source_files: Vec<_> = sql_models
        .iter()
        .map(|m| {
            let file = db.set_source_file(
                m.path.clone(),
                m.content.clone(),
                project.project_dir.clone(),
            );
            if m.path == target_path {
                target = Some(file);
            }
            file
        })
        .collect();
    db.set_workspace(source_files, vec![project_input]);
    let workspace = db.workspace();

    let target = target.ok_or_else(|| {
        anyhow::anyhow!(
            "staged feed-driven keyed model {:?} (expected at {}) not found among discovered \
             models",
            recipe.model_name,
            target_path.display()
        )
    })?;
    let diagnostics = smelt_db::file_diagnostics(&db, workspace, target);
    let plan_result = smelt_db::maintenance_plan_report(&db, workspace, target);
    Ok((plan_result.map(|r| r.plan), diagnostics))
}

/// `change_feed_source_admits_recompute_only_on_spark` (plan Phase 5 TDD
/// list): the Spark twin of
/// `maintenance_conformance::gate::change_feed_source_admits_recompute_only`
/// — a `change_feed`-declared source's admitted cells are all full-input
/// re-derivation, never a fold, regardless of target backend.
#[test]
fn change_feed_source_admits_recompute_only_on_spark() {
    let Some(_url) = spark_connect_url() else {
        eprintln!(
            "SPARK_CONNECT_URL unset — skipping \
             change_feed_source_admits_recompute_only_on_spark"
        );
        return;
    };

    let n = case_count();
    let mut runner = TestRunner::deterministic();
    let combiner_strat = arb_keyed_combiner();

    let mut checked = 0;
    for i in 0..n {
        let combiner = combiner_strat.new_tree(&mut runner).unwrap().current();
        let recipe = feed::feed_keyed_recipe(combiner);

        let tmp = tempfile::TempDir::new().expect("tempdir");
        let project_dir = tmp.path().join("project");
        let db_path = tmp.path().join("unused.duckdb");
        std::fs::create_dir_all(&project_dir).expect("create project dir");
        let project = feed::stage_feed_keyed_for_target(
            &recipe,
            &project_dir,
            &db_path,
            ConformanceTarget::SparkDelta,
        )
        .unwrap_or_else(|e| {
            panic!("case {i}: failed to stage feed-driven keyed recipe on Spark: {e}")
        });

        let (plan, diags) = classify_keyed_full(&project, &recipe)
            .unwrap_or_else(|e| panic!("case {i}: classify failed on Spark: {e}"));
        let plan = plan.unwrap_or_else(|| {
            panic!("case {i}: no maintenance plan returned at all on Spark: diagnostics={diags:#?}")
        });

        assert!(
            !plan.cells.is_empty(),
            "case {i}: change_feed-driven keyed recipe {recipe:?} admitted zero cells on Spark \
             (expected at least the universal Backfill recompute cell): diagnostics={diags:#?}"
        );
        for cell in &plan.cells {
            assert_eq!(
                cell.technique,
                Technique::DeleteInsert,
                "case {i}: a change_feed-declared source must admit ONLY full-input \
                 re-derivation (Technique::DeleteInsert) on Spark too, never a fold — got \
                 {:?} for cell {cell:?}",
                cell.technique,
            );
        }
        assert!(
            !plan.cells.iter().any(|c| matches!(
                &c.trigger,
                Trigger::NewData { source } if source == &recipe.source.name
            )),
            "case {i}: a change_feed source must never admit a targeted NewData fold cell on \
             Spark either: {plan:#?}"
        );
        checked += 1;
    }

    assert!(
        checked > 0,
        "N={n} deterministic sample never staged a change_feed-driven keyed recipe on Spark — \
         generator/derivation regression"
    );
}
