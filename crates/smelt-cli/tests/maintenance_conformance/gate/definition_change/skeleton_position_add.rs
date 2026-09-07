use proptest::strategy::{Strategy, ValueTree};
use proptest::test_runner::TestRunner;

use super::super::partition_pool::stage_recipe;
use super::{derive_plan_with_real_deployed_schema, rt_insert_and_run};
use smelt_logical::maintenance::Trigger;
use smelt_maintenance_testkit::recipe::{arb_recipe, ConstructKind, ModelEdit, RecipePool};
use smelt_maintenance_testkit::render;
use smelt_maintenance_testkit::s_tracker::STracker;
use smelt_maintenance_testkit::schedule_gen::GenRow;

/// The skeleton-add direction of the same production derivation
/// (`docs/plans/20260809-sensitivity-precision.md` Phase 6): widening the
/// `GROUP BY` (`ModelEdit::AddGroupingColumn`) is a grain change, never a
/// column backfill (EX-39) — the plan derived with the REAL deployed-schema
/// snapshot must carry `Refusal::SkeletonChanged`, never a
/// `Trigger::ColumnAdded` cell.
#[test]
fn skeleton_position_add_derives_skeleton_column_added_refusal() {
    let mut runner = TestRunner::deterministic();
    let pool = RecipePool {
        constructs: vec![ConstructKind::AdditiveAgg],
    };
    let recipe = arb_recipe(pool).new_tree(&mut runner).unwrap().current();

    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project = stage_recipe(&recipe, &tmp).expect("stage recipe");

    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let mut tracker = STracker::new(&recipe.source);
    let w1_start = chrono::NaiveDate::from_ymd_opt(2024, 1, 1).expect("valid date");
    let w1_end = w1_start + chrono::Duration::days(1);

    rt.block_on(rt_insert_and_run(
        &project,
        &recipe,
        w1_start,
        w1_end,
        &[GenRow {
            d: w1_start,
            id: 1,
            val: Some(10),
        }],
        &mut tracker,
    ))
    .expect("creation run establishes the deployed-schema baseline");

    std::fs::write(
        project
            .project_dir
            .join(format!("models/{}.sql", recipe.model_name)),
        render::render_model_file_with_edit(&recipe, ModelEdit::AddGroupingColumn),
    )
    .expect("write rewritten model file");

    let plan = derive_plan_with_real_deployed_schema(&project, &recipe)
        .expect("derive plan with real deployed schema");
    assert!(
        plan.refusals.iter().any(|r| matches!(
            r,
            smelt_logical::maintenance::Refusal::SkeletonChanged { column }
                if column == &recipe.source.key_column
        )),
        "a GROUP BY widening add must refuse SkeletonChanged naming {:?}: {plan:#?}",
        recipe.source.key_column
    );
    assert!(
        !plan
            .cells
            .iter()
            .any(|c| matches!(&c.trigger, Trigger::ColumnAdded { .. })),
        "a skeleton-position add must admit no ColumnAdded cell at all: {plan:#?}"
    );
}
