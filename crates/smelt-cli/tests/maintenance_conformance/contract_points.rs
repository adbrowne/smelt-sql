//! Conformance-gate coverage for the contract lattice's per-point oracle
//! (`docs/outcomes/20260809-contract-lattice-v1/phases/06-plan.md`; tests
//! 6-8 of the phase's TDD list). The two relaxed fixtures here are
//! deliberately hand-driven (not `drive_and_assert`/`arb_schedule_for`) —
//! `ModelRecipe`'s generic schedule generators know nothing about a
//! declared `contract:` relaxation, so exercising the relaxed oracle needs
//! an explicit, narrated sequence of runs rather than a generated one.
//! `render::render_smelt_yml` already sets `probes: cadence: off` for the
//! whole harness (see its own doc comment), so neither fixture needs to opt
//! out of probe firing separately.

use chrono::{Datelike, NaiveDate};
use proptest::strategy::{Strategy, ValueTree};
use proptest::test_runner::TestRunner;

use smelt_logical::contract::ContractPoint;
use smelt_maintenance_testkit::link_c_harness::base_request;
use smelt_maintenance_testkit::oracle::except_all_row_count_via_backend;
use smelt_maintenance_testkit::recipe::{
    arb_recipe, ConstructKind, ContractDecl, ModelRecipe, RecipePool,
};
use smelt_maintenance_testkit::render;
use smelt_maintenance_testkit::s_tracker::STracker;
use smelt_maintenance_testkit::schedule_gen::{arb_schedule_for, GenRow};
use smelt_state::RunOutcomeKind;

use crate::gate::{
    assert_equivalence, assert_equivalence_at_point, assert_equivalence_at_point_with_frontier,
    drive_and_assert, stage_recipe,
};

fn day(offset: i64) -> NaiveDate {
    NaiveDate::from_ymd_opt(2024, 1, 1)
        .expect("valid base date")
        .checked_add_signed(chrono::Duration::days(offset))
        .expect("valid offset date")
}

fn days_from_ce(d: NaiveDate) -> i64 {
    d.num_days_from_ce() as i64
}

/// A deterministic pinned [`ModelRecipe`] drawing only [`ConstructKind::AdditiveAgg`]
/// — mirrors `pinned.rs`'s own `pinned_body_recipe`/`harness_self_check.rs`'s
/// draw, kept local since neither is exported from its module.
fn pinned_additive_agg_recipe() -> ModelRecipe {
    let mut runner = TestRunner::deterministic();
    let pool = RecipePool {
        constructs: vec![ConstructKind::AdditiveAgg],
    };
    arb_recipe(pool).new_tree(&mut runner).unwrap().current()
}

async fn insert_row(
    project: &smelt_maintenance_testkit::link_c_harness::LinkCProject,
    recipe: &ModelRecipe,
    row: &GenRow,
) -> anyhow::Result<()> {
    let backend = project.backend().await?;
    smelt_backend::Backend::execute_sql(
        backend.as_ref(),
        &format!(
            "INSERT INTO main.sources_{} VALUES (DATE '{}', {}, {})",
            recipe.source.name,
            row.d.format("%Y-%m-%d"),
            row.id,
            row.val_sql(),
        ),
    )
    .await
    .map_err(|e| anyhow::anyhow!("insert row: {e}"))?;
    Ok(())
}

/// `default_recipes_are_still_asserted_exactly` (phase 6 TDD list, test 6):
/// harness self-check — a default recipe driven through
/// `assert_equivalence_at_point(Default)` behaves exactly as
/// `assert_equivalence` (which now delegates to it) — no silent weakening
/// of the standing pool's own oracle.
#[test]
fn default_recipes_are_still_asserted_exactly() {
    let mut runner = TestRunner::deterministic();
    let recipe = pinned_additive_agg_recipe();
    let schedule = arb_schedule_for(&recipe)
        .new_tree(&mut runner)
        .unwrap()
        .current();

    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project = stage_recipe(&recipe, &tmp).expect("stage recipe");

    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let (tracker, k) = rt
        .block_on(drive_and_assert(&project, &recipe, &schedule))
        .expect("green schedule must uphold equivalence");

    rt.block_on(assert_equivalence(&project, &recipe, &tracker, k))
        .expect("assert_equivalence must hold for a default recipe");
    rt.block_on(assert_equivalence_at_point(
        &project,
        &recipe,
        &tracker,
        k,
        &ContractPoint::Default,
    ))
    .expect(
        "assert_equivalence_at_point(Default) must reproduce assert_equivalence's own \
         behaviour byte-for-byte — assert_equivalence delegates to it",
    );
}

/// `frozen_horizon_recipe_upholds_relaxed_oracle_and_not_the_default`
/// (phase 6 TDD list, test 7): a late row lands in an already-frozen
/// partition; the model's declared `contract.frozen_horizon` means the real
/// write-eligibility clamp never rewrites that partition. The RELAXED
/// oracle (`ContractPoint::FrozenHorizon`) must hold; the STRICT/default
/// oracle must FAIL — proof the relaxation is genuinely under test, not
/// silently subsumed by the default comparison.
#[test]
fn frozen_horizon_recipe_upholds_relaxed_oracle_and_not_the_default() {
    let mut recipe = pinned_additive_agg_recipe();
    recipe.model_name = "frozen_horizon_fixture".to_string();
    let h_days = 2;
    recipe.contract = Some(ContractDecl::FrozenHorizon { days: h_days });

    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project = stage_recipe(&recipe, &tmp).expect("stage recipe");
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");

    let mut tracker = STracker::new(&recipe.source);
    let point = ContractPoint::FrozenHorizon { h: h_days };

    // Run 1: a normal, small window — H = 2 days is wider than this run's
    // own 1-day span, so the write clamp does not narrow it at all.
    let row0 = GenRow {
        d: day(0),
        id: 1,
        val: Some(10),
    };
    rt.block_on(insert_row(&project, &recipe, &row0))
        .expect("insert row0");
    let mut request = base_request("dev");
    request.start = Some(day(0).format("%Y-%m-%d").to_string());
    request.end = Some(day(1).format("%Y-%m-%d").to_string());
    rt.block_on(project.run_quiet("run-0", request))
        .expect("run 1 must succeed");
    let k0 = tracker.record_run(day(0), day(1), vec![row0.clone()]);
    rt.block_on(assert_equivalence_at_point(
        &project, &recipe, &tracker, k0, &point,
    ))
    .expect("relaxed oracle must hold before any late arrival");

    // A late row lands back in the day-0 partition — no run touches it.
    let late = GenRow {
        d: day(0),
        id: 2,
        val: Some(999),
    };
    rt.block_on(insert_row(&project, &recipe, &late))
        .expect("insert late row");

    // Run 2: a WIDE catch-up window whose own end - H floor (day3) is past
    // day 0 — the real write-eligibility clamp narrows this run's start to
    // day 3, so the day-0 partition (and the late row now sitting in it) is
    // never rewritten.
    let mut request = base_request("dev");
    request.start = Some(day(0).format("%Y-%m-%d").to_string());
    request.end = Some(day(5).format("%Y-%m-%d").to_string());
    rt.block_on(project.run_quiet("run-1", request))
        .expect("run 2 must succeed");
    let k1 = tracker.record_run(day(0), day(5), vec![row0.clone(), late.clone()]);

    rt.block_on(assert_equivalence_at_point(
        &project, &recipe, &tracker, k1, &point,
    ))
    .expect(
        "the relaxed frozen-horizon oracle must hold — it never expects the frozen \
         partition to reflect the late row",
    );

    let default_result = rt.block_on(assert_equivalence(&project, &recipe, &tracker, k1));
    assert!(
        default_result.is_err(),
        "the STRICT/default oracle must FAIL here — it expects the late row to be \
         reflected once a run's own window covers it, but the real system (honouring the \
         declared frozen_horizon) never rewrote that partition. If this passes, the \
         relaxation is not actually being exercised."
    );
}

/// `deferral_recipe_upholds_restated_oracle_with_a_skipped_run`
/// (`docs/outcomes/20260904-decided-gap-residue/phases/02-plan.md` test
/// list; supersedes phase-6's `deferral_recipe_upholds_bracketed_oracle_with_a_skipped_run`):
/// a two-model fixture (mirrors
/// `crates/smelt-runtime/tests/contract_deferral_skip_e2e.rs`'s own shape)
/// opens a lag between a `contract.deferral`-declared model's own maintained
/// frontier and the shared source's landed-delta frontier by advancing the
/// latter through an undeclared sibling model alone. Run B and the licensed
/// skip in run C are recorded as *landings* — `deferred_model` never folds
/// either window, so the tracker's processed set `S` must not be inflated
/// past what `deferred_model` actually processed (the bug the superseded
/// bracket comparator masked). The restated oracle
/// (`ExactOverProcessedSWithLagBound`: strict equality over `S` plus
/// `deferral::settled_lag_bound` over what has landed but not been
/// processed) must hold throughout.
#[test]
fn deferral_recipe_upholds_restated_oracle_with_a_skipped_run() {
    let mut recipe = pinned_additive_agg_recipe();
    recipe.model_name = "deferred_model".to_string();
    let d_days = 2;
    recipe.contract = Some(ContractDecl::Deferral { days: d_days });

    let mut upstream_recipe = recipe.clone();
    upstream_recipe.model_name = "upstream_advancer".to_string();
    upstream_recipe.contract = None;

    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project = stage_recipe(&recipe, &tmp).expect("stage recipe");
    std::fs::write(
        project.project_dir.join("models/upstream_advancer.sql"),
        render::render_model_file(&upstream_recipe),
    )
    .expect("write upstream_advancer.sql");

    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let mut tracker = STracker::new(&recipe.source);
    let point = ContractPoint::Deferral { d: d_days };

    // Run A: both models select (empty `select`), establishing both the
    // model's own maintained frontier and the source's landed-delta
    // frontier at day 1.
    let row0 = GenRow {
        d: day(0),
        id: 1,
        val: Some(10),
    };
    rt.block_on(insert_row(&project, &recipe, &row0))
        .expect("insert row0");
    let mut request = base_request("dev");
    request.start = Some(day(0).format("%Y-%m-%d").to_string());
    request.end = Some(day(1).format("%Y-%m-%d").to_string());
    rt.block_on(project.run_quiet("run-a", request))
        .expect("run A must succeed");
    let k0 = tracker.record_run(day(0), day(1), vec![row0.clone()]);
    rt.block_on(assert_equivalence_at_point_with_frontier(
        &project,
        &recipe,
        &tracker,
        k0,
        &point,
        Some(days_from_ce(day(1))),
    ))
    .expect("bracket must hold immediately after run A (lag is 0)");

    // Run B: ONLY `upstream_advancer` runs over [day1, day3) — the shared
    // source's landed-delta frontier advances to day 3 while
    // `deferred_model`'s own maintained frontier stays at day 1 (it never
    // ran this window).
    let row1 = GenRow {
        d: day(1),
        id: 2,
        val: Some(20),
    };
    let row2 = GenRow {
        d: day(1) + chrono::Duration::days(1),
        id: 3,
        val: Some(30),
    };
    rt.block_on(insert_row(&project, &recipe, &row1))
        .expect("insert row1");
    rt.block_on(insert_row(&project, &recipe, &row2))
        .expect("insert row2");
    let mut request = base_request("dev");
    request.select = vec!["upstream_advancer".to_string()];
    request.start = Some(day(1).format("%Y-%m-%d").to_string());
    request.end = Some(day(3).format("%Y-%m-%d").to_string());
    rt.block_on(project.run_quiet("run-b", request))
        .expect("run B must succeed");
    // Recorded as a LANDING, not a run — `deferred_model` never folds this
    // window (only `upstream_advancer` ran it), so recording it via
    // `record_run` would inflate the tracker's processed set `S` past what
    // `deferred_model` actually processed (the bug the superseded bracket
    // comparator masked, since its lower leg held vacuously). `row1`/`row2`
    // are simply visible in the source now — landed, not processed.
    tracker.record_landing(vec![row1.clone(), row2.clone()]);
    let input_frontier_after_b = days_from_ce(day(3));
    rt.block_on(assert_equivalence_at_point_with_frontier(
        &project,
        &recipe,
        &tracker,
        k0,
        &point,
        Some(input_frontier_after_b),
    ))
    .expect(
        "restated oracle must hold after run B — the landed-but-unprocessed rows (row1, row2) \
         are at/after the settled cutoff",
    );

    // Run C: `deferred_model` is selected over [day3, day4) — the measured
    // lag (input frontier day3 minus maintained frontier day1 = 2 days) is
    // exactly `D`, so this run must be a licensed skip, not a real
    // execution.
    let mut request = base_request("dev");
    request.select = vec!["deferred_model".to_string()];
    request.start = Some(day(3).format("%Y-%m-%d").to_string());
    request.end = Some(
        (day(3) + chrono::Duration::days(1))
            .format("%Y-%m-%d")
            .to_string(),
    );
    let outcome = rt
        .block_on(project.run_quiet("run-c", request))
        .expect("run C must succeed (a licensed skip, not a failure)");
    let record = outcome
        .models
        .get("deferred_model")
        .expect("deferred_model must have a manifest entry even when skipped");
    assert_eq!(record.strategy, "skipped_deferral");
    assert_eq!(record.outcome, RunOutcomeKind::Skipped);
    assert_eq!(record.row_count, 0);

    // The skip advances neither the tracker (no new window was folded, and
    // nothing new landed either) nor the source's landed-delta frontier
    // (upstream_advancer was not selected this run) — the restated oracle
    // must still hold, unchanged.
    rt.block_on(assert_equivalence_at_point_with_frontier(
        &project,
        &recipe,
        &tracker,
        k0,
        &point,
        Some(input_frontier_after_b),
    ))
    .expect("restated oracle must hold after the licensed skip");
}

/// `deferral_comparator_rejects_a_state_the_bracket_admitted`
/// (`docs/outcomes/20260904-decided-gap-residue/phases/02-plan.md` test
/// list): METAMORPHIC — proves the restated comparator is no longer
/// vacuous. After run A, every row is deleted from `main.deferred_model`
/// (a deliberately wrong incremental state: the model silently forgot what
/// it had already folded). Both legs of the SUPERSEDED bracket still hold
/// inline (`maintained EXCEPT ALL full_refresh(S)` is 0 rows — trivially,
/// since `maintained` is now empty — and `s_at_settled(...)` is empty
/// because the settled cutoff precedes all recorded event time, so that leg
/// was vacuous too), yet `assert_equivalence_at_point_with_frontier` must
/// return `Err` under the restated `ExactOverProcessedSWithLagBound`
/// comparator, which checks strict equality over `S` rather than a bracket
/// with a vacuous lower leg.
#[test]
fn deferral_comparator_rejects_a_state_the_bracket_admitted() {
    let mut recipe = pinned_additive_agg_recipe();
    recipe.model_name = "deferred_model".to_string();
    let d_days = 2;
    recipe.contract = Some(ContractDecl::Deferral { days: d_days });

    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project = stage_recipe(&recipe, &tmp).expect("stage recipe");

    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let mut tracker = STracker::new(&recipe.source);
    let point = ContractPoint::Deferral { d: d_days };

    // Run A: establishes the model's own maintained frontier at day 1.
    let row0 = GenRow {
        d: day(0),
        id: 1,
        val: Some(10),
    };
    rt.block_on(insert_row(&project, &recipe, &row0))
        .expect("insert row0");
    let mut request = base_request("dev");
    request.start = Some(day(0).format("%Y-%m-%d").to_string());
    request.end = Some(day(1).format("%Y-%m-%d").to_string());
    rt.block_on(project.run_quiet("run-a", request))
        .expect("run A must succeed");
    let k0 = tracker.record_run(day(0), day(1), vec![row0.clone()]);
    let input_frontier = days_from_ce(day(1));

    // Deliberately corrupt the maintained state: delete everything, as if
    // the model forgot what it had already folded.
    let backend = rt.block_on(project.backend()).expect("backend");
    rt.block_on(smelt_backend::Backend::execute_sql(
        backend.as_ref(),
        &format!("DELETE FROM main.{}", recipe.model_name),
    ))
    .expect("delete every row from the maintained table");

    // The SUPERSEDED bracket's leg 1 (`maintained EXCEPT ALL full_refresh(S)`)
    // holds trivially — `maintained` is empty, so it cannot have rows the
    // oracle lacks.
    rt.block_on(tracker.materialize_s_for_point(backend.as_ref(), k0, &point))
        .expect("materialize S");
    let full_oracle_sql = tracker.s_restricted_oracle_sql(&recipe);
    let maintained_sql = format!("SELECT * FROM main.{}", recipe.model_name);
    let maintained_minus_full = rt
        .block_on(except_all_row_count_via_backend(
            backend.as_ref(),
            &maintained_sql,
            &full_oracle_sql,
        ))
        .expect("except all count");
    assert_eq!(
        maintained_minus_full, 0,
        "the superseded bracket's leg 1 must hold vacuously against an empty maintained table"
    );

    // The SUPERSEDED bracket's leg 2 (`full_refresh(S_settled)`) is empty —
    // the settled cutoff (day1 - 2 = day(-1)) precedes all recorded event
    // time (day0), so there is nothing to require, i.e. this leg was
    // vacuous too.
    let settled = tracker.s_at_settled(k0, &point, input_frontier);
    assert!(
        settled.is_empty(),
        "the superseded bracket's leg 2 must be vacuous here — the settled cutoff precedes \
         all recorded event time, got: {settled:?}"
    );

    // The RESTATED comparator must catch what the bracket missed: strict
    // equality over S fails, since row0 is in the oracle but not in the
    // (now-empty) maintained table.
    let result = rt.block_on(assert_equivalence_at_point_with_frontier(
        &project,
        &recipe,
        &tracker,
        k0,
        &point,
        Some(input_frontier),
    ));
    assert!(
        result.is_err(),
        "the restated comparator must reject a maintained state the superseded bracket \
         admitted, but it returned Ok"
    );
}
