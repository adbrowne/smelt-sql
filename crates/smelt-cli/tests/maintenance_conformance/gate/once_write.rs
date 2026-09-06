//! The once-write column family: its constant-payload schedule plus the pool, NULL-payload, fallback, multi-candidate, decomposed-fold, and state-column-hiding cases.

use proptest::strategy::{Strategy, ValueTree};
use proptest::test_runner::TestRunner;

use super::keyed_oracle::{
    all_physical_column_names, assert_downstream_hides_state, classify_keyed,
    drive_keyed_and_assert,
};
use super::keyed_support::{
    insert_row_keyed, keyed_case_count, stage_keyed_recipe, stage_keyed_recipe_with_downstream,
};
use smelt_logical::maintenance::Technique;
use smelt_maintenance_testkit::link_c_harness::{base_request, LinkCProject};
use smelt_maintenance_testkit::oracle::multiset_equal_via_backend;
use smelt_maintenance_testkit::recipe::{
    arb_keyed_schedule, arb_once_write_null_schedule, KeyedCombiner, KeyedRecipe, KeyedSchedule,
};
use smelt_maintenance_testkit::render;
use smelt_maintenance_testkit::schedule_gen::GenRow;

/// The once-write family's dedicated constant-payload schedule (shared by
/// [`once_write_pool_upholds_end_state_equivalence`] and phase 8's
/// `once_write_fallback_pool_upholds_end_state_equivalence`/
/// `once_write_multi_candidate_pool_upholds_end_state_equivalence`): the
/// shared key `1` recurs across windows with the SAME value throughout —
/// the once-write provenance proof's own world-fact precondition
/// (`incremental_shapes.md` §"The column-family catalogue") — plus a late
/// redelivery of the already-merged first window.
pub(crate) fn once_write_constant_payload_schedule() -> KeyedSchedule {
    let d1 = chrono::NaiveDate::from_ymd_opt(2024, 1, 1).expect("valid date");
    let d2 = chrono::NaiveDate::from_ymd_opt(2024, 1, 2).expect("valid date");
    KeyedSchedule(vec![
        smelt_maintenance_testkit::recipe::KeyedRunWindow {
            start: d1,
            end: d1 + chrono::Duration::days(1),
            rows: vec![GenRow {
                d: d1,
                id: 1,
                val: Some(7),
            }],
        },
        // The shared key `1` recurs with the SAME value — the once-write
        // world-fact holds by construction, so a `COALESCE`-based
        // first-write-wins merge equals the full-refresh oracle (`MAX(val)`
        // over a single distinct value is that value; a fallback/second
        // candidate over the same single distinct value resolves the same
        // way).
        smelt_maintenance_testkit::recipe::KeyedRunWindow {
            start: d2,
            end: d2 + chrono::Duration::days(1),
            rows: vec![
                GenRow {
                    d: d2,
                    id: 1,
                    val: Some(7),
                },
                GenRow {
                    d: d2,
                    id: 2,
                    val: Some(42),
                },
            ],
        },
        // Late redelivery of the ALREADY-MERGED first window, replaying the
        // same rows with the same values — the world-fact-preserving
        // direction of "the first-written value survives". The oracle IS
        // consulted here: the once-write merge re-applied against an
        // already-reflected delta is a no-op, so the maintained state must
        // still equal the full-refresh oracle over the (now
        // duplicate-carrying) source.
        smelt_maintenance_testkit::recipe::KeyedRunWindow {
            start: d1,
            end: d1 + chrono::Duration::days(1),
            rows: vec![GenRow {
                d: d1,
                id: 1,
                val: Some(7),
            }],
        },
    ])
}

/// Phase 4 (`docs/plans/20260809-keyed-frontier.md`): the once-write family
/// (`COALESCE(MAX(val))`, declared-FD-backed —
/// [`KeyedRecipe::new_window_forward_once_write`]) upholds end-state
/// equivalence across a genuine key-recurrence schedule
/// ([`once_write_constant_payload_schedule`]) — reuses the same
/// `drive_keyed_and_assert`/`STracker` oracle machinery every other keyed
/// combiner family runs through.
#[tokio::test]
async fn once_write_pool_upholds_end_state_equivalence() {
    let recipe = KeyedRecipe::new_window_forward_once_write();
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project = stage_keyed_recipe(&recipe, &tmp).expect("stage once-write keyed recipe");

    let plan = classify_keyed(&project, &recipe).expect("classify once-write keyed recipe");
    assert!(
        !plan.cells.is_empty(),
        "expected the once-write keyed recipe to admit at least one cell: {plan:#?}"
    );
    assert!(
        plan.cells
            .iter()
            .any(|c| c.technique == Technique::KeyedFold),
        "expected a KeyedFold cell for the declared-FD-backed once-write column: {plan:#?}"
    );

    let schedule = once_write_constant_payload_schedule();

    drive_keyed_and_assert(&project, &recipe, &schedule)
        .await
        .expect("once-write keyed schedule must uphold end-state equivalence");
}

/// `once_write_null_pool_upholds_end_state_equivalence`
/// (`docs/outcomes/20260815-keyed-grain-residue/phases/05-plan.md` test 4):
/// a deterministic proptest sample over [`arb_once_write_null_schedule`]
/// crossed with all three once-write spellings (`OnceWrite`,
/// `OnceWriteFallback`, `OnceWriteMultiCandidate`), each staged and driven
/// through the real `execute_project` pipeline by [`drive_keyed_and_assert`],
/// asserting the `STracker` full-refresh oracle after every window. This is
/// the test that replaces `once_write_null_payload_then_value_upholds_
/// equivalence`'s hand-written case as the *proof* of the once-write
/// family's NULL-preservation obligation.
#[tokio::test]
async fn once_write_null_pool_upholds_end_state_equivalence() {
    let mut runner = TestRunner::deterministic();
    let schedule_strat = arb_once_write_null_schedule();

    let combiners = [
        KeyedCombiner::OnceWrite,
        KeyedCombiner::OnceWriteFallback,
        KeyedCombiner::OnceWriteMultiCandidate,
        KeyedCombiner::OnceWriteKeyFallback,
    ];

    let mut cases = 0;
    for i in 0..10 {
        let schedule = schedule_strat.new_tree(&mut runner).unwrap().current();
        for combiner in combiners {
            let recipe = KeyedRecipe::new_window_forward_once_write_with(combiner);
            let tmp = tempfile::TempDir::new().expect("tempdir");
            let project = stage_keyed_recipe(&recipe, &tmp).unwrap_or_else(|e| {
                panic!("case {i} {combiner:?}: failed to stage once-write keyed recipe: {e}")
            });

            let plan = classify_keyed(&project, &recipe).unwrap_or_else(|e| {
                panic!("case {i} {combiner:?}: classify once-write keyed recipe failed: {e}")
            });
            assert!(
                !plan.cells.is_empty(),
                "case {i} {combiner:?}: expected the once-write keyed recipe to admit at \
                 least one cell: {plan:#?}"
            );

            drive_keyed_and_assert(&project, &recipe, &schedule)
                .await
                .unwrap_or_else(|e| {
                    panic!(
                        "case {i} {combiner:?}: once-write NULL schedule {schedule:?} \
                         equivalence check failed: {e}"
                    )
                });
            cases += 1;
        }
    }
    assert!(
        cases > 0,
        "deterministic once-write NULL sample produced zero cases — generator regression"
    );
}

/// Phase 8 task 4: the once-write family's fallback-bearing spelling
/// (`COALESCE(MAX(val), 0)`, [`KeyedCombiner::OnceWriteFallback`]) upholds
/// end-state equivalence over the same constant-payload world-fact
/// schedule — this spelling admits onto hidden `(value, written)` state
/// (`decompose_once_write`) rather than the bare spelling's stateless
/// merge, so this is the state-bearing family's own end-to-end DuckDB
/// witness.
#[tokio::test]
async fn once_write_fallback_pool_upholds_end_state_equivalence() {
    let recipe = KeyedRecipe::new_window_forward_once_write_with(KeyedCombiner::OnceWriteFallback);
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project =
        stage_keyed_recipe(&recipe, &tmp).expect("stage once-write-fallback keyed recipe");

    let plan =
        classify_keyed(&project, &recipe).expect("classify once-write-fallback keyed recipe");
    assert!(
        !plan.cells.is_empty(),
        "expected the once-write-fallback keyed recipe to admit at least one cell: {plan:#?}"
    );
    assert!(
        plan.cells
            .iter()
            .any(|c| c.technique == Technique::KeyedFold),
        "expected a KeyedFold cell for the declared-FD-backed once-write-fallback column: \
         {plan:#?}"
    );

    let schedule = once_write_constant_payload_schedule();

    drive_keyed_and_assert(&project, &recipe, &schedule)
        .await
        .expect("once-write-fallback keyed schedule must uphold end-state equivalence");
}

/// Human decision (c) (`docs/outcomes/20260904-decided-gap-residue`
/// outcome.md Decision log): the once-write family's route-2 `unique_key`
/// skip (`COALESCE(MAX(<key>), 0)`, [`KeyedCombiner::OnceWriteKeyFallback`])
/// upholds end-state equivalence with **no** declared functional
/// dependency — the rendered model file is asserted to carry no
/// `functional_dependencies:` block at all, the absent block being the
/// witness that the FD-free route is what admits this column.
#[tokio::test]
async fn once_write_key_fallback_pool_upholds_end_state_equivalence() {
    let recipe =
        KeyedRecipe::new_window_forward_once_write_with(KeyedCombiner::OnceWriteKeyFallback);
    assert!(
        !render::render_keyed_model_file(&recipe).contains("functional_dependencies:"),
        "the unique_key-member route must declare no functional_dependencies block"
    );
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project =
        stage_keyed_recipe(&recipe, &tmp).expect("stage once-write-key-fallback keyed recipe");

    let plan =
        classify_keyed(&project, &recipe).expect("classify once-write-key-fallback keyed recipe");
    assert!(
        !plan.cells.is_empty(),
        "expected the once-write-key-fallback keyed recipe to admit at least one cell: {plan:#?}"
    );
    assert!(
        plan.cells
            .iter()
            .any(|c| c.technique == Technique::KeyedFold),
        "expected a KeyedFold cell for the FD-free once-write-key-fallback column: {plan:#?}"
    );

    let schedule = once_write_constant_payload_schedule();

    drive_keyed_and_assert(&project, &recipe, &schedule)
        .await
        .expect("once-write-key-fallback keyed schedule must uphold end-state equivalence");
}

/// Phase 8 task 4: the once-write family's multi-candidate spelling
/// (`COALESCE(MAX(val), MIN(val))`, [`KeyedCombiner::OnceWriteMultiCandidate`])
/// upholds end-state equivalence over the same constant-payload world-fact
/// schedule — each candidate admits its own hidden `(value, written)` state
/// pair (`decompose_once_write`).
#[tokio::test]
async fn once_write_multi_candidate_pool_upholds_end_state_equivalence() {
    let recipe =
        KeyedRecipe::new_window_forward_once_write_with(KeyedCombiner::OnceWriteMultiCandidate);
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project =
        stage_keyed_recipe(&recipe, &tmp).expect("stage once-write-multi-candidate keyed recipe");

    let plan = classify_keyed(&project, &recipe)
        .expect("classify once-write-multi-candidate keyed recipe");
    assert!(
        !plan.cells.is_empty(),
        "expected the once-write-multi-candidate keyed recipe to admit at least one cell: \
         {plan:#?}"
    );
    assert!(
        plan.cells
            .iter()
            .any(|c| c.technique == Technique::KeyedFold),
        "expected a KeyedFold cell for the declared-FD-backed once-write-multi-candidate \
         column: {plan:#?}"
    );

    let schedule = once_write_constant_payload_schedule();

    drive_keyed_and_assert(&project, &recipe, &schedule)
        .await
        .expect("once-write-multi-candidate keyed schedule must uphold end-state equivalence");
}

/// Phase 8 task 4: `AVG(val)`/`STDDEV_SAMP(val)` window-forward keyed
/// recipes, driven through [`drive_keyed_and_assert`] over generated
/// [`arb_keyed_schedule`] schedules, equal the `STracker` oracle after every
/// window. Iterates the two decomposed-fold combiners explicitly (not
/// draw-dependent) — `arb_keyed_combiner()` was widened by this phase to
/// include both, so `keyed_pool_upholds_end_state_equivalence` already
/// exercises them too, but only probabilistically; this test guarantees
/// both get dedicated generative coverage every run.
#[test]
fn decomposed_fold_pool_upholds_end_state_equivalence() {
    let n = keyed_case_count();
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");

    for combiner in [
        KeyedCombiner::DecomposedAvg,
        KeyedCombiner::DecomposedStddev,
    ] {
        let mut runner = TestRunner::deterministic();
        let schedule_strat = arb_keyed_schedule();
        let recipe = KeyedRecipe::new_window_forward(combiner);

        for i in 0..n {
            let schedule = schedule_strat.new_tree(&mut runner).unwrap().current();

            let tmp = tempfile::TempDir::new().expect("tempdir");
            let project = stage_keyed_recipe(&recipe, &tmp).unwrap_or_else(|e| {
                panic!("case {i} ({combiner:?}): recipe {recipe:?} failed to stage: {e}")
            });

            let plan = classify_keyed(&project, &recipe).unwrap_or_else(|e| {
                panic!("case {i} ({combiner:?}): recipe {recipe:?} classify failed: {e}")
            });
            assert!(
                !plan.cells.is_empty(),
                "case {i} ({combiner:?}): recipe {recipe:?} admitted zero cells — \
                 generator/derivation regression"
            );

            rt.block_on(drive_keyed_and_assert(&project, &recipe, &schedule))
                .unwrap_or_else(|e| {
                    panic!(
                        "case {i} ({combiner:?}): recipe {recipe:?} schedule {schedule:?} \
                         equivalence check failed: {e}"
                    )
                });
        }
    }
}

/// Phase 8 task 5: for each new state-bearing family plus `OrderMonotone`,
/// the maintained table's `information_schema` reports at least one
/// `__`-marked physical column after a real run — a vacuity guard for
/// [`downstream_select_star_consumer_sees_only_presented_columns`]'s hiding
/// assertions (a recipe whose table never actually carried hidden state
/// would make that test's "no `__` columns downstream" check trivially
/// true rather than a real proof).
#[tokio::test]
async fn state_bearing_recipes_physically_carry_state_columns() {
    let mut runner = TestRunner::deterministic();

    for combiner in [
        KeyedCombiner::OrderMonotone,
        KeyedCombiner::DecomposedAvg,
        KeyedCombiner::DecomposedStddev,
    ] {
        let recipe = KeyedRecipe::new_window_forward(combiner);
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let project = stage_keyed_recipe(&recipe, &tmp)
            .unwrap_or_else(|e| panic!("{combiner:?}: failed to stage: {e}"));
        let schedule = arb_keyed_schedule()
            .new_tree(&mut runner)
            .unwrap()
            .current();
        drive_keyed_and_assert(&project, &recipe, &schedule)
            .await
            .unwrap_or_else(|e| panic!("{combiner:?}: equivalence check failed: {e}"));

        let physical_columns = all_physical_column_names(&project, &recipe.model_name);
        assert!(
            physical_columns.iter().any(|c| c.contains("__")),
            "{combiner:?}: model {:?} carries zero `__`-marked physical state columns \
             (columns: {physical_columns:?}) — vacuity: the downstream hiding assertions \
             would prove nothing",
            recipe.model_name
        );
    }

    for combiner in [
        KeyedCombiner::OnceWriteFallback,
        KeyedCombiner::OnceWriteMultiCandidate,
    ] {
        let recipe = KeyedRecipe::new_window_forward_once_write_with(combiner);
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let project = stage_keyed_recipe(&recipe, &tmp)
            .unwrap_or_else(|e| panic!("{combiner:?}: failed to stage: {e}"));
        let schedule = once_write_constant_payload_schedule();
        drive_keyed_and_assert(&project, &recipe, &schedule)
            .await
            .unwrap_or_else(|e| panic!("{combiner:?}: equivalence check failed: {e}"));

        let physical_columns = all_physical_column_names(&project, &recipe.model_name);
        assert!(
            physical_columns.iter().any(|c| c.contains("__")),
            "{combiner:?}: model {:?} carries zero `__`-marked physical state columns \
             (columns: {physical_columns:?}) — vacuity: the downstream hiding assertions \
             would prove nothing",
            recipe.model_name
        );
    }
}

/// Phase 8 task 5: for each state-bearing family, a staged downstream model
/// `SELECT * FROM smelt.<model>` materializes with exactly the
/// upstream's presented columns (no `__` names) and multiset-equals the
/// upstream's presented contents after a real run
/// ([`assert_downstream_hides_state`]) — success criterion 4's end-to-end
/// witness against a real DuckDB, complementing row 4's compile-time unit
/// tests.
#[tokio::test]
async fn downstream_select_star_consumer_sees_only_presented_columns() {
    let mut runner = TestRunner::deterministic();

    for combiner in [
        KeyedCombiner::OrderMonotone,
        KeyedCombiner::DecomposedAvg,
        KeyedCombiner::DecomposedStddev,
    ] {
        let recipe = KeyedRecipe::new_window_forward(combiner);
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let project = stage_keyed_recipe_with_downstream(&recipe, &tmp)
            .unwrap_or_else(|e| panic!("{combiner:?}: failed to stage with downstream: {e}"));
        let schedule = arb_keyed_schedule()
            .new_tree(&mut runner)
            .unwrap()
            .current();
        drive_keyed_and_assert(&project, &recipe, &schedule)
            .await
            .unwrap_or_else(|e| panic!("{combiner:?}: equivalence check failed: {e}"));

        assert_downstream_hides_state(&project, &recipe.model_name).await;
    }

    for combiner in [
        KeyedCombiner::OnceWriteFallback,
        KeyedCombiner::OnceWriteMultiCandidate,
    ] {
        let recipe = KeyedRecipe::new_window_forward_once_write_with(combiner);
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let project = stage_keyed_recipe_with_downstream(&recipe, &tmp)
            .unwrap_or_else(|e| panic!("{combiner:?}: failed to stage with downstream: {e}"));
        let schedule = once_write_constant_payload_schedule();
        drive_keyed_and_assert(&project, &recipe, &schedule)
            .await
            .unwrap_or_else(|e| panic!("{combiner:?}: equivalence check failed: {e}"));

        assert_downstream_hides_state(&project, &recipe.model_name).await;
    }
}

/// The once-write family's NULL-payload direction — the case a total
/// (fallback-carrying) projection would break. A key's first window carries
/// ONLY a NULL payload; a later window delivers the real value. The
/// first-non-null merge (`COALESCE(target, delta)`) must let the real value
/// through, matching the full-refresh oracle. Had the projection carried a
/// literal fallback (`COALESCE(MAX(val), -1)`), the first window would have
/// written `-1` into the target and locked it in forever — the divergence
/// the classifier's NULL-preservation obligation refuses
/// (`incremental_shapes.md` §"The column-family catalogue").
///
/// Retained as a PINNED MINIMAL WITNESS, not the direction's proof: since
/// `GenRow::val` became nullable
/// (`docs/outcomes/20260815-keyed-grain-residue/phases/05-plan.md`), the
/// proof lives in the generated pool
/// (`once_write_null_pool_upholds_end_state_equivalence`, below) via
/// `arb_once_write_null_schedule`. This hand-written case stays as a small,
/// readable, exact-shape fixture for the one scenario the generator's own
/// doc comment names explicitly. The oracle here is the same full-refresh
/// body every other keyed case asserts against, evaluated over the physical
/// source table: the schedule is insert-only and every inserted row
/// precedes the run that processes it, so `S` after each run IS the whole
/// source table.
#[tokio::test]
async fn once_write_null_payload_then_value_upholds_equivalence() {
    let recipe = KeyedRecipe::new_window_forward_once_write();
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project = stage_keyed_recipe(&recipe, &tmp).expect("stage once-write keyed recipe");

    let source_table = format!("main.sources_{}", recipe.source.name);
    let oracle_sql = render::render_keyed_oracle_body_over(&recipe, &source_table);
    let maintained_sql = format!("SELECT * FROM main.{}", recipe.model_name);

    let d1 = chrono::NaiveDate::from_ymd_opt(2024, 1, 1).expect("valid date");
    let d2 = chrono::NaiveDate::from_ymd_opt(2024, 1, 2).expect("valid date");

    /// One staged row of the NULL-bearing schedule: `(key, payload)`, where
    /// `None` stages a NULL payload. Kept as a local tuple rather than
    /// `GenRow` for this pinned fixture's own hand-rolled insert loop below.
    type NullableRow = (i64, Option<i64>);

    // Each entry is one run window: the day it covers and the rows staged
    // into the driving source before it runs.
    let windows: Vec<(chrono::NaiveDate, Vec<NullableRow>)> =
        vec![(d1, vec![(1, None)]), (d2, vec![(1, Some(7))])];

    for (i, (day, rows)) in windows.iter().enumerate() {
        {
            let conn = project.connect().expect("connect");
            for (id, val) in rows {
                let val_sql = val.map_or_else(|| "NULL".to_string(), |v| v.to_string());
                conn.execute(
                    &format!(
                        "INSERT INTO {source_table} VALUES (DATE '{}', {id}, {val_sql})",
                        day.format("%Y-%m-%d")
                    ),
                    [],
                )
                .expect("stage source row");
            }
        }

        let mut request = base_request("dev");
        request.start = Some(day.format("%Y-%m-%d").to_string());
        request.end = Some(
            (*day + chrono::Duration::days(1))
                .format("%Y-%m-%d")
                .to_string(),
        );
        project
            .run_quiet(&format!("once-write-null-run-{i}"), request)
            .await
            .expect("run once-write window");

        let backend = project.backend().await.expect("backend");
        let equal = multiset_equal_via_backend(backend.as_ref(), &maintained_sql, &oracle_sql)
            .await
            .expect("compare maintained state to the full-refresh oracle");
        assert!(
            equal,
            "once-write NULL-payload equivalence violated after window {i}: maintained \
             ({maintained_sql:?}) != oracle ({oracle_sql:?})"
        );
    }
}

/// The once-write family's own distinguishing mechanics
/// (`docs/specs/incremental_shapes.md` §"The column-family catalogue" —
/// `COALESCE(target, delta)`, "the target's value wins once set"): a later
/// redelivery of an already-folded window carrying a DIFFERENT value for
/// the same key must NOT overwrite the first-written value — unlike the
/// extremal-fold family's `MAX`, which would take the greater of the two.
/// This is a technique-mechanics probe (design doc §7 "plan-claim probes"),
/// not an end-state-equivalence assertion: deliberately redelivering a
/// DIFFERENT value violates the once-write provenance proof's own
/// world-fact precondition (the declared FD asserts `val` is a genuine
/// per-key constant), so the full-refresh oracle is not consulted here —
/// [`once_write_pool_upholds_end_state_equivalence`] above covers the
/// world-fact-preserving equivalence claim.
#[tokio::test]
async fn once_write_merge_keeps_first_value_despite_later_differing_redelivery() {
    let recipe = KeyedRecipe::new_window_forward_once_write();
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project = stage_keyed_recipe(&recipe, &tmp).expect("stage once-write keyed recipe");

    let plan = classify_keyed(&project, &recipe).expect("classify once-write keyed recipe");
    assert!(
        !plan.cells.is_empty(),
        "expected the once-write keyed recipe to admit at least one cell: {plan:#?}"
    );

    let d = chrono::NaiveDate::from_ymd_opt(2024, 1, 1).expect("valid date");
    insert_row_keyed(
        &project,
        &recipe,
        &GenRow {
            d,
            id: 1,
            val: Some(7),
        },
    )
    .expect("insert first row");

    let mut request = base_request("dev");
    request.start = Some("2024-01-01".to_string());
    request.end = Some("2024-01-02".to_string());
    project
        .run_quiet("keyed-once-write-1", request.clone())
        .await
        .expect("first fold of the window must succeed");

    let once_val_after_first = once_write_stored_value(&project, &recipe, 1)
        .await
        .expect("id=1 present after first run");
    assert_eq!(
        once_val_after_first, 7,
        "expected the first-written value to be stored"
    );

    // A late redelivery carrying a DIFFERENT (larger) value for the SAME
    // key, within the SAME already-folded window.
    insert_row_keyed(
        &project,
        &recipe,
        &GenRow {
            d,
            id: 1,
            val: Some(99),
        },
    )
    .expect("insert differing late row");

    // Once-write grades `Grade::Idempotent` (no reprocessing ledger) — the
    // redelivery must succeed, not refuse with `KeyedReprocessedWindow`.
    project
        .run_quiet("keyed-once-write-2", request)
        .await
        .expect(
            "re-running an already-folded once-write keyed window must succeed — \
             idempotent-graded cells carry no reprocessing ledger",
        );

    let once_val_after_redelivery = once_write_stored_value(&project, &recipe, 1)
        .await
        .expect("id=1 present after redelivery");
    assert_eq!(
        once_val_after_redelivery, 7,
        "the once-write merge (COALESCE(target, delta)) must keep the FIRST-written value \
         (7), never overwrite with the later-redelivered value (99) — unlike the \
         extremal-fold family's MAX, which would take 99"
    );
}

/// Read back the once-write recipe's stored `once_val` for one key —
/// `once_write_merge_keeps_first_value_despite_later_differing_redelivery`'s
/// own small helper (not reused elsewhere, kept local rather than added to
/// the shared oracle/snapshot helpers above).
pub(crate) async fn once_write_stored_value(
    project: &LinkCProject,
    recipe: &KeyedRecipe,
    id: i64,
) -> anyhow::Result<i64> {
    let backend = project.backend().await?;
    let sql = format!(
        "SELECT once_val FROM main.{} WHERE id = {id}",
        recipe.model_name
    );
    let batches = backend.execute_sql(&sql).await?;
    let mut value: Option<i64> = None;
    for batch in &batches {
        for row_idx in 0..batch.num_rows() {
            let text = arrow::util::display::array_value_to_string(batch.column(0), row_idx)?;
            value = Some(
                text.parse()
                    .map_err(|e| anyhow::anyhow!("once_val not an integer ({text:?}): {e}"))?,
            );
        }
    }
    value.ok_or_else(|| anyhow::anyhow!("no row for id={id} in {}", recipe.model_name))
}
