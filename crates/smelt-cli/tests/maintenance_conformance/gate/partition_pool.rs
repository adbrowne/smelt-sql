//! The append-only partition-grain `ModelRecipe` pool: staging, the S-restricted oracle assertions (including the contract-lattice points), the schedule driver, and the standing proptest gate over it.

use chrono::Datelike;
use proptest::strategy::{Strategy, ValueTree};
use proptest::test_runner::TestRunner;

use smelt_logical::contract::{oracle_obligation, ContractPoint, OracleObligation};
use smelt_maintenance_testkit::link_c_harness::{base_request, LinkCProject};
use smelt_maintenance_testkit::migrate_step::{run_migrate_step, MigrateStepOutcome};
use smelt_maintenance_testkit::oracle::multiset_equal_via_backend;
use smelt_maintenance_testkit::recipe::{arb_recipe, ModelEdit, ModelRecipe, RecipePool};
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
pub(crate) const DEFAULT_CASES: usize = 12;

pub(crate) fn case_count() -> usize {
    std::env::var("SMELT_CONFORMANCE_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_CASES)
}

/// Stage `recipe` into a fresh temp project + DuckDB file, returning the
/// loaded [`LinkCProject`].
pub(crate) fn stage_recipe(
    recipe: &ModelRecipe,
    tmp: &tempfile::TempDir,
) -> anyhow::Result<LinkCProject> {
    let project_dir = tmp.path().join("project");
    let db_path = tmp.path().join("db.duckdb");
    std::fs::create_dir_all(&project_dir)?;
    render::stage(recipe, &project_dir, &db_path)
}

/// Routed through [`smelt_backend::Backend::execute_sql`] rather than a raw
/// `duckdb::Connection`
/// (`docs/plans/20260720-prod-w9-spark-conformance-twin.md` Phase 2) — same
/// `INSERT` text, only the execution channel moved (this helper is private
/// to this file, so the conversion has no blast radius on the sibling test
/// files in this directory that also call into `gate.rs`'s *public*
/// surface).
pub(crate) async fn insert_row(
    project: &LinkCProject,
    recipe: &ModelRecipe,
    row: &smelt_maintenance_testkit::schedule_gen::GenRow,
) -> anyhow::Result<()> {
    let backend = project.backend().await?;
    backend
        .execute_sql(&format!(
            "INSERT INTO main.sources_{} VALUES (DATE '{}', {}, {})",
            recipe.source.name,
            row.d.format("%Y-%m-%d"),
            row.id,
            row.val_sql(),
        ))
        .await
        .map_err(|e| anyhow::anyhow!("insert row: {e}"))?;
    Ok(())
}

/// Drive `schedule` against `project`/`recipe` through the real
/// `execute_project` pipeline, asserting S-restricted multiset equivalence
/// after every `RunWindow` step (design §6). Returns the fully-populated
/// [`STracker`] plus the index of the last recorded run, so
/// `harness_self_check` can reuse the green end-state without re-driving
/// the whole schedule.
pub(crate) async fn drive_and_assert(
    project: &LinkCProject,
    recipe: &ModelRecipe,
    schedule: &ConformanceSchedule,
) -> anyhow::Result<(STracker, usize)> {
    let mut migrate_outcomes = Vec::new();
    drive_and_assert_collecting(project, recipe, schedule, &mut migrate_outcomes).await
}

/// [`drive_and_assert`], additionally recording every
/// [`ConformanceStep::MigrateModel`] step's [`MigrateStepOutcome`] into
/// `migrate_outcomes` — the generative gate
/// (`definition_edit_pool_upholds_new_definition_equivalence`) needs to know
/// whether at least one case in its deterministic sample actually took the
/// `Applied` leg, mirroring `admission_rate_stays_above_floor`'s
/// anti-vacuity discipline; `drive_and_assert` itself just discards the
/// vec so its 9 existing call sites need no change.
pub(crate) async fn drive_and_assert_collecting(
    project: &LinkCProject,
    recipe: &ModelRecipe,
    schedule: &ConformanceSchedule,
    migrate_outcomes: &mut Vec<MigrateStepOutcome>,
) -> anyhow::Result<(STracker, usize)> {
    let mut tracker = STracker::new(&recipe.source);
    let mut last_k: Option<usize> = None;
    // The most recent `RewriteModel` step's edit (Phase 9), or `None` before
    // any rewrite — threads into every subsequent assertion so the oracle
    // re-renders against the CURRENT on-disk body, never the pre-rewrite one
    // (plan Phase 9 review checklist).
    let mut current_edit: Option<ModelEdit> = None;

    for (i, step) in schedule.0.iter().enumerate() {
        match step {
            ConformanceStep::RunWindow { start, end, rows } => {
                for row in rows {
                    insert_row(project, recipe, row).await?;
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

                assert_equivalence_with_edit(project, recipe, &tracker, k, current_edit).await?;
            }
            ConformanceStep::AppendLateRow(row) => {
                insert_row(project, recipe, row).await?;
            }
            ConformanceStep::RerunWindow { start, end } => {
                // Redelivery: same window as an earlier `RunWindow`, no new
                // rows. Never-fold-twice under the partition-grain
                // DELETE+INSERT full-replace technique must hold (Phase 6).
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

                assert_equivalence_with_edit(project, recipe, &tracker, k, current_edit).await?;
            }
            ConformanceStep::FullRefreshRun => {
                // Unwindowed run: `execute_project` takes the full-refresh
                // arm (drop + rebuild from the CURRENT full source
                // contents) whenever no `start`/`end` is supplied (Phase 6).
                let snapshot = {
                    let conn = project.connect()?;
                    read_source_snapshot(&conn, &recipe.source)
                };

                let mut request = base_request("dev");
                request.full_refresh = true;
                request.start = None;
                request.end = None;
                project.run_quiet(&format!("run-{i}"), request).await?;

                let k = tracker.record_full_refresh(snapshot);
                last_k = Some(k);

                assert_equivalence_with_edit(project, recipe, &tracker, k, current_edit).await?;
            }
            ConformanceStep::BackfillRegion { start, end } => {
                // An explicit backfill: same execution shape as `RunWindow`
                // with no accompanying insert (Phase 6).
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

                assert_equivalence_with_edit(project, recipe, &tracker, k, current_edit).await?;
            }
            ConformanceStep::RewriteModel { edit } => {
                // Definition change (Phase 9): rewrite the model file on
                // disk with `edit` applied. No run happens here — the next
                // *Window step re-discovers the model from disk (already
                // the harness's behaviour) and compiles/executes whatever
                // SQL is now on disk.
                let model_path = project
                    .project_dir
                    .join(format!("models/{}.sql", recipe.model_name));
                std::fs::write(
                    &model_path,
                    render::render_model_file_with_edit(recipe, *edit),
                )?;
                current_edit = Some(*edit);
            }
            ConformanceStep::MigrateModel { edit } => {
                // Definition change routed through the shipped `smelt
                // migrate` derive→apply path (task 3's shared helper): the
                // new-definition oracle must hold IMMEDIATELY after this
                // step, with no intervening catch-up run — unlike
                // `RewriteModel`.
                let backend = project.backend().await?;
                let outcome = run_migrate_step(
                    &project.project_dir,
                    "dev",
                    backend.as_ref(),
                    recipe,
                    *edit,
                    || async {
                        let mut request = base_request("dev");
                        request.full_refresh = true;
                        request.start = None;
                        request.end = None;
                        project
                            .run_quiet(&format!("run-{i}-full-refresh"), request)
                            .await?;
                        Ok(())
                    },
                )
                .await?;
                migrate_outcomes.push(outcome);
                current_edit = Some(*edit);

                match outcome {
                    MigrateStepOutcome::Applied => {
                        // No new source data was read — the migration only
                        // changed the definition, so the S-tracker's
                        // existing last `k` (from the prior RunWindow) is
                        // still the right point to assert against.
                    }
                    MigrateStepOutcome::FullRefreshed => {
                        // The maintained table now reflects the full
                        // current source contents under the migrated
                        // definition — record a fresh full-refresh point,
                        // mirroring the `FullRefreshRun` arm above.
                        let snapshot = {
                            let conn = project.connect()?;
                            read_source_snapshot(&conn, &recipe.source)
                        };
                        let k = tracker.record_full_refresh(snapshot);
                        last_k = Some(k);
                    }
                }

                let k = last_k.ok_or_else(|| {
                    anyhow::anyhow!(
                        "MigrateModel step at index {i} had no prior RunWindow to assert \
                         against: {schedule:?}"
                    )
                })?;
                assert_equivalence_with_edit(project, recipe, &tracker, k, current_edit).await?;
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
pub(crate) async fn assert_equivalence(
    project: &LinkCProject,
    recipe: &ModelRecipe,
    tracker: &STracker,
    k: usize,
) -> anyhow::Result<()> {
    assert_equivalence_at_point(project, recipe, tracker, k, &ContractPoint::Default).await
}

/// [`assert_equivalence`] parameterised by contract-lattice point
/// (`docs/outcomes/20260809-contract-lattice-v1/phases/06-plan.md`):
/// dispatches on `smelt_logical::contract::oracle_obligation(point)` rather
/// than re-deriving a per-point comparator — [`OracleObligation::Exact`]
/// and [`OracleObligation::ExactOverRestrictedS`] are the same both-
/// direction `multiset_equal_via_backend` check
/// [`assert_equivalence_with_edit`] already performs, evaluated against
/// `tracker.s_at_for_point(k, point)` instead of the unrestricted `s_at(k)`;
/// [`OracleObligation::ExactOverProcessedSWithLagBound`] (`deferral`) adds a
/// second leg — every landed-but-unprocessed event time must be within the
/// declared lag bound (`deferral::settled_lag_bound`). `input_frontier` is
/// only consulted for that obligation — every other point ignores it.
pub(crate) async fn assert_equivalence_at_point(
    project: &LinkCProject,
    recipe: &ModelRecipe,
    tracker: &STracker,
    k: usize,
    point: &ContractPoint,
) -> anyhow::Result<()> {
    assert_equivalence_at_point_with_frontier(project, recipe, tracker, k, point, None).await
}

/// [`assert_equivalence_at_point`] with an explicit `input_frontier`
/// (days-from-CE) for the [`OracleObligation::ExactOverProcessedSWithLagBound`]
/// lag-bound leg — required whenever `point` is [`ContractPoint::Deferral`];
/// ignored otherwise.
pub(crate) async fn assert_equivalence_at_point_with_frontier(
    project: &LinkCProject,
    recipe: &ModelRecipe,
    tracker: &STracker,
    k: usize,
    point: &ContractPoint,
    input_frontier: Option<i64>,
) -> anyhow::Result<()> {
    let backend = project.backend().await?;
    let maintained_sql = format!("SELECT * FROM main.{}", recipe.model_name);

    match oracle_obligation(point) {
        OracleObligation::Exact | OracleObligation::ExactOverRestrictedS => {
            tracker
                .materialize_s_for_point(backend.as_ref(), k, point)
                .await?;
            let oracle_sql = tracker.s_restricted_oracle_sql(recipe);
            let equal =
                multiset_equal_via_backend(backend.as_ref(), &maintained_sql, &oracle_sql).await?;
            if !equal {
                anyhow::bail!(
                    "equivalence violated for model {:?} at run {k} under point {point:?}: \
                     maintained ({maintained_sql:?}) != oracle ({oracle_sql:?})",
                    recipe.model_name
                );
            }
            Ok(())
        }
        OracleObligation::ExactOverProcessedSWithLagBound => {
            let input_frontier = input_frontier.ok_or_else(|| {
                anyhow::anyhow!(
                    "point {point:?} has an ExactOverProcessedSWithLagBound oracle obligation \
                     but no input_frontier was supplied"
                )
            })?;
            let d = match point {
                ContractPoint::Deferral { d } => *d,
                _ => anyhow::bail!(
                    "ExactOverProcessedSWithLagBound is only licensed for ContractPoint::Deferral, \
                     got {point:?}"
                ),
            };

            // Leg 1: strict equality over the processed set S — identical to
            // the Exact/ExactOverRestrictedS legs above (deferral does not
            // restrict the run window).
            tracker
                .materialize_s_for_point(backend.as_ref(), k, point)
                .await?;
            let oracle_sql = tracker.s_restricted_oracle_sql(recipe);
            let equal =
                multiset_equal_via_backend(backend.as_ref(), &maintained_sql, &oracle_sql).await?;
            if !equal {
                anyhow::bail!(
                    "equivalence violated for model {:?} at run {k} under point {point:?}: \
                     maintained ({maintained_sql:?}) != oracle ({oracle_sql:?})",
                    recipe.model_name
                );
            }

            // Leg 2: every landed-but-unprocessed event time is within the
            // declared lag bound D (`deferral::settled_lag_bound`) — the gate
            // encodes no comparator of its own.
            let unprocessed_event_times: Vec<i64> = tracker
                .landed_not_processed(k)
                .iter()
                .map(|row| row.event_time().num_days_from_ce() as i64)
                .collect();
            smelt_logical::contract::deferral::settled_lag_bound(
                &unprocessed_event_times,
                input_frontier,
                d,
            )
            .map_err(|violation| {
                anyhow::anyhow!(
                    "deferral lag bound violated for model {:?} at run {k}: {violation:?}",
                    recipe.model_name
                )
            })?;
            Ok(())
        }
    }
}

/// [`assert_equivalence`] generalised over an optional post-`RewriteModel`
/// [`ModelEdit`] (Phase 9): when `edit` is `Some`, the oracle re-renders
/// against the REWRITTEN body
/// (`STracker::s_restricted_oracle_sql_with_edit`) rather than the
/// original recipe's own body — never comparing the old body against the
/// new output (plan Phase 9 review checklist). `edit: None` reproduces
/// [`assert_equivalence`]'s exact pre-Phase-9 behaviour.
pub(crate) async fn assert_equivalence_with_edit(
    project: &LinkCProject,
    recipe: &ModelRecipe,
    tracker: &STracker,
    k: usize,
    edit: Option<ModelEdit>,
) -> anyhow::Result<()> {
    let backend = project.backend().await?;
    tracker.materialize_s(backend.as_ref(), k).await?;
    let maintained_sql = format!("SELECT * FROM main.{}", recipe.model_name);
    let oracle_sql = match edit {
        Some(edit) => tracker.s_restricted_oracle_sql_with_edit(recipe, edit),
        None => tracker.s_restricted_oracle_sql(recipe),
    };
    let equal = multiset_equal_via_backend(backend.as_ref(), &maintained_sql, &oracle_sql).await?;
    if !equal {
        anyhow::bail!(
            "S-restricted equivalence violated for model {:?} at run {k} (edit {edit:?}): \
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
