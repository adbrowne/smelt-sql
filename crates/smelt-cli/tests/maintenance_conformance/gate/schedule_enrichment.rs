//! Schedule enrichment cases: redelivery idempotence, full-refresh interleave, and boundary-row placement.

use proptest::strategy::{Strategy, ValueTree};
use proptest::test_runner::TestRunner;

use super::partition_pool::{drive_and_assert, insert_row, stage_recipe};
use smelt_maintenance_testkit::link_c_harness::base_request;
use smelt_maintenance_testkit::recipe::{arb_recipe, ConstructKind, RecipePool};
use smelt_maintenance_testkit::schedule_gen::{
    boundary_rows_for, scan_clamp_for, ConformanceSchedule, ConformanceStep, GenRow,
};
use smelt_maintenance_testkit::verdict::{classify, Verdict};

// ---------------------------------------------------------------------
// Phase 6: schedule enrichment — redelivery, full-refresh interleave,
// boundary-value placement.
// ---------------------------------------------------------------------

/// `redelivery_of_processed_window_is_idempotent` (plan Phase 6 TDD list):
/// re-running an already-processed window with no new rows never
/// double-counts under the partition-grain DELETE+INSERT full-replace
/// technique — the never-fold-twice obligation checked end-to-end through
/// the real run pipeline.
#[test]
fn redelivery_of_processed_window_is_idempotent() {
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

    let d = chrono::NaiveDate::from_ymd_opt(2024, 1, 1).expect("valid date");
    let window_end = d + chrono::Duration::days(1);

    let schedule = ConformanceSchedule(vec![
        ConformanceStep::RunWindow {
            start: d,
            end: window_end,
            rows: vec![
                GenRow {
                    d,
                    id: 1,
                    val: Some(10),
                },
                GenRow {
                    d,
                    id: 2,
                    val: Some(20),
                },
            ],
        },
        ConformanceStep::RerunWindow {
            start: d,
            end: window_end,
        },
        ConformanceStep::RerunWindow {
            start: d,
            end: window_end,
        },
    ]);

    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(drive_and_assert(&project, &recipe, &schedule))
        .expect(
            "redelivering an already-processed window must stay idempotent — equivalence \
             must hold after every redelivery, never double-counted",
        );
}

/// `full_refresh_interleave_resets_state_correctly` (plan Phase 6 TDD list):
/// a mid-schedule `full_refresh` run resets coverage + the reconciliation
/// ledger such that subsequent incremental runs still uphold equivalence.
#[test]
fn full_refresh_interleave_resets_state_correctly() {
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

    let d1 = chrono::NaiveDate::from_ymd_opt(2024, 1, 1).expect("valid date");
    let d2 = chrono::NaiveDate::from_ymd_opt(2024, 1, 2).expect("valid date");
    let d3 = chrono::NaiveDate::from_ymd_opt(2024, 1, 3).expect("valid date");

    let schedule = ConformanceSchedule(vec![
        ConformanceStep::RunWindow {
            start: d1,
            end: d1 + chrono::Duration::days(1),
            rows: vec![GenRow {
                d: d1,
                id: 1,
                val: Some(10),
            }],
        },
        // A mid-schedule full-refresh interleave: drops + rebuilds from the
        // CURRENT full source contents (just {d1's row} at this point),
        // resetting coverage.
        ConformanceStep::FullRefreshRun,
        ConformanceStep::RunWindow {
            start: d2,
            end: d2 + chrono::Duration::days(1),
            rows: vec![GenRow {
                d: d2,
                id: 2,
                val: Some(20),
            }],
        },
        ConformanceStep::RunWindow {
            start: d3,
            end: d3 + chrono::Duration::days(1),
            rows: vec![GenRow {
                d: d3,
                id: 3,
                val: Some(30),
            }],
        },
    ]);

    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(drive_and_assert(&project, &recipe, &schedule))
        .expect(
            "a mid-schedule full_refresh interleave must reset coverage cleanly — \
             equivalence must hold both immediately after the refresh and through every \
             subsequent windowed run",
        );
}

/// `boundary_rows_within_reach_are_reflected` (plan Phase 6 TDD list): a
/// just-inside-reach row (`schedule_gen::boundary_rows_for`, plan-aware
/// placement against the recipe's OWN derived `ScanClamp`) appears in the
/// maintained output after the run whose window covers it, and the row one
/// calendar day further out (`just_outside`) does not.
///
/// Every construct in `RecipePool::partition_append_only` (the only pool
/// wired up so far) shares the output's partition axis, so `project_source_link`
/// (`smelt-logical/src/maintenance/derive.rs`) always derives a `ScanClamp`
/// with `before = after = 0` for it — there is no construct in the current
/// pool that produces a genuinely nonzero margin. That means this test
/// cannot exercise an error in margin *derivation* itself (there is no
/// nonzero margin here to derive correctly or incorrectly). What it does
/// exercise, at that zero margin: an off-by-one in how the clamp gets turned
/// into the actual scan predicate. `just_inside` sits exactly at the window
/// start and must be read; `just_outside` sits one day before the window
/// start and must not be — an under- or over-derived scan predicate (e.g. an
/// inclusive/exclusive boundary mistake) would flip one of these two
/// assertions even though the margin itself is zero on both sides. A future
/// pool with a genuinely nonzero clamp (e.g. a lookback join) would extend
/// this same test to cover margin derivation proper.
#[test]
fn boundary_rows_within_reach_are_reflected() {
    let mut runner = TestRunner::deterministic();
    let pool = RecipePool {
        constructs: vec![ConstructKind::AdditiveAgg],
    };
    let recipe = arb_recipe(pool).new_tree(&mut runner).unwrap().current();

    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project = stage_recipe(&recipe, &tmp).expect("stage recipe");

    let verdict = classify(&project, &recipe).expect("classify");
    let plan = match verdict {
        Verdict::Admitted(plan) => plan,
        Verdict::Refused(diags) => {
            panic!("expected additive-agg append-only recipe to admit: {diags:#?}")
        }
    };
    let clamp = scan_clamp_for(&plan, &recipe.source.name).unwrap_or_else(|| {
        panic!(
            "expected an admitted NewData scan clamp for source {:?}: {plan:#?}",
            recipe.source.name
        )
    });

    let window = (
        chrono::NaiveDate::from_ymd_opt(2024, 1, 10).expect("valid date"),
        chrono::NaiveDate::from_ymd_opt(2024, 1, 11).expect("valid date"),
    );
    let mut next_id = 1_i64;
    let boundary = boundary_rows_for(clamp, window, &mut next_id);

    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(insert_row(&project, &recipe, &boundary.just_inside))
        .expect("insert just-inside row");
    rt.block_on(insert_row(&project, &recipe, &boundary.just_outside))
        .expect("insert just-outside row");

    let mut request = base_request("dev");
    request.start = Some(window.0.format("%Y-%m-%d").to_string());
    request.end = Some(window.1.format("%Y-%m-%d").to_string());
    rt.block_on(project.run_quiet("boundary-run", request))
        .expect("triggering run over the boundary window");

    let conn = project.connect().expect("connect for read-back");
    let count: i64 = conn
        .query_row(
            &format!(
                "SELECT COUNT(*) FROM main.{} WHERE {} = DATE '{}'",
                recipe.model_name,
                recipe.source.clock_column,
                boundary.just_inside.d.format("%Y-%m-%d"),
            ),
            [],
            |row| row.get(0),
        )
        .expect("count rows for the boundary just-inside day");
    assert!(
        count > 0,
        "a just-inside-reach row (day {}) must appear in the maintained output after the \
         triggering run over {window:?} — an under-derived clamp would drop it",
        boundary.just_inside.d,
    );

    let outside_count: i64 = conn
        .query_row(
            &format!(
                "SELECT COUNT(*) FROM main.{} WHERE {} = DATE '{}'",
                recipe.model_name,
                recipe.source.clock_column,
                boundary.just_outside.d.format("%Y-%m-%d"),
            ),
            [],
            |row| row.get(0),
        )
        .expect("count rows for the boundary just-outside day");
    assert_eq!(
        outside_count, 0,
        "a just-outside-reach row (day {}) must NOT appear in the maintained output after the \
         triggering run over {window:?} — an over-derived (or off-by-one) scan predicate would \
         read it even though it lies outside the derived reach",
        boundary.just_outside.d,
    );
}
