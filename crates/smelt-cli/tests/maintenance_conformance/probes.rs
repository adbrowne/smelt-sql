//! Plan-claim probes (design doc
//! `docs/research/20260711-generative-maintenance-conformance.md` §7
//! "Plan-claim probes — checking that derived properties hold";
//! `docs/plans/20260712-generative-maintenance-conformance.md` Phase 4): a
//! direct runtime check that a derived plan claim actually holds, beyond
//! end-state equivalence alone — end-state equivalence can miss a claim
//! being wrong in a compensating way.

use chrono::NaiveDate;
use proptest::strategy::{Strategy, ValueTree};
use proptest::test_runner::TestRunner;

use smelt_logical::maintenance::{Technique, Trigger};
use smelt_maintenance_testkit::link_c_harness::base_request;
use smelt_maintenance_testkit::probes::{
    compiled_sql_matches_derived_clamp,
    rows_outside_write_window_are_byte_unchanged as probe_write_window_containment,
    technique_pins_agree_at_fixed_s as probe_technique_interchangeability, CaseContext,
    ReachabilityReport,
};
use smelt_maintenance_testkit::recipe::{
    arb_recipe, ConstructKind, KeyedCombiner, KeyedRecipe, MutableEnrichedRecipe, RecipePool,
};
use smelt_maintenance_testkit::schedule_gen::{
    arb_schedule_for, is_permutable, reorder_windows, GenRow,
};
use smelt_maintenance_testkit::verdict::{classify, Verdict};
use smelt_state::reconciliation::Region;

use crate::gate::{
    classify_keyed, classify_mixed, drive_and_assert, insert_fact_row, insert_row_keyed,
    snapshot_table_rows, stage_keyed_recipe, stage_mixed_recipe, stage_recipe,
};

/// Read back `SELECT d, id, val, attr FROM main.<model_name> ORDER BY id` —
/// every output column, ordered by the fact row's own key for stable
/// row-by-row comparison.
fn read_maintained_rows(
    project: &smelt_maintenance_testkit::link_c_harness::LinkCProject,
    recipe: &MutableEnrichedRecipe,
) -> Vec<(String, i64, i64, i64)> {
    let conn = project.connect().expect("connect for probe read-back");
    let mut stmt = conn
        .prepare(&format!(
            "SELECT CAST({d} AS VARCHAR), {id}, {val}, {attr} FROM main.{model} ORDER BY {id}",
            d = recipe.fact.clock_column,
            id = recipe.fact.key_column,
            val = recipe.fact.payload_column,
            attr = recipe.dimension.payload_column,
            model = recipe.model_name,
        ))
        .expect("prepare maintained read-back");
    stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, i64>(3)?,
        ))
    })
    .expect("query maintained rows")
    .collect::<Result<Vec<_>, _>>()
    .expect("collect maintained rows")
}

/// `dimension_mutation_recomputes_the_whole_touched_region` (rewrite of
/// `dimension_mutation_touches_only_sensitive_groups`,
/// `docs/plans/20260808-membership-sensitivity.md` Phase 3): the recipe's
/// dimension is read purely in the `JOIN`'s `ON` predicate — a row-admission
/// read — so per `incremental_models.md` §"The plan matrix" the model's
/// admitted `UpstreamMutation` cell is now membership-sensitive
/// (`Technique::DeleteInsert`), never `Technique::ColumnScopedMerge`
/// (Phase 1's review checklist: "membership cells cannot receive
/// ColumnScopedMerge"; Phase 2's own reachability verdict: no fixture in
/// this workspace reaches `ColumnScopedMerge` anymore). The ORIGINAL
/// premise this probe checked — "dim mutation touches only the
/// value-sensitive columns" — is no longer true of anything reachable: a
/// `grain: partition` model's `DeleteInsert` membership cell has no live
/// runtime dispatch of its own (`resolve_live_membership_recompute_cell`'s
/// own doc comment: left to the plain unconditional region `DELETE`+
/// `INSERT` batch loop), so a dimension mutation followed by a catch-up run
/// over the SAME window now recomputes the WHOLE touched region — rows, not
/// columns — honestly matching `incremental_models.md` §"The plan matrix"'s
/// rule that a membership-sensitive group "must be repaired by a technique
/// that can create and delete rows". This probe now asserts exactly that:
/// the cell is `DeleteInsert`, and the whole-region rewrite still reproduces
/// the full-refresh oracle for BOTH the mutated and unmutated dimension
/// keys, even though the unmutated key's row was rewritten too (its VALUES
/// are unchanged, unlike a truly column-scoped, row-untouched merge).
#[tokio::test]
async fn dimension_mutation_recomputes_the_whole_touched_region() {
    let recipe = MutableEnrichedRecipe::new();
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project = stage_mixed_recipe(&recipe, &tmp).expect("stage mixed recipe");

    // A probe that can't structurally apply is skipped explicitly, counted
    // — never silently vacuous (design §7: "Probes are per-case opt-in...
    // skipped explicitly").
    let plan = classify_mixed(&project, &recipe).expect("classify mixed recipe");
    let cell = match plan.cell_for(&Trigger::UpstreamMutation {
        source: recipe.dimension.name.clone(),
    }) {
        Some(cell) => cell,
        None => {
            eprintln!(
                "SKIP dimension_mutation_recomputes_the_whole_touched_region: no \
                 UpstreamMutation cell admitted for {:?} — probe structurally does not apply",
                recipe.model_name
            );
            return;
        }
    };
    assert_eq!(
        cell.technique,
        Technique::DeleteInsert,
        "a dimension read purely in the JOIN's ON predicate is membership-sensitive — the \
         admitted UpstreamMutation cell must be the recompute family, never ColumnScopedMerge"
    );

    let d: NaiveDate = NaiveDate::from_ymd_opt(2024, 1, 1).expect("valid date");
    insert_fact_row(&project, &recipe, &GenRow { d, id: 1, val: 11 }).expect("insert fact row 1");
    insert_fact_row(&project, &recipe, &GenRow { d, id: 2, val: 22 }).expect("insert fact row 2");

    let mut request = base_request("dev");
    request.start = Some("2024-01-01".to_string());
    request.end = Some("2024-01-02".to_string());
    project
        .run_quiet("probe-create", request.clone())
        .await
        .expect("creation run");

    let before = read_maintained_rows(&project, &recipe);

    // Mutate ONLY the dimension row id=1 references.
    {
        let conn = project.connect().expect("connect for mutation");
        conn.execute(
            &format!(
                "UPDATE main.sources_{} SET {} = 999 WHERE {} = 1",
                recipe.dimension.name, recipe.dimension.payload_column, recipe.dimension.key_column,
            ),
            [],
        )
        .expect("mutate dimension id=1");
    }

    // The catch-up run: same window, so the whole-region recompute resyncs
    // it (no column-scoped dispatch exists for this shape anymore).
    let outcome = project
        .run_quiet("probe-catchup", request)
        .await
        .expect("catch-up run");
    let record = outcome
        .models
        .get(&recipe.model_name)
        .expect("model ran on catch-up");
    assert_ne!(
        record.strategy, "column_scoped_merge",
        "no live dispatch exists for this cell's technique — the catch-up run must fall \
         through to the plain region-recompute batch loop"
    );

    let after = read_maintained_rows(&project, &recipe);

    let before_1 = before.iter().find(|r| r.1 == 1).expect("id=1 row before");
    let after_1 = after.iter().find(|r| r.1 == 1).expect("id=1 row after");
    let before_2 = before.iter().find(|r| r.1 == 2).expect("id=2 row before");
    let after_2 = after.iter().find(|r| r.1 == 2).expect("id=2 row after");

    assert_eq!(
        (&before_1.0, before_1.1, before_1.2),
        (&after_1.0, after_1.1, after_1.2),
        "the {{d, id, val}} values (folded from the append-only fact, never the dimension) \
         must be unchanged for the mutated row even under a whole-region rewrite"
    );
    assert_ne!(
        before_1.3, after_1.3,
        "the {{attr}} value (sourced from the dimension) must reflect the mutation"
    );
    assert_eq!(
        after_1.3, 999,
        "id=1's attr must pick up the mutated dimension value"
    );

    assert_eq!(
        (&before_2.0, before_2.1, before_2.2, before_2.3),
        (&after_2.0, after_2.1, after_2.2, after_2.3),
        "a row referencing an UNMUTATED dimension key must still reproduce byte-identical \
         values in every group — the honest claim after this rewrite is row-level correctness \
         under a whole-region recompute, not that the row was left physically untouched"
    );
}

/// `redelivered_window_refuses_for_additive_keyed` (plan Phase 5 TDD list):
/// re-running a folded window refuses (`KeyedReprocessedWindow`) before the
/// action re-runs (`incremental_shapes.md` §"Reprocessing"; `incremental_models.md`
/// §"The reconciliation ledger" — never-fold-twice).
#[tokio::test]
async fn redelivered_window_refuses_for_additive_keyed() {
    let recipe = KeyedRecipe::new_window_forward(KeyedCombiner::Additive);
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project = stage_keyed_recipe(&recipe, &tmp).expect("stage keyed recipe");

    let plan = classify_keyed(&project, &recipe).expect("classify additive keyed recipe");
    assert!(
        !plan.cells.is_empty(),
        "expected the additive keyed recipe to admit at least one cell: {plan:#?}"
    );

    let d: NaiveDate = NaiveDate::from_ymd_opt(2024, 1, 1).expect("valid date");
    insert_row_keyed(&project, &recipe, &GenRow { d, id: 1, val: 5 }).expect("insert row");

    let mut request = base_request("dev");
    request.start = Some("2024-01-01".to_string());
    request.end = Some("2024-01-02".to_string());
    project
        .run_quiet("keyed-redelivery-1", request.clone())
        .await
        .expect("first fold of the window must succeed");

    let maintained_before = {
        let backend = project.backend().await.expect("backend");
        snapshot_table_rows(backend.as_ref(), &recipe.model_name)
            .await
            .expect("snapshot before the refused redelivery")
    };

    // Re-deliver the SAME window: never-fold-twice must refuse BEFORE the
    // action re-runs, never silently double-count.
    let rerun = project.run_quiet("keyed-redelivery-2", request).await;
    let err = rerun.expect_err(
        "re-running an already-folded additive keyed window must be refused \
         (KeyedReprocessedWindow) — never-fold-twice",
    );
    let message = format!("{err:#}");
    assert!(
        message.contains("already reflected"),
        "refusal must name the never-fold-twice reason, got: {message}"
    );
    assert!(
        message.contains("KeyedReprocessedWindow"),
        "refusal must name the diagnostic code KeyedReprocessedWindow, got: {message}"
    );
    assert!(
        message.contains("2024-01-01"),
        "refusal must name the reprocessed window's bounds, got: {message}"
    );
    assert!(
        message.contains("--full-refresh"),
        "refusal must point at the --full-refresh remedy, got: {message}"
    );

    let maintained_after = {
        let backend = project.backend().await.expect("backend");
        snapshot_table_rows(backend.as_ref(), &recipe.model_name)
            .await
            .expect("snapshot after the refused redelivery")
    };
    assert_eq!(
        maintained_before, maintained_after,
        "the refused redelivery must leave the maintained table's contents byte-identical — \
         the refusal happens before any write"
    );
}

/// `persisted_reconciliation_store_reflects_recompute_reset` (plan Phase 5
/// TDD list): after two `execute_project` runs of a partition-grain recipe,
/// `.smelt/reconciliation.json` contains recompute-reset entries for
/// exactly the recomputed regions (closes design §2 gap 6 — zero
/// integration coverage of reconciliation-ledger persistence).
#[tokio::test]
async fn persisted_reconciliation_store_reflects_recompute_reset() {
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

    let d1: NaiveDate = NaiveDate::from_ymd_opt(2024, 1, 1).expect("valid date");
    let d2: NaiveDate = NaiveDate::from_ymd_opt(2024, 1, 2).expect("valid date");

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

    // Run 1: recompute region [2024-01-01, 2024-01-02).
    insert(d1, 1, 10);
    let mut r1 = base_request("dev");
    r1.start = Some("2024-01-01".to_string());
    r1.end = Some("2024-01-02".to_string());
    project.run_quiet("recon-run-1", r1).await.expect("run 1");

    // Run 2: recompute a DISJOINT region [2024-01-02, 2024-01-03).
    insert(d2, 2, 20);
    let mut r2 = base_request("dev");
    r2.start = Some("2024-01-02".to_string());
    r2.end = Some("2024-01-03".to_string());
    project.run_quiet("recon-run-2", r2).await.expect("run 2");

    let store = smelt_state::file_store::FileStore::new(
        &project.project_dir,
        "dev",
        smelt_core::config::StateMode::Environments,
    )
    .load_reconciliation_store()
    .expect("load persisted reconciliation store");
    let ledger = store.get(&recipe.model_name).unwrap_or_else(|| {
        panic!(
            "no reconciliation ledger persisted for model {:?}: store={store:?}",
            recipe.model_name
        )
    });

    let region1 = Region::new("2024-01-01", "2024-01-02");
    let region2 = Region::new("2024-01-02", "2024-01-03");
    assert!(
        ledger.get(&region1, "{*}").is_some(),
        "expected a recompute-reset entry for the first recomputed region {region1:?}, \
         got records={:#?}",
        ledger.records
    );
    assert!(
        ledger.get(&region2, "{*}").is_some(),
        "expected a recompute-reset entry for the second recomputed region {region2:?}, \
         got records={:#?}",
        ledger.records
    );
    assert_eq!(
        ledger.records.len(),
        2,
        "expected exactly the two recomputed regions' entries, got {:#?}",
        ledger.records
    );
}

/// Read every column of `main.<model_name>` back as text, one row per output
/// row, columns cast to `VARCHAR` and rows sorted by every column — a
/// construct-agnostic full-state snapshot for direct final-state comparison
/// (plan Phase 6 review checklist: "Permutation probe compares full final
/// states, not summaries"). Column names/order are read off
/// `information_schema.columns` since different `BodyConstruct`s project
/// different schemas.
fn read_full_output_as_text(
    project: &smelt_maintenance_testkit::link_c_harness::LinkCProject,
    model_name: &str,
) -> Vec<Vec<Option<String>>> {
    let conn = project.connect().expect("connect for full-state read-back");
    let columns: Vec<String> = {
        let mut stmt = conn
            .prepare(&format!(
                "SELECT column_name FROM information_schema.columns \
                 WHERE table_schema = 'main' AND table_name = '{model_name}' \
                 ORDER BY ordinal_position",
            ))
            .expect("prepare column listing");
        stmt.query_map([], |row| row.get::<_, String>(0))
            .expect("query column listing")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect column listing")
    };
    assert!(
        !columns.is_empty(),
        "model {model_name:?} reported zero columns via information_schema — staging bug"
    );

    let select_list = columns
        .iter()
        .map(|c| format!("CAST({c} AS VARCHAR)"))
        .collect::<Vec<_>>()
        .join(", ");
    let order_list = columns.join(", ");
    let sql = format!("SELECT {select_list} FROM main.{model_name} ORDER BY {order_list}");
    let ncols = columns.len();

    let mut stmt = conn.prepare(&sql).expect("prepare full-state read-back");
    stmt.query_map([], |row| {
        (0..ncols)
            .map(|i| row.get::<_, Option<String>>(i))
            .collect::<Result<Vec<_>, _>>()
    })
    .expect("query full-state rows")
    .collect::<Result<Vec<_>, _>>()
    .expect("collect full-state rows")
}

/// `window_order_permutations_converge` (plan Phase 6 TDD list): two valid
/// orderings of the same generated append-only schedule (same recipe, same
/// windows/rows, only the running ORDER differs) converge to identical final
/// maintained-table states — the order/set-determinacy corollary
/// (`incremental_models.md` §"The equivalence invariant": "the right-hand side
/// depends only on the SET S, never the order it was processed"). Restricted
/// to schedules with no `AppendLateRow` step
/// (`schedule_gen::is_permutable`) — a late row's catch-up rerun has a
/// genuine ordering dependency on its own prior insert.
#[test]
fn window_order_permutations_converge() {
    let mut runner = TestRunner::deterministic();
    let recipe_strat = arb_recipe(RecipePool::partition_append_only());
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");

    let mut checked = 0;
    for i in 0..10 {
        let recipe = recipe_strat.new_tree(&mut runner).unwrap().current();

        // Draw schedules until a permutable one turns up — a bounded
        // retry loop, never unbounded (design §7 "skipped explicitly, never
        // silently vacuous").
        let mut schedule = None;
        for _ in 0..20 {
            let candidate = arb_schedule_for(&recipe)
                .new_tree(&mut runner)
                .unwrap()
                .current();
            if is_permutable(&candidate) {
                schedule = Some(candidate);
                break;
            }
        }
        let Some(schedule) = schedule else {
            eprintln!(
                "SKIP case {i}: no permutable schedule drawn in 20 attempts for recipe \
                 {recipe:?} — probe structurally does not apply this case"
            );
            continue;
        };

        let tmp_a = tempfile::TempDir::new().expect("tempdir a");
        let project_a = stage_recipe(&recipe, &tmp_a)
            .unwrap_or_else(|e| panic!("case {i}: failed to stage project A: {e}"));
        let verdict = classify(&project_a, &recipe)
            .unwrap_or_else(|e| panic!("case {i}: classify failed: {e}"));
        if !matches!(verdict, Verdict::Admitted(_)) {
            continue;
        }

        // Reverse the window order — still a permutation of the same steps.
        let order: Vec<usize> = (0..schedule.0.len()).rev().collect();
        let permuted = reorder_windows(&schedule, &order);

        rt.block_on(drive_and_assert(&project_a, &recipe, &schedule))
            .unwrap_or_else(|e| panic!("case {i}: original-order schedule failed: {e}"));

        let tmp_b = tempfile::TempDir::new().expect("tempdir b");
        let project_b = stage_recipe(&recipe, &tmp_b)
            .unwrap_or_else(|e| panic!("case {i}: failed to stage project B: {e}"));
        rt.block_on(drive_and_assert(&project_b, &recipe, &permuted))
            .unwrap_or_else(|e| panic!("case {i}: reversed-order schedule failed: {e}"));

        let rows_a = read_full_output_as_text(&project_a, &recipe.model_name);
        let rows_b = read_full_output_as_text(&project_b, &recipe.model_name);
        assert_eq!(
            rows_a, rows_b,
            "case {i}: recipe {recipe:?} — original-order schedule {schedule:?} vs \
             reversed-order {permuted:?} diverged in final maintained state"
        );
        checked += 1;
    }

    assert!(
        checked > 0,
        "no permutable + admitted case reached across the deterministic sample — \
         generator/derivation regression"
    );
}

/// `compiled_sql_filter_matches_derived_clamp` (plan Phase 7 TDD list;
/// design §7 row 1): the filter in `SqlCapturingReporter`'s captured SQL
/// matches the admitted cell's derived `ScanClamp` — plan-vs-execution
/// consistency, checked at the SQL-text level (never re-derived; see
/// `smelt_maintenance_testkit::probes::compiled_sql_matches_derived_clamp`'s
/// doc comment).
#[tokio::test]
async fn compiled_sql_filter_matches_derived_clamp() {
    let mut runner = TestRunner::deterministic();
    let pool = RecipePool {
        constructs: vec![ConstructKind::AdditiveAgg],
    };
    let recipe = arb_recipe(pool).new_tree(&mut runner).unwrap().current();

    let ctx = CaseContext::stage_partition(recipe)
        .expect("stage + classify the additive-agg append-only recipe")
        .expect("expected the additive-agg append-only recipe to admit a NewData cell");

    compiled_sql_matches_derived_clamp(&ctx)
        .await
        .expect_checked_ok("compiled_sql_matches_derived_clamp");
}

/// `rows_outside_write_window_are_byte_unchanged` (plan Phase 7 TDD list;
/// design §7 row 2): output rows outside a run's write window are
/// byte-unchanged across that run — `incremental_models.md` §Constraints
/// "Write window = output window".
#[tokio::test]
async fn rows_outside_write_window_are_byte_unchanged() {
    let mut runner = TestRunner::deterministic();
    let pool = RecipePool {
        constructs: vec![ConstructKind::AdditiveAgg],
    };
    let recipe = arb_recipe(pool).new_tree(&mut runner).unwrap().current();

    let ctx = CaseContext::stage_partition(recipe)
        .expect("stage + classify the additive-agg append-only recipe")
        .expect("expected the additive-agg append-only recipe to admit a NewData cell");

    probe_write_window_containment(&ctx)
        .await
        .expect_checked_ok("rows_outside_write_window_are_byte_unchanged");
}

/// `technique_pins_agree_at_fixed_s` (plan Phase 7 TDD list; design §7
/// "Technique interchangeability"): for the `grain: key` additive-combiner
/// recipe, the fold family (windowed `KeyedFold` runs) and the recompute
/// family (a no-window full-table rebuild) reach identical final states over
/// the SAME seed data — `incremental_models.md` §"Per-cell admission"
/// "Interchangeability and choice". Also asserts a pin naming an unadmitted
/// technique for this cell refuses rather than silently resolving (review
/// checklist). See `smelt_maintenance_testkit::probes`'s module doc comment
/// for why this probe compares the two real execution paths rather than the
/// `maintenance.cells[].technique` frontmatter pin, which this phase's
/// implementation work confirmed is not wired into execution anywhere
/// today.
#[tokio::test]
async fn technique_pins_agree_at_fixed_s() {
    let recipe = KeyedRecipe::new_window_forward(KeyedCombiner::Additive);

    let ctx = CaseContext::stage_keyed(recipe)
        .expect("stage + classify the additive keyed recipe")
        .expect("expected the additive keyed recipe to admit a NewData KeyedFold cell");

    probe_technique_interchangeability(&ctx)
        .await
        .expect_checked_ok("technique_pins_agree_at_fixed_s");
}

/// `probe_skips_are_counted_never_silent` (plan Phase 7 TDD list; design §7
/// "Probes are per-case opt-in... skipped explicitly"/§8 "generator
/// health"): every probe that structurally can't apply to a case increments
/// a per-probe skip counter surfaced in the reachability report; a probe at
/// 100% skip across the sample fails the report. The sample deliberately
/// mixes partition-grain and `grain: key` cases so every probe (each scoped
/// to exactly one of those pools — see `smelt_maintenance_testkit::probes`)
/// fires at least once.
#[test]
fn probe_skips_are_counted_never_silent() {
    let mut runner = TestRunner::deterministic();
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let mut report = ReachabilityReport::default();

    let partition_pool = RecipePool {
        constructs: vec![
            ConstructKind::AdditiveAgg,
            ConstructKind::PassThrough,
            ConstructKind::Filter,
        ],
    };
    for _ in 0..3 {
        let recipe = arb_recipe(partition_pool.clone())
            .new_tree(&mut runner)
            .unwrap()
            .current();
        let Some(ctx) = CaseContext::stage_partition(recipe)
            .expect("stage + classify a partition-grain sample case")
        else {
            continue;
        };
        let clamp_outcome = rt.block_on(compiled_sql_matches_derived_clamp(&ctx));
        report.record("compiled_sql_matches_derived_clamp", &clamp_outcome);
        let window_outcome = rt.block_on(probe_write_window_containment(&ctx));
        report.record(
            "rows_outside_write_window_are_byte_unchanged",
            &window_outcome,
        );
        let technique_outcome = rt.block_on(probe_technique_interchangeability(&ctx));
        report.record("technique_pins_agree_at_fixed_s", &technique_outcome);
    }

    for combiner in [KeyedCombiner::Additive, KeyedCombiner::Idempotent] {
        let recipe = KeyedRecipe::new_window_forward(combiner);
        let Some(ctx) =
            CaseContext::stage_keyed(recipe).expect("stage + classify a keyed sample case")
        else {
            continue;
        };
        let clamp_outcome = rt.block_on(compiled_sql_matches_derived_clamp(&ctx));
        report.record("compiled_sql_matches_derived_clamp", &clamp_outcome);
        let window_outcome = rt.block_on(probe_write_window_containment(&ctx));
        report.record(
            "rows_outside_write_window_are_byte_unchanged",
            &window_outcome,
        );
        let technique_outcome = rt.block_on(probe_technique_interchangeability(&ctx));
        report.record("technique_pins_agree_at_fixed_s", &technique_outcome);
    }

    // Every probe recorded at least one Checked outcome (not just skips) —
    // the vacuity check itself.
    report.assert_no_probe_fully_skipped();

    // And, over this sample, every probe was actually exercised at least
    // once (belt-and-suspenders on the sample's own construction: if this
    // ever fails without `assert_no_probe_fully_skipped` also failing, the
    // report's own accounting has a gap).
    for probe in [
        "compiled_sql_matches_derived_clamp",
        "rows_outside_write_window_are_byte_unchanged",
        "technique_pins_agree_at_fixed_s",
    ] {
        assert!(
            report.checked(probe) > 0,
            "probe {probe:?} never recorded a Checked outcome across the sample: {report:#?}"
        );
    }
}
