//! State-deletion conformance leg
//! (`docs/outcomes/20260816-state-residency/phases/08-plan.md`;
//! `docs/specs/state.md`'s residency rule): deleting `.smelt/` mid-sequence,
//! and separately starting from a fresh clone of the project directory,
//! never breaks the equivalence invariant — for keyed additive folds (the
//! engine-resident `_smelt_ledger`'s never-fold-twice guarantee) and
//! idempotent-graded region-recompute models (the engine-resident
//! `_smelt_frontier` record, phase 4/7's fused write). This is the
//! end-to-end proof that criterion 2's "engine-resident" claim is actually
//! load-bearing: before phase 4, both tables lived in
//! `.smelt/reconciliation.json`, and deleting it silently reset the
//! never-fold-twice bookkeeping.

use chrono::NaiveDate;
use proptest::strategy::{Strategy, ValueTree};
use proptest::test_runner::TestRunner;

use smelt_maintenance_testkit::link_c_harness::{base_request, LinkCProject};
use smelt_maintenance_testkit::recipe::{
    arb_recipe, ConstructKind, KeyedCombiner, KeyedRecipe, RecipePool,
};
use smelt_maintenance_testkit::s_tracker::STracker;
use smelt_maintenance_testkit::schedule_gen::{
    arb_schedule_for, ConformanceStep, GenRow, StateResidencyOp,
};
use smelt_maintenance_testkit::verdict::{classify, Verdict};

use crate::gate::{
    assert_equivalence, classify_keyed, drive_and_assert, drive_keyed_and_assert_with_state_ops,
    drop_state_dir, insert_row_keyed, stage_keyed_recipe, stage_recipe,
};

fn case_count() -> usize {
    std::env::var("SMELT_CONFORMANCE_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(4)
}

fn ledger_row_count(project: &LinkCProject, model_name: &str) -> i64 {
    let conn = project.connect().expect("connect to read _smelt_ledger");
    conn.query_row(
        &format!("SELECT COUNT(*) FROM main._smelt_ledger WHERE model_name = '{model_name}'"),
        [],
        |row| row.get(0),
    )
    .expect("count ledger rows")
}

type FrontierRow = (String, String, String, String, String, String);

fn frontier_rows(project: &LinkCProject, model_name: &str) -> Vec<FrontierRow> {
    let conn = project.connect().expect("connect to read _smelt_frontier");
    let mut stmt = conn
        .prepare(&format!(
            "SELECT model_name, grp, input_name, delta_id, region_start, region_end \
             FROM main._smelt_frontier WHERE model_name = '{model_name}' \
             ORDER BY region_start, grp, input_name"
        ))
        .expect("prepare frontier read");
    stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, String>(5)?,
        ))
    })
    .expect("query frontier rows")
    .collect::<Result<Vec<_>, _>>()
    .expect("collect frontier rows")
}

/// `drop_state_dir_step_actually_removes_the_directory` (test 1): anti-
/// vacuity. Without this, the whole residency leg could pass while testing
/// nothing — an unobservable no-op step (`docs/outcomes/
/// 20260816-state-residency/phases/08-plan.md` test list item 1).
#[test]
fn drop_state_dir_step_actually_removes_the_directory() {
    let mut runner = TestRunner::deterministic();
    let pool = RecipePool {
        constructs: vec![ConstructKind::AdditiveAgg],
    };
    let recipe = arb_recipe(pool).new_tree(&mut runner).unwrap().current();

    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project = stage_recipe(&recipe, &tmp).expect("stage recipe");

    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let d1 = NaiveDate::from_ymd_opt(2024, 1, 1).expect("valid date");
    let d2 = d1 + chrono::Duration::days(1);
    let schedule = smelt_maintenance_testkit::schedule_gen::ConformanceSchedule(vec![
        ConformanceStep::RunWindow {
            start: d1,
            end: d2,
            rows: vec![GenRow {
                d: d1,
                id: 1,
                val: 10,
            }],
        },
    ]);
    rt.block_on(drive_and_assert(&project, &recipe, &schedule))
        .expect("drive a single RunWindow so .smelt/ is created");

    assert!(
        project.project_dir.join(".smelt").exists(),
        "expected .smelt/ to exist after a RunWindow"
    );

    drop_state_dir(&project).expect("drop .smelt/");

    assert!(
        !project.project_dir.join(".smelt").exists(),
        "DropStateDir must actually remove .smelt/ — an unobservable no-op step would pass \
         forever while proving nothing"
    );
}

/// `keyed_additive_fold_survives_state_dir_deletion` (test 2, the
/// flagship): an additive-graded keyed fold's engine-resident `_smelt_ledger`
/// rows — and the never-fold-twice refusal they back — survive a `.smelt/`
/// deletion. Redelivering the already-folded window must STILL refuse
/// (`KeyedReprocessedWindow`) after the drop; if `.smelt/` deletion silently
/// reset the ledger, the redelivery would wrongly succeed and double-count.
#[tokio::test]
async fn keyed_additive_fold_survives_state_dir_deletion() {
    let recipe = KeyedRecipe::new_window_forward(KeyedCombiner::Additive);
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project = stage_keyed_recipe(&recipe, &tmp).expect("stage keyed recipe");

    let plan = classify_keyed(&project, &recipe).expect("classify additive keyed recipe");
    assert!(
        !plan.cells.is_empty(),
        "expected the additive keyed recipe to admit at least one cell: {plan:#?}"
    );

    let d = NaiveDate::from_ymd_opt(2024, 1, 1).expect("valid date");
    insert_row_keyed(&project, &recipe, &GenRow { d, id: 1, val: 5 }).expect("insert row");

    let mut request = base_request("dev");
    request.start = Some("2024-01-01".to_string());
    request.end = Some("2024-01-02".to_string());
    project
        .run_quiet("keyed-residency-1", request.clone())
        .await
        .expect("first fold of the window must succeed");

    let ledger_rows_before = ledger_row_count(&project, &recipe.model_name);
    assert!(
        ledger_rows_before > 0,
        "expected the additive fold to record ledger rows before the drop"
    );

    std::fs::remove_dir_all(project.project_dir.join(".smelt")).expect("drop .smelt/ mid-sequence");

    let ledger_rows_after_drop = ledger_row_count(&project, &recipe.model_name);
    assert_eq!(
        ledger_rows_before, ledger_rows_after_drop,
        "the engine-resident _smelt_ledger rows must be untouched by a .smelt/ deletion"
    );

    // Re-deliver the SAME window: never-fold-twice must STILL refuse — the
    // ledger's engine residency, not `.smelt/`, is what makes this a real
    // guarantee.
    let rerun = project.run_quiet("keyed-residency-2", request).await;
    let err = rerun.expect_err(
        "re-running an already-folded additive keyed window must still be refused \
         (KeyedReprocessedWindow) after a .smelt/ deletion",
    );
    let message = format!("{err:#}");
    assert!(
        message.contains("KeyedReprocessedWindow"),
        "refusal must name the diagnostic code KeyedReprocessedWindow, got: {message}"
    );
    assert!(
        message.contains("already reflected"),
        "refusal must name the never-fold-twice reason, got: {message}"
    );

    let ledger_rows_final = ledger_row_count(&project, &recipe.model_name);
    assert_eq!(
        ledger_rows_before, ledger_rows_final,
        "a refused redelivery must not add or remove ledger rows"
    );
}

/// `state_dir_deletion_mid_schedule_preserves_equivalence` (test 3):
/// generative, over the append-only `AdditiveAgg` pool — a `DropStateDir`
/// step injected at a generated index (always after the schedule's first
/// `RunWindow`, so `.smelt/` exists to delete); equivalence is asserted
/// after every subsequent run step by `drive_and_assert`'s own per-step
/// oracle check.
#[test]
fn state_dir_deletion_mid_schedule_preserves_equivalence() {
    let n = case_count();
    let mut runner = TestRunner::deterministic();
    let recipe_strat = arb_recipe(RecipePool::partition_append_only());
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");

    for i in 0..n {
        let recipe = recipe_strat.new_tree(&mut runner).unwrap().current();
        let mut schedule = arb_schedule_for(&recipe)
            .new_tree(&mut runner)
            .unwrap()
            .current();

        // Insert strictly after index 0: `arb_schedule_for` always starts
        // with a `RunWindow` (`build_schedule`'s first loop), so `.smelt/`
        // is guaranteed to exist by the time `DropStateDir` runs.
        let insert_at = (1..=schedule.0.len())
            .new_tree(&mut runner)
            .unwrap()
            .current();
        schedule.0.insert(insert_at, ConformanceStep::DropStateDir);

        let tmp = tempfile::TempDir::new().expect("tempdir");
        let project = stage_recipe(&recipe, &tmp)
            .unwrap_or_else(|e| panic!("case {i}: recipe {recipe:?} failed to stage: {e}"));

        rt.block_on(drive_and_assert(&project, &recipe, &schedule))
            .unwrap_or_else(|e| {
                panic!(
                    "case {i}: recipe {recipe:?} schedule (DropStateDir at {insert_at}) \
                     {schedule:?} equivalence check failed: {e}"
                )
            });
    }
}

/// `fresh_clone_mid_schedule_preserves_equivalence` (test 4): same
/// generative pool/seeding as test 3, but a `FreshClone` step instead —
/// distinct because the project's absolute path itself changes, catching
/// anything keyed on the old path (interval-store lookups, model-hash
/// keying, legacy-file import) that a same-path `DropStateDir` cannot.
#[test]
fn fresh_clone_mid_schedule_preserves_equivalence() {
    let n = case_count();
    let mut runner = TestRunner::deterministic();
    let recipe_strat = arb_recipe(RecipePool::partition_append_only());
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");

    for i in 0..n {
        let recipe = recipe_strat.new_tree(&mut runner).unwrap().current();
        let mut schedule = arb_schedule_for(&recipe)
            .new_tree(&mut runner)
            .unwrap()
            .current();

        let insert_at = (1..=schedule.0.len())
            .new_tree(&mut runner)
            .unwrap()
            .current();
        schedule.0.insert(insert_at, ConformanceStep::FreshClone);

        let tmp = tempfile::TempDir::new().expect("tempdir");
        let project = stage_recipe(&recipe, &tmp)
            .unwrap_or_else(|e| panic!("case {i}: recipe {recipe:?} failed to stage: {e}"));

        rt.block_on(drive_and_assert(&project, &recipe, &schedule))
            .unwrap_or_else(|e| {
                panic!(
                    "case {i}: recipe {recipe:?} schedule (FreshClone at {insert_at}) \
                     {schedule:?} equivalence check failed: {e}"
                )
            });
    }
}

/// `region_recompute_frontier_survives_state_dir_deletion` (test 5): the
/// idempotent-graded region-recompute path's per-batch `_smelt_frontier`
/// rows (phase 7's fused write) are byte-identical across a `.smelt/`
/// deletion, and a post-drop rerun of an already-recomputed region still
/// upholds equivalence.
#[tokio::test]
async fn region_recompute_frontier_survives_state_dir_deletion() {
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
        "expected the additive-agg append-only recipe to admit: {verdict:?}"
    );

    let mut tracker = STracker::new(&recipe.source);
    let d1 = NaiveDate::from_ymd_opt(2024, 1, 1).expect("valid date");
    let d2 = d1 + chrono::Duration::days(1);

    let insert = |d: NaiveDate, id: i64, val: i64| {
        let conn = project.connect().expect("connect for insert");
        conn.execute(
            &format!(
                "INSERT INTO main.sources_{} VALUES (DATE '{}', {}, {})",
                recipe.source.name,
                d.format("%Y-%m-%d"),
                id,
                val,
            ),
            [],
        )
        .expect("insert source row");
    };

    insert(d1, 1, 10);
    let snapshot = {
        let conn = project.connect().unwrap();
        smelt_maintenance_testkit::schedule_gen::read_source_snapshot(&conn, &recipe.source)
    };
    let mut r1 = base_request("dev");
    r1.start = Some(d1.format("%Y-%m-%d").to_string());
    r1.end = Some(d2.format("%Y-%m-%d").to_string());
    project
        .run_quiet("region-residency-1", r1)
        .await
        .expect("run 1");
    let k1 = tracker.record_run(d1, d2, snapshot);
    assert_equivalence(&project, &recipe, &tracker, k1)
        .await
        .expect("equivalence after run 1");

    let frontier_before = frontier_rows(&project, &recipe.model_name);
    assert!(
        !frontier_before.is_empty(),
        "expected a recorded _smelt_frontier row for the recomputed region"
    );

    std::fs::remove_dir_all(project.project_dir.join(".smelt")).expect("drop .smelt/ mid-sequence");

    let frontier_after_drop = frontier_rows(&project, &recipe.model_name);
    assert_eq!(
        frontier_before, frontier_after_drop,
        "the engine-resident _smelt_frontier rows must be byte-identical across a .smelt/ \
         deletion"
    );

    // Rerun the SAME already-recomputed region post-drop: idempotent-graded
    // region recompute must still uphold equivalence with no `.smelt/`
    // bookkeeping to lean on.
    let snapshot = {
        let conn = project.connect().unwrap();
        smelt_maintenance_testkit::schedule_gen::read_source_snapshot(&conn, &recipe.source)
    };
    let mut r2 = base_request("dev");
    r2.start = Some(d1.format("%Y-%m-%d").to_string());
    r2.end = Some(d2.format("%Y-%m-%d").to_string());
    project
        .run_quiet("region-residency-2", r2)
        .await
        .expect("rerun of an already-recomputed region after .smelt/ deletion");
    let k2 = tracker.record_run(d1, d2, snapshot);
    assert_equivalence(&project, &recipe, &tracker, k2)
        .await
        .expect("equivalence after post-drop rerun");
}

/// Wires [`drive_keyed_and_assert_with_state_ops`] into a `#[test]` — proves
/// the keyed-pool residency hook is reachable and actually exercises a
/// `StateResidencyOp` mid-schedule, over the additive combiner (the family
/// with real engine-resident state to survive).
#[tokio::test]
async fn keyed_schedule_with_residency_op_preserves_equivalence() {
    let recipe = KeyedRecipe::new_window_forward(KeyedCombiner::Additive);
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project = stage_keyed_recipe(&recipe, &tmp).expect("stage keyed recipe");

    let plan = classify_keyed(&project, &recipe).expect("classify additive keyed recipe");
    assert!(!plan.cells.is_empty());

    let base = NaiveDate::from_ymd_opt(2024, 1, 1).expect("valid date");
    let schedule = smelt_maintenance_testkit::recipe::KeyedSchedule(vec![
        smelt_maintenance_testkit::recipe::KeyedRunWindow {
            start: base,
            end: base + chrono::Duration::days(1),
            rows: vec![GenRow {
                d: base,
                id: 1,
                val: 5,
            }],
        },
        smelt_maintenance_testkit::recipe::KeyedRunWindow {
            start: base + chrono::Duration::days(1),
            end: base + chrono::Duration::days(2),
            rows: vec![GenRow {
                d: base + chrono::Duration::days(1),
                id: 2,
                val: 7,
            }],
        },
    ]);
    let mut ops = std::collections::BTreeMap::new();
    ops.insert(1_usize, StateResidencyOp::DropStateDir);

    drive_keyed_and_assert_with_state_ops(&project, &recipe, &schedule, &ops)
        .await
        .expect("keyed schedule with a DropStateDir residency op must uphold equivalence");
}
