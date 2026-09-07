use proptest::strategy::{Strategy, ValueTree};
use proptest::test_runner::TestRunner;

use super::super::partition_pool::{assert_equivalence_with_edit, insert_row, stage_recipe};
use super::{derive_plan_with_real_deployed_schema, rt_insert_and_run};
use smelt_logical::maintenance::{Technique, Trigger};
use smelt_maintenance_testkit::link_c_harness::base_request;
use smelt_maintenance_testkit::recipe::{arb_recipe, ConstructKind, ModelEdit, RecipePool};
use smelt_maintenance_testkit::render;
use smelt_maintenance_testkit::s_tracker::STracker;
use smelt_maintenance_testkit::schedule_gen::{read_source_snapshot, GenRow};
use smelt_maintenance_testkit::verdict::{classify, Verdict};

/// `pure_backfill_column_add_executes_in_place_update`
/// (`docs/plans/20260809-sensitivity-precision.md` Phase 6): a `PassThrough`
/// recipe's `AddPayloadColumn` edit (`val * 2 AS val_doubled`, `render.rs`)
/// reads only the already-stored `val` column — no upstream re-read — so
/// [`smelt_logical::analysis::definition_change::classify_definition_change`]
/// must classify it `PureBackfill` and the derived plan must admit
/// `Technique::InPlaceUpdate` for the `Trigger::ColumnAdded` cell, once the
/// runtime's real deployed-schema snapshot is threaded in (never `&[]`, the
/// fail-closed default `smelt-db`'s own I/O-blind diagnostic path uses).
///
/// Drives the real `execute_project` pipeline across the rewrite: a plain
/// windowed re-run (never `FullRefreshRun`, unlike the pre-existing
/// `column_add_recovery::column_add_between_runs_recovers_equivalence`,
/// which predates this phase's production `ColumnAdded` trigger) must
/// dispatch `Technique::InPlaceUpdate` (`RunOutcome.models[..].strategy ==
/// "in_place_update"`, the same observable
/// `value_enriched_recipe_executes_column_scoped_merge` uses for
/// `column_scoped_merge`) and land an end state equal to the rewritten
/// body's own oracle.
#[test]
fn pure_backfill_column_add_executes_in_place_update() {
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

    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let mut tracker = STracker::new(&recipe.source);

    rt.block_on(async {
        // Creation run under the ORIGINAL body — establishes a deployed
        // schema baseline (no `val_doubled` yet) and a green equivalence
        // starting point.
        rt_insert_and_run(&project, &recipe, w1_start, w1_end, &[GenRow { d: w1_start, id: 1, val: Some(10) }], &mut tracker)
            .await
            .expect("creation run");

        // Definition change: add `val_doubled AS val * 2` — a pure function
        // of the already-stored `val` column.
        std::fs::write(
            project
                .project_dir
                .join(format!("models/{}.sql", recipe.model_name)),
            render::render_model_file_with_edit(&recipe, ModelEdit::AddPayloadColumn),
        )
        .expect("write rewritten model file");

        // (a) Plan-shape assertion, with the REAL deployed-schema snapshot
        // threaded in — the production shape, never `&[]`.
        let plan = derive_plan_with_real_deployed_schema(&project, &recipe)
            .expect("derive plan with real deployed schema");
        let column_added_cell = plan
            .cells
            .iter()
            .find(|c| {
                matches!(&c.trigger, Trigger::ColumnAdded { columns } if columns == &vec!["val_doubled".to_string()])
            })
            .unwrap_or_else(|| {
                panic!("no ColumnAdded([\"val_doubled\"]) cell in derived plan: {plan:#?}")
            });
        assert_eq!(
            column_added_cell.technique,
            Technique::InPlaceUpdate,
            "a pure function of already-stored columns must admit InPlaceUpdate: {plan:#?}"
        );

        // (b) A plain windowed re-run (next window, new row) must dispatch
        // InPlaceUpdate — never a raw column-count crash, never a silent
        // recompute fallback.
        insert_row(&project, &recipe, &GenRow { d: w2_start, id: 2, val: Some(20) })
            .await
            .expect("insert row 2");
        let snapshot = {
            let conn = project.connect().expect("connect");
            read_source_snapshot(&conn, &recipe.source)
        };
        let mut request = base_request("dev");
        request.start = Some(w2_start.format("%Y-%m-%d").to_string());
        request.end = Some(w2_end.format("%Y-%m-%d").to_string());
        let outcome = project
            .run_quiet("post-rewrite-window", request)
            .await
            .expect("post-rewrite windowed run");
        let record = outcome
            .models
            .get(&recipe.model_name)
            .expect("model ran post-rewrite");
        assert_eq!(
            record.strategy, "in_place_update",
            "a PureBackfill column-add must dispatch Technique::InPlaceUpdate on a plain \
             windowed re-run: {record:?}"
        );

        let k = tracker.record_run(w2_start, w2_end, snapshot);

        // (c) End-state equivalence against the rewritten body's own oracle.
        assert_equivalence_with_edit(&project, &recipe, &tracker, k, Some(ModelEdit::AddPayloadColumn))
            .await
            .expect(
                "post-rewrite end state must equal the rewritten body's own oracle over full S",
            );
    });
}
