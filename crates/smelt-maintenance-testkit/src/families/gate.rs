//! The append-only partition-grain `ModelRecipe` pool family — the
//! target-parametrized twin of `maintenance_conformance/gate.rs`'s
//! append-only leg. Staging, row insertion, snapshot reads,
//! S-materialization, and the multiset comparison all route through
//! [`smelt_backend::Backend`] (never a raw host connection or a
//! host-filesystem load path).

use anyhow::Result;
use proptest::strategy::{Strategy, ValueTree};
use proptest::test_runner::TestRunner;

use smelt_backend::Backend;

use crate::families::ConformanceBackend;
use crate::link_c_harness::{base_request, LinkCProject};
use crate::oracle::multiset_equal_via_backend;
use crate::recipe::{arb_recipe, ConstructKind, ModelEdit, ModelRecipe, RecipePool};
use crate::render;
use crate::s_tracker::STracker;
use crate::schedule_gen::{
    boundary_rows_for, read_source_snapshot_via_backend, scan_clamp_for, ConformanceSchedule,
    ConformanceStep, GenRow,
};
use crate::verdict::{classify, Verdict};

/// Stage `recipe` into a fresh temp project dir targeting `schema`/`target`
/// — mirrors `render::stage_for_target`'s drop-before-seed idempotency.
pub fn stage_recipe_for(
    recipe: &ModelRecipe,
    tmp: &tempfile::TempDir,
    target: crate::recipe::ConformanceTarget,
) -> Result<LinkCProject> {
    let project_dir = tmp.path().join("project");
    // Never opened as a file on a non-DuckDB arm — kept only for
    // `LinkCProject`'s bookkeeping and the DuckDB-arm fallback path.
    let db_path = tmp.path().join("unused.duckdb");
    std::fs::create_dir_all(&project_dir)?;
    render::stage_for_target(recipe, &project_dir, &db_path, target)
}

/// Insert one row into `recipe`'s staged source table via `Backend::execute_sql`
/// — never a host-path read.
pub async fn insert_row_for(
    backend: &dyn Backend,
    schema: &str,
    recipe: &ModelRecipe,
    row: &GenRow,
) -> Result<()> {
    backend
        .execute_sql(&format!(
            "INSERT INTO {schema}.sources_{name} VALUES (DATE '{d}', {id}, {val})",
            name = recipe.source.name,
            d = row.d.format("%Y-%m-%d"),
            id = row.id,
            val = row.val,
        ))
        .await
        .map_err(|e| anyhow::anyhow!("insert row: {e}"))?;
    Ok(())
}

/// The S-restricted oracle assertion (mirrors
/// `maintenance_conformance/gate.rs::assert_equivalence`): materialize `S_k`
/// as a temp view ([`STracker::materialize_s_as_view`]), then `EXCEPT
/// ALL`-compare it against the maintained table `<schema>.<model_name>`.
pub async fn assert_equivalence_for(
    schema: &str,
    recipe: &ModelRecipe,
    backend: &dyn Backend,
    tracker: &STracker,
    k: usize,
) -> Result<()> {
    assert_equivalence_for_with_edit(schema, recipe, backend, tracker, k, None).await
}

/// [`assert_equivalence_for`] generalised over an optional post-`RewriteModel`
/// [`ModelEdit`]: when `edit` is `Some`, the oracle re-renders against the
/// REWRITTEN body (`STracker::s_restricted_oracle_sql_with_edit`) rather than
/// the original recipe's own body.
pub async fn assert_equivalence_for_with_edit(
    schema: &str,
    recipe: &ModelRecipe,
    backend: &dyn Backend,
    tracker: &STracker,
    k: usize,
    edit: Option<ModelEdit>,
) -> Result<()> {
    tracker.materialize_s_as_view(backend, k).await?;
    let maintained_sql = format!("SELECT * FROM {schema}.{}", recipe.model_name);
    let oracle_sql = match edit {
        Some(edit) => tracker.s_restricted_oracle_sql_with_edit(recipe, edit),
        None => tracker.s_restricted_oracle_sql(recipe),
    };
    let equal = multiset_equal_via_backend(backend, &maintained_sql, &oracle_sql).await?;
    if !equal {
        anyhow::bail!(
            "S-restricted equivalence violated for model {:?} at run {k} (edit {edit:?}): \
             maintained ({maintained_sql:?}) != oracle ({oracle_sql:?})",
            recipe.model_name
        );
    }
    Ok(())
}

/// Drive `schedule` against `project`/`recipe` through the real
/// `execute_project` pipeline (`LinkCProject::run_with_target`), asserting
/// S-restricted multiset equivalence after every `RunWindow` step. One
/// backend is opened up front and reused for every insert/snapshot/
/// materialize/compare call in this drive. Returns the populated `STracker`
/// plus the last recorded run index, so the harness self-check can reuse the
/// green end-state without re-driving the whole schedule.
pub async fn drive_and_assert_for(
    b: &dyn ConformanceBackend,
    case: usize,
    project: &LinkCProject,
    recipe: &ModelRecipe,
    schedule: &ConformanceSchedule,
) -> Result<(STracker, usize)> {
    let target = b.target(case);
    let schema = b.schema(case);
    let backend = project.backend_for_target(target.clone()).await?;
    let mut tracker = STracker::new(&recipe.source);
    let mut last_k: Option<usize> = None;
    // The most recent `RewriteModel` step's edit, or `None` before any
    // rewrite — threads into every subsequent assertion so the oracle
    // re-renders against the CURRENT on-disk body.
    let mut current_edit: Option<ModelEdit> = None;

    for (i, step) in schedule.0.iter().enumerate() {
        match step {
            ConformanceStep::RunWindow { start, end, rows } => {
                for row in rows {
                    b.before_step().await;
                    insert_row_for(backend.as_ref(), &schema, recipe, row).await?;
                }

                let snapshot =
                    read_source_snapshot_via_backend(backend.as_ref(), &schema, &recipe.source)
                        .await?;

                let mut request = base_request(b.engine_name());
                request.start = Some(start.format("%Y-%m-%d").to_string());
                request.end = Some(end.format("%Y-%m-%d").to_string());
                b.before_step().await;
                project
                    .run_with_target(
                        target.clone(),
                        &format!("{}-run-{i}", b.engine_name()),
                        request,
                        &smelt_runtime::NoOpReporter,
                    )
                    .await?;

                let k = tracker.record_run(*start, *end, snapshot);
                last_k = Some(k);
                assert_equivalence_for_with_edit(
                    &schema,
                    recipe,
                    backend.as_ref(),
                    &tracker,
                    k,
                    current_edit,
                )
                .await?;
            }
            ConformanceStep::AppendLateRow(row) => {
                b.before_step().await;
                insert_row_for(backend.as_ref(), &schema, recipe, row).await?;
            }
            ConformanceStep::RerunWindow { start, end } => {
                // Redelivery: same window as an earlier `RunWindow`, no new
                // rows. Never-fold-twice under the partition-grain
                // DELETE+INSERT full-replace technique must hold.
                let snapshot =
                    read_source_snapshot_via_backend(backend.as_ref(), &schema, &recipe.source)
                        .await?;

                let mut request = base_request(b.engine_name());
                request.start = Some(start.format("%Y-%m-%d").to_string());
                request.end = Some(end.format("%Y-%m-%d").to_string());
                b.before_step().await;
                project
                    .run_with_target(
                        target.clone(),
                        &format!("{}-run-{i}", b.engine_name()),
                        request,
                        &smelt_runtime::NoOpReporter,
                    )
                    .await?;

                let k = tracker.record_run(*start, *end, snapshot);
                last_k = Some(k);
                assert_equivalence_for_with_edit(
                    &schema,
                    recipe,
                    backend.as_ref(),
                    &tracker,
                    k,
                    current_edit,
                )
                .await?;
            }
            ConformanceStep::FullRefreshRun => {
                // Unwindowed run: `execute_project` takes the full-refresh
                // arm (drop + rebuild from the CURRENT full source
                // contents) whenever no `start`/`end` is supplied.
                let snapshot =
                    read_source_snapshot_via_backend(backend.as_ref(), &schema, &recipe.source)
                        .await?;

                let mut request = base_request(b.engine_name());
                request.full_refresh = true;
                request.start = None;
                request.end = None;
                b.before_step().await;
                project
                    .run_with_target(
                        target.clone(),
                        &format!("{}-run-{i}", b.engine_name()),
                        request,
                        &smelt_runtime::NoOpReporter,
                    )
                    .await?;

                let k = tracker.record_full_refresh(snapshot);
                last_k = Some(k);
                assert_equivalence_for_with_edit(
                    &schema,
                    recipe,
                    backend.as_ref(),
                    &tracker,
                    k,
                    current_edit,
                )
                .await?;
            }
            ConformanceStep::BackfillRegion { start, end } => {
                // An explicit backfill: same execution shape as `RunWindow`
                // with no accompanying insert.
                let snapshot =
                    read_source_snapshot_via_backend(backend.as_ref(), &schema, &recipe.source)
                        .await?;

                let mut request = base_request(b.engine_name());
                request.start = Some(start.format("%Y-%m-%d").to_string());
                request.end = Some(end.format("%Y-%m-%d").to_string());
                b.before_step().await;
                project
                    .run_with_target(
                        target.clone(),
                        &format!("{}-run-{i}", b.engine_name()),
                        request,
                        &smelt_runtime::NoOpReporter,
                    )
                    .await?;

                let k = tracker.record_run(*start, *end, snapshot);
                last_k = Some(k);
                assert_equivalence_for_with_edit(
                    &schema,
                    recipe,
                    backend.as_ref(),
                    &tracker,
                    k,
                    current_edit,
                )
                .await?;
            }
            ConformanceStep::RewriteModel { edit } => {
                // Definition change: rewrite the model file on disk with
                // `edit` applied. No run happens here — the next *Window
                // step re-discovers the model from disk and compiles/
                // executes whatever SQL is now on disk.
                let model_path = project
                    .project_dir
                    .join(format!("models/{}.sql", recipe.model_name));
                std::fs::write(
                    &model_path,
                    render::render_model_file_with_edit(recipe, *edit),
                )?;
                current_edit = Some(*edit);
            }
        }
    }

    let last_k =
        last_k.ok_or_else(|| anyhow::anyhow!("schedule {schedule:?} had no RunWindow step"))?;
    Ok((tracker, last_k))
}

/// Count rows returned by a `SELECT COUNT(*) ...` query via the Backend
/// trait — routed through `Backend::execute_sql` rather than a raw host
/// connection.
pub async fn count_via_backend(backend: &dyn Backend, sql: &str) -> Result<i64> {
    let batches = backend
        .execute_sql(sql)
        .await
        .map_err(|e| anyhow::anyhow!("count query {sql:?}: {e}"))?;
    let mut total = 0i64;
    for batch in &batches {
        let col = batch
            .column(0)
            .as_any()
            .downcast_ref::<arrow::array::Int64Array>()
            .ok_or_else(|| anyhow::anyhow!("count query's count(*) column was not Int64"))?;
        for i in 0..batch.num_rows() {
            total += col.value(i);
        }
    }
    Ok(total)
}

/// The append-only pool's own equivalence sweep — the twin of
/// `maintenance_conformance::gate::append_only_partition_pool_upholds_equivalence`.
pub async fn run_append_only_partition_pool(b: &dyn ConformanceBackend, n: usize) -> Result<()> {
    let mut runner = TestRunner::deterministic();
    let recipe_strat = arb_recipe(RecipePool::partition_append_only());
    let mut admitted_cases = 0;

    for i in 0..n {
        let recipe = recipe_strat.new_tree(&mut runner).unwrap().current();
        let schedule = crate::schedule_gen::arb_schedule_for(&recipe)
            .new_tree(&mut runner)
            .unwrap()
            .current();

        let tmp = tempfile::TempDir::new().expect("tempdir");
        let project = stage_recipe_for(&recipe, &tmp, b.target(i))
            .unwrap_or_else(|e| panic!("case {i}: recipe {recipe:?} failed to stage: {e}"));

        let verdict = classify(&project, &recipe)
            .unwrap_or_else(|e| panic!("case {i}: recipe {recipe:?} classify failed: {e}"));

        match verdict {
            Verdict::Refused(_) => continue,
            Verdict::Admitted(_) => {
                admitted_cases += 1;
                drive_and_assert_for(b, i, &project, &recipe, &schedule)
                    .await
                    .unwrap_or_else(|e| {
                        panic!(
                            "case {i}: recipe {recipe:?} schedule {schedule:?} equivalence \
                             check failed: {e}"
                        )
                    });
            }
        }
    }

    anyhow::ensure!(
        admitted_cases > 0,
        "N={n} deterministic sample admitted zero cases — generator/derivation regression"
    );
    Ok(())
}

/// Generator-health admission-rate sweep — the twin of
/// `maintenance_conformance::gate::admission_rate_stays_above_floor`.
pub async fn run_admission_rate_stays_above_floor(
    b: &dyn ConformanceBackend,
    n: usize,
) -> Result<()> {
    let mut runner = TestRunner::deterministic();
    let recipe_strat = arb_recipe(RecipePool::partition_append_only());

    let mut admitted = 0;
    for i in 0..n {
        let recipe = recipe_strat.new_tree(&mut runner).unwrap().current();
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let project = stage_recipe_for(&recipe, &tmp, b.target(i))
            .unwrap_or_else(|e| panic!("case {i}: recipe {recipe:?} failed to stage: {e}"));
        let verdict = classify(&project, &recipe)
            .unwrap_or_else(|e| panic!("case {i}: recipe {recipe:?} classify failed: {e}"));
        if matches!(verdict, Verdict::Admitted(_)) {
            admitted += 1;
        }
    }

    let rate = admitted as f64 / n as f64;
    anyhow::ensure!(
        rate >= 0.40,
        "admission rate {rate:.2} over N={n} fell below the 40% generator-health floor \
         ({admitted}/{n} admitted)"
    );
    Ok(())
}

/// Redelivery idempotency — the twin of
/// `maintenance_conformance::gate::redelivery_of_processed_window_is_idempotent`.
pub async fn run_redelivery_of_processed_window_is_idempotent(
    b: &dyn ConformanceBackend,
) -> Result<()> {
    let mut runner = TestRunner::deterministic();
    let pool = RecipePool {
        constructs: vec![ConstructKind::AdditiveAgg],
    };
    let recipe = arb_recipe(pool).new_tree(&mut runner).unwrap().current();

    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project = stage_recipe_for(&recipe, &tmp, b.target(0)).expect("stage additive-agg recipe");
    let verdict = classify(&project, &recipe).expect("classify");
    anyhow::ensure!(
        matches!(verdict, Verdict::Admitted(_)),
        "expected additive-agg append-only recipe to admit: {verdict:?}"
    );

    let d = chrono::NaiveDate::from_ymd_opt(2024, 1, 1).expect("valid date");
    let window_end = d + chrono::Duration::days(1);

    let schedule = ConformanceSchedule(vec![
        ConformanceStep::RunWindow {
            start: d,
            end: window_end,
            rows: vec![GenRow { d, id: 1, val: 10 }, GenRow { d, id: 2, val: 20 }],
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

    drive_and_assert_for(b, 0, &project, &recipe, &schedule)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "redelivering an already-processed window must stay idempotent — equivalence \
                 must hold after every redelivery, never double-counted: {e}"
            )
        })?;
    Ok(())
}

/// Full-refresh interleave — the twin of
/// `maintenance_conformance::gate::full_refresh_interleave_resets_state_correctly`.
pub async fn run_full_refresh_interleave_resets_state_correctly(
    b: &dyn ConformanceBackend,
) -> Result<()> {
    let mut runner = TestRunner::deterministic();
    let pool = RecipePool {
        constructs: vec![ConstructKind::AdditiveAgg],
    };
    let recipe = arb_recipe(pool).new_tree(&mut runner).unwrap().current();

    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project = stage_recipe_for(&recipe, &tmp, b.target(0)).expect("stage additive-agg recipe");
    let verdict = classify(&project, &recipe).expect("classify");
    anyhow::ensure!(
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
                val: 10,
            }],
        },
        ConformanceStep::FullRefreshRun,
        ConformanceStep::RunWindow {
            start: d2,
            end: d2 + chrono::Duration::days(1),
            rows: vec![GenRow {
                d: d2,
                id: 2,
                val: 20,
            }],
        },
        ConformanceStep::RunWindow {
            start: d3,
            end: d3 + chrono::Duration::days(1),
            rows: vec![GenRow {
                d: d3,
                id: 3,
                val: 30,
            }],
        },
    ]);

    drive_and_assert_for(b, 0, &project, &recipe, &schedule)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "a mid-schedule full_refresh interleave must reset coverage cleanly — \
                 equivalence must hold both immediately after the refresh and through every \
                 subsequent windowed run: {e}"
            )
        })?;
    Ok(())
}

/// Boundary-row reach — the twin of
/// `maintenance_conformance::gate::boundary_rows_within_reach_are_reflected`.
pub async fn run_boundary_rows_within_reach_are_reflected(
    b: &dyn ConformanceBackend,
) -> Result<()> {
    let mut runner = TestRunner::deterministic();
    let pool = RecipePool {
        constructs: vec![ConstructKind::AdditiveAgg],
    };
    let recipe = arb_recipe(pool).new_tree(&mut runner).unwrap().current();

    let tmp = tempfile::TempDir::new().expect("tempdir");
    let target = b.target(0);
    let schema = b.schema(0);
    let project = stage_recipe_for(&recipe, &tmp, target.clone()).expect("stage recipe");

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

    let backend = project.backend_for_target(target.clone()).await?;

    insert_row_for(backend.as_ref(), &schema, &recipe, &boundary.just_inside)
        .await
        .expect("insert just-inside row");
    insert_row_for(backend.as_ref(), &schema, &recipe, &boundary.just_outside)
        .await
        .expect("insert just-outside row");

    let mut request = base_request(b.engine_name());
    request.start = Some(window.0.format("%Y-%m-%d").to_string());
    request.end = Some(window.1.format("%Y-%m-%d").to_string());
    project
        .run_with_target(
            target.clone(),
            &format!("{}-boundary-run", b.engine_name()),
            request,
            &smelt_runtime::NoOpReporter,
        )
        .await
        .expect("triggering run over the boundary window");

    let count = count_via_backend(
        backend.as_ref(),
        &format!(
            "SELECT COUNT(*) FROM {schema}.{} WHERE {} = DATE '{}'",
            recipe.model_name,
            recipe.source.clock_column,
            boundary.just_inside.d.format("%Y-%m-%d"),
        ),
    )
    .await
    .expect("count rows for the boundary just-inside day");
    anyhow::ensure!(
        count > 0,
        "a just-inside-reach row (day {}) must appear in the maintained output after the \
         triggering run over {window:?} — an under-derived clamp would drop it",
        boundary.just_inside.d,
    );

    let outside_count = count_via_backend(
        backend.as_ref(),
        &format!(
            "SELECT COUNT(*) FROM {schema}.{} WHERE {} = DATE '{}'",
            recipe.model_name,
            recipe.source.clock_column,
            boundary.just_outside.d.format("%Y-%m-%d"),
        ),
    )
    .await
    .expect("count rows for the boundary just-outside day");
    anyhow::ensure!(
        outside_count == 0,
        "a just-outside-reach row (day {}) must NOT appear in the maintained output after the \
         triggering run over {window:?} — an over-derived (or off-by-one) scan predicate would \
         read it even though it lies outside the derived reach",
        boundary.just_outside.d,
    );
    Ok(())
}

/// Column-add recovery — the twin of
/// `maintenance_conformance::gate::column_add_between_runs_recovers_equivalence`.
pub async fn run_column_add_between_runs_recovers_equivalence(
    b: &dyn ConformanceBackend,
) -> Result<()> {
    let mut runner = TestRunner::deterministic();
    let pool = RecipePool {
        constructs: vec![ConstructKind::AdditiveAgg],
    };
    let recipe = arb_recipe(pool).new_tree(&mut runner).unwrap().current();

    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project = stage_recipe_for(&recipe, &tmp, b.target(0)).expect("stage recipe");
    let verdict = classify(&project, &recipe).expect("classify");
    anyhow::ensure!(
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
                val: 10,
            }],
        },
        ConformanceStep::RunWindow {
            start: w2_start,
            end: w2_end,
            rows: vec![GenRow {
                d: w2_start,
                id: 2,
                val: 20,
            }],
        },
        // Definition change: add a derived payload column, same skeleton.
        ConformanceStep::RewriteModel {
            edit: ModelEdit::AddPayloadColumn,
        },
        // Catch-up: a full-refresh run recomputes the WHOLE table under the
        // rewritten body over the current full source contents.
        ConformanceStep::FullRefreshRun,
        // A subsequent ordinary windowed run must keep working post-recovery.
        ConformanceStep::RunWindow {
            start: w3_start,
            end: w3_end,
            rows: vec![GenRow {
                d: w3_start,
                id: 3,
                val: 30,
            }],
        },
    ]);

    drive_and_assert_for(b, 0, &project, &recipe, &schedule)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "a column-add RewriteModel followed by a full-refresh catch-up must recover \
                 full equivalence against the rewritten body's own oracle, and stay recovered \
                 through a subsequent windowed run: {e}"
            )
        })?;
    Ok(())
}
