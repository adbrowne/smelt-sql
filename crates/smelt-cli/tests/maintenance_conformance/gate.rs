//! The standing proptest gate over the append-only partition-grain
//! `ModelRecipe` pool
//! (`docs/plans/20260712-generative-maintenance-conformance.md` Phase 3).

use proptest::strategy::{Strategy, ValueTree};
use proptest::test_runner::TestRunner;

use smelt_maintenance_testkit::link_c_harness::{base_request, LinkCProject};
use smelt_maintenance_testkit::oracle::multiset_equal;
use smelt_maintenance_testkit::recipe::{arb_recipe, ModelRecipe, RecipePool};
use smelt_maintenance_testkit::render;
use smelt_maintenance_testkit::s_tracker::STracker;
use smelt_maintenance_testkit::schedule_gen::{
    arb_schedule_for, read_source_snapshot, ConformanceSchedule, ConformanceStep,
};
use smelt_maintenance_testkit::verdict::{classify, Verdict};

/// Default deterministic case count for
/// `append_only_partition_pool_upholds_equivalence` — small enough to stay
/// on par with `property_discovery`'s per-target budget (plan Phase 3
/// Implementation shape: "each case ≈ 1-3s … 12 cases keeps the target on
/// par").
const DEFAULT_CASES: usize = 12;

fn case_count() -> usize {
    std::env::var("SMELT_CONFORMANCE_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_CASES)
}

/// Stage `recipe` into a fresh temp project + DuckDB file, returning the
/// loaded [`LinkCProject`].
pub fn stage_recipe(recipe: &ModelRecipe, tmp: &tempfile::TempDir) -> anyhow::Result<LinkCProject> {
    let project_dir = tmp.path().join("project");
    let db_path = tmp.path().join("db.duckdb");
    std::fs::create_dir_all(&project_dir)?;
    render::stage(recipe, &project_dir, &db_path)
}

fn insert_row(
    project: &LinkCProject,
    recipe: &ModelRecipe,
    row: &smelt_maintenance_testkit::schedule_gen::GenRow,
) -> anyhow::Result<()> {
    let conn = project.connect()?;
    conn.execute(
        &format!(
            "INSERT INTO main.sources_{} VALUES (DATE '{}', {}, {})",
            recipe.source.name,
            row.d.format("%Y-%m-%d"),
            row.id,
            row.val,
        ),
        [],
    )?;
    Ok(())
}

/// Drive `schedule` against `project`/`recipe` through the real
/// `execute_project` pipeline, asserting S-restricted multiset equivalence
/// after every `RunWindow` step (design §6). Returns the fully-populated
/// [`STracker`] plus the index of the last recorded run, so
/// `harness_self_check` can reuse the green end-state without re-driving
/// the whole schedule.
pub async fn drive_and_assert(
    project: &LinkCProject,
    recipe: &ModelRecipe,
    schedule: &ConformanceSchedule,
) -> anyhow::Result<(STracker, usize)> {
    let mut tracker = STracker::new(&recipe.source);
    let mut last_k: Option<usize> = None;

    for (i, step) in schedule.0.iter().enumerate() {
        match step {
            ConformanceStep::RunWindow { start, end, rows } => {
                for row in rows {
                    insert_row(project, recipe, row)?;
                }

                let snapshot = {
                    let conn = project.connect()?;
                    read_source_snapshot(&conn, &recipe.source)
                };

                let mut request = base_request("dev");
                request.start = Some(start.format("%Y-%m-%d").to_string());
                request.end = Some(end.format("%Y-%m-%d").to_string());
                project.run_quiet(&format!("run-{i}"), request).await?;

                let k = tracker.record_run(*start, *end, snapshot);
                last_k = Some(k);

                assert_equivalence(project, recipe, &tracker, k)?;
            }
            ConformanceStep::AppendLateRow(row) => {
                insert_row(project, recipe, row)?;
            }
        }
    }

    let last_k =
        last_k.ok_or_else(|| anyhow::anyhow!("schedule {schedule:?} had no RunWindow step"))?;
    Ok((tracker, last_k))
}

/// The S-restricted oracle assertion (design §6): materialize `S_k`, then
/// `EXCEPT ALL`-compare it (both directions) against the maintained output
/// table `main.<model_name>`.
pub fn assert_equivalence(
    project: &LinkCProject,
    recipe: &ModelRecipe,
    tracker: &STracker,
    k: usize,
) -> anyhow::Result<()> {
    let conn = project.connect()?;
    tracker.materialize_s(&conn, k)?;
    let maintained_sql = format!("SELECT * FROM main.{}", recipe.model_name);
    let oracle_sql = tracker.s_restricted_oracle_sql(recipe);
    let equal = multiset_equal(&conn, &maintained_sql, &oracle_sql);
    if !equal {
        anyhow::bail!(
            "S-restricted equivalence violated for model {:?} at run {k}: \
             maintained ({maintained_sql:?}) != oracle ({oracle_sql:?})",
            recipe.model_name
        );
    }
    Ok(())
}

/// `append_only_partition_pool_upholds_equivalence` (plan Phase 3 TDD
/// list): the standing proptest gate — deterministic seed, small N (default
/// [`DEFAULT_CASES`], `SMELT_CONFORMANCE_CASES` env override). Each case:
/// generate recipe → classify → if admitted, generate schedule → drive real
/// `execute_project` per step → assert S-restricted multiset equivalence
/// after every run step.
#[test]
fn append_only_partition_pool_upholds_equivalence() {
    let n = case_count();
    let mut runner = TestRunner::deterministic();
    let recipe_strat = arb_recipe(RecipePool::partition_append_only());

    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let mut admitted_cases = 0;

    for i in 0..n {
        let recipe = recipe_strat.new_tree(&mut runner).unwrap().current();
        let schedule = arb_schedule_for(&recipe)
            .new_tree(&mut runner)
            .unwrap()
            .current();

        let tmp = tempfile::TempDir::new().expect("tempdir");
        let project = stage_recipe(&recipe, &tmp)
            .unwrap_or_else(|e| panic!("case {i}: recipe {recipe:?} failed to stage: {e}"));

        let verdict = classify(&project, &recipe)
            .unwrap_or_else(|e| panic!("case {i}: recipe {recipe:?} classify failed: {e}"));

        match verdict {
            Verdict::Refused(_) => continue,
            Verdict::Admitted(_) => {
                admitted_cases += 1;
                rt.block_on(drive_and_assert(&project, &recipe, &schedule))
                    .unwrap_or_else(|e| {
                        panic!(
                            "case {i}: recipe {recipe:?} schedule {schedule:?} \
                             equivalence check failed: {e}"
                        )
                    });
            }
        }
    }

    assert!(
        admitted_cases > 0,
        "N={n} deterministic sample admitted zero cases — generator/derivation regression"
    );
}

/// `admission_rate_stays_above_floor` (plan Phase 3 TDD list): generator
/// health — over the deterministic sample, at least 40% of (non-adversarial)
/// recipes admit at least one cell.
#[test]
fn admission_rate_stays_above_floor() {
    const N: usize = 50;
    let mut runner = TestRunner::deterministic();
    let recipe_strat = arb_recipe(RecipePool::partition_append_only());

    let mut admitted = 0;
    for i in 0..N {
        let recipe = recipe_strat.new_tree(&mut runner).unwrap().current();
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let project = stage_recipe(&recipe, &tmp)
            .unwrap_or_else(|e| panic!("case {i}: recipe {recipe:?} failed to stage: {e}"));
        let verdict = classify(&project, &recipe)
            .unwrap_or_else(|e| panic!("case {i}: recipe {recipe:?} classify failed: {e}"));
        if matches!(verdict, Verdict::Admitted(_)) {
            admitted += 1;
        }
    }

    let rate = admitted as f64 / N as f64;
    assert!(
        rate >= 0.40,
        "admission rate {rate:.2} over N={N} fell below the 40% generator-health floor \
         ({admitted}/{N} admitted)"
    );
}
