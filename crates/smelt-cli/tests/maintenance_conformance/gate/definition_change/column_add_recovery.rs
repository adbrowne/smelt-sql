//! Phase 9: definition-change steps — `ConformanceStep::RewriteModel`
//! (`docs/plans/20260712-generative-maintenance-conformance.md` Phase 9;
//! `definition_deltas.md` §"The verdict per column group"). Asserts TODAY's
//! contract only: whatever technique executes for a window always compiles
//! and runs the model's CURRENT on-disk SQL (`link_c_harness::LinkCProject`'s
//! per-run re-discovery), so a rewrite followed by a re-run of the affected
//! window(s) recovers full equivalence against the REWRITTEN body's own
//! oracle. This is deliberately NOT the spec's unbuilt `SkeletonAdd`/
//! `PureBackfill`/`UpstreamRederive` definition-change classification — see
//! `schedule_gen::ConformanceStep::RewriteModel`'s doc comment.

use proptest::strategy::{Strategy, ValueTree};
use proptest::test_runner::TestRunner;

use super::super::partition_pool::{drive_and_assert, stage_recipe};
use smelt_maintenance_testkit::recipe::{arb_recipe, ConstructKind, ModelEdit, RecipePool};
use smelt_maintenance_testkit::schedule_gen::{ConformanceSchedule, ConformanceStep, GenRow};
use smelt_maintenance_testkit::verdict::{classify, Verdict};

/// `column_add_between_runs_recovers_equivalence` (plan Phase 9 TDD list):
/// schedule: runs → `RewriteModel` (add integer payload column) → catch-up
/// runs; final state equals the oracle of the NEW body over full S.
///
/// The catch-up is a `FullRefreshRun`, not a plain windowed re-run: today's
/// windowed (DELETE+INSERT) incremental path never persists a deployed
/// schema snapshot (`execute.rs`'s "Save deployed schema for full-refresh
/// models" comment marks that as full-refresh-only), so
/// `schema_evolution::check_and_migrate`'s ALTER-TABLE detection always sees
/// `FirstDeployment` for an incremental model and never fires — a windowed
/// re-run against an already-materialized table with a changed column shape
/// hits a raw DuckDB column-count mismatch, not a graceful recompute. A
/// `full_refresh` run (`DROP`+recreate, taking the non-incremental
/// materialization arm — `execute.rs`'s `(Some(_inc), None, _, _)` "no time
/// window ... fall back to full refresh" arm) sidesteps this cleanly: it is
/// today's actual recovery path, matching the interval-store hash
/// invalidation's own "the next run recovers equivalence" framing (Phase
/// 9's Goal) rather than the unbuilt `PureBackfill` classification.
#[test]
fn column_add_between_runs_recovers_equivalence() {
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
    let w2_start = w1_end;
    let w2_end = w2_start + chrono::Duration::days(1);
    let w3_start = w2_end;
    let w3_end = w3_start + chrono::Duration::days(1);

    let schedule = ConformanceSchedule(vec![
        // Two windows run under the ORIGINAL body.
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
        // Definition change: add a derived payload column, same skeleton.
        ConformanceStep::RewriteModel {
            edit: ModelEdit::AddPayloadColumn,
        },
        // Catch-up: a full-refresh run recomputes the WHOLE table under the
        // rewritten body over the current full source contents — today's
        // actual recovery path (see this test's doc comment).
        ConformanceStep::FullRefreshRun,
        // A subsequent ordinary windowed run must keep working post-recovery
        // — the rewrite's effect must persist, not just paper over the one
        // full-refresh run.
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
    rt.block_on(drive_and_assert(&project, &recipe, &schedule))
        .expect(
            "a column-add RewriteModel followed by a full-refresh catch-up must recover full \
             equivalence against the rewritten body's own oracle, and stay recovered through \
             a subsequent windowed run",
        );
}
