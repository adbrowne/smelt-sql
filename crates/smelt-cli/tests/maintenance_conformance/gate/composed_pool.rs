//! The composed pool's standing gates: equivalence across all three locality routes, and the admission-rate floor.

use proptest::strategy::{Strategy, ValueTree};
use proptest::test_runner::TestRunner;

use super::composed_routes::{
    drive_composed_derived_and_assert, drive_composed_route2_and_assert,
    drive_composed_route3_and_assert,
};
use super::composed_support::{
    assert_composed_admitted_with_expected_route, classify_composed_full, composed_case_count,
    drive_composed_route1_and_assert,
};
use smelt_backend_duckdb::DuckDbBackend;
use smelt_maintenance_testkit::recipe::{
    arb_composed_route, arb_composed_route3_schedule, arb_keyed_schedule, ComposedKeyedRecipe,
    ComposedRoute,
};
use smelt_maintenance_testkit::render;

/// `composed_keyed_pool_upholds_equivalence` (plan Phase A6 TDD list): the
/// standing proptest gate over the composed pool — deterministic seed,
/// small N. Each case draws one of the three [`ComposedRoute`]s, stages
/// it, confirms the real locality gate admits it with the expected slice
/// shape, then drives its equivalence check (route 1 through real
/// `execute_project`; routes 2/3 through the direct driver against a real
/// `DuckDbBackend`) — asserting equivalence, and a per-slice probe, after
/// **every** step.
#[test]
fn composed_keyed_pool_upholds_equivalence() {
    let n = composed_case_count();
    let mut runner = TestRunner::deterministic();
    let route_strat = arb_composed_route();
    let keyed_schedule_strat = arb_keyed_schedule();
    let route3_schedule_strat = arb_composed_route3_schedule();
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");

    let mut admitted_cases = 0;
    for i in 0..n {
        let route = route_strat.new_tree(&mut runner).unwrap().current();
        let recipe = ComposedKeyedRecipe::new(route);

        let tmp = tempfile::TempDir::new().expect("tempdir");
        let project_dir = tmp.path().join("project");
        let db_path = tmp.path().join("db.duckdb");
        std::fs::create_dir_all(&project_dir).expect("create project dir");
        let project = render::stage_composed(&recipe, &project_dir, &db_path).unwrap_or_else(|e| {
            panic!("case {i}: composed recipe {recipe:?} failed to stage: {e}")
        });

        let (plan, diags) = classify_composed_full(&project, &recipe).unwrap_or_else(|e| {
            panic!("case {i}: composed recipe {recipe:?} classify failed: {e}")
        });
        let plan = plan.unwrap_or_else(|| {
            panic!("case {i}: no plan derived for {recipe:?}: diagnostics={diags:#?}")
        });
        assert_composed_admitted_with_expected_route(&recipe, &plan).unwrap_or_else(|e| {
            panic!("case {i}: {e}");
        });
        admitted_cases += 1;

        match route {
            ComposedRoute::KeyEmbedded => {
                let schedule = keyed_schedule_strat
                    .new_tree(&mut runner)
                    .unwrap()
                    .current();
                rt.block_on(drive_composed_route1_and_assert(
                    &project, &recipe, &schedule,
                ))
                .unwrap_or_else(|e| {
                    panic!(
                        "case {i}: composed route-1 recipe {recipe:?} schedule {schedule:?} \
                         failed: {e}"
                    )
                });
            }
            ComposedRoute::KeyDetermined => {
                let schedule = keyed_schedule_strat
                    .new_tree(&mut runner)
                    .unwrap()
                    .current();
                let backend = rt.block_on(async {
                    DuckDbBackend::new(&project.db_path, "main")
                        .await
                        .expect("open backend")
                });
                rt.block_on(drive_composed_route2_and_assert(
                    &backend, &recipe, &schedule,
                ))
                .unwrap_or_else(|e| {
                    panic!(
                        "case {i}: composed route-2 recipe {recipe:?} schedule {schedule:?} \
                         failed: {e}"
                    )
                });
            }
            ComposedRoute::KeyDerived => {
                let schedule = keyed_schedule_strat
                    .new_tree(&mut runner)
                    .unwrap()
                    .current();
                let backend = rt.block_on(async {
                    DuckDbBackend::new(&project.db_path, "main")
                        .await
                        .expect("open backend")
                });
                rt.block_on(drive_composed_derived_and_assert(
                    &backend, &recipe, &schedule,
                ))
                .unwrap_or_else(|e| {
                    panic!(
                        "case {i}: composed derived-sub-route recipe {recipe:?} schedule \
                         {schedule:?} failed: {e}"
                    )
                });
            }
            ComposedRoute::RecurrenceBounded => {
                let schedule = route3_schedule_strat
                    .new_tree(&mut runner)
                    .unwrap()
                    .current();
                let backend = rt.block_on(async {
                    DuckDbBackend::new(&project.db_path, "main")
                        .await
                        .expect("open backend")
                });
                rt.block_on(drive_composed_route3_and_assert(
                    &backend, &recipe, &schedule,
                ))
                .unwrap_or_else(|e| {
                    panic!(
                        "case {i}: composed route-3 recipe {recipe:?} schedule {schedule:?} \
                         failed: {e}"
                    )
                });
            }
        }
    }

    assert!(
        admitted_cases > 0,
        "N={n} deterministic composed sample admitted zero cases — generator/derivation \
         regression"
    );
}

/// `composed_keyed_admission_rate_stays_above_floor` (plan Phase A6 TDD
/// list): generator health — every generated composed recipe is
/// deliberately constructed to admit exactly its own route (unlike the
/// append-only-partition pool's randomly-drawn constructs, this pool has
/// no randomised refusal branch), so the floor is high: a regression that
/// silently breaks one route's admission must fail this test rather than
/// hollow out the standing gate above unnoticed.
#[test]
fn composed_keyed_admission_rate_stays_above_floor() {
    const N: usize = 30;
    let mut runner = TestRunner::deterministic();
    let route_strat = arb_composed_route();

    let mut admitted = 0;
    for i in 0..N {
        let route = route_strat.new_tree(&mut runner).unwrap().current();
        let recipe = ComposedKeyedRecipe::new(route);
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let project_dir = tmp.path().join("project");
        let db_path = tmp.path().join("db.duckdb");
        std::fs::create_dir_all(&project_dir).expect("create project dir");
        let project = render::stage_composed(&recipe, &project_dir, &db_path).unwrap_or_else(|e| {
            panic!("case {i}: composed recipe {recipe:?} failed to stage: {e}")
        });
        let (plan, _diags) = classify_composed_full(&project, &recipe).unwrap_or_else(|e| {
            panic!("case {i}: composed recipe {recipe:?} classify failed: {e}")
        });
        let admitted_here = plan
            .map(|p| assert_composed_admitted_with_expected_route(&recipe, &p).is_ok())
            .unwrap_or(false);
        if admitted_here {
            admitted += 1;
        }
    }

    let rate = admitted as f64 / N as f64;
    assert!(
        rate >= 0.90,
        "composed-pool admission rate {rate:.2} over N={N} fell below the 90% floor \
         ({admitted}/{N} admitted) — a route-admission regression would silently hollow out the \
         standing gate"
    );
}
