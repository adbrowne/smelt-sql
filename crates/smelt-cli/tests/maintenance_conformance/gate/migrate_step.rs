//! `ConformanceStep::MigrateModel` — the shipped `smelt migrate` derive/apply path staged mid-schedule, plus the definition-edit pool's generative gate.

use proptest::strategy::{Strategy, ValueTree};
use proptest::test_runner::TestRunner;

use super::partition_pool::{
    assert_equivalence, assert_equivalence_with_edit, drive_and_assert_collecting, insert_row,
    stage_recipe,
};
use smelt_maintenance_testkit::link_c_harness::base_request;
use smelt_maintenance_testkit::migrate_step::MigrateStepOutcome;
use smelt_maintenance_testkit::recipe::{arb_recipe, ConstructKind, ModelEdit, RecipePool};
use smelt_maintenance_testkit::render;
use smelt_maintenance_testkit::s_tracker::STracker;
use smelt_maintenance_testkit::schedule_gen::{
    arb_schedule_with_definition_edit, read_source_snapshot, ConformanceSchedule, ConformanceStep,
    GenRow,
};
use smelt_maintenance_testkit::verdict::{classify, Verdict};

// ---------------------------------------------------------------------
// `ConformanceStep::MigrateModel` — the shipped `smelt migrate`
// derive→apply path, staged mid-schedule
// (`docs/outcomes/20260815-definition-delta-migrate/phases/05-plan.md`).
// Unlike `RewriteModel` above, the new-definition oracle must hold
// IMMEDIATELY after the step, with no intervening catch-up run.
// ---------------------------------------------------------------------

/// `migrate_step_applies_plan_and_recovers_new_definition_equivalence`
/// (plan test 2): a `PassThrough` recipe's `AddPayloadColumn` edit
/// (`val * 2 AS val_doubled`, a pure function of the already-stored `val`
/// column — the same shape `smelt-cli/tests/migrate_apply.rs`'s
/// `MODEL_V2_SELF_DERIVED` fixture exercises) must admit an in-place
/// `smelt_logical::backbuild` technique (`Technique::SelfDerivedColumnAdd`),
/// so the `MigrateModel` step takes the `Applied` leg — executed via
/// [`run_migrate_step`], never a full refresh. Two windows run under the
/// original body first (establishing a deployed-schema baseline), then the
/// migration, asserted equal to the rewritten body's own oracle with NO
/// intervening run, then a further windowed run must keep it equal.
#[test]
fn migrate_step_applies_plan_and_recovers_new_definition_equivalence() {
    let mut runner = TestRunner::deterministic();
    let pool = RecipePool {
        constructs: vec![ConstructKind::PassThrough],
    };
    let recipe = arb_recipe(pool).new_tree(&mut runner).unwrap().current();
    assert!(
        recipe.evolution.contains(&ModelEdit::AddPayloadColumn),
        "PassThrough recipes must carry the AddPayloadColumn evolution: {recipe:?}"
    );

    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project = stage_recipe(&recipe, &tmp).expect("stage recipe");
    let verdict = classify(&project, &recipe).expect("classify");
    assert!(
        matches!(verdict, Verdict::Admitted(_)),
        "expected the PassThrough append-only recipe to admit: {verdict:?}"
    );

    let w1_start = chrono::NaiveDate::from_ymd_opt(2024, 1, 1).expect("valid date");
    let w1_end = w1_start + chrono::Duration::days(1);
    let w2_start = w1_end;
    let w2_end = w2_start + chrono::Duration::days(1);
    let w3_start = w2_end;
    let w3_end = w3_start + chrono::Duration::days(1);

    let schedule = ConformanceSchedule(vec![
        // Two windows run under the ORIGINAL body — establishes a deployed
        // schema baseline `derive_plan` diffs against.
        ConformanceStep::RunWindow {
            start: w1_start,
            end: w1_end,
            rows: vec![GenRow {
                d: w1_start,
                id: 1,
                val: Some(10),
            }],
        },
        ConformanceStep::RunWindow {
            start: w2_start,
            end: w2_end,
            rows: vec![GenRow {
                d: w2_start,
                id: 2,
                val: Some(20),
            }],
        },
        // The migration: derive → apply in place → assert immediately.
        ConformanceStep::MigrateModel {
            edit: ModelEdit::AddPayloadColumn,
        },
        // A subsequent ordinary windowed run must keep working post-apply.
        ConformanceStep::RunWindow {
            start: w3_start,
            end: w3_end,
            rows: vec![GenRow {
                d: w3_start,
                id: 3,
                val: Some(30),
            }],
        },
    ]);

    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let mut outcomes = Vec::new();
    rt.block_on(drive_and_assert_collecting(
        &project,
        &recipe,
        &schedule,
        &mut outcomes,
    ))
    .expect(
        "a PassThrough AddPayloadColumn migration must apply in place and recover equivalence \
         against the rewritten body's own oracle with no intervening run, and stay recovered \
         through a subsequent windowed run",
    );

    assert_eq!(
        outcomes,
        vec![MigrateStepOutcome::Applied],
        "PassThrough + AddPayloadColumn is a pure function of the already-stored `val` column \
         and must admit an in-place backbuild technique (Technique::SelfDerivedColumnAdd), never \
         fall back to a full refresh: {outcomes:?}"
    );
}

/// `migrate_step_refuses_and_full_refreshes_when_no_technique_admits`
/// (plan test 3): an `AdditiveAgg` recipe's `AddGroupingColumn` edit widens
/// the `GROUP BY` — a skeleton/grain change
/// (`skeleton_position_add_derives_skeleton_column_added_refusal` above
/// proves the LIVE maintenance driver refuses this shape too), so the
/// derived `smelt_logical::backbuild` plan admits no in-place technique
/// (`plan.statements.is_empty()`, the same condition
/// `commands::migrate::apply_plan` reports as
/// `MigrateError::FullRefreshRequired`). The `MigrateModel` step must
/// therefore take the `FullRefreshed` leg, and equivalence must still hold
/// afterward.
#[test]
fn migrate_step_refuses_and_full_refreshes_when_no_technique_admits() {
    let mut runner = TestRunner::deterministic();
    let pool = RecipePool {
        constructs: vec![ConstructKind::AdditiveAgg],
    };
    let recipe = arb_recipe(pool).new_tree(&mut runner).unwrap().current();
    assert!(
        recipe.evolution.contains(&ModelEdit::AddGroupingColumn),
        "AdditiveAgg recipes must carry the AddGroupingColumn evolution: {recipe:?}"
    );

    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project = stage_recipe(&recipe, &tmp).expect("stage recipe");
    let verdict = classify(&project, &recipe).expect("classify");
    assert!(
        matches!(verdict, Verdict::Admitted(_)),
        "expected the additive-agg append-only recipe to admit: {verdict:?}"
    );

    let w1_start = chrono::NaiveDate::from_ymd_opt(2024, 1, 1).expect("valid date");
    let w1_end = w1_start + chrono::Duration::days(1);
    let w2_start = w1_end;
    let w2_end = w2_start + chrono::Duration::days(1);

    let schedule = ConformanceSchedule(vec![
        // Establishes a deployed schema baseline.
        ConformanceStep::RunWindow {
            start: w1_start,
            end: w1_end,
            rows: vec![GenRow {
                d: w1_start,
                id: 1,
                val: Some(10),
            }],
        },
        // A skeleton change: no in-place technique admits — the step must
        // fall back to a full refresh.
        ConformanceStep::MigrateModel {
            edit: ModelEdit::AddGroupingColumn,
        },
        // A subsequent ordinary windowed run must keep working post-refresh.
        ConformanceStep::RunWindow {
            start: w2_start,
            end: w2_end,
            rows: vec![GenRow {
                d: w2_start,
                id: 2,
                val: Some(20),
            }],
        },
    ]);

    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let mut outcomes = Vec::new();
    rt.block_on(drive_and_assert_collecting(
        &project,
        &recipe,
        &schedule,
        &mut outcomes,
    ))
    .expect(
        "a skeleton-changing migration must fall back to a full refresh and still recover \
         equivalence against the rewritten body's own oracle",
    );

    assert_eq!(
        outcomes,
        vec![MigrateStepOutcome::FullRefreshed],
        "AdditiveAgg + AddGroupingColumn widens the GROUP BY — a skeleton change with no \
         admissible in-place backbuild technique — and must take the FullRefreshed leg, never \
         Applied: {outcomes:?}"
    );
}

/// Default deterministic case count for
/// `definition_edit_pool_upholds_new_definition_equivalence` — every recipe
/// this test drives also stages a `MigrateModel` step and its own
/// `derive_plan` call, so it stays at [`DEFAULT_CASES`]'s scale rather than
/// `admission_rate_stays_above_floor`'s larger N=50 health check.
pub(crate) const DEFINITION_EDIT_CASES: usize = 12;

pub(crate) fn definition_edit_case_count() -> usize {
    std::env::var("SMELT_CONFORMANCE_DEFINITION_EDIT_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFINITION_EDIT_CASES)
}

/// `definition_edit_pool_upholds_new_definition_equivalence` (plan test 4):
/// deterministic-seeded sample over the append-only partition pool
/// restricted to recipes with a non-empty `evolution` (today: every
/// `RecipePool::partition_append_only()` construct), scheduled via
/// [`arb_schedule_with_definition_edit`] and driven via
/// [`drive_and_assert_collecting`] — equivalence is asserted after EVERY
/// step (including immediately after the `MigrateModel` step, with no
/// intervening run). Fails if zero admitted cases in the sample took the
/// `Applied` leg — the anti-vacuity discipline
/// `admission_rate_stays_above_floor` already established for admission
/// itself, extended here to the migration leg: a sample that only ever
/// full-refreshed would silently stop exercising the in-place `smelt
/// migrate` apply path this whole phase exists to cover.
#[test]
fn definition_edit_pool_upholds_new_definition_equivalence() {
    let n = definition_edit_case_count();
    let mut runner = TestRunner::deterministic();
    let recipe_strat = arb_recipe(RecipePool::partition_append_only());

    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let mut admitted_cases = 0;
    let mut all_outcomes: Vec<MigrateStepOutcome> = Vec::new();

    for i in 0..n {
        let recipe = recipe_strat.new_tree(&mut runner).unwrap().current();
        if recipe.evolution.is_empty() {
            continue;
        }
        let schedule = arb_schedule_with_definition_edit(&recipe)
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
                let mut outcomes = Vec::new();
                rt.block_on(drive_and_assert_collecting(
                    &project,
                    &recipe,
                    &schedule,
                    &mut outcomes,
                ))
                .unwrap_or_else(|e| {
                    panic!(
                        "case {i}: recipe {recipe:?} schedule {schedule:?} equivalence check \
                         failed: {e:?}"
                    )
                });
                all_outcomes.extend(outcomes);
            }
        }
    }

    assert!(
        admitted_cases > 0,
        "N={n} deterministic sample admitted zero cases — generator/derivation regression"
    );
    assert!(
        all_outcomes.contains(&MigrateStepOutcome::Applied),
        "N={n} deterministic sample took the Applied leg zero times across {} MigrateModel \
         step(s) — a vacuous pass that never exercises the in-place smelt-migrate apply path: \
         {all_outcomes:?}",
        all_outcomes.len()
    );
}

/// `skeleton_position_add_is_refused_or_recomputed_never_corrupted` (plan
/// Phase 9 TDD list): adding a column in a grouping/skeleton position
/// mid-schedule never yields a silently-wrong maintained state — either a
/// named refusal (via the real [`classify`] verdict protocol) or a full
/// recompute whose result equals the rewritten body's own oracle. The
/// recompute is driven via `FullRefreshRun`, the only technique that
/// actually survives a schema-shape change today — see
/// `column_add_between_runs_recovers_equivalence`'s doc comment for why a
/// plain windowed re-run against an already-materialized table hits a raw
/// column-count mismatch instead.
#[test]
fn skeleton_position_add_is_refused_or_recomputed_never_corrupted() {
    let mut runner = TestRunner::deterministic();
    let pool = RecipePool {
        constructs: vec![ConstructKind::AdditiveAgg],
    };
    let recipe = arb_recipe(pool).new_tree(&mut runner).unwrap().current();

    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project = stage_recipe(&recipe, &tmp).expect("stage recipe");
    let verdict = classify(&project, &recipe).expect("classify");
    assert!(
        matches!(verdict, Verdict::Admitted(_)),
        "expected additive-agg append-only recipe to admit: {verdict:?}"
    );

    let w1_start = chrono::NaiveDate::from_ymd_opt(2024, 1, 1).expect("valid date");
    let w1_end = w1_start + chrono::Duration::days(1);

    let mut tracker = STracker::new(&recipe.source);
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");

    // Pre-rewrite run, under the original body — establishes a green
    // baseline before the skeleton-changing rewrite.
    rt.block_on(insert_row(
        &project,
        &recipe,
        &GenRow {
            d: w1_start,
            id: 1,
            val: Some(10),
        },
    ))
    .expect("seed row");
    let snapshot = {
        let conn = project.connect().expect("connect");
        read_source_snapshot(&conn, &recipe.source)
    };
    let mut request = base_request("dev");
    request.start = Some(w1_start.format("%Y-%m-%d").to_string());
    request.end = Some(w1_end.format("%Y-%m-%d").to_string());
    rt.block_on(project.run_quiet("run-0", request))
        .expect("initial run before rewrite");
    let k0 = tracker.record_run(w1_start, w1_end, snapshot);
    rt.block_on(assert_equivalence(&project, &recipe, &tracker, k0))
        .expect("pre-rewrite equivalence");

    // Rewrite: add the source's row-key column into the GROUP BY — a
    // grain/skeleton change, `incremental_models.md`'s `SkeletonAdd`
    // territory.
    std::fs::write(
        project
            .project_dir
            .join(format!("models/{}.sql", recipe.model_name)),
        render::render_model_file_with_edit(&recipe, ModelEdit::AddGroupingColumn),
    )
    .expect("write rewritten model file");

    // Classify the NOW-rewritten project through the real derivation
    // (`verdict.rs`'s fail-loud protocol: a refusal always carries a named
    // Maintenance*/admission diagnostic, never an unexplained empty plan).
    let post_verdict = classify(&project, &recipe).expect("classify after rewrite");
    match post_verdict {
        Verdict::Refused(diags) => {
            assert!(
                !diags.is_empty(),
                "a refused skeleton-position rewrite must carry a named diagnostic \
                 (verdict.rs's own fail-loud check already enforces this — asserted here too \
                 so this test would fail if that changed)"
            );
        }
        Verdict::Admitted(_) => {
            let mut request2 = base_request("dev");
            request2.full_refresh = true;
            rt.block_on(project.run_quiet("run-1", request2))
                .expect("an admitted post-rewrite plan must not fail to execute");

            let snapshot = {
                let conn = project.connect().expect("connect");
                read_source_snapshot(&conn, &recipe.source)
            };
            let k1 = tracker.record_full_refresh(snapshot);
            rt.block_on(assert_equivalence_with_edit(
                &project,
                &recipe,
                &tracker,
                k1,
                Some(ModelEdit::AddGroupingColumn),
            ))
            .expect(
                "an admitted skeleton-position rewrite's recompute must equal the rewritten \
                 body's own oracle — never silently diverge",
            );
        }
    }
}
