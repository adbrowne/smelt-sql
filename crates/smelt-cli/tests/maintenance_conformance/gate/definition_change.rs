//! Definition-change steps (`ConformanceStep::RewriteModel`): column adds between runs, the pure-backfill in-place update, and the skeleton-position-add refusal.

use proptest::strategy::{Strategy, ValueTree};
use proptest::test_runner::TestRunner;

use super::partition_pool::{
    assert_equivalence, assert_equivalence_with_edit, drive_and_assert, insert_row, stage_recipe,
};
use smelt_logical::maintenance::{MutationProfile, SourceFacts, Technique, Trigger};
use smelt_maintenance_testkit::link_c_harness::{base_request, LinkCProject};
use smelt_maintenance_testkit::recipe::{
    arb_recipe, ConstructKind, ModelEdit, ModelRecipe, RecipePool,
};
use smelt_maintenance_testkit::render;
use smelt_maintenance_testkit::s_tracker::STracker;
use smelt_maintenance_testkit::schedule_gen::{
    read_source_snapshot, ConformanceSchedule, ConformanceStep, GenRow,
};
use smelt_maintenance_testkit::verdict::{classify, Verdict};

// ---------------------------------------------------------------------
// Phase 9: definition-change steps — `ConformanceStep::RewriteModel`
// (`docs/plans/20260712-generative-maintenance-conformance.md` Phase 9;
// `definition_deltas.md` §"The verdict per column group"). Asserts TODAY's
// contract only: whatever technique executes for a window always compiles
// and runs the model's CURRENT on-disk SQL (`link_c_harness::LinkCProject`'s
// per-run re-discovery), so a rewrite followed by a re-run of the affected
// window(s) recovers full equivalence against the REWRITTEN body's own
// oracle. This is deliberately NOT the spec's unbuilt `SkeletonAdd`/
// `PureBackfill`/`UpstreamRederive` definition-change classification — see
// `schedule_gen::ConformanceStep::RewriteModel`'s doc comment.
// ---------------------------------------------------------------------

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

/// Real deployed-schema column names for a staged recipe's model, read
/// straight from the on-disk `FileStore` `smelt-runtime`'s maintenance
/// driver itself reads (`crate::maintenance_driver::
/// resolve_live_in_place_update_cell`'s own doc comment) — never a
/// synthetic stand-in. `None` when no schema has been deployed yet (before
/// the model's first successful run).
pub(crate) fn deployed_column_names(project: &LinkCProject, table: &str) -> Vec<String> {
    let file_store = smelt_state::file_store::FileStore::new(&project.project_dir, "dev");
    file_store
        .load_schema(table)
        .ok()
        .flatten()
        .map(|s| s.columns.into_iter().map(|c| c.name).collect())
        .unwrap_or_default()
}

/// Derive the real production `MaintenancePlan` for `model_name` directly
/// (mirroring `crate::maintenance_driver::resolve_live_in_place_update_cell`'s
/// own input assembly — not a re-derivation of admission, just the same
/// input-gathering a `smelt-db` diagnostics call site cannot do because it
/// has no I/O access to `deployed_column_names`), threading the REAL
/// deployed-schema snapshot read via [`deployed_column_names`].
pub(crate) fn derive_plan_with_real_deployed_schema(
    project: &LinkCProject,
    recipe: &ModelRecipe,
) -> anyhow::Result<smelt_logical::maintenance::MaintenancePlan> {
    let config = smelt_core::config::Config::load(&project.project_dir)?;
    let discovery =
        smelt_core::ModelDiscovery::new(project.project_dir.clone(), config.paths.clone());
    let sql_models = discovery.discover_models()?;
    let model = sql_models
        .iter()
        .find(|m| m.name == recipe.model_name)
        .ok_or_else(|| anyhow::anyhow!("model {:?} not discovered", recipe.model_name))?;
    let metadata = model
        .metadata
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("staged recipe model must declare frontmatter"))?;
    let stripped_sql = smelt_parser::strip_frontmatter(&model.content).to_string();
    let sources = vec![SourceFacts {
        name: recipe.source.name.clone(),
        mutation: MutationProfile::AppendOnly,
        partition_col: Some(recipe.source.clock_column.clone()),
        unique_key: vec![],
        allow_full_scan: false,
    }];
    let table = model.db_name_owned();
    let deployed = deployed_column_names(project, &table);
    let result = smelt_db::queries::maintenance::derive_model_maintenance_plan(
        &stripped_sql,
        &table,
        metadata,
        &sources,
        &std::collections::HashSet::new(),
        None,
        &[],
        &deployed,
        &std::collections::BTreeMap::new(),
        None,
        None,
    )
    .ok_or_else(|| anyhow::anyhow!("model {:?} carries no maintenance plan", recipe.model_name))?;
    Ok(result.plan)
}

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
/// `column_add_between_runs_recovers_equivalence`, which predates this
/// phase's production `ColumnAdded` trigger) must dispatch
/// `Technique::InPlaceUpdate` (`RunOutcome.models[..].strategy ==
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

/// Shared `RunWindow`-step logic (insert rows, snapshot, run, record,
/// assert equivalence) — factored out of
/// [`pure_backfill_column_add_executes_in_place_update`]'s creation-run leg
/// so that leg reads identically to `drive_and_assert`'s own `RunWindow`
/// arm.
pub(crate) async fn rt_insert_and_run(
    project: &LinkCProject,
    recipe: &ModelRecipe,
    start: chrono::NaiveDate,
    end: chrono::NaiveDate,
    rows: &[GenRow],
    tracker: &mut STracker,
) -> anyhow::Result<()> {
    for row in rows {
        insert_row(project, recipe, row).await?;
    }
    let snapshot = {
        let conn = project.connect()?;
        read_source_snapshot(&conn, &recipe.source)
    };
    let mut request = base_request("dev");
    request.start = Some(start.format("%Y-%m-%d").to_string());
    request.end = Some(end.format("%Y-%m-%d").to_string());
    project.run_quiet("creation-run", request).await?;
    let k = tracker.record_run(start, end, snapshot);
    assert_equivalence(project, recipe, tracker, k).await
}

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
