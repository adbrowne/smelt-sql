//! Spark twin of `maintenance_conformance/gate.rs`'s append-only partition
//! pool (`docs/plans/20260720-prod-w9-spark-conformance-twin.md` Phase 3):
//! the same recipe pool, schedule driver, and S-restricted multiset oracle,
//! driven against a live Spark Connect/Delta backend through
//! `LinkCProject::run_with_target(ConformanceTarget::SparkDelta, ...)`
//! instead of DuckDB. Staging, row insertion, snapshot reads, S-materialization,
//! and the multiset comparison all route through `smelt_backend::Backend`
//! (never a raw host connection or a host-filesystem load path).

use proptest::strategy::{Strategy, ValueTree};
use proptest::test_runner::TestRunner;

use smelt_backend::Backend;
use smelt_maintenance_testkit::link_c_harness::{base_request, LinkCProject};
use smelt_maintenance_testkit::oracle::multiset_equal_via_backend;
use smelt_maintenance_testkit::recipe::{
    arb_recipe, ConformanceTarget, ConstructKind, ModelEdit, ModelRecipe, RecipePool,
    SPARK_CONFORMANCE_SCHEMA,
};
use smelt_maintenance_testkit::render;
use smelt_maintenance_testkit::s_tracker::STracker;
use smelt_maintenance_testkit::schedule_gen::{
    arb_schedule_for, boundary_rows_for, read_source_snapshot_via_backend, scan_clamp_for,
    ConformanceSchedule, ConformanceStep, GenRow,
};
use smelt_maintenance_testkit::verdict::{classify, Verdict};

/// Default deterministic case count for
/// `append_only_partition_pool_upholds_equivalence_on_spark` —
/// `SMELT_CONFORMANCE_SPARK_CASES` env override (plan Phase 3 TDD list).
/// Smaller than the DuckDB leg's default (12): each case round-trips over a
/// real Spark Connect server rather than an in-process DuckDB file.
const DEFAULT_CASES: usize = 4;

pub fn case_count() -> usize {
    std::env::var("SMELT_CONFORMANCE_SPARK_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_CASES)
}

/// `SPARK_CONNECT_URL` from the environment — mirrors
/// `crates/smelt-cli/tests/common/mod.rs::spark_connect_url`'s convention,
/// kept local (rather than pulling in that whole shared module) since this
/// test target is a standalone binary, like `maintenance_conformance` is.
pub fn spark_connect_url() -> Option<String> {
    std::env::var("SPARK_CONNECT_URL").ok()
}

/// Stage `recipe` targeting Spark/Delta into a fresh temp project dir,
/// dropping any stale model/source table from a prior run first
/// (`render::stage_for_target`'s Spark arm) — the Delta warehouse persists
/// across test invocations, unlike DuckDB's fresh-temp-file-per-case default.
pub fn stage_recipe_spark(
    recipe: &ModelRecipe,
    tmp: &tempfile::TempDir,
) -> anyhow::Result<LinkCProject> {
    let project_dir = tmp.path().join("project");
    // Never opened as a file on the Spark arm — kept only for `LinkCProject`'s
    // bookkeeping and the (unused-here) DuckDB-arm fallback paths it shares.
    let db_path = tmp.path().join("unused.duckdb");
    std::fs::create_dir_all(&project_dir)?;
    render::stage_for_target(
        recipe,
        &project_dir,
        &db_path,
        ConformanceTarget::SparkDelta,
    )
}

/// Insert one row into `recipe`'s staged Spark/Delta source table via the
/// Backend trait's `execute_sql` (Delta `INSERT INTO`) — never a host-path
/// read (`multi_backend.md`'s backend-client-API requirement).
async fn insert_row(
    backend: &dyn Backend,
    recipe: &ModelRecipe,
    row: &GenRow,
) -> anyhow::Result<()> {
    backend
        .execute_sql(&format!(
            "INSERT INTO {schema}.sources_{name} VALUES (DATE '{d}', {id}, {val})",
            schema = SPARK_CONFORMANCE_SCHEMA,
            name = recipe.source.name,
            d = row.d.format("%Y-%m-%d"),
            id = row.id,
            val = row.val,
        ))
        .await
        .map_err(|e| anyhow::anyhow!("insert row on Spark: {e}"))?;
    Ok(())
}

/// The S-restricted oracle assertion for the Spark leg (mirrors
/// `maintenance_conformance/gate.rs::assert_equivalence`): materialize `S_k`
/// as a temp view ([`STracker::materialize_s_as_view`] — Spark has no
/// `CREATE TEMP TABLE` DDL), then `EXCEPT ALL`-compare it against the
/// maintained Delta table `<schema>.<model_name>`.
pub async fn assert_equivalence_spark(
    recipe: &ModelRecipe,
    backend: &dyn Backend,
    tracker: &STracker,
    k: usize,
) -> anyhow::Result<()> {
    assert_equivalence_spark_with_edit(recipe, backend, tracker, k, None).await
}

/// [`assert_equivalence_spark`] generalised over an optional
/// post-`RewriteModel` [`ModelEdit`] (plan Phase 4), mirroring
/// `maintenance_conformance/gate.rs::assert_equivalence_with_edit`: when
/// `edit` is `Some`, the oracle re-renders against the REWRITTEN body
/// (`STracker::s_restricted_oracle_sql_with_edit`) rather than the original
/// recipe's own body. `edit: None` reproduces
/// [`assert_equivalence_spark`]'s exact behaviour.
pub async fn assert_equivalence_spark_with_edit(
    recipe: &ModelRecipe,
    backend: &dyn Backend,
    tracker: &STracker,
    k: usize,
    edit: Option<ModelEdit>,
) -> anyhow::Result<()> {
    tracker.materialize_s_as_view(backend, k).await?;
    let maintained_sql = format!(
        "SELECT * FROM {SPARK_CONFORMANCE_SCHEMA}.{}",
        recipe.model_name
    );
    let oracle_sql = match edit {
        Some(edit) => tracker.s_restricted_oracle_sql_with_edit(recipe, edit),
        None => tracker.s_restricted_oracle_sql(recipe),
    };
    let equal = multiset_equal_via_backend(backend, &maintained_sql, &oracle_sql).await?;
    if !equal {
        anyhow::bail!(
            "Spark S-restricted equivalence violated for model {:?} at run {k} (edit {edit:?}): \
             maintained ({maintained_sql:?}) != oracle ({oracle_sql:?})",
            recipe.model_name
        );
    }
    Ok(())
}

/// Drive `schedule` against `project`/`recipe` on Spark/Delta through the
/// real `execute_project` pipeline (`LinkCProject::run_with_target`),
/// asserting S-restricted multiset equivalence after every `RunWindow` step.
/// One Spark backend is opened up front and reused for every insert/snapshot/
/// materialize/compare call in this drive — a fresh `SparkBackend::new` per
/// call would both be slow (a new PySpark session each time) and break the
/// temp-view contract (`materialize_s_as_view`'s doc comment: the view is
/// scoped to the session that created it). Returns the populated `STracker`
/// plus the last recorded run index, so the self-check test can reuse the
/// green end-state without re-driving the whole schedule.
pub async fn drive_and_assert_spark(
    project: &LinkCProject,
    recipe: &ModelRecipe,
    schedule: &ConformanceSchedule,
) -> anyhow::Result<(STracker, usize)> {
    let backend = project
        .backend_for_target(ConformanceTarget::SparkDelta)
        .await?;
    let mut tracker = STracker::new(&recipe.source);
    let mut last_k: Option<usize> = None;
    // The most recent `RewriteModel` step's edit (plan Phase 4, mirroring
    // `maintenance_conformance/gate.rs::drive_and_assert`'s `current_edit`),
    // or `None` before any rewrite — threads into every subsequent assertion
    // so the oracle re-renders against the CURRENT on-disk body.
    let mut current_edit: Option<ModelEdit> = None;

    for (i, step) in schedule.0.iter().enumerate() {
        match step {
            ConformanceStep::RunWindow { start, end, rows } => {
                for row in rows {
                    insert_row(backend.as_ref(), recipe, row).await?;
                }

                let snapshot = read_source_snapshot_via_backend(
                    backend.as_ref(),
                    SPARK_CONFORMANCE_SCHEMA,
                    &recipe.source,
                )
                .await?;

                let mut request = base_request("spark");
                request.start = Some(start.format("%Y-%m-%d").to_string());
                request.end = Some(end.format("%Y-%m-%d").to_string());
                project
                    .run_with_target(
                        ConformanceTarget::SparkDelta,
                        &format!("spark-run-{i}"),
                        request,
                        &smelt_runtime::NoOpReporter,
                    )
                    .await?;

                let k = tracker.record_run(*start, *end, snapshot);
                last_k = Some(k);
                assert_equivalence_spark_with_edit(
                    recipe,
                    backend.as_ref(),
                    &tracker,
                    k,
                    current_edit,
                )
                .await?;
            }
            ConformanceStep::AppendLateRow(row) => {
                insert_row(backend.as_ref(), recipe, row).await?;
            }
            ConformanceStep::RerunWindow { start, end } => {
                // Redelivery: same window as an earlier `RunWindow`, no new
                // rows. Never-fold-twice under the partition-grain
                // DELETE+INSERT full-replace technique must hold on Spark
                // too.
                let snapshot = read_source_snapshot_via_backend(
                    backend.as_ref(),
                    SPARK_CONFORMANCE_SCHEMA,
                    &recipe.source,
                )
                .await?;

                let mut request = base_request("spark");
                request.start = Some(start.format("%Y-%m-%d").to_string());
                request.end = Some(end.format("%Y-%m-%d").to_string());
                project
                    .run_with_target(
                        ConformanceTarget::SparkDelta,
                        &format!("spark-run-{i}"),
                        request,
                        &smelt_runtime::NoOpReporter,
                    )
                    .await?;

                let k = tracker.record_run(*start, *end, snapshot);
                last_k = Some(k);
                assert_equivalence_spark_with_edit(
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
                let snapshot = read_source_snapshot_via_backend(
                    backend.as_ref(),
                    SPARK_CONFORMANCE_SCHEMA,
                    &recipe.source,
                )
                .await?;

                let mut request = base_request("spark");
                request.full_refresh = true;
                request.start = None;
                request.end = None;
                project
                    .run_with_target(
                        ConformanceTarget::SparkDelta,
                        &format!("spark-run-{i}"),
                        request,
                        &smelt_runtime::NoOpReporter,
                    )
                    .await?;

                let k = tracker.record_full_refresh(snapshot);
                last_k = Some(k);
                assert_equivalence_spark_with_edit(
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
                let snapshot = read_source_snapshot_via_backend(
                    backend.as_ref(),
                    SPARK_CONFORMANCE_SCHEMA,
                    &recipe.source,
                )
                .await?;

                let mut request = base_request("spark");
                request.start = Some(start.format("%Y-%m-%d").to_string());
                request.end = Some(end.format("%Y-%m-%d").to_string());
                project
                    .run_with_target(
                        ConformanceTarget::SparkDelta,
                        &format!("spark-run-{i}"),
                        request,
                        &smelt_runtime::NoOpReporter,
                    )
                    .await?;

                let k = tracker.record_run(*start, *end, snapshot);
                last_k = Some(k);
                assert_equivalence_spark_with_edit(
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
            ConformanceStep::DropStateDir | ConformanceStep::FreshClone => {
                // State-residency steps are DuckDB-only
                // (`docs/outcomes/20260816-state-residency/phases/08-plan.md`
                // task 4): the Spark twin has no ledger builder, so an
                // additive-graded cell downgrades to the recompute family
                // (phase 5's `MaintenanceStateDowngraded`) rather than
                // carrying any engine-resident state a residency step could
                // meaningfully delete or clone away from. Refuse rather than
                // silently no-op.
                anyhow::bail!(
                    "residency steps are DuckDB-only — the Spark twin has no ledger builder \
                     to survive a `.smelt/` deletion or clone (phase 5's ledger-less-backend \
                     downgrade)"
                );
            }
            ConformanceStep::MigrateApply => {
                // `MigrateApply` is a DuckDB-CLI-driven step
                // (`docs/outcomes/20260816-definition-delta-migrate-v2/
                // phases/06-plan.md`) — never part of the Spark pool, which
                // has no `MigrateApply`-emitting generator or pinned
                // schedule. Fail loud rather than silently skip, so a future
                // schedule that accidentally reaches this arm is caught
                // immediately instead of quietly passing over unproven
                // ground.
                panic!("ConformanceStep::MigrateApply is not part of the Spark conformance pool");
            }
        }
    }

    let last_k =
        last_k.ok_or_else(|| anyhow::anyhow!("schedule {schedule:?} had no RunWindow step"))?;
    Ok((tracker, last_k))
}

/// `append_only_partition_pool_upholds_equivalence_on_spark` (plan Phase 3
/// TDD list): the Spark twin of `maintenance_conformance::gate::append_only_partition_pool_upholds_equivalence`
/// — same recipe pool and schedule driver, deterministic seed, case count
/// from `SMELT_CONFORMANCE_SPARK_CASES` (default 4).
#[test]
fn append_only_partition_pool_upholds_equivalence_on_spark() {
    let Some(_url) = spark_connect_url() else {
        eprintln!(
            "SPARK_CONNECT_URL unset — skipping \
             append_only_partition_pool_upholds_equivalence_on_spark"
        );
        return;
    };

    let n = case_count();
    let mut runner = TestRunner::deterministic();
    let recipe_strat = arb_recipe(RecipePool::partition_append_only());

    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let mut admitted_cases = 0;

    for i in 0..n {
        let recipe = recipe_strat.new_tree(&mut runner).unwrap().current();
        let schedule = arb_schedule_for(&recipe)
            .new_tree(&mut runner)
            .unwrap()
            .current();

        let tmp = tempfile::TempDir::new().expect("tempdir");
        let project = stage_recipe_spark(&recipe, &tmp).unwrap_or_else(|e| {
            panic!("case {i}: recipe {recipe:?} failed to stage on Spark: {e}")
        });

        let verdict = classify(&project, &recipe)
            .unwrap_or_else(|e| panic!("case {i}: recipe {recipe:?} classify failed: {e}"));

        match verdict {
            Verdict::Refused(_) => continue,
            Verdict::Admitted(_) => {
                admitted_cases += 1;
                rt.block_on(drive_and_assert_spark(&project, &recipe, &schedule))
                    .unwrap_or_else(|e| {
                        panic!(
                            "case {i}: recipe {recipe:?} schedule {schedule:?} Spark \
                             equivalence check failed: {e}"
                        )
                    });
            }
        }
    }

    assert!(
        admitted_cases > 0,
        "N={n} deterministic sample admitted zero cases on Spark — generator/derivation \
         regression"
    );
}

/// Default deterministic sample size for
/// `admission_rate_stays_above_floor_on_spark` — `SMELT_CONFORMANCE_SPARK_ADMISSION_N`
/// env override, smaller than the DuckDB leg's 50 (each case still round-trips
/// staging DDL over the live Spark Connect server even though no run is
/// driven).
const ADMISSION_DEFAULT_N: usize = 20;

fn admission_sample_size() -> usize {
    std::env::var("SMELT_CONFORMANCE_SPARK_ADMISSION_N")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(ADMISSION_DEFAULT_N)
}

/// `admission_rate_stays_above_floor_on_spark` (plan Phase 4 TDD list): the
/// Spark twin of `maintenance_conformance::gate::admission_rate_stays_above_floor`
/// — generator health over the SAME recipe pool, same 40% floor, staged (not
/// driven) on Spark.
#[test]
fn admission_rate_stays_above_floor_on_spark() {
    let Some(_url) = spark_connect_url() else {
        eprintln!("SPARK_CONNECT_URL unset — skipping admission_rate_stays_above_floor_on_spark");
        return;
    };

    let n = admission_sample_size();
    let mut runner = TestRunner::deterministic();
    let recipe_strat = arb_recipe(RecipePool::partition_append_only());

    let mut admitted = 0;
    for i in 0..n {
        let recipe = recipe_strat.new_tree(&mut runner).unwrap().current();
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let project = stage_recipe_spark(&recipe, &tmp).unwrap_or_else(|e| {
            panic!("case {i}: recipe {recipe:?} failed to stage on Spark: {e}")
        });
        let verdict = classify(&project, &recipe)
            .unwrap_or_else(|e| panic!("case {i}: recipe {recipe:?} classify failed: {e}"));
        if matches!(verdict, Verdict::Admitted(_)) {
            admitted += 1;
        }
    }

    let rate = admitted as f64 / n as f64;
    assert!(
        rate >= 0.40,
        "Spark admission rate {rate:.2} over N={n} fell below the 40% generator-health floor \
         ({admitted}/{n} admitted)"
    );
}

/// `redelivery_of_processed_window_is_idempotent_on_spark` (plan Phase 4 TDD
/// list): the Spark twin of
/// `maintenance_conformance::gate::redelivery_of_processed_window_is_idempotent`
/// — re-running an already-processed window with no new rows never
/// double-counts under the partition-grain DELETE+INSERT full-replace
/// technique on Spark/Delta either.
#[test]
fn redelivery_of_processed_window_is_idempotent_on_spark() {
    let Some(_url) = spark_connect_url() else {
        eprintln!(
            "SPARK_CONNECT_URL unset — skipping \
             redelivery_of_processed_window_is_idempotent_on_spark"
        );
        return;
    };

    let mut runner = TestRunner::deterministic();
    let pool = RecipePool {
        constructs: vec![ConstructKind::AdditiveAgg],
    };
    let recipe = arb_recipe(pool).new_tree(&mut runner).unwrap().current();

    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project = stage_recipe_spark(&recipe, &tmp).expect("stage spark recipe");
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

    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(drive_and_assert_spark(&project, &recipe, &schedule))
        .expect(
            "redelivering an already-processed window must stay idempotent on Spark — \
             equivalence must hold after every redelivery, never double-counted",
        );
}

/// `full_refresh_interleave_resets_state_correctly_on_spark` (plan Phase 4
/// TDD list): the Spark twin of
/// `maintenance_conformance::gate::full_refresh_interleave_resets_state_correctly`
/// — a mid-schedule `full_refresh` run resets coverage such that subsequent
/// incremental runs still uphold equivalence on Spark/Delta.
#[test]
fn full_refresh_interleave_resets_state_correctly_on_spark() {
    let Some(_url) = spark_connect_url() else {
        eprintln!(
            "SPARK_CONNECT_URL unset — skipping \
             full_refresh_interleave_resets_state_correctly_on_spark"
        );
        return;
    };

    let mut runner = TestRunner::deterministic();
    let pool = RecipePool {
        constructs: vec![ConstructKind::AdditiveAgg],
    };
    let recipe = arb_recipe(pool).new_tree(&mut runner).unwrap().current();

    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project = stage_recipe_spark(&recipe, &tmp).expect("stage spark recipe");
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

    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(drive_and_assert_spark(&project, &recipe, &schedule))
        .expect(
            "a mid-schedule full_refresh interleave must reset coverage cleanly on Spark — \
             equivalence must hold both immediately after the refresh and through every \
             subsequent windowed run",
        );
}

/// Count rows returned by a `SELECT COUNT(*) ...` query via the Backend
/// trait — the boundary leg's read-back, routed through
/// `Backend::execute_sql` rather than a raw host connection (spec's
/// backend-client-API requirement).
async fn count_via_backend(backend: &dyn Backend, sql: &str) -> anyhow::Result<i64> {
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

/// `boundary_rows_within_reach_are_reflected_on_spark` (plan Phase 4 TDD
/// list): the Spark twin of
/// `maintenance_conformance::gate::boundary_rows_within_reach_are_reflected`
/// — a just-inside-reach row appears in the maintained Delta output after the
/// run whose window covers it; the row one calendar day further out does
/// not. Read-back goes through `Backend::execute_sql` rather than a raw
/// DuckDB connection (no host connection exists on the Spark leg).
#[test]
fn boundary_rows_within_reach_are_reflected_on_spark() {
    let Some(_url) = spark_connect_url() else {
        eprintln!(
            "SPARK_CONNECT_URL unset — skipping boundary_rows_within_reach_are_reflected_on_spark"
        );
        return;
    };

    let mut runner = TestRunner::deterministic();
    let pool = RecipePool {
        constructs: vec![ConstructKind::AdditiveAgg],
    };
    let recipe = arb_recipe(pool).new_tree(&mut runner).unwrap().current();

    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project = stage_recipe_spark(&recipe, &tmp).expect("stage spark recipe");

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
    let backend = rt
        .block_on(project.backend_for_target(ConformanceTarget::SparkDelta))
        .expect("open spark backend");

    rt.block_on(insert_row(backend.as_ref(), &recipe, &boundary.just_inside))
        .expect("insert just-inside row");
    rt.block_on(insert_row(
        backend.as_ref(),
        &recipe,
        &boundary.just_outside,
    ))
    .expect("insert just-outside row");

    let mut request = base_request("spark");
    request.start = Some(window.0.format("%Y-%m-%d").to_string());
    request.end = Some(window.1.format("%Y-%m-%d").to_string());
    rt.block_on(project.run_with_target(
        ConformanceTarget::SparkDelta,
        "spark-boundary-run",
        request,
        &smelt_runtime::NoOpReporter,
    ))
    .expect("triggering run over the boundary window");

    let count = rt
        .block_on(count_via_backend(
            backend.as_ref(),
            &format!(
                "SELECT COUNT(*) FROM {SPARK_CONFORMANCE_SCHEMA}.{} WHERE {} = DATE '{}'",
                recipe.model_name,
                recipe.source.clock_column,
                boundary.just_inside.d.format("%Y-%m-%d"),
            ),
        ))
        .expect("count rows for the boundary just-inside day");
    assert!(
        count > 0,
        "a just-inside-reach row (day {}) must appear in the maintained Spark output after the \
         triggering run over {window:?} — an under-derived clamp would drop it",
        boundary.just_inside.d,
    );

    let outside_count = rt
        .block_on(count_via_backend(
            backend.as_ref(),
            &format!(
                "SELECT COUNT(*) FROM {SPARK_CONFORMANCE_SCHEMA}.{} WHERE {} = DATE '{}'",
                recipe.model_name,
                recipe.source.clock_column,
                boundary.just_outside.d.format("%Y-%m-%d"),
            ),
        ))
        .expect("count rows for the boundary just-outside day");
    assert_eq!(
        outside_count, 0,
        "a just-outside-reach row (day {}) must NOT appear in the maintained Spark output after \
         the triggering run over {window:?} — an over-derived (or off-by-one) scan predicate \
         would read it even though it lies outside the derived reach",
        boundary.just_outside.d,
    );
}

/// `column_add_between_runs_recovers_equivalence_on_spark` (plan Phase 4 TDD
/// list): the Spark twin of
/// `maintenance_conformance::gate::column_add_between_runs_recovers_equivalence`
/// — a column-add `RewriteModel` followed by a `full_refresh` catch-up
/// recovers full equivalence against the rewritten body's own oracle on
/// Spark/Delta too, and stays recovered through a subsequent windowed run.
#[test]
fn column_add_between_runs_recovers_equivalence_on_spark() {
    let Some(_url) = spark_connect_url() else {
        eprintln!(
            "SPARK_CONNECT_URL unset — skipping \
             column_add_between_runs_recovers_equivalence_on_spark"
        );
        return;
    };

    let mut runner = TestRunner::deterministic();
    let pool = RecipePool {
        constructs: vec![ConstructKind::AdditiveAgg],
    };
    let recipe = arb_recipe(pool).new_tree(&mut runner).unwrap().current();

    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project = stage_recipe_spark(&recipe, &tmp).expect("stage spark recipe");
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

    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(drive_and_assert_spark(&project, &recipe, &schedule))
        .expect(
            "a column-add RewriteModel followed by a full-refresh catch-up must recover full \
             equivalence on Spark against the rewritten body's own oracle, and stay recovered \
             through a subsequent windowed run",
        );
}
